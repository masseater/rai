# rai-core

`rai-core` は workspace 全体の **共通基盤**。`rai-cmd-*` はこの crate にのみ依存し、サブコマンド同士は直接依存しない。Cross-cutting な関心事はすべてここに置く。

## モジュール一覧

| Module | 役割 |
|---|---|
| `lib.rs` | `Ctx`, `Result`, `Context` の再エクスポート |
| `cli` | `Run` trait |
| `logging` | `tracing-subscriber` 初期化 (`-v` / `RAI_LOG`) |
| `term` | 端末ユーティリティ (raw mode, alt-screen, `StatusBar` 等。`crossterm` ベース) |
| `signals` | `signal-hook` ベースの割込みハンドリング |
| `proc` | 子プロセス起動ヘルパ (`libc` 依存) |
| `shell` | `$SHELL -c` ラップとシェル別 quoting。[shell-execution-policy](wiki://shell-execution-policy) |
| `ts` | タイムスタンプ整形 (`chrono`) |
| `claude` | `PermissionMode` 等、Claude CLI 共有型 |
| `panic_hook` | panic 時に端末を復元してから panic を出すフック |

## 主要 API

### `Run` trait

```rust
pub trait Run {
    fn run(self, ctx: &Ctx) -> Result<()>;
}
```

全サブコマンドの `Cmd` 型が実装する。`rai-core::cli::Run` を経由してトップレベル enum が dispatch する ([architecture](wiki://architecture))。

### `Ctx`

```rust
#[derive(Debug, Clone, Default)]
pub struct Ctx {}
```

現状は空構造体。HTTP クライアント・設定ファイル・キャッシュディレクトリのような **グローバル状態を持つときは必ずここに入れる**。サブコマンドが個別に Singleton を作って結合しないための受け皿。

### `Result` / `Context`

`anyhow::{Result, Context}` をそのまま再エクスポート。`bail!` / `anyhow!` も呼び出し側で `anyhow::` を直接使う。

### `logging::init`

`main.rs` から 1 回だけ呼ぶ。`-v` フラグまたは `RAI_LOG=...` で詳細度を制御する。

### `panic_hook::install_panic_restore`

`StatusBar` や raw mode を使う長時間サブコマンドは、起動時にこれを呼んで panic 時の端末復元を保証する。`panic = "abort"` プロファイルなので drop は走らない前提で書く。

### `shell::*`

外部コマンドを起動する **唯一の正規ルート**。`Command::new("<bin>")` を新たに書かない。詳細は [shell-execution-policy](wiki://shell-execution-policy)。

## 設計ルール

- **stable な公開 API を保つ**: 全 `rai-cmd-*` が同時に依存しているため、`rai-core` の API 変更は workspace 全体の同時リファクタとして扱う。単一 crate 変更ではない。
- **サブコマンド固有のロジックを混ぜない**: 1 つのサブコマンドしか使わないものは、そのサブコマンドの crate に閉じておく。早すぎる共通化は `rai-core` を肥らせる。
- **`Ctx` の拡張は慎重に**: 全サブコマンドが受け取る型なので、追加フィールドはすべての crate に影響する。

## Development Commands

```sh
cargo build -p rai-core
cargo test  -p rai-core
```

`crates/rai-core/tests/shell_smoke.rs` は実シェルを叩く統合テスト。CI でも走る。

## See Also

- [architecture](wiki://architecture) — workspace 全体の俯瞰
- [shell-execution-policy](wiki://shell-execution-policy) — `shell` モジュールの背景
- [adding-subcommand](wiki://adding-subcommand) — `rai-core` をどう使うか
