//! `rai issue develop` — Issue から worktree + tmux + agent を起動する。

use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use chrono::Local;
use clap::{Args, ValueEnum};
use rai_core::{
    cli::Run,
    shell::{self, Shell},
    Ctx, Result,
};
use serde::Deserialize;

const DEFAULT_ENGINE_CMD: &str = "ccs c1 --print --output-format stream-json --verbose {PERMISSION_MODE} -- {PROMPT} | {RAI} claude format";

#[derive(Debug, Args)]
pub struct Cmd {
    /// Issue 識別子: 番号 / URL / 省略 (省略時は fzf 複数選択)。
    #[arg(value_name = "ISSUE")]
    issue: Vec<String>,

    /// `OWNER/REPO` を上書き。
    #[arg(long, value_name = "OWNER/REPO")]
    repo: Option<String>,

    /// ブランチ名を直接指定。未指定なら slug から自動生成。
    #[arg(long, short = 'b')]
    branch: Option<String>,

    /// agent CLI の起動コマンド (shell 文字列)。
    ///
    /// プレースホルダ:
    /// - `{PROMPT}`        : 現 issue のプロンプト (shell-quoted)
    /// - `{PERMISSION_MODE}`: `--permission-mode <MODE>` 一式 (`--permission-mode` 未指定なら空)
    /// - `{RAI}`           : 実行中の `rai` バイナリ絶対パス (shell-quoted)
    ///
    /// プレースホルダを 1 つも含まない文字列を渡した場合は legacy 互換動作で末尾に
    /// `{PERMISSION_MODE}` と `{PROMPT}` を append する。
    #[arg(
        long,
        short = 'e',
        value_name = "CMD",
        default_value = DEFAULT_ENGINE_CMD
    )]
    engine_cmd: String,

    /// prompt をファイルから読み込む。
    #[arg(long, value_name = "FILE")]
    prompt_template: Option<PathBuf>,

    /// tmux を介さず前面で実行 (デバッグ用)。
    #[arg(long)]
    no_tmux: bool,

    /// agent 終了後の自動 commit / push / PR 作成を無効化する。
    #[arg(long)]
    no_auto_publish: bool,

    /// 自動作成する PR の base branch。
    #[arg(long, value_name = "BRANCH")]
    pr_base: Option<String>,

    /// agent (`claude`) に渡す `--permission-mode` を明示する。
    #[arg(long, value_name = "MODE", value_enum)]
    permission_mode: Option<PermissionMode>,
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
    fn as_arg(self) -> &'static str {
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

#[derive(Debug, Args)]
pub struct FinalizeCmd {
    /// Issue URL.
    #[arg(long)]
    issue_url: String,

    /// Issue number.
    #[arg(long)]
    issue_number: u64,

    /// Issue title.
    #[arg(long)]
    issue_title: String,

    /// Repository in OWNER/REPO form.
    #[arg(long)]
    repo: String,

    /// Branch being developed.
    #[arg(long)]
    branch: String,

    /// PR base branch, when known.
    #[arg(long)]
    pr_base: Option<String>,

    /// engine_cmd template forwarded from `rai issue develop` so the finalize
    /// agent can be spawned with the same engine as the implementation agent.
    #[arg(long, value_name = "CMD", default_value = DEFAULT_ENGINE_CMD)]
    engine_cmd: String,

    /// `--permission-mode` forwarded from `rai issue develop`.
    #[arg(long, value_name = "MODE", value_enum)]
    permission_mode: Option<PermissionMode>,
}

