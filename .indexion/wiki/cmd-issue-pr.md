# `rai issue` / `rai pr`

GitHub Issue / PR に対する操作系サブコマンド群。仕様: `docs/specs/13-issue-inventory.md` / `docs/specs/15-issue-triage.md` / `docs/specs/08-pr-wait.md`。

旧 `rai issue develop` は `rai develop issue` に統合済み ([cmd-develop](wiki://cmd-develop))。

## `rai issue`

| Command | 用途 |
|---|---|
| `rai issue inventory` | Issue 一覧を取得し、固定 prompt で AI engine に棚卸しさせる |
| `rai issue triage` | triage ラベル付き Issue を 1 件ずつレビューし、close/keep を判断する |

### `issue inventory`

GitHub Issue の棚卸し。**AI 判断結果を Issue 自体に「コメント + ラベル」として焼き込む** ことで、ユーザーが後から AI に依存せず機械的に処理 (例: `gh issue list --label triage:close-candidate | xargs gh issue close`) できる状態にする。

設計上の責務分割:

- Issue 取得・コメント投稿・ラベル付与は **`rai` の責務**。
- AI engine には **GitHub アクセスを行わせない**。AI には固定 prompt + Issue JSON だけを渡す。
- AI 判定が遅い・不安定でも、`--save-verdicts` で結果を保存しておけば後で `--from-verdicts` で AI を呼ばずに再 apply できる。

主なフラグ:

| Flag | 用途 |
|---|---|
| `--repo OWNER/REPO` | 対象リポジトリ。省略時は `gh repo view` で解決。 |
| `--engine-cmd "<cmd>"` | AI engine 起動コマンド (shell 文字列)。例: `"ccs_print c1"`。 |
| `--apply` | コメント・ラベルを実際に GitHub に書き込む。デフォルトは dry-run。 |
| `--save-verdicts PATH` | engine 出力を保存。後で `--from-verdicts` 再利用可能。 |
| `--from-verdicts PATH` | 保存済み verdict ファイルから読み込み (AI を呼ばない)。 |

### `issue triage`

`inventory --apply` で triage ラベルとコメントを焼き込んだ後の **人間レビュー段階** を担う。Issue 1 件ずつ本文 + コメントを表示して `c`(close) / `k`(keep) / `s`(skip) / `q`(quit) を入力する対話 UI。`gh issue close` とラベル削除はループ後にまとめて実行する。

## `rai pr`

| Command | 用途 |
|---|---|
| `rai pr wait` | PR の CI (check-runs) 完了まで polling し、完了時に通知する |

### `pr wait`

`gh pr checks --watch` のフォーマットが粗いので独自に集計表示と通知連携をする。完了時にデスクトップ通知 (macOS の `osascript display notification`) を出す。

## See Also

- [cmd-develop](wiki://cmd-develop) — Issue / PR を起点に worktree + agent を立てる側
- [cmd-git-gh-gwq-dev](wiki://cmd-git-gh-gwq-dev) — `gh` まわりの他のヘルパ (`rai gh rate-limit`)
- [specs-workflow](wiki://specs-workflow) — 関連仕様の索引
