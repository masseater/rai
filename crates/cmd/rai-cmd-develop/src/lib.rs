//! `rai develop` — Issue / PR を起点に worktree + tmux + agent CLI を起動する。
//!
//! 仕様: `docs/specs/18-develop.md` を参照。

pub mod common;
pub mod finalize;
pub mod gh_pr;
pub mod issue;
pub mod pr;
pub mod resume;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub struct Cmd {
    #[command(subcommand)]
    sub: DevelopCmd,
}

#[derive(Debug, Subcommand)]
enum DevelopCmd {
    /// Issue から worktree + tmux + agent CLI を一気通貫で起動する。
    Issue(issue::Cmd),
    /// PR の worktree に入り、コンフリクトや CI 失敗を agent CLI に修復させる。
    Pr(pr::Cmd),
    /// 既存 worktree を保持したまま、agent セッションを再開する。
    Resume(resume::Cmd),
    /// Internal post-agent publish hook for `rai develop`.
    #[command(name = "finalize-agent", hide = true)]
    FinalizeAgent(finalize::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            DevelopCmd::Issue(c) => c.run(ctx),
            DevelopCmd::Pr(c) => c.run(ctx),
            DevelopCmd::Resume(c) => c.run(ctx),
            DevelopCmd::FinalizeAgent(c) => c.run(ctx),
        }
    }
}
