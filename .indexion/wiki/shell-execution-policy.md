# Shell Execution Policy

`rai` は **個人向け CLI** であり、ユーザーが日常使うシェル (fish / zsh / bash 等) の機能 — function, alias, builtin, パイプ, リダイレクト — を **そのまま** 受け取れることを体験の核とする。仕様: `docs/specs/16-shell-execution-policy.md`。

## 基本原則

**外部コマンドは必ずユーザーのデフォルトシェル (`$SHELL`) 経由で起動する。**

`std::process::Command::new("<bin>")` の `execvp` 直叩きは **禁止**。理由:

- `execvp` は PATH 上の実バイナリしか解決できない
- fish の `function ccs_print` や zsh の `alias` は実バイナリではないので ENOENT になる
- `rai develop` の `--engine-cmd "ccs_print c1 ..."` のようなユーザー入力が動かなくなる

代わりに `Command::new($SHELL).arg("-c").arg("<cmd>")` でラップする。

## 正規 API: `rai_core::shell`

`Command::new` を新たに書かず、必ずこのモジュールを通す。

| 関数 | 用途 |
|---|---|
| `shell::user_shell_path()` | `$SHELL` を解決。未設定なら `/bin/sh`。 |
| `shell::detect_shell_kind(path)` | basename が `fish` なら `Shell::Fish`、それ以外は `Shell::Posix`。 |
| `shell::detect_user_shell()` | パスと種別を一括取得。 |
| `shell::user_shell_argv(&["bin", "arg1", "arg2"])` | 引数列を `$SHELL -c` で実行する `Command`。`Command::new(...).args(...)` の置き換え。 |
| `shell::user_shell_command("foo \| bar")` | ユーザー入力のシェル文字列をそのまま `$SHELL -c` で実行。 |
| `shell::shell_command(shell_path, cmd)` | パス指定の低レベル版。`Command::new(shell_path)` を呼ぶのはここだけ (ポリシー上の唯一の例外)。 |
| `shell::quote_for(kind)` / `shell::quote_posix` / `shell::quote_fish` | シェル種別ごとのクォート。 |
| `shell::quote_path(kind, path)` | パスを引用済み文字列に。 |

## 引用ルール

POSIX と fish はシングルクォート内のエスケープが異なるので別関数。

- POSIX: `shell_words::quote` をそのまま。`a b` → `'a b'`。
- fish: シングルクォート内は `\\` と `\'` のみエスケープ。`it's` → `'it\'s'`、`a\b` → `'a\\b'`。

## シェル別構文

ユーザーシェル経由でラップした全体コマンドはユーザーシェルで実行される。POSIX と fish の構文を混在させない:

| 機能 | POSIX | Fish |
|---|---|---|
| pipefail | `set -o pipefail` | (なし。代替: `$pipestatus` を走査) |
| 直前 exit code | `$?` | `$status` |
| 条件分岐 | `[ "$x" -ne 0 ]` | `test "$x" -ne 0` |
| 複合ブロック | `( ... ); rc=$?` | `begin; ...; end` |

`rai develop` の `build_posix_agent_block` / `build_fish_agent_block` は両方を抱える参考実装。新規にラップを書く場合も同じパターンで分岐させる。

## ユーザー入力の扱い

`--engine-cmd`, `--on-update`, `--agent-cmd` のような **ユーザー入力のシェル文字列** は:

- **トークン分割しない**: そのまま `$SHELL -c` に渡す。fish function やパイプ・リダイレクトを保持するため。
- **クォートし直さない**: ユーザーは既にシェル文字列として書いている。

逆に rai 内部で引数列を組み立てて起動するときは `user_shell_argv(&[...])` を使い、各要素を `quote_for(kind)` で引用する。

## 例外: `execvp` 直叩きが許される唯一のケース

`shell::shell_command` の内部で `Command::new(shell_path)` を呼ぶときだけ。それ以外のところで `Command::new` を書いてはいけない。

## Good Example

```rust
use rai_core::shell;

let st = shell::user_shell_argv(&["gh", "pr", "list", "--state", "open"])
    .status()?;
```

## Bad Example

```rust
// fish function / alias が解決されず ENOENT になる。禁止。
let st = std::process::Command::new("gh")
    .args(["pr", "list", "--state", "open"])
    .status()?;
```

## See Also

- [rai-core](wiki://rai-core) — `shell` モジュールの位置づけ
- [cmd-develop](wiki://cmd-develop) — `--engine-cmd` プレースホルダ展開の応用例
- [architecture](wiki://architecture) — ポリシーが workspace 全体に効く理由
