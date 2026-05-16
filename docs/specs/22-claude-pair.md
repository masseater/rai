# 22 — `rai claude pair`

## 目的

`claude --print` を 2 種類のプロンプト (A / B) で交互に回し続けるループを提供する。
既存の `rai pair` の「2 コマンドを交互に N サイクル / 時間打ち切り」ループを基盤に、
**claude セッションの維持** と **プロンプト先頭への `/goal` 自動付与** を上乗せする。

## ユーザー価値

- 「A 視点で計画 → B 視点でレビュー」のような 2 役の往復を、長時間 (〜数十サイクル)
  自動で回せる。
- 各役 (A / B) は **独立した claude セッション** を保ち、ループの中で文脈を蓄積
  していく。プロンプトを毎回送り直す必要がない。
- `/goal` を自動で先頭に付けるので、ユーザーは「今回のゴール本文」だけ書けばよい。

## 機能要件

- 起動形式: `rai claude pair [OPTIONS] --prompt-a <STR> --prompt-b <STR>`
- 必須フラグ:
  - `--prompt-a <STR>`: A 役に毎サイクル渡す本文。
  - `--prompt-b <STR>`: B 役に毎サイクル渡す本文。
- 任意フラグ:
  - `--max-cycles <N>` (default 10): A→B で 1 サイクル。`rai pair` と同じ意味。
  - `--max-hours <H>` (default 48): 累積最大実行時間。
  - `--permission-mode <MODE>`: そのまま `rai claude print` に渡す。
  - `--id-a <UUID>` / `--id-b <UUID>`: セッション ID を手動で指定して再開する用途。
    未指定なら起動時に RFC 4122 v4 風の新規 UUID を **A / B 別々に** 生成する。
  - `--prepend <STR>` (default `/goal`): `--prompt-a` / `--prompt-b` の先頭に
    付与する文字列。`""` を渡すと付与しない。間に区切りの半角空白を 1 つだけ挟む。
  - `--no-status-bar`: `rai pair` と同じく下部固定ステータスバーを無効化。
  - `--rai-bin <PATH>`: `rai claude print` を呼ぶ際の rai 実行ファイルパス。テスト用。
    未指定時は `std::env::current_exe()`。

## 振る舞い

- セッション ID:
  - 起動時に A 用 / B 用の 2 つの UUID を 1 度だけ生成する。
  - ループ中はその 2 つを使い続け、`rai claude print --session-id <UUID> ...`
    の継続切替に任せる。
  - 起動時に標準エラーへ "session A=<UUID> session B=<UUID>" を 1 行出す。
- プロンプト:
  - `<prepend> <prompt>` を組み立てて `rai claude print` の positional 引数に渡す。
  - `--prepend ""` のときは prompt をそのまま渡す。
- ループ実行:
  - `rai pair` と同等の挙動 (status bar / max-hours / max-cycles / SIGINT)。
  - 内部実装は `rai pair` の Rust ロジックを再利用 (重複実装しない)。
    具体的には、`rai pair` の Cmd を直接構築するのではなく、組み立てた
    `rai claude print` シェル文字列を `--command-a` / `--command-b` として
    `rai_cmd_pair::Cmd` に渡して `Run` を呼ぶ。
- exit code:
  - `rai pair` と同じ規約 (完走 0、子失敗はその code、time-out 124、SIGINT 130 等)。

## 受け入れ条件

- [ ] `rai claude pair --prompt-a 'plan' --prompt-b 'review' --max-cycles 2` が
      2 サイクル、計 4 回 claude を起動する。
- [ ] 起動時に標準エラーへ A / B 各セッション UUID が表示される。
- [ ] A 側の 2 回目は `--resume` 経由で初回会話の続きを引いている (claude の
      セッションファイルに 2 件目以降の turn が追記される)。B 側も独立に同様。
- [ ] `--prepend ""` を指定すると `/goal` が付かず、prompt 本文のみが渡る。
- [ ] `--id-a <UUID>` を指定すると、起動時に新規生成せずその UUID を使い回す。
- [ ] `--no-status-bar` で固定表示が消えて `rai pair` の no-status-bar と同じ
      ログだけになる。
- [ ] 1 サイクル目の A が non-zero で死ぬと B は実行されず、exit code は claude の
      それを引き継ぐ (= `rai pair` と同じ早期停止)。

## 期待する成果物

- `crates/cmd/rai-cmd-claude/src/pair.rs` (新規)。
- `crates/cmd/rai-cmd-claude/src/lib.rs` に `ClaudeCmd::Pair` variant を追加。
- ユニットテスト:
  - 組み立てる `rai claude print` シェル文字列が、prompt のクォーティングを含めて
    想定どおり (`/goal <prompt>` のクォーティング・`--session-id <UUID>` の有無)。
  - `--prepend ""` で `/goal` が消える。

## 非対象

- 3 役以上のローテーション。
- prompt のテンプレート展開 (`{{cycle}}` のような placeholder)。
- 失敗時の自動リトライ / 再生成。
- セッション ID の永続化 (resume を別 invocation で引き継ぎたい場合は `--id-a`
  / `--id-b` で明示する)。
