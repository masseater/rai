# 08 — `rai pr wait`

Source issue: [#8](https://github.com/masseater/rai/issues/8)

## 目的

GitHub PR の CI (check-runs) の完了を polling で待ち、完了時にデスクトップ通知を出す。`gh pr checks --watch` ではフォーマットが粗いため、独自に集計表示と通知連携をする。

## 機能要件

- PR 識別:
  - 引数なし: 現ブランチに紐付く open PR を `gh` から自動解決。
  - PR 番号: 現リポジトリの `owner/repo` を解決して使う。
  - PR URL: `https://github.com/owner/repo/pull/N` をパース。
  - `--repo OWNER/REPO` で自動検出を上書き可能。
- ループ (既定 `--interval 10`):
  - `gh api repos/<owner>/<repo>/commits/<head_sha>/check-runs` を取得。
  - 集計: total / completed / success / failure / in_progress / pending(queued|requested|pending|waiting) / skipped。
  - tty: 1 行 in-place 更新 (`\r\033[K`)。non-tty: 1 行 1 状態で stdout に流す。
- 完了時:
  - すべて success: ✅ 表示。
  - 1 つでも failure: ❌ 表示。
  - success/failure 双方ゼロ: ⚠️ 表示。
  - 内訳 (Success / Failure / Skipped) を表示。
  - `terminal-notifier` (macOS) があれば通知を出す。なければ `notify-rust` などにフォールバック。両方なければ通知抑止。
- フラグ:
  - `--interval N`、`--repo OWNER/REPO`、`--no-notify`、`--json`、`--exit-on-fail`。

## 端末/シグナル要件

- in-place 更新中に SIGINT を受けたら、必ず改行してから終了し、行が壊れた状態で残らないようにする。
- non-tty では in-place しない (CI/ログ親和)。

## 受け入れ条件

- [ ] PR 番号 / URL / 省略 (現ブランチ) の 3 系統で PR を解決できる。
- [ ] check-runs を polling し、合計と各状態を集計表示する。
- [ ] 完了時 ✅/❌/⚠️ で結果分岐する。
- [ ] terminal-notifier に互換した通知 (`-open` で PR URL を開く) が動く。
- [ ] Ctrl-C で in-place 行が壊れない。
- [ ] `--json` で安定スキーマを出す。
- [ ] `--exit-on-fail` で失敗時に exit 1。

## 期待する成果物

- `crates/cmd/rai-cmd-pr` crate (将来 `rai pr *` を束ねる)。
- `rai pr wait` を本体に配線。
- README に fish からの移行手順 (`alias gh_pr_ci_wait 'rai pr wait'`) を記載。

## 非対象

- PR の作成 / マージ操作。
- CI ログのストリーミング表示。
