//! `rai issue triage` — triage ラベル付き Issue を 1 件ずつ表示し、
//! 人間は close/keep の判断だけを行う。残りの操作 (close 実行、ラベル削除) は
//! 巡回完了後にまとめて適用する。

use std::io::{self, BufRead, Write};

use anyhow::{bail, Context};
use clap::{Args, ValueEnum};
use rai_core::{cli::Run, shell, Ctx, Result};
use serde::Deserialize;

use crate::{gh_capture, resolve_repo};

const ISSUE_VIEW_FIELDS: &str =
    "number,title,url,state,author,labels,createdAt,updatedAt,body,comments";

#[derive(Debug, Args)]
pub struct Cmd {
    /// `OWNER/REPO` を上書き。
    #[arg(long, value_name = "OWNER/REPO")]
    repo: Option<String>,

    /// レビュー対象の Issue が持っているべきラベル。
    #[arg(long, default_value = "triage:close-candidate")]
    label: String,

    /// close 時の `--reason`。
    #[arg(long, value_enum, default_value = "completed")]
    reason: CloseReason,

    /// close 時に投稿する共通コメント本文。
    #[arg(long, value_name = "BODY")]
    close_comment: Option<String>,

    /// keep 判断時にも triage ラベルを残す (既定では削除)。
    #[arg(long)]
    keep_label_on_keep: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CloseReason {
    Completed,
    #[value(name = "not-planned")]
    NotPlanned,
}

impl CloseReason {
    fn as_gh_arg(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::NotPlanned => "not planned",
        }
    }
}

#[derive(Deserialize)]
struct IssueRef {
    number: u64,
    title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decision {
    Close,
    Keep,
    Skip,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let repo = resolve_repo(self.repo.as_deref())?;
        let issues = list_issues(&repo, &self.label)?;

        if issues.is_empty() {
            eprintln!("no open issues with label `{}` on {repo}", self.label);
            return Ok(());
        }

        eprintln!(
            "{} issue(s) to review (label=`{}`)",
            issues.len(),
            self.label
        );

        let mut decisions: Vec<(u64, Decision)> = Vec::with_capacity(issues.len());
        for (idx, issue) in issues.iter().enumerate() {
            print_separator(idx + 1, issues.len(), issue);
            show_issue(&repo, issue.number)?;
            match prompt_decision(&mut io::stderr(), &mut io::stdin().lock())? {
                Some(d) => decisions.push((issue.number, d)),
                None => {
                    eprintln!(
                        "aborted; discarding the {} decision(s) made so far",
                        decisions.len()
                    );
                    return Ok(());
                }
            }
        }

        apply_decisions(
            &repo,
            &self.label,
            &decisions,
            self.reason,
            self.close_comment.as_deref(),
            self.keep_label_on_keep,
        )
    }
}

fn list_issues(repo: &str, label: &str) -> Result<Vec<IssueRef>> {
    let json = gh_capture(&[
        "issue",
        "list",
        "--repo",
        repo,
        "--state",
        "open",
        "--label",
        label,
        "--limit",
        "500",
        "--json",
        "number,title",
    ])?;
    serde_json::from_str(&json).context("failed to parse `gh issue list` JSON")
}

fn print_separator(idx: usize, total: usize, issue: &IssueRef) {
    eprintln!();
    eprintln!("============================================================");
    eprintln!("[{idx}/{total}] #{} — {}", issue.number, issue.title);
    eprintln!("============================================================");
}

fn show_issue(repo: &str, number: u64) -> Result<()> {
    let json = gh_capture(&[
        "issue",
        "view",
        &number.to_string(),
        "--repo",
        repo,
        "--json",
        ISSUE_VIEW_FIELDS,
    ])?;
    let view: IssueView =
        serde_json::from_str(&json).context("failed to parse `gh issue view` JSON")?;
    let mut out = io::stdout();
    render_issue(&mut out, &view).context("failed to render issue")?;
    out.flush().ok();
    Ok(())
}

#[derive(Deserialize)]
struct IssueView {
    number: u64,
    title: String,
    url: String,
    state: String,
    author: AuthorRef,
    #[serde(default)]
    labels: Vec<LabelRef>,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    body: String,
    #[serde(default)]
    comments: Vec<CommentEntry>,
}

#[derive(Deserialize)]
struct AuthorRef {
    #[serde(default)]
    login: String,
}

#[derive(Deserialize)]
struct LabelRef {
    name: String,
}

#[derive(Deserialize)]
struct CommentEntry {
    author: AuthorRef,
    #[serde(rename = "createdAt")]
    created_at: String,
    body: String,
}

