# `rai claude`

`claude` CLI (Claude Code) と連携するサブコマンド群。仕様: `docs/specs/04-claude-format.md` / `docs/specs/21-claude-print.md` / `docs/specs/22-claude-pair.md`。

## サブコマンド

| Command | 用途 |
|---|---|
| `rai claude format` | `claude --output-format stream-json --verbose` の NDJSON を整形して表示する |
| `rai claude print` | `claude --print` を session-id 単位で「初回 → 継続」自動切替で呼ぶラッパー |
| `rai claude pair` | 2 つのプロンプトを `--print` で交互に回す pair ループ |

ccs プロファイル横断のレートリミット表示は `rai ccs usage` に分離されている ([cmd-ccs](wiki://cmd-ccs))。

## `claude format`

stdin → stdout のフィルタ。fish 関数 `ccs_print` の jq フィルタ部分を Rust に切り出して、ccs / claude の呼び出しから独立させたもの。

```sh
claude --output-format stream-json --verbose ... | rai claude format
```

`rai develop` の `--engine-cmd` 既定パイプライン後段でも使われる ([cmd-develop](wiki://cmd-develop))。

## `claude print`

`claude --print` (非対話 print モード) を **指定した session-id と紐付けたまま** 何度でも呼び出せるラッパー。不変条件:

- 同じ session-id を渡せば、2 回目以降は同じ会話の続きとして実行される
- 初回かどうかの判定は rai 側が行い、`--resume` / `--continue` 等の切替を自動でやる

## `claude pair`

`claude --print` を 2 種類のプロンプト (A / B) で交互に回し続けるループ。既存の `rai pair` ([cmd-pair-repeat](wiki://cmd-pair-repeat)) の「2 コマンドを交互に N サイクル / 時間打ち切り」ループを基盤に、claude セッションの維持と prompt 先頭への `/goal` 自動付与を上乗せする。

## 共通: `PermissionMode`

`rai_core::claude::PermissionMode` を共有する ([cmd-develop](wiki://cmd-develop) 参照)。`claude --permission-mode` がそのまま受け取る 6 値:

```
acceptEdits  auto  bypassPermissions  default  dontAsk  plan
```

新 variant 追加時は `#[value(name = "...")]` を必ず付ける (snake_case 自動シリアライズは NG)。

## See Also

- [cmd-ccs](wiki://cmd-ccs) — ccs プロファイル横断のレートリミット表示 (旧 `claude usage`)
- [cmd-develop](wiki://cmd-develop) — `--engine-cmd` 経由で `claude format` を後段に置く例
- [cmd-pair-repeat](wiki://cmd-pair-repeat) — `claude pair` の基盤になる `rai pair`
- [rai-core](wiki://rai-core) — `claude::PermissionMode` の置き場所
