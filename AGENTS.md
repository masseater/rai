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

## Interactive Shell Verification

- Features that require an interactive shell or terminal UI (for example TUI,
  raw-mode key handling, `fzf`, or foreground agent sessions) must be verified
  inside `tmux`. Do not treat a non-interactive command or unit test alone as
  sufficient for those flows.

## Adding a Subcommand

1. Write the spec first under `docs/specs/NN-<name>.md`.
2. `cp -r crates/cmd/rai-cmd-hello crates/cmd/rai-cmd-<name>` and rename the crate.
3. Register the crate in `[workspace.dependencies]` in the root `Cargo.toml`.
4. Add `rai-cmd-<name>.workspace = true` to `crates/rai/Cargo.toml`.
5. Add one variant to the `Cmd` enum and one match arm in `crates/rai/src/main.rs`.

See `crates/cmd/AGENTS.md` for the full template walkthrough.

## External Process Execution (外部プロセス起動ポリシー)

**基本方針: 外部コマンドは必ずユーザーのデフォルトシェル経由で実行する。**

- 外部バイナリ・ツールを呼び出す場合は、`Command::new("<bin>")` で `execvp` 直叩きしてはならない。
  必ずユーザーの `$SHELL` を介して `Command::new(shell).arg("-c").arg(<cmd>)`
  （fish の場合は `-c`、POSIX 系も `-c`）でラップする。
- 理由: ユーザー環境では fish の `function` や zsh の alias など、シェル関数として
  定義されたコマンド（例: `ccs_print`）が日常的に使われている。`execvp` 直叩きは
  PATH 上の実バイナリしか解決できないため、これらが ENOENT になる。
  rai はユーザー寄りの個人 CLI なので、ユーザーが日常使う「シェル関数を含むコマンド」を
  そのまま受け取れる挙動を **既定** とする。
- 引数のクォーティングは shell ごとに違う（POSIX は `shell_words::quote`、fish は専用エスケープ）。
  `rai-core` 側に shell 検出 + クォーティングのユーティリティを置き、各サブコマンドは
  そこから利用する。POSIX 専用構文（`set -o pipefail`, `$?`, `[ ... ]` 等）と
  fish 専用構文（`begin; …; end`, `$pipestatus` 等）を混在させない。
- 例外（直 `execvp` してよい唯一のケース）:
  - その実行自体がシェル起動である場合（`Command::new($SHELL).arg("-c")…`）。
  - 完全に rai 内部限定で、ユーザー設定や `--engine-cmd` 等のユーザー入力に依存しない、
    かつシェル機能を一切要求しないと保証できる場合のみ。判断に迷ったらシェル経由を選ぶ。
- ユーザー指定の `--engine-cmd` のような自由入力は **常に** シェル経由で実行する。
  ここを直叩きにすると fish function 依存の値（`ccs_print c1` など）が動かない。

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
