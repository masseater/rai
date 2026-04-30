//! `rai gwq clean` — fish 版 gwq-clean (248 行) の Rust 移植。

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, bail, Context};
use clap::Args;
use rai_core::{cli::Run, shell, Ctx, Result};

#[derive(Debug, Args)]
pub struct Cmd {
    /// 全 worktree を一覧 (preselect なし)。
    #[arg(long)]
    all: bool,
    /// dirty な worktree も preselect 対象に含める。
    #[arg(long)]
    include_dirty: bool,
    /// default branch を上書き。
    #[arg(long, value_name = "BR")]
    default_branch: Option<String>,
    /// 対象 remote (default: origin)。
    #[arg(long, default_value = "origin")]
    remote: String,
    /// 副作用なしで予定だけ表示。
    #[arg(long)]
    dry_run: bool,
    /// 確認プロンプトをスキップ。
    #[arg(long)]
    yes: bool,
    /// 機械可読 JSON で結果を出力 (non-tty では必須)。
    #[arg(long)]
    json: bool,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        if !io::stdout().is_terminal() && !self.json {
            bail!("non-tty: pass --json (or run from a tty)");
        }

        let default_branch = match &self.default_branch {
            Some(b) => b.clone(),
            None => detect_default_branch(&self.remote)?,
        };

        // fetch --prune
        let st = shell::user_shell_argv(&["git", "fetch", "--prune", "--quiet", &self.remote])
            .status()
            .context("failed to spawn git fetch")?;
        if !st.success() {
            eprintln!("warn: git fetch --prune {} failed", self.remote);
        }

        let merged: BTreeSet<String> = list_merged(&self.remote, &default_branch)?;
        let gone: BTreeSet<String> = list_gone()?;
        let last_commits = list_last_commits()?;
        let entries = list_worktrees(&default_branch, &merged, &gone, &last_commits)?;

        let preselect: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                matches!(e.status, Status::Merged | Status::Gone)
                    || (self.include_dirty && matches!(e.status, Status::Dirty))
            })
            .map(|(i, _)| i)
            .collect();

        let chosen: Vec<usize> = if self.json {
            preselect.clone()
        } else {
            select_with_fzf(&entries, &preselect)?
        };

        if chosen.is_empty() {
            if self.json {
                println!("{}", serde_json::json!({"removed": [], "failed": []}));
            } else {
                eprintln!("nothing to do");
            }
            return Ok(());
        }

        if !self.json && !self.yes {
            eprint!("よろしいですか？ [y/N]: ");
            io::stderr().flush().ok();
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line)?;
            if !matches!(line.trim().chars().next(), Some('y') | Some('Y')) {
                eprintln!("aborted");
                return Ok(());
            }
        }

        let mut removed: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        for idx in chosen {
            let e = &entries[idx];
            if self.dry_run {
                eprintln!("would remove: {} ({})", e.branch, e.path.display());
                removed.push(e.branch.clone());
                continue;
            }
            if remove_one(e).is_ok() {
                if branch_exists(&e.branch) {
                    eprintln!("✗ failed: {}", e.branch);
                    failed.push(e.branch.clone());
                } else {
                    eprintln!("✓ removed: {}", e.branch);
                    removed.push(e.branch.clone());
                }
            } else {
                eprintln!("✗ failed: {}", e.branch);
                failed.push(e.branch.clone());
            }
        }

        if self.json {
            println!(
                "{}",
                serde_json::json!({
                    "removed": removed,
                    "failed": failed,
                    "default_branch": default_branch,
                })
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum Status {
    Merged,
    Gone,
    Dirty,
    Active,
}

impl Status {
    fn tag(self) -> &'static str {
        match self {
            Status::Merged => "MERGED",
            Status::Gone => "GONE",
            Status::Dirty => "DIRTY",
            Status::Active => "ACTIVE",
        }
    }
}

#[derive(Debug)]
struct Entry {
    path: PathBuf,
    branch: String,
    status: Status,
    last_commit: String,
    dirty: bool,
}

fn detect_default_branch(remote: &str) -> Result<String> {
    let head_ref = format!("refs/remotes/{remote}/HEAD");
    let symref = shell::user_shell_argv(&["git", "symbolic-ref", "--quiet", &head_ref]).output()?;
    if symref.status.success() {
        let s = String::from_utf8_lossy(&symref.stdout).trim().to_string();
        let prefix = format!("refs/remotes/{remote}/");
        if let Some(rest) = s.strip_prefix(&prefix) {
            return Ok(rest.to_string());
        }
    }
    for cand in ["main", "master"] {
        let cand_ref = format!("refs/heads/{cand}");
        let st = shell::user_shell_argv(&["git", "rev-parse", "--verify", "--quiet", &cand_ref])
            .status()?;
        if st.success() {
            return Ok(cand.to_string());
        }
    }
    bail!("could not detect default branch (no origin/HEAD, main, or master)")
}

fn list_merged(remote: &str, default_branch: &str) -> Result<BTreeSet<String>> {
    let merged_ref = format!("{remote}/{default_branch}");
    let out = shell::user_shell_argv(&["git", "branch", "--merged", &merged_ref]).output()?;
    if !out.status.success() {
        return Ok(BTreeSet::new());
    }
    let mut set = BTreeSet::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let s = line.trim_start_matches('*').trim();
        if s.is_empty() {
            continue;
        }
        set.insert(s.to_string());
    }
    Ok(set)
}

fn list_gone() -> Result<BTreeSet<String>> {
    let out = shell::user_shell_argv(&["git", "branch", "-vv"]).output()?;
    let mut set = BTreeSet::new();
    if !out.status.success() {
        return Ok(set);
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if !line.contains(": gone]") {
            continue;
        }
        let s = line.trim_start_matches('*').trim_start_matches('+').trim();
        if let Some(name) = s.split_whitespace().next() {
            set.insert(name.to_string());
        }
    }
    Ok(set)
}