fn render_issue<W: Write>(out: &mut W, view: &IssueView) -> io::Result<()> {
    let labels = if view.labels.is_empty() {
        "(none)".to_string()
    } else {
        view.labels
            .iter()
            .map(|l| l.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };
    writeln!(out, "#{} {}", view.number, view.title)?;
    writeln!(out, "{}", view.url)?;
    writeln!(
        out,
        "state={} author=@{} created={} updated={}",
        view.state, view.author.login, view.created_at, view.updated_at
    )?;
    writeln!(out, "labels: {labels}")?;
    writeln!(out, "\n--- body ---")?;
    if view.body.trim().is_empty() {
        writeln!(out, "(empty)")?;
    } else {
        writeln!(out, "{}", view.body)?;
    }
    writeln!(out, "\n--- comments ({}) ---", view.comments.len())?;
    for (i, c) in view.comments.iter().enumerate() {
        writeln!(
            out,
            "\n[{idx}] @{login} {at}",
            idx = i + 1,
            login = c.author.login,
            at = c.created_at
        )?;
        writeln!(out, "{}", c.body)?;
    }
    Ok(())
}

fn prompt_decision<W: Write, R: BufRead>(out: &mut W, input: &mut R) -> Result<Option<Decision>> {
    loop {
        write!(out, "\n[c]lose / [k]eep / [s]kip / [q]uit ? ")
            .context("failed to write triage prompt")?;
        out.flush().ok();
        let mut line = String::new();
        let n = input.read_line(&mut line).context("failed to read stdin")?;
        if n == 0 {
            return Ok(None); // EOF == quit
        }
        match line.trim().chars().next() {
            Some('c' | 'C') => return Ok(Some(Decision::Close)),
            Some('k' | 'K') => return Ok(Some(Decision::Keep)),
            Some('s' | 'S') => return Ok(Some(Decision::Skip)),
            Some('q' | 'Q') => return Ok(None),
            _ => {
                writeln!(out, "? unrecognized; type c/k/s/q").ok();
            }
        }
    }
}

fn apply_decisions(
    repo: &str,
    label: &str,
    decisions: &[(u64, Decision)],
    reason: CloseReason,
    close_comment: Option<&str>,
    keep_label_on_keep: bool,
) -> Result<()> {
    let mut closed = 0u32;
    let mut kept = 0u32;
    let mut skipped = 0u32;

    for (number, d) in decisions {
        match d {
            Decision::Close => {
                close_one(repo, *number, reason, close_comment)?;
                closed += 1;
            }
            Decision::Keep => {
                if !keep_label_on_keep {
                    remove_label(repo, *number, label)?;
                }
                kept += 1;
            }
            Decision::Skip => {
                skipped += 1;
            }
        }
    }

    eprintln!("\n--- triage applied: closed={closed} kept={kept} skipped={skipped} ---");
    Ok(())
}

fn close_one(repo: &str, number: u64, reason: CloseReason, comment: Option<&str>) -> Result<()> {
    let mut args: Vec<String> = vec![
        "issue".into(),
        "close".into(),
        number.to_string(),
        "--repo".into(),
        repo.into(),
        "--reason".into(),
        reason.as_gh_arg().into(),
    ];
    if let Some(c) = comment {
        args.push("--comment".into());
        args.push(c.to_string());
    }
    let argv: Vec<&str> = std::iter::once("gh")
        .chain(args.iter().map(String::as_str))
        .collect();
    let status = shell::user_shell_argv(&argv)
        .status()
        .context("failed to spawn `gh issue close`")?;
    if !status.success() {
        bail!(
            "`gh issue close {number}` failed (status {:?})",
            status.code()
        );
    }
    eprintln!("closed #{number}");
    Ok(())
}

fn remove_label(repo: &str, number: u64, label: &str) -> Result<()> {
    let number_str = number.to_string();
    let status = shell::user_shell_argv(&[
        "gh",
        "issue",
        "edit",
        &number_str,
        "--repo",
        repo,
        "--remove-label",
        label,
    ])
    .status()
    .context("failed to spawn `gh issue edit`")?;
    if !status.success() {
        bail!(
            "`gh issue edit {number} --remove-label {label}` failed (status {:?})",
            status.code()
        );
    }
    eprintln!("kept #{number} (label `{label}` removed)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::{prompt_decision, Decision};

    fn run(input: &str) -> (Option<Decision>, String) {
        let mut out: Vec<u8> = Vec::new();
        let mut reader = BufReader::new(Cursor::new(input.as_bytes().to_vec()));
        let result = prompt_decision(&mut out, &mut reader).unwrap();
        (result, String::from_utf8(out).unwrap())
    }

    #[test]
    fn close_input() {
        let (d, _) = run("c\n");
        assert_eq!(d, Some(Decision::Close));
    }

    #[test]
    fn keep_input_uppercase() {
        let (d, _) = run("K\n");
        assert_eq!(d, Some(Decision::Keep));
    }

    #[test]
    fn skip_input() {
        let (d, _) = run("s\n");
        assert_eq!(d, Some(Decision::Skip));
    }

    #[test]
    fn quit_returns_none() {
        let (d, _) = run("q\n");
        assert_eq!(d, None);
    }

    #[test]
    fn eof_returns_none() {
        let (d, _) = run("");
        assert_eq!(d, None);
    }

    #[test]
    fn invalid_then_valid_reprompts() {
        let (d, out) = run("???\nc\n");
        assert_eq!(d, Some(Decision::Close));
        assert!(out.contains("unrecognized"));
    }
}
