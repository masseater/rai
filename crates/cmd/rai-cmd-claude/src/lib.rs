//! `rai claude` — Claude Code (`claude` CLI) と連携するサブコマンド群。
//!
//! 仕様: `docs/specs/04-claude-format.md` / `docs/specs/21-claude-print.md` /
//! `docs/specs/22-claude-pair.md` / `docs/specs/23-claude-usage.md` 参照。

pub mod format;
pub mod pair;
pub mod print;
pub mod usage;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub struct Cmd {
    #[command(subcommand)]
    sub: ClaudeCmd,
}

#[derive(Debug, Subcommand)]
enum ClaudeCmd {
    /// `claude --output-format stream-json --verbose` の NDJSON を整形して表示する。
    Format(format::Cmd),
    /// `claude --print` を session-id 単位で「初回 → 継続」自動切替で呼ぶラッパー。
    Print(print::Cmd),
    /// 2 つのプロンプトを交互に `claude --print` で回す pair ループ。
    Pair(pair::Cmd),
    /// ccs 全 Claude プロファイルの 5h / 7d レートリミット残量サマリを表示する。
    Usage(usage::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            ClaudeCmd::Format(c) => c.run(ctx),
            ClaudeCmd::Print(c) => c.run(ctx),
            ClaudeCmd::Pair(c) => c.run(ctx),
            ClaudeCmd::Usage(c) => c.run(ctx),
        }
    }
}
