//! `rai develop resume` — 既存 worktree を保持したまま、agent セッションを再開する。
//!
//! `rai develop issue` / `rai develop pr` のセッションが rate limit や
//! context limit で途中終了した時の復帰用。既存 worktree を `git reset` /
//! `git pull` で巻き戻さずに、resume 専用 prompt で agent を再起動する。

use anyhow::{bail, Context};
use clap::Args;
use rai_core::{cli::Run, shell, Ctx, Result};
use serde::Deserialize;

use crate::common::{self, find_existing_worktree, gh_capture, AgentArgs, Flavor, LaunchContext};
use crate::finalize;

#[derive(Debug, Args)]
pub struct Cmd {
    /// 復帰対象の Issue / PR: 番号 / URL を 1 つ以上。
    #[arg(value_name = "TARGET", required = true)]
    target: Vec<String>,

    /// `OWNER/REPO` を上書き。未指定なら現在の git リポジトリから解決。
    #[arg(long, value_name = "OWNER/REPO")]
    repo: Option<String>,

    /// 数値だけ渡された TARGET の flavor。URL を渡した場合は無視する。
    #[arg(long, value_enum)]
    flavor: Option<Flavor>,

    /// issue flavor で同じ番号の worktree が複数あるときに直接指定する branch 名。
    /// 単一 TARGET にのみ使える。
    #[arg(long, short = 'b', value_name = "BRANCH")]
    branch: Option<String>,

    /// issue flavor の finalize agent に渡す PR base branch。
    #[arg(long, value_name = "BRANCH")]
    pr_base: Option<String>,

    #[command(flatten)]
    agent: AgentArgs,
}

#[derive(Debug)]
struct Target {
    flavor: Flavor,
    owner: String,
    repo: String,
    number: u64,
    title: String,
    url: String,
    branch: String,
    /// pr flavor のみ。finalize agent や prompt 用に保持する。
    base_ref: Option<String>,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        if self.target.len() > 1 && self.branch.is_some() {
            bail!("--branch can only be used with a single TARGET");
        }
        let targets = resolve_targets(&self)?;
        for t in targets {
            run_one(&self, &t)?;
        }
        Ok(())
    }
}

fn run_one(cmd: &Cmd, target: &Target) -> Result<()> {
    eprintln!("resume: {}", target.url);
    eprintln!("branch: {}", target.branch);

    let wt = find_existing_worktree(&target.branch).with_context(|| {
        format!(
            "cannot resume `{}`: no worktree for branch `{}`. Start with `rai develop {} {}` instead.",
            target.url,
            target.branch,
            target.flavor.label(),
            target.number
        )
    })?;
    eprintln!("worktree: {}", wt.path.display());

    let prompt = build_resume_prompt(target, !cmd.agent.no_auto_publish)?;
    let (_shell_path, shell_kind) = shell::detect_user_shell();
    let finalizer = if cmd.agent.no_auto_publish {
        None
    } else {
        Some(build_finalize_command(cmd, target, shell_kind)?)
    };

    common::launch(
        &LaunchContext {
            repo: &target.repo,
            branch: &target.branch,
            flavor: target.flavor,
            number: target.number,
            prompt: &prompt,
            finalizer: finalizer.as_deref(),
            agent: &cmd.agent,
        },
        &wt,
    )
}

fn resolve_targets(cmd: &Cmd) -> Result<Vec<Target>> {
    let mut out = Vec::with_capacity(cmd.target.len());
    for arg in &cmd.target {
        out.push(resolve_target(cmd, arg)?);
    }
    Ok(out)
}

fn resolve_target(cmd: &Cmd, arg: &str) -> Result<Target> {
    if let Some((owner, repo, number, flavor)) = parse_url(arg) {
        return build_target(cmd, &owner, &repo, number, flavor);
    }
    if let Ok(n) = arg.parse::<u64>() {
        let (owner, repo) = common::resolve_repo(cmd.repo.as_deref())?;
        let flavor = cmd.flavor.unwrap_or(Flavor::Issue);
        return build_target(cmd, &owner, &repo, n, flavor);
    }
    bail!("invalid TARGET: {arg} (expected number or GitHub issue/pull URL)")
}

