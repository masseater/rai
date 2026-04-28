//! `rai completion` — emit a shell completion script to stdout.
//!
//! The completion definition is generated from the top-level `clap::Command`
//! of the `rai` binary, so it always reflects the current set of subcommands
//! and options without any hand-maintained completion table.

use clap::{Args, Command};
use clap_complete::{generate, Shell};
use rai_core::Result;
use std::io;

#[derive(Debug, Args)]
pub struct Cmd {
    /// Target shell.
    #[arg(value_enum)]
    shell: Shell,
}

impl Cmd {
    /// Write the completion script for the requested shell to stdout.
    ///
    /// `cmd` must be the top-level `clap::Command` of the binary (typically
    /// `Cli::command()` from `crates/rai/src/main.rs`).
    pub fn print(self, cmd: &mut Command) -> Result<()> {
        let bin = cmd.get_name().to_string();
        generate(self.shell, cmd, bin, &mut io::stdout());
        Ok(())
    }
}
