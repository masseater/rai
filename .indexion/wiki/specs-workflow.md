# Specs Workflow

`rai` は **Spec-First** 開発を強制する。実装より先に `docs/specs/NN-<name>.md` を書く。

## ルール

- 実装前に **必ず** `docs/specs/` に仕様を書く。
- Spec は **何を作るか** を書く: 目的 / ユーザー価値 / 成果物 / 受入条件。
- Spec は **どう作るか** を書かない: 実装計画 / 内部設計 / ライブラリ選定。それらは code, PR description, 設計メモに置く。
- 仕様なしの実装は禁止。途中で仕様が変わったら **コードより先に spec を更新する**。

(出典: workspace ルート `CLAUDE.md` / `AGENTS.md`)

## ファイル命名

```
docs/specs/NN-<name>.md
```

`NN` はゼロパディングなしの 2 桁連番 (現状 01〜23、09 は廃止統合済)。

## 仕様カタログ

| No | Title | Subcommand | ページ |
|----|---|---|---|
| 01 | `rai pair` | `rai pair` | [cmd-pair-repeat](wiki://cmd-pair-repeat) |
| 02 | `rai date` | `rai date` | [cmd-misc](wiki://cmd-misc) |
| 03 | `rai gh rate-limit` | `rai gh rate-limit` | [cmd-git-gh-gwq-dev](wiki://cmd-git-gh-gwq-dev) |
| 04 | `rai claude format` | `rai claude format` | [cmd-claude](wiki://cmd-claude) |
| 05 | `rai dev` | `rai dev` | [cmd-git-gh-gwq-dev](wiki://cmd-git-gh-gwq-dev) |
| 06 | `rai git autopull` | `rai git autopull` | [cmd-git-gh-gwq-dev](wiki://cmd-git-gh-gwq-dev) |
| 07 | `rai git track-mine` | `rai git track-mine` | [cmd-git-gh-gwq-dev](wiki://cmd-git-gh-gwq-dev) |
| 08 | `rai pr wait` | `rai pr wait` | [cmd-issue-pr](wiki://cmd-issue-pr) |
| 10 | `rai gwq clean` | `rai gwq clean` | [cmd-git-gh-gwq-dev](wiki://cmd-git-gh-gwq-dev) |
| 11 | `rai conflicts` | `rai conflicts` | [cmd-conflicts](wiki://cmd-conflicts) |
| 12 | `rai completion` | `rai completion` | [cmd-misc](wiki://cmd-misc) |
| 13 | `rai issue inventory` | `rai issue inventory` | [cmd-issue-pr](wiki://cmd-issue-pr) |
| 14 | `rai doctor` | `rai doctor` | [cmd-misc](wiki://cmd-misc) |
| 15 | `rai issue triage` | `rai issue triage` | [cmd-issue-pr](wiki://cmd-issue-pr) |
| 16 | 外部プロセス起動ポリシー | (横断ルール) | [shell-execution-policy](wiki://shell-execution-policy) |
| 17 | `rai repeat` | `rai repeat` | [cmd-pair-repeat](wiki://cmd-pair-repeat) |
| 18 | `rai develop` | `rai develop {issue,pr}` | [cmd-develop](wiki://cmd-develop) |
| 19 | `rai develop resume` | `rai develop resume` | [cmd-develop](wiki://cmd-develop) |
| 20 | AI prompt wording | (横断ルール) | [cmd-develop](wiki://cmd-develop) |
| 21 | `rai claude print` | `rai claude print` | [cmd-claude](wiki://cmd-claude) |
| 22 | `rai claude pair` | `rai claude pair` | [cmd-claude](wiki://cmd-claude) |
| 23 | `rai ccs usage` | `rai ccs usage` | [cmd-ccs](wiki://cmd-ccs) |

## 番号 09 が欠番

旧 `09-issue-develop.md` は `18-develop.md` に統合された。`rai issue develop` も `rai develop issue` に再編済み。古い番号は復活させない。

## Spec を書く流れ

1. 新サブコマンド (または横断ルール) のアイデアがある。
2. 次の連番 `NN` を取り、`docs/specs/NN-<name>.md` を作る。
3. テンプレート構成:
   - `# NN — <タイトル>`
   - Source issue リンク (任意)
   - `## 目的`
   - `## 機能要件` / `## 受入条件`
4. 仕様レビューが済んでから [adding-subcommand](wiki://adding-subcommand) に進む。

## 仕様変更時

> 仕様が途中で変わったら、コードより先に spec を更新する。

これは厳格に守る。spec とコードが乖離すると 23 本ある仕様書全部の信用が落ちる。

## See Also

- [overview](wiki://overview) / [architecture](wiki://architecture) — 仕様書群が何を支えているか
- [adding-subcommand](wiki://adding-subcommand) — spec を書いた後の実装手順
