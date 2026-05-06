//! `rai develop pr` — 既存 PR の worktree に入り、コンフリ/CI 失敗を agent に修復させる。

use anyhow::{bail, Context};
use clap::Args;
use rai_core::{cli::Run, shell, Ctx, Result};
use serde::Deserialize;

use crate::common::{self, gh_capture, gwq_add_existing_branch, AgentArgs, Flavor, LaunchContext};

#[derive(Debug, Args)]
pub struct Cmd {
    /// PR 識別子: 番号 / URL / 省略 (省略時は fzf 複数選択)。
    #[arg(value_name = "PR")]
    pr: Vec<String>,

    /// `OWNER/REPO` を上書き。
    #[arg(long, value_name = "OWNER/REPO")]
    repo: Option<String>,

    #[command(flatten)]
    agent: AgentArgs,
}

#[derive(Debug, Deserialize)]
struct PrJson {
    number: u64,
    title: String,
    url: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "mergeable")]
    mergeable: Option<String>,
    #[serde(rename = "statusCheckRollup", default)]
    status_check_rollup: Vec<StatusCheck>,
    #[serde(rename = "headRepository")]
    head_repository: Option<HeadRepo>,
    #[serde(rename = "headRepositoryOwner")]
    head_repository_owner: Option<HeadOwner>,
}

#[derive(Debug, Deserialize)]
struct HeadRepo {
    name: String,
}

#[derive(Debug, Deserialize)]
struct HeadOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
struct StatusCheck {
    /// CheckRun → name, StatusContext → context. どちらかが入る。
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context: Option<String>,
    /// CheckRun: conclusion (SUCCESS / FAILURE / ...)
    /// StatusContext: state (SUCCESS / FAILURE / ERROR / PENDING)
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(rename = "detailsUrl", default)]
    details_url: Option<String>,
    #[serde(rename = "targetUrl", default)]
    target_url: Option<String>,
}

#[derive(Debug)]
struct Pr {
    owner: String,
    repo: String,
    number: u64,
    title: String,
    url: String,
    head_ref: String,
    base_ref: String,
    mergeable: Option<String>,
    failures: Vec<FailedCheck>,
    head_owner: Option<String>,
    head_repo: Option<String>,
}

#[derive(Debug, Clone)]
struct FailedCheck {
    name: String,
    state: String,
    url: Option<String>,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let prs = resolve_prs(&self)?;
        for pr in prs {
            run_one(&self, &pr)?;
        }
        Ok(())
    }
}

fn run_one(cmd: &Cmd, pr: &Pr) -> Result<()> {
    eprintln!("pr: {}", pr.url);
    eprintln!("branch: {}", pr.head_ref);

    if is_fork(pr, &pr.owner) {
        bail!(
            "PR #{} is from a fork ({}/{}); fork PRs are not yet supported by `rai develop pr`",
            pr.number,
            pr.head_owner.clone().unwrap_or_default(),
            pr.head_repo.clone().unwrap_or_default()
        );
    }

    let head_ref = pr.head_ref.clone();
    let wt = common::ensure_worktree(&head_ref, |branch| {
        ensure_local_branch_tracking_origin(branch)?;
        gwq_add_existing_branch(branch)
    })?;
    eprintln!("worktree: {}", wt.path.display());

    let prompt = build_prompt(cmd.agent.prompt_template.as_deref(), pr)?;
    let (_shell_path, shell_kind) = shell::detect_user_shell();
    let finalizer = if cmd.agent.no_auto_publish {
        None
    } else {
        Some(build_finalize_command(cmd, pr, shell_kind)?)
    };

    common::launch(
        &LaunchContext {
            repo: &pr.repo,
            branch: &head_ref,
            flavor: Flavor::Pr,
            number: pr.number,
            prompt: &prompt,
            finalizer: finalizer.as_deref(),
            agent: &cmd.agent,
        },
        &wt,
    )
}

