# 23 — `rai ccs usage`

## 目的

`ccs` で管理している複数の Claude Max アカウント (`c1` / `c2` / `team` 等) について、
Anthropic 側のレートリミット枠 (5 時間ローリング / 7 日 (週次)) の **残量と reset 時刻**
を 1 つの表にまとめて表示する。

## 背景 / ユーザー価値

- Claude Max には **5 時間ローリング枠** と **7 日 (週次) 枠** の 2 種類のリミットがある。
- これらは Claude Code の対話セッション内 `/usage` で確認できるが、`claude --print`
  経由では取れない (claude が "You are currently using your subscription to power your
  Claude Code usage" を返すだけ、2026-05-19 ローカルで確認済)。
- 結果、ユーザーは `ccs <profile>` を切り替えるときに「c1 は 5h 枠に当たりそうか」
  「c2 ならまだ余裕があるか」を勘で判断するしかない。
- `rai ccs usage` 1 発で全 ccs Claude プロファイルの 5h / 7d 枠を **横並び比較**
  できるようにする。これにより、ユーザーは安全側に倒した profile 切替判断ができる。
- 既存の `ccusage` (ローカル jsonl の token 合算) は累計コストを見る用途で、
  Anthropic 側のリミット枠とは別物のため代替にならない。

## 機能要件

- 起動形式: `rai ccs usage [OPTIONS]`
- フラグ:
  - `--profile <NAME>...` (繰り返し可): 対象プロファイルを絞る。未指定なら ccs の
    `type == "account"` プロファイルを全件対象。
  - `--json`: 機械可読 JSON を stdout に出力する。スクリプト連携用。
  - `--watch [SECS]` (default `60`): 指定秒ごとに再取得して同じ画面を更新する。
    Ctrl-C で抜ける。`--json` とは併用しない。
  - `--timeout <SECS>` (default `8`): 1 プロファイルあたりの HTTP タイムアウト。
  - `--ccs-bin <PATH>` (default `ccs`): ccs 実行ファイルを差し替える (テスト用)。
- `--period` / `--since` / `--until` は提供しない (Anthropic 側の取得 API に該当意味
  が無い)。
- 表示は profile ごとに 1 行で、以下の項目を含む:
  - PROFILE 名 (default profile には `*` マーカー)
  - TIER (Anthropic 側で持っている `rateLimitTier`、取れない場合は空欄)
  - 5h 枠の使用率 (`used_percentage`) と reset 時刻 (ローカルタイム)
  - 7d 枠の使用率と reset 時刻
  - NOTE 列 (異常時の理由: `no credentials` / `refresh failed` / `auth failed` /
    `timeout` / `no usage yet` 等)
- credentials の `accessToken` が `expiresAt` を過ぎている場合、`refreshToken` を
  使って Anthropic OAuth エンドポイントから token を更新し、その結果で usage を
  取得する。更新に成功した場合は新しい `accessToken` / `refreshToken` / `expiresAt`
  を読み出し元 (`.credentials.json` ファイル or macOS keychain) に書き戻す
  (refresh token は rotate するため)。`refreshToken` 自体が無効化されていれば
  `refresh failed` として行に出すに留め、他 profile の表示は止めない。
- 異常 (token 失効、no credentials、401、timeout 等) は **行単位** で表に出して
  全体の表示は止めない。標準出力の整形は破壊しない。
- TTY 出力時のみ、5h 枠 80%+ を赤、60%+ を黄で軽くハイライトする。`--json`
  指定時および非 TTY 出力時は色を出さない。
- JSON 出力スキーマ (簡約):
  ```json
  {
    "fetched_at": "2026-05-19T01:31:48Z",
    "profiles": [
      {
        "name": "c1",
        "is_default": true,
        "tier": "max_20x",
        "five_hour":  { "used_percentage": 25.0, "resets_at": 1779169200 },
        "seven_day":  { "used_percentage": 84.0, "resets_at": 1779303600 },
        "error": null
      }
    ]
  }
  ```
  `used_percentage` は 0–100 の float、`resets_at` は Unix epoch 秒 (UTC)。
  Anthropic 側の応答 (`utilization` / ISO8601 `resets_at`) からの正規化結果。

## 不変条件

- **アクセストークン (`claudeAiOauth.accessToken`) を一切ログ・stdout・stderr に
  出さない**。`-v` 付き debug でも `Bearer ****` でマスクする。HTTP エラー文字列に
  含まれないようにラップする。
- credentials の書き戻しは **OAuth refresh 成功時の token 3 点組のみ** に限定する。
  それ以外のフィールド (`scopes` / `subscriptionType` / `rateLimitTier` 等) や
  ファイル全体の構造は保つ。書き戻し先は **読み出し元と同一の場所** (file→同 file、
  keychain→同 service 名) で、他プロファイルの credentials には触れない。
- 個別 profile のエラーで全体を落とさない。1 件でも HTTP/token 系のエラーがあれば
  最終 exit code を非 0 にする。それ以外 (全件成功 or "no usage yet" のみ) は 0。
- ccs 自体の `auth list --json` が失敗した場合は前提崩壊として即終了する (`exit !=0`)。

## ユーザー受け入れ条件

- [ ] `rai ccs usage` を引数なしで実行すると、ccs の全 account profile が
      1 行ずつ表に出る。
- [ ] `rai ccs usage --profile c1` のように絞り込める。
- [ ] `rai ccs usage --json` で機械可読 JSON が得られる。
- [ ] `rai ccs usage --watch 30` で 30 秒間隔で再描画され、Ctrl-C で抜ける。
- [ ] `accessToken` が expired でも `refreshToken` が有効なら自動 refresh して
      usage を取得し、新 token を読み出し元 (file or keychain) に書き戻す。
- [ ] `refreshToken` 自体が無効な場合のみ "refresh failed" メモが出る。他の
      profile は通常通り表示される。
- [ ] アクセストークンがどの出力 (stdout / stderr / `-v` ログ) にも現れない。
- [ ] 既存の `--engine-cmd` 系のように rai は外部コマンドを必ずユーザーシェル経由
      (`$SHELL -c`) で呼ぶ — `ccs auth list --json` も例外ではない。

## 非対象

- 5h / 7d 以外のメトリクス (累計 token / cost) — 既存の `ccusage` がカバーする領域。
- 過去履歴の表示 / `--since` `--until` — Anthropic の取得 API に該当意味が無い。
- 非対話シェルでの色表示。
