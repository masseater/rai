//! `rai issue` — GitHub Issue を起点にした開発支援サブコマンド群。
//!
//! 仕様: `docs/specs/09-issue-develop.md`, `docs/specs/13-issue-inventory.md`,
//! `docs/specs/15-issue-triage.md` 参照。

pub mod develop;
pub mod inventory;
pub mod triage;

use anyhow::{bail, Context};
use clap::{Args, Subcommand};
use rai_core::{cli::Run, shell, Ctx, Result};
use serde::Deserialize;

#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub struct Cmd {
    #[command(subcommand)]
    sub: IssueCmd,
}

#[derive(Debug, Subcommand)]
enum IssueCmd {
    /// Issue から worktree + tmux + agent CLI を一気通貫で起動する。
    Develop(develop::Cmd),
    /// Internal post-agent publish hook for `rai issue develop`.
    #[command(name = "finalize-agent", hide = true)]
    FinalizeAgent(develop::FinalizeCmd),
    /// Issue 一覧を取得し、固定 prompt で AI engine に棚卸しさせる。
    Inventory(inventory::Cmd),
    /// triage ラベル付き Issue を 1 件ずつレビューし、close/keep を判断する。
    Triage(triage::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            IssueCmd::Develop(c) => c.run(ctx),
            IssueCmd::FinalizeAgent(c) => c.run(ctx),
            IssueCmd::Inventory(c) => c.run(ctx),
            IssueCmd::Triage(c) => c.run(ctx),
        }
    }
}

pub(crate) fn resolve_repo(repo_override: Option<&str>) -> Result<String> {
    if let Some(repo) = repo_override {
        validate_repo(repo)?;
        return Ok(repo.to_string());
    }

    let json = gh_capture(&["repo", "view", "--json", "nameWithOwner"])?;
    #[derive(Deserialize)]
    struct RepoView {
        #[serde(rename = "nameWithOwner")]
        name_with_owner: String,
    }
    let view: RepoView =
        serde_json::from_str(&json).context("failed to parse `gh repo view` JSON")?;
    validate_repo(&view.name_with_owner)?;
    Ok(view.name_with_owner)
}

pub(crate) fn validate_repo(repo: &str) -> Result<()> {
    let Some((owner, name)) = repo.split_once('/') else {
        bail!("repo must be OWNER/REPO");
    };
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        bail!("repo must be OWNER/REPO");
    }
    Ok(())
}

pub(crate) fn gh_capture(args: &[&str]) -> Result<String> {
    let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 1);
    argv.push("gh");
    argv.extend_from_slice(args);
    let out = shell::user_shell_argv(&argv)
        .output()
        .context("failed to spawn `gh` via user shell")?;
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
