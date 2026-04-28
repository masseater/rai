# rai-core Development Guide

## Overview

`rai-core` is the shared foundation every `rai-cmd-*` subcommand crate depends
on. It owns the dispatch trait, runtime context, logging, and a small set of
runtime utilities (terminal, signals, process, timestamp, panic hook).

Cross-cutting concerns belong here. Anything specific to a single subcommand
stays in that subcommand's crate.

## Structure

| File              | Description                                                       |
| ----------------- | ----------------------------------------------------------------- |
| `src/lib.rs`      | Re-exports `anyhow::{Context, Result}` and defines `Ctx`.         |
| `src/cli.rs`      | The `Run` trait every subcommand implements.                      |
| `src/logging.rs`  | `tracing-subscriber` initializer; honours `-v` and `RAI_LOG`.     |
| `src/term.rs`     | Terminal helpers (raw mode, alt-screen, cursor — via `crossterm`). |
| `src/signals.rs`  | Signal-hook based interrupt handling.                              |
| `src/proc.rs`     | Child process spawning helpers (`libc`-aware).                     |
| `src/ts.rs`       | Timestamp helpers (`chrono`).                                      |
| `src/panic_hook.rs` | Panic hook that restores the terminal before printing the panic. |

## API / Exports

| Symbol                  | Purpose                                                  |
| ----------------------- | -------------------------------------------------------- |
| `Run` (trait)           | `fn run(self, ctx: &Ctx) -> Result<()>`. Every `Cmd` impls it. |
| `Ctx`                   | Common runtime context. Add global config fields here.   |
| `Result`, `Context`     | Re-exported from `anyhow` for ergonomic error handling.  |
| `logging::init`         | Initialize tracing; called once from `crates/rai/src/main.rs`. |

## Notes

- New global configuration (HTTP client, config dir, cache dir, …) goes onto
  `Ctx`. Subcommands stay decoupled from each other by depending only on this.
- Keep `rai-core` free of subcommand-specific logic. If only one subcommand
  uses something, leave it in that subcommand.
- Public items must remain stable for all `rai-cmd-*` crates simultaneously —
  treat changes as a workspace-wide refactor, not a single-crate change.

## Development Commands

```sh
cargo build -p rai-core
cargo test  -p rai-core
```

> Development commands last updated: 2026-04-28
