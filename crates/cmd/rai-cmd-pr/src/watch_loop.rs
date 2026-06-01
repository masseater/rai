//! `rai pr watch-loop` — PR 更新を親 watcher で集約監視し、必要時に agent を起動する。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use chrono::Local;
use clap::{Args, Subcommand};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute, queue,
    style::{Attribute, Print, SetAttribute},
    terminal::{self, ClearType},
};
use rai_core::{claude::PermissionMode, cli::Run, shell, signals, Ctx, Result};
use serde::{Deserialize, Serialize};

const STATE_VERSION: u32 = 1;
const DEFAULT_ENGINE_CMD: &str = "ccs c1 --print --output-format stream-json --verbose {PERMISSION_MODE} -- {PROMPT} | {RAI} claude format";

#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub struct Cmd {
    #[command(subcommand)]
    sub: Option<WatchLoopCmd>,
}

#[derive(Debug, Subcommand)]
enum WatchLoopCmd {
    /// watcher を起動する。既定では background daemon として起動する。
    Start(StartCmd),
    /// 稼働中 watcher を TUI で表示・停止する。
    Tui(TuiCmd),
    /// watcher 一覧を表示する。
    Status(StatusCmd),
    /// watcher を停止する。
    Stop(StopCmd),
    /// Internal daemon entrypoint.
    #[command(hide = true)]
    Daemon(DaemonCmd),
}

impl Run for Cmd {
    fn run(self, ctx: &Ctx) -> Result<()> {
        match self.sub {
            Some(WatchLoopCmd::Start(c)) => c.run(ctx),
            Some(WatchLoopCmd::Tui(c)) => c.run(ctx),
            Some(WatchLoopCmd::Status(c)) => c.run(ctx),
            Some(WatchLoopCmd::Stop(c)) => c.run(ctx),
            Some(WatchLoopCmd::Daemon(c)) => c.run(ctx),
            None => TuiCmd.run(ctx),
        }
    }
}

#[derive(Debug, Args, Clone)]
struct StartCmd {
    /// PR 識別子: URL または番号。省略時は自分の open PR を選択する。
    #[arg(value_name = "PR")]
    prs: Vec<String>,

    /// 番号 PR 用の `OWNER/REPO`。
    #[arg(long, value_name = "OWNER/REPO")]
    repo: Option<String>,

    /// polling 間隔 (秒)。
    #[arg(long, default_value_t = 60)]
    interval: u64,

    /// 初回取得時点で修正対象なら agent を起動する。
    #[arg(long)]
    trigger_initial: bool,

    /// conflict / CI failure / changes requested に限らず fingerprint 変化で起動する。
    #[arg(long)]
    on_any_update: bool,

    /// foreground で実行する。
    #[arg(long)]
    foreground: bool,

    /// agent CLI の起動コマンド。`rai develop pr` へ渡す。
    #[arg(long, short = 'e', value_name = "CMD", default_value = DEFAULT_ENGINE_CMD)]
    engine_cmd: String,

    /// agent 終了後の自動 commit / push を無効化する。
    #[arg(long)]
    no_auto_publish: bool,

    /// agent (`claude`) に渡す `--permission-mode` を明示する。
    #[arg(long, value_name = "MODE", value_enum)]
    permission_mode: Option<PermissionMode>,
}

#[derive(Debug, Args)]
struct TuiCmd;

#[derive(Debug, Args)]
struct StatusCmd;

#[derive(Debug, Args)]
struct StopCmd {
    /// watcher ID。
    id: String,
}

#[derive(Debug, Args)]
struct DaemonCmd {
    /// watcher ID。
    #[arg(long)]
    id: String,

    /// PR 識別子: URL または番号。
    #[arg(value_name = "PR", required = true)]
    prs: Vec<String>,

    #[arg(long, value_name = "OWNER/REPO")]
    repo: Option<String>,

    #[arg(long, default_value_t = 60)]
    interval: u64,

    #[arg(long)]
    trigger_initial: bool,

    #[arg(long)]
    on_any_update: bool,

    #[arg(long, short = 'e', value_name = "CMD", default_value = DEFAULT_ENGINE_CMD)]
    engine_cmd: String,

    #[arg(long)]
    no_auto_publish: bool,

    #[arg(long, value_name = "MODE", value_enum)]
    permission_mode: Option<PermissionMode>,
}

impl From<DaemonCmd> for StartCmd {
    fn from(value: DaemonCmd) -> Self {
        Self {
            prs: value.prs,
            repo: value.repo,
            interval: value.interval,
            trigger_initial: value.trigger_initial,
            on_any_update: value.on_any_update,
            foreground: true,
            engine_cmd: value.engine_cmd,
            no_auto_publish: value.no_auto_publish,
            permission_mode: value.permission_mode,
        }
    }
}

impl Run for StartCmd {
    fn run(mut self, _ctx: &Ctx) -> Result<()> {
        if self.interval == 0 {
            bail!("--interval must be greater than 0");
        }
        if self.prs.is_empty() {
            self.prs = pick_current_user_prs(self.repo.as_deref())?;
        }
        let id = make_id();
        if self.foreground {
            run_daemon(id, self)
        } else {
            spawn_daemon(&id, &self)?;
            println!("watch-loop started: {id}");
            println!("manage: rai pr watch-loop tui");
            Ok(())
        }
    }
}

