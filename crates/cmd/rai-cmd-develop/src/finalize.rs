//! `rai develop finalize-agent` — agent 完走後の commit / push / PR 作成 (or push) を仕上げる。

use anyhow::{bail, Context};
use clap::Args;
use rai_core::{cli::Run, shell, Ctx, Result};

use crate::common::{self, gh_capture, git_capture, Flavor, PermissionMode};

#[derive(Debug, Args)]
pub struct Cmd {
    /// 起点が Issue / PR どちらか。
    #[arg(long, value_enum)]
    flavor: Flavor,

    /// Issue / PR の URL。
    #[arg(long)]
    url: String,

    /// Issue / PR の番号。
    #[arg(long)]
    number: u64,

    /// Issue / PR のタイトル。
    #[arg(long)]
    title: String,

    /// `OWNER/REPO`。
    #[arg(long)]
    repo: String,

    /// 作業ブランチ。
    #[arg(long)]
    branch: String,

    /// PR base branch (issue では PR 作成時に使う / pr では merge 候補)。
    #[arg(long)]
    pr_base: Option<String>,

    /// engine_cmd template.
    #[arg(long, value_name = "CMD", default_value = common::DEFAULT_ENGINE_CMD)]
    engine_cmd: String,

    /// `--permission-mode` を実装 agent と揃える。
    #[arg(long, value_name = "MODE", value_enum)]
    permission_mode: Option<PermissionMode>,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        finalize_after_agent(&self)
    }
}

fn finalize_after_agent(ctx: &Cmd) -> Result<()> {
    eprintln!("rai: agent completed; checking local state");

    let has_local = has_local_changes()?;
    let has_commits = has_publishable_commits(ctx.pr_base.as_deref())?;

    if !has_local && !has_commits {
        match ctx.flavor {
            Flavor::Issue => {
                eprintln!(
                    "rai: no local changes or unpublished commits; cleaning up empty worktree"
                );
                cleanup_empty_worktree(&ctx.branch);
            }
            Flavor::Pr => {
                eprintln!(
                    "rai: no local changes or unpublished commits; nothing to push for PR #{}",
                    ctx.number
                );
            }
        }
        return Ok(());
    }

    eprintln!(
        "rai: delegating commit / push to finalize agent (flavor={:?}, has_local={has_local}, has_commits={has_commits})",
        ctx.flavor
    );

    let prompt = build_finalize_prompt(ctx, has_local, has_commits);
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let rai_exe = exe.display().to_string();
    let (shell_path, shell_kind) = shell::detect_user_shell();

    let engine_cmd = common::build_engine_cmd(&ctx.engine_cmd, ctx.permission_mode);
    let full_cmd =
        common::build_agent_shell_command(&engine_cmd, &prompt, &rai_exe, None, shell_kind);

    let status = shell::shell_command(&shell_path, &full_cmd)
        .status()
        .with_context(|| format!("failed to spawn finalize agent via `{shell_path} -c`"))?;
    if !status.success() {
        bail!("finalize agent exited with {:?}", status.code());
    }

    if let Flavor::Issue = ctx.flavor {
        match existing_pr_url(&ctx.branch)? {
            Some(url) => println!("rai: PR: {url}"),
            None => eprintln!(
                "rai: warning — finalize agent finished but no PR detected for branch `{}`. Inspect the worktree and finish manually.",
                ctx.branch
            ),
        }
    }

    Ok(())
}

