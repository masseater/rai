# `rai date` / `rai doctor` / `rai hello` / `rai completion`

小物サブコマンド。

## `rai date`

仕様: `docs/specs/02-date.md`。

fish 関数 `mydate` 相当の「`date +%Y%m%d` のごく薄いラッパ」。シェルスクリプトや tmux ペインタイトルなど、日付文字列だけ欲しい場面で使う 1 行ユーティリティ。

## `rai doctor`

仕様: `docs/specs/14-doctor.md`。

`rai` 全体が依存している外部 CLI が `PATH` にあるか診断する。新しい環境で `rai` を入れたユーザーが、何が足りないか即分かる状態を作る。

現在の `REQUIRED_TOOLS` (`crates/cmd/rai-cmd-doctor/src/lib.rs`):

```
git  gh  gwq  tmux  fzf  claude  ccs  tee
```

不足があれば exit code 非ゼロ + 不足ツール一覧を出す。

**新規依存を増やしたら忘れず追加する**。`rai-cmd-*` のどこかで新しい外部 CLI を `shell::user_shell_argv` で呼ぶようになったら、`REQUIRED_TOOLS` にも 1 行足す。詳細は [adding-subcommand](wiki://adding-subcommand)。

## `rai hello`

新規サブコマンド追加のテンプレート ([adding-subcommand](wiki://adding-subcommand))。動作としては挨拶を出すだけ。`crates/cmd/rai-cmd-hello/` を `cp -r` して新コマンドの雛形にする。

## `rai completion`

仕様: `docs/specs/12-completion.md`。

`rai` のサブコマンドツリーに対するシェル補完スクリプトを stdout に出力する。bash / zsh / fish / powershell / elvish 対応。

```sh
rai completion fish | source                            # 即時適用
rai completion --source fish >> ~/.config/fish/config.fish  # 永続化 (推奨)
```

`--source` は **バイナリパスから補完を再ロードする小さなスニペット** を吐く。これを rc に入れておくと、`rai` を更新しても新しいシェルでは更新後バイナリの補完が自動適用される。

### 実装上の例外

`rai-cmd-completion` だけは `Run::run` 経由ではなく、`crates/rai/src/main.rs` の match arm が `Cmd::Completion(c) => c.print(&mut Cli::command())` を直接呼ぶ。理由はトップレベルの `clap::Command` ツリーが必要だから。詳細は [architecture](wiki://architecture)。

## See Also

- [adding-subcommand](wiki://adding-subcommand) — `hello` をテンプレートに新コマンドを追加
- [architecture](wiki://architecture) — `completion` だけ dispatch 経路が違う理由
- [getting-started](wiki://getting-started) — `completion` の永続化手順
