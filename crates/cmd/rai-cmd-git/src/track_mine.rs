//! `rai git track-mine` — 自分の open PR の head ブランチをまとめて track する。

use anyhow::{anyhow, bail, Context};
use clap::{Args, ValueEnum};
use rai_core::{cli::Run, shell, Ctx, Result};
use serde::Deserialize;

use crate::git;

#[derive(Debug, Args)]
pub struct Cmd {
    /// 取得対象 PR の作者ログイン (未指定なら `gh api user`)。
    #[arg(long)]
    author: Option<String>,

    /// fetch & track 先 remote。
    #[arg(long, default_value = "origin")]
    remote: String,

    /// PR 取得上限。
    #[arg(long, default_value_t = 200)]
    limit: u32,

    /// PR の state フィルタ。
    #[arg(long, value_enum, default_value_t = State::Open)]
    state: State,

    /// 副作用を起こさず予定だけ表示。
    #[arg(long)]
    dry_run: bool,

    /// JSON サマリで出力。
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum State {
    Open,
    Closed,
    All,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Open => "open",
            State::Closed => "closed",
            State::All => "all",
        }
    }
}

#[derive(Debug, Deserialize)]
struct PrItem {
    #[serde(rename = "headRefName")]
    head_ref_name: String,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let user = match &self.author {
            Some(u) => u.clone(),
            None => gh_capture(&["api", "user", "--jq", ".login"])
                .context("`gh api user` failed. Run `gh auth login`.")?,
        };
        let user = user.trim().to_string();
        if user.is_empty() {
            bail!("could not resolve gh user. Try `gh auth login`.");
        }

        let limit = self.limit.to_string();
        let pr_args = [
            "pr",
            "list",
            "--author",
            &user,
            "--state",
            self.state.as_str(),
            "--limit",
            &limit,
            "--json",
            "headRefName",
        ];
        let pr_json = gh_capture(&pr_args)?;
        let prs: Vec<PrItem> =
            serde_json::from_str(&pr_json).context("failed to parse `gh pr list` JSON")?;

        // fetch first.
        if !self.dry_run {
            let st = shell::user_shell_argv(&["git", "fetch", "--prune", &self.remote])
                .status()
                .context("failed to spawn git fetch")?;
            if !st.success() {
                bail!("git fetch --prune {} failed", self.remote);
            }
        }

        let mut created: Vec<String> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        let mut missing: Vec<String> = Vec::new();

        for pr in &prs {
            let br = pr.head_ref_name.clone();
            // local exists?
            let local_ref = format!("refs/heads/{br}");
            if git::rev_parse_args(&["--verify", "--quiet", &local_ref]).is_ok() {
                if !self.json {
                    eprintln!("skip: already exists locally: {br}");
                }
                skipped.push(br);
                continue;
            }
            // remote ref exists?
            let remote_ref = format!("refs/remotes/{}/{}", self.remote, br);
            if git::rev_parse_args(&["--verify", "--quiet", &remote_ref]).is_err() {
                if !self.json {
                    eprintln!("skip: {}/{} not found", self.remote, br);
                }
                missing.push(br);
                continue;
            }
            if self.dry_run {
                if !self.json {
                    eprintln!("would create: {br} -> {remote_ref}");
                }
                created.push(br);
                continue;
            }
            let st = shell::user_shell_argv(&["git", "branch", "--track", &br, &remote_ref])
                .status()
                .context("failed to spawn git branch")?;
            if !st.success() {
                if !self.json {
                    eprintln!("warn: failed to create {br}");
                }
                continue;
            }
            if !self.json {
                eprintln!("created: {br}");
            }
            created.push(br);
        }

        if self.json {
            let body = serde_json::json!({
                "created": created,
                "skipped": skipped,
                "missing": missing,
                "remote": self.remote,
                "user": user,
                "branches": prs.iter().map(|p| &p.head_ref_name).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&body)?);
        } else {
            println!(
                "created={} skipped={} missing={} remote={} user={}",
                created.len(),
                skipped.len(),
                missing.len(),
                self.remote,
                user,
            );
        }
        Ok(())
    }
}

fn gh_capture(args: &[&str]) -> Result<String> {
    let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 1);
    argv.push("gh");
    argv.extend_from_slice(args);
    let out = shell::user_shell_argv(&argv)
        .output()
        .context("failed to spawn `gh`. Is GitHub CLI installed and on PATH?")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("authentication") || stderr.contains("logged in") {
            return Err(anyhow!(
                "gh authentication required. Run `gh auth login` first.\n{stderr}"
            ));
        }
        return Err(anyhow!(
            "`gh {}` failed (status {:?}): {}",
            args.join(" "),
            out.status.code(),
            stderr
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}
