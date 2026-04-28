# crates/cmd Development Guide

## Overview

This directory holds one Cargo crate per `rai <subcommand>`. Each crate is a
library (no `[[bin]]`) that exposes a `clap::Args` struct named `Cmd` and
implements `rai_core::cli::Run`. The top-level `rai` binary dispatches by
embedding these `Cmd` types as variants of its own enum.

`rai-cmd-hello/` is the canonical template — copy it when adding a command.

## Structure

| File         | Description                                                |
| ------------ | ---------------------------------------------------------- |
| `Cargo.toml` | Inherits version/edition/license via `*.workspace = true`. |
| `src/lib.rs` | Defines `pub struct Cmd` (`#[derive(Args)]`) + `impl Run`. |

## Adding a New Subcommand

1. Write the spec first: `docs/specs/NN-<name>.md` (see root `AGENTS.md`).
2. Copy the template:
   ```sh
   cp -r crates/cmd/rai-cmd-hello crates/cmd/rai-cmd-<name>
   ```
3. In `crates/cmd/rai-cmd-<name>/Cargo.toml`, set `name = "rai-cmd-<name>"`
   and `description = "..."`. Replace deps with what the command actually uses.
4. In `src/lib.rs`, rename the type, define the `Args` fields, and implement
   `Run::run` for the command's behavior.
5. Add the crate to the root `Cargo.toml` `[workspace.dependencies]`:
   ```toml
   rai-cmd-<name> = { path = "crates/cmd/rai-cmd-<name>", version = "0.1.0" }
   ```
6. Add `rai-cmd-<name>.workspace = true` to `crates/rai/Cargo.toml`.
7. In `crates/rai/src/main.rs`, add one variant to `enum Cmd` and one arm to
   `impl Run for Cmd`.

## Conventions

- The exported `Args`-derived struct **must** be named `Cmd`.
- Subcommands depend on `rai-core` for `Run`, `Ctx`, `Result`, and shared
  helpers (`logging`, `term`, `signals`, `proc`, `ts`, `panic_hook`).
- Never depend on another `rai-cmd-*` crate. If logic is shared, lift it
  into `rai-core`.
- Shared third-party deps come from the workspace via
  `<dep>.workspace = true`. Add new shared deps in the root `Cargo.toml`.
- Crate `description` doubles as the help line shown by `clap` — keep it
  short and accurate.

## Development Commands

```sh
cargo build -p rai-cmd-<name>
cargo test  -p rai-cmd-<name>
cargo run   -p rai -- <name> [args]
```

> Development commands last updated: 2026-04-28
