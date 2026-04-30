//! `rai conflicts run` — main loop。

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context};
use clap::Args;
use rai_core::{cli::Run, shell, signals, ts, Ctx, Result};
use serde::Deserialize;

use crate::queue::{self, Entry, Paths};

#[derive(Debug, Args)]
pub struct Cmd {
    /// agent CLI 起動コマンド (shell-words で分割される)。
    #[arg(long, value_name = "CMD")]
    agent_cmd: String,

    /// 対象 PR の作者フィルタ。
    #[arg(long, default_value = "@me")]
    author: String,

    /// 全 PR を対象にする。
    #[arg(long)]
    all: bool,

    /// enqueue 間隔 (秒)。
    #[arg(long, default_value_t = 300)]
    interval: u64,

    /// 同時 worker 数。
    #[arg(long, default_value_t = 3)]
    jobs: u32,

    /// 1 サイクル後 pending=0 で exit。
    #[arg(long)]
    once: bool,

    #[arg(long, value_name = "PATH")]
    state_dir: Option<PathBuf>,

    #[arg(long, value_name = "PATH")]
    cache_dir: Option<PathBuf>,

    /// 強制対象 PR 番号 (CONFLICTING フィルタをバイパス)。
    #[arg(value_name = "PR")]
    pr: Vec<u64>,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let paths = Paths::new(self.state_dir.clone(), self.cache_dir.clone());
        paths.ensure_dirs()?;

        let _lock = queue::Lock::try_acquire(&paths.lock_file())?;

        let signal_slot = signals::install()?;
        let agent_argv =
            shell_words::split(&self.agent_cmd).context("failed to split --agent-cmd")?;
        if agent_argv.is_empty() {
            bail!("--agent-cmd is empty");
        }
        let agent_argv = Arc::new(agent_argv);
        let paths = Arc::new(paths);

        let (done_tx, done_rx) = mpsc::channel::<u64>();
        let mut alive: BTreeMap<u64, thread::JoinHandle<()>> = BTreeMap::new();

        let queue_mutex = Arc::new(Mutex::new(()));

        loop {
            // enqueue.
            if let Err(e) = enqueue(&paths, &queue_mutex, &self.pr, &self.author, self.all) {
                ts::println(format!("enqueue error: {e}"));
            }

            // spawn workers up to --jobs.
            while alive.len() < self.jobs as usize && signal_slot.load(Ordering::SeqCst) == 0 {
                let claimed = pop_pending(&paths, &queue_mutex)?;
                let Some((pr, entry)) = claimed else { break };
                let paths = paths.clone();
                let agent_argv = agent_argv.clone();
                let queue_mutex = queue_mutex.clone();
                let done_tx = done_tx.clone();
                let handle = thread::Builder::new()
                    .name(format!("conflicts-worker-{pr}"))
                    .spawn(move || {
                        let result = process_one(&paths, &agent_argv, pr, &entry);
                        finalize(&paths, &queue_mutex, pr, result);
                        done_tx.send(pr).ok();
                    })?;
                alive.insert(pr, handle);
            }

            // reap finished workers (non-blocking).
            while let Ok(pr) = done_rx.try_recv() {
                if let Some(h) = alive.remove(&pr) {
                    let _ = h.join();
                }
            }

            // termination conditions.
            let pending = count_pending(&paths, &queue_mutex)?;
            let signaled = signal_slot.load(Ordering::SeqCst) != 0;
            if signaled && alive.is_empty() {
                break;
            }
            if self.once && pending == 0 && alive.is_empty() {
                break;
            }
            if signaled {
                ts::println(format!("waiting for {} in-flight workers", alive.len()));
            }

            thread::sleep(Duration::from_secs(if signaled {
                1
            } else {
                self.interval
            }));
        }

        for (pr, h) in alive {
            ts::println(format!("await worker pr={pr}"));
            let _ = h.join();
        }

        // shutdown summary.
        let q = queue::load(&paths.queue_json())?;
        eprintln!("--- shutdown summary ---");
        for (pr, e) in &q.entries {
            eprintln!("{pr}\t{}\t{}", e.status, e.title);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct PrInfo {
    number: u64,
    title: String,
    url: String,
    mergeable: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "headRefOid")]
    head_ref_oid: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
}

