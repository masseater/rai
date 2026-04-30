//! `rai issue inventory` — Issue 一覧を取得し、棚卸しの判定を Issue にコメント＋ラベルで焼き込む。

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{bail, Context};
use clap::Args;
use rai_core::{cli::Run, Ctx, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{gh_capture, resolve_repo};

const ISSUE_FIELDS: &str =
    "number,title,url,state,author,assignees,labels,milestone,comments,createdAt,updatedAt,body";

const COMMENT_MARKER: &str = "<!-- rai-issue-inventory -->";

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
    #[arg(long, conflicts_with = "from_verdicts")]
    print_prompt: bool,

    /// engine の生出力 (verdict JSON を含む) を保存する先。
    #[arg(long, value_name = "FILE", conflicts_with = "from_verdicts")]
    save_verdicts: Option<PathBuf>,

    /// engine を起動せず、保存済み verdict 出力ファイルから読み込む。
    #[arg(long, value_name = "FILE")]
    from_verdicts: Option<PathBuf>,

    /// 各 Issue にコメントとラベルを実際に書き込む (既定は dry-run)。
    #[arg(long)]
    apply: bool,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
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

#[derive(Debug, Deserialize, Serialize)]
struct VerdictDoc {
    verdicts: Vec<Verdict>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Verdict {
    number: u64,
    #[serde(default)]
    category: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    labels: Vec<String>,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let repo = resolve_repo(self.repo.as_deref())?;

        let engine_output = if let Some(path) = &self.from_verdicts {
            fs::read_to_string(path)
                .with_context(|| format!("failed to read --from-verdicts {}", path.display()))?
        } else {
            let issues = fetch_issues(&repo, &self)?;
            let prompt = build_prompt(&repo, &self, &issues)?;

            if self.print_prompt {
                println!("{prompt}");
                return Ok(());
            }

            let captured = run_engine_capture(&self.engine_cmd, &prompt, self.prompt_stdin)?;
            if let Some(path) = &self.save_verdicts {
                fs::write(path, &captured).with_context(|| {
                    format!("failed to write --save-verdicts {}", path.display())
                })?;
                eprintln!("verdicts saved to {}", path.display());
            }
            captured
        };

        let doc = parse_verdicts(&engine_output)?;
        if doc.verdicts.is_empty() {
            eprintln!("no verdicts produced by engine");
            return Ok(());
        }

        if !self.apply {
            print_dry_run(&doc.verdicts);
            return Ok(());
        }

        apply_verdicts(&repo, &doc.verdicts)
    }
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
        r#"GitHub Issue の棚卸しを行ってください。

制約:
- 下の JSON は `rai` が GitHub CLI で取得済みの Issue 一覧です。
- 追加で `gh issue list/view`、Web 閲覧、その他の手段で Issue を取得しないでください。
- JSON にない事実は断定せず、必要なら「追加確認が必要」と理由に明記してください。
- Issue の更新・close・label 変更は AI 側では行わないでください。`rai` が verdict JSON を読んで機械的に適用します。

最終出力 (この順番で書く):
1. 人間向けの簡潔なサマリ (Markdown)。close 候補・重複・優先度などを箇条書き。
2. 続けて、機械処理用の verdict JSON を ```json フェンスブロック 1 つで出力する。

verdict JSON の schema:
```json
{{
  "verdicts": [
    {{
      "number": 42,
      "category": "close-candidate",
      "summary": "1行の要約 (日本語可)",
      "reason": "Markdown で書く詳細理由。Issue にそのままコメントされる。",
      "labels": ["triage:close-candidate"]
    }}
  ]
}}
```

`category` の推奨値と意味:
- `close-candidate`: 既に解決済みなどでクローズしてよさそう
- `duplicate`: 別の Issue と重複 (`reason` に重複先 #番号 を明記)
- `stale`: 長期間動きなし、放棄候補
- `needs-info`: 報告者から追加情報が必要
- `keep`: 維持。継続して取り組む価値あり
- `split`: スコープが大きすぎるので分割すべき

ラベル付けの規約:
- 必ず `triage:` で始まるラベルを 1 つ以上含めること (例: `triage:close-candidate`)。
- 補助ラベル (`triage:priority-high` など) を追加しても良い。
- ラベルが存在しない場合は `rai` が自動作成する。

入力 JSON:
```json
{input}
```
"#
    ))
}

fn run_engine_capture(engine_cmd: &str, prompt: &str, prompt_stdin: bool) -> Result<String> {
    let mut argv = shell_words::split(engine_cmd).context("failed to split --engine-cmd")?;
    if argv.is_empty() {
        bail!("--engine-cmd is empty");
    }

    let program = argv.remove(0);
    let mut command = Command::new(&program);
    command.stdout(Stdio::piped());

    if prompt_stdin {
        command.args(&argv).stdin(Stdio::piped());
    } else {
        argv.push(prompt.to_string());
        command.args(&argv);
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn engine command `{program}`"))?;

    if prompt_stdin {
        let mut stdin = child.stdin.take().context("failed to open engine stdin")?;
        stdin
            .write_all(prompt.as_bytes())
            .context("failed to write prompt to engine stdin")?;
        drop(stdin);
    }

    let mut stdout_pipe = child
        .stdout
        .take()
        .context("failed to open engine stdout")?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut user_stdout = io::stdout();
    loop {
        let n = stdout_pipe
            .read(&mut chunk)
            .context("failed to read engine stdout")?;
        if n == 0 {
            break;
        }
        user_stdout
            .write_all(&chunk[..n])
            .context("failed to mirror engine stdout")?;
        user_stdout.flush().ok();
        buf.extend_from_slice(&chunk[..n]);
    }

    let status = child.wait().context("failed to wait engine command")?;
    if !status.success() {
        bail!("engine command exited with {:?}", status.code());
    }

    String::from_utf8(buf).context("engine stdout is not valid UTF-8")
}

fn parse_verdicts(engine_output: &str) -> Result<VerdictDoc> {
    let block = extract_json_block(engine_output)
        .context("failed to find ```json block in engine output")?;
    serde_json::from_str(block).with_context(|| format!("failed to parse verdict JSON: {block}"))
}

fn extract_json_block(s: &str) -> Option<&str> {
    let lower = s.to_ascii_lowercase();
    let start_idx = lower.find("```json")?;
    let after_open = &s[start_idx + "```json".len()..];
    let end_rel = after_open.find("```")?;
    Some(after_open[..end_rel].trim_matches(|c: char| c.is_whitespace() || c == '\n'))
}

fn print_dry_run(verdicts: &[Verdict]) {
    println!(
        "\n--- rai issue inventory: dry-run ({} verdicts) ---",
        verdicts.len()
    );
    for v in verdicts {
        let labels = if v.labels.is_empty() {
            "(none)".to_string()
        } else {
            v.labels.join(", ")
        };
        println!(
            "  #{:<4} [{}] labels=[{}] {}",
            v.number, v.category, labels, v.summary
        );
    }
    println!("\nRe-run with --apply to commit comments and labels to GitHub.");
    println!("After applying, mechanically close candidates with e.g.:");
    println!("  gh issue list --label triage:close-candidate --json number -q '.[].number' \\");
    println!("    | xargs -I {{}} gh issue close {{}}");
}

fn apply_verdicts(repo: &str, verdicts: &[Verdict]) -> Result<()> {
    ensure_labels_exist(repo, verdicts)?;
    for v in verdicts {
        apply_one(repo, v)?;
    }
    Ok(())
}

fn ensure_labels_exist(repo: &str, verdicts: &[Verdict]) -> Result<()> {
    let mut wanted: BTreeSet<&str> = BTreeSet::new();
    for v in verdicts {
        for l in &v.labels {
            wanted.insert(l.as_str());
        }
    }
    if wanted.is_empty() {
        return Ok(());
    }

    let existing = list_labels(repo)?;
    for label in wanted.difference(&existing.iter().map(String::as_str).collect()) {
        eprintln!("creating label `{label}` on {repo}");
        // `gh label create` exits non-zero if the label already exists; we've
        // pre-filtered, so any non-zero here is a real error.
        let status = Command::new("gh")
            .args(["label", "create", label, "--repo", repo])
            .status()
            .context("failed to spawn `gh label create`")?;
        if !status.success() {
            bail!(
                "`gh label create {label}` failed (status {:?})",
                status.code()
            );
        }
    }
    Ok(())
}

fn list_labels(repo: &str) -> Result<BTreeSet<String>> {
    let json = gh_capture(&[
        "label", "list", "--repo", repo, "--limit", "500", "--json", "name",
    ])?;
    #[derive(Deserialize)]
    struct LabelEntry {
        name: String,
    }
    let labels: Vec<LabelEntry> =
        serde_json::from_str(&json).context("failed to parse `gh label list` JSON")?;
    Ok(labels.into_iter().map(|l| l.name).collect())
}

fn apply_one(repo: &str, v: &Verdict) -> Result<()> {
    if !v.labels.is_empty() {
        let mut args: Vec<String> = vec![
            "issue".into(),
            "edit".into(),
            v.number.to_string(),
            "--repo".into(),
            repo.into(),
        ];
        for label in &v.labels {
            args.push("--add-label".into());
            args.push(label.clone());
        }
        let status = Command::new("gh")
            .args(&args)
            .status()
            .context("failed to spawn `gh issue edit`")?;
        if !status.success() {
            bail!(
                "`gh issue edit {}` failed (status {:?})",
                v.number,
                status.code()
            );
        }
    }

    let body = render_comment_body(v);
    let status = Command::new("gh")
        .args([
            "issue",
            "comment",
            &v.number.to_string(),
            "--repo",
            repo,
            "--body",
            &body,
        ])
        .status()
        .context("failed to spawn `gh issue comment`")?;
    if !status.success() {
        bail!(
            "`gh issue comment {}` failed (status {:?})",
            v.number,
            status.code()
        );
    }

    eprintln!("applied #{} ({})", v.number, v.category);
    Ok(())
}

fn render_comment_body(v: &Verdict) -> String {
    let labels = if v.labels.is_empty() {
        "(none)".to_string()
    } else {
        v.labels
            .iter()
            .map(|l| format!("`{l}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let category = if v.category.is_empty() {
        "(unspecified)"
    } else {
        v.category.as_str()
    };
    format!(
        "## rai issue inventory\n\n\
         **判定:** `{category}`\n\
         **付与ラベル:** {labels}\n\n\
         **要約:** {summary}\n\n\
         {reason}\n\n\
         ---\n\
         この投稿は `rai issue inventory` による自動判定です。実際の close / keep などの操作はメンテナが行ってください。\n\
         {marker}\n",
        category = category,
        labels = labels,
        summary = if v.summary.is_empty() { "(none)" } else { v.summary.as_str() },
        reason = if v.reason.is_empty() { "_理由の記載なし_" } else { v.reason.as_str() },
        marker = COMMENT_MARKER,
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        build_prompt, extract_json_block, parse_verdicts, render_comment_body, Cmd, IssueState,
        Verdict, COMMENT_MARKER,
    };

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
            save_verdicts: None,
            from_verdicts: None,
            apply: false,
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

        assert!(prompt.contains("追加で `gh issue list/view`"));
        assert!(prompt.contains("verdict JSON"));
        assert!(prompt.contains("\"repo\": \"owner/repo\""));
        assert!(prompt.contains("\"number\": 42"));
        assert!(prompt.contains("\"issue_count\": 1"));
        assert!(prompt.contains("triage:close-candidate"));
    }

    #[test]
    fn extract_json_block_picks_first_fenced_json() {
        let s = "summary text\n\n```json\n{\"verdicts\": []}\n```\nfooter";
        assert_eq!(extract_json_block(s), Some("{\"verdicts\": []}"));
    }

    #[test]
    fn extract_json_block_returns_none_when_missing() {
        assert_eq!(extract_json_block("no fence here"), None);
    }

    #[test]
    fn parse_verdicts_round_trips_minimal_doc() {
        let s = "intro\n```json\n{\n  \"verdicts\": [\n    {\"number\": 7, \"category\": \"close-candidate\", \"summary\": \"done\", \"labels\": [\"triage:close-candidate\"]}\n  ]\n}\n```";
        let doc = parse_verdicts(s).unwrap();
        assert_eq!(doc.verdicts.len(), 1);
        assert_eq!(doc.verdicts[0].number, 7);
        assert_eq!(doc.verdicts[0].category, "close-candidate");
        assert_eq!(doc.verdicts[0].labels, vec!["triage:close-candidate"]);
    }

    #[test]
    fn render_comment_body_includes_marker_and_fields() {
        let v = Verdict {
            number: 12,
            category: "duplicate".into(),
            summary: "dup of #5".into(),
            reason: "see #5 which already covers this".into(),
            labels: vec!["triage:duplicate".into()],
        };
        let body = render_comment_body(&v);
        assert!(body.contains("`duplicate`"));
        assert!(body.contains("dup of #5"));
        assert!(body.contains("`triage:duplicate`"));
        assert!(body.contains(COMMENT_MARKER));
    }
}
