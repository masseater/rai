//! `rai gh` — GitHub CLI (`gh`) と連携するサブコマンド群。
//!
//! 仕様: `docs/specs/03-gh-rate-limit.md` 参照。

pub mod rate_limit;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub struct Cmd {
    #[command(subcommand)]
    sub: GhCmd,
}

#[derive(Debug, Subcommand)]
enum GhCmd {
    /// GitHub API のレートリミット残量と reset 時刻を表示する。
    #[command(name = "rate-limit")]
    RateLimit(rate_limit::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            GhCmd::RateLimit(c) => c.run(ctx),
        }
    }
}
