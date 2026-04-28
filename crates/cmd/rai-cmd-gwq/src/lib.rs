//! `rai gwq` — gwq worktree のお掃除サブコマンド群。
//!
//! 仕様: `docs/specs/10-gwq-clean.md` 参照。

pub mod clean;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    sub: GwqCmd,
}

#[derive(Debug, Subcommand)]
enum GwqCmd {
    /// マージ済み / リモート消失 / dirty な worktree を fzf で選んで掃除する。
    Clean(clean::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            GwqCmd::Clean(c) => c.run(ctx),
        }
    }
}
