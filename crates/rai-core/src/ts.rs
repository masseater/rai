//! fish-compat な `[YYYY-MM-DD HH:MM:SS] message` ログフォーマッタ。
//!
//! `tracing` とは別系統で、ユーザ向け stdout に「移行コスト 0」のログを
//! 流したい subcommand (例: `rai pair`) が直接使う。

use chrono::Local;

/// 現在ローカル時刻を `YYYY-MM-DD HH:MM:SS` で返す。
pub fn now_str() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// `[ts] message` の 1 行を組み立てて返す。改行は付けない。
pub fn line(msg: impl AsRef<str>) -> String {
    format!("[{}] {}", now_str(), msg.as_ref())
}

/// `[ts] message` を stdout に 1 行 println する。
pub fn println(msg: impl AsRef<str>) {
    println!("{}", line(msg));
}
