# `rai ccs`

`ccs` CLI と連携するサブコマンド群。仕様: `docs/specs/23-ccs-usage.md`。

## サブコマンド

| Command | 用途 |
|---|---|
| `rai ccs usage` | ccs 配下の Claude プロファイル (`type == account`) を横並びで取り、Anthropic 側のレートリミット (5h / 7d) 残量と reset 時刻を 1 つの表で見せる |

## `ccs usage`

`ccs auth list --json` で account 系プロファイルを列挙し、それぞれの `instance_path` から credentials を解決して `https://api.anthropic.com/api/oauth/usage` を叩く。

credentials の解決順:

1. `${instance_path}/.credentials.json` (旧 / 一部 profile の file-based)
2. macOS Keychain `Claude Code-credentials-<sha256(instance_path)[0..8]>` (現行 ccs の既定)

`accessToken` は stdout / stderr / ログのいずれにも出ない (`Bearer ****`)。1 件でも `expired` / `auth failed` / `timeout` / `http error` / `no credentials` があれば exit code は 非 0、ただし他 profile の表示は止まらない。

主なフラグ:

- `--profile <NAME>` (繰り返し) — 対象 profile を絞る
- `--json` — 機械可読 JSON を stdout に出す (`utilization` / `resets_at` の正規化済み)
- `--watch [SECS]` — 既定 60 秒間隔で再描画。Ctrl-C で抜ける
- `--timeout <SECS>` — 1 profile あたりの HTTP タイムアウト (既定 8)
- `--ccs-bin <PATH>` — テスト用に ccs バイナリを差し替える

## 不変条件

- 外部コマンドはすべてユーザーシェル経由 (`$SHELL -c`) で起動する。`ccs auth list --json` も `security find-generic-password` も例外なし
- credentials の読み書きはしない (refresh は ccs / claude 側の責務)
- 5h 80%+ は赤、60%+ は黄でハイライト (TTY 出力時のみ)

## See Also

- [cmd-claude](wiki://cmd-claude) — `claude` CLI 連携。旧 `rai claude usage` はここに統合されていた
- [specs-workflow](wiki://specs-workflow) — 仕様番号 23 の現在の場所