fn build_target(cmd: &Cmd, owner: &str, repo: &str, number: u64, flavor: Flavor) -> Result<Target> {
    match flavor {
        Flavor::Issue => {
            let title = fetch_issue_title(owner, repo, number)?;
            let url = format!("https://github.com/{owner}/{repo}/issues/{number}");
            let branch = resolve_issue_branch(cmd, number)?;
            Ok(Target {
                flavor,
                owner: owner.to_string(),
                repo: repo.to_string(),
                number,
                title,
                url,
                branch,
                base_ref: None,
            })
        }
        Flavor::Pr => {
            let pr = fetch_pr(owner, repo, number)?;
            if pr.is_fork(owner) {
                bail!(
                    "PR #{number} is from a fork ({}/{}); fork PRs are not supported by `rai develop resume`",
                    pr.head_owner.unwrap_or_default(),
                    pr.head_repo.unwrap_or_default()
                );
            }
            Ok(Target {
                flavor,
                owner: owner.to_string(),
                repo: repo.to_string(),
                number: pr.number,
                title: pr.title,
                url: pr.url,
                branch: pr.head_ref,
                base_ref: Some(pr.base_ref),
            })
        }
    }
}

fn resolve_issue_branch(cmd: &Cmd, number: u64) -> Result<String> {
    if let Some(b) = cmd.branch.as_deref() {
        return Ok(b.to_string());
    }
    let mut candidates = common::issue_branches_for(number)?;
    candidates.sort();
    candidates.dedup();
    match candidates.len() {
        0 => bail!(
            "no local branch matching `develop/issue-{number}-*` found. Pass --branch to specify, or run `rai develop issue {number}` to start fresh."
        ),
        1 => Ok(candidates.into_iter().next().unwrap()),
        // 候補が複数あるときは fzf で 1 つ選ばせる。`pick_with_fzf` は `(number, title)`
        // 用 API で `#<n>\t<value>` を画面に出してしまうので、ここでは branch 名だけを
        // 1 行ずつ流す純粋な文字列セレクタ `pick_string_with_fzf` を使う。
        _ => common::pick_string_with_fzf(candidates),
    }
}

fn parse_url(s: &str) -> Option<(String, String, u64, Flavor)> {
    let stripped = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))?;
    let mut it = stripped.split('/');
    let owner = it.next()?.to_string();
    let repo = it.next()?.to_string();
    let kind = it.next()?;
    let n: u64 = it.next()?.parse().ok()?;
    let flavor = match kind {
        "issues" => Flavor::Issue,
        "pull" | "pulls" => Flavor::Pr,
        _ => return None,
    };
    Some((owner, repo, n, flavor))
}

