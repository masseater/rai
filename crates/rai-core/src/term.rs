//! ターミナル状態の安全な操作 (DECSTBM スクロール領域 + 下部ステータス行)。
//!
//! `StatusBar::enable(lines)` で下段 `lines` 行を確保 → ステータス行として使う。
//! `Drop` で必ず元の状態 (full screen scroll region / autowrap on / 下段クリア) に戻す。
//!
//! - alt screen には入らない (DECSTBM だけで成立させる)
//! - autowrap を一時 OFF (子プロセス出力で右端折返し時に状態行を巻き込まないため)
//! - tty でない / 行数が小さすぎる場合は `enable()` が `Ok(None)` を返して degrade

use std::io::{self, IsTerminal, Write};

const ESC: &str = "\x1b";

/// ターミナル下部に固定表示するためのスクロール領域 RAII。
pub struct StatusBar {
    rows: u16,
    cols: u16,
    lines: u16,
    last_msgs: Vec<String>,
}

impl StatusBar {
    /// 端末下部に `lines` 行のステータス領域を確保する。
    /// tty でない / 行数が足りない / `lines == 0` の場合は `Ok(None)`。
    pub fn enable(lines: u16) -> io::Result<Option<Self>> {
        if lines == 0 || !io::stdout().is_terminal() {
            return Ok(None);
        }
        let (cols, rows) = match crossterm::terminal::size() {
            Ok(sz) => sz,
            Err(_) => return Ok(None),
        };
        if rows < lines + 2 {
            return Ok(None);
        }
        let bar = Self {
            rows,
            cols,
            lines,
            last_msgs: vec![String::new(); lines as usize],
        };
        bar.apply_region()?;
        Ok(Some(bar))
    }

    /// スクロール領域 + autowrap off を適用する。
    /// 子プロセスが alt screen を抜けた直後など、状態が壊れた時に再呼び出しできる。
    pub fn apply_region(&self) -> io::Result<()> {
        let mut out = io::stdout().lock();
        // autowrap off (DECAWM)
        write!(out, "{ESC}[?7l")?;
        // scroll region: 1..rows-lines (1-indexed)。下端 lines 行を status 用に空ける。
        let scroll_bottom = self.rows.saturating_sub(self.lines).max(1);
        write!(out, "{ESC}[1;{scroll_bottom}r")?;
        // カーソルを領域内に戻す。
        write!(out, "{ESC}[1;1H")?;
        out.flush()
    }

    /// SIGWINCH などで端末サイズが変わったら呼ぶ。
    pub fn resize(&mut self) -> io::Result<()> {
        let (cols, rows) = crossterm::terminal::size()?;
        self.cols = cols;
        self.rows = rows;
        self.apply_region()?;
        let last: Vec<String> = self.last_msgs.clone();
        let refs: Vec<&str> = last.iter().map(String::as_str).collect();
        self.draw(&refs)
    }

    /// 状態行を 1 回描く。`msgs[i]` は status 領域の上から i 行目に対応する。
    /// 各行は端末幅で切り詰める。
    pub fn draw(&mut self, msgs: &[&str]) -> io::Result<()> {
        let mut out = io::stdout().lock();
        write!(out, "{ESC}7")?; // DECSC: save cursor
        for i in 0..self.lines {
            let row = self.rows.saturating_sub(self.lines).saturating_add(1 + i);
            let msg = msgs.get(i as usize).copied().unwrap_or("");
            let trunc = truncate(msg, self.cols as usize);
            write!(out, "{ESC}[{row};1H")?;
            write!(out, "{ESC}[2K")?; // clear entire line
            write!(out, "{ESC}[7m{trunc}{ESC}[0m")?; // reverse video
        }
        write!(out, "{ESC}8")?; // DECRC: restore cursor
        out.flush()?;
        let mut next: Vec<String> = msgs
            .iter()
            .take(self.lines as usize)
            .map(|s| (*s).to_string())
            .collect();
        while next.len() < self.lines as usize {
            next.push(String::new());
        }
        self.last_msgs = next;
        Ok(())
    }
}

impl Drop for StatusBar {
    fn drop(&mut self) {
        // best-effort: 失敗しても何もできない。
        let mut out = io::stdout();
        let _ = write!(out, "{ESC}[r"); // scroll region 解除
        let _ = write!(out, "{ESC}[?7h"); // autowrap 復活
        for i in 0..self.lines {
            let row = self.rows.saturating_sub(i);
            let _ = write!(out, "{ESC}[{row};1H");
            let _ = write!(out, "{ESC}[2K");
        }
        let _ = out.flush();
    }
}

/// パニック時にも端末を最低限の状態に戻すグローバルフックを入れる。
pub fn install_panic_restore() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut out = io::stdout();
        let _ = writeln!(out, "{ESC}[r{ESC}[?7h");
        let _ = out.flush();
        prev(info);
    }));
}

/// `s` の先頭 `n` 文字 (chars 単位) を返す。
pub fn truncate(s: &str, n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_passes_through() {
        assert_eq!(truncate("abc", 5), "abc");
    }

    #[test]
    fn truncate_long_appends_ellipsis() {
        assert_eq!(truncate("abcdef", 4), "abc…");
    }

    #[test]
    fn truncate_zero() {
        assert_eq!(truncate("abc", 0), "");
    }
}
