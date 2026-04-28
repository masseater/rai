//! `rai date` — fish 関数 `mydate` 互換の薄い日付フォーマッタ。
//!
//! 仕様: `docs/specs/02-date.md` 参照。

use chrono::{Local, SecondsFormat, Utc};
use clap::Args;
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
pub struct Cmd {
    /// `YYYYMMDD-HHMMSS` 形式で出力。
    #[arg(long, conflicts_with_all = ["iso", "epoch"])]
    time: bool,

    /// ISO-8601 形式で出力 (例 2026-04-28T13:24:55+09:00)。
    #[arg(long, conflicts_with_all = ["time", "epoch"])]
    iso: bool,

    /// UNIX epoch 秒で出力。
    #[arg(long, conflicts_with_all = ["time", "iso"])]
    epoch: bool,

    /// タイムゾーンを UTC に固定する (既定はシステムローカル / `TZ` 尊重)。
    #[arg(long)]
    utc: bool,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let s = if self.utc {
            format_with(Utc::now(), self.kind())
        } else {
            format_with(Local::now(), self.kind())
        };
        println!("{s}");
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Date,
    Time,
    Iso,
    Epoch,
}

impl Cmd {
    fn kind(&self) -> Kind {
        if self.time {
            Kind::Time
        } else if self.iso {
            Kind::Iso
        } else if self.epoch {
            Kind::Epoch
        } else {
            Kind::Date
        }
    }
}

fn format_with<Tz>(now: chrono::DateTime<Tz>, kind: Kind) -> String
where
    Tz: chrono::TimeZone,
    Tz::Offset: std::fmt::Display,
{
    match kind {
        Kind::Date => now.format("%Y%m%d").to_string(),
        Kind::Time => now.format("%Y%m%d-%H%M%S").to_string(),
        Kind::Iso => now.to_rfc3339_opts(SecondsFormat::Secs, false),
        Kind::Epoch => now.timestamp().to_string(),
    }
}
