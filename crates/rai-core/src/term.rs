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
    last_rendered: Vec<String>,
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
            last_rendered: vec![String::new(); lines as usize],
        };
        bar.apply_region()?;
        bar.clear_status_area()?;
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
        // 親ログと子プロセス出力が常に status 領域の上で流れるよう、領域末尾へ戻す。
        write!(out, "{ESC}[{scroll_bottom};1H")?;
        out.flush()
    }

    /// 親プロセスがログを書く直前などに、カーソルをスクロール領域へ戻す。
    pub fn prepare_output(&self) -> io::Result<()> {
        let mut out = io::stdout().lock();
        let scroll_bottom = self.rows.saturating_sub(self.lines).max(1);
        write!(out, "{ESC}[{scroll_bottom};1H")?;
        out.flush()
    }

    /// 子プロセスが端末状態を変更した可能性がある後に、領域と表示を復旧する。
    pub fn resume(&mut self) -> io::Result<()> {
        self.apply_region()?;
        self.redraw()
    }

    /// SIGWINCH などで端末サイズが変わったら呼ぶ。
    pub fn resize(&mut self) -> io::Result<()> {
        let (cols, rows) = crossterm::terminal::size()?;
        self.cols = cols;
        self.rows = rows;
        self.apply_region()?;
        self.clear_status_area()?;
        self.redraw()
    }

    /// 状態行を 1 回描く。`msgs[i]` は status 領域の上から i 行目に対応する。
    /// 各行は端末幅で切り詰める。
    pub fn draw(&mut self, msgs: &[&str]) -> io::Result<()> {
        self.draw_inner(msgs, false)
    }

    /// 現在覚えている状態行を強制的に描き直す。
    pub fn redraw(&mut self) -> io::Result<()> {
        let msgs = self.last_msgs.clone();
        let refs: Vec<&str> = msgs.iter().map(String::as_str).collect();
        self.draw_inner(&refs, true)
    }

    fn draw_inner(&mut self, msgs: &[&str], force: bool) -> io::Result<()> {
        let mut out = io::stdout().lock();
        let logical = self.normalize_msgs(msgs);
        let rendered = self.render_msgs(msgs);
        if !force && rendered == self.last_rendered {
            self.last_msgs = logical;
            return Ok(());
        }

        write!(out, "{ESC}7")?; // DECSC: save cursor
        for i in 0..self.lines {
            let idx = i as usize;
            let Some(line) = rendered.get(idx) else {
                continue;
            };
            if !force && self.last_rendered.get(idx) == Some(line) {
                continue;
            }
            let row = self.rows.saturating_sub(self.lines).saturating_add(1 + i);
            write!(out, "{ESC}[{row};1H")?;
            write!(out, "{ESC}[7m{line}{ESC}[0m")?; // reverse video
        }
        write!(out, "{ESC}8")?; // DECRC: restore cursor
        out.flush()?;
        self.last_msgs = logical;
        self.last_rendered = rendered;
        Ok(())
    }

    fn clear_status_area(&self) -> io::Result<()> {
        let mut out = io::stdout().lock();
        write!(out, "{ESC}7")?; // DECSC: save cursor
        for i in 0..self.lines {
            let row = self.rows.saturating_sub(self.lines).saturating_add(1 + i);
            write!(out, "{ESC}[{row};1H")?;
            write!(out, "{ESC}[2K")?;
        }
        write!(out, "{ESC}8")?; // DECRC: restore cursor
        out.flush()
    }

    fn render_msgs(&self, msgs: &[&str]) -> Vec<String> {
        (0..self.lines as usize)
            .map(|i| render_status_line(msgs.get(i).copied().unwrap_or(""), self.cols as usize))
            .collect()
    }

    fn normalize_msgs(&self, msgs: &[&str]) -> Vec<String> {
        (0..self.lines as usize)
            .map(|i| msgs.get(i).copied().unwrap_or("").to_string())
            .collect()
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

fn render_status_line(s: &str, cols: usize) -> String {
    let mut line = truncate(s, cols);
    let len = line.chars().count();
    if len < cols {
        line.push_str(&" ".repeat(cols - len));
    }
    line
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

    #[test]
    fn render_status_line_pads_to_width() {
        assert_eq!(render_status_line("abc", 5), "abc  ");
    }

    #[test]
    fn render_status_line_truncates_to_width() {
        assert_eq!(render_status_line("abcdef", 4), "abc…");
    }
}
