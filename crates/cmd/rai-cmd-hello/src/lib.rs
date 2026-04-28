//! `rai hello` — sample subcommand.
//!
//! Use this crate as the template when adding a new subcommand:
//! 1. `cp -r crates/cmd/rai-cmd-hello crates/cmd/rai-cmd-<name>`
//! 2. Rename the package and types, implement [`rai_core::cli::Run`].
//! 3. Add the crate to `[workspace.dependencies]` in the root `Cargo.toml`.
//! 4. Wire it up in `crates/rai/src/main.rs` (one enum variant + one match arm).

use clap::Args;
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
pub struct Cmd {
    /// Name to greet.
    #[arg(default_value = "world")]
    name: String,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        println!("hello, {}!", self.name);
        Ok(())
    }
}
