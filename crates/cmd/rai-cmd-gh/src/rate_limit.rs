//! `rai gh rate-limit` — `gh api rate_limit` をパースして人間/機械両用に整形する。

use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use chrono::{DateTime, Local, TimeZone, Utc};
use clap::{Args, ValueEnum};
use rai_core::{cli::Run, Ctx, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Args)]
pub struct Cmd {
    /// core / search / graphql の 3 リソースをまとめて表示。
    #[arg(long)]
    all: bool,

    /// JSON で出力 (機械可読)。
    #[arg(long, conflicts_with = "watch")]
    json: bool,

    /// 表示タイムゾーン。
    #[arg(long, value_enum, default_value_t = Tz::Local)]
    tz: Tz,

    /// 監視モード: 指定秒間隔で再描画。
    #[arg(long, value_name = "SEC")]
    watch: Option<u64>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Tz {
    Local,
    Utc,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        match self.watch {
            Some(sec) if sec > 0 => loop {
                let snapshot = fetch()?;
                print!("\x1b[2J\x1b[H");
                self.render(&snapshot)?;
                thread::sleep(Duration::from_secs(sec));
            },
            _ => {
                let snapshot = fetch()?;
                self.render(&snapshot)?;
                Ok(())
            }
        }
    }
}

impl Cmd {
    fn render(&self, snap: &Snapshot) -> Result<()> {
        let resources = if self.all {
            vec!["core", "search", "graphql"]
        } else {
            vec!["core"]
        };

        if self.json {
            let mut out = serde_json::Map::new();
            out.insert(
                "now".into(),
                serde_json::Value::String(self.now_str(snap.now)),
            );
            for name in &resources {
                let r = snap
                    .resources
                    .get(*name)
                    .ok_or_else(|| anyhow!("rate_limit response missing resource: {name}"))?;
                let reset_dt = Utc.timestamp_opt(r.reset, 0).single().ok_or_else(|| {
                    anyhow!("invalid reset timestamp for {name}: {}", r.reset)
                })?;
                let in_secs = (reset_dt - snap.now).num_seconds().max(0);
                out.insert(
                    (*name).to_string(),
                    serde_json::json!({
                        "limit": r.limit,
                        "remaining": r.remaining,
                        "used": r.used,
                        "reset_epoch": r.reset,
                        "reset": self.now_str(reset_dt),
                        "in_seconds": in_secs,
                    }),
                );
            }
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(());
        }

        for (idx, name) in resources.iter().enumerate() {
            let r = snap
                .resources
                .get(*name)
                .ok_or_else(|| anyhow!("rate_limit response missing resource: {name}"))?;
            let reset_dt = Utc
                .timestamp_opt(r.reset, 0)
                .single()
                .ok_or_else(|| anyhow!("invalid reset timestamp for {name}: {}", r.reset))?;
            let remaining = (reset_dt - snap.now).num_seconds().max(0);
            if idx > 0 {
                println!();
            }
            println!("[{}] {}/{} (used {})", name, r.remaining, r.limit, r.used);
            println!("  Now:   {}", self.now_str(snap.now));
            println!("  Reset: {}", self.now_str(reset_dt));
            println!("  In:    {}", human_duration(remaining));
        }
        Ok(())
    }

    fn now_str(&self, dt: DateTime<Utc>) -> String {
        match self.tz {
            Tz::Utc => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            Tz::Local => dt
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        }
    }
}

#[derive(Debug)]
struct Snapshot {
    now: DateTime<Utc>,
    resources: std::collections::BTreeMap<String, Resource>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Resource {
    limit: u64,
    remaining: u64,
    used: u64,
    reset: i64,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    resources: std::collections::BTreeMap<String, Resource>,
}

fn fetch() -> Result<Snapshot> {
    let output = Command::new("gh")
        .args(["api", "rate_limit"])
        .output()
        .context("failed to spawn `gh`. Is GitHub CLI installed and on PATH?")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("authentication") || stderr.contains("logged in") {
            bail!("gh authentication required. Run `gh auth login` first.\n{stderr}");
        }
        bail!(
            "`gh api rate_limit` failed (status {:?}): {}",
            output.status.code(),
            stderr
        );
    }
    let body: ApiResponse = serde_json::from_slice(&output.stdout)
        .context("failed to parse `gh api rate_limit` JSON")?;
    Ok(Snapshot {
        now: Utc::now(),
        resources: body.resources,
    })
}

fn human_duration(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{h}h {m}min {s}sec")
}
