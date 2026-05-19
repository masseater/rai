# `rai conflicts`

CONFLICTING になっている自分の open PR を検出し、専用 worktree で agent CLI を呼んでコンフリクトを自動解消し、`git push --force-with-lease` まで完走させる **長時間バッチ**。fish 関数 `resolve-conflicts` (533 行) の Rust 移植。本リポジトリ最大の負債。仕様: `docs/specs/11-conflicts.md`。

## サブコマンド

| Command | 用途 |
|---|---|
| `rai conflicts run` | CONFLICTING な PR を agent CLI で自動解消する |
| `rai conflicts status` | `queue.json` の現在状態を表示する |
| `rai conflicts reset-failed` | failed な entry を pending に戻す |

## キュー

進行状況は `queue.json` (永続ファイル) に保存される。state は `pending` / `running` / `succeeded` / `failed` 等。`status` / `reset-failed` はこのファイルへの薄い CLI。

## `run` の流れ (おおまか)

1. `gh` で自分の open PR を取得し、CONFLICTING のみ抽出。
2. `queue.json` を更新し、未処理 entry を 1 件ずつ:
   1. 専用 worktree を用意 (`gwq`)
   2. agent CLI を起動して `git rebase` / merge コンフリクトを解消させる
   3. 解消後 `git push --force-with-lease`
3. 結果 (`succeeded` / `failed`) を `queue.json` に書き戻す。

## `rai develop` との関係

worktree + agent の起動骨格は [cmd-develop](wiki://cmd-develop) と似ているが、

- `develop` は対話的に 1 件起動して PR 仕上げまで持っていく
- `conflicts` は非対話バッチで複数 PR を順に処理する

役割が違うので crate は分かれている。共通化したいパターンが見えたら [rai-core](wiki://rai-core) に上げる。

## 注意

- バッチ実行中に rai 自身を更新したり、対象 PR を手で直したりすると `queue.json` が古くなる。`reset-failed` で巻き戻せるのは failed のみ。
- `--force-with-lease` を使うので、別端末からの push が間に挟まると弾かれる (期待動作)。

## See Also

- [cmd-develop](wiki://cmd-develop) — worktree + agent 起動の参考実装
- [shell-execution-policy](wiki://shell-execution-policy) — agent 起動のシェル経由規約
- [specs-workflow](wiki://specs-workflow) — `docs/specs/11-conflicts.md`
