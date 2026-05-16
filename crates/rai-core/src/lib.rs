//! `rai-core` provides the shared foundation that every `rai` subcommand
//! crate is expected to build on top of.

pub mod claude;
pub mod cli;
pub mod logging;
pub mod panic_hook;
pub mod proc;
pub mod shell;
pub mod signals;
pub mod term;
pub mod ts;

pub use anyhow::{Context, Result};

/// Common runtime context handed to every subcommand.
///
/// New global configuration (HTTP client, config file, cache dir, ...) should
/// land here so individual subcommands stay decoupled from each other.
#[derive(Debug, Clone, Default)]
pub struct Ctx {}

impl Ctx {
    pub fn new() -> Self {
        Self::default()
    }
}
