# `rai pair` / `rai repeat`

長時間ループ系の汎用サブコマンド。

## `rai pair`

仕様: `docs/specs/01-pair.md`。

2 つのコマンド (A, B) を 1 サイクルとして交互に回し続けるループ + 下部固定ステータスバー。`rai` バイナリの **共通基盤** (端末復元 / signal 処理 / panic フック / 子プロセス実行ヘルパ / ログフォーマット) もここで確立され、後続サブコマンドから再利用される。

主なフラグ:

| Flag | 用途 |
|---|---|
| `--command-a "<CMD>"` | A 側コマンド (`$SHELL -c "<CMD>"`) |
| `--command-b "<CMD>"` | B 側コマンド |
| `--max-cycles N` | 最大サイクル数 (A→B で 1 サイクル)。デフォルト 10 |
| `--max-hours N` | 累積最大実行時間 (時間)。0 で無制限。デフォルト 48 |
| `--no-status-bar` | 下部固定ステータスバーを無効化 (現行 fish 版互換モード) |
| `--shell PATH` | 子コマンド実行用シェル。未指定時は `$SHELL` → `/bin/sh` |

タイムアウトは `timeout(1)` がある環境では使う (`proc::find_timeout_bin`)。exit 124 を予約。signal-hook で SIGINT を握り、ステータスバーを片付けてから抜ける。

## `rai repeat`

仕様: `docs/specs/17-repeat.md`。

任意のシェルコマンドを「回数」または「経過時間」で制限しながらループ実行する小さなランナー。`while true; sleep` ワンライナーや `watch` の使い回しを、回数上限・時間上限・失敗時の早期停止という最低限の品質保証付きで再利用しやすくしたもの。

主なフラグ:

| Flag | 用途 |
|---|---|
| `-n N` / `--count N` | 最大繰り返し回数 (≥1) |
| `-d D` / `--duration D` | 最大経過時間。例: `30s`, `5m`, `1h30m`, `500ms` |
| `-i D` / `--interval D` | 各イテレーション間の sleep |
| `<COMMAND>` | 実行コマンド (位置引数)。`$SHELL -c` にそのまま渡される |

`--count` と `--duration` は **OR**。少なくとも一方が必須 (`clap::ArgGroup` で強制)。

コマンド本体は `$SHELL -c <CMD>` 経由なので fish function / zsh alias もそのまま動く ([shell-execution-policy](wiki://shell-execution-policy))。

### 違い

- **`pair`**: 2 種類の処理を交互に。`claude pair` の基盤 ([cmd-claude](wiki://cmd-claude))。
- **`repeat`**: 1 種類の処理を繰り返し。watch 代わりの軽量版。

## See Also

- [cmd-claude](wiki://cmd-claude) — `claude pair` の基盤として `rai pair` を使う
- [rai-core](wiki://rai-core) — `term`, `signals`, `proc`, `shell` モジュール
- [shell-execution-policy](wiki://shell-execution-policy) — `--command-*` / `<COMMAND>` がそのまま `$SHELL -c` に渡る前提
