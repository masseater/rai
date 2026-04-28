//! `rai claude format` — `claude --output-format stream-json` の NDJSON を整形する stdin/stdout フィルタ。

use std::io::{self, BufRead, Write};

use anyhow::Context;
use clap::Args;
use rai_core::{cli::Run, Ctx, Result};
use serde_json::Value;

#[derive(Debug, Args)]
pub struct Cmd {
    /// 絵文字を ASCII fallback に置換する。
    #[arg(long)]
    no_emoji: bool,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        // SIGPIPE で即終了 (Rust デフォルトは SIG_IGN しているので戻す)。
        #[cfg(unix)]
        // SAFETY: signal(2) is async-signal-safe; restoring default disposition is documented.
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }

        let glyphs = Glyphs::new(self.no_emoji);
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut out = stdout.lock();
        let mut handle = stdin.lock();
        let mut line = String::new();
        loop {
            line.clear();
            match handle.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => return Err(anyhow::Error::new(e).context("stdin read failed")),
            }
            let trimmed = line.trim_end_matches(['\n', '\r']);
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue, // 非 JSON 行はスキップ
            };
            if let Err(e) = render_event(&mut out, &glyphs, &value) {
                if is_broken_pipe(&e) {
                    return Ok(());
                }
                return Err(e).context("render failed");
            }
        }
        out.flush().ok();
        Ok(())
    }
}

struct Glyphs {
    info: &'static str,
    text: &'static str,
    tool: &'static str,
    arrow: &'static str,
    think: &'static str,
    ok: &'static str,
    err: &'static str,
    unknown: &'static str,
}

impl Glyphs {
    fn new(no_emoji: bool) -> Self {
        if no_emoji {
            Self {
                info: "[i]",
                text: "[text]",
                tool: "[tool]",
                arrow: "[->]",
                think: "[think]",
                ok: "[ok]",
                err: "[err]",
                unknown: "[?]",
            }
        } else {
            Self {
                info: "ℹ️ ",
                text: "💬",
                tool: "🔧",
                arrow: "↩️ ",
                think: "🧠",
                ok: "✅",
                err: "❌",
                unknown: "❓",
            }
        }
    }
}

fn render_event(out: &mut impl Write, g: &Glyphs, v: &Value) -> io::Result<()> {
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    match ty {
        "system" => render_system(out, g, v),
        "assistant" | "user" => render_message(out, g, ty, v),
        "result" => render_result(out, g, v),
        "error" => render_error(out, g, v),
        "" => writeln!(out, "{} type missing: {}", g.unknown, v),
        other => writeln!(out, "{} type: {} ({})", g.unknown, other, v),
    }
}

fn render_system(out: &mut impl Write, g: &Glyphs, v: &Value) -> io::Result<()> {
    let subtype = v.get("subtype").and_then(|x| x.as_str()).unwrap_or("");
    if subtype == "init" {
        let session = v.get("session_id").and_then(|x| x.as_str()).unwrap_or("?");
        let model = v.get("model").and_then(|x| x.as_str()).unwrap_or("?");
        writeln!(out, "{} session={} model={}", g.info, session, model)
    } else {
        let summary = compact_summary(v);
        writeln!(out, "{} system[{}]: {}", g.info, subtype, summary)
    }
}

fn render_message(out: &mut impl Write, g: &Glyphs, ty: &str, v: &Value) -> io::Result<()> {
    let blocks = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array());
    let role_label = if ty == "assistant" {
        "assistant"
    } else {
        "user"
    };
    if let Some(blocks) = blocks {
        for block in blocks {
            render_block(out, g, role_label, block)?;
        }
    } else {
        // content が無い場合はそのまま raw を出す。
        writeln!(out, "{} {}: {}", g.unknown, role_label, v)?;
    }
    Ok(())
}

fn render_block(out: &mut impl Write, g: &Glyphs, role: &str, b: &Value) -> io::Result<()> {
    let ty = b.get("type").and_then(|x| x.as_str()).unwrap_or("");
    match ty {
        "text" => {
            let text = b.get("text").and_then(|x| x.as_str()).unwrap_or("");
            writeln!(out, "{} {}: {}", g.text, role, text)
        }
        "thinking" => {
            let text = b.get("thinking").and_then(|x| x.as_str()).unwrap_or("");
            writeln!(out, "{} {}: {}", g.think, role, text)
        }
        "tool_use" => {
            let name = b.get("name").and_then(|x| x.as_str()).unwrap_or("?");
            let input = b
                .get("input")
                .map(|x| serde_json::to_string(x).unwrap_or_default())
                .unwrap_or_default();
            writeln!(out, "{} tool_use[{}]: {}", g.tool, name, input)
        }
        "tool_result" => {
            let id = b.get("tool_use_id").and_then(|x| x.as_str()).unwrap_or("?");
            let content = b
                .get("content")
                .map(stringify_tool_content)
                .unwrap_or_default();
            writeln!(out, "{} tool_result[{}]: {}", g.arrow, id, content)
        }
        other => writeln!(out, "{} block type: {} ({})", g.unknown, other, b),
    }
}

fn stringify_tool_content(c: &Value) -> String {
    if let Some(s) = c.as_str() {
        return s.to_string();
    }
    if let Some(arr) = c.as_array() {
        let mut parts = Vec::with_capacity(arr.len());
        for item in arr {
            if let Some(s) = item.get("text").and_then(|x| x.as_str()) {
                parts.push(s.to_string());
            } else {
                parts.push(item.to_string());
            }
        }
        return parts.join("\n");
    }
    c.to_string()
}

fn render_result(out: &mut impl Write, g: &Glyphs, v: &Value) -> io::Result<()> {
    let cost = v
        .get("total_cost_usd")
        .and_then(|x| x.as_f64())
        .map(|x| format!("${x:.4}"))
        .unwrap_or_else(|| "?".into());
    let turns = v
        .get("num_turns")
        .and_then(|x| x.as_i64())
        .map(|x| x.to_string())
        .unwrap_or_else(|| "?".into());
    let dur = v
        .get("duration_ms")
        .and_then(|x| x.as_i64())
        .map(|x| format!("{x}ms"))
        .unwrap_or_else(|| "?".into());
    writeln!(
        out,
        "{} done cost={} turns={} duration={}",
        g.ok, cost, turns, dur
    )
}

fn render_error(out: &mut impl Write, g: &Glyphs, v: &Value) -> io::Result<()> {
    let msg = v
        .get("message")
        .and_then(|x| x.as_str())
        .unwrap_or("(no message)");
    writeln!(out, "{} error: {}", g.err, msg)
}

fn compact_summary(v: &Value) -> String {
    let mut interesting = serde_json::Map::new();
    if let Value::Object(map) = v {
        for (k, val) in map {
            if k == "type" || k == "subtype" {
                continue;
            }
            interesting.insert(k.clone(), val.clone());
        }
    }
    if interesting.is_empty() {
        String::new()
    } else {
        serde_json::to_string(&Value::Object(interesting)).unwrap_or_default()
    }
}

fn is_broken_pipe(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::BrokenPipe
}
