//! `rai claude` — Claude Code (`claude` CLI) と連携するサブコマンド群。
//!
//! 仕様: `docs/specs/04-claude-format.md` 参照。

pub mod format;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
pub struct Cmd {
    #[command(subcommand)]
    sub: ClaudeCmd,
}

#[derive(Debug, Subcommand)]
enum ClaudeCmd {
    /// `claude --output-format stream-json --verbose` の NDJSON を整形して表示する。
    Format(format::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            ClaudeCmd::Format(c) => c.run(ctx),
        }
    }
}
