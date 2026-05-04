//! `rai develop` の Issue / PR で共通するヘルパー群。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use chrono::Local;
use clap::{Args, ValueEnum};
use rai_core::{
    shell::{self, Shell},
    Result,
};
use serde::Deserialize;

pub const DEFAULT_ENGINE_CMD: &str = "ccs c1 --print --output-format stream-json --verbose {PERMISSION_MODE} -- {PROMPT} | {RAI} claude format";

/// `rai develop {issue,pr}` 共通の agent 起動関連オプション。
#[derive(Debug, Args, Clone)]
pub struct AgentArgs {
    /// agent CLI の起動コマンド (shell 文字列)。
    ///
    /// プレースホルダ:
    /// - `{PROMPT}`        : 現タスクの prompt (shell-quoted)
    /// - `{PERMISSION_MODE}`: `--permission-mode <MODE>` 一式 (`--permission-mode` 未指定なら空)
    /// - `{RAI}`           : 実行中の `rai` バイナリ絶対パス (shell-quoted)
    ///
    /// プレースホルダを 1 つも含まない文字列を渡した場合は legacy 互換動作で末尾に
    /// `{PERMISSION_MODE}` と `{PROMPT}` を append する。
    #[arg(long, short = 'e', value_name = "CMD", default_value = DEFAULT_ENGINE_CMD)]
    pub engine_cmd: String,

    /// prompt をファイルから読み込む。
    #[arg(long, value_name = "FILE")]
    pub prompt_template: Option<PathBuf>,

    /// tmux を介さず前面で実行 (デバッグ用)。
    #[arg(long)]
    pub no_tmux: bool,

    /// agent 終了後の自動 commit / push / PR 作成 (or push) を無効化する。
    #[arg(long)]
    pub no_auto_publish: bool,

    /// agent (`claude`) に渡す `--permission-mode` を明示する。
    #[arg(long, value_name = "MODE", value_enum)]
    pub permission_mode: Option<PermissionMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PermissionMode {
    #[value(name = "acceptEdits")]
    AcceptEdits,
    #[value(name = "auto")]
    Auto,
    #[value(name = "bypassPermissions")]
    BypassPermissions,
    #[value(name = "default")]
    Default,
    #[value(name = "dontAsk")]
    DontAsk,
    #[value(name = "plan")]
    Plan,
}

impl PermissionMode {
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::AcceptEdits => "acceptEdits",
            Self::Auto => "auto",
            Self::BypassPermissions => "bypassPermissions",
            Self::Default => "default",
            Self::DontAsk => "dontAsk",
            Self::Plan => "plan",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Flavor {
    Issue,
    Pr,
}

impl Flavor {
    pub fn label(self) -> &'static str {
        match self {
            Flavor::Issue => "issue",
            Flavor::Pr => "pr",
        }
    }
}

#[derive(Debug)]
pub struct Worktree {
    pub path: PathBuf,
    pub created: bool,
}

/// tmux セッション + agent 実行に必要な入力。Issue / PR で共通。
pub struct LaunchContext<'a> {
    pub repo: &'a str,
    pub branch: &'a str,
    pub flavor: Flavor,
    pub number: u64,
    pub prompt: &'a str,
    pub finalizer: Option<&'a str>,
    pub agent: &'a AgentArgs,
}

pub fn read_prompt_template(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .with_context(|| format!("failed to read prompt template: {}", path.display()))
}

pub fn build_engine_cmd(engine_cmd: &str, permission_mode: Option<PermissionMode>) -> String {
    let flag = match permission_mode {
        Some(mode) => format!("--permission-mode {}", mode.as_arg()),
        None => String::new(),
    };
    if engine_cmd.contains("{PERMISSION_MODE}") {
        return engine_cmd.replace("{PERMISSION_MODE}", &flag);
    }
    if let Some(mode) = permission_mode {
        format!("{engine_cmd} --permission-mode {}", mode.as_arg())
    } else {
        engine_cmd.to_string()
    }
}

