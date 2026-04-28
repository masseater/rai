# 09 — `rai issue fix`

Source issue: [#9](https://github.com/masseater/rai/issues/9)

## 目的

GitHub Issue を起点に、専用の git worktree (`gwq`) と tmux session を立ち上げ、その中で agent CLI (Claude Code 等) を自動起動して、Issue を一気通貫で実装〜PR 作成まで自走させる。fish 関数 `gh-issue-fix` の Rust 移植。

## 機能要件

- 入力解決 (どれか 1 つ):
  - 引数なし: `gh issue list --state open --limit 50` を fzf で選択。
  - URL: `https://github.com/OWNER/REPO/issues/N` をパース。
  - 番号: 現リポジトリの `nameWithOwner` から `OWNER/REPO` を解決。
  - `--repo OWNER/REPO` で上書き。
- ブランチ名生成:
  - `-b/--branch` 指定があればそれを使う。
  - 未指定の場合は issue title から slug を作成し、`fix/issue-<N>[-<slug>]-<YYYYMMDD-HHMMSS>` を生成。
  - slug: lower → `[^a-z0-9]+` を `-` に → 前後 `-` 削除 → 先頭 40 文字。fish 版と同じ規則。
- worktree (`gwq`):
  - 既存 `gwq get <branch>` がある場合は `attach / force-recreate / abort` の 3 択を tty で確認。
    - attach: 既存 tmux があれば attach、無ければ `gwq tmux run`。
    - force-recreate: `gwq tmux kill` → `gwq remove --force` → 新規 add。
    - abort: exit 130。
  - 新規時は `gwq add -b <branch>`。
- agent 実行:
  - prompt は固定文 (issue URL を一気通貫で実装し、`gh pr create` まで自走するよう指示する内容)。
  - 既定の engine_cmd は fish 版互換 (`ccs_print c1`)。`-e/--engine-cmd CMD` で上書き可能。
  - prompt は `--prompt-template FILE` でファイルから読める。
  - `tmux new-session -d -s gwq-run-issue-<N>-<ts> -c <wt-path> <full_cmd>` で起動。
  - `--no-tmux` で tmux を介さず前面実行 (デバッグ用)。
- ロールバック: gwq add 後 tmux 起動失敗 → `gwq remove` で巻き戻す。

## 受け入れ条件

- [ ] 引数なしで fzf による issue 選択ができる。
- [ ] URL / 番号 / 省略 の 3 系統解決ができる。
- [ ] branch 名生成が現行 fish 版と一致 (slug 規則, ts 形式)。
- [ ] gwq existing 時の attach / force-recreate / abort が動く。
- [ ] tmux session が `gwq-run-issue-<N>-<ts>` で立ち上がり、`-c` で worktree path に cd される。
- [ ] tmux 起動失敗時に worktree が残らない (ロールバック)。

## 期待する成果物

- `crates/cmd/rai-cmd-issue` crate (`rai issue *` を束ねる)。
- `rai issue fix` を本体に配線。
- README に fish からの移行手順 (`alias gh-issue-fix 'rai issue fix'`) を記載。

## 非対象

- agent CLI 自体の実装 / 認証。
- PR 作成後のレビューループ。
