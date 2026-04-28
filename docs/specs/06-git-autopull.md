# 06 — `rai git autopull`

Source issue: [#6](https://github.com/masseater/rai/issues/6)

## 目的

カレントブランチの upstream を一定間隔で fetch し、HEAD と乖離していたら fast-forward だけで pull する。長時間張り付いている開発ブランチで「気付かないうちに upstream が進んでた」を潰す。

## 機能要件

- カレントリポジトリのカレントブランチの `@{u}` を対象とする。
- `@{u}` 未設定 / repo 外で実行された場合は exit 1 + 案内メッセージ。
- 一定間隔で `git fetch --quiet` し、HEAD と `@{u}` が異なる場合に `git pull --ff-only` 相当を実行する。
- pull 失敗時はログだけ吐いて次サイクルに進む (デフォルト)。`--strict` で即 exit 1。
- `--once` で 1 サイクル後 exit 0 (cron 用)。
- `--on-update CMD` 指定時は pull 成功直後に CMD を実行する。
- `--no-fast-forward` で「検出のみ、pull はしない」モードに切替。
- SIGINT / SIGTERM を受け取ったら次の sleep を待たず即終了。
- non-tty (cron) 環境でも問題なく動く。

## CLI 仕様

```
rai git autopull [--interval N=30]
                 [--remote origin]
                 [--branch BR]
                 [--once]
                 [--on-update CMD]
                 [--no-fast-forward]
                 [--strict]
```

## 受け入れ条件

- [ ] `@{u}` 未設定で exit 1 + 案内メッセージ。
- [ ] HEAD 一致のときは pull しない。
- [ ] HEAD 不一致で `git pull --ff-only` 相当を実行する。
- [ ] Ctrl-C で直ちに終了。
- [ ] `--once` で 1 サイクル後 exit 0。
- [ ] `--on-update` 指定時、pull 成功直後にコマンドが起動する。

## 期待する成果物

- `crates/cmd/rai-cmd-git` crate (将来 `rai git *` を束ねる)。
- `rai git autopull` を本体に配線。
- README に fish からの移行手順 (`alias git-autopull 'rai git autopull'`) を記載。

## 非対象

- non-fast-forward マージや conflict 解決 (この issue では扱わない)。