fn fetch_issue_title(owner: &str, repo: &str, number: u64) -> Result<String> {
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

#[derive(Debug, Deserialize)]
struct PrJson {
    number: u64,
    title: String,
    url: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
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

#[derive(Debug)]
struct PrLite {
    number: u64,
    title: String,
    url: String,
    head_ref: String,
    base_ref: String,
    head_owner: Option<String>,
    head_repo: Option<String>,
}

impl PrLite {
    fn is_fork(&self, base_owner: &str) -> bool {
        match (&self.head_owner, &self.head_repo) {
            (Some(owner), Some(_)) => owner != base_owner,
            _ => false,
        }
    }
}

fn fetch_pr(owner: &str, repo: &str, number: u64) -> Result<PrLite> {
    let json = gh_capture(&[
        "pr",
        "view",
        &number.to_string(),
        "--repo",
        &format!("{owner}/{repo}"),
        "--json",
        "number,title,url,headRefName,baseRefName,headRepository,headRepositoryOwner",
    ])?;
    let v: PrJson = serde_json::from_str(&json).context("failed to parse `gh pr view` JSON")?;
    Ok(PrLite {
        number: v.number,
        title: v.title,
        url: v.url,
        head_ref: v.head_ref_name,
        base_ref: v.base_ref_name,
        head_owner: v.head_repository_owner.map(|o| o.login),
        head_repo: v.head_repository.map(|r| r.name),
    })
}

fn build_resume_prompt(target: &Target, auto_publish: bool) -> Result<String> {
    Ok(match target.flavor {
        Flavor::Issue => {
            issue_resume_prompt(&target.url, &target.title, &target.branch, auto_publish)
        }
        Flavor::Pr => pr_resume_prompt(&target.url, &target.title, &target.branch, target.number),
    })
}

fn issue_resume_prompt(url: &str, title: &str, branch: &str, auto_publish: bool) -> String {
    // auto_publish=true (デフォルト): rai は agent 終了後に finalize agent を起動して
    // PR 作成までの片付けを任せる。agent 側にも「PR を出すところまで」と伝え、
    // finalize が走らなかった場合のフォールバックを兼ねる。
    // auto_publish=false (--no-auto-publish): finalize は起動されないので、agent が
    // 単独で PR 作成まで完了させる必要がある。
    let publish_clause = if auto_publish {
        "残作業を仕上げて PR を出すところまで完了させてください。\
テスト・ビルド・lint をローカルで通し、commit & push、\
`gh pr create` で PR を作成 (PR 本文に `Closes {url}`)。\
既に同じブランチへの PR があれば新規作成せず、追加 push のみ行ってください。"
    } else {
        // Rust の行継続 `\<newline>` は次行の先頭空白も食うので、`(` は **行末側** に
        // 置いて改行前にスペース + `(` をまとめておく。これで生成プロンプトは
        // `完結させてください (`--no-auto-publish`…` と自然な間隔になる。
        "残作業を仕上げて **agent 自身** で PR 作成まで完結させてください (\
`--no-auto-publish` 指定のため rai 側は後段の finalize agent を起動しません)。\
テスト・ビルド・lint をローカルで通し、commit & push、\
`gh pr create` で PR を作成 (PR 本文に `Closes {url}`)。\
既に同じブランチへの PR があれば新規作成せず、追加 push のみ行ってください。"
    };
    let publish_clause = publish_clause.replace("{url}", url);
    format!(
        "GitHub Issue {url} (`{title}`) の作業を **途中から** 再開してください。\
worktree のブランチは `{branch}`。前回のセッションは rate limit / context limit / tmux 事故などで途中終了しています。\
最初に必ず `git status` と `git log --oneline -20` で現在の進捗 (未コミット変更・既存 commit) を把握し、\
未コミット変更があれば論理的な単位で commit を整えながら、{publish_clause}\
commit-msg hook がメッセージを弾いた場合はメッセージを直して再 commit してください。`--no-verify` 等の hook 回避は禁止です。"
    )
}

fn pr_resume_prompt(url: &str, title: &str, branch: &str, number: u64) -> String {
    format!(
        "GitHub PR {url} (`{title}`) の作業を **途中から** 再開してください。\
worktree のブランチは `{branch}`。前回のセッションは rate limit / context limit / tmux 事故などで途中終了しています。\
最初に `git status`, `git log --oneline -20` で worktree の進捗を確認し、\
`gh pr view {number}` / `gh pr checks {number}` で PR の最新状態 (mergeable, CI) も確認してください。\
コンフリクト解消や CI 失敗修正など残作業を仕上げ、commit & `git push origin HEAD:{branch}` で同じ PR ブランチに反映してください。\
**新規 PR は作成しないでください**。既存 PR への追加 push が前提です。\
commit-msg hook がメッセージを弾いた場合はメッセージを直して再 commit してください。`--no-verify` 等の hook 回避は禁止です。"
    )
}

fn build_finalize_command(
    cmd: &Cmd,
    target: &Target,
    shell_kind: rai_core::shell::Shell,
) -> Result<String> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let q = shell::quote_for(shell_kind);
    let mut parts = vec![
        shell::quote_path(shell_kind, &exe),
        "develop".to_string(),
        "finalize-agent".to_string(),
        "--flavor".to_string(),
        target.flavor.label().to_string(),
        "--url".to_string(),
        q(&target.url),
        "--number".to_string(),
        target.number.to_string(),
        "--title".to_string(),
        q(&target.title),
        "--repo".to_string(),
        q(&format!("{}/{}", target.owner, target.repo)),
        "--branch".to_string(),
        q(&target.branch),
        "--engine-cmd".to_string(),
        q(&cmd.agent.engine_cmd),
    ];
    if let Some(mode) = cmd.agent.permission_mode {
        parts.push("--permission-mode".to_string());
        parts.push(mode.as_arg().to_string());
    }
    let pr_base = match target.flavor {
        Flavor::Issue => cmd
            .pr_base
            .clone()
            .or_else(finalize::local_origin_head_branch),
        Flavor::Pr => target.base_ref.clone(),
    };
    if let Some(base) = pr_base.as_deref() {
        parts.push("--pr-base".to_string());
        parts.push(q(base));
    }
    Ok(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_url_handles_issue_and_pull() {
        assert_eq!(
            parse_url("https://github.com/o/r/issues/9"),
            Some(("o".into(), "r".into(), 9, Flavor::Issue))
        );
        assert_eq!(
            parse_url("https://github.com/o/r/pull/42"),
            Some(("o".into(), "r".into(), 42, Flavor::Pr))
        );
        assert!(parse_url("not a url").is_none());
        assert!(parse_url("https://github.com/o/r/discussions/3").is_none());
    }

    #[test]
    fn issue_resume_prompt_auto_publish_true() {
        let p = issue_resume_prompt(
            "https://github.com/o/r/issues/9",
            "Resume me",
            "develop/issue-9-resume-me-20260504-101010",
            true,
        );
        assert_eq!(
            p,
            "GitHub Issue https://github.com/o/r/issues/9 (`Resume me`) の作業を **途中から** 再開してください。worktree のブランチは `develop/issue-9-resume-me-20260504-101010`。前回のセッションは rate limit / context limit / tmux 事故などで途中終了しています。最初に必ず `git status` と `git log --oneline -20` で現在の進捗 (未コミット変更・既存 commit) を把握し、未コミット変更があれば論理的な単位で commit を整えながら、残作業を仕上げて PR を出すところまで完了させてください。テスト・ビルド・lint をローカルで通し、commit & push、`gh pr create` で PR を作成 (PR 本文に `Closes https://github.com/o/r/issues/9`)。既に同じブランチへの PR があれば新規作成せず、追加 push のみ行ってください。commit-msg hook がメッセージを弾いた場合はメッセージを直して再 commit してください。`--no-verify` 等の hook 回避は禁止です。"
        );
    }

    #[test]
    fn issue_resume_prompt_auto_publish_false_tells_agent_to_finish_pr_itself() {
        let p = issue_resume_prompt(
            "https://github.com/o/r/issues/9",
            "Resume me",
            "develop/issue-9-resume-me-20260504-101010",
            false,
        );
        assert_eq!(
            p,
            "GitHub Issue https://github.com/o/r/issues/9 (`Resume me`) の作業を **途中から** 再開してください。worktree のブランチは `develop/issue-9-resume-me-20260504-101010`。前回のセッションは rate limit / context limit / tmux 事故などで途中終了しています。最初に必ず `git status` と `git log --oneline -20` で現在の進捗 (未コミット変更・既存 commit) を把握し、未コミット変更があれば論理的な単位で commit を整えながら、残作業を仕上げて **agent 自身** で PR 作成まで完結させてください (`--no-auto-publish` 指定のため rai 側は後段の finalize agent を起動しません)。テスト・ビルド・lint をローカルで通し、commit & push、`gh pr create` で PR を作成 (PR 本文に `Closes https://github.com/o/r/issues/9`)。既に同じブランチへの PR があれば新規作成せず、追加 push のみ行ってください。commit-msg hook がメッセージを弾いた場合はメッセージを直して再 commit してください。`--no-verify` 等の hook 回避は禁止です。"
        );
    }

    #[test]
    fn pr_resume_prompt_forbids_new_pr() {
        let p = pr_resume_prompt("https://github.com/o/r/pull/42", "Fix CI", "feature/x", 42);
        assert_eq!(
            p,
            "GitHub PR https://github.com/o/r/pull/42 (`Fix CI`) の作業を **途中から** 再開してください。worktree のブランチは `feature/x`。前回のセッションは rate limit / context limit / tmux 事故などで途中終了しています。最初に `git status`, `git log --oneline -20` で worktree の進捗を確認し、`gh pr view 42` / `gh pr checks 42` で PR の最新状態 (mergeable, CI) も確認してください。コンフリクト解消や CI 失敗修正など残作業を仕上げ、commit & `git push origin HEAD:feature/x` で同じ PR ブランチに反映してください。**新規 PR は作成しないでください**。既存 PR への追加 push が前提です。commit-msg hook がメッセージを弾いた場合はメッセージを直して再 commit してください。`--no-verify` 等の hook 回避は禁止です。"
        );
    }
}