impl Run for DaemonCmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let id = self.id.clone();
        run_daemon(id, self.into())
    }
}

impl Run for StatusCmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let states = load_states()?;
        if states.is_empty() {
            println!("no watch-loop daemons");
            return Ok(());
        }
        for s in states {
            let status = if pid_alive(s.pid) {
                "running"
            } else {
                "stopped"
            };
            println!(
                "{} pid={} {} interval={}s targets={} last_poll={} last_spawn={} error={}",
                s.id,
                s.pid,
                status,
                s.interval_secs,
                s.targets.len(),
                s.last_poll_at.as_deref().unwrap_or("-"),
                s.last_spawn_at.as_deref().unwrap_or("-"),
                s.last_error.as_deref().unwrap_or("-"),
            );
        }
        Ok(())
    }
}

impl Run for StopCmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        stop_by_id(&self.id)
    }
}

impl Run for TuiCmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        if !io::stdout().is_terminal() {
            bail!("TUI requires a terminal");
        }
        run_tui()
    }
}

fn spawn_daemon(id: &str, cmd: &StartCmd) -> Result<()> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let log_path = state_dir()?.join(format!("{id}.log"));
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open log file: {}", log_path.display()))?;
    let err = log
        .try_clone()
        .with_context(|| format!("failed to clone log file: {}", log_path.display()))?;

    let mut child = Command::new(exe);
    child
        .args(["pr", "watch-loop", "daemon", "--id", id])
        .args(&cmd.prs)
        .arg("--interval")
        .arg(cmd.interval.to_string())
        .arg("--engine-cmd")
        .arg(&cmd.engine_cmd)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .stdin(Stdio::null());
    if let Some(repo) = &cmd.repo {
        child.args(["--repo", repo]);
    }
    if cmd.trigger_initial {
        child.arg("--trigger-initial");
    }
    if cmd.on_any_update {
        child.arg("--on-any-update");
    }
    if cmd.no_auto_publish {
        child.arg("--no-auto-publish");
    }
    if let Some(mode) = cmd.permission_mode {
        child.args(["--permission-mode", mode.as_arg()]);
    }
    child.spawn().context("failed to spawn watch-loop daemon")?;
    Ok(())
}

fn run_daemon(id: String, cmd: StartCmd) -> Result<()> {
    let targets = resolve_targets(&cmd.prs, cmd.repo.as_deref())?;
    let mut state = WatchState::new(id, std::process::id(), &cmd, targets);
    save_state(&state)?;
    let signal_slot = signals::install()?;

    loop {
        if signal_slot.load(Ordering::SeqCst) != 0 {
            state.stopping = true;
            state.last_error = None;
            save_state(&state)?;
            return Ok(());
        }

        match poll_once(&mut state, &cmd) {
            Ok(()) => state.last_error = None,
            Err(e) => state.last_error = Some(e.to_string()),
        }
        state.last_poll_at = Some(now());
        save_state(&state)?;

        for _ in 0..cmd.interval {
            if signal_slot.load(Ordering::SeqCst) != 0 {
                break;
            }
            thread::sleep(Duration::from_secs(1));
        }
    }
}

fn poll_once(state: &mut WatchState, cmd: &StartCmd) -> Result<()> {
    let snapshots = fetch_snapshots(&state.targets)?;
    for snapshot in snapshots {
        let Some(target) = state.targets.iter_mut().find(|t| {
            t.owner == snapshot.owner && t.repo == snapshot.repo && t.number == snapshot.number
        }) else {
            continue;
        };

        let fingerprint = snapshot.fingerprint();
        let first_seen = target.last_fingerprint.is_none();
        let changed = target.last_fingerprint.as_deref() != Some(fingerprint.as_str());
        target.title = Some(snapshot.title.clone());
        target.url = snapshot.url.clone();
        target.last_actionable = snapshot.actionable_reason();

        if changed {
            target.last_seen_at = Some(now());
        }
        let should_trigger = changed
            && (!first_seen || cmd.trigger_initial)
            && (cmd.on_any_update || target.last_actionable.is_some());
        target.last_fingerprint = Some(fingerprint);

        if should_trigger {
            spawn_develop_pr(target, cmd)?;
            let ts = now();
            target.last_spawn_at = Some(ts.clone());
            state.last_spawn_at = Some(ts);
        }
    }
    Ok(())
}

fn spawn_develop_pr(target: &TargetState, cmd: &StartCmd) -> Result<()> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let pr = target.url.clone().unwrap_or_else(|| {
        format!(
            "https://github.com/{}/{}/pull/{}",
            target.owner, target.repo, target.number
        )
    });
    let mut child = Command::new(exe);
    child
        .args(["develop", "pr", &pr])
        .arg("--engine-cmd")
        .arg(&cmd.engine_cmd)
        .stdin(Stdio::null());
    if cmd.no_auto_publish {
        child.arg("--no-auto-publish");
    }
    if let Some(mode) = cmd.permission_mode {
        child.args(["--permission-mode", mode.as_arg()]);
    }
    child
        .spawn()
        .with_context(|| format!("failed to spawn `rai develop pr {pr}`"))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WatchState {
    version: u32,
    id: String,
    pid: u32,
    started_at: String,
    interval_secs: u64,
    trigger_initial: bool,
    on_any_update: bool,
    stopping: bool,
    last_poll_at: Option<String>,
    last_spawn_at: Option<String>,
    last_error: Option<String>,
    targets: Vec<TargetState>,
}