fn is_fork(pr: &Pr, base_owner: &str) -> bool {
    match (&pr.head_owner, &pr.head_repo) {
        (Some(owner), Some(_)) => owner != base_owner,
        _ => false,
    }
}

fn ensure_local_branch_tracking_origin(branch: &str) -> Result<()> {
    // 1. Fetch latest remote ref so origin/<branch> is up-to-date.
    let st = shell::user_shell_argv(&["git", "fetch", "origin", branch])
        .status()
        .with_context(|| format!("failed to spawn `git fetch origin {branch}`"))?;
    if !st.success() {
        bail!("`git fetch origin {branch}` failed");
    }

    // 2. If local branch already exists, leave it alone — `gwq add` + `git pull --rebase`
    //    on the worktree side will reconcile.
    if local_branch_exists(branch)? {
        return Ok(());
    }

    // 3. Otherwise create a tracking branch from the remote ref.
    let upstream = format!("origin/{branch}");
    let st = shell::user_shell_argv(&["git", "branch", "--track", branch, &upstream])
        .status()
        .with_context(|| format!("failed to spawn `git branch --track {branch} {upstream}`"))?;
    if !st.success() {
        bail!("`git branch --track {branch} {upstream}` failed");
    }
    Ok(())
}

fn local_branch_exists(branch: &str) -> Result<bool> {
    let out = shell::user_shell_argv(&[
        "git",
        "show-ref",
        "--verify",
        "--quiet",
        &format!("refs/heads/{branch}"),
    ])
    .output()
    .context("failed to spawn `git show-ref`")?;
    Ok(out.status.success())
}

fn resolve_prs(cmd: &Cmd) -> Result<Vec<Pr>> {
    if !cmd.pr.is_empty() {
        let mut prs = Vec::with_capacity(cmd.pr.len());
        for arg in &cmd.pr {
            prs.push(resolve_pr_arg(cmd, arg)?);
        }
        return Ok(prs);
    }

    let (o, r) = common::resolve_repo(cmd.repo.as_deref())?;
    let json = gh_capture(&[
        "pr",
        "list",
        "--repo",
        &format!("{o}/{r}"),
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
        serde_json::from_str(&json).context("failed to parse `gh pr list` JSON")?;
    if items.is_empty() {
        bail!("no open PRs found");
    }
    let selected = common::pick_with_fzf(items.into_iter().map(|it| (it.number, it.title)))?;
    let mut prs = Vec::with_capacity(selected.len());
    for (n, _title) in selected {
        prs.push(fetch_pr(&o, &r, n)?);
    }
    Ok(prs)
}

fn resolve_pr_arg(cmd: &Cmd, arg: &str) -> Result<Pr> {
    if let Some((o, r, n)) = parse_pr_url(arg) {
        return fetch_pr(&o, &r, n);
    }
    if let Ok(n) = arg.parse::<u64>() {
        let (o, r) = common::resolve_repo(cmd.repo.as_deref())?;
        return fetch_pr(&o, &r, n);
    }
    bail!("invalid PR identifier: {arg}");
}

fn parse_pr_url(s: &str) -> Option<(String, String, u64)> {
    let stripped = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))?;
    let mut it = stripped.split('/');
    let owner = it.next()?.to_string();
    let repo = it.next()?.to_string();
    if it.next()? != "pull" {
        return None;
    }
    let n: u64 = it.next()?.parse().ok()?;
    Some((owner, repo, n))
}

