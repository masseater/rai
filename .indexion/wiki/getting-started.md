# Getting Started

## Install

Homebrew tap (推奨):

```sh
brew tap masseater/rai
brew install rai
```

ソースから:

```sh
cargo install --path crates/rai
```

リリースは `cargo-dist` で配布される (`dist-workspace.toml` / `.github/workflows/release.yml`)。

## Verify

`rai` は外部 CLI に大きく依存する。新しい環境ではまず `doctor` で揃っているか確認する:

```sh
rai doctor
```

詳細は [cmd-misc](wiki://cmd-misc)。

## 最初のコマンド

```sh
rai --help                      # サブコマンド一覧
rai <subcommand> --help         # 個別ヘルプ
rai date                        # 動作確認用に軽いもの
```

ログ詳細度は `-v` または `RAI_LOG=debug` で上げる:

```sh
RAI_LOG=debug rai pr wait <PR>
```

## 開発コマンド

| Command | 用途 |
|---|---|
| `cargo fmt --all` | フォーマット |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint (CI と同じく warning は error 扱い) |
| `cargo test --workspace` | テスト |
| `cargo run -p rai -- <subcommand> [args]` | サブコマンドをローカル実行 |

CI (Linux + macOS) はこの 3 つを揃って green にする (`.github/workflows/ci.yml`)。push 前にローカルで通すこと。

## Shell Completion

```sh
# fish: 今のセッションだけ
rai completion fish | source

# 永続化: rai の更新でも追従させる --source 推奨
rai completion --source fish >> ~/.config/fish/config.fish
rai completion --source zsh  >> ~/.zshrc
rai completion --source bash >> ~/.bashrc
```

`--source` はバイナリパスから補完を再ロードする小さなスニペットを吐く。バイナリを差し替えてもシェル再起動だけで補完が追従する。

## 次に読む

- [overview](wiki://overview) — `rai` 全体像
- [architecture](wiki://architecture) — workspace 構造
- [cmd-develop](wiki://cmd-develop) — 中心となる develop ワークフロー
