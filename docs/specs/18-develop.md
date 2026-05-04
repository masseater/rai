# 18 — `rai develop`

Source issues: [#9](https://github.com/masseater/rai/issues/9), [#13](https://github.com/masseater/rai/issues/13)

旧 `09-issue-develop.md` を統合・拡張したもの。`rai issue develop` は廃止し、
`rai develop issue` / `rai develop pr` の 2 系統に再編する。

## 目的

GitHub の Issue / Pull Request を起点に、専用の git worktree (`gwq`) と tmux session
を立ち上げ、その中で agent CLI (Claude Code 等) を自動起動して、

- `rai develop issue <ISSUE>`: Issue を一気通貫で開発〜PR 作成まで自走させる。
- `rai develop pr <PR>`: 既存 PR のコンフリクトや CI 失敗を解消・修正させる。

両者で worktree / tmux / agent / finalize の仕組みを共有する。

## 機能要件

### 共通

- worktree (`gwq`):
  - 既存 worktree がある場合は **無人で最新化する**: `git reset --hard`,
    `git clean -fd`, `git pull --rebase` を順に実行してから作業を開始する。
    対話プロンプト (attach / force-recreate / abort) は廃止する。
  - 新規時は `gwq add -b <branch>` (issue) もしくは `gwq add <branch>` (pr) で作成。
- tmux session:
  - セッション名は `<repo>-<flavor>-<N>-<YYYYMMDD-HHMMSS>` (`<flavor>` は `issue` / `pr`)。
  - `tmux new-session -d -s <session> -c <wt-path> <full_cmd>` で起動。
  - `<full_cmd>` は `set -o pipefail; (...)` (POSIX) / `begin; ...; end` + `$pipestatus`
    (fish) で囲み、パイプライン途中の失敗を取り逃さない。
  - 起動 750ms 後にセッション生存を確認し、即死していればログ末尾を出して非ゼロ終了 (fail-fast)。
- agent 実行:
  - 既定の engine_cmd は実バイナリのみで構成されたパイプライン:
    `ccs c1 --print --output-format stream-json --verbose {PERMISSION_MODE} -- {PROMPT} | {RAI} claude format`。
  - `-e/--engine-cmd CMD` で上書き可能。プレースホルダ `{PROMPT}` / `{PERMISSION_MODE}` /
    `{RAI}` を含む場合はそれぞれ shell-quote 済み prompt / `--permission-mode <MODE>` (空可) /
    現在の `rai` バイナリ絶対パスへ置換する。プレースホルダを 1 つも含まない場合は legacy 互換で
    末尾に `--permission-mode <MODE>` と prompt を append する。
  - `--prompt-template FILE` で prompt をファイルから読める。
  - `--no-tmux` で前面実行 (デバッグ用)。
- agent 権限モード:
  - `--permission-mode MODE` で `claude` 等の `--permission-mode` を明示。
  - 受理する MODE: `acceptEdits` / `auto` / `bypassPermissions` / `default` / `dontAsk` / `plan`。
- agent 終了後の自動公開 (finalize agent):
  - agent が正常終了し、worktree に未コミット変更または未 push の commit が残っていれば、
    同じ engine_cmd / `--permission-mode` で **finalize agent** を起動し、
    commit / push / `gh pr create` (または既存 PR への push) を委ねる。
  - commit メッセージや PR タイトルは rai 側でハードコードしない。commit-msg hook が
    commit 時に勝手にルールを伝える前提。`--no-verify` 等の hook 回避は禁止する prompt を出す。
  - agent 異常終了時は finalize agent を起動しない。
  - 未コミット変更も push 対象 commit も無い場合は worktree を自動的に片付ける
    (`gwq remove --force <branch>`)。issue 系のみ。pr 系は既存ブランチを残す。
  - `--no-auto-publish` で finalize agent 起動を含む agent 終了後の処理を全て無効化できる。

### `rai develop issue <ISSUE>...`

- 入力解決:
  - 引数なし: `gh issue list --state open --limit 50` を fzf 複数選択。
  - URL: `https://github.com/OWNER/REPO/issues/N` をパース。
  - 番号: 現リポジトリの `nameWithOwner` から `OWNER/REPO` を解決。
  - URL / 番号は複数指定できる。`--repo OWNER/REPO` で上書き。
- ブランチ名生成:
  - `-b/--branch` 指定があればそれを使う (複数 Issue 指定時は使えない)。
  - 未指定時は issue title から slug を作り、
    `develop/issue-<N>[-<slug>]-<YYYYMMDD-HHMMSS>` を生成。
  - slug: lower → `[^a-z0-9]+` を `-` に → 前後 `-` 削除 → 先頭 40 文字。
- finalize agent:
  - PR 本文に `Closes <issue-url>` を含めるよう指示。
  - `--pr-base BRANCH` で base branch を指定。

### `rai develop pr <PR>...`

- 入力解決:
  - 引数なし: `gh pr list --state open --limit 50` を fzf 複数選択。
  - URL: `https://github.com/OWNER/REPO/pull/N` をパース。
  - 番号: 現リポジトリ (`--repo` で上書き可) の PR を解決。
- worktree:
  - PR の head ref をローカルに `git fetch origin <headRef>:<headRef>` で取り込み、
    `gwq add <headRef>` で worktree 化する。fork 元 PR は対象外 (将来課題)。
- prompt:
  - PR URL / タイトル / `mergeable` / `statusCheckRollup` / 失敗 CI ジョブ名一覧を埋め込み、
    以下を agent に指示する:
    1. mergeable=CONFLICTING の場合は base ブランチを merge してコンフリクト解消。
    2. statusCheckRollup に FAILURE がある場合は失敗ジョブを `gh run view --log-failed` 等で
       原因調査し修正。
    3. push して PR の状態が改善するまで自走。
  - 既存ブランチへの追加 push が前提なので、新規 PR 作成は finalize agent から指示しない。

## 受け入れ条件

- [ ] `rai develop issue` が旧 `rai issue develop` と同等に動く (fzf / URL / 番号 / 複数指定 / `--repo` / `--branch` / `--prompt-template` / `--engine-cmd` / `--permission-mode` / `--no-tmux` / `--no-auto-publish` / `--pr-base`)。
- [ ] `rai develop pr <PR>` が PR の head ブランチで worktree を作り、tmux + agent を起動できる。
- [ ] PR が CONFLICTING のとき agent prompt にコンフリクト解消の指示が含まれる。
- [ ] PR の CI が FAILURE のとき agent prompt に失敗ジョブ名と修正指示が含まれる。
- [ ] 既存 worktree がある場合、対話なしで `git reset --hard` + `git clean -fd` + `git pull --rebase` が走る。
- [ ] tmux session 名が `<repo>-issue-<N>-<ts>` / `<repo>-pr-<N>-<ts>` で立ち上がる。
- [ ] agent 正常終了後、`develop issue` 側で未コミット変更または未 push commit があれば finalize agent が起動し、PR が無ければ作成、あれば既存 PR へ push する。
- [ ] `develop pr` 側では finalize agent が新規 PR を作らず、既存 PR への push に専念する。
- [ ] `--no-auto-publish` で finalize agent 起動を抑止できる。
- [ ] `--permission-mode MODE` が agent / finalize agent 双方に伝搬する。
- [ ] tmux 起動失敗時に **新規作成した worktree のみ** ロールバックされる (既存 worktree は保持)。
- [ ] `rai issue develop` / `rai issue finalize-agent` は廃止され、`rai develop *` に集約されている。

## 期待する成果物

- `crates/cmd/rai-cmd-develop` crate (`rai develop {issue,pr,finalize-agent}` を束ねる)。
- `rai develop` を `crates/rai/src/main.rs` に配線。
- `rai-cmd-issue` から `develop` / `finalize-agent` を取り除き、`inventory` / `triage` のみに縮小。
- README / gotchas.md / CLAUDE.md を新コマンド体系に追従させる。

## 非対象

- agent CLI 自体の実装 / 認証。
- fork 由来 PR の worktree 化。
- `rai conflicts` (バッチ複数 PR 自動解消) との統合。本仕様は単発 PR を対象とする。
