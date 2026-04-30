# AGENTS.md

Essential guidelines for working on the `rai` repository.

## Project Overview

`rai` is an extensible personal CLI distributed as a single binary. The repository
is a Cargo workspace where each `rai <subcommand>` lives in its own library crate
under `crates/cmd/` so subcommands can be added by editing only one enum variant
in the top-level dispatcher.

## Spec-First Rule (仕様書ファースト)

- Before any implementation work, write a spec under `docs/specs/`.
- Specs describe **what** to build (purpose, user value, deliverables, acceptance
  criteria). They must not describe **how** (implementation plan, internal design,
  library choices).
- Implementation strategy belongs in code, PR descriptions, or design memos —
  not in the spec.
- Implementation without a spec is forbidden. If the spec changes mid-flight,
  update the spec **before** the code.

## Structure

### Top-level Directories

| Directory          | Description                                                         |
| ------------------ | ------------------------------------------------------------------- |
| `crates/rai/`      | Binary crate. Parses args and dispatches to a subcommand crate.     |
| `crates/rai-core/` | Shared foundation: `Run` trait, `Ctx`, logging, term, signals, etc. |
| `crates/cmd/`      | One library crate per subcommand (`rai-cmd-<name>`).                |
| `docs/specs/`      | SSOT specs for each subcommand (numbered `NN-<name>.md`).           |
| `.github/workflows/` | CI (`ci.yml`) and release (`release.yml`) pipelines.              |
| `.agents/`         | Per-task workspaces (out-of-tree notes, not consumed at build).     |

### Top-level Files

| File                   | Description                                                |
| ---------------------- | ---------------------------------------------------------- |
| `Cargo.toml`           | Workspace root: members, shared deps, release profile.     |
| `Cargo.lock`           | Workspace lockfile (committed).                            |
| `rust-toolchain.toml`  | Pinned toolchain.                                          |
| `rustfmt.toml`         | Formatter config.                                          |
| `clippy.toml`          | Lint config.                                               |
| `.editorconfig`        | Editor defaults (4 spaces; 2 for md/yml/toml/json).        |
| `AGENTS.md`            | AI agent guide (this file, SSOT for the repo).             |
| `CLAUDE.md`            | Symlink to `AGENTS.md`.                                    |
| `README.md`            | Human-facing entry point on GitHub.                        |

### Subcommand Crate Conventions (`crates/cmd/rai-cmd-*/`)

| File/Directory     | Description                                                      |
| ------------------ | ---------------------------------------------------------------- |
| `Cargo.toml`       | Inherits workspace settings; declares only crate-local deps.     |
| `src/lib.rs`       | Exposes a `Cmd` struct (`#[derive(Args)]`) implementing `Run`.   |

### Naming Conventions

| Pattern              | Description                                                |
| -------------------- | ---------------------------------------------------------- |
| `rai-cmd-<name>`     | Subcommand crate. Maps to `rai <name>` on the CLI.         |
| `docs/specs/NN-*.md` | Spec file. `NN` is a zero-padded ordinal.                  |
| `CLAUDE.md`          | Always a symlink to `AGENTS.md`. Never standalone content. |

## Development Commands

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p rai -- <subcommand> [args]
```

`RAI_LOG=debug` (or `-v`) raises log verbosity at runtime.

> Development commands last updated: 2026-04-28

## Adding a Subcommand

1. Write the spec first under `docs/specs/NN-<name>.md`.
2. `cp -r crates/cmd/rai-cmd-hello crates/cmd/rai-cmd-<name>` and rename the crate.
3. Register the crate in `[workspace.dependencies]` in the root `Cargo.toml`.
4. Add `rai-cmd-<name>.workspace = true` to `crates/rai/Cargo.toml`.
5. Add one variant to the `Cmd` enum and one match arm in `crates/rai/src/main.rs`.

See `crates/cmd/AGENTS.md` for the full template walkthrough.

## Important Instructions

- The binary is **only** `rai`. Subcommands are library crates — never add a `[[bin]]`.
- Subcommands depend on `rai-core` for the `Run` trait, `Ctx`, and shared utilities.
  Never reach across to another `rai-cmd-*` crate.
- Shared third-party deps live in `[workspace.dependencies]`. Crates declare them
  with `<dep>.workspace = true`.
- `rai-cmd-completion` is a deliberate exception: its dispatch arm in
  `crates/rai/src/main.rs` calls `Cmd::print(&mut Cli::command())` directly so it
  can introspect the top-level clap command tree. Do not generalise this away.
- CI runs `fmt`, `clippy -D warnings`, and `test` on Linux and macOS — keep all
  three green locally before pushing.
- Pursue the ideal state. Backwards-compatibility shims are not a goal of this repo.
- Format any information passed to AI agents as Markdown (AIに渡す情報はmarkdownに整形すること).
