//! 子プロセス起動ヘルパ。
//!
//! - PATH 上のバイナリを探すユーティリティ (`gtimeout` / `timeout` 検出に使う)
//! - `ExitStatus` を「シェルでの $? 慣習」に揃えた整数に変換するヘルパ
//!
//! シェル経由でコマンドを起動する API は `crate::shell` を参照。

use std::path::{Path, PathBuf};
use std::process::ExitStatus;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

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
