# 19 — `rai develop resume`

`rai develop issue` / `rai develop pr` で起動した agent セッションが rate limit や
context limit、`tmux` の事故などで途中終了したときに、**worktree の進捗 (未コミット
変更も含む) を保持したまま** agent CLI を起こし直して続きを完了させるためのコマンド。

## 目的

- 途中まで進んだ作業を捨てずに済ませる: 既存 worktree を `git reset --hard` /
  `git pull --rebase` で巻き戻さない。
- agent への伝達を再開向けに切り替える: 「最初に `git status` / `git log` で
  進捗を確認し、続きから自走しろ」という prompt を流す。
- finalize agent (commit / push / PR 作成 or 追加 push) の自動公開フローは
  そのまま再利用する。

## 機能要件

### CLI

```
rai develop resume <TARGET>...
    [--repo OWNER/REPO]
    [--flavor issue|pr]
    [--branch BRANCH]      # 単一 TARGET 時のみ。issue 系で曖昧なときに直接指定。
    [--pr-base BRANCH]     # issue flavor の finalize 用 PR base。
    # AgentArgs (engine_cmd / prompt_template / no_tmux / no_auto_publish / permission_mode)
```

`<TARGET>` は以下を受け付ける:

- `https://github.com/OWNER/REPO/issues/N` → issue flavor。
- `https://github.com/OWNER/REPO/pull/N` → pr flavor。
- 数値 `N` → `--flavor` の指定値で issue / pr を決める。未指定なら issue。

複数 TARGET を渡すと順番に再開する。`--branch` は単一 TARGET でのみ使え、
issue 系で同じ番号の worktree が複数ある場合に明示するために使う。

### Worktree 解決

- **絶対に `git reset --hard` / `git clean -fd` / `git pull --rebase` を呼ばない。**
  worktree の状態は前回終了時のまま使う。
- issue flavor:
  - `--branch` が渡されればそれを使う。
  - 渡されなければ `git for-each-ref refs/heads/develop/issue-<N>-*` で候補を
    列挙する。
    - 0 件: エラー (worktree が見つからない)。
    - 1 件: それを使う。
    - 2 件以上: fzf で選択させる。
  - 解決した branch を `gwq get <branch>` に渡してパスを取得する。
    取得できなければエラー。
- pr flavor:
  - PR の `headRefName` をそのまま branch とする。fork 由来 PR は対象外
    (`develop pr` と同じく明示エラー)。
  - `gwq get <headRefName>` で worktree のパスを取得する。
    取得できなければエラー。

### tmux + agent 起動

- `common::launch` を再利用し、`<repo>-<flavor>-<N>-<YYYYMMDD-HHMMSS>` で
  新しい tmux セッションを立ち上げる (タイムスタンプが変わるので旧セッション
  名と衝突しない)。
- engine_cmd / permission_mode / prompt_template / no_tmux / no_auto_publish の
  扱いは `develop issue` / `develop pr` と同一。
- prompt は **resume 専用** に切り替える:
  - 共通: 「途中から再開してください」「最初に `git status` と
    `git log --oneline -20` で進捗を確認」「commit-msg hook 違反は再 commit、
    `--no-verify` 等は禁止」。
  - issue flavor: 残作業を仕上げ、commit / push し、PR が無ければ
    `gh pr create` (本文に `Closes <issue-url>`)、既にあれば追加 push のみ。
  - pr flavor: 既存 PR への追加 push のみ。新規 PR を作らない。

### finalize agent

- `--no-auto-publish` で抑止可能。既定は `develop issue` / `develop pr` と同じく
  ON。
- finalize agent を起動するときの `--engine-cmd` / `--permission-mode` /
  `--pr-base` の伝搬も同じ。

## 受け入れ条件

- [ ] `rai develop resume <ISSUE_NUMBER>` が `develop/issue-<N>-*` の worktree を
      見つけて、未コミット変更を破壊せずに tmux + agent を再起動できる。
- [ ] 同じ番号の worktree が複数ある場合は fzf で選択できる。
- [ ] `rai develop resume <ISSUE_URL>` が同等に動く。
- [ ] `rai develop resume <PR_URL>` (または `<PR_NUMBER> --flavor pr`) が PR の
      head ブランチで worktree を見つけて再開できる。
- [ ] resume では `git reset --hard` / `git clean -fd` / `git pull --rebase` が
      **走らない**。
- [ ] worktree が見つからない場合は明示的なエラーで終わる (`develop issue` /
      `develop pr` を案内する)。
- [ ] resume prompt に `git status` / `git log` 確認の指示が含まれる。
- [ ] issue flavor の resume prompt に `Closes <issue-url>` 含めて PR 作成する
      指示があり、既存 PR には追加 push のみ行う旨が書かれている。
- [ ] pr flavor の resume prompt は `新規 PR は作成しないでください` を含む。
- [ ] `--no-auto-publish` で finalize agent 起動を抑止できる。
- [ ] `--permission-mode` が agent / finalize agent 双方に伝搬する。
- [ ] fork 由来 PR は明示エラーで弾く (`develop pr` と同じポリシー)。

## 期待する成果物

- `crates/cmd/rai-cmd-develop` 配下に `resume.rs` を追加し、`DevelopCmd::Resume`
  variant として公開する。
- `common.rs` に「refresh しない既存 worktree 取得」ヘルパを追加し、`resume`
  から利用する (`develop issue` / `develop pr` 既存の `ensure_worktree` は不変)。
- `gotchas.md` に resume の挙動 (refresh しない・worktree 必須) を追記する。

## 非対象

- 死んだ tmux セッション自体の検出やお掃除 (タイムスタンプで session 名が
  変わるので衝突しない。古い session が残っていれば `tmux kill-session` 等で
  ユーザーが片付ける)。
- 死んだ worktree が無い場合の自動再構築 (これは `develop issue` の領域)。
- agent 出力ログから「どこまで進んだか」を解析して prompt に注入する処理
  (将来課題)。
