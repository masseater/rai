//! `rai ccs` — `ccs` CLI と連携するサブコマンド群。
//!
//! 仕様: `docs/specs/23-ccs-usage.md` 参照。

pub mod usage;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub struct Cmd {
    #[command(subcommand)]
    sub: CcsCmd,
}

#[derive(Debug, Subcommand)]
enum CcsCmd {
    /// ccs 全 Claude プロファイルの 5h / 7d レートリミット残量サマリを表示する。
    Usage(usage::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            CcsCmd::Usage(c) => c.run(ctx),
        }
    }
}
