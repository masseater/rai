# rai

`rai` は拡張可能な個人用 CLI です。`rai <subcommand>` を増やしていく前提で
Cargo workspace の monorepo として構成されています。

## 構成

```
rai/
├─ Cargo.toml                 # workspace ルート（共通依存・プロファイル）
├─ crates/
│  ├─ rai/                    # バイナリ。引数パースと dispatch だけを担う
│  ├─ rai-core/               # 共通基盤 (Run trait, logging, Ctx)
│  └─ cmd/
│     └─ rai-cmd-hello/       # サブコマンド 1 つにつき 1 crate
└─ .github/workflows/
   ├─ ci.yml                  # fmt / clippy / test (Linux & macOS)
   └─ release.yml             # tag push でクロスビルド & Homebrew 更新
```

設計の肝:

- バイナリは `rai` のみ。サブコマンドはすべて library crate。
- 各サブコマンドは `clap::Args` 構造体を公開し、`rai_core::cli::Run` を実装する。
- `crates/rai/src/main.rs` の `Cmd` enum に variant を 1 行足すだけで配線完了。
- 共通依存は workspace dependencies で集約。crate 側は `clap.workspace = true` のように書く。

## 使い方

```sh
cargo run -p rai -- hello
cargo run -p rai -- hello rai
cargo run -p rai -- --verbose hello
```

ログレベルは `RAI_LOG=debug rai hello` のように `RAI_LOG` で上書き可能。

## サブコマンドの追加手順

1. テンプレートを複製する:
   ```sh
   cp -r crates/cmd/rai-cmd-hello crates/cmd/rai-cmd-<name>
   ```
2. `crates/cmd/rai-cmd-<name>/Cargo.toml` の `name` と
   `src/lib.rs` の型・処理を新サブコマンド用に書き換える。
3. ルート `Cargo.toml` の `[workspace.dependencies]` に
   `rai-cmd-<name> = { path = "crates/cmd/rai-cmd-<name>", version = "0.1.0" }` を追加。
4. `crates/rai/Cargo.toml` の `[dependencies]` に
   `rai-cmd-<name>.workspace = true` を追加。
5. `crates/rai/src/main.rs` の `Cmd` enum に variant を 1 つ、
   `Run::run` の match に 1 行足す。

これだけで `rai <name>` が生える。

## 開発

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI (`.github/workflows/ci.yml`) で同じことを Linux / macOS で回している。

## リリースと Homebrew 配布

`v*` タグを push すると `.github/workflows/release.yml` が走り、
macOS (arm64 / x86_64) と Linux (arm64 / x86_64) のバイナリを
GitHub Release に公開する。

Homebrew tap を有効化するには:

1. tap リポジトリ `masseater/homebrew-rai` を作成し `Formula/rai.rb` を置く。
2. tap への push 権限を持つ PAT を本リポジトリに
   `HOMEBREW_TAP_TOKEN` シークレットとして登録する。
3. リポジトリ変数 `ENABLE_HOMEBREW_BUMP` を `true` にする。

これで release ジョブの後段が tap の Formula を自動更新する。

ユーザは:

```sh
brew tap masseater/rai
brew install rai
```

でインストールできるようになる。

## ライセンス

MIT OR Apache-2.0
