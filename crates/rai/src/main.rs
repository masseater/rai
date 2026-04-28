//! `rai` — top-level CLI entry point.
//!
//! Keep this file thin: it should only parse args, set up the runtime
//! context, and dispatch to a subcommand crate.

use clap::{Parser, Subcommand};
use rai_core::{cli::Run, logging, Ctx, Result};

#[derive(Debug, Parser)]
#[command(name = "rai", version, about, long_about = None, propagate_version = true)]
struct Cli {
    /// Increase log verbosity (info → debug). `RAI_LOG` overrides this.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

/// All subcommands. Add a new variant per crate under `crates/cmd/`.
#[derive(Debug, Subcommand)]
enum Cmd {
    /// Sample subcommand.
    Hello(rai_cmd_hello::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self {
            Cmd::Hello(c) => c.run(ctx),
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    logging::init(cli.verbose);
    let ctx = Ctx::new();
    cli.cmd.run(&ctx)
}
