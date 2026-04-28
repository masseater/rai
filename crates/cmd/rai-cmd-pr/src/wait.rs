//! `rai pr wait` — GitHub PR の check-runs を polling し、完了で通知する。

use std::io::{self, IsTerminal, Write};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use clap::Args;
use rai_core::{cli::Run, proc, signals, Ctx, Result};
use serde::Deserialize;

#[derive(Debug, Args)]
pub struct Cmd {
    /// PR 識別子: 番号 (`123`) / URL / 省略 (現ブランチから自動)。
    #[arg(value_name = "PR")]
    pr: Option<String>,

    /// polling 間隔 (秒)。
    #[arg(long, default_value_t = 10)]
    interval: u64,

    /// `OWNER/REPO` を上書き。
    #[arg(long, value_name = "OWNER/REPO")]
    repo: Option<String>,

    /// デスクトップ通知を抑止する。
    #[arg(long)]
    no_notify: bool,

    /// 機械可読な JSON で結果を出力。
    #[arg(long)]
    json: bool,

    /// CI 失敗時に exit 1 で終了する。
    #[arg(long)]
    exit_on_fail: bool,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let signal_slot = signals::install()?;
        let pr = resolve_pr(self.pr.as_deref(), self.repo.as_deref())?;
        eprintln!(
            "watching {}/{} #{} (head {})",
            pr.owner, pr.repo, pr.number, &pr.head_sha[..7.min(pr.head_sha.len())],
        );

        let tty = io::stderr().is_terminal();
        let mut last_line_len: usize = 0;

        let summary: Summary = loop {
            if signal_slot.load(Ordering::SeqCst) != 0 {
                if tty && last_line_len > 0 {
                    eprintln!();
                }
                eprintln!("interrupted");
                std::process::exit(signals::exit_code(signal_slot.load(Ordering::SeqCst)));
            }

            let summary = fetch_check_runs(&pr)?;
            let line = format!(
                "total={} completed={} success={} failure={} in_progress={} pending={} skipped={}",
                summary.total,
                summary.completed,
                summary.success,
                summary.failure,
                summary.in_progress,
                summary.pending,
                summary.skipped,
            );
            if tty {
                eprint!("\r\x1b[K{line}");
                io::stderr().flush().ok();
                last_line_len = line.len();
            } else {
                eprintln!("{line}");
            }

            if summary.is_done() {
                if tty && last_line_len > 0 {
                    eprintln!();
                }
                break summary;
            }
            for _ in 0..self.interval {
                if signal_slot.load(Ordering::SeqCst) != 0 {
                    break;
                }
                thread::sleep(Duration::from_secs(1));
            }
        };

        let outcome = summary.outcome();
        let pr_url = format!("https://github.com/{}/{}/pull/{}", pr.owner, pr.repo, pr.number);

        if self.json {
            let body = serde_json::json!({
                "owner": pr.owner,
                "repo": pr.repo,
                "number": pr.number,
                "head_sha": pr.head_sha,
                "url": pr_url,
                "outcome": outcome.as_str(),
                "summary": summary.as_json(),
            });
            println!("{}", serde_json::to_string_pretty(&body)?);
        } else {
            let glyph = match outcome {
                Outcome::Success => "✅",
                Outcome::Failure => "❌",
                Outcome::Indeterminate => "⚠️ ",
            };
            println!(
                "{} {} #{} — Success: {} / Failure: {} / Skipped: {}",
                glyph,
                outcome.as_str(),
                pr.number,
                summary.success,
                summary.failure,
                summary.skipped,
            );
        }

        if !self.no_notify {
            send_notify(outcome, &pr, &pr_url, &summary).ok();
        }

        if self.exit_on_fail && matches!(outcome, Outcome::Failure | Outcome::Indeterminate) {
            std::process::exit(1);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Pr {
    owner: String,
    repo: String,
    number: u64,
    head_sha: String,
}

fn resolve_pr(arg: Option<&str>, repo_override: Option<&str>) -> Result<Pr> {
    let (owner, repo, number) = match arg {
        Some(a) => {
            if let Some((o, r, n)) = parse_pr_url(a) {
                (o, r, n)
            } else if let Ok(n) = a.parse::<u64>() {
                let (o, r) = resolve_repo(repo_override)?;
                (o, r, n)
            } else {
                bail!("invalid PR identifier: {a}");
            }
        }
        None => {
            let (o, r) = resolve_repo(repo_override)?;
            let json = gh_capture(&[
                "pr",
                "view",
                "--json",
                "number,headRefOid",
            ])?;
            #[derive(Deserialize)]
            struct V {
                number: u64,
                #[serde(rename = "headRefOid")]
                head_ref_oid: String,
            }
            let v: V = serde_json::from_str(&json).context("failed to parse `gh pr view` JSON")?;
            return Ok(Pr {
                owner: o,
                repo: r,
                number: v.number,
                head_sha: v.head_ref_oid,
            });
        }
    };
    let json = gh_capture(&[
        "pr",
        "view",
        &number.to_string(),
        "--repo",
        &format!("{owner}/{repo}"),
        "--json",
        "headRefOid",
    ])?;
    #[derive(Deserialize)]
    struct V {
        #[serde(rename = "headRefOid")]
        head_ref_oid: String,
    }
    let v: V = serde_json::from_str(&json).context("failed to parse `gh pr view` JSON")?;
    Ok(Pr {
        owner,
        repo,
        number,
        head_sha: v.head_ref_oid,
    })
}

fn parse_pr_url(s: &str) -> Option<(String, String, u64)> {
    let stripped = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))?;
    let mut it = stripped.split('/');
    let owner = it.next()?.to_string();
    let repo = it.next()?.to_string();
    if it.next()? != "pull" {
        return None;
    }
    let number: u64 = it.next()?.parse().ok()?;
    Some((owner, repo, number))
}