impl Run for FinalizeCmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        finalize_after_agent(&PublishContext {
            issue_url: self.issue_url,
            issue_number: self.issue_number,
            issue_title: self.issue_title,
            repo: self.repo,
            branch: self.branch,
            pr_base: self.pr_base,
            engine_cmd: self.engine_cmd,
            permission_mode: self.permission_mode,
        })
    }
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        if self.issue.len() > 1 && self.branch.is_some() {
            bail!("--branch can only be used with a single issue");
        }

        let issues = resolve_issues(&self)?;
        if issues.len() > 1 && self.branch.is_some() {
            bail!("--branch can only be used with a single issue");
        }

        for issue in issues {
            run_one(&self, &issue)?;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct Issue {
    owner: String,
    repo: String,
    number: u64,
    title: String,
    url: String,
}

#[derive(Debug)]
struct Worktree {
    path: PathBuf,
    created: bool,
}

fn run_one(cmd: &Cmd, issue: &Issue) -> Result<()> {
    eprintln!("issue: {}", issue.url);

    let branch = match &cmd.branch {
        Some(b) => b.clone(),
        None => default_branch(&issue.title, issue.number),
    };
    eprintln!("branch: {branch}");

    let wt = ensure_worktree(&branch)?;
    eprintln!("worktree: {}", wt.path.display());

    let prompt = build_prompt(
        cmd.prompt_template.as_deref(),
        &issue.url,
        &issue.title,
        !cmd.no_auto_publish,
    )?;
    let (shell_path, shell_kind) = shell::detect_user_shell();
    let finalizer = if cmd.no_auto_publish {
        None
    } else {
        Some(build_finalize_command(cmd, issue, &branch, shell_kind)?)
    };
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let rai_exe = exe.display().to_string();
    let engine_cmd = build_engine_cmd(&cmd.engine_cmd, cmd.permission_mode);
    let full_cmd = build_agent_shell_command(
        &engine_cmd,
        &prompt,
        &rai_exe,
        finalizer.as_deref(),
        shell_kind,
    );

    if cmd.no_tmux {
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
    let session = format!("{}-issue-{}-{ts}", issue.repo, issue.number);
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
            rollback_worktree(&branch);
        }
        return Err(anyhow::Error::new(e).context("failed to spawn tmux"));
    }
    let spawn = spawn.unwrap();
    if !spawn.success() {
        if wt.created {
            rollback_worktree(&branch);
        }
        bail!("tmux new-session exited with {:?}", spawn.code());
    }

    // tmux new-session -d returns success even when the inner command fails
    // (e.g. `ccs` not on PATH for tmux's default-shell). Verify the session
    // is still alive shortly after launch and surface the captured log on
    // immediate failure (fail-fast).
    thread::sleep(Duration::from_millis(750));
    if !tmux_has_session(&session) {
        let tail = read_log_tail(&log_path, 40).unwrap_or_default();
        if wt.created {
            rollback_worktree(&branch);
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
    let _ = (&issue.owner, &issue.repo);
    Ok(())
}

fn rollback_worktree(branch: &str) {
    eprintln!("tmux start failed; rolling back worktree");
    shell::user_shell_argv(&["gwq", "remove", "--force", branch])
        .status()
        .ok();
}

fn engine_log_path(session: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("rai-issue-develop");
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create log dir: {}", dir.display()))?;
    Ok(dir.join(format!("{session}.log")))
}

fn wrap_with_log(inner: &str, log_path: &Path, shell_kind: Shell) -> String {
    let log = shell::quote_path(shell_kind, log_path);
    match shell_kind {
        Shell::Posix => format!("({inner}) 2>&1 | tee -a {log}"),
        Shell::Fish => format!("begin; {inner}; end 2>&1 | tee -a {log}"),
    }
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

fn resolve_issues(cmd: &Cmd) -> Result<Vec<Issue>> {
    if !cmd.issue.is_empty() {
        let mut issues = Vec::with_capacity(cmd.issue.len());
        for arg in &cmd.issue {
            issues.push(resolve_issue_arg(cmd, arg)?);
        }
        return Ok(issues);
    }

    let (o, r) = resolve_repo(cmd.repo.as_deref())?;
    let selected = pick_issues_with_fzf(&o, &r)?;
    Ok(selected
        .into_iter()
        .map(|(n, title)| {
            let url = format!("https://github.com/{o}/{r}/issues/{n}");
            Issue {
                owner: o.clone(),
                repo: r.clone(),
                number: n,
                title,
                url,
            }
        })
        .collect())
}

fn resolve_issue_arg(cmd: &Cmd, arg: &str) -> Result<Issue> {
    if let Some((o, r, n)) = parse_issue_url(arg) {
        let title = fetch_title(&o, &r, n)?;
        let url = format!("https://github.com/{o}/{r}/issues/{n}");
        return Ok(Issue {
            owner: o,
            repo: r,
            number: n,
            title,
            url,
        });
    }
    if let Ok(n) = arg.parse::<u64>() {
        let (o, r) = resolve_repo(cmd.repo.as_deref())?;
        let title = fetch_title(&o, &r, n)?;
        let url = format!("https://github.com/{o}/{r}/issues/{n}");
        return Ok(Issue {
            owner: o,
            repo: r,
            number: n,
            title,
            url,
        });
    }
    bail!("invalid issue identifier: {arg}");
}

fn parse_issue_url(s: &str) -> Option<(String, String, u64)> {
    let stripped = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))?;
    let mut it = stripped.split('/');
    let owner = it.next()?.to_string();
    let repo = it.next()?.to_string();
    if it.next()? != "issues" {
        return None;
    }
    let n: u64 = it.next()?.parse().ok()?;
    Some((owner, repo, n))
}

fn resolve_repo(repo_override: Option<&str>) -> Result<(String, String)> {
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

fn fetch_title(owner: &str, repo: &str, number: u64) -> Result<String> {
    let json = gh_capture(&[
        "issue",
        "view",
        &number.to_string(),
        "--repo",
        &format!("{owner}/{repo}"),
        "--json",
        "title",
    ])?;
    #[derive(Deserialize)]
    struct V {
        title: String,
    }
    let v: V = serde_json::from_str(&json).context("failed to parse `gh issue view` JSON")?;
    Ok(v.title)
}

fn pick_issues_with_fzf(owner: &str, repo: &str) -> Result<Vec<(u64, String)>> {
    let json = gh_capture(&[
        "issue",
        "list",
        "--repo",
        &format!("{owner}/{repo}"),
        "--state",
        "open",
        "--limit",
        "50",
        "--json",
        "number,title",
    ])?;
    #[derive(Deserialize)]
    struct Item {
        number: u64,
        title: String,
    }
    let items: Vec<Item> =
        serde_json::from_str(&json).context("failed to parse `gh issue list` JSON")?;
    if items.is_empty() {
        bail!("no open issues found");
    }
    let mut fzf = shell::user_shell_argv(&["fzf", "--multi"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn `fzf`")?;
    {
        let mut stdin = fzf.stdin.take().ok_or_else(|| anyhow!("fzf stdin"))?;
        for it in &items {
            writeln!(stdin, "#{}\t{}", it.number, it.title).ok();
        }
    }
    let out = fzf.wait_with_output()?;
    if !out.status.success() {
        std::process::exit(130);
    }
    let s = String::from_utf8_lossy(&out.stdout);
    parse_selected_issues(&s)
}

fn parse_selected_issues(s: &str) -> Result<Vec<(u64, String)>> {
    let mut selected = Vec::new();
    for line in s.lines() {
        selected.push(parse_selected_issue(line)?);
    }
    if selected.is_empty() {
        std::process::exit(130);
    }
    Ok(selected)
}

fn parse_selected_issue(line: &str) -> Result<(u64, String)> {
    let (left, title) = line
        .split_once('\t')
        .ok_or_else(|| anyhow!("invalid fzf output"))?;
    let n: u64 = left
        .trim_start_matches('#')
        .parse()
        .map_err(|_| anyhow!("could not parse issue number from {line}"))?;
    Ok((n, title.to_string()))
}

fn default_branch(title: &str, number: u64) -> String {
    let slug = slugify(title);
    let ts = Local::now().format("%Y%m%d-%H%M%S");
    if slug.is_empty() {
        format!("develop/issue-{number}-{ts}")
    } else {
        format!("develop/issue-{number}-{slug}-{ts}")
    }
}

fn slugify(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut out = String::new();
    let mut prev_dash = true;
    for c in lower.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    trimmed
        .chars()
        .take(40)
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn ensure_worktree(branch: &str) -> Result<Worktree> {
    if let Ok(path) = gwq_get(branch) {
        let action = prompt_existing(branch)?;
        match action {
            ExistingAction::Attach => Ok(Worktree {
                path,
                created: false,
            }),
            ExistingAction::ForceRecreate => {
                shell::user_shell_argv(&["gwq", "tmux", "kill", branch])
                    .status()
                    .ok();
                let st = shell::user_shell_argv(&["gwq", "remove", "--force", branch])
                    .status()
                    .context("failed to spawn gwq remove")?;
                if !st.success() {
                    bail!("gwq remove failed");
                }
                gwq_add(branch).map(|path| Worktree {
                    path,
                    created: true,
                })
            }
            ExistingAction::Abort => std::process::exit(130),
        }
    } else {
        gwq_add(branch).map(|path| Worktree {
            path,
            created: true,
        })
    }
}

fn gwq_get(branch: &str) -> Result<PathBuf> {
    let out = shell::user_shell_argv(&["gwq", "get", branch]).output()?;
    if !out.status.success() {
        bail!("gwq get {branch} not found");
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(PathBuf::from(s))
}

fn gwq_add(branch: &str) -> Result<PathBuf> {
    let st = shell::user_shell_argv(&["gwq", "add", "-b", branch])
        .status()
        .context("failed to spawn gwq add")?;
    if !st.success() {
        bail!("gwq add -b {branch} failed");
    }
    gwq_get(branch).context("failed to resolve gwq path after add")
}

enum ExistingAction {
    Attach,
    ForceRecreate,
    Abort,
}

fn prompt_existing(branch: &str) -> Result<ExistingAction> {
    if !io::stdin().is_terminal() {
        bail!(
            "worktree for `{branch}` exists; pass --branch with a fresh name or run in tty for prompt"
        );
    }
    eprint!("worktree for `{branch}` exists. [a]ttach / [f]orce-recreate / [q]abort: ");
    io::stderr().flush().ok();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    match line.trim().chars().next().unwrap_or('q') {
        'a' | 'A' => Ok(ExistingAction::Attach),
        'f' | 'F' => Ok(ExistingAction::ForceRecreate),
        _ => Ok(ExistingAction::Abort),
    }
}

fn build_prompt(
    template: Option<&std::path::Path>,
    url: &str,
    title: &str,
    auto_publish: bool,
) -> Result<String> {
    if let Some(p) = template {
        let body = fs::read_to_string(p)
            .with_context(|| format!("failed to read prompt template: {}", p.display()))?;
        return Ok(body
            .replace("{ISSUE_URL}", url)
            .replace("{ISSUE_TITLE}", title));
    }
    if auto_publish {
        Ok(format!(
            "GitHub Issue {url} (`{title}`) を一気通貫で開発し、PR を出すところまで自走してください。実装したらテスト・ビルド・lint をローカルで通し、commit して push し、`gh pr create` で PR を作成します。PR 本文には `Closes {url}` を含めてください。commit-msg hook がメッセージを弾いた場合はメッセージを直して commit し直してください。`--no-verify` などで hook を回避するのは禁止です。万一あなたが PR まで辿り着かずに終了した場合の保険として、`rai issue develop` 側が finalize agent を起動して残りを引き取りますが、これはあくまで fallback なので、原則あなた自身で PR まで完了させてください。"
        ))
    } else {
        Ok(format!(
            "GitHub Issue {url} (`{title}`) を一気通貫で開発し、commit、push、`gh pr create` で PR を作成するところまで自走してください。テスト・ビルド・lint をローカルで通すこと。commit-msg hook がメッセージを弾いたらメッセージを直して commit し直してください。`--no-verify` などで hook を回避するのは禁止です。"
        ))
    }
}

fn build_engine_cmd(engine_cmd: &str, permission_mode: Option<PermissionMode>) -> String {
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

fn build_agent_shell_command(
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
    // Capture worst pipe status to emulate POSIX `set -o pipefail`.
    let pipefail = "set -l __rai_pipe $pipestatus; set -l __rai_agent_status 0; for s in $__rai_pipe; if test $s -ne 0; set __rai_agent_status $s; end; end";
    let agent_block = format!("begin; {agent}; end; {pipefail}");
    match finalizer {
        Some(finalizer) => format!(
            "{agent_block}; if test $__rai_agent_status -ne 0; echo \"rai: agent exited with status $__rai_agent_status; skip auto publish\" >&2; exit $__rai_agent_status; end; {finalizer}"
        ),
        None => agent_block,
    }
}

fn build_finalize_command(
    cmd: &Cmd,
    issue: &Issue,
    branch: &str,
    shell_kind: Shell,
) -> Result<String> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let q = shell::quote_for(shell_kind);
    let mut parts = vec![
        shell::quote_path(shell_kind, &exe),
        "issue".to_string(),
        "finalize-agent".to_string(),
        "--issue-url".to_string(),
        q(&issue.url),
        "--issue-number".to_string(),
        issue.number.to_string(),
        "--issue-title".to_string(),
        q(&issue.title),
        "--repo".to_string(),
        q(&format!("{}/{}", issue.owner, issue.repo)),
        "--branch".to_string(),
        q(branch),
        "--engine-cmd".to_string(),
        q(&cmd.engine_cmd),
    ];
    if let Some(mode) = cmd.permission_mode {
        parts.push("--permission-mode".to_string());
        parts.push(mode.as_arg().to_string());
    }
    let pr_base = cmd.pr_base.clone().or_else(local_origin_head_branch);
    if let Some(base) = pr_base.as_deref() {
        parts.push("--pr-base".to_string());
        parts.push(q(base));
    }
    Ok(parts.join(" "))
}

#[derive(Debug)]
struct PublishContext {
    issue_url: String,
    #[allow(dead_code)]
    issue_number: u64,
    issue_title: String,
    repo: String,
    branch: String,
    pr_base: Option<String>,
    engine_cmd: String,
    permission_mode: Option<PermissionMode>,
}

fn finalize_after_agent(ctx: &PublishContext) -> Result<()> {
    eprintln!("rai: agent completed; checking local state");

    let has_local = has_local_changes()?;
    let has_commits = has_publishable_commits(ctx.pr_base.as_deref())?;

    if !has_local && !has_commits {
        eprintln!("rai: no local changes or unpublished commits; cleaning up empty worktree");
        cleanup_empty_worktree(&ctx.branch);
        return Ok(());
    }

    eprintln!(
        "rai: delegating commit / push / PR to finalize agent (has_local={has_local}, has_commits={has_commits})"
    );

    let prompt = build_finalize_prompt(ctx, has_local, has_commits);
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let rai_exe = exe.display().to_string();
    let (shell_path, shell_kind) = shell::detect_user_shell();
    let engine_cmd = build_engine_cmd(&ctx.engine_cmd, ctx.permission_mode);
    let full_cmd = build_agent_shell_command(&engine_cmd, &prompt, &rai_exe, None, shell_kind);

    let status = shell::shell_command(&shell_path, &full_cmd)
        .status()
        .with_context(|| format!("failed to spawn finalize agent via `{shell_path} -c`"))?;
    if !status.success() {
        bail!("finalize agent exited with {:?}", status.code());
    }

    match existing_pr_url(&ctx.branch)? {
        Some(url) => {
            println!("rai: PR: {url}");
        }
        None => {
            eprintln!(
                "rai: warning — finalize agent finished but no PR detected for branch `{}`. Inspect the worktree and finish manually.",
                ctx.branch
            );
        }
    }

    Ok(())
}

fn build_finalize_prompt(ctx: &PublishContext, has_local: bool, has_commits: bool) -> String {
    let state = match (has_local, has_commits) {
        (true, true) => "未コミットの変更と未 push の commit が両方残っています",
        (true, false) => "未コミットの変更が残っています",
        (false, true) => "未 push の commit が残っています",
        (false, false) => unreachable!("finalize agent invoked with nothing to publish"),
    };
    let base_sentence = match ctx.pr_base.as_deref() {
        Some(base) => format!(" PR を作成する際は base を `{base}` にしてください。"),
        None => String::new(),
    };
    format!(
        "GitHub Issue {url} (`{title}`) の作業を引き取って commit、push、PR の作成まで仕上げてください。\
worktree のブランチは `{branch}` で、現在 {state}。\
未コミット変更があれば論理的な単位で commit し、`git push -u origin HEAD:{branch}` で push したあと、\
リポジトリ `{repo}` に対して `gh pr create` で PR を作成してください。本文には `Closes {url}` を含めること。\
既に同じブランチに PR がある場合は新規作成せず、その URL を表示するだけで終わってください。{base_sentence} \
commit-msg hook がメッセージを弾いた場合はメッセージを直して commit し直してください。\
`--no-verify` などで hook を回避するのは禁止です。",
        url = ctx.issue_url,
        title = ctx.issue_title,
        branch = ctx.branch,
        state = state,
        base_sentence = base_sentence,
        repo = ctx.repo,
    )
}

fn has_local_changes() -> Result<bool> {
    Ok(!git_capture(&["status", "--porcelain"])?.trim().is_empty())
}

fn has_publishable_commits(pr_base: Option<&str>) -> Result<bool> {
    if let Some(base) = pr_base {
        if has_commits_since(&format!("origin/{base}"))? {
            return Ok(true);
        }
        if has_commits_since(base)? {
            return Ok(true);
        }
    }

    let Ok(upstream) = git_capture(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
    else {
        return Ok(false);
    };
    has_commits_since(upstream.trim())
}

fn has_commits_since(base_ref: &str) -> Result<bool> {
    let Ok(base) = git_capture(&["merge-base", "HEAD", base_ref]) else {
        return Ok(false);
    };
    let range = format!("{}..HEAD", base.trim());
    let count = git_capture(&["rev-list", "--count", &range])?;
    Ok(count.trim().parse::<u64>().unwrap_or(0) > 0)
}

fn existing_pr_url(branch: &str) -> Result<Option<String>> {
    let out = gh_capture(&[
        "pr",
        "list",
        "--head",
        branch,
        "--json",
        "url",
        "--limit",
        "1",
        "--jq",
        ".[0].url // \"\"",
    ])?;
    let url = out.trim();
    if url.is_empty() {
        Ok(None)
    } else {
        Ok(Some(url.to_string()))
    }
}

fn git_capture(args: &[&str]) -> Result<String> {
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

fn cleanup_empty_worktree(branch: &str) {
    let safe_cwd = std::env::temp_dir();
    let kill = shell::user_shell_argv(&["gwq", "tmux", "kill", branch])
        .current_dir(&safe_cwd)
        .status();
    if let Err(e) = kill {
        eprintln!("rai: gwq tmux kill failed to spawn: {e}");
    }
    let rm = shell::user_shell_argv(&["gwq", "remove", "--force", branch])
        .current_dir(&safe_cwd)
        .status();
    match rm {
        Ok(s) if s.success() => eprintln!("rai: removed empty worktree for {branch}"),
        Ok(s) => eprintln!(
            "rai: gwq remove exited with {:?}; leaving worktree",
            s.code()
        ),
        Err(e) => eprintln!("rai: gwq remove failed to spawn: {e}; leaving worktree"),
    }
}

fn local_origin_head_branch() -> Option<String> {
    let out = shell::user_shell_argv(&[
        "git",
        "symbolic-ref",
        "--quiet",
        "--short",
        "refs/remotes/origin/HEAD",
    ])
    .output()
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&out.stdout);
    let branch = branch.trim().strip_prefix("origin/")?;
    Some(branch.to_string())
}

fn gh_capture(args: &[&str]) -> Result<String> {
    let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 1);
    argv.push("gh");
    argv.extend_from_slice(args);
    let out = shell::user_shell_argv(&argv)
        .output()
        .context("failed to spawn `gh`")?;
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rai_core::shell::{detect_shell_kind, quote_fish, Shell};

    use super::{
        build_agent_shell_command, build_engine_cmd, build_finalize_prompt, build_prompt,
        default_branch, parse_selected_issues, read_log_tail, slugify, wrap_with_log,
        PermissionMode, PublishContext,
    };

    #[test]
    fn default_branch_uses_develop_issue_prefix() {
        let branch = default_branch("Add issue workflow", 9);

        assert!(branch.starts_with("develop/issue-9-add-issue-workflow-"));
        assert!(!branch.starts_with("fix/"));
    }

    #[test]
    fn default_branch_without_slug_uses_develop_issue_prefix() {
        let branch = default_branch("!!!", 9);

        assert!(branch.starts_with("develop/issue-9-"));
        assert!(!branch.starts_with("fix/"));
    }

    #[test]
    fn slugify_keeps_existing_rules() {
        assert_eq!(
            slugify("Hello, RAI issue develop!"),
            "hello-rai-issue-develop"
        );
        assert_eq!(
            slugify("abcdefghijklmnopqrstuvwxyz0123456789-extra"),
            "abcdefghijklmnopqrstuvwxyz0123456789-ext"
        );
    }

    #[test]
    fn parse_selected_issues_accepts_multiple_fzf_lines() {
        let issues = parse_selected_issues("#12\tFirst issue\n#34\tSecond issue\n").unwrap();

        assert_eq!(
            issues,
            vec![(12, "First issue".into()), (34, "Second issue".into())]
        );
    }

    #[test]
    fn default_prompt_directs_agent_to_publish_with_finalize_as_fallback() {
        let prompt = build_prompt(
            None,
            "https://github.com/o/r/issues/13",
            "Auto publish",
            true,
        )
        .unwrap();

        // The implementation agent itself is told to commit/push/PR.
        assert!(prompt.contains("PR を出すところまで自走"));
        assert!(prompt.contains("`gh pr create`"));
        assert!(prompt.contains("Closes https://github.com/o/r/issues/13"));
        assert!(prompt.contains("commit-msg hook"));
        assert!(prompt.contains("--no-verify"));
        // finalize agent is only mentioned as a fallback safety net.
        assert!(prompt.contains("finalize agent"));
        assert!(prompt.contains("fallback"));
        // No hardcoded enumeration of commit-rule sources — the hook fires on its own.
        assert!(!prompt.contains(".commitlintrc"));
        assert!(!prompt.contains(".husky"));
        assert!(!prompt.contains("CONTRIBUTING.md"));
    }

    #[test]
    fn default_prompt_asks_agent_to_publish_when_auto_publish_is_disabled() {
        let prompt = build_prompt(
            None,
            "https://github.com/o/r/issues/13",
            "Manual publish",
            false,
        )
        .unwrap();

        assert!(prompt.contains("`gh pr create`"));
        assert!(prompt.contains("commit-msg hook"));
        assert!(prompt.contains("--no-verify"));
        assert!(!prompt.contains("finalize agent"));
        assert!(!prompt.contains(".commitlintrc"));
    }

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
    fn agent_shell_command_quotes_rai_path_with_spaces() {
        let cmd = build_agent_shell_command(
            "{RAI} claude format <<<{PROMPT}",
            "p",
            "/path with spaces/rai",
            None,
            Shell::Posix,
        );

        assert!(cmd.contains("'/path with spaces/rai'"));
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
        assert!(cmd.contains("for s in $__rai_pipe"));
        assert!(cmd.contains("if test $__rai_agent_status -ne 0"));
        assert!(cmd.ends_with("rai finalize"));
        // POSIX-only constructs must be absent.
        assert!(!cmd.contains("set -o pipefail"));
        assert!(!cmd.contains("$?"));
    }

    #[test]
    fn agent_shell_command_fish_without_finalizer_omits_check() {
        let cmd = build_agent_shell_command("agent --x", "p", "/opt/rai/rai", None, Shell::Fish);

        assert!(cmd.starts_with("begin; agent --x 'p'; end;"));
        assert!(!cmd.contains("skip auto publish"));
    }

    #[test]
    fn detect_shell_kind_recognises_fish_and_falls_back_to_posix() {
        assert_eq!(detect_shell_kind("/opt/homebrew/bin/fish"), Shell::Fish);
        assert_eq!(detect_shell_kind("/usr/local/bin/fish"), Shell::Fish);
        assert_eq!(detect_shell_kind("/bin/zsh"), Shell::Posix);
        assert_eq!(detect_shell_kind("/bin/bash"), Shell::Posix);
        assert_eq!(detect_shell_kind("/bin/sh"), Shell::Posix);
        assert_eq!(detect_shell_kind(""), Shell::Posix);
    }

    #[test]
    fn shell_quote_fish_escapes_quotes_and_backslashes() {
        assert_eq!(quote_fish("plain"), "'plain'");
        assert_eq!(quote_fish("a'b"), "'a\\'b'");
        assert_eq!(quote_fish("a\\b"), "'a\\\\b'");
        assert_eq!(quote_fish("space here"), "'space here'");
    }

    #[test]
    fn wrap_with_log_uses_begin_end_for_fish() {
        let wrapped = wrap_with_log("inner", Path::new("/tmp/run with space.log"), Shell::Fish);
        assert_eq!(
            wrapped,
            "begin; inner; end 2>&1 | tee -a '/tmp/run with space.log'"
        );
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
        assert_eq!(
            build_engine_cmd("ccs c1 {PERMISSION_MODE} -- {PROMPT}", None),
            "ccs c1  -- {PROMPT}"
        );
    }

    #[test]
    fn build_engine_cmd_appends_permission_mode_when_set_legacy() {
        assert_eq!(
            build_engine_cmd("claude", Some(PermissionMode::BypassPermissions)),
            "claude --permission-mode bypassPermissions"
        );
    }

    #[test]
    fn build_engine_cmd_passes_through_when_not_set_legacy() {
        assert_eq!(build_engine_cmd("claude", None), "claude");
    }

    #[test]
    fn default_engine_cmd_uses_real_binaries_only() {
        assert!(crate::develop::DEFAULT_ENGINE_CMD.starts_with("ccs c1"));
        assert!(crate::develop::DEFAULT_ENGINE_CMD.contains("{PROMPT}"));
        assert!(crate::develop::DEFAULT_ENGINE_CMD.contains("{RAI} claude format"));
        assert!(!crate::develop::DEFAULT_ENGINE_CMD.contains("ccs_print"));
    }

    #[test]
    fn wrap_with_log_tees_to_quoted_path() {
        let wrapped = wrap_with_log(
            "agent --x",
            Path::new("/tmp/has space/run.log"),
            Shell::Posix,
        );

        assert_eq!(
            wrapped,
            "(agent --x) 2>&1 | tee -a '/tmp/has space/run.log'"
        );
    }

    #[test]
    fn read_log_tail_returns_last_lines() {
        let dir = std::env::temp_dir().join("rai-issue-develop-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("read_log_tail.log");
        std::fs::write(&path, "a\nb\nc\nd\ne\n").unwrap();

        let tail = read_log_tail(&path, 3).unwrap();
        assert_eq!(tail, "c\nd\ne");

        let tail_all = read_log_tail(&path, 100).unwrap();
        assert_eq!(tail_all, "a\nb\nc\nd\ne");
    }

    #[test]
    fn finalize_prompt_delegates_repo_conventions_to_agent() {
        let ctx = PublishContext {
            issue_url: "https://github.com/o/r/issues/13".to_string(),
            issue_number: 13,
            issue_title: "Some work".to_string(),
            repo: "o/r".to_string(),
            branch: "develop/issue-13-some-work-20260430".to_string(),
            pr_base: Some("main".to_string()),
            engine_cmd: super::DEFAULT_ENGINE_CMD.to_string(),
            permission_mode: None,
        };

        let prompt = build_finalize_prompt(&ctx, true, false);

        assert!(prompt.contains("https://github.com/o/r/issues/13"));
        assert!(prompt.contains("develop/issue-13-some-work-20260430"));
        assert!(prompt.contains("`Closes https://github.com/o/r/issues/13`"));
        assert!(prompt.contains("gh pr create"));
        assert!(prompt.contains("o/r"));
        assert!(prompt.contains("--no-verify"));
        assert!(prompt.contains("base を `main`"));
        assert!(prompt.contains("commit-msg hook"));
        // No hardcoded enumeration of commit-rule sources.
        assert!(!prompt.contains(".commitlintrc"));
        assert!(!prompt.contains(".husky"));
        assert!(!prompt.contains("CONTRIBUTING.md"));
    }

    #[test]
    fn finalize_prompt_omits_base_sentence_when_unspecified() {
        let ctx = PublishContext {
            issue_url: "https://github.com/o/r/issues/9".to_string(),
            issue_number: 9,
            issue_title: "T".to_string(),
            repo: "o/r".to_string(),
            branch: "b".to_string(),
            pr_base: None,
            engine_cmd: super::DEFAULT_ENGINE_CMD.to_string(),
            permission_mode: None,
        };

        let prompt = build_finalize_prompt(&ctx, false, true);

        assert!(!prompt.contains("base を"));
        assert!(prompt.contains("未 push の commit"));
    }
}
