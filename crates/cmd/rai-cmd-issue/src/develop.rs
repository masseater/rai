//! `rai issue develop` — Issue から worktree + tmux + agent を起動する。

use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context};
use chrono::Local;
use clap::Args;
use rai_core::{cli::Run, Ctx, Result};
use serde::Deserialize;

#[derive(Debug, Args)]
pub struct Cmd {
    /// Issue 識別子: 番号 / URL / 省略 (省略時は fzf 選択)。
    #[arg(value_name = "ISSUE")]
    issue: Option<String>,

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
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let (owner, repo, number, title, url) = resolve_issue(&self)?;
        eprintln!("issue: {url}");

        let branch = match &self.branch {
            Some(b) => b.clone(),
            None => default_branch(&title, number),
        };
        eprintln!("branch: {branch}");

        let wt_path = ensure_worktree(&branch)?;
        eprintln!("worktree: {}", wt_path.display());

        let prompt = build_prompt(self.prompt_template.as_deref(), &url, &title)?;
        let full_cmd = format!("{} {}", self.engine_cmd, shell_words::quote(&prompt),);

        if self.no_tmux {
            let status = Command::new("sh")
                .arg("-c")
                .arg(&full_cmd)
                .current_dir(&wt_path)
                .status()
                .context("failed to spawn engine_cmd")?;
            if !status.success() {
                bail!("engine_cmd exited with {:?}", status.code());
            }
            return Ok(());
        }

        let ts = Local::now().format("%Y%m%d-%H%M%S");
        let session = format!("gwq-run-issue-{number}-{ts}");
        let status = Command::new("tmux")
            .args([
                "new-session",
                "-d",
                "-s",
                &session,
                "-c",
                &wt_path.display().to_string(),
                &full_cmd,
            ])
            .status();
        match status {
            Ok(s) if s.success() => {
                println!("tmux session: {session}");
                println!("cwd: {}", wt_path.display());
                println!("attach: tmux attach -t {session}");
                let _ = (&owner, &repo);
                Ok(())
            }
            other => {
                eprintln!("tmux start failed; rolling back worktree");
                Command::new("gwq")
                    .args(["remove", "--force", &branch])
                    .status()
                    .ok();
                match other {
                    Ok(s) => bail!("tmux exited with {:?}", s.code()),
                    Err(e) => Err(anyhow::Error::new(e).context("failed to spawn tmux")),
                }
            }
        }
    }
}

fn resolve_issue(cmd: &Cmd) -> Result<(String, String, u64, String, String)> {
    if let Some(arg) = &cmd.issue {
        if let Some((o, r, n)) = parse_issue_url(arg) {
            let title = fetch_title(&o, &r, n)?;
            let url = format!("https://github.com/{o}/{r}/issues/{n}");
            return Ok((o, r, n, title, url));
        }
        if let Ok(n) = arg.parse::<u64>() {
            let (o, r) = resolve_repo(cmd.repo.as_deref())?;
            let title = fetch_title(&o, &r, n)?;
            let url = format!("https://github.com/{o}/{r}/issues/{n}");
            return Ok((o, r, n, title, url));
        }
        bail!("invalid issue identifier: {arg}");
    }
    let (o, r) = resolve_repo(cmd.repo.as_deref())?;
    let (n, title) = pick_issue_with_fzf(&o, &r)?;
    let url = format!("https://github.com/{o}/{r}/issues/{n}");
    Ok((o, r, n, title, url))
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

fn pick_issue_with_fzf(owner: &str, repo: &str) -> Result<(u64, String)> {
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
    let line = s.lines().next().unwrap_or("");
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

fn ensure_worktree(branch: &str) -> Result<PathBuf> {
    if let Ok(path) = gwq_get(branch) {
        let action = prompt_existing(branch)?;
        match action {
            ExistingAction::Attach => Ok(path),
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
                gwq_add(branch)
            }
            ExistingAction::Abort => std::process::exit(130),
        }
    } else {
        gwq_add(branch)
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

fn build_prompt(template: Option<&std::path::Path>, url: &str, title: &str) -> Result<String> {
    if let Some(p) = template {
        let body = fs::read_to_string(p)
            .with_context(|| format!("failed to read prompt template: {}", p.display()))?;
        return Ok(body
            .replace("{ISSUE_URL}", url)
            .replace("{ISSUE_TITLE}", title));
    }
    Ok(format!(
        "GitHub Issue {url} (`{title}`) を一気通貫で開発し、`gh pr create` で PR を作成するまで自走してください。テスト・ビルド・clippy をローカルで通すこと。"
    ))
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
    use super::{default_branch, slugify};

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
}
