# Overview

`rai` は作者個人のワークフローのための **拡張可能な personal CLI** で、単一バイナリとして配布される。日常的に使う外部ツール (`git` / `gh` / `gwq` / `tmux` / `claude` / `fzf` / `ccs` 等) を薄くラップし、特定の繰り返し操作を 1 コマンドに凝集する。

## 設計の核

- **1 バイナリ + サブコマンド crate 群**: トップレベルバイナリは `crates/rai/` だけ。各 `rai <subcommand>` は `crates/cmd/rai-cmd-<name>/` の独立 library crate に閉じる。新コマンド追加は enum variant 1 つと match arm 1 つの編集で完結する。詳細は [architecture](wiki://architecture) / [adding-subcommand](wiki://adding-subcommand)。
- **共通基盤は `rai-core`**: `Run` trait, `Ctx`, logging, term, signals, proc, shell, ts, claude(共通型) は `crates/rai-core/` に集約される。詳細は [rai-core](wiki://rai-core)。
- **外部コマンドは必ずユーザーシェル経由**: `Command::new("<bin>")` の直叩きは原則禁止。`$SHELL -c` でラップして fish の function や zsh の alias もそのまま解決させる。詳細は [shell-execution-policy](wiki://shell-execution-policy)。
- **Spec-First**: 実装前に `docs/specs/NN-<name>.md` を書く。詳細は [specs-workflow](wiki://specs-workflow)。

## サブコマンド一覧

| Command | 用途 | ページ |
|---|---|---|
| `rai develop {issue,pr,resume}` | Issue/PR を起点に worktree + tmux + agent CLI を起動する | [cmd-develop](wiki://cmd-develop) |
| `rai claude {format,print,pair,usage}` | `claude` CLI ヘルパ群 | [cmd-claude](wiki://cmd-claude) |
| `rai conflicts {run,status,reset-failed}` | CONFLICTING PR を agent で自動解消する長時間バッチ | [cmd-conflicts](wiki://cmd-conflicts) |
| `rai issue {inventory,triage}` / `rai pr wait` | Issue 棚卸し・triage / PR CI 完了待ち | [cmd-issue-pr](wiki://cmd-issue-pr) |
| `rai git {autopull,track-mine}` / `rai gh rate-limit` / `rai gwq clean` / `rai dev` | リポジトリ操作系 | [cmd-git-gh-gwq-dev](wiki://cmd-git-gh-gwq-dev) |
| `rai pair` / `rai repeat` | 長時間ループ系 | [cmd-pair-repeat](wiki://cmd-pair-repeat) |
| `rai date` / `rai doctor` / `rai hello` / `rai completion` | 小物 | [cmd-misc](wiki://cmd-misc) |

## See Also

- [getting-started](wiki://getting-started) — install と最初のコマンド
- [architecture](wiki://architecture) — workspace 構造とディスパッチ
- [specs-workflow](wiki://specs-workflow) — Spec-First ルールと仕様カタログ
