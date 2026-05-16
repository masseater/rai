# 21 — `rai claude print`

## 目的

`claude --print`（非対話 print モード）を **指定したセッション ID と紐付けたまま**
何度でも呼び出せるラッパーを提供する。`rai claude print` は次の不変条件を保つ:

- 同じ session-id を渡せば、`rai claude print` の **2 回目以降は同じ会話の続き**
  として claude が応答する。
- 1 回目は `claude --session-id <UUID>`、2 回目以降は `claude --resume <UUID>` に
  自動で切り替える (claude は同じ session-id への `--session-id` 連打を拒否するため)。
- セッションの所属判定 (初回か継続か) は rai 側で永続化する小さな marker ファイル
  だけで行い、claude のセッション保管ディレクトリの内部レイアウトには依存しない。

## ユーザー価値

- `rai claude pair` のような上位ループが「同じ A 会話を回し続けたい」「同じ B 会話を
  回し続けたい」を 1 行で書ける。
- 単体でも、長時間の修正セッションを `--session-id` 付きで分割実行する用途に使える
  (cron や `rai repeat` から呼んで会話を維持できる)。
- claude CLI の session 切替の細かい仕様 (`--session-id` は新規専用 / `--resume` は
  継続専用) を呼び出し側から隠す。

## 機能要件

- 起動形式: `rai claude print [OPTIONS] --session-id <UUID> <PROMPT>`
  - `<PROMPT>` は単一の文字列。複数語のシェル展開や paste で渡しやすいよう、
    positional 1 個で受け取り、内部でそのまま claude に渡す。
  - `--session-id <UUID>` は **必須**。UUID 形式以外 (RFC 4122 v4) は弾く必要は
    ない (claude 側が検証する)。
- セッション継続ロジック:
  - 初回判定用 marker ファイル: `$XDG_STATE_HOME/rai/claude-print/<UUID>` (未設定時
    `$HOME/.local/state/rai/claude-print/<UUID>`)。
  - marker が無い → `claude --print --session-id <UUID> ...` を実行する。
    claude が 0 で終わったあとに marker を空ファイルで作る。
  - marker がある → `claude --print --resume <UUID> ...` を実行する。
  - 非 0 終了でも marker は作成する。session 自体は既に確保されているケースが
    多く、次回も同じ UUID を `--session-id` でぶつけると確実に二重生成エラー
    になるため、二段目以降は `--resume` に倒すのが安全側。
- フラグ:
  - `--permission-mode <MODE>`: そのまま claude にパススルー。`PermissionMode` の
    値は `rai develop` と同じ 6 種 (acceptEdits / auto / bypassPermissions /
    default / dontAsk / plan)。未指定なら `--permission-mode` 自体を付けない。
  - `--output-format <FMT>`: そのまま claude にパススルー。`text` / `json` /
    `stream-json` の 3 種。デフォルトは `text` (claude のデフォルトと一致)。
  - `--claude-verbose`: claude の `--verbose` をそのままパススルー (stream-json と
    併用するために必要)。長名は `rai` 共通の global `-v/--verbose` と衝突するため
    分けて命名している。
  - `--fork-session`: そのまま claude にパススルー (継続時の枝分かれに使う)。
  - `--claude-bin <PATH>`: 起動する claude のパスを差し替える。テスト用。デフォルト
    は `claude` (PATH 解決はユーザーシェル経由)。
- 子プロセス起動:
  - 必ずユーザーのログインシェル経由 (`rai-core::shell::user_shell_argv`) で実行
    する。`Command::new("claude")` の直叩きは禁止 (rai 全体ポリシー)。
- 標準入出力:
  - stdin / stdout / stderr はそのままパススルー。`rai claude print` 自身は
    出力に追記しない (ログを汚さない)。
- exit code:
  - claude の exit code をそのまま返す。signal 死は `128 + signo`。
- エラー:
  - `--session-id` 不在 → clap が usage error で `exit 2`。
  - prompt が空文字列 → clap が usage error。

## 受け入れ条件

- [ ] `rai claude print --session-id $(uuidgen) "echo hello"` が claude を起動し、
      stdout に応答が流れる (実 claude が必要なので手動確認 OK)。
- [ ] 同じ UUID で 2 回目を呼ぶと、claude 側で "Session ID ... is already in use"
      にならず、`--resume` で続きの会話として動く。
- [ ] marker ディレクトリの内容が `<UUID>` というファイル 1 つだけ生成される。
- [ ] `--permission-mode bypassPermissions` を付けると claude にもそのまま渡る。
- [ ] `--output-format stream-json --verbose` で stream-json が stdout に流れる。
- [ ] 起動シェルは `$SHELL` (= ユーザーシェル) であり、fish の function (`ccs_print`
      など) が `--claude-bin` で指定可能。
- [ ] claude が non-zero で終わった場合、その exit code がそのまま返る。

## 期待する成果物

- `crates/cmd/rai-cmd-claude/src/print.rs` (新規)。
- `crates/cmd/rai-cmd-claude/src/lib.rs` に `ClaudeCmd::Print` variant を追加。
- ユニットテスト:
  - marker パス計算が `XDG_STATE_HOME` を尊重する。
  - 初回 / 継続でビルドされる argv が想定どおり (`--session-id` vs `--resume`)。
  - `--permission-mode` / `--output-format` / `--verbose` が argv に正しく現れる。

## 非対象

- 並列実行 (常に直列で 1 回ずつ)。
- 出力整形 (`rai claude format` を別途 pipe する想定)。
- stdin から prompt を流し込む `--input-format stream-json` 対応。
