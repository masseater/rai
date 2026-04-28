//! `rai git` — 開発支援用の git サブコマンド群。
//!
//! 仕様: `docs/specs/06-git-autopull.md`, `docs/specs/07-git-track-mine.md` 参照。

pub mod autopull;
pub mod git;
pub mod track_mine;

use clap::{Args, Subcommand};
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub struct Cmd {
    #[command(subcommand)]
    sub: GitCmd,
}

#[derive(Debug, Subcommand)]
enum GitCmd {
    /// upstream を間欠 fetch して fast-forward だけで pull する。
    Autopull(autopull::Cmd),
    /// 自分が author の最近のブランチを表示し、選択して checkout する。
    #[command(name = "track-mine")]
    TrackMine(track_mine::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            GitCmd::Autopull(c) => c.run(ctx),
            GitCmd::TrackMine(c) => c.run(ctx),
        }
    }
}
