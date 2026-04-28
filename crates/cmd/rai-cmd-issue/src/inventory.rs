//! `rai issue inventory` — Issue 一覧を取得し、棚卸し prompt を agent に渡す。

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{bail, Context};
use clap::{Args, ValueEnum};
use rai_core::{cli::Run, Ctx, Result};
use serde::Deserialize;
use serde_json::{json, Value};

const ISSUE_FIELDS: &str =
    "number,title,url,state,author,assignees,labels,milestone,comments,createdAt,updatedAt,body";

#[derive(Debug, Args)]
pub struct Cmd {
    /// `OWNER/REPO` を上書き。
    #[arg(long, value_name = "OWNER/REPO")]
    repo: Option<String>,

    /// 取得する Issue の状態。
    #[arg(long, value_enum, default_value = "open")]
    state: IssueState,

    /// 取得件数。
    #[arg(long, default_value_t = 100)]
    limit: u16,

    /// label で絞り込む。複数指定可。
    #[arg(long, value_name = "LABEL")]
    label: Vec<String>,

    /// assignee で絞り込む。
    #[arg(long, value_name = "LOGIN")]
    assignee: Option<String>,

    /// author で絞り込む。
    #[arg(long, value_name = "LOGIN")]
    author: Option<String>,

    /// GitHub search query で絞り込む。
    #[arg(long, value_name = "QUERY")]
    search: Option<String>,

    /// AI engine CLI の起動コマンド (shell-words で分割)。
    #[arg(long, short = 'e', value_name = "CMD", default_value = "ccs_print c1")]
    engine_cmd: String,

    /// prompt を engine CLI の標準入力に渡す。
    #[arg(long)]
    prompt_stdin: bool,

    /// engine を起動せず、生成した prompt を stdout に出力する。
    #[arg(long)]
    print_prompt: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum IssueState {
    Open,
    Closed,
    All,
}

impl IssueState {
    fn as_gh_arg(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::All => "all",
        }
    }
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let repo = resolve_repo(self.repo.as_deref())?;
        let issues = fetch_issues(&repo, &self)?;
        let prompt = build_prompt(&repo, &self, &issues)?;

        if self.print_prompt {
            println!("{prompt}");
            return Ok(());
        }

        run_engine(&self.engine_cmd, &prompt, self.prompt_stdin)
    }
}

fn resolve_repo(repo_override: Option<&str>) -> Result<String> {
    if let Some(repo) = repo_override {
        validate_repo(repo)?;
        return Ok(repo.to_string());
    }

    let json = gh_capture(&["repo", "view", "--json", "nameWithOwner"])?;
    #[derive(Deserialize)]
    struct RepoView {
        #[serde(rename = "nameWithOwner")]
        name_with_owner: String,
    }
    let view: RepoView =
        serde_json::from_str(&json).context("failed to parse `gh repo view` JSON")?;
    validate_repo(&view.name_with_owner)?;
    Ok(view.name_with_owner)
}

fn validate_repo(repo: &str) -> Result<()> {
    let Some((owner, name)) = repo.split_once('/') else {
        bail!("repo must be OWNER/REPO");
    };
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        bail!("repo must be OWNER/REPO");
    }
    Ok(())
}

fn fetch_issues(repo: &str, cmd: &Cmd) -> Result<Value> {
    let limit = cmd.limit.to_string();
    let mut args = vec![
        "issue",
        "list",
        "--repo",
        repo,
        "--state",
        cmd.state.as_gh_arg(),
        "--limit",
        &limit,
        "--json",
        ISSUE_FIELDS,
    ];

    for label in &cmd.label {
        args.push("--label");
        args.push(label);
    }
    if let Some(assignee) = &cmd.assignee {
        args.push("--assignee");
        args.push(assignee);
    }
    if let Some(author) = &cmd.author {
        args.push("--author");
        args.push(author);
    }
    if let Some(search) = &cmd.search {
        args.push("--search");
        args.push(search);
    }

    let json = gh_capture(&args)?;
    serde_json::from_str(&json).context("failed to parse `gh issue list` JSON")
}

fn build_prompt(repo: &str, cmd: &Cmd, issues: &Value) -> Result<String> {
    let issue_count = issues.as_array().map_or(0, Vec::len);
    let input = json!({
        "repo": repo,
        "state": cmd.state.as_gh_arg(),
        "limit": cmd.limit,
        "labels": cmd.label,
        "assignee": cmd.assignee,
        "author": cmd.author,
        "search": cmd.search,
        "issue_count": issue_count,
        "issues": issues,
    });
    let input = serde_json::to_string_pretty(&input)?;

    Ok(format!(
        r#"GitHub Issue の棚卸しをしてください。

制約:
- 下の JSON は `rai` が GitHub CLI で取得済みの Issue 一覧です。
- GitHub Issue の取得は完了しています。あなたは `gh issue list`、`gh issue view`、Web閲覧、その他の手段で Issue を追加取得しないでください。
- JSON にない事実は断定せず、必要なら「追加確認が必要」と明記してください。
- Issue の更新、close、label 変更などの書き込み操作は行わないでください。

出力:
1. 全体サマリ
2. close 候補
3. 重複・統合候補
4. 分割した方がよい Issue
5. 優先して着手すべき Issue
6. 各 Issue の推奨アクション表 (#, title, action, reason)

入力 JSON:
```json
{input}
```
"#
    ))
}

fn run_engine(engine_cmd: &str, prompt: &str, prompt_stdin: bool) -> Result<()> {
    let mut argv = shell_words::split(engine_cmd).context("failed to split --engine-cmd")?;
    if argv.is_empty() {
        bail!("--engine-cmd is empty");
    }

    let mut command = Command::new(&argv[0]);
    if prompt_stdin {
        command.args(&argv[1..]).stdin(Stdio::piped());
        let mut child = command.spawn().context("failed to spawn engine command")?;
        let mut stdin = child.stdin.take().context("failed to open engine stdin")?;
        stdin
            .write_all(prompt.as_bytes())
            .context("failed to write prompt to engine stdin")?;
        drop(stdin);
        let status = child.wait().context("failed to wait engine command")?;
        if !status.success() {
            bail!("engine command exited with {:?}", status.code());
        }
        return Ok(());
    }

    argv.push(prompt.to_string());
    let status = command
        .args(&argv[1..])
        .status()
        .context("failed to spawn engine command")?;
    if !status.success() {
        bail!("engine command exited with {:?}", status.code());
    }
    Ok(())
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
    use serde_json::json;

    use super::{build_prompt, Cmd, IssueState};

    fn cmd() -> Cmd {
        Cmd {
            repo: None,
            state: IssueState::Open,
            limit: 100,
            label: vec!["bug".to_string()],
            assignee: Some("@me".to_string()),
            author: None,
            search: Some("sort:updated-desc".to_string()),
            engine_cmd: "ccs_print c1".to_string(),
            prompt_stdin: false,
            print_prompt: false,
        }
    }

    #[test]
    fn prompt_contains_fetched_issues_and_no_fetch_constraint() {
        let issues = json!([
            {
                "number": 42,
                "title": "Clean up old issues",
                "url": "https://github.com/owner/repo/issues/42",
                "state": "OPEN"
            }
        ]);

        let prompt = build_prompt("owner/repo", &cmd(), &issues).unwrap();

        assert!(prompt.contains("GitHub Issue の取得は完了しています"));
        assert!(prompt.contains("`gh issue list`"));
        assert!(prompt.contains("\"repo\": \"owner/repo\""));
        assert!(prompt.contains("\"number\": 42"));
        assert!(prompt.contains("\"issue_count\": 1"));
    }
}
