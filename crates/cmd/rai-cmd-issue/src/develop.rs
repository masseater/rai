//! `rai issue develop` — Issue から worktree + tmux + agent を起動する。

use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context};
use chrono::Local;
use clap::{Args, ValueEnum};
use rai_core::{cli::Run, Ctx, Result};
use serde::Deserialize;

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
    #[arg(long, short = 'e', value_name = "CMD", default_value = "ccs_print c1")]
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
    let finalizer = if cmd.no_auto_publish {
        None
    } else {
        Some(build_finalize_command(cmd, issue, &branch)?)
    };
    let engine_cmd = build_engine_cmd(&cmd.engine_cmd, cmd.permission_mode);
    let full_cmd = build_agent_shell_command(&engine_cmd, &prompt, finalizer.as_deref());

    if cmd.no_tmux {
        let status = Command::new("sh")
            .arg("-c")
            .arg(&full_cmd)
            .current_dir(&wt.path)
            .status()
            .context("failed to spawn engine_cmd")?;
        if !status.success() {
            bail!("engine_cmd exited with {:?}", status.code());
        }
        return Ok(());
    }

    let ts = Local::now().format("%Y%m%d-%H%M%S");
    let session = format!("gwq-run-issue-{}-{ts}", issue.number);
    let status = Command::new("tmux")
        .args([
            "new-session",
            "-d",
            "-s",
            &session,
            "-c",
            &wt.path.display().to_string(),
            &full_cmd,
        ])
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("tmux session: {session}");
            println!("cwd: {}", wt.path.display());
            println!("attach: tmux attach -t {session}");
            let _ = (&issue.owner, &issue.repo);
            Ok(())
        }
        other => {
            if wt.created {
                eprintln!("tmux start failed; rolling back worktree");
                Command::new("gwq")
                    .args(["remove", "--force", &branch])
                    .status()
                    .ok();
            }
            match other {
                Ok(s) => bail!("tmux exited with {:?}", s.code()),
                Err(e) => Err(anyhow::Error::new(e).context("failed to spawn tmux")),
            }
        }
    }
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
    let mut fzf = Command::new("fzf")
        .arg("--multi")
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
                Command::new("gwq")
                    .args(["tmux", "kill", branch])
                    .status()
                    .ok();
                let st = Command::new("gwq")
                    .args(["remove", "--force", branch])
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
    let out = Command::new("gwq").args(["get", branch]).output()?;
    if !out.status.success() {
        bail!("gwq get {branch} not found");
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(PathBuf::from(s))
}

fn gwq_add(branch: &str) -> Result<PathBuf> {
    let st = Command::new("gwq")
        .args(["add", "-b", branch])
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
            "GitHub Issue {url} (`{title}`) を一気通貫で開発してください。テスト・ビルド・clippy をローカルで通すこと。agent終了後、未コミット変更またはローカルcommitがあれば `rai issue develop` が commit / push / `gh pr create` を自動実行します。"
        ))
    } else {
        Ok(format!(
            "GitHub Issue {url} (`{title}`) を一気通貫で開発し、`gh pr create` で PR を作成するまで自走してください。テスト・ビルド・clippy をローカルで通すこと。"
        ))
    }
}

fn build_engine_cmd(engine_cmd: &str, permission_mode: Option<PermissionMode>) -> String {
    match permission_mode {
        Some(mode) => format!("{engine_cmd} --permission-mode {}", mode.as_arg()),
        None => engine_cmd.to_string(),
    }
}

fn build_agent_shell_command(engine_cmd: &str, prompt: &str, finalizer: Option<&str>) -> String {
    let agent = format!("{} {}", engine_cmd, shell_words::quote(prompt));
    match finalizer {
        Some(finalizer) => format!(
            "({agent}); __rai_agent_status=$?; if [ \"$__rai_agent_status\" -ne 0 ]; then echo \"rai: agent exited with status $__rai_agent_status; skip auto publish\" >&2; exit \"$__rai_agent_status\"; fi; {finalizer}"
        ),
        None => agent,
    }
}

fn build_finalize_command(cmd: &Cmd, issue: &Issue, branch: &str) -> Result<String> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let mut parts = vec![
        shell_quote_path(&exe),
        "issue".to_string(),
        "finalize-agent".to_string(),
        "--issue-url".to_string(),
        shell_quote(&issue.url),
        "--issue-number".to_string(),
        issue.number.to_string(),
        "--issue-title".to_string(),
        shell_quote(&issue.title),
        "--repo".to_string(),
        shell_quote(&format!("{}/{}", issue.owner, issue.repo)),
        "--branch".to_string(),
        shell_quote(branch),
    ];
    let pr_base = cmd.pr_base.clone().or_else(local_origin_head_branch);
    if let Some(base) = pr_base.as_deref() {
        parts.push("--pr-base".to_string());
        parts.push(shell_quote(base));
    }
    Ok(parts.join(" "))
}

fn shell_quote(s: &str) -> String {
    shell_words::quote(s).into_owned()
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote(&path.display().to_string())
}

