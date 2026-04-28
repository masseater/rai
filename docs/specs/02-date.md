# 02 — `rai date`

Source issue: [#2](https://github.com/masseater/rai/issues/2)

## 目的

fish 関数 `mydate` 相当の「`date +%Y%m%d` のごく薄いラッパ」を `rai date` として提供する。シェルスクリプトや tmux ペインタイトルなど、日付文字列だけ欲しい場面で使える単一行ユーティリティ。

## 機能要件

- 引数なしで `YYYYMMDD` を 1 行 (末尾改行付き) で stdout に出力する。
- 出力形式は以下から選べる:
  - 既定: `YYYYMMDD`
  - `--time`: `YYYYMMDD-HHMMSS`
  - `--iso`: ISO-8601 (例 `2026-04-28T13:24:55+09:00`)
  - `--epoch`: UNIX epoch 秒
- タイムゾーン:
  - 既定で `TZ` 環境変数を尊重するシステムローカルタイム。
  - `--utc` で UTC 強制。
- 不正フラグは exit 2 で usage 表示。

## 受け入れ条件

- [ ] 引数なしの出力が `date +%Y%m%d` と完全一致する (同時刻に走らせて diff なし)。
- [ ] tty/非 tty どちらでも単一行 + 末尾改行のみ。余計な ANSI を出さない。
- [ ] `--utc` 指定時に TZ が UTC として扱われる。
- [ ] 不正フラグで exit 2、エラーメッセージは stderr。

## 期待する成果物

- `crates/cmd/rai-cmd-date` crate。
- `rai date` を `Cmd` enum に配線。
- README に fish の `mydate` から移行する手順 (`alias mydate 'rai date'`) を追記。

## 非対象

- カレンダー演算 (`-d "+1 day"` 等の datediff)。最小機能のみ。
