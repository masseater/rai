# Adding a New Subcommand

新規 `rai <name>` を足すときの手順。`crates/cmd/rai-cmd-hello` がテンプレート。

## 0. Spec を先に書く

実装より先に `docs/specs/NN-<name>.md` を作る。`NN` は次の連番。詳細は [specs-workflow](wiki://specs-workflow)。

## 1. テンプレートを複製

```sh
cp -r crates/cmd/rai-cmd-hello crates/cmd/rai-cmd-<name>
```

## 2. 新 crate の `Cargo.toml` を編集

`crates/cmd/rai-cmd-<name>/Cargo.toml`:

```toml
[package]
name = "rai-cmd-<name>"
description = "..."   # clap の help 行に出る。短く正確に。
version.workspace      = true
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
repository.workspace   = true
authors.workspace      = true

[dependencies]
rai-core.workspace = true
clap.workspace     = true
anyhow.workspace   = true
# 必要なものだけ追加する
```

サードパーティ deps は `[workspace.dependencies]` から `<dep>.workspace = true` で取る。新規に共通化したいものは root `Cargo.toml` に追加。

## 3. `src/lib.rs` を書く

```rust
//! `rai <name>` — <short description>.
//!
//! 仕様: `docs/specs/NN-<name>.md`.

use clap::Args;
use rai_core::{cli::Run, Ctx, Result};

#[derive(Debug, Args)]
pub struct Cmd {
    // --flag fields
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        // ...
        Ok(())
    }
}
```

**`pub struct Cmd` の名前は必ず `Cmd`**。`rai-core::cli::Run` を実装する。詳細は [rai-core](wiki://rai-core)。

サブサブコマンドを持つ場合は `Subcommand` enum を追加し、`Cmd` 内に `#[command(subcommand)]` で埋める。`rai-cmd-claude` / `rai-cmd-develop` / `rai-cmd-conflicts` 等が参考実装 ([cmd-claude](wiki://cmd-claude) / [cmd-develop](wiki://cmd-develop) / [cmd-conflicts](wiki://cmd-conflicts))。

## 4. 外部コマンドを呼ぶ場合は `shell::` 経由

```rust
use rai_core::shell;

let st = shell::user_shell_argv(&["gh", "pr", "list", "--state", "open"])
    .status()?;
```

`Command::new("<bin>")` を直接書かない。詳細は [shell-execution-policy](wiki://shell-execution-policy)。

## 5. Workspace に登録

ルート `Cargo.toml` の `[workspace.dependencies]` に追加:

```toml
rai-cmd-<name> = { path = "crates/cmd/rai-cmd-<name>", version = "0.1.0" }
```

`crates/rai/Cargo.toml` の `[dependencies]` に:

```toml
rai-cmd-<name>.workspace = true
```

## 6. Dispatcher に variant 追加

`crates/rai/src/main.rs`:

```rust
enum Cmd {
    // ...既存
    /// <description>
    YourName(rai_cmd_<name>::Cmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self {
            // ...既存
            Cmd::YourName(c) => c.run(ctx),
        }
    }
}
```

## 7. 検証

```sh
cargo build -p rai-cmd-<name>
cargo test  -p rai-cmd-<name>
cargo run   -p rai -- <name> --help

# workspace 全体
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI と同じく warning 0 を保つ。

## 8. `rai doctor` に登録 (外部 CLI を新規依存させた場合のみ)

`crates/cmd/rai-cmd-doctor/src/lib.rs` の `REQUIRED_TOOLS` に追加。`rai doctor` で missing が検出できるようにしておく ([cmd-misc](wiki://cmd-misc))。

## 禁止事項

- 別の `rai-cmd-*` crate に依存しない。共通ロジックは `rai-core` に上げる。
- `[[bin]]` を増やさない。バイナリは `rai` 1 つだけ。
- 仕様書を書かずに着手しない。

## See Also

- [architecture](wiki://architecture) — workspace 全体像
- [rai-core](wiki://rai-core) — 共通基盤に何があるか
- [shell-execution-policy](wiki://shell-execution-policy) — 外部コマンド起動規約
- [specs-workflow](wiki://specs-workflow) — Spec-First ルール