/// finalize agent に渡すプロンプトを組み立てる。
///
/// **Precondition:** `has_local || has_commits` のいずれかは true である必要がある
/// (= publish するものがある状態でだけ呼ぶ)。`finalize_after_agent` 側で「両方 false
/// なら finalize agent を起動しない」早期リターンを行っているのでこの関数は安全に
/// 呼べる。万一その不変条件が将来崩れた場合は、release ビルドでも気付けるように
/// `unreachable!` で panic させる (= 中立文言で黙って続行すると finalize agent が
/// 何もすることがない状態で起動して時間と API クレジットを浪費する)。
fn build_finalize_prompt(ctx: &Cmd, has_local: bool, has_commits: bool) -> String {
    let state = match (has_local, has_commits) {
        (true, true) => "未コミットの変更と未 push の commit が両方残っています",
        (true, false) => "未コミットの変更が残っています",
        (false, true) => "未 push の commit が残っています",
        (false, false) => unreachable!(
            "build_finalize_prompt called with nothing to publish; \
             this is guarded by finalize_after_agent's early-return"
        ),
    };
    match ctx.flavor {
        Flavor::Issue => {
            let base_sentence = match ctx.pr_base.as_deref() {
                Some(base) => format!(" PR を作成する際は base を `{base}` にしてください。"),
                None => String::new(),
            };
            format!(
                "GitHub Issue {url} (`{title}`) の現在の作業状態を確認し、commit、push、PR の作成まで仕上げてください。\
worktree のブランチは `{branch}` で、現在 {state}。\
未コミット変更があれば論理的な単位で commit し、`git push -u origin HEAD:{branch}` で push したあと、\
リポジトリ `{repo}` に対して `gh pr create` で PR を作成してください。本文には `Closes {url}` を含めること。\
既に同じブランチに PR がある場合は新規作成せず、その URL を表示するだけで終わってください。{base_sentence} \
commit-msg hook がメッセージを弾いた場合はメッセージを直して commit し直してください。\
`--no-verify` などで hook を回避するのは禁止です。",
                url = ctx.url,
                title = ctx.title,
                branch = ctx.branch,
                state = state,
                base_sentence = base_sentence,
                repo = ctx.repo,
            )
        }
        Flavor::Pr => format!(
            "GitHub PR {url} (`{title}`) の現在の作業状態を確認し、commit、push まで仕上げてください。\
worktree のブランチは `{branch}` で、現在 {state}。\
未コミット変更があれば論理的な単位で commit し、`git push origin HEAD:{branch}` で同じ PR ブランチに push してください。\
**新規 PR は作成しないでください**。既存 PR への追加 push が前提です。\
commit-msg hook がメッセージを弾いた場合はメッセージを直して commit し直してください。\
`--no-verify` などで hook を回避するのは禁止です。",
            url = ctx.url,
            title = ctx.title,
            branch = ctx.branch,
            state = state,
        ),
    }
}

fn has_local_changes() -> Result<bool> {
    Ok(!git_capture(&["status", "--porcelain"])?.trim().is_empty())
}