fn resolve_repo(repo_override: Option<&str>) -> Result<(String, String)> {
    if let Some(s) = repo_override {
        let (o, r) = s
            .split_once('/')
            .ok_or_else(|| anyhow!("--repo must be OWNER/REPO"))?;
        return Ok((o.to_string(), r.to_string()));
    }
    let json = gh_capture(&[
        "repo",
        "view",
        "--json",
        "owner,name",
    ])?;
    #[derive(Deserialize)]
    struct V {
        owner: Owner,
        name: String,
    }
    #[derive(Deserialize)]
    struct Owner {
        login: String,
    }
    let v: V = serde_json::from_str(&json).context("failed to parse `gh repo view` JSON")?;
    Ok((v.owner.login, v.name))
}

#[derive(Debug, Clone, Copy)]
enum Outcome {
    Success,
    Failure,
    Indeterminate,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::Failure => "failure",
            Outcome::Indeterminate => "indeterminate",
        }
    }
}

#[derive(Debug, Default)]
struct Summary {
    total: u32,
    completed: u32,
    success: u32,
    failure: u32,
    in_progress: u32,
    pending: u32,
    skipped: u32,
}

impl Summary {
    fn is_done(&self) -> bool {
        self.total > 0 && self.completed + self.skipped == self.total
    }
    fn outcome(&self) -> Outcome {
        if self.failure > 0 {
            Outcome::Failure
        } else if self.success > 0 {
            Outcome::Success
        } else {
            Outcome::Indeterminate
        }
    }
    fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "total": self.total,
            "completed": self.completed,
            "success": self.success,
            "failure": self.failure,
            "in_progress": self.in_progress,
            "pending": self.pending,
            "skipped": self.skipped,
        })
    }
}

#[derive(Deserialize)]
struct CheckRuns {
    total_count: u32,
    check_runs: Vec<Run1>,
}

#[derive(Deserialize)]
struct Run1 {
    status: String,
    conclusion: Option<String>,
}

fn fetch_check_runs(pr: &Pr) -> Result<Summary> {
    let json = gh_capture(&[
        "api",
        &format!(
            "repos/{}/{}/commits/{}/check-runs?per_page=100",
            pr.owner, pr.repo, pr.head_sha
        ),
    ])?;
    let body: CheckRuns =
        serde_json::from_str(&json).context("failed to parse check-runs JSON")?;
    let mut s = Summary {
        total: body.total_count,
        ..Default::default()
    };
    for r in body.check_runs {
        match r.status.as_str() {
            "completed" => {
                s.completed += 1;
                match r.conclusion.as_deref() {
                    Some("success") | Some("neutral") => s.success += 1,
                    Some("failure") | Some("timed_out") | Some("cancelled") | Some("action_required") | Some("startup_failure") | Some("stale") => s.failure += 1,
                    Some("skipped") => s.skipped += 1,
                    _ => {}
                }
            }
            "in_progress" => s.in_progress += 1,
            "queued" | "requested" | "pending" | "waiting" => s.pending += 1,
            _ => {}
        }
    }
    Ok(s)
}

fn gh_capture(args: &[&str]) -> Result<String> {
    let out = Command::new("gh")
        .args(args)
        .output()
        .context("failed to spawn `gh`")?;
    if !out.status.success() {
        bail!(
            "`gh {}` failed (status {:?}): {}",
            args.join(" "),
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn send_notify(outcome: Outcome, pr: &Pr, url: &str, s: &Summary) -> Result<()> {
    let title = format!("PR #{} {}", pr.number, outcome.as_str());
    let msg = format!(
        "Success: {} / Failure: {} / Skipped: {}",
        s.success, s.failure, s.skipped
    );
    if proc::find_in_path("terminal-notifier").is_some() {
        Command::new("terminal-notifier")
            .args([
                "-title",
                &title,
                "-message",
                &msg,
                "-open",
                url,
            ])
            .status()
            .ok();
    } else if proc::find_in_path("notify-send").is_some() {
        Command::new("notify-send").args([&title, &msg]).status().ok();
    }
    Ok(())
}
