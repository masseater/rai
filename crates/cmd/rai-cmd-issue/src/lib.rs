//! `rai issue` — GitHub Issue を起点にした開発支援サブコマンド群。
//!
//! 仕様: `docs/specs/09-issue-develop.md`, `docs/specs/13-issue-inventory.md` 参照。

pub mod develop;
pub mod inventory;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

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
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            IssueCmd::Develop(c) => c.run(ctx),
            IssueCmd::FinalizeAgent(c) => c.run(ctx),
            IssueCmd::Inventory(c) => c.run(ctx),
        }
    }
}