fn fetch_pr_list(author: &str, all: bool) -> Result<Vec<PrInfo>> {
    let mut args = vec![
        "pr",
        "list",
        "--state",
        "open",
        "--limit",
        "200",
        "--json",
        "number,title,url,mergeable,headRefName,headRefOid,baseRefName",
    ];
    if !all {
        args.push("--author");
        args.push(author);
    }
    let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 1);
    argv.push("gh");
    argv.extend(args.iter().copied());
    let out = shell::user_shell_argv(&argv)
        .output()
        .context("failed to spawn `gh pr list`")?;
    if !out.status.success() {
        bail!(
            "gh pr list failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let list: Vec<PrInfo> =
        serde_json::from_slice(&out.stdout).context("failed to parse `gh pr list` JSON")?;
    Ok(list)
}

fn fetch_pr_one(pr: u64) -> Result<PrInfo> {
    let pr_str = pr.to_string();
    let out = shell::user_shell_argv(&[
        "gh",
        "pr",
        "view",
        &pr_str,
        "--json",
        "number,title,url,mergeable,headRefName,headRefOid,baseRefName",
    ])
    .output()
    .context("failed to spawn `gh pr view`")?;
    if !out.status.success() {
        bail!(
            "gh pr view {pr} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let info: PrInfo =
        serde_json::from_slice(&out.stdout).context("failed to parse `gh pr view` JSON")?;
    Ok(info)
}

fn enqueue(
    paths: &Paths,
    qmx: &Mutex<()>,
    explicit: &[u64],
    author: &str,
    all: bool,
) -> Result<()> {
    let _g = qmx.lock().unwrap();
    let mut q = queue::load(&paths.queue_json())?;
    let now = queue::now_iso();

    if !explicit.is_empty() {
        for pr in explicit {
            let info = match fetch_pr_one(*pr) {
                Ok(i) => i,
                Err(e) => {
                    ts::println(format!("warn: pr {pr}: {e}"));
                    continue;
                }
            };
            upsert(&mut q, &info, &now, true);
        }
    } else {
        let list = fetch_pr_list(author, all)?;
        for info in list {
            if info.mergeable != "CONFLICTING" {
                continue;
            }
            upsert(&mut q, &info, &now, false);
        }
    }
    queue::save(&paths.queue_json(), &mut q)?;
    Ok(())
}

fn upsert(q: &mut crate::queue::Queue, info: &PrInfo, now: &str, force: bool) {
    let key = info.number.to_string();
    let existing = q.entries.get(&key).cloned();
    let new_entry = Entry {
        head_sha: info.head_ref_oid.clone(),
        base_ref: info.base_ref_name.clone(),
        head_ref: info.head_ref_name.clone(),
        mergeable: info.mergeable.clone(),
        title: info.title.clone(),
        url: info.url.clone(),
        status: "pending".into(),
        attempts: existing.as_ref().map(|e| e.attempts).unwrap_or(0),
        enqueued_at: existing
            .as_ref()
            .map(|e| e.enqueued_at.clone())
            .unwrap_or_else(|| now.to_string()),
        updated_at: now.to_string(),
        started_at: existing
            .as_ref()
            .map(|e| e.started_at.clone())
            .unwrap_or_default(),
        finished_at: existing
            .as_ref()
            .map(|e| e.finished_at.clone())
            .unwrap_or_default(),
        log_path: existing
            .as_ref()
            .map(|e| e.log_path.clone())
            .unwrap_or_default(),
        error: String::new(),
    };

    match existing {
        Some(prev) if !force => {
            if prev.head_sha != new_entry.head_sha
                || prev.status == "failed"
                || prev.status == "done"
            {
                q.entries.insert(key, new_entry);
            }
        }
        _ => {
            q.entries.insert(key, new_entry);
        }
    }
}

fn pop_pending(paths: &Paths, qmx: &Mutex<()>) -> Result<Option<(u64, Entry)>> {
    let _g = qmx.lock().unwrap();
    let mut q = queue::load(&paths.queue_json())?;
    let oldest = q
        .entries
        .iter()
        .filter(|(_, e)| e.status == "pending")
        .min_by_key(|(_, e)| e.enqueued_at.clone())
        .map(|(k, _)| k.clone());
    let Some(key) = oldest else { return Ok(None) };
    let entry = q.entries.get_mut(&key).unwrap();
    entry.status = "claimed".into();
    entry.updated_at = queue::now_iso();
    let snap = entry.clone();
    queue::save(&paths.queue_json(), &mut q)?;
    let pr: u64 = key.parse().unwrap_or(0);
    Ok(Some((pr, snap)))
}

fn count_pending(paths: &Paths, qmx: &Mutex<()>) -> Result<usize> {
    let _g = qmx.lock().unwrap();
    let q = queue::load(&paths.queue_json())?;
    Ok(q.entries.values().filter(|e| e.status == "pending").count())
}

struct WorkerOutcome {
    log_path: PathBuf,
    error: Option<String>,
}

fn process_one(paths: &Paths, agent_argv: &[String], pr: u64, entry: &Entry) -> WorkerOutcome {
    let log_path = paths
        .logs_dir()
        .join(format!("{pr}-{}.log", short(&entry.head_sha)));
    let log_writer = OpenOptions::new().create(true).append(true).open(&log_path);
    let log_writer = match log_writer {
        Ok(f) => Mutex::new(f),
        Err(e) => {
            return WorkerOutcome {
                log_path,
                error: Some(format!("log open failed: {e}")),
            };
        }
    };

    let log = |s: &str| {
        if let Ok(mut f) = log_writer.lock() {
            let _ = writeln!(*f, "[{}] {s}", queue::now_iso());
            let _ = f.flush();
        }
    };

    log(&format!("=== begin pr={pr} sha={} ===", entry.head_sha));
    let wt = paths.worktree_dir(pr);
    let _ = run_git(
        &log,
        &["worktree", "remove", "--force", &wt.display().to_string()],
    );
    if let Err(e) = fs::create_dir_all(wt.parent().unwrap_or(&wt)) {
        log(&format!("mkdir parent: {e}"));
    }

    let pr_ref = format!("refs/remotes/origin/pr/{pr}");
    if let Err(e) = run_git(
        &log,
        &[
            "fetch",
            "origin",
            &format!("pull/{pr}/head:{pr_ref}"),
            "--force",
        ],
    ) {
        return WorkerOutcome {
            log_path,
            error: Some(format!("fetch pr: {e}")),
        };
    }
    if let Err(e) = run_git(&log, &["fetch", "origin", &entry.base_ref]) {
        return WorkerOutcome {
            log_path,
            error: Some(format!("fetch base: {e}")),
        };
    }
    if let Err(e) = run_git(
        &log,
        &[
            "worktree",
            "add",
            "--detach",
            &wt.display().to_string(),
            &format!("origin/pr/{pr}"),
        ],
    ) {
        return WorkerOutcome {
            log_path,
            error: Some(format!("worktree add: {e}")),
        };
    }

    let cleanup_wt = || {
        let _ = run_git(
            &log,
            &["worktree", "remove", "--force", &wt.display().to_string()],
        );
    };

    if let Err(e) = run_in(
        &log,
        &wt,
        "gh",
        &["pr", "checkout", &pr.to_string(), "--force"],
    ) {
        cleanup_wt();
        return WorkerOutcome {
            log_path,
            error: Some(format!("gh pr checkout: {e}")),
        };
    }

    let merge = run_in(
        &log,
        &wt,
        "git",
        &["merge", "--no-edit", &format!("origin/{}", entry.base_ref)],
    );
    let conflict = merge.is_err();

    if conflict {
        let prompt = format!(
            "PR #{pr} ({title}) のコンフリクトを解消して `git push --force-with-lease` まで完了させてください。マージ中: origin/{base} -> HEAD。リポジトリ作業ディレクトリ: {wt}",
            title = entry.title,
            base = entry.base_ref,
            wt = wt.display(),
        );
        let mut argv = agent_argv.to_vec();
        argv.push(prompt);
        let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let mut c = shell::user_shell_argv(&argv_refs);
        c.current_dir(&wt);
        attach_log(&mut c, &log_path);
        let st = c.status();
        match st {
            Ok(s) if s.success() => {}
            Ok(s) => {
                let _ = run_in(&log, &wt, "git", &["merge", "--abort"]);
                cleanup_wt();
                return WorkerOutcome {
                    log_path,
                    error: Some(format!("agent exit={:?}", s.code())),
                };
            }
            Err(e) => {
                let _ = run_in(&log, &wt, "git", &["merge", "--abort"]);
                cleanup_wt();
                return WorkerOutcome {
                    log_path,
                    error: Some(format!("agent spawn: {e}")),
                };
            }
        }

        if has_unresolved_markers(&wt) {
            let _ = run_in(&log, &wt, "git", &["merge", "--abort"]);
            cleanup_wt();
            return WorkerOutcome {
                log_path,
                error: Some("unresolved markers remain".to_string()),
            };
        }

        if is_dirty(&wt) {
            let _ = run_in(&log, &wt, "git", &["add", "-A"]);
            let _ = run_in(
                &log,
                &wt,
                "git",
                &[
                    "commit",
                    "-m",
                    &format!(
                        "Merge {} into PR #{} (automated conflict resolution)",
                        entry.base_ref, pr
                    ),
                ],
            );
        }
    }

    let push_needed = should_push(&wt, &entry.base_ref);
    if push_needed {
        if let Err(e) = run_in(&log, &wt, "git", &["push", "--force-with-lease"]) {
            cleanup_wt();
            return WorkerOutcome {
                log_path,
                error: Some(format!("git push: {e}")),
            };
        }
    }

    cleanup_wt();
    log("=== done ===");
    WorkerOutcome {
        log_path,
        error: None,
    }
}

fn finalize(paths: &Paths, qmx: &Mutex<()>, pr: u64, outcome: WorkerOutcome) {
    let _g = qmx.lock().unwrap();
    let Ok(mut q) = queue::load(&paths.queue_json()) else {
        return;
    };
    let key = pr.to_string();
    if let Some(e) = q.entries.get_mut(&key) {
        e.attempts += 1;
        e.log_path = outcome.log_path.display().to_string();
        e.finished_at = queue::now_iso();
        e.updated_at = e.finished_at.clone();
        match outcome.error {
            None => {
                e.status = "done".into();
                e.error.clear();
            }
            Some(err) => {
                e.status = "failed".into();
                e.error = err;
            }
        }
    }
    let _ = queue::save(&paths.queue_json(), &mut q);
}

fn run_git(log: &dyn Fn(&str), args: &[&str]) -> Result<()> {
    log(&format!("git {}", args.join(" ")));
    let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 1);
    argv.push("git");
    argv.extend_from_slice(args);
    let st = shell::user_shell_argv(&argv).status()?;
    if !st.success() {
        bail!("git {} -> {:?}", args.join(" "), st.code());
    }
    Ok(())
}

fn run_in(log: &dyn Fn(&str), cwd: &Path, bin: &str, args: &[&str]) -> Result<()> {
    log(&format!(
        "[{cwd}] {bin} {a}",
        cwd = cwd.display(),
        a = args.join(" ")
    ));
    let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 1);
    argv.push(bin);
    argv.extend_from_slice(args);
    let st = shell::user_shell_argv(&argv).current_dir(cwd).status()?;
    if !st.success() {
        bail!("{bin} {} -> {:?}", args.join(" "), st.code());
    }
    Ok(())
}

fn attach_log(cmd: &mut Command, log_path: &std::path::Path) {
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .ok();
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .ok();
    if let Some(s) = stdout {
        cmd.stdout(Stdio::from(s));
    }
    if let Some(s) = stderr {
        cmd.stderr(Stdio::from(s));
    }
}

fn has_unresolved_markers(wt: &Path) -> bool {
    let wt_str = wt.display().to_string();
    let out = shell::user_shell_argv(&[
        "git",
        "-C",
        &wt_str,
        "diff",
        "--name-only",
        "--diff-filter=U",
    ])
    .output();
    matches!(out, Ok(o) if !o.stdout.is_empty())
}

fn is_dirty(wt: &Path) -> bool {
    let wt_str = wt.display().to_string();
    let out = shell::user_shell_argv(&["git", "-C", &wt_str, "status", "--porcelain"]).output();
    matches!(out, Ok(o) if !o.stdout.is_empty())
}

fn should_push(wt: &Path, base: &str) -> bool {
    let _ = base; // base is informational here; ahead vs upstream check is enough.
    let wt_str = wt.display().to_string();
    let out = shell::user_shell_argv(&[
        "git",
        "-C",
        &wt_str,
        "rev-list",
        "--left-right",
        "--count",
        "@{u}...HEAD",
    ])
    .output();
    let Ok(o) = out else { return false };
    if !o.status.success() {
        return false;
    }
    let s = String::from_utf8_lossy(&o.stdout);
    let mut it = s.split_whitespace();
    let _behind = it.next().and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
    let ahead = it.next().and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
    let _ = anyhow::Result::<()>::Ok(()); // silence unused import warnings
    ahead > 0
}

fn short(s: &str) -> &str {
    s.get(..7).unwrap_or(s)
}