pub fn build_agent_shell_command(
    engine_cmd: &str,
    prompt: &str,
    rai_exe: &str,
    finalizer: Option<&str>,
    shell_kind: Shell,
) -> String {
    let quote = shell::quote_for(shell_kind);
    let has_placeholder = engine_cmd.contains("{PROMPT}") || engine_cmd.contains("{RAI}");
    let agent = if has_placeholder {
        engine_cmd
            .replace("{PROMPT}", &quote(prompt))
            .replace("{RAI}", &quote(rai_exe))
    } else {
        format!("{} {}", engine_cmd, quote(prompt))
    };
    match shell_kind {
        Shell::Posix => build_posix_agent_block(&agent, finalizer),
        Shell::Fish => build_fish_agent_block(&agent, finalizer),
    }
}

fn build_posix_agent_block(agent: &str, finalizer: Option<&str>) -> String {
    let agent_block = format!("set -o pipefail; ({agent})");
    match finalizer {
        Some(finalizer) => format!(
            "{agent_block}; __rai_agent_status=$?; if [ \"$__rai_agent_status\" -ne 0 ]; then echo \"rai: agent exited with status $__rai_agent_status; skip auto publish\" >&2; exit \"$__rai_agent_status\"; fi; {finalizer}"
        ),
        None => agent_block,
    }
}

fn build_fish_agent_block(agent: &str, finalizer: Option<&str>) -> String {
    let pipefail = "set -l __rai_pipe $pipestatus; set -l __rai_agent_status 0; for s in $__rai_pipe; if test $s -ne 0; set __rai_agent_status $s; end; end";
    let agent_block = format!("begin; {agent}; end; {pipefail}");
    match finalizer {
        Some(finalizer) => format!(
            "{agent_block}; if test $__rai_agent_status -ne 0; echo \"rai: agent exited with status $__rai_agent_status; skip auto publish\" >&2; exit $__rai_agent_status; end; {finalizer}"
        ),
        None => agent_block,
    }
}

pub fn wrap_with_log(inner: &str, log_path: &Path, shell_kind: Shell) -> String {
    let log = shell::quote_path(shell_kind, log_path);
    match shell_kind {
        Shell::Posix => format!("({inner}) 2>&1 | tee -a {log}"),
        Shell::Fish => format!("begin; {inner}; end 2>&1 | tee -a {log}"),
    }
}

pub fn engine_log_path(session: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("rai-develop");
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create log dir: {}", dir.display()))?;
    Ok(dir.join(format!("{session}.log")))
}

pub fn launch(ctx: &LaunchContext, wt: &Worktree) -> Result<()> {
    let (shell_path, shell_kind) = shell::detect_user_shell();
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let rai_exe = exe.display().to_string();
    let engine_cmd = build_engine_cmd(&ctx.agent.engine_cmd, ctx.agent.permission_mode);
    let full_cmd =
        build_agent_shell_command(&engine_cmd, ctx.prompt, &rai_exe, ctx.finalizer, shell_kind);

    if ctx.agent.no_tmux {
        let status = shell::shell_command(&shell_path, &full_cmd)
            .current_dir(&wt.path)
            .status()
            .with_context(|| format!("failed to spawn `{shell_path} -c`"))?;
        if !status.success() {
            bail!("engine_cmd exited with {:?}", status.code());
        }
        return Ok(());
    }

    let ts = Local::now().format("%Y%m%d-%H%M%S");
    let session = format!("{}-{}-{}-{ts}", ctx.repo, ctx.flavor.label(), ctx.number);
    let log_path = engine_log_path(&session)?;
    let wrapped_cmd = wrap_with_log(&full_cmd, &log_path, shell_kind);

    let spawn = shell::user_shell_argv(&[
        "tmux",
        "new-session",
        "-d",
        "-s",
        &session,
        "-c",
        &wt.path.display().to_string(),
        &wrapped_cmd,
    ])
    .status();
    if let Err(e) = spawn {
        if wt.created {
            rollback_worktree(ctx.branch);
        }
        return Err(anyhow::Error::new(e).context("failed to spawn tmux"));
    }
    let spawn = spawn.unwrap();
    if !spawn.success() {
        if wt.created {
            rollback_worktree(ctx.branch);
        }
        bail!("tmux new-session exited with {:?}", spawn.code());
    }

    thread::sleep(Duration::from_millis(750));
    if !tmux_has_session(&session) {
        let tail = read_log_tail(&log_path, 40).unwrap_or_default();
        if wt.created {
            rollback_worktree(ctx.branch);
        }
        bail!(
            "tmux session `{session}` exited immediately. log: {}\n--- last lines ---\n{}",
            log_path.display(),
            if tail.trim().is_empty() {
                "(empty log)".to_string()
            } else {
                tail
            }
        );
    }

    println!("tmux session: {session}");
    println!("cwd: {}", wt.path.display());
    println!("log: {}", log_path.display());
    println!("attach: tmux attach -t {session}");
    Ok(())
}

