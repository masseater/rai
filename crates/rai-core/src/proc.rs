//! 子プロセス起動ヘルパ。
//!
//! - shell -c "<cmd>" を組み立てる薄いラッパ
//! - PATH 上のバイナリを探すユーティリティ (`gtimeout` / `timeout` 検出に使う)
//! - `ExitStatus` を「シェルでの $? 慣習」に揃えた整数に変換するヘルパ

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

/// 環境変数 `SHELL` → `/bin/sh` の順で実行用シェルを決定する。
pub fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// `shell -c "cmd"` の `Command` を作る。stdout/stderr は呼び出し側で設定する想定。
pub fn shell_command(shell: &str, cmd: &str) -> Command {
    let mut c = Command::new(shell);
    c.arg("-c").arg(cmd);
    c
}

/// PATH 上で最初に見つかった実行可能ファイルのパスを返す。
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = Path::new(&dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// `gtimeout` (GNU coreutils on macOS) → `timeout` の順で探す。どちらも無ければ None。
pub fn find_timeout_bin() -> Option<PathBuf> {
    find_in_path("gtimeout").or_else(|| find_in_path("timeout"))
}

/// `ExitStatus` をシェル慣習の整数に変換 (signal kill は 128 + signo)。
#[cfg(unix)]
pub fn shell_exit_code(status: &ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    if let Some(sig) = status.signal() {
        return 128 + sig;
    }
    1
}

#[cfg(not(unix))]
pub fn shell_exit_code(status: &ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}
