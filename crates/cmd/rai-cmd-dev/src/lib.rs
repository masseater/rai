//! `rai dev` — ghq + gwq + fzf でリポジトリ/worktree を選ぶ。
//!
//! 仕様: `docs/specs/05-dev.md` 参照。

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context};
use clap::Args;
use rai_core::{cli::Run, proc, Ctx, Result};

#[derive(Debug, Args)]
pub struct Cmd {
    /// 候補を絞り込む正規表現 (Rust regex は使わず、fzf に query として渡す)。
    #[arg(long)]
    filter: Option<String>,

    /// 全候補を出す (デフォルトは fish 版同等の表示候補のみ)。
    #[arg(long)]
    all: bool,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let mut entries: Vec<PathBuf> = Vec::new();
        let mut seen: BTreeSet<PathBuf> = BTreeSet::new();

        for path in collect_ghq()? {
            if seen.insert(path.clone()) {
                entries.push(path);
            }
        }
        for path in collect_gwq()? {
            if seen.insert(path.clone()) {
                entries.push(path);
            }
        }

        if entries.is_empty() {
            return Ok(());
        }

        let ghq_root = ghq_root().ok();
        let labels: Vec<String> = entries
            .iter()
            .map(|p| label_for(p, ghq_root.as_deref()))
            .collect();
        let lines: Vec<String> = labels
            .iter()
            .zip(entries.iter())
            .map(|(label, path)| format!("{label}\t{}", path.display()))
            .collect();

        let mut fzf = Command::new("fzf");
        fzf.arg("--with-nth=1").arg("--delimiter=\t");
        if let Some(q) = &self.filter {
            fzf.arg("--query").arg(q);
        }
        if !self.all {
            // current behavior: just show all; --all is forwarded as a no-op for parity.
        }

        let mut child = fzf
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("failed to spawn `fzf`. Is it installed and on PATH?")?;

        {
            let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("fzf stdin"))?;
            for line in &lines {
                writeln!(stdin, "{line}").ok();
            }
        }

        let output = child.wait_with_output()?;
        if !output.status.success() {
            // fzf cancel = 130
            std::process::exit(proc::shell_exit_code(&output.status));
        }

        let s = String::from_utf8_lossy(&output.stdout);
        let line = s.lines().next().unwrap_or("");
        if let Some((_, path)) = line.split_once('\t') {
            println!("{path}");
        }
        Ok(())
    }
}

fn collect_ghq() -> Result<Vec<PathBuf>> {
    if proc::find_in_path("ghq").is_none() {
        return Ok(Vec::new());
    }
    let out = Command::new("ghq").args(["list", "--full-path"]).output()?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_lines(&out.stdout))
}

fn collect_gwq() -> Result<Vec<PathBuf>> {
    if proc::find_in_path("gwq").is_none() {
        return Ok(Vec::new());
    }
    let out = Command::new("gwq").args(["list", "--full-path"]).output()?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_lines(&out.stdout))
}

fn parse_lines(bytes: &[u8]) -> Vec<PathBuf> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn ghq_root() -> Result<PathBuf> {
    let out = Command::new("ghq").arg("root").output()?;
    if !out.status.success() {
        return Err(anyhow!("`ghq root` failed"));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Ok(PathBuf::from(s.trim()))
}

fn label_for(path: &Path, ghq_root: Option<&Path>) -> String {
    let mut p = path.display().to_string();
    if let Some(root) = ghq_root {
        let prefix = format!("{}/", root.display());
        if let Some(rest) = p.strip_prefix(&prefix) {
            p = rest.to_string();
        }
    }
    if let Some(rest) = p.strip_prefix("github.com/") {
        return rest.to_string();
    }
    p
}
