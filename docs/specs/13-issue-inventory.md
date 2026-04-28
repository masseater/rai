# 13 — `rai issue inventory`

## 目的

GitHub Issue の棚卸しに必要な Issue 一覧を `rai` が取得し、固定プロンプトに埋め込んで指定した AI engine に渡す。Issue 取得は CLI の責務とし、AI engine には GitHub からの取得を任せない。

## 機能要件

- `rai issue inventory` として提供する。
- 対象リポジトリ:
  - 未指定時は現在の GitHub リポジトリを使う。
  - `--repo OWNER/REPO` で上書きできる。
- Issue 取得:
  - `gh issue list` を使って Issue を取得する。
  - 既定では open Issue を最大 100 件取得する。
  - `--state open|closed|all` で状態を指定できる。
  - `--limit N` で取得件数を指定できる。
  - `--label LABEL`、`--assignee LOGIN`、`--author LOGIN`、`--search QUERY` で絞り込める。
- AI engine 実行:
  - `-e/--engine-cmd CMD` で engine CLI を指定できる。
  - 既定の engine は既存 agent 系コマンドと同じ `ccs_print c1`。
  - 固定プロンプトには取得済み Issue JSON と取得条件を含める。
  - 固定プロンプトでは、AI engine が Issue 取得や `gh issue list/view` を実行しないことを明示する。
  - 既定では prompt を engine CLI の最後の引数として渡す。
  - `--prompt-stdin` 指定時は prompt を標準入力で渡す。
- `--print-prompt` 指定時は engine を起動せず、生成した prompt を stdout に出力する。

## 受け入れ条件

- [ ] `rai issue inventory --repo OWNER/REPO` が `gh issue list` で Issue を取得できる。
- [ ] `--state`、`--limit`、絞り込みオプションが `gh issue list` に反映される。
- [ ] 生成 prompt に取得済み Issue JSON が含まれる。
- [ ] 生成 prompt に「AI engine が Issue 取得を行わない」制約が含まれる。
- [ ] engine CLI に prompt が渡される。
- [ ] `--print-prompt` で engine を起動せず prompt を確認できる。

## 期待する成果物

- `crates/cmd/rai-cmd-issue` に `inventory` サブコマンドを追加。
- `rai issue inventory` を `rai issue` の dispatcher に配線。
- README に `rai issue inventory` の用途を記載。

## 非対象

- GitHub Issue の更新、close、label 変更などの書き込み操作。
- AI engine の認証・設定。
- AI engine が GitHub にアクセスして追加調査するワークフロー。
