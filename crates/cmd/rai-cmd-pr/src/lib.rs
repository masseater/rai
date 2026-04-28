//! `rai pr` — GitHub Pull Request 関連サブコマンド群。
//!
//! 仕様: `docs/specs/08-pr-wait.md` 参照。

pub mod wait;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    sub: PrCmd,
}

#[derive(Debug, Subcommand)]
enum PrCmd {
    /// PR の CI (check-runs) 完了まで polling し、終了時に通知する。
    Wait(wait::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            PrCmd::Wait(c) => c.run(ctx),
        }
    }
}
