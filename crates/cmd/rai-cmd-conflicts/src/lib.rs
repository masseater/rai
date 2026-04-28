//! `rai conflicts` — CONFLICTING な PR を agent CLI で自動解消する長時間バッチ。
//!
//! 仕様: `docs/specs/11-conflicts.md` 参照。

pub mod queue;
pub mod reset;
pub mod run;
pub mod status;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    sub: ConflictsCmd,
}

#[derive(Debug, Subcommand)]
enum ConflictsCmd {
    /// CONFLICTING な PR を agent CLI で自動解消する。
    Run(run::Cmd),
    /// queue.json の現在状態を表示する。
    Status(status::Cmd),
    /// failed な entry を pending に戻す。
    #[command(name = "reset-failed")]
    ResetFailed(reset::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            ConflictsCmd::Run(c) => c.run(ctx),
            ConflictsCmd::Status(c) => c.run(ctx),
            ConflictsCmd::ResetFailed(c) => c.run(ctx),
        }
    }
}
