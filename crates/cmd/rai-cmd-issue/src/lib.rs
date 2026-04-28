//! `rai issue` — GitHub Issue を起点に worktree + tmux + agent を起動する。
//!
//! 仕様: `docs/specs/09-issue-fix.md` 参照。

pub mod fix;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    sub: IssueCmd,
}

#[derive(Debug, Subcommand)]
enum IssueCmd {
    /// Issue から worktree + tmux + agent CLI を一気通貫で起動する。
    Fix(fix::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            IssueCmd::Fix(c) => c.run(ctx),
        }
    }
}