impl WatchState {
    fn new(id: String, pid: u32, cmd: &StartCmd, targets: Vec<Target>) -> Self {
        Self {
            version: STATE_VERSION,
            id,
            pid,
            started_at: now(),
            interval_secs: cmd.interval,
            trigger_initial: cmd.trigger_initial,
            on_any_update: cmd.on_any_update,
            stopping: false,
            last_poll_at: None,
            last_spawn_at: None,
            last_error: None,
            targets: targets.into_iter().map(TargetState::from).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TargetState {
    owner: String,
    repo: String,
    number: u64,
    url: Option<String>,
    title: Option<String>,
    last_fingerprint: Option<String>,
    last_seen_at: Option<String>,
    last_spawn_at: Option<String>,
    last_actionable: Option<String>,
}

impl From<Target> for TargetState {
    fn from(value: Target) -> Self {
        Self {
            owner: value.owner,
            repo: value.repo,
            number: value.number,
            url: None,
            title: None,
            last_fingerprint: None,
            last_seen_at: None,
            last_spawn_at: None,
            last_actionable: None,
        }
    }
}

impl TargetState {
    fn repo_label(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Target {
    owner: String,
    repo: String,
    number: u64,
}

fn resolve_targets(args: &[String], repo_override: Option<&str>) -> Result<Vec<Target>> {
    let mut out = Vec::with_capacity(args.len());
    let mut repo_cache: Option<(String, String)> = None;
    for arg in args {
        if let Some((owner, repo, number)) = parse_pr_url(arg) {
            out.push(Target {
                owner,
                repo,
                number,
            });
            continue;
        }
        let number = arg
            .parse::<u64>()
            .with_context(|| format!("invalid PR identifier: {arg}"))?;
        let (owner, repo) = match &repo_cache {
            Some(v) => v.clone(),
            None => {
                let v = resolve_repo(repo_override)?;
                repo_cache = Some(v.clone());
                v
            }
        };
        out.push(Target {
            owner,
            repo,
            number,
        });
    }
    out.sort();
    out.dedup();
    Ok(out)
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
    let number: u64 = it.next()?.parse().ok()?;
    Some((owner, repo, number))
}

fn resolve_repo(repo_override: Option<&str>) -> Result<(String, String)> {
    if let Some(s) = repo_override {
        let (o, r) = s
            .split_once('/')
            .ok_or_else(|| anyhow!("--repo must be OWNER/REPO"))?;
        return Ok((o.to_string(), r.to_string()));
    }
    let json = gh_capture(&["repo", "view", "--json", "owner,name"])?;
    #[derive(Deserialize)]
    struct V {
        owner: Owner,
        name: String,
    }
    #[derive(Deserialize)]
    struct Owner {
        login: String,
    }
    let v: V = serde_json::from_str(&json).context("failed to parse `gh repo view` JSON")?;
    Ok((v.owner.login, v.name))
}

fn resolve_repo_for_picker(repo_override: Option<&str>) -> Result<(String, String)> {
    match resolve_repo(repo_override) {
        Ok(repo) => Ok(repo),
        Err(e) if repo_override.is_none() => prompt_repo().with_context(|| {
            format!(
                "failed to resolve current git repository via `gh repo view`: {e}. \
                 Enter OWNER/REPO or pass --repo."
            )
        }),
        Err(e) => Err(e),
    }
}

fn prompt_repo() -> Result<(String, String)> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        bail!("not in a GitHub git repository; pass --repo OWNER/REPO");
    }
    eprint!("OWNER/REPO: ");
    io::stderr().flush().ok();
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .context("failed to read OWNER/REPO")?;
    parse_owner_repo(line.trim())
}

fn parse_owner_repo(s: &str) -> Result<(String, String)> {
    let (owner, repo) = s
        .split_once('/')
        .ok_or_else(|| anyhow!("repository must be OWNER/REPO"))?;
    if owner.is_empty() || repo.is_empty() {
        bail!("repository must be OWNER/REPO");
    }
    Ok((owner.to_string(), repo.to_string()))
}

fn pick_current_user_prs(repo_override: Option<&str>) -> Result<Vec<String>> {
    let (owner, repo) = resolve_repo_for_picker(repo_override)?;
    let login = gh_login()?;
    let prs = list_current_user_prs(&owner, &repo, &login)?;
    if prs.is_empty() {
        bail!("no open PRs for @{login} in {owner}/{repo}");
    }
    let selected = pick_prs_with_fzf(&prs)?;
    Ok(selected.into_iter().map(|pr| pr.url).collect())
}

fn gh_login() -> Result<String> {
    let out = gh_capture(&["api", "user", "--jq", ".login"])
        .context("failed to resolve logged-in GitHub user; run `gh auth login`")?;
    let login = out.trim();
    if login.is_empty() {
        bail!("`gh api user --jq .login` returned an empty login");
    }
    Ok(login.to_string())
}

#[derive(Debug, Clone)]
struct PickablePr {
    number: u64,
    title: String,
    url: String,
    head_ref: String,
    updated_at: String,
}

fn list_current_user_prs(owner: &str, repo: &str, login: &str) -> Result<Vec<PickablePr>> {
    let json = gh_capture(&[
        "pr",
        "list",
        "--repo",
        &format!("{owner}/{repo}"),
        "--author",
        login,
        "--state",
        "open",
        "--limit",
        "100",
        "--json",
        "number,title,url,headRefName,updatedAt",
    ])?;
    #[derive(Deserialize)]
    struct Item {
        number: u64,
        title: String,
        url: String,
        #[serde(rename = "headRefName")]
        head_ref_name: String,
        #[serde(rename = "updatedAt")]
        updated_at: String,
    }
    let items: Vec<Item> =
        serde_json::from_str(&json).context("failed to parse `gh pr list` JSON")?;
    Ok(items
        .into_iter()
        .map(|it| PickablePr {
            number: it.number,
            title: it.title,
            url: it.url,
            head_ref: it.head_ref_name,
            updated_at: it.updated_at,
        })
        .collect())
}

fn pick_prs_with_fzf(prs: &[PickablePr]) -> Result<Vec<PickablePr>> {
    let mut fzf =
        shell::user_shell_argv(&["fzf", "--multi", "--with-nth=1,2,3,4", "--delimiter=\t"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("failed to spawn `fzf`")?;
    {
        let mut stdin = fzf.stdin.take().ok_or_else(|| anyhow!("fzf stdin"))?;
        for pr in prs {
            writeln!(
                stdin,
                "#{}\t{}\t{}\t{}\t{}",
                pr.number, pr.title, pr.head_ref, pr.updated_at, pr.url
            )
            .ok();
        }
    }
    let out = fzf.wait_with_output()?;
    if !out.status.success() {
        bail!("user cancelled PR selection");
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut selected = Vec::new();
    for line in stdout.lines() {
        let Some(url) = line.split('\t').next_back() else {
            continue;
        };
        if let Some(pr) = prs.iter().find(|pr| pr.url == url) {
            selected.push(pr.clone());
        }
    }
    if selected.is_empty() {
        bail!("user cancelled PR selection");
    }
    Ok(selected)
}

#[derive(Debug)]
struct PrSnapshot {
    owner: String,
    repo: String,
    number: u64,
    title: String,
    url: Option<String>,
    updated_at: Option<String>,
    head_ref_oid: Option<String>,
    mergeable: Option<String>,
    review_decision: Option<String>,
    checks: Vec<CheckSnapshot>,
}

impl PrSnapshot {
    fn fingerprint(&self) -> String {
        let mut checks: Vec<String> = self
            .checks
            .iter()
            .map(|c| {
                format!(
                    "{}:{}:{}",
                    c.name,
                    c.status_or_state,
                    c.conclusion.clone().unwrap_or_default()
                )
            })
            .collect();
        checks.sort();
        format!(
            "updated={}|head={}|mergeable={}|review={}|checks={}",
            self.updated_at.as_deref().unwrap_or(""),
            self.head_ref_oid.as_deref().unwrap_or(""),
            self.mergeable.as_deref().unwrap_or(""),
            self.review_decision.as_deref().unwrap_or(""),
            checks.join(",")
        )
    }

    fn actionable_reason(&self) -> Option<String> {
        if self
            .mergeable
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("CONFLICTING"))
        {
            return Some("conflicting".to_string());
        }
        if self
            .review_decision
            .as_deref()
            .is_some_and(|d| d.eq_ignore_ascii_case("CHANGES_REQUESTED"))
        {
            return Some("changes requested".to_string());
        }
        let failed = self.checks.iter().filter(|c| c.is_failed()).count();
        if failed > 0 {
            return Some(format!("{failed} failed check(s)"));
        }
        None
    }
}

#[derive(Debug, Clone)]
struct CheckSnapshot {
    name: String,
    status_or_state: String,
    conclusion: Option<String>,
}

impl CheckSnapshot {
    fn is_failed(&self) -> bool {
        let value = self
            .conclusion
            .as_deref()
            .unwrap_or(&self.status_or_state)
            .to_ascii_uppercase();
        matches!(
            value.as_str(),
            "FAILURE"
                | "ERROR"
                | "TIMED_OUT"
                | "CANCELLED"
                | "ACTION_REQUIRED"
                | "STARTUP_FAILURE"
                | "STALE"
        )
    }
}

fn fetch_snapshots(targets: &[TargetState]) -> Result<Vec<PrSnapshot>> {
    let mut by_repo: BTreeMap<(String, String), Vec<u64>> = BTreeMap::new();
    for t in targets {
        by_repo
            .entry((t.owner.clone(), t.repo.clone()))
            .or_default()
            .push(t.number);
    }

    let mut out = Vec::new();
    for ((owner, repo), mut numbers) in by_repo {
        numbers.sort_unstable();
        numbers.dedup();
        out.extend(fetch_repo_snapshots(&owner, &repo, &numbers)?);
    }
    Ok(out)
}

fn fetch_repo_snapshots(owner: &str, repo: &str, numbers: &[u64]) -> Result<Vec<PrSnapshot>> {
    let query = build_graphql_query(numbers);
    let json = gh_capture(&[
        "api",
        "graphql",
        "-f",
        &format!("owner={owner}"),
        "-f",
        &format!("repo={repo}"),
        "-f",
        &format!("query={query}"),
    ])?;
    let v: serde_json::Value =
        serde_json::from_str(&json).context("failed to parse `gh api graphql` JSON")?;
    let repository = v
        .get("data")
        .and_then(|d| d.get("repository"))
        .ok_or_else(|| anyhow!("GraphQL response did not include repository"))?;

    let mut out = Vec::new();
    for n in numbers {
        let key = format!("pr{n}");
        let Some(pr) = repository.get(&key) else {
            continue;
        };
        if pr.is_null() {
            continue;
        }
        out.push(parse_snapshot(owner, repo, *n, pr)?);
    }
    Ok(out)
}

fn build_graphql_query(numbers: &[u64]) -> String {
    let mut fields = String::new();
    for n in numbers {
        fields.push_str(&format!(
            r#"
    pr{n}: pullRequest(number: {n}) {{
      number
      title
      url
      updatedAt
      headRefOid
      mergeable
      reviewDecision
      statusCheckRollup {{
        contexts(first: 100) {{
          nodes {{
            __typename
            ... on CheckRun {{
              name
              status
              conclusion
            }}
            ... on StatusContext {{
              context
              state
            }}
          }}
        }}
      }}
    }}
"#
        ));
    }
    format!(
        r#"
query($owner: String!, $repo: String!) {{
  repository(owner: $owner, name: $repo) {{{fields}
  }}
}}
"#
    )
}

fn parse_snapshot(
    owner: &str,
    repo: &str,
    number: u64,
    pr: &serde_json::Value,
) -> Result<PrSnapshot> {
    let title = pr
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>")
        .to_string();
    let checks = pr
        .get("statusCheckRollup")
        .and_then(|v| v.get("contexts"))
        .and_then(|v| v.get("nodes"))
        .and_then(|v| v.as_array())
        .map(|nodes| nodes.iter().filter_map(parse_check).collect())
        .unwrap_or_default();
    Ok(PrSnapshot {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
        title,
        url: pr.get("url").and_then(|v| v.as_str()).map(str::to_string),
        updated_at: pr
            .get("updatedAt")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        head_ref_oid: pr
            .get("headRefOid")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        mergeable: pr
            .get("mergeable")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        review_decision: pr
            .get("reviewDecision")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        checks,
    })
}

fn parse_check(v: &serde_json::Value) -> Option<CheckSnapshot> {
    let ty = v.get("__typename")?.as_str()?;
    match ty {
        "CheckRun" => Some(CheckSnapshot {
            name: v.get("name")?.as_str()?.to_string(),
            status_or_state: v.get("status")?.as_str()?.to_string(),
            conclusion: v
                .get("conclusion")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }),
        "StatusContext" => Some(CheckSnapshot {
            name: v.get("context")?.as_str()?.to_string(),
            status_or_state: v.get("state")?.as_str()?.to_string(),
            conclusion: None,
        }),
        _ => None,
    }
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

fn apply_repo_filter(states: &[WatchState], repo_filter: Option<&str>) -> Vec<WatchState> {
    let Some(repo_filter) = repo_filter else {
        return states.to_vec();
    };
    states
        .iter()
        .filter_map(|state| {
            let mut filtered = state.clone();
            filtered
                .targets
                .retain(|target| target.repo_label() == repo_filter);
            if filtered.targets.is_empty() {
                None
            } else {
                Some(filtered)
            }
        })
        .collect()
}

fn next_repo_filter(states: &[WatchState], current: Option<&str>) -> Option<String> {
    let repos = repo_filters(states);
    if repos.is_empty() {
        return None;
    }
    match current {
        None => repos.first().cloned(),
        Some(cur) => {
            let idx = repos.iter().position(|repo| repo == cur)?;
            repos.get(idx + 1).cloned()
        }
    }
}

fn repo_filters(states: &[WatchState]) -> Vec<String> {
    let mut repos: Vec<String> = states
        .iter()
        .flat_map(|state| state.targets.iter().map(TargetState::repo_label))
        .collect();
    repos.sort();
    repos.dedup();
    repos
}

fn run_tui() -> Result<()> {
    let mut term = TermGuard::enter()?;
    let mut app = TuiApp::default();
    loop {
        let states = load_states()?;
        app.clamp_selection(&states);
        draw_tui(&mut term.stdout, &states, &app)?;
        if event::poll(Duration::from_millis(1000))? {
            match event::read()? {
                Event::Key(key) => {
                    if handle_tui_key(&mut app, &states, key.code)? {
                        return Ok(());
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
}

#[derive(Debug, Default)]
struct TuiApp {
    mode: TuiMode,
    selected: usize,
    repo_filter: Option<String>,
    message: Option<String>,
}

impl TuiApp {
    fn clamp_selection(&mut self, states: &[WatchState]) {
        match &self.mode {
            TuiMode::Dashboard => {
                let visible_len = apply_repo_filter(states, self.repo_filter.as_deref()).len();
                if self.selected >= visible_len {
                    self.selected = visible_len.saturating_sub(1);
                }
            }
            TuiMode::PrSelect { prs, selected, .. } => {
                if *selected >= prs.len() {
                    self.mode.set_pr_selected(prs.len().saturating_sub(1));
                }
            }
            TuiMode::RepoInput { .. } => {}
        }
    }
}

#[derive(Debug, Default)]
enum TuiMode {
    #[default]
    Dashboard,
    RepoInput {
        input: String,
        message: String,
    },
    PrSelect {
        repo: String,
        prs: Vec<PickablePr>,
        selected: usize,
        picked: BTreeSet<usize>,
        message: Option<String>,
    },
}

impl TuiMode {
    fn set_pr_selected(&mut self, next: usize) {
        if let Self::PrSelect { selected, .. } = self {
            *selected = next;
        }
    }
}

fn handle_tui_key(app: &mut TuiApp, states: &[WatchState], code: KeyCode) -> Result<bool> {
    let mode = std::mem::take(&mut app.mode);
    match mode {
        TuiMode::Dashboard => {
            app.mode = TuiMode::Dashboard;
            handle_dashboard_key(app, states, code)
        }
        TuiMode::RepoInput { mut input, message } => {
            match code {
                KeyCode::Esc => {
                    app.mode = TuiMode::Dashboard;
                    app.message = Some("start cancelled".to_string());
                }
                KeyCode::Enter => match parse_owner_repo(input.trim()) {
                    Ok((owner, repo)) => load_pr_picker(app, owner, repo),
                    Err(e) => {
                        app.mode = TuiMode::RepoInput {
                            input,
                            message: e.to_string(),
                        };
                    }
                },
                KeyCode::Backspace => {
                    input.pop();
                    app.mode = TuiMode::RepoInput { input, message };
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    app.mode = TuiMode::RepoInput { input, message };
                }
                _ => {
                    app.mode = TuiMode::RepoInput { input, message };
                }
            }
            Ok(false)
        }
        TuiMode::PrSelect {
            repo,
            prs,
            mut selected,
            mut picked,
            mut message,
        } => {
            let mut quit = false;
            match code {
                KeyCode::Esc => {
                    app.mode = TuiMode::Dashboard;
                    app.message = Some("start cancelled".to_string());
                    return Ok(false);
                }
                KeyCode::Char('q') => quit = true,
                KeyCode::Char('j') | KeyCode::Down => {
                    selected = (selected + 1).min(prs.len().saturating_sub(1));
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Char(' ') => {
                    if picked.contains(&selected) {
                        picked.remove(&selected);
                    } else {
                        picked.insert(selected);
                    }
                }
                KeyCode::Enter => {
                    let chosen: Vec<String> = picked
                        .iter()
                        .filter_map(|idx| prs.get(*idx))
                        .map(|pr| pr.url.clone())
                        .collect();
                    if chosen.is_empty() {
                        message = Some("select at least one PR with Space".to_string());
                    } else {
                        match start_watch_for_prs(chosen) {
                            Ok(()) => {
                                app.mode = TuiMode::Dashboard;
                                app.repo_filter = Some(repo);
                                app.message = Some("watcher started".to_string());
                                return Ok(false);
                            }
                            Err(e) => message = Some(format!("start failed: {e:#}")),
                        }
                    }
                }
                _ => {}
            }
            app.mode = TuiMode::PrSelect {
                repo,
                prs,
                selected,
                picked,
                message,
            };
            Ok(quit)
        }
    }
}

fn handle_dashboard_key(app: &mut TuiApp, states: &[WatchState], code: KeyCode) -> Result<bool> {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        KeyCode::Char('s') => open_start_picker(app),
        KeyCode::Char('r') => {
            app.repo_filter = next_repo_filter(states, app.repo_filter.as_deref());
            app.selected = 0;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let visible_len = apply_repo_filter(states, app.repo_filter.as_deref()).len();
            app.selected = (app.selected + 1).min(visible_len.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.selected = app.selected.saturating_sub(1);
        }
        KeyCode::Char('x') | KeyCode::Delete => {
            let visible = apply_repo_filter(states, app.repo_filter.as_deref());
            if let Some(s) = visible.get(app.selected) {
                match stop_by_id_silent(&s.id) {
                    Ok(()) => app.message = Some(format!("stopped {}", s.id)),
                    Err(e) => app.message = Some(format!("stop failed: {e:#}")),
                }
            }
        }
        _ => {}
    }
    Ok(false)
}

fn open_start_picker(app: &mut TuiApp) {
    match resolve_repo_for_picker(None) {
        Ok((owner, repo)) => load_pr_picker(app, owner, repo),
        Err(e) => {
            app.mode = TuiMode::RepoInput {
                input: String::new(),
                message: format!("repo not detected: {e}. enter OWNER/REPO"),
            };
        }
    }
}

fn load_pr_picker(app: &mut TuiApp, owner: String, repo: String) {
    match gh_login().and_then(|login| list_current_user_prs(&owner, &repo, &login)) {
        Ok(prs) if prs.is_empty() => {
            app.mode = TuiMode::Dashboard;
            app.message = Some(format!("no open PRs for logged-in user in {owner}/{repo}"));
        }
        Ok(prs) => {
            app.mode = TuiMode::PrSelect {
                repo: format!("{owner}/{repo}"),
                prs,
                selected: 0,
                picked: BTreeSet::new(),
                message: None,
            };
        }
        Err(e) => {
            app.mode = TuiMode::Dashboard;
            app.message = Some(format!("failed to load PRs: {e:#}"));
        }
    }
}

fn start_watch_for_prs(prs: Vec<String>) -> Result<()> {
    let cmd = StartCmd {
        prs,
        repo: None,
        interval: 60,
        trigger_initial: false,
        on_any_update: false,
        foreground: false,
        engine_cmd: DEFAULT_ENGINE_CMD.to_string(),
        no_auto_publish: false,
        permission_mode: None,
    };
    let id = make_id();
    spawn_daemon(&id, &cmd)
}

struct TermGuard {
    stdout: io::Stdout,
}

impl TermGuard {
    fn enter() -> Result<Self> {
        let mut stdout = io::stdout();
        terminal::enable_raw_mode()?;
        execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(Self { stdout })
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn draw_tui(stdout: &mut io::Stdout, states: &[WatchState], app: &TuiApp) -> Result<()> {
    let (cols, rows) = terminal::size()?;
    match &app.mode {
        TuiMode::Dashboard => draw_dashboard(stdout, states, app, cols, rows),
        TuiMode::RepoInput { input, message } => draw_repo_input(stdout, input, message, cols),
        TuiMode::PrSelect {
            repo,
            prs,
            selected,
            picked,
            message,
        } => draw_pr_select(
            stdout,
            repo,
            prs,
            *selected,
            picked,
            message.as_deref(),
            (cols, rows),
        ),
    }
}

fn draw_header(stdout: &mut io::Stdout) -> Result<()> {
    queue!(
        stdout,
        cursor::MoveTo(0, 0),
        terminal::Clear(ClearType::All),
        SetAttribute(Attribute::Bold),
        Print("rai pr watch-loop"),
        SetAttribute(Attribute::Reset)
    )?;
    Ok(())
}

fn draw_dashboard(
    stdout: &mut io::Stdout,
    states: &[WatchState],
    app: &TuiApp,
    cols: u16,
    rows: u16,
) -> Result<()> {
    draw_header(stdout)?;
    let visible = apply_repo_filter(states, app.repo_filter.as_deref());
    let filter_label = app.repo_filter.as_deref().unwrap_or("all");
    queue!(
        stdout,
        cursor::MoveTo(0, 1),
        Print(fit(
            &format!(
                "filter={filter_label}  s: add PR watcher  r: repo filter  x: stop watcher  j/k: select  q: quit"
            ),
            cols
        ))
    )?;
    if let Some(message) = &app.message {
        queue!(stdout, cursor::MoveTo(0, 2), Print(fit(message, cols)))?;
    }
    if visible.is_empty() {
        queue!(stdout, cursor::MoveTo(0, 3), Print("no watch-loop daemons"))?;
        stdout.flush()?;
        return Ok(());
    }

    let mut row = 3u16;
    for (idx, state) in visible.iter().enumerate() {
        if row >= rows {
            break;
        }
        let marker = if idx == app.selected { "> " } else { "  " };
        let status = if pid_alive(state.pid) {
            "running"
        } else {
            "stopped"
        };
        if idx == app.selected {
            queue!(stdout, SetAttribute(Attribute::Reverse))?;
        }
        queue!(
            stdout,
            cursor::MoveTo(0, row),
            Print(fit(
                &format!(
                    "{marker}{} pid={} {} every={}s targets={} last_poll={} last_spawn={} err={}",
                    state.id,
                    state.pid,
                    status,
                    state.interval_secs,
                    state.targets.len(),
                    state.last_poll_at.as_deref().unwrap_or("-"),
                    state.last_spawn_at.as_deref().unwrap_or("-"),
                    state.last_error.as_deref().unwrap_or("-")
                ),
                cols
            )),
            SetAttribute(Attribute::Reset)
        )?;
        row += 1;
        for target in &state.targets {
            if row >= rows {
                break;
            }
            queue!(
                stdout,
                cursor::MoveTo(2, row),
                Print(fit(
                    &format!(
                        "#{} {}/{} {} action={} seen={} spawn={}",
                        target.number,
                        target.owner,
                        target.repo,
                        target.title.as_deref().unwrap_or(""),
                        target.last_actionable.as_deref().unwrap_or("-"),
                        target.last_seen_at.as_deref().unwrap_or("-"),
                        target.last_spawn_at.as_deref().unwrap_or("-")
                    ),
                    cols.saturating_sub(2)
                ))
            )?;
            row += 1;
        }
    }
    stdout.flush()?;
    Ok(())
}

fn draw_repo_input(stdout: &mut io::Stdout, input: &str, message: &str, cols: u16) -> Result<()> {
    draw_header(stdout)?;
    queue!(
        stdout,
        cursor::MoveTo(0, 1),
        Print("add PR watcher: enter OWNER/REPO, Enter: load PRs, Esc: cancel"),
        cursor::MoveTo(0, 3),
        Print(fit(message, cols)),
        cursor::MoveTo(0, 5),
        Print(fit(&format!("OWNER/REPO: {input}"), cols))
    )?;
    stdout.flush()?;
    Ok(())
}

fn draw_pr_select(
    stdout: &mut io::Stdout,
    repo: &str,
    prs: &[PickablePr],
    selected: usize,
    picked: &BTreeSet<usize>,
    message: Option<&str>,
    size: (u16, u16),
) -> Result<()> {
    let (cols, rows) = size;
    draw_header(stdout)?;
    queue!(
        stdout,
        cursor::MoveTo(0, 1),
        Print(fit(
            &format!("{repo}  Space: select PR  Enter: start watcher  Esc: back  q: quit"),
            cols
        ))
    )?;
    if let Some(message) = message {
        queue!(stdout, cursor::MoveTo(0, 2), Print(fit(message, cols)))?;
    }
    if prs.is_empty() {
        queue!(stdout, cursor::MoveTo(0, 4), Print("no open PRs"))?;
        stdout.flush()?;
        return Ok(());
    }
    let mut row = 4u16;
    for (idx, pr) in prs.iter().enumerate() {
        if row >= rows {
            break;
        }
        let cursor = if idx == selected { "> " } else { "  " };
        let mark = if picked.contains(&idx) { "[x]" } else { "[ ]" };
        if idx == selected {
            queue!(stdout, SetAttribute(Attribute::Reverse))?;
        }
        queue!(
            stdout,
            cursor::MoveTo(0, row),
            Print(fit(
                &format!(
                    "{cursor}{mark} #{} {} {} updated={}",
                    pr.number, pr.title, pr.head_ref, pr.updated_at
                ),
                cols
            )),
            SetAttribute(Attribute::Reset)
        )?;
        row += 1;
    }
    stdout.flush()?;
    Ok(())
}

fn fit(s: &str, cols: u16) -> String {
    let max = cols as usize;
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn stop_by_id(id: &str) -> Result<()> {
    stop_by_id_silent(id)?;
    println!("stopped: {id}");
    Ok(())
}

fn stop_by_id_silent(id: &str) -> Result<()> {
    let path = state_path(id)?;
    let body = fs::read_to_string(&path)
        .with_context(|| format!("failed to read state file: {}", path.display()))?;
    let mut state: WatchState =
        serde_json::from_str(&body).context("failed to parse watch-loop state")?;
    if pid_alive(state.pid) {
        let pid = state.pid.to_string();
        shell::user_shell_argv(&["kill", &pid])
            .status()
            .with_context(|| format!("failed to spawn `kill {pid}`"))?;
    }
    state.stopping = true;
    save_state(&state)?;
    Ok(())
}

fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    shell::user_shell_argv(&["kill", "-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn save_state(state: &WatchState) -> Result<()> {
    let path = state_path(&state.id)?;
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(state)?;
    fs::write(&tmp, body).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("failed to rename {}", path.display()))?;
    Ok(())
}

fn load_states() -> Result<Vec<WatchState>> {
    let dir = state_dir()?;
    let mut states = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let body = fs::read_to_string(&path)
            .with_context(|| format!("failed to read state file: {}", path.display()))?;
        match serde_json::from_str::<WatchState>(&body) {
            Ok(s) => states.push(s),
            Err(e) => eprintln!("rai: warning: skip invalid state {}: {e}", path.display()),
        }
    }
    states.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(states)
}

fn state_path(id: &str) -> Result<PathBuf> {
    Ok(state_dir()?.join(format!("{id}.json")))
}

fn state_dir() -> Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg)
    } else {
        let home = std::env::var("HOME").context("HOME is not set")?;
        PathBuf::from(home).join(".local/state")
    };
    let dir = base.join("rai/pr-watch-loop");
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    Ok(dir)
}

fn make_id() -> String {
    format!(
        "{}-{}",
        Local::now().format("%Y%m%d-%H%M%S"),
        std::process::id()
    )
}

fn now() -> String {
    Local::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pr_url_extracts_owner_repo_number() {
        assert_eq!(
            parse_pr_url("https://github.com/o/r/pull/42"),
            Some(("o".to_string(), "r".to_string(), 42))
        );
        assert_eq!(parse_pr_url("https://github.com/o/r/issues/42"), None);
    }

    #[test]
    fn resolve_targets_dedups() {
        let got = resolve_targets(
            &[
                "https://github.com/o/r/pull/2".to_string(),
                "https://github.com/o/r/pull/2".to_string(),
            ],
            None,
        )
        .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].number, 2);
    }

    #[test]
    fn parse_owner_repo_requires_both_parts() {
        assert_eq!(
            parse_owner_repo("owner/repo").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
        assert!(parse_owner_repo("owner").is_err());
        assert!(parse_owner_repo("owner/").is_err());
        assert!(parse_owner_repo("/repo").is_err());
    }

    #[test]
    fn fingerprint_sorts_checks() {
        let a = PrSnapshot {
            owner: "o".into(),
            repo: "r".into(),
            number: 1,
            title: "t".into(),
            url: None,
            updated_at: Some("u".into()),
            head_ref_oid: Some("h".into()),
            mergeable: Some("MERGEABLE".into()),
            review_decision: None,
            checks: vec![
                CheckSnapshot {
                    name: "b".into(),
                    status_or_state: "SUCCESS".into(),
                    conclusion: None,
                },
                CheckSnapshot {
                    name: "a".into(),
                    status_or_state: "FAILURE".into(),
                    conclusion: None,
                },
            ],
        };
        let f1 = a.fingerprint();
        let b = PrSnapshot {
            checks: vec![a.checks[1].clone(), a.checks[0].clone()],
            ..a
        };
        assert_eq!(f1, b.fingerprint());
    }

    #[test]
    fn actionable_detects_failed_check() {
        let snap = PrSnapshot {
            owner: "o".into(),
            repo: "r".into(),
            number: 1,
            title: "t".into(),
            url: None,
            updated_at: None,
            head_ref_oid: None,
            mergeable: Some("MERGEABLE".into()),
            review_decision: None,
            checks: vec![CheckSnapshot {
                name: "ci".into(),
                status_or_state: "COMPLETED".into(),
                conclusion: Some("FAILURE".into()),
            }],
        };
        assert_eq!(snap.actionable_reason(), Some("1 failed check(s)".into()));
    }
}
