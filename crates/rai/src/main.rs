//! `rai` — top-level CLI entry point.
//!
//! Keep this file thin: it should only parse args, set up the runtime
//! context, and dispatch to a subcommand crate.

use clap::{CommandFactory, Parser, Subcommand};
use rai_core::{cli::Run, logging, Ctx, Result};

#[derive(Debug, Parser)]
#[command(name = "rai", version, about, long_about = None, propagate_version = true)]
struct Cli {
    /// Increase log verbosity (info → debug). `RAI_LOG` overrides this.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

/// All subcommands. Add a new variant per crate under `crates/cmd/`.
#[derive(Debug, Subcommand)]
enum Cmd {
    /// Sample subcommand.
    Hello(rai_cmd_hello::Cmd),
    /// 2 つのコマンドを交互に回し続けるループ + 下部固定ステータスバー。
    Pair(rai_cmd_pair::Cmd),
    /// fish の `mydate` 互換の薄い日付フォーマッタ。
    Date(rai_cmd_date::Cmd),
    /// GitHub CLI (`gh`) と連携するサブコマンド群。
    Gh(rai_cmd_gh::Cmd),
    /// Claude Code (`claude` CLI) と連携するサブコマンド群。
    Claude(rai_cmd_claude::Cmd),
    /// ghq + gwq + fzf でリポジトリ/worktree を選ぶサブコマンド群。
    Dev(rai_cmd_dev::Cmd),
    /// 開発支援用の git サブコマンド群 (autopull, track-mine)。
    Git(rai_cmd_git::Cmd),
    /// GitHub Pull Request 関連サブコマンド群。
    Pr(rai_cmd_pr::Cmd),
    /// GitHub Issue を起点に worktree + tmux + agent を起動する。
    Issue(rai_cmd_issue::Cmd),
    /// gwq worktree のお掃除サブコマンド群。
    Gwq(rai_cmd_gwq::Cmd),
    /// CONFLICTING な PR を agent CLI で自動解消する長時間バッチ。
    Conflicts(rai_cmd_conflicts::Cmd),
    /// 指定シェル向けの補完スクリプトを stdout に出力する。
    Completion(rai_cmd_completion::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self {
            Cmd::Hello(c) => c.run(ctx),
            Cmd::Pair(c) => c.run(ctx),
            Cmd::Date(c) => c.run(ctx),
            Cmd::Gh(c) => c.run(ctx),
            Cmd::Claude(c) => c.run(ctx),
            Cmd::Dev(c) => c.run(ctx),
            Cmd::Git(c) => c.run(ctx),
            Cmd::Pr(c) => c.run(ctx),
            Cmd::Issue(c) => c.run(ctx),
            Cmd::Gwq(c) => c.run(ctx),
            Cmd::Conflicts(c) => c.run(ctx),
            Cmd::Completion(c) => c.print(&mut Cli::command()),
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    logging::init(cli.verbose);
    let ctx = Ctx::new();
    cli.cmd.run(&ctx)
}
