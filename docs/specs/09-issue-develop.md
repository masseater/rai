# 09 — `rai issue develop`

Source issues: [#9](https://github.com/masseater/rai/issues/9), [#13](https://github.com/masseater/rai/issues/13)

## 目的

GitHub Issue を起点に、専用の git worktree (`gwq`) と tmux session を立ち上げ、その中で agent CLI (Claude Code 等) を自動起動して、Issue を一気通貫で開発〜PR 作成まで自走させる。fish 関数 `gh-issue-fix` の Rust 移植。

## 機能要件

- 入力解決:
  - 引数なし: `gh issue list --state open --limit 50` を fzf 複数選択で選ぶ。
  - URL: `https://github.com/OWNER/REPO/issues/N` をパース。
  - 番号: 現リポジトリの `nameWithOwner` から `OWNER/REPO` を解決。
  - URL / 番号は複数指定できる。
  - `--repo OWNER/REPO` で上書き。
- ブランチ名生成:
  - `-b/--branch` 指定があればそれを使う。ただし複数 Issue 指定時は使えない。
  - 未指定の場合は issue title から slug を作成し、`develop/issue-<N>[-<slug>]-<YYYYMMDD-HHMMSS>` を生成。
  - slug: lower → `[^a-z0-9]+` を `-` に → 前後 `-` 削除 → 先頭 40 文字。fish 版と同じ規則。
- worktree (`gwq`):
  - 既存 `gwq get <branch>` がある場合は `attach / force-recreate / abort` の 3 択を tty で確認。
    - attach: 既存 tmux があれば attach、無ければ `gwq tmux run`。
    - force-recreate: `gwq tmux kill` → `gwq remove --force` → 新規 add。
    - abort: exit 130。
  - 新規時は `gwq add -b <branch>`。
- agent 実行:
  - prompt は固定文 (issue URL を一気通貫で実装し、ローカル検証まで自走するよう指示する内容)。
  - 既定の engine_cmd は実バイナリのみで構成されたパイプライン:
    `ccs c1 --print --output-format stream-json --verbose {PERMISSION_MODE} -- {PROMPT} | {RAI} claude format`。
    `tmux` の `default-shell` (zsh / bash) からも fish 関数に依存せず実行できる。
  - `-e/--engine-cmd CMD` で上書き可能。プレースホルダ `{PROMPT}` / `{PERMISSION_MODE}` /
    `{RAI}` を含む場合はそれぞれ shell-quote 済み prompt / `--permission-mode <MODE>` (空可) /
    現在の `rai` バイナリ絶対パスへ置換する。プレースホルダを 1 つも含まない場合は legacy 互換で
    末尾に `--permission-mode <MODE>` と prompt を append する。
  - prompt は `--prompt-template FILE` でファイルから読める。
  - `tmux new-session -d -s gwq-run-issue-<N>-<ts> -c <wt-path> <full_cmd>` で起動。
    `<full_cmd>` は `set -o pipefail; (...)` で囲み、パイプライン途中の失敗を取り逃さないようにする。
  - 複数 Issue 選択時は Issue ごとに worktree と tmux session を作成する。
  - `--no-tmux` で tmux を介さず前面実行 (デバッグ用)。
- agent 終了後の自動公開:
  - rai 自身は commit メッセージや PR タイトルを組み立てない。代わりに、agent が正常終了し
    かつ worktree に未コミット変更または未 push の commit が残っている場合、同じ engine_cmd で
    **finalize agent** を起動する。finalize agent は対象リポジトリの commit 規約 (`git log` /
    `commitlint.config.*` / `.husky/commit-msg` / `CONTRIBUTING.md` 等) を調査した上で、規約に
    沿った commit を作成し、`git push` と `gh pr create` を実行する責務を持つ。これは
    「リポジトリごとに違う commit / PR の流儀」を rai 側にハードコードしないための設計。
  - finalize agent は実装 agent と同じ engine_cmd / `--permission-mode` で起動される。
  - agent 異常終了時は finalize agent を起動しない。
  - 実装 agent が自分で commit / push / PR まで終わらせていてもよい。その場合 finalize agent は
    起動されず、空の worktree クリーンアップのみが行われる。
  - 既に同じ branch の PR がある場合は重複作成しない (finalize agent 側の責務)。
  - 未コミット変更も push 対象 commit も無い (= worktree が空) 場合は finalize agent を起動せず、
    `gwq remove --force <branch>` で worktree を自動的に片付ける。
  - `--no-auto-publish` で finalize agent の起動を含む agent 終了後の処理をすべて無効化できる。
  - `--pr-base BRANCH` で PR の base branch を指定できる (finalize agent への入力に渡る)。
- agent 権限モード:
  - `--permission-mode MODE` で agent (`claude`) の `--permission-mode` を明示できる。
  - 受理する MODE: `acceptEdits` / `auto` / `bypassPermissions` / `default` / `dontAsk` / `plan`。
  - 未指定なら engine_cmd の既定挙動に委ねる。
- ロールバック: gwq add 後 tmux 起動失敗 → `gwq remove` で巻き戻す。

## 受け入れ条件

- [ ] 引数なしで fzf による issue 複数選択ができる。
- [ ] URL / 番号 / 省略 の 3 系統解決ができる。
- [ ] URL / 番号を複数指定すると Issue ごとに起動できる。
- [ ] 複数 Issue 指定時に `--branch` を使うとエラーになる。
- [ ] branch 名生成が現行 fish 版と一致 (slug 規則, ts 形式)。
- [ ] gwq existing 時の attach / force-recreate / abort が動く。
- [ ] tmux session が `gwq-run-issue-<N>-<ts>` で立ち上がり、`-c` で worktree path に cd される。
- [ ] agent 正常終了後に未コミット変更または未 push commit があれば、rai が finalize agent を
      起動する。finalize agent は対象 repo の commit 規約を自力で調査し、規約に従った commit /
      push / `gh pr create` を実施する。rai 自身は commit メッセージ / PR タイトルをハードコード
      しない (リポジトリごとに違う conventional-commits / scope ルール / PR テンプレートを尊重)。
- [ ] finalize agent は実装 agent と同じ engine_cmd / `--permission-mode` で起動される。
- [ ] 同じ branch の PR が既にある場合は PR を重複作成しない。
- [ ] agent 異常終了時は自動 commit / push / PR 作成を行わない。
- [ ] `--no-auto-publish` で agent 終了後の自動公開を無効化できる。
- [ ] agent 正常終了後に未コミット変更も push 対象 commit も無い場合は worktree が `gwq remove` で自動削除される。
- [ ] `--permission-mode MODE` を渡すと agent コマンドへ `--permission-mode <MODE>` が伝搬される。
- [ ] 既定 engine_cmd が fish 関数等の shell 固有機能に依存せず、tmux の default-shell が zsh/bash
      でも実行できる (実バイナリのみ)。
- [ ] tmux 起動失敗時に worktree が残らない (ロールバック)。

## 期待する成果物

- `crates/cmd/rai-cmd-issue` crate (`rai issue *` を束ねる)。
- `rai issue develop` を本体に配線。
- README に fish からの移行手順 (`alias gh-issue-fix 'rai issue develop'`) を記載。

## 非対象

- agent CLI 自体の実装 / 認証。
- PR 作成後のレビューループ。
