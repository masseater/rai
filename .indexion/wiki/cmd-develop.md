# `rai develop`

Issue / PR を起点に **worktree + tmux + agent CLI** を一気に起動する。`rai` でもっとも複雑なサブコマンドであり、Issue 着手・PR 救済・セッション再開を 1 系統に統合した中心機能。仕様: `docs/specs/18-develop.md` / `docs/specs/19-develop-resume.md`。

## サブコマンド

| Command | 用途 |
|---|---|
| `rai develop issue <ISSUE>` | 新規 Issue から worktree を作って agent を回し、PR まで仕上げる |
| `rai develop pr <PR>` | 既存 PR の worktree でコンフリクト解消 / CI 修正 |
| `rai develop resume <ISSUE\|PR>` | 既存 worktree を破壊せず agent セッションだけ立て直す |
| `rai develop finalize-agent` (hidden) | agent 完了後に commit / push / `gh pr create` を任せる内部 hook |

## 共通の起動フロー

`crates/cmd/rai-cmd-develop/src/common.rs` が起動骨格を提供する:

1. **Worktree 準備**: `gwq` で worktree を作る (Issue) / 既存 PR worktree に入る (PR)。
2. **Prompt 構築**: `--prompt-template` 指定があれば読み込み、なければ default prompt を組み立てる。
3. **engine_cmd 展開**: `--engine-cmd` のプレースホルダ (`{PROMPT}`, `{PERMISSION_MODE}`, `{RAI}`) を置換。
4. **agent ブロック構築**: pipefail 擬似実装 (POSIX は `set -o pipefail` / fish は `$pipestatus`) で包む。
5. **tmux セッション起動**: agent コマンド + finalizer ブロックを `$SHELL -c` でラップして `tmux new-session` に渡す。
6. **fail-fast 確認**: 起動 750 ms 後にセッション生存を確認、即死していれば `/tmp/rai-develop/<session>.log` の末尾を吐いて非ゼロ exit。

## `--engine-cmd` 既定値

```
ccs c1 --print --output-format stream-json --verbose {PERMISSION_MODE} -- {PROMPT} | {RAI} claude format
```

- 全工程が `$SHELL -c` を通るので、`ccs_print` のような fish function でも解決される ([shell-execution-policy](wiki://shell-execution-policy))。
- パイプライン形にしてあるのは「どのシェルでも安定して動く」「失敗時にどの段階で落ちたか分かる」ため。
- プレースホルダを 1 つも含まない `--engine-cmd` を渡したときは legacy 互換で末尾に `--permission-mode <MODE>` と shell-quote 済み prompt を append する。

## `PermissionMode`

`claude --permission-mode` がそのまま受け取る 6 値:

```
acceptEdits  auto  bypassPermissions  default  dontAsk  plan
```

`rai_core::claude::PermissionMode` に各 variant `#[value(name = "...")]` でシリアライズ文字列を固定してある。clap の snake_case 自動シリアライズに任せると `claude` が受け取らない文字列になるので、新 variant 追加時は `#[value(name = ...)]` を必ず付ける。`--permission-mode` 未指定時は `--permission-mode` フラグそのものを engine_cmd に渡さない。

## tmux ラップとシェル分岐

`build_agent_shell_command` / `wrap_with_log` は `Shell::Posix` / `Shell::Fish` で構文を分岐する:

- POSIX: `set -o pipefail; (...); rc=$?`
- Fish: `begin; ...; end` + `$pipestatus` 走査で最悪値を `$__rai_agent_status` に格納

POSIX 専用構文と fish 専用構文を **混在させない**。詳細は [shell-execution-policy](wiki://shell-execution-policy)。

## Finalize Agent

agent が prompt を完了した直後、未コミット差分や未 push commit があれば `develop finalize-agent` を **同じ engine_cmd / `--permission-mode` で** 再起動する。commit/push/`gh pr create` を任せるが、commit メッセージは rai が決め打ちしない:

- 各リポジトリには commitlint preset, scope, allowed types, husky hook 等があり、`Implement issue #N: ...` のような subject テンプレを rai が決めると最低 1 つは既存 ruleset に弾かれる。
- 代わりに finalize 用 prompt は「`git commit` の commit-msg hook に弾かれたら直して再試行」「`--no-verify` は禁止」だけ伝える。
- commit-rule のソース (`commitlint.config.*`, `.husky/commit-msg`, `CONTRIBUTING.md`) を prompt に列挙しない。hook が自動で伝える。

`AgentArgs` に新フラグを追加するときは、`issue::build_finalize_command` / `pr::build_finalize_command` / `resume::build_finalize_command` と `finalize::Cmd` の 4 箇所すべてに通す。

## `develop resume` の不変条件

resume は **既存 worktree を破壊しない**:

- `develop issue` / `develop pr` の `ensure_worktree` は `git reset --hard` + `git clean -fd` + `git pull --rebase` で worktree を初期化する。
- 一方 `resume` は未コミット変更や rebase 中の状態を保持したまま、tmux + agent だけ立て直す。
- issue 系は `git for-each-ref refs/heads/develop/issue-<N>-*` で worktree branch を列挙し、複数あれば fzf で選ばせる。
- pr 系は PR の `headRefName` をそのまま branch とする。
- worktree が見つからない場合は明示エラー (新規作成は `develop issue` の責務)。

`common::find_existing_worktree` は **refresh しない** 専用ヘルパで、resume だけが使う。新規作成パス (`ensure_worktree`) と混同しないこと。

## 既存 worktree の最新化 (`develop issue` / `develop pr`)

`ensure_worktree` は対話プロンプトを廃止しており、`git reset --hard HEAD` + `git clean -fd` + `git pull --rebase` で **無人で** 最新化する。`gwq remove` で巻き戻すのは新規作成した worktree のみ。

## 制約

- **fork 由来 PR は非対応**。`headRepositoryOwner` が base owner と異なる PR は明示エラーで弾く。fork 対応は将来課題。
- **tmux なし環境では `--no-tmux`** で前景実行に切替可能だが、長時間 agent はセッション復帰できなくなるので非推奨。

## See Also

- [shell-execution-policy](wiki://shell-execution-policy) — `--engine-cmd` 展開とシェル分岐の前提
- [cmd-claude](wiki://cmd-claude) — `rai claude format` が agent 出力のパイプ後段
- [cmd-conflicts](wiki://cmd-conflicts) — 似た構造 (worktree + agent) で動く別バッチ
- [specs-workflow](wiki://specs-workflow) — `docs/specs/18-develop.md` / `19-develop-resume.md` / `20-ai-prompt-wording.md`
