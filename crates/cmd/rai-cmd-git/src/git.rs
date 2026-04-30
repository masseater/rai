//! `git` コマンドを呼ぶための薄いヘルパ。

use std::process::Output;

use anyhow::{anyhow, Context, Result};
use rai_core::shell;

/// `git <args...>` を実行して stdout を返す。失敗時は stderr を含むエラー。
pub fn run(args: &[&str]) -> Result<String> {
    let out = capture(args)?;
    if !out.status.success() {
        return Err(anyhow!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn capture(args: &[&str]) -> Result<Output> {
    let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 1);
    argv.push("git");
    argv.extend_from_slice(args);
    shell::user_shell_argv(&argv)
        .output()
        .with_context(|| format!("failed to spawn `git {}`", args.join(" ")))
}

pub fn rev_parse(rev: &str) -> Result<String> {
    run(&["rev-parse", rev])
}

pub fn rev_parse_args(args: &[&str]) -> Result<String> {
    let mut full = vec!["rev-parse"];
    full.extend_from_slice(args);
    run(&full)
}

pub fn current_branch() -> Result<String> {
    run(&["symbolic-ref", "--quiet", "--short", "HEAD"])
}
