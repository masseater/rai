//! Subcommand abstraction.
//!
//! Every subcommand crate exposes a `clap::Args`-derived struct and
//! implements [`Run`] so the top-level `rai` binary can stay a thin
//! dispatcher.

use crate::{Ctx, Result};

/// Trait that every concrete subcommand implements.
pub trait Run {
    fn run(self, ctx: &Ctx) -> Result<()>;
}
