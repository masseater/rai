//! `rai develop issue` — Issue を起点に worktree + tmux + agent を起動する。

use anyhow::{bail, Context};
use chrono::Local;
use clap::Args;
use rai_core::{cli::Run, shell, Ctx, Result};
use serde::Deserialize;

use crate::common::{self, gh_capture, gwq_add_new_branch, AgentArgs, Flavor, LaunchContext};
use crate::finalize;

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

    /// 自動作成する PR の base branch。
    #[arg(long, value_name = "BRANCH")]
    pr_base: Option<String>,

    #[command(flatten)]
    agent: AgentArgs,
}

#[derive(Debug)]
struct Issue {
    owner: String,
    repo: String,
    number: u64,
    title: String,
    url: String,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        // `--branch` は単一 issue 専用。
        // 1. CLI 引数で 2 件以上渡された場合 → ここで弾く (fzf を起動せずに済む)。
        // 2. CLI 引数が空 (= fzf モード) で `--branch` 指定 → 「fzf で何件選ばれるか
        //    実行するまで分からない上に、ユーザーに対話的に選ばせた後で弾くのは
        //    UX 的に最悪」なので、fzf を起動する前に弾く。
        // 3. fzf で 2 件以上選んでしまった場合 → `resolve_issues` 後の最終チェック
        //    (= 念のための safety net)。
        if self.issue.len() > 1 && self.branch.is_some() {
            bail!("--branch can only be used with a single issue");
        }
        if self.issue.is_empty() && self.branch.is_some() {
            bail!(
                "--branch cannot be combined with interactive (fzf) issue selection; \
                 pass a single issue number or URL explicitly"
            );
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

fn run_one(cmd: &Cmd, issue: &Issue) -> Result<()> {
    eprintln!("issue: {}", issue.url);

    let branch = match &cmd.branch {
        Some(b) => b.clone(),
        None => default_branch(&issue.title, issue.number),
    };
    eprintln!("branch: {branch}");

    let wt = common::ensure_worktree(&branch, gwq_add_new_branch)?;
    eprintln!("worktree: {}", wt.path.display());

    let prompt = build_prompt(
        cmd.agent.prompt_template.as_deref(),
        &issue.url,
        &issue.title,
        !cmd.agent.no_auto_publish,
    )?;

    let (_shell_path, shell_kind) = shell::detect_user_shell();
    let finalizer = if cmd.agent.no_auto_publish {
        None
    } else {
        Some(build_finalize_command(cmd, issue, &branch, shell_kind)?)
    };

    common::launch(
        &LaunchContext {
            repo: &issue.repo,
            branch: &branch,
            flavor: Flavor::Issue,
            number: issue.number,
            prompt: &prompt,
            finalizer: finalizer.as_deref(),
            agent: &cmd.agent,
        },
        &wt,
    )
}

fn resolve_issues(cmd: &Cmd) -> Result<Vec<Issue>> {
    if !cmd.issue.is_empty() {
        let mut issues = Vec::with_capacity(cmd.issue.len());
        for arg in &cmd.issue {
            issues.push(resolve_issue_arg(cmd, arg)?);
        }
        return Ok(issues);
    }

    let (o, r) = common::resolve_repo(cmd.repo.as_deref())?;
    let json = gh_capture(&[
        "issue",
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
        serde_json::from_str(&json).context("failed to parse `gh issue list` JSON")?;
    if items.is_empty() {
        bail!("no open issues found");
    }
    let selected = common::pick_with_fzf(items.into_iter().map(|it| (it.number, it.title)))?;
    Ok(selected
        .into_iter()
        .map(|(n, title)| Issue {
            owner: o.clone(),
            repo: r.clone(),
            number: n,
            title,
            url: format!("https://github.com/{o}/{r}/issues/{n}"),
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
        let (o, r) = common::resolve_repo(cmd.repo.as_deref())?;
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

fn build_prompt(
    template: Option<&std::path::Path>,
    url: &str,
    title: &str,
    auto_publish: bool,
) -> Result<String> {
    if let Some(p) = template {
        let body = common::read_prompt_template(p)?;
        return Ok(body
            .replace("{ISSUE_URL}", url)
            .replace("{ISSUE_TITLE}", title));
    }
    // 役割分担:
    // - auto_publish=true (デフォルト): agent は commit & push まで、PR 作成は後段の
    //   finalize agent に任せる。二重 `gh pr create` の試行を避ける + PR 本文の
    //   テンプレを finalize 側に集約できる。
    // - auto_publish=false (`--no-auto-publish`): finalize は起動しないので、agent
    //   自身が PR 作成まで完結する。既存 PR の重複作成を避ける safety net も明示。
    if auto_publish {
        Ok(format!(
            "GitHub Issue {url} (`{title}`) を実装し、テスト・ビルド・lint をローカルで通したうえで、論理単位の commit を作って push するところまで自走してください。PR 作成 (= `gh pr create`) は **rai が後段で finalize agent を起動して担当する** ので、agent 側からは作成しないでください。commit-msg hook がメッセージを弾いた場合はメッセージを直して commit し直してください。`--no-verify` などで hook を回避するのは禁止です。"
        ))
    } else {
        Ok(format!(
            "GitHub Issue {url} (`{title}`) を一気通貫で開発し、commit、push、`gh pr create` で PR を作成するところまで自走してください (rai 側は `--no-auto-publish` 指定のため finalize agent を起動しません)。テスト・ビルド・lint をローカルで通すこと。PR 本文には `Closes {url}` を含めてください。既に同じブランチへの PR があれば新規作成せず、追加 push のみ行ってください。commit-msg hook がメッセージを弾いたらメッセージを直して commit し直してください。`--no-verify` などで hook を回避するのは禁止です。"
        ))
    }
}

fn build_finalize_command(
    cmd: &Cmd,
    issue: &Issue,
    branch: &str,
    shell_kind: rai_core::shell::Shell,
) -> Result<String> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let q = shell::quote_for(shell_kind);
    let mut parts = vec![
        shell::quote_path(shell_kind, &exe),
        "develop".to_string(),
        "finalize-agent".to_string(),
        "--flavor".to_string(),
        "issue".to_string(),
        "--url".to_string(),
        q(&issue.url),
        "--number".to_string(),
        issue.number.to_string(),
        "--title".to_string(),
        q(&issue.title),
        "--repo".to_string(),
        q(&format!("{}/{}", issue.owner, issue.repo)),
        "--branch".to_string(),
        q(branch),
        "--engine-cmd".to_string(),
        q(&cmd.agent.engine_cmd),
    ];
    if let Some(mode) = cmd.agent.permission_mode {
        parts.push("--permission-mode".to_string());
        parts.push(mode.as_arg().to_string());
    }
    let pr_base = cmd
        .pr_base
        .clone()
        .or_else(finalize::local_origin_head_branch);
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
    fn default_branch_uses_develop_issue_prefix() {
        let branch = default_branch("Add issue workflow", 9);
        assert!(branch.starts_with("develop/issue-9-add-issue-workflow-"));
    }

    #[test]
    fn default_branch_without_slug_uses_develop_issue_prefix() {
        let branch = default_branch("!!!", 9);
        assert!(branch.starts_with("develop/issue-9-"));
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
    fn slugify_japanese_title_falls_back_to_empty() {
        // ASCII 英数字以外は全て区切りに変換されるため、純日本語タイトルは空に落ちる。
        // `default_branch` 側でこの場合に `develop/issue-{number}-{ts}` に倒すフォールバック
        // を用意しているので、空文字列が返ることが期待挙動。
        assert_eq!(slugify("バグ修正"), "");
        // 日本語混じり (ASCII 部分が拾われる) も検証する。
        assert_eq!(slugify("バグ fix for foo-bar"), "fix-for-foo-bar");
    }

    #[test]
    fn default_branch_for_japanese_title_uses_number_timestamp_only() {
        let branch = default_branch("バグ修正", 42);
        assert!(branch.starts_with("develop/issue-42-"), "branch = {branch}");
        // slug 部分が無いので、`develop/issue-{number}-{ts}` 形 (ts は数値とハイフンのみ)。
        let rest = branch.strip_prefix("develop/issue-42-").unwrap();
        assert!(
            rest.chars().all(|c| c.is_ascii_digit() || c == '-'),
            "rest = {rest}"
        );
    }

    #[test]
    fn default_prompt_auto_publish_stops_at_push_and_defers_pr_to_finalize() {
        let prompt = build_prompt(
            None,
            "https://github.com/o/r/issues/13",
            "Auto publish",
            true,
        )
        .unwrap();
        // build_prompt の戻り値は引数から一意。AGENTS.md Testing ガイドラインに従い
        // 完全一致で検証する。auto_publish=true では agent は commit&push まで、PR
        // 作成は後段の finalize agent に任せる役割分担を明示する。
        assert_eq!(
            prompt,
            "GitHub Issue https://github.com/o/r/issues/13 (`Auto publish`) を実装し、テスト・ビルド・lint をローカルで通したうえで、論理単位の commit を作って push するところまで自走してください。PR 作成 (= `gh pr create`) は **rai が後段で finalize agent を起動して担当する** ので、agent 側からは作成しないでください。commit-msg hook がメッセージを弾いた場合はメッセージを直して commit し直してください。`--no-verify` などで hook を回避するのは禁止です。"
        );
    }

    #[test]
    fn default_prompt_when_auto_publish_disabled() {
        let prompt = build_prompt(
            None,
            "https://github.com/o/r/issues/13",
            "Manual publish",
            false,
        )
        .unwrap();
        assert_eq!(
            prompt,
            "GitHub Issue https://github.com/o/r/issues/13 (`Manual publish`) を一気通貫で開発し、commit、push、`gh pr create` で PR を作成するところまで自走してください (rai 側は `--no-auto-publish` 指定のため finalize agent を起動しません)。テスト・ビルド・lint をローカルで通すこと。PR 本文には `Closes https://github.com/o/r/issues/13` を含めてください。既に同じブランチへの PR があれば新規作成せず、追加 push のみ行ってください。commit-msg hook がメッセージを弾いたらメッセージを直して commit し直してください。`--no-verify` などで hook を回避するのは禁止です。"
        );
    }
}
