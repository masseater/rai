# 01 — `rai pair`

Source issue: [#1](https://github.com/masseater/rai/issues/1)

## 目的

2 つのコマンド (A, B) を 1 サイクルとして交互に回し続けながら、ターミナル下部に現在の状態を固定表示する。`rai` バイナリの **共通基盤** (端末復元 / signal 処理 / panic フック / 子プロセス実行ヘルパ / ログフォーマット) もここで確立し、後続 subcommand から再利用される。

## 想定ユーザー

- 「Claude Code セッションを A/B 切り替えで長時間 (〜2 日) 回したい」など、2 コマンドを延々と交互に動かしたい開発者。
- 既存の fish 関数 `command_pair_loop` 利用者 (ログ互換が前提)。

## 機能要件 (何をするか)

- A → B を 1 サイクルとして `--max-cycles` 回まで交互実行する。
- 累積実行時間が `--max-hours` を超えた時点で停止する (打ち切り)。
- 子プロセスが non-zero で終わったら以後は実行せず、その exit code で終了する。
- 実行中はターミナル下部に状態を 1Hz で更新表示する。表示内容は最低限「`cycle X/N | <A|B> running | elapsed=…s remaining=…s | <cmd 先頭>`」。
- 上部スクロール領域は子プロセスの stdout/stderr が通常通り流れる。
- 下部ステータス領域は起動直後にクリアされ、過去のコマンド出力や前回の表示が残らない。
- `rai pair` 自身のログと子プロセスのログは、下部ステータス領域を避けたスクロール領域へ流れる。
- ステータス表示は必要な行だけを更新し、通常の 1Hz 更新で視認できるちらつきを起こさない。
- `--no-status-bar` で固定表示を完全に無効化し、現行 fish 版と同じ振る舞いになる。
- 出力ログは `[YYYY-MM-DD HH:MM:SS] message` 形式 (現行 fish 版と互換)。
- 標準出力/標準エラーが tty でない場合は自動で `--no-status-bar` 相当に degrade する。

## 非機能要件

- どんな終了経路 (正常 / 非ゼロ / SIGINT / SIGTERM / SIGHUP / panic / 親切断) でも端末状態を完全に復元する。autowrap、scroll region、カーソル位置、状態行の余白を全て元に戻す。
- 子プロセスが alt screen に入ったり予期しない端末状態を残しても、復帰経路を持つ。最後の砦として `--no-status-bar` がある。
- SIGWINCH に追従して状態行と scroll region を再計算する。

## CLI 仕様

```
rai pair --command-a '<cmd>' --command-b '<cmd>'
         [--max-cycles N]      # default 10
         [--max-hours H]       # default 48
         [--no-status-bar]
         [--shell <bin>]       # default: $SHELL or /bin/sh
```

- 終了コード:
  - 全サイクル完走: 0
  - 子コマンドの失敗: その exit code
  - 時間打ち切り: 124
  - SIGINT: 130
  - SIGTERM: 143

## 受け入れ条件

- [ ] `rai pair --command-a 'sleep 5' --command-b 'sleep 5' --max-cycles 2` で下段に状態が常駐し、上段に出力が流れる。
- [ ] `rai pair --command-a 'printf "a\n"' --command-b 'printf "b\n"' --max-cycles 2` で各コマンドのログが下部ステータス領域へ残らない。
- [ ] ステータス行は 1Hz 更新中に毎回全消去されず、変更がない行は再描画されない。
- [ ] 実行中に Ctrl-C で端末が完全に元通り (autowrap / scroll region / カーソル / 余白行)。
- [ ] 子で `vim` を起動して `:q` 後、状態行が壊れずに復活する。
- [ ] 子で `htop` を強制 kill しても端末が戻る。
- [ ] `kill -TERM` / `kill -HUP` でも同等。
- [ ] `rai pair … 2>&1 | tee log` のようなパイプ越しでも壊れない (自動 no-status-bar)。
- [ ] panic を発生させるテストで端末が復元されることを CI で確認できる。
- [ ] `--no-status-bar` 時のログが fish 版とフォーマット互換。
- [ ] 既存 fish ユースケース (`ccs_print c1 '/pr-comment-resolve'` × `'/refactor'` を 10 cycles / 48 hours) を 1 サイクル以上完走できる。

## 期待する成果物

- `crates/cmd/rai-cmd-pair` crate (`rai pair` 本体)。
- `rai-core` への共通基盤追加 (端末復元 RAII / signal 処理 / panic フック / 子プロセス起動ヘルパ / ログフォーマッタ)。これらは Tier 1 以降からも再利用される前提で公開する。
- `rai pair` のユーザー向け説明と fish からの移行手順を README に追記。
- 既存 `~/.config/fish/functions/command_pair_loop.fish` を削除し `alias command_pair_loop 'rai pair'` に置き換える手順を README に明記。

## 非対象 (この issue でやらない)

- 3 コマンド以上の任意ループ。
- ステータス表示内容の DSL/テンプレート化。
- リトライ/指数バックオフ。
- リモート監視・Web UI。