fn list_last_commits() -> Result<HashMap<String, String>> {
    let out = shell::user_shell_argv(&[
        "git",
        "for-each-ref",
        "--format=%(refname:short)\t%(committerdate:short) %(subject)",
        "refs/heads",
    ])
    .output()?;
    let mut map = HashMap::new();
    if !out.status.success() {
        return Ok(map);
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some((name, info)) = line.split_once('\t') {
            map.insert(name.to_string(), info.to_string());
        }
    }
    Ok(map)
}

fn list_worktrees(
    default_branch: &str,
    merged: &BTreeSet<String>,
    gone: &BTreeSet<String>,
    last_commits: &HashMap<String, String>,
) -> Result<Vec<Entry>> {
    let out = shell::user_shell_argv(&["git", "worktree", "list", "--porcelain"]).output()?;
    if !out.status.success() {
        bail!("git worktree list --porcelain failed");
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut cur_path: Option<PathBuf> = None;
    let mut cur_branch: Option<String> = None;
    let mut cur_bare = false;
    let body = String::from_utf8_lossy(&out.stdout).into_owned();
    for line in body.lines() {
        if line.is_empty() {
            flush_entry(
                &mut entries,
                cur_path.take(),
                cur_branch.take(),
                cur_bare,
                default_branch,
                merged,
                gone,
                last_commits,
            );
            cur_bare = false;
        } else if let Some(p) = line.strip_prefix("worktree ") {
            cur_path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch ") {
            cur_branch = Some(b.trim_start_matches("refs/heads/").to_string());
        } else if line == "bare" {
            cur_bare = true;
        }
    }
    flush_entry(
        &mut entries,
        cur_path.take(),
        cur_branch.take(),
        cur_bare,
        default_branch,
        merged,
        gone,
        last_commits,
    );
    Ok(entries)
}

#[allow(clippy::too_many_arguments)]
fn flush_entry(
    out: &mut Vec<Entry>,
    path: Option<PathBuf>,
    branch: Option<String>,
    bare: bool,
    default_branch: &str,
    merged: &BTreeSet<String>,
    gone: &BTreeSet<String>,
    last_commits: &HashMap<String, String>,
) {
    if bare {
        return;
    }
    let path = match path {
        Some(p) => p,
        None => return,
    };
    let branch = match branch {
        Some(b) => b,
        None => return,
    };
    if branch == default_branch {
        return;
    }
    let dirty = is_dirty(&path);
    let status = if merged.contains(&branch) {
        Status::Merged
    } else if gone.contains(&branch) {
        Status::Gone
    } else if dirty {
        Status::Dirty
    } else {
        Status::Active
    };
    let last = last_commits.get(&branch).cloned().unwrap_or_default();
    out.push(Entry {
        path,
        branch,
        status,
        last_commit: last,
        dirty,
    });
}

fn is_dirty(path: &Path) -> bool {
    let path_str = path.display().to_string();
    let out = shell::user_shell_argv(&["git", "-C", &path_str, "status", "--porcelain"]).output();
    match out {
        Ok(o) => !o.stdout.is_empty(),
        Err(_) => false,
    }
}

fn select_with_fzf(entries: &[Entry], preselect: &[usize]) -> Result<Vec<usize>> {
    let mut fzf = shell::user_shell_argv(&[
        "fzf",
        "--multi",
        "--reverse",
        "--no-sort",
        "--ansi",
        "--with-nth=2..",
        "--delimiter=\t",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit())
    .spawn()
    .context("failed to spawn `fzf`")?;
    {
        let mut stdin = fzf.stdin.take().ok_or_else(|| anyhow!("fzf stdin"))?;
        let preset: BTreeSet<usize> = preselect.iter().copied().collect();
        for (i, e) in entries.iter().enumerate() {
            let marker = if preset.contains(&i) { "*" } else { " " };
            let dirty = if e.dirty { "dirty" } else { "clean" };
            // fzf doesn't natively preselect via stdin; we encode marker so user can multi-select via Tab.
            writeln!(
                stdin,
                "{i}\t{}{marker}\t[{:>6}]  {}  {}  {}",
                marker,
                e.status.tag(),
                e.branch,
                e.last_commit,
                dirty,
            )
            .ok();
        }
    }
    let out = fzf.wait_with_output()?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    let mut chosen = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some((idx_s, _)) = line.split_once('\t') {
            if let Ok(i) = idx_s.parse::<usize>() {
                chosen.push(i);
            }
        }
    }
    Ok(chosen)
}

fn remove_one(e: &Entry) -> Result<()> {
    let st = shell::user_shell_argv(&["gwq", "remove", "-f", "-b", &e.branch]).status();
    let gwq_ok = matches!(st, Ok(s) if s.success());
    if !gwq_ok && e.path.exists() {
        // fallback: rm -rf + git worktree prune
        if let Err(err) = fs::remove_dir_all(&e.path) {
            eprintln!("warn: rm -rf {}: {err}", e.path.display());
        }
        shell::user_shell_argv(&["git", "worktree", "prune"])
            .status()
            .ok();
    }
    if branch_exists(&e.branch) {
        // squash-merged etc.: try git branch -D
        shell::user_shell_argv(&["git", "branch", "-D", &e.branch])
            .status()
            .ok();
    }
    Ok(())
}

fn branch_exists(branch: &str) -> bool {
    let head_ref = format!("refs/heads/{branch}");
    let st =
        shell::user_shell_argv(&["git", "rev-parse", "--verify", "--quiet", &head_ref]).status();
    matches!(st, Ok(s) if s.success())
}
