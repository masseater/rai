# 17 — `rai repeat`

## 目的

任意のシェルコマンドを「回数」または「経過時間」で制限しながらループ実行する
小さなランナーを `rai repeat` として提供する。`while true; sleep` のワンライナーや
`watch` で済ませていた使い回しを、回数上限・時間上限・失敗時の早期停止という最低限の
品質保証付きで再利用しやすくする。

## ユーザー価値

- 「失敗するまで」「N 回まで」「N 分間」のいずれの軸でも同じコマンドで回せる。
- ユーザーのログインシェル経由で実行されるので、`ccs c1` のような fish function /
  alias / シェル関数もそのまま渡せる (rai 全体のポリシーに揃える)。
- 進捗 (今が何回目か / 何秒経過したか / 残り) が標準出力で常時見える。

## 機能要件

- 起動形式: `rai repeat [OPTIONS] <COMMAND>`
  - `<COMMAND>` は **単一のシェル文字列** で、内部で `$SHELL -c <COMMAND>` として
    実行する。引数列ではない。
- フラグ:
  - `--count <N>` / `-n <N>`: 最大実行回数 (1 以上の整数)。
  - `--duration <D>` / `-d <D>`: 最初の起動からの最大経過時間。
  - `--interval <D>` / `-i <D>`: 各イテレーション**完了**から次回起動までに sleep
    する時間。省略時は 0 (即座に次回)。
- 停止条件:
  - `--count` と `--duration` のうち、指定された条件のいずれかを満たした時点で停止
    (両方指定された場合は **OR**、先に達したほうで止まる)。
  - 子プロセスが非ゼロで終了したら **即時停止** し、その exit code を `rai repeat`
    の exit code として返す (デフォルト挙動)。
  - `--count` と `--duration` の **両方が未指定** の場合は usage error として
    `exit 2`。「無限ループ」は明示的に書かないと選べない。
- 期間文字列 (`<D>`):
  - `30s`, `5m`, `1h`, `1h30m`, `500ms` のような単位付き。
  - 単位は `ms`, `s`, `m`, `h`, `d`。
  - 単位なしの裸の数値は許容しない (誤解を防ぐ)。
- 進捗ログ:
  - 各イテレーションの開始前に `iteration N (elapsed=...s)` を 1 行 stderr に出す。
  - 子プロセスの stdout/stderr はそのままパススルー (rai 側で書き換えない)。
- exit code:
  - すべて成功で停止条件に到達 → `0`。
  - 子プロセス失敗で打ち切り → 子プロセスの exit code (シグナル死は 128 + signal)。
  - 引数エラー → `2`。

## 受け入れ条件

- [ ] `rai repeat --count 3 'echo hi'` が `hi` を 3 回出力して exit 0。
- [ ] `rai repeat --duration 2s 'true'` が約 2 秒後に exit 0 で停止する。
- [ ] `rai repeat --count 100 --duration 1s 'true'` が `count=100` ではなく
      `duration=1s` で停止する (OR 動作)。
- [ ] `rai repeat --count 5 'false'` が 1 回目の失敗で停止し exit 1 を返す。
- [ ] `rai repeat 'echo hi'` (count/duration なし) が `exit 2` でエラー。
- [ ] `rai repeat --count 3 --interval 100ms 'date +%S'` のイテレーション間隔が
      100ms 以上空いている。
- [ ] `--duration 1xz` のような不正な単位で `exit 2` + stderr にメッセージ。
- [ ] 子プロセスは `$SHELL -c <CMD>` で起動されるので、ユーザーが定義したシェル
      関数 (例: fish の `function`) もそのまま実行できる。

## 期待する成果物

- `crates/cmd/rai-cmd-repeat` crate (`Cmd` struct + `Run` 実装)。
- `crates/rai/src/main.rs` の `Cmd` enum に variant + match arm を 1 つずつ追加。
- ルート `Cargo.toml` の `[workspace.dependencies]` に登録、
  `crates/rai/Cargo.toml` から workspace 依存として参照。

## 非対象

- 並列実行 (常に直列で 1 コマンドずつ)。
- 失敗時の自動リトライ / 指数バックオフ。
- 「最後の N 回の出力だけ残す」のような出力加工。`tee` で外部に任せる。
- cron 的な絶対時刻指定 (`every 5m at :00`)。経過時間ベースのみ。
