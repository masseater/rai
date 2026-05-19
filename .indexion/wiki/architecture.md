# Architecture

`rai` は Cargo workspace。バイナリは `rai` 1 つだけで、サブコマンドはすべて library crate として並ぶ。

## Workspace 構成

```
crates/
├── rai/           ← binary crate。CLI parse + dispatch のみ
├── rai-core/      ← 共通基盤 (Run trait, Ctx, shell, logging, ...)
└── cmd/
    └── rai-cmd-*/ ← サブコマンドごとの library crate
```

`Cargo.toml` の `[workspace.dependencies]` に各 crate がパス指定で並んでおり、`rai` 本体と `rai-cmd-*` は `<dep>.workspace = true` でこれを参照する。バージョン・edition・rust-version・license は `[workspace.package]` から継承する。

## Dispatch の仕組み

`crates/rai/src/main.rs` は薄い:

```rust
enum Cmd {
    Hello(rai_cmd_hello::Cmd),
    Develop(rai_cmd_develop::Cmd),
    Claude(rai_cmd_claude::Cmd),
    // ... 1 サブコマンド = 1 variant
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self {
            Cmd::Hello(c)   => c.run(ctx),
            Cmd::Develop(c) => c.run(ctx),
            // ...
        }
    }
}
```

各 `rai-cmd-<name>` crate は `pub struct Cmd` (`#[derive(clap::Args)]`) を公開し、`rai_core::cli::Run` を実装する。トップレベル `Cmd` enum の variant がそれを wrap するだけ。

### 例外: `rai completion`

`rai-cmd-completion` は補完スクリプトを生成するためにトップレベル `Cli::command()` (`clap::Command` ツリー) を必要とする。そのため `main.rs` の match arm は `Cmd::Completion(c) => c.print(&mut Cli::command())` と書かれており、`Run::run` を経由しない唯一の経路になっている。`crates/rai/src/main.rs` を一般化する際にこの分岐は残すこと。

### Exit Code 130

ユーザーが対話 UI (fzf 等) をキャンセルすると `rai_cmd_develop::common::UserCancelled` が `Result::Err` で伝搬する。`main.rs` はこれを downcast して `exit(130)` する。`std::process::exit` 直接呼びは `Result` の destructors を巻き戻さないので、必ず `bail!(UserCancelled)` 経由で `main` まで戻す。

## 依存関係ルール

| ルール | 理由 |
|---|---|
| `rai-cmd-*` → `rai-core` のみ参照 | サブコマンド間の crate 依存禁止。共通化が必要なら `rai-core` に上げる。 |
| 共通サードパーティは `[workspace.dependencies]` に登録 | バージョン分散を防ぐ。crate 側は `<dep>.workspace = true`。 |
| `[[bin]]` を増やさない | バイナリは `rai` 1 つだけ。サブコマンドは必ず library crate。 |
| 後方互換は追わない | 個人 CLI なので理想形を優先する (workspace `AGENTS.md` 参照)。 |

## Release Profile

```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
panic = "abort"
```

`panic = "abort"` のため、panic 中に terminal 復元したい処理は `panic_hook` ([rai-core](wiki://rai-core) 参照) に登録しておく必要がある。

## See Also

- [rai-core](wiki://rai-core) — 共通基盤の中身
- [adding-subcommand](wiki://adding-subcommand) — 新規サブコマンド追加手順
- [shell-execution-policy](wiki://shell-execution-policy) — 外部コマンド起動の流儀
