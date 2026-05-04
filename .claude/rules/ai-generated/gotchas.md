# Gotchas

## Background

`rai` の各サブコマンドは `claude` / `gh` / `git` / `gwq` / `tmux` / `fzf` 等の
外部コマンドを大量に呼び出す。Rust ソースだけ見ても挙動が見えづらい部分があるので
ここに集めておく。

## Rules

### 外部プロセス起動 (Shell-Function Friendly)

- `rai` から外部コマンドを起動するときは **必ず** ユーザーのデフォルトシェル
  (`$SHELL`) 経由で実行する。`Command::new("<bin>")` の `execvp` 直叩きは禁止。
  詳細は `docs/specs/16-shell-execution-policy.md` と `AGENTS.md` の
  "External Process Execution" を参照。
- 共通ユーティリティは `rai-core::shell` にある。各サブコマンドが個別に
  `Shell` enum や `shell_quote` を再実装しないこと。代表 API:
  - `shell::user_shell_path()` / `shell::detect_user_shell()` …… `$SHELL` の解決と種別判定。
  - `shell::user_shell_argv(&["bin", "arg1", "arg2"])` …… 引数列を `$SHELL -c` で実行する `Command`。
    `Command::new(...).args(...)` の置き換え。
  - `shell::user_shell_command("foo | bar")` …… ユーザー入力のシェル文字列をそのまま `$SHELL -c` で実行。
  - `shell::quote_for(shell_kind)` / `shell::quote_path(shell_kind, path)` …… シェル種別に応じた引用。
- ユーザー入力 (`--engine-cmd`, `--on-update`, `--agent-cmd` など) は
  **トークン分割せず** シェル文字列としてそのまま `$SHELL -c` に渡す。
  fish の function や zsh の alias、パイプ・リダイレクトをそのまま使えるのが
  この CLI の前提。
- `rai-core::shell::shell_command(...)` が `Command::new(shell_path)` を呼ぶのは
  ポリシー上の唯一の例外。それ以外で `Command::new` を新たに書かない。

### `rai develop {issue,pr}` 周り

`rai issue develop` は `rai develop issue` に統合された。`rai develop pr <PR>` は既存 PR の
worktree でコンフリクト解消 / CI 修正を agent CLI に任せるための spinoff。
共通基盤は `crates/cmd/rai-cmd-develop/src/common.rs` にあり、Issue / PR は
`issue.rs` / `pr.rs` に分かれる。finalize は `finalize.rs` で `--flavor issue|pr` 切替。


- `claude --permission-mode` の値は 6 種:
  `acceptEdits`, `auto`, `bypassPermissions`, `default`, `dontAsk`, `plan`。
  `PermissionMode` 列挙を変更するときは `claude --help` と突き合わせ、
  `#[value(name = "...")]` で正確な文字列に揃える (snake_case 自動シリアライズ NG)。
- `permission_mode` は `Option<PermissionMode>` で、未指定時は `--permission-mode` を
  付けない。`dontAsk` を既定にしたいなら `default_value_t = PermissionMode::DontAsk` に変える。
- `engine_cmd` の既定値:
  `ccs c1 --print --output-format stream-json --verbose {PERMISSION_MODE} -- {PROMPT} | {RAI} claude format`。
  全ステップが `$SHELL -c` 経由で実行されるので、`ccs_print` のような fish function を
  ユーザーが `--engine-cmd` に渡しても解決される。既定をパイプライン形にしているのは
  「どのシェルでも安定して動く」「失敗時にどの段階で落ちたか分かる」ため。
- `tmux` は wrap したコマンド全体を `$SHELL` (ユーザーログインシェル) で実行する。
  `build_agent_shell_command` と `wrap_with_log` は `Shell` (POSIX / Fish) で分岐し、
  そのシェルが解釈可能な構文を出力する。fish では `set -o pipefail` / `$?` / `[ ... ]` が
  使えないので、`begin; …; end` / `$pipestatus` / `test` を使う。POSIX 専用構文と fish
  専用構文を混在させない。
- プレースホルダ (`{PROMPT}` / `{PERMISSION_MODE}` / `{RAI}`) は
  `build_engine_cmd` と `build_agent_shell_command` で置換される。
  ユーザーが渡した `--engine-cmd` がプレースホルダを 1 つも含まない場合は、
  legacy 互換で末尾に `--permission-mode <MODE>`(指定があれば) と
  シェルクォート済み prompt を append する。クォーティングは
  `shell::quote_for(shell_kind)` を使い、シェル種別に応じて切り替える。
