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
  - marker が無い → marker を **先に** 空ファイルで作り、その後 `claude --print
    --session-id <UUID> ...` を spawn する。claude は `--session-id` を投げた
    瞬間に session を「登録済」にするため、応答途中で SIGKILL / OOM / 親死で
    rai が殺されても session は claude 側に残る。marker を後書きする実装では
    次回呼び出しが再び `--session-id` を当てて `"Session ID … is already in
    use"` で殺されるので、必ず spawn より **先** に書く。
  - marker がある → `claude --print --resume <UUID> ...` を実行する。
  - claude が非 0 で終わっても marker は消さない。session は既に確保されている
    可能性が高く、二段目以降は `--resume` に倒すのが安全側。claude が起動その
    ものに失敗するケース (PATH に無い等) では、ユーザーが marker を手動削除
    して復帰する。
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
  - **CLI からの起動は常に tmux 経由 (= デフォルトで tmux モードのみ)**。
    `rai claude print` を呼ぶたびに新規 detached tmux session
    (`rai-claude-print-<short-uuid>-<unix-ns>`) を 1 つ立て、その中で claude を
    回す。claude 終了後も `exec tail -f /dev/null` で pane を保持するので、
    `tmux attach -t <name>` で後追い確認できる (= 「print 終わっても消えない」要件)。
    `sleep infinity` を使わないのは macOS の BSD `sleep` が `infinity` を
    受け付けず即時 exit してしまい、pane ごと死ぬため。
  - tmux に渡す shell-command は default-shell の引用ルール (fish / zsh / bash で
    異なる) を避けるため、**POSIX `/bin/sh` 用の一時スクリプトファイル** に書き出して
    そのパスだけ渡す。tmux session 名は `<sentinel>.tmux` sidecar に書いて
    おくので後追いツールから引ける。
  - exit code は tmux 内のスクリプトが書く sentinel ファイル
    (`<marker_dir>/<UUID>.<ts>.rc`) を `rai claude print` 本体がポーリングして取得し、
    そのまま返す。tmux session は残置する (ユーザーが手動で kill する)。
- 標準入出力:
  - stdin / stdout / stderr はそのままパススルー。`rai claude print` 自身は
    出力に追記しない (ログを汚さない)。tmux 経由起動の場合、claude の出力は
    tmux pane の中に出るので、対面確認は `tmux attach` 経由になる。
- exit code:
  - claude の exit code をそのまま返す。signal 死は `128 + signo`。
- エラー:
  - `--session-id` 不在 → clap が usage error で `exit 2`。
  - prompt が空文字列 → rai 側で `exit 2` 相当のエラー。
  - `--output-format stream-json` を `--claude-verbose` 無しで指定 → rai 側で
    早期エラー (claude も同じ組合せを拒否するが、エラーメッセージが分かりづらい
    ので rai で先に弾く)。

## 受け入れ条件

- [ ] `rai claude print --session-id $(uuidgen) "echo hello"` が新規 tmux session を
      立ち上げ、その中で claude を起動する (実 claude が必要なので手動確認 OK)。
- [ ] claude が終了したあとも tmux session は残っており、`tmux attach -t <name>` で
      pane を覗くと claude の出力 + `--- rai claude print: claude exited rc=... ---`
      バナーが見える。
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
  - marker パス計算が `XDG_STATE_HOME` / `HOME` 各種未設定/設定パターンで期待
    どおり。テストは `std::env` を mutate せず純粋関数で検証する。
  - 初回 / 継続でビルドされる argv が想定どおり (`--session-id` vs `--resume`)。
  - `--permission-mode` / `--output-format` / `--verbose` が argv に正しく現れる。
  - validate: `stream-json` 単独で reject、`--claude-verbose` 併用で OK。
- 統合テスト (stub claude スクリプト + tempdir):
  - 同じ session-id への 1 回目 → 2 回目で `--session-id` → `--resume` に切替わる。
  - claude が非 0 終了でも marker は残り、次回 `--resume` に倒れる。

## 非対象

- 並列実行 (常に直列で 1 回ずつ)。
- 出力整形 (`rai claude format` を別途 pipe する想定)。
- stdin から prompt を流し込む `--input-format stream-json` 対応。
