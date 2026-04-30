//! `rai git autopull` — upstream を間欠 fetch + fast-forward pull。

use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context};
use clap::Args;
use rai_core::{cli::Run, shell, signals, ts, Ctx, Result};

use crate::git;

#[derive(Debug, Args)]
pub struct Cmd {
    /// fetch 間隔 (秒)。
    #[arg(long, default_value_t = 30)]
    interval: u64,

    /// 対象 remote (未指定なら upstream の remote)。
    #[arg(long)]
    remote: Option<String>,

    /// 対象ブランチ (未指定なら現在のブランチ)。
    #[arg(long)]
    branch: Option<String>,

    /// 1 サイクルだけ実行して終了 (cron 用)。
    #[arg(long)]
    once: bool,

    /// pull 成功直後に実行するコマンド (`sh -c` 経由)。
    #[arg(long, value_name = "CMD")]
    on_update: Option<String>,

    /// 検出のみで pull はしない。
    #[arg(long)]
    no_fast_forward: bool,

    /// pull 失敗時に exit 1 する。
    #[arg(long)]
    strict: bool,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let signal_slot = signals::install()?;

        let branch = match &self.branch {
            Some(b) => b.clone(),
            None => git::current_branch().context("failed to detect current branch")?,
        };
        let upstream_ref = format!("{branch}@{{u}}");
        let upstream = match git::rev_parse_args(&["--abbrev-ref", &upstream_ref]) {
            Ok(s) if !s.is_empty() => s,
            _ => bail!("`{branch}` has no upstream. Set with `git branch --set-upstream-to=...`"),
        };

        let (default_remote, default_remote_branch) = upstream
            .split_once('/')
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .unwrap_or_else(|| ("origin".to_string(), branch.clone()));
        let remote = self.remote.clone().unwrap_or(default_remote);
        let remote_branch = default_remote_branch;

        ts::println(format!(
            "autopull start branch={branch} upstream={upstream} interval={}s once={} ff={}",
            self.interval, self.once, !self.no_fast_forward,
        ));

        loop {
            if signal_slot.load(Ordering::SeqCst) != 0 {
                ts::println("signal received, exiting");
                break;
            }

            if let Err(e) = run_cycle(&self, &branch, &remote, &remote_branch, &upstream) {
                ts::println(format!("cycle error: {e}"));
                if self.strict {
                    return Err(e);
                }
            }

            if self.once {
                break;
            }

            for _ in 0..self.interval {
                if signal_slot.load(Ordering::SeqCst) != 0 {
                    break;
                }
                thread::sleep(Duration::from_secs(1));
            }
        }
        Ok(())
    }
}

fn run_cycle(
    cmd: &Cmd,
    branch: &str,
    remote: &str,
    remote_branch: &str,
    upstream: &str,
) -> Result<()> {
    let fetch = shell::user_shell_argv(&["git", "fetch", "--quiet", remote, remote_branch])
        .status()
        .context("failed to spawn git fetch")?;
    if !fetch.success() {
        bail!("git fetch {remote} {remote_branch} failed");
    }

    let local = git::rev_parse("HEAD")?;
    let upstream_sha = git::rev_parse(upstream)?;

    if local == upstream_sha {
        return Ok(());
    }

    ts::println(format!(
        "diverged: HEAD={} upstream={}",
        short(&local),
        short(&upstream_sha)
    ));

    if cmd.no_fast_forward {
        return Ok(());
    }

    let pull = shell::user_shell_argv(&["git", "pull", "--ff-only", remote, remote_branch])
        .status()
        .context("failed to spawn git pull")?;
    if !pull.success() {
        bail!("git pull --ff-only failed");
    }
    let after = git::rev_parse("HEAD")?;
    ts::println(format!(
        "fast-forwarded {} -> {} on {branch}",
        short(&local),
        short(&after)
    ));

    if let Some(hook) = &cmd.on_update {
        let status = shell::user_shell_command(hook)
            .status()
            .context("failed to spawn --on-update command")?;
        if !status.success() {
            bail!("--on-update command exited with {:?}", status.code());
        }
    }
    Ok(())
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}