fn has_publishable_commits(pr_base: Option<&str>) -> Result<bool> {
    // pr_base が明示されているケースは authoritative。`origin/{base}` または
    // `{base}` のどちらかに新しい commit があれば true。それ以外でも、`merge-base` が
    // 解決できない (= remote をまだ fetch していない / `{base}` が手元に無い等) 場合は
    // 「リモートのどのブランチからも到達できない commit が HEAD 上にあるか」で
    // 最終判定する。`merge-base` 失敗で常に false を返すと、新規ブランチで commit
    // 済みの成果物を捨ててしまう。
    if let Some(base) = pr_base {
        if has_commits_since(&format!("origin/{base}"))? {
            return Ok(true);
        }
        if has_commits_since(base)? {
            return Ok(true);
        }
        return remote_unreachable_commits();
    }

    // pr_base 未指定: upstream が設定済みならそれが答え。
    if let Ok(upstream) =
        git_capture(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
    {
        return has_commits_since(upstream.trim());
    }

    // upstream も未設定 (= まだ push されていない新規 branch) のフォールバック。
    // `origin/HEAD` が設定されていればそれを基準に未 push commit を数え、それも
    // 無ければ remote 不到達 commit を数える。
    if let Ok(origin_head) = git_capture(&[
        "rev-parse",
        "--abbrev-ref",
        "--symbolic-full-name",
        "origin/HEAD",
    ]) {
        if has_commits_since(origin_head.trim())? {
            return Ok(true);
        }
    }
    remote_unreachable_commits()
}

/// `git rev-list HEAD --not --remotes=origin --count > 0` の判定。
/// `merge-base` を必要としない最終フォールバック。
fn remote_unreachable_commits() -> Result<bool> {
    let count = git_capture(&["rev-list", "HEAD", "--not", "--remotes=origin", "--count"])?;
    Ok(count.trim().parse::<u64>().unwrap_or(0) > 0)
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

fn cleanup_empty_worktree(branch: &str) {
    let safe_cwd = std::env::temp_dir();
    // gwq remove と同じく、spawn 失敗と非ゼロ終了の両方を別個に報告する。kill 失敗は
    // worktree 削除自体を止めない (tmux session が既に消えていて 0 以外で返るケースも
    // あるため) が、ユーザーに何が起きたかは知らせる。
    match shell::user_shell_argv(&["gwq", "tmux", "kill", branch])
        .current_dir(&safe_cwd)
        .status()
    {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!(
            "rai: gwq tmux kill exited with {:?}; proceeding to gwq remove",
            s.code()
        ),
        Err(e) => eprintln!("rai: gwq tmux kill failed to spawn: {e}"),
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

pub(crate) fn local_origin_head_branch() -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(flavor: Flavor) -> Cmd {
        // URL は flavor に合わせて issues / pull を切り替える。以前は両方で
        // issues/13 を使っていたが、Pr 用ケースの期待値プロンプトに
        // `https://github.com/o/r/issues/13` が embed されてしまい、テストが
        // 「PR なのに Issue URL」という壊れた状態を fixate していた。
        let url = match flavor {
            Flavor::Issue => "https://github.com/o/r/issues/13".to_string(),
            Flavor::Pr => "https://github.com/o/r/pull/13".to_string(),
        };
        Cmd {
            flavor,
            url,
            number: 13,
            title: "T".into(),
            repo: "o/r".into(),
            branch: "feat/x".into(),
            pr_base: Some("main".into()),
            engine_cmd: common::DEFAULT_ENGINE_CMD.to_string(),
            permission_mode: None,
        }
    }

    #[test]
    fn issue_finalize_prompt_includes_pr_creation() {
        let p = build_finalize_prompt(&sample(Flavor::Issue), true, true);
        // build_finalize_prompt の出力は入力から一意。AGENTS.md Testing ガイドライン
        // に従い完全一致で検証する。
        assert_eq!(
            p,
            "GitHub Issue https://github.com/o/r/issues/13 (`T`) の現在の作業状態を確認し、commit、push、PR の作成まで仕上げてください。worktree のブランチは `feat/x` で、現在 未コミットの変更と未 push の commit が両方残っています。未コミット変更があれば論理的な単位で commit し、`git push -u origin HEAD:feat/x` で push したあと、リポジトリ `o/r` に対して `gh pr create` で PR を作成してください。本文には `Closes https://github.com/o/r/issues/13` を含めること。既に同じブランチに PR がある場合は新規作成せず、その URL を表示するだけで終わってください。 PR を作成する際は base を `main` にしてください。 commit-msg hook がメッセージを弾いた場合はメッセージを直して commit し直してください。`--no-verify` などで hook を回避するのは禁止です。"
        );
    }

    #[test]
    fn pr_finalize_prompt_forbids_new_pr_creation() {
        let p = build_finalize_prompt(&sample(Flavor::Pr), false, true);
        assert_eq!(
            p,
            "GitHub PR https://github.com/o/r/pull/13 (`T`) の現在の作業状態を確認し、commit、push まで仕上げてください。worktree のブランチは `feat/x` で、現在 未 push の commit が残っています。未コミット変更があれば論理的な単位で commit し、`git push origin HEAD:feat/x` で同じ PR ブランチに push してください。**新規 PR は作成しないでください**。既存 PR への追加 push が前提です。commit-msg hook がメッセージを弾いた場合はメッセージを直して commit し直してください。`--no-verify` などで hook を回避するのは禁止です。"
        );
    }
}
