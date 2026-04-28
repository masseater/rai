//! `rai conflicts reset-failed` — failed な entry を pending に戻す。

use std::path::PathBuf;

use clap::Args;
use rai_core::{cli::Run, Ctx, Result};

use crate::queue::{self, Paths};

#[derive(Debug, Args)]
pub struct Cmd {
    #[arg(long, value_name = "PATH")]
    state_dir: Option<PathBuf>,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let paths = Paths::new(self.state_dir, None);
        paths.ensure_dirs()?;
        let _lock = queue::Lock::try_acquire(&paths.lock_file())?;
        let mut q = queue::load(&paths.queue_json())?;
        let mut count = 0u32;
        for e in q.entries.values_mut() {
            if e.status == "failed" {
                e.status = "pending".into();
                e.error.clear();
                e.attempts = 0;
                e.updated_at = queue::now_iso();
                count += 1;
            }
        }
        queue::save(&paths.queue_json(), &mut q)?;
        eprintln!("reset {count} failed entries to pending");
        Ok(())
    }
}