pub fn rollback_worktree(branch: &str) {
    eprintln!("tmux start failed; rolling back worktree");
    shell::user_shell_argv(&["gwq", "remove", "--force", branch])
        .status()
        .ok();
}

fn tmux_has_session(session: &str) -> bool {
    shell::user_shell_argv(&["tmux", "has-session", "-t", session])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn read_log_tail(path: &Path, max_lines: usize) -> Option<String> {
    let body = fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = body.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    Some(lines[start..].join("\n"))
}

/// 既存 worktree がある場合は無人で `git reset --hard` + `git clean -fd` +
/// `git pull --rebase` を流して最新化し、無ければ作成する。
///
/// `gwq_add` は閉じた callable で渡すことで Issue (新規 branch) と PR (既存 ref)
/// の両方に対応できる。
pub fn ensure_worktree<F>(branch: &str, gwq_add: F) -> Result<Worktree>
where
    F: FnOnce(&str) -> Result<PathBuf>,
{
    if let Ok(path) = gwq_get(branch) {
        eprintln!("rai: worktree for `{branch}` exists; resetting + pulling before work");
        refresh_existing_worktree(&path)?;
        return Ok(Worktree {
            path,
            created: false,
        });
    }
    gwq_add(branch).map(|path| Worktree {
        path,
        created: true,
    })
}

fn refresh_existing_worktree(path: &Path) -> Result<()> {
    run_in(path, &["git", "reset", "--hard", "HEAD"])
        .context("failed to `git reset --hard HEAD` on existing worktree")?;
    run_in(path, &["git", "clean", "-fd"])
        .context("failed to `git clean -fd` on existing worktree")?;
    run_in(path, &["git", "pull", "--rebase"])
        .context("failed to `git pull --rebase` on existing worktree")?;
    Ok(())
}

fn run_in(cwd: &Path, argv: &[&str]) -> Result<()> {
    let st = shell::user_shell_argv(argv)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("failed to spawn `{}`", argv.join(" ")))?;
    if !st.success() {
        bail!("`{}` failed (status {:?})", argv.join(" "), st.code());
    }
    Ok(())
}

