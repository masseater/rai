//! `rai dev` — ghq/gwq + fzf 系。
//!
//! 仕様: `docs/specs/05-dev.md` 参照。

pub mod pick;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    sub: DevCmd,
}

#[derive(Debug, Subcommand)]
enum DevCmd {
    /// ghq + gwq の候補から fzf で 1 つ選び、フルパスを stdout に出す。
    Pick(pick::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            DevCmd::Pick(c) => c.run(ctx),
        }
    }
}