#[derive(Debug)]
struct PublishContext {
    issue_url: String,
    issue_number: u64,
    issue_title: String,
    repo: String,
    branch: String,
    pr_base: Option<String>,
}

fn finalize_after_agent(ctx: &PublishContext) -> Result<()> {
    eprintln!("rai: agent completed; checking local changes");

    let had_local_changes = has_local_changes()?;
    if had_local_changes {
        git(&["add", "-A"])?;
        git(&[
            "commit",
            "-m",
            &commit_subject(ctx.issue_number, &ctx.issue_title),
        ])?;
    }

    if !had_local_changes && !has_publishable_commits(ctx.pr_base.as_deref())? {
        eprintln!("rai: no local changes or unpublished commits; cleaning up empty worktree");
        cleanup_empty_worktree(&ctx.branch);
        return Ok(());
    }

    let push_ref = format!("HEAD:{}", ctx.branch);
    git(&["push", "-u", "origin", &push_ref])?;

    if let Some(url) = existing_pr_url(&ctx.branch)? {
        println!("rai: existing PR: {url}");
        return Ok(());
    }

    let title = pr_title(ctx.issue_number, &ctx.issue_title);
    let body = format!(
        "Closes {}\n\nCreated automatically by `rai issue develop` after the agent finished successfully.",
        ctx.issue_url
    );
    let mut args: Vec<&str> = vec![
        "pr",
        "create",
        "--repo",
        ctx.repo.as_str(),
        "--head",
        ctx.branch.as_str(),
        "--title",
        title.as_str(),
        "--body",
        body.as_str(),
    ];
    if let Some(base) = &ctx.pr_base {
        args.push("--base");
        args.push(base.as_str());
    }
    let url = gh_capture(&args)?;
    print!("{url}");
    Ok(())
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

fn git(args: &[&str]) -> Result<()> {
    let st = Command::new("git")
        .args(args)
        .status()
        .with_context(|| format!("failed to spawn `git {}`", args.join(" ")))?;
    if !st.success() {
        bail!("`git {}` failed with {:?}", args.join(" "), st.code());
    }
    Ok(())
}

fn git_capture(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
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
    let kill = Command::new("gwq")
        .args(["tmux", "kill", branch])
        .current_dir(&safe_cwd)
        .status();
    if let Err(e) = kill {
        eprintln!("rai: gwq tmux kill failed to spawn: {e}");
    }
    let rm = Command::new("gwq")
        .args(["remove", "--force", branch])
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
    let out = Command::new("git")
        .args([
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

fn commit_subject(number: u64, title: &str) -> String {
    let prefix = format!("Implement issue #{number}: ");
    format!(
        "{prefix}{}",
        truncate_title(title, 72usize.saturating_sub(prefix.len()))
    )
}

fn pr_title(number: u64, title: &str) -> String {
    format!("Implement issue #{number}: {}", truncate_title(title, 80))
}

fn truncate_title(title: &str, max_chars: usize) -> String {
    let compact = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    compact.chars().take(max_chars).collect::<String>()
}

fn gh_capture(args: &[&str]) -> Result<String> {
    let out = Command::new("gh")
        .args(args)
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
    use super::{
        build_agent_shell_command, build_engine_cmd, build_prompt, commit_subject, default_branch,
        parse_selected_issues, pr_title, slugify, PermissionMode,
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
    fn default_prompt_describes_auto_publish_hook() {
        let prompt = build_prompt(
            None,
            "https://github.com/o/r/issues/13",
            "Auto publish",
            true,
        )
        .unwrap();

        assert!(prompt.contains("agent終了後"));
        assert!(prompt.contains("commit / push / `gh pr create`"));
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

        assert!(prompt.contains("`gh pr create` で PR を作成するまで"));
        assert!(!prompt.contains("agent終了後"));
    }

    #[test]
    fn agent_shell_command_runs_finalizer_only_after_success() {
        let cmd = build_agent_shell_command("agent --flag", "hello world", Some("rai finalize"));

        assert!(cmd.contains("agent --flag 'hello world'"));
        assert!(cmd.contains("skip auto publish"));
        assert!(cmd.ends_with("rai finalize"));
    }

    #[test]
    fn build_engine_cmd_appends_permission_mode_when_set() {
        assert_eq!(
            build_engine_cmd("ccs_print c1", Some(PermissionMode::BypassPermissions)),
            "ccs_print c1 --permission-mode bypassPermissions"
        );
        assert_eq!(
            build_engine_cmd("ccs_print c1", Some(PermissionMode::DontAsk)),
            "ccs_print c1 --permission-mode dontAsk"
        );
    }

    #[test]
    fn build_engine_cmd_passes_through_when_not_set() {
        assert_eq!(build_engine_cmd("ccs_print c1", None), "ccs_print c1");
    }

    #[test]
    fn publish_titles_are_compact() {
        let long = "A title with many words that should be truncated before it grows past the intended subject length";

        assert!(commit_subject(13, long).len() <= 72);
        assert!(pr_title(13, long).starts_with("Implement issue #13: "));
    }
}
