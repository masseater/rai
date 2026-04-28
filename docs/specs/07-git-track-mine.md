# 07 — `rai git track-mine`

Source issue: [#7](https://github.com/masseater/rai/issues/7)

## 目的

自分が出している open PR の head ブランチを、ローカルに tracking branch として一括で生やす。レビュー対応や rebase の起点として、いちいち `git fetch && git checkout -t origin/...` をしなくていいようにする。

## 機能要件

- 既定の挙動:
  - `gh api user --jq .login` で自分のログインを解決し、`gh pr list --author <self> --state open --limit 200` の結果から head ブランチ名を集める。
  - `git fetch --prune <remote>` を先に走らせる。
  - 各ブランチについて:
    - ローカルに既に存在 → スキップ (`already exists locally`)
    - 対応する `refs/remotes/<remote>/<br>` が無い → スキップ (`remote/<br> not found`)
    - それ以外 → `git branch --track <br> refs/remotes/<remote>/<br>`
  - 最後にサマリ `created=… skipped=… missing=… remote=… user=…` を 1 行出す。
- `--author USER` でログインを上書き。
- `--remote NAME` (既定 `origin`)、`--limit N` (既定 200)。
- `--state open|closed|all` (既定 `open`)。
- `--dry-run` で副作用なし、何が作成/スキップされるかだけ表示。
- `--json` で機械可読サマリ `{created, skipped, missing, remote, user, branches: […]}`。
- gh 認証エラーで exit 1 + 明示メッセージ。

## 受け入れ条件

- [ ] open PR の headRefName 一覧を取得し、未存在ローカルだけ `branch --track` で生やす。
- [ ] 既存ローカル / remote 不在 はそれぞれカウントされる。
- [ ] `--dry-run` で副作用なし。
- [ ] gh 認証エラーで exit 1 + 明示メッセージ。
- [ ] `--json` がパース可能。

## 期待する成果物

- `crates/cmd/rai-cmd-git` の中に `track-mine` サブコマンドを追加 (autopull と同居)。
- README に fish からの移行手順 (`alias git-track-all-remote-branches 'rai git track-mine'`) を記載。

## 非対象

- 他人の PR のトラッキング (`--author` で明示指定するケース以外で自動取り込みはしない)。
