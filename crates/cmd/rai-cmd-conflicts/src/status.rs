//! `rai conflicts status` — queue.json の現在状態を表示する。

use std::path::PathBuf;

use clap::Args;
use rai_core::{cli::Run, Ctx, Result};

use crate::queue::{self, Paths};

#[derive(Debug, Args)]
pub struct Cmd {
    #[arg(long, value_name = "PATH")]
    state_dir: Option<PathBuf>,

    #[arg(long)]
    json: bool,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let paths = Paths::new(self.state_dir, None);
        paths.ensure_dirs()?;
        let q = queue::load(&paths.queue_json())?;
        if self.json {
            println!("{}", serde_json::to_string_pretty(&q)?);
            return Ok(());
        }
        println!("pr\tstatus\tattempts\thead_sha\ttitle");
        for (pr, e) in &q.entries {
            println!(
                "{pr}\t{}\t{}\t{}\t{}",
                e.status,
                e.attempts,
                short(&e.head_sha),
                e.title,
            );
        }
        Ok(())
    }
}

fn short(s: &str) -> &str {
    s.get(..7).unwrap_or(s)
}