fn fetch_pr(owner: &str, repo: &str, number: u64) -> Result<Pr> {
    let json = gh_capture(&[
        "pr",
        "view",
        &number.to_string(),
        "--repo",
        &format!("{owner}/{repo}"),
        "--json",
        "number,title,url,headRefName,baseRefName,mergeable,statusCheckRollup,headRepository,headRepositoryOwner",
    ])?;
    let v: PrJson = serde_json::from_str(&json).context("failed to parse `gh pr view` JSON")?;
    let failures = v
        .status_check_rollup
        .iter()
        .filter_map(failed_from)
        .collect();
    Ok(Pr {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number: v.number,
        title: v.title,
        url: v.url,
        head_ref: v.head_ref_name,
        base_ref: v.base_ref_name,
        mergeable: v.mergeable,
        failures,
        head_owner: v.head_repository_owner.map(|o| o.login),
        head_repo: v.head_repository.map(|r| r.name),
    })
}

fn failed_from(check: &StatusCheck) -> Option<FailedCheck> {
    let outcome = check
        .conclusion
        .clone()
        .or_else(|| check.state.clone())
        .unwrap_or_default();
    let upper = outcome.to_uppercase();
    if upper == "FAILURE" || upper == "ERROR" || upper == "TIMED_OUT" || upper == "CANCELLED" {
        Some(FailedCheck {
            name: check
                .name
                .clone()
                .or_else(|| check.context.clone())
                .unwrap_or_else(|| "<unnamed-check>".to_string()),
            state: upper,
            url: check
                .details_url
                .clone()
                .or_else(|| check.target_url.clone()),
        })
    } else {
        None
    }
}

fn build_prompt(template: Option<&std::path::Path>, pr: &Pr) -> Result<String> {
    if let Some(p) = template {
        let body = common::read_prompt_template(p)?;
        return Ok(body
            .replace("{PR_URL}", &pr.url)
            .replace("{PR_TITLE}", &pr.title)
            .replace("{PR_NUMBER}", &pr.number.to_string())
            .replace("{PR_HEAD}", &pr.head_ref)
            .replace("{PR_BASE}", &pr.base_ref));
    }

    let mergeable = pr.mergeable.as_deref().unwrap_or("UNKNOWN");
    let conflict_section = if mergeable.eq_ignore_ascii_case("CONFLICTING") {
        format!(
            "- このブランチは base `{base}` とコンフリクトしています。`git fetch origin {base}` のあと `git merge origin/{base}` (または `git rebase origin/{base}`) でコンフリクトを解消し、各ファイルの解消結果が PR の意図に沿っているかを確認してから commit してください。",
            base = pr.base_ref
        )
    } else {
        String::new()
    };

    let ci_section = if pr.failures.is_empty() {
        String::new()
    } else {
        let mut lines = vec!["- 以下の CI ジョブが失敗しています。失敗ログを `gh run view --log-failed` 等で取得し、根本原因を修正してください:".to_string()];
        for f in &pr.failures {
            match &f.url {
                Some(url) => lines.push(format!("  - `{}` ({}): {url}", f.name, f.state)),
                None => lines.push(format!("  - `{}` ({})", f.name, f.state)),
            }
        }
        lines.join("\n")
    };

    let nothing_to_do = conflict_section.is_empty() && ci_section.is_empty();
    let body = if nothing_to_do {
        format!(
            "PR {url} (`{title}`) は現状コンフリクトも CI 失敗も検出されていません。レビューコメントなど他の指摘があれば対応し、必要なら追加 commit を push してください。",
            url = pr.url,
            title = pr.title
        )
    } else {
        let mut sections = vec![format!(
            "PR {url} (`{title}`) について、以下を実施してください:",
            url = pr.url,
            title = pr.title
        )];
        if !conflict_section.is_empty() {
            sections.push(conflict_section);
        }
        if !ci_section.is_empty() {
            sections.push(ci_section);
        }
        sections.push(
            "- 修正内容をテスト・ビルド・lint でローカル検証し、commit & `git push` で同じブランチに反映してください。"
                .to_string(),
        );
        sections.push(
            "- commit-msg hook がメッセージを弾いた場合は直して再 commit すること。`--no-verify` 等の hook 回避は禁止です。"
                .to_string(),
        );
        sections.push(
            "- 新規 PR は作成しないでください。既存 PR への追加 push が前提です。".to_string(),
        );
        sections.join("\n")
    };
    Ok(body)
}

