//! `rai pr` — GitHub Pull Request 関連サブコマンド群。
//!
//! 仕様: `docs/specs/08-pr-wait.md` 参照。

pub mod wait;
pub mod watch_loop;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub struct Cmd {
    #[command(subcommand)]
    sub: PrCmd,
}

#[derive(Debug, Subcommand)]
enum PrCmd {
    /// PR の CI (check-runs) 完了まで polling し、終了時に通知する。
    Wait(wait::Cmd),
    /// 複数 PR を親 watcher で監視し、更新時に `rai develop pr` を起動する。
    #[command(name = "watch-loop")]
    WatchLoop(watch_loop::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            PrCmd::Wait(c) => c.run(ctx),
            PrCmd::WatchLoop(c) => c.run(ctx),
        }
    }
}