- agent ブロックは `pipefail` を擬似実装する。例えば `ccs c1` が exit 127 で死んでも
  後段の `rai claude format` が 0 を返す可能性があるが、その場合でも auto-publish は
  抑止する。POSIX は `set -o pipefail; (...); $?`、fish は `$pipestatus` を歩いて
  最悪値を `$__rai_agent_status` に格納してから finalizer チェックする。
- `rai develop` は `tmux new-session` の 750 ms 後にセッション生存を確認し、
  即死していれば `/tmp/rai-develop/<session>.log` の末尾を出して非ゼロ終了する
  (fail-fast)。wrap を変更するときも、起動 1 秒以内に死んだセッションが
  非ゼロ exit + ログパス表示になる、という不変条件を保つこと。
- `finalize_after_agent` は **rai 側で `git commit -m '...'` を打たない**。各リポジトリには
  commitlint preset / scope / allowed types / husky hook 等のルールがあり、
  `Implement issue #N: ...` のような subject テンプレを rai が決め打ちすると最低 1 つは
  既存 commitlint 設定に弾かれる。代わりに、未コミット差分や未 push commit があるときは
  同じ engine_cmd / `--permission-mode` で **finalize agent** を起動して
  commit / push / `gh pr create` を任せる。プロンプトには commit-rule のソース
  (`commitlint.config.*`, `.husky/commit-msg`, `CONTRIBUTING.md` 等) を列挙しない。
  `git commit` 時に commit-msg hook が自動でルールを伝えるので、プロンプトは
  「hook に弾かれたら直して再試行」「`--no-verify` は禁止」だけ伝える。rai 側の subject
  テンプレも、ルールファイルのリストも復活させない。
- `finalize::Cmd` は `--engine-cmd` と `--permission-mode` を `build_finalize_command`
  経由で受け取る。新しい engine 関連フラグを `AgentArgs` に追加するときは、
  `issue::build_finalize_command` / `pr::build_finalize_command` /
  `resume::build_finalize_command` と `finalize::Cmd` にも忘れず通すこと。
- `rai develop resume <ISSUE|PR>` は **既存 worktree を破壊しない**。`develop issue` /
  `develop pr` の `ensure_worktree` は `git reset --hard` + `git clean -fd` +
  `git pull --rebase` を実行するが、resume はその逆で、未コミットの変更や
  rebase 中の状態を保持したまま新しい tmux セッションだけ立て直す。issue 系は
  `git for-each-ref refs/heads/develop/issue-<N>-*` で worktree branch を列挙し、
  複数あれば fzf で選ばせる。pr 系は PR の `headRefName` をそのまま branch と
  して扱う。worktree が見つからない場合は明示エラーで終わる (新規作成は
  `develop issue` の責務)。`common::find_existing_worktree` は **refresh しない**
  ヘルパで、resume だけが使う。新規作成パス (`ensure_worktree`) と混同しない。
- `rai develop pr` は同一リポジトリ PR 専用。fork 由来 PR (`headRepositoryOwner` が
  base owner と異なる) は明示エラーで弾く。fork 対応は将来課題。
- 既存 worktree が見つかったときは `git reset --hard HEAD` + `git clean -fd` +
  `git pull --rebase` で **無人で** 最新化する。旧 attach / force-recreate / abort の
  対話プロンプトは廃止された。`gwq remove` で巻き戻すのは新規作成した worktree のみ。

## Pickup Topics

### Valid `--permission-mode` Values (verified 2026-04-30)

```
acceptEdits  auto  bypassPermissions  default  dontAsk  plan
```

`PermissionMode` enum は各 variant に `#[value(name = "...")]` を付けて、
`claude --permission-mode` がそのまま受け取る文字列にシリアライズされること。

### Good Example

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PermissionMode {
    #[value(name = "acceptEdits")]
    AcceptEdits,
    #[value(name = "dontAsk")]
    DontAsk,
    // ... all six variants with explicit name annotations
}
```

### Bad Example

```rust
// Missing #[value(name = "...")] -- clap serialises as snake_case,
// which does NOT match what `claude --permission-mode` expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PermissionMode {
    AcceptEdits,
    DontAsk,
}
```

### Good Example: 外部コマンドを呼ぶ

```rust
use rai_core::shell;

let st = shell::user_shell_argv(&["gh", "pr", "list", "--state", "open"])
    .status()?;
```

### Bad Example: 外部コマンドを直叩き

```rust
// fish の function や alias が解決されず ENOENT になる。禁止。
let st = std::process::Command::new("gh")
    .args(["pr", "list", "--state", "open"])
    .status()?;
```