fn build_finalize_command(
    cmd: &Cmd,
    pr: &Pr,
    shell_kind: rai_core::shell::Shell,
) -> Result<String> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let q = shell::quote_for(shell_kind);
    let mut parts = vec![
        shell::quote_path(shell_kind, &exe),
        "develop".to_string(),
        "finalize-agent".to_string(),
        "--flavor".to_string(),
        "pr".to_string(),
        "--url".to_string(),
        q(&pr.url),
        "--number".to_string(),
        pr.number.to_string(),
        "--title".to_string(),
        q(&pr.title),
        "--repo".to_string(),
        q(&format!("{}/{}", pr.owner, pr.repo)),
        "--branch".to_string(),
        q(&pr.head_ref),
        "--engine-cmd".to_string(),
        q(&cmd.agent.engine_cmd),
        "--pr-base".to_string(),
        q(&pr.base_ref),
    ];
    if let Some(mode) = cmd.agent.permission_mode {
        parts.push("--permission-mode".to_string());
        parts.push(mode.as_arg().to_string());
    }
    Ok(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pr(mergeable: &str, failures: Vec<FailedCheck>) -> Pr {
        Pr {
            owner: "o".into(),
            repo: "r".into(),
            number: 42,
            title: "Fix stuff".into(),
            url: "https://github.com/o/r/pull/42".into(),
            head_ref: "feature/x".into(),
            base_ref: "main".into(),
            mergeable: Some(mergeable.into()),
            failures,
            head_owner: Some("o".into()),
            head_repo: Some("r".into()),
        }
    }

    #[test]
    fn prompt_calls_out_conflict_when_pr_is_conflicting() {
        let pr = sample_pr("CONFLICTING", Vec::new());
        let prompt = build_prompt(None, &pr).unwrap();
        assert!(prompt.contains("コンフリクトしています"));
        assert!(prompt.contains("origin/main"));
        assert!(prompt.contains("--no-verify"));
        assert!(prompt.contains("新規 PR は作成しないでください"));
    }

    #[test]
    fn prompt_lists_failed_ci_jobs() {
        let pr = sample_pr(
            "MERGEABLE",
            vec![
                FailedCheck {
                    name: "ci/test".into(),
                    state: "FAILURE".into(),
                    url: Some("https://example/jobs/1".into()),
                },
                FailedCheck {
                    name: "ci/lint".into(),
                    state: "FAILURE".into(),
                    url: None,
                },
            ],
        );
        let prompt = build_prompt(None, &pr).unwrap();
        assert!(prompt.contains("CI ジョブが失敗"));
        assert!(prompt.contains("`ci/test`"));
        assert!(prompt.contains("https://example/jobs/1"));
        assert!(prompt.contains("`ci/lint`"));
    }

    #[test]
    fn prompt_falls_back_when_nothing_to_do() {
        let pr = sample_pr("MERGEABLE", Vec::new());
        let prompt = build_prompt(None, &pr).unwrap();
        assert!(prompt.contains("コンフリクトも CI 失敗も検出されていません"));
    }

    #[test]
    fn parse_pr_url_extracts_owner_repo_number() {
        assert_eq!(
            parse_pr_url("https://github.com/o/r/pull/42"),
            Some(("o".into(), "r".into(), 42))
        );
        assert!(parse_pr_url("not a url").is_none());
        assert!(parse_pr_url("https://github.com/o/r/issues/42").is_none());
    }

    #[test]
    fn is_fork_when_head_owner_differs() {
        let mut pr = sample_pr("MERGEABLE", Vec::new());
        pr.head_owner = Some("forkuser".into());
        assert!(is_fork(&pr, "o"));
        pr.head_owner = Some("o".into());
        assert!(!is_fork(&pr, "o"));
    }
}
