# 03 — `rai gh rate-limit`

Source issue: [#3](https://github.com/masseater/rai/issues/3)

## 目的

GitHub API のレートリミット残量と reset までの残り時間を、人間に読める形で確認できるようにする。fish 関数 `gh_rate_limit` を `rai gh rate-limit` に置き換える。

## 機能要件

- 既定では core (REST) リソースの以下を表示:
  - `Now`: 現在時刻 (`YYYY-MM-DD HH:MM:SS`)
  - `Reset`: reset 時刻 (`YYYY-MM-DD HH:MM:SS`)
  - `In`: reset までの残り時間 (`Xh Ymin Zsec`)
- `--all` で core / search / graphql の 3 リソースをまとめて表示。
- `--json` で安定スキーマの単一 JSON オブジェクトを stdout に出す (機械可読)。
- `--tz local|utc` で表示タイムゾーンを切替 (既定 local)。
- データ取得元は `gh api rate_limit`。`gh` 認証エラー時は明確なメッセージで exit 1。
- `--watch <sec>` (任意) を指定すると指定秒間隔で表示更新する監視モード。

## 受け入れ条件

- [ ] 既定で core 1 リソースの Now/Reset/残り時間が出る。
- [ ] `--all` で 3 リソース分が表示される。
- [ ] `--json` 出力が安定スキーマで、jq でパースできる。
- [ ] 認証エラーで exit 1 + 「gh auth login が必要」など明確なメッセージ。
- [ ] `--tz utc` で時刻が UTC 表示になる。

## 期待する成果物

- `crates/cmd/rai-cmd-gh` crate (将来 gh 関連を束ねる想定で `gh` をネームスペースに)。
- `rai gh rate-limit` を `rai` 本体に配線。
- README に fish からの移行手順 (`alias git_rate_limit 'rai gh rate-limit'`) を記載。

## 非対象

- `gh` CLI 以外の認証経路 (PAT/octocrab 直叩き) はこの issue では扱わない。
