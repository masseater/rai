//! ユーザーシェル経由で外部コマンドを起動するためのユーティリティ。
//!
//! `rai` は外部コマンドを必ずユーザーの対話シェル (`$SHELL`) 経由で起動する。
//! 詳細は `docs/specs/16-shell-execution-policy.md` と `AGENTS.md` の
//! "External Process Execution" を参照。

use std::path::Path;
use std::process::Command;

/// 起動先シェルの種別。クォーティング規則と shell 構文の差を吸収するために持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// sh / bash / zsh などの POSIX 系。
    Posix,
    /// fish。シングルクォート内のエスケープ規則と pipefail まわりが POSIX と異なる。
    Fish,
}

/// 環境変数 `SHELL` → `/bin/sh` の順で実行用シェルのパスを決定する。
pub fn user_shell_path() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// シェルパス文字列から種別を判定する。basename が `fish` のときだけ Fish。
pub fn detect_shell_kind(path: &str) -> Shell {
    let base = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if base == "fish" {
        Shell::Fish
    } else {
        Shell::Posix
    }
}

/// 現在の環境のシェルパスと種別を一括で返す便利関数。
pub fn detect_user_shell() -> (String, Shell) {
    let path = user_shell_path();
    let kind = detect_shell_kind(&path);
    (path, kind)
}

/// `Shell` に対応するクォート関数を返す。
pub fn quote_for(shell: Shell) -> fn(&str) -> String {
    match shell {
        Shell::Posix => quote_posix,
        Shell::Fish => quote_fish,
    }
}

/// POSIX 系シェルでの安全な引用 (`shell_words::quote` を String に持ち上げただけ)。
pub fn quote_posix(s: &str) -> String {
    shell_words::quote(s).to_string()
}

/// fish のシングルクォート規則に従ったエスケープ。
///
/// fish のシングルクォート内では `\\` と `\'` のみがエスケープシーケンス。
/// それ以外の文字はそのまま。
pub fn quote_fish(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// 任意のパスをそのシェルで安全に展開できる文字列にクォートする。
pub fn quote_path(shell: Shell, path: &Path) -> String {
    quote_for(shell)(&path.display().to_string())
}

/// `shell -c "<cmd>"` の `Command` を作る。stdout / stderr / stdin は呼び出し側で設定する。
///
/// `cmd` はシェルが解釈する **文字列** であり、引数の安全な構築は呼び出し側の責任。
pub fn shell_command(shell_path: &str, cmd: &str) -> Command {
    let mut c = Command::new(shell_path);
    c.arg("-c").arg(cmd);
    c
}

/// 現在の環境のシェルで `cmd` 文字列を実行する `Command` を返す。
///
/// `Command::new("<bin>")` の代わりにこれを使うことで、ユーザーが定義した
/// シェル関数 / alias / 組込みも解決される。
pub fn user_shell_command(cmd: &str) -> Command {
    shell_command(&user_shell_path(), cmd)
}

/// 現在の環境のシェルでバイナリと引数列を実行する `Command` を返す。
///
/// `argv` は引数列の意味で渡し、内部でシェル種別に応じてクォートしてから
/// `shell -c` 用の 1 文字列に組み立てる。これにより呼び出し側は
/// `Command::new(...).args(...)` と同じ感覚で書ける。
pub fn user_shell_argv(argv: &[&str]) -> Command {
    let (shell_path, kind) = detect_user_shell();
    let q = quote_for(kind);
    let cmd: Vec<String> = argv.iter().map(|s| q(s)).collect();
    shell_command(&shell_path, &cmd.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_fish() {
        assert_eq!(detect_shell_kind("/usr/local/bin/fish"), Shell::Fish);
        assert_eq!(detect_shell_kind("fish"), Shell::Fish);
    }

    #[test]
    fn detect_posix() {
        assert_eq!(detect_shell_kind("/bin/sh"), Shell::Posix);
        assert_eq!(detect_shell_kind("/bin/bash"), Shell::Posix);
        assert_eq!(detect_shell_kind("/usr/bin/zsh"), Shell::Posix);
        assert_eq!(detect_shell_kind(""), Shell::Posix);
    }

    #[test]
    fn quote_fish_basic() {
        assert_eq!(quote_fish("hello"), "'hello'");
        assert_eq!(quote_fish("it's"), "'it\\'s'");
        assert_eq!(quote_fish("a\\b"), "'a\\\\b'");
    }

    #[test]
    fn quote_posix_basic() {
        // shell_words::quote は単純な英数字はそのまま、空白を含むと '...' でくるむ。
        assert_eq!(quote_posix("hello"), "hello");
        assert_eq!(quote_posix("a b"), "'a b'");
    }
}
