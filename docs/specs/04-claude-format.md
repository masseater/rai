# 04 — `rai claude format`

Source issue: [#4](https://github.com/masseater/rai/issues/4)

## 目的

`claude --output-format stream-json --verbose` 系の出力 (NDJSON) を、人間が追える整形テキストに変換する **stdin → stdout のフィルタ** を提供する。fish 関数 `ccs_print` の jq フィルタ部分を Rust に切り出して、ccs/claude の呼び出しから独立させる。

## 機能要件

- stdin から 1 行 1 JSON event を読み、整形した複数行を stdout に流す。
- イベントタイプごとの最低限の見た目:
  - `system.subtype=init`: セッション開始ヘッダ (session id / model)
  - `system` その他: `ℹ️  system[<subtype>]: …`
  - `assistant`/`user`: 含まれる content block を順に render (`text`/`tool_use`/`tool_result`/`thinking`/未知タイプ含む)
  - `result`: ✅ 完了サマリ (cost / turns / duration ms)
  - `error`: ❌ エラー本文
  - 未知 type: `❓ type: <raw>` で必ず可視化 (silently drop しない)
- 非 JSON 行 (ANSI bannerなど) はスキップしてクラッシュしない。
- ターミナル幅で content を truncate しない (情報を落とさない)。
- `--no-emoji` で絵文字を ASCII fallback (例: `[text]` `[tool]` `[->]` `[?]`) に置換。
- パイプ閉鎖 (SIGPIPE) で即終了する。
- 終了コード: stdin EOF まで処理して 0 / 内部エラー (I/O など) で 1。

## 受け入れ条件

- [ ] system/init / system 他 / assistant text / tool_use / tool_result / thinking / result / error / 未知 type が現行 fish 版と同等の見た目で出る。
- [ ] 非 JSON 行が混じってもクラッシュせず、後続が処理される。
- [ ] パイプ閉鎖で `rai claude format` が即終了する。
- [ ] `--no-emoji` で絵文字が ASCII にフォールバックする。

## 期待する成果物

- `crates/cmd/rai-cmd-claude` crate (将来 `rai claude *` を束ねる)。
- `rai claude format` を `rai` 本体に配線。
- README に「`claude -p ... --output-format stream-json --verbose | rai claude format`」のサンプルを記載。
- 既存 `ccs_print` を「ccs を呼んで `rai claude format` にパイプする薄い wrapper」に置き換える手順を README に明記。

## 非対象

- ccs / claude 自体の spawn / 認証 (フィルタに徹する)。
- `--ndjson-out` などの passthrough オプションは将来 issue に切り出す。