pub fn gwq_get(branch: &str) -> Result<PathBuf> {
    let out = shell::user_shell_argv(&["gwq", "get", branch]).output()?;
    if !out.status.success() {
        bail!("gwq get {branch} not found");
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(PathBuf::from(s))
}

/// `develop resume` 用。既存 worktree のパスを取り出すだけで、`git reset` 等の
/// 破壊的な refresh は **行わない**。`ensure_worktree` と違って、見つからない
/// 場合は新規作成も試みずにエラーで終わる。
pub fn find_existing_worktree(branch: &str) -> Result<Worktree> {
    let path =
        gwq_get(branch).with_context(|| format!("no existing worktree for branch `{branch}`"))?;
    Ok(Worktree {
        path,
        created: false,
    })
}

/// `develop/issue-<N>-*` パターンに合致するローカル branch を列挙する。
/// `develop resume` で issue 番号から worktree を探すために使う。
pub fn issue_branches_for(number: u64) -> Result<Vec<String>> {
    let pattern = format!("refs/heads/develop/issue-{number}-*");
    let raw = match git_capture(&["for-each-ref", "--format=%(refname:short)", &pattern]) {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()),
    };
    Ok(raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn gwq_add_new_branch(branch: &str) -> Result<PathBuf> {
    let st = shell::user_shell_argv(&["gwq", "add", "-b", branch])
        .status()
        .context("failed to spawn gwq add")?;
    if !st.success() {
        bail!("gwq add -b {branch} failed");
    }
    gwq_get(branch).context("failed to resolve gwq path after add")
}

pub fn gwq_add_existing_branch(branch: &str) -> Result<PathBuf> {
    let st = shell::user_shell_argv(&["gwq", "add", branch])
        .status()
        .context("failed to spawn gwq add")?;
    if !st.success() {
        bail!("gwq add {branch} failed");
    }
    gwq_get(branch).context("failed to resolve gwq path after add")
}

pub fn resolve_repo(repo_override: Option<&str>) -> Result<(String, String)> {
    if let Some(s) = repo_override {
        let (o, r) = s
            .split_once('/')
            .ok_or_else(|| anyhow!("--repo must be OWNER/REPO"))?;
        return Ok((o.to_string(), r.to_string()));
    }
    let json = gh_capture(&["repo", "view", "--json", "nameWithOwner"])?;
    #[derive(Deserialize)]
    struct V {
        #[serde(rename = "nameWithOwner")]
        name_with_owner: String,
    }
    let v: V = serde_json::from_str(&json).context("failed to parse `gh repo view` JSON")?;
    let (o, r) = v
        .name_with_owner
        .split_once('/')
        .ok_or_else(|| anyhow!("unexpected nameWithOwner: {}", v.name_with_owner))?;
    Ok((o.to_string(), r.to_string()))
}

pub fn gh_capture(args: &[&str]) -> Result<String> {
    let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 1);
    argv.push("gh");
    argv.extend_from_slice(args);
    let out = shell::user_shell_argv(&argv)
        .output()
        .context("failed to spawn `gh` via user shell")?;
    if !out.status.success() {
        bail!(
            "`gh {}` failed (status {:?}): {}",
            args.join(" "),
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn git_capture(args: &[&str]) -> Result<String> {
    let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 1);
    argv.push("git");
    argv.extend_from_slice(args);
    let out = shell::user_shell_argv(&argv)
        .output()
        .with_context(|| format!("failed to spawn `git {}`", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "`git {}` failed (status {:?}): {}",
            args.join(" "),
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn pick_with_fzf(items: impl IntoIterator<Item = (u64, String)>) -> Result<Vec<(u64, String)>> {
    let items: Vec<_> = items.into_iter().collect();
    if items.is_empty() {
        bail!("nothing to pick");
    }
    let mut fzf = shell::user_shell_argv(&["fzf", "--multi"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn `fzf`")?;
    {
        let mut stdin = fzf.stdin.take().ok_or_else(|| anyhow!("fzf stdin"))?;
        for (n, title) in &items {
            writeln!(stdin, "#{n}\t{title}").ok();
        }
    }
    let out = fzf.wait_with_output()?;
    if !out.status.success() {
        std::process::exit(130);
    }
    let s = String::from_utf8_lossy(&out.stdout);
    parse_selected(&s)
}

fn parse_selected(s: &str) -> Result<Vec<(u64, String)>> {
    let mut selected = Vec::new();
    for line in s.lines() {
        selected.push(parse_one(line)?);
    }
    if selected.is_empty() {
        std::process::exit(130);
    }
    Ok(selected)
}

fn parse_one(line: &str) -> Result<(u64, String)> {
    let (left, title) = line
        .split_once('\t')
        .ok_or_else(|| anyhow!("invalid fzf output"))?;
    let n: u64 = left
        .trim_start_matches('#')
        .parse()
        .map_err(|_| anyhow!("could not parse number from {line}"))?;
    Ok((n, title.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rai_core::shell::Shell;
    use std::path::Path;

    #[test]
    fn agent_shell_command_runs_finalizer_only_after_success() {
        let cmd = build_agent_shell_command(
            "agent --flag",
            "hello world",
            "/opt/rai/rai",
            Some("rai finalize"),
            Shell::Posix,
        );
        assert!(cmd.starts_with("set -o pipefail; (agent --flag 'hello world')"));
        assert!(cmd.contains("skip auto publish"));
        assert!(cmd.ends_with("rai finalize"));
    }

    #[test]
    fn agent_shell_command_substitutes_placeholders() {
        let cmd = build_agent_shell_command(
            "ccs c1 --print -- {PROMPT} | {RAI} claude format",
            "hello world",
            "/opt/rai/rai",
            None,
            Shell::Posix,
        );
        assert_eq!(
            cmd,
            "set -o pipefail; (ccs c1 --print -- 'hello world' | /opt/rai/rai claude format)"
        );
    }

    #[test]
    fn agent_shell_command_emits_fish_block_for_fish_shell() {
        let cmd = build_agent_shell_command(
            "ccs c1 -- {PROMPT} | {RAI} claude format",
            "hello world",
            "/opt/rai/rai",
            Some("rai finalize"),
            Shell::Fish,
        );
        assert!(cmd.starts_with(
            "begin; ccs c1 -- 'hello world' | '/opt/rai/rai' claude format; end; set -l __rai_pipe $pipestatus"
        ));
        assert!(cmd.ends_with("rai finalize"));
    }

    #[test]
    fn build_engine_cmd_substitutes_permission_mode_placeholder() {
        assert_eq!(
            build_engine_cmd(
                "ccs c1 {PERMISSION_MODE} -- {PROMPT}",
                Some(PermissionMode::DontAsk)
            ),
            "ccs c1 --permission-mode dontAsk -- {PROMPT}"
        );
    }

    #[test]
    fn build_engine_cmd_appends_when_no_placeholder() {
        assert_eq!(
            build_engine_cmd("claude", Some(PermissionMode::BypassPermissions)),
            "claude --permission-mode bypassPermissions"
        );
        assert_eq!(build_engine_cmd("claude", None), "claude");
    }

    #[test]
    fn wrap_with_log_uses_quoted_paths() {
        let p = wrap_with_log(
            "agent --x",
            Path::new("/tmp/has space/run.log"),
            Shell::Posix,
        );
        assert_eq!(p, "(agent --x) 2>&1 | tee -a '/tmp/has space/run.log'");

        let f = wrap_with_log("inner", Path::new("/tmp/x.log"), Shell::Fish);
        assert_eq!(f, "begin; inner; end 2>&1 | tee -a '/tmp/x.log'");
    }

    #[test]
    fn flavor_label_matches_session_naming() {
        assert_eq!(Flavor::Issue.label(), "issue");
        assert_eq!(Flavor::Pr.label(), "pr");
    }

    #[test]
    fn default_engine_cmd_uses_real_binaries_only() {
        assert!(DEFAULT_ENGINE_CMD.starts_with("ccs c1"));
        assert!(DEFAULT_ENGINE_CMD.contains("{PROMPT}"));
        assert!(DEFAULT_ENGINE_CMD.contains("{RAI} claude format"));
        assert!(!DEFAULT_ENGINE_CMD.contains("ccs_print"));
    }
}
