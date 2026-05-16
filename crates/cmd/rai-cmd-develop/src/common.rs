//! `rai develop` の Issue / PR で共通するヘルパー群。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use chrono::Local;
use clap::{Args, ValueEnum};
use rai_core::{
    shell::{self, Shell},
    Result,
};
use serde::Deserialize;

pub const DEFAULT_ENGINE_CMD: &str = "ccs c1 --print --output-format stream-json --verbose {PERMISSION_MODE} -- {PROMPT} | {RAI} claude format";

/// ユーザーが対話 UI (fzf 等) で操作をキャンセルしたことを示すエラー型。
/// `Result` の Err として返し、`rai` のトップレベルが downcast して exit 130 で
/// 終了する。`std::process::exit` を直接呼ぶ実装は Result の destructors を
/// 巻き戻さないため、ここでは `bail!(UserCancelled)` に倒して呼び出し側のクリーン
/// アップを保証する。
#[derive(Debug)]
pub struct UserCancelled;

impl std::fmt::Display for UserCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("user cancelled")
    }
}

impl std::error::Error for UserCancelled {}

/// `rai develop {issue,pr}` 共通の agent 起動関連オプション。
#[derive(Debug, Args, Clone)]
pub struct AgentArgs {
    /// agent CLI の起動コマンド (shell 文字列)。
    ///
    /// プレースホルダ:
    /// - `{PROMPT}`        : 現タスクの prompt (shell-quoted)
    /// - `{PERMISSION_MODE}`: `--permission-mode <MODE>` 一式 (`--permission-mode` 未指定なら空)
    /// - `{RAI}`           : 実行中の `rai` バイナリ絶対パス (shell-quoted)
    ///
    /// プレースホルダを 1 つも含まない文字列を渡した場合は legacy 互換動作で末尾に
    /// `{PERMISSION_MODE}` と `{PROMPT}` を append する。
    #[arg(long, short = 'e', value_name = "CMD", default_value = DEFAULT_ENGINE_CMD)]
    pub engine_cmd: String,

    /// prompt をファイルから読み込む。
    #[arg(long, value_name = "FILE")]
    pub prompt_template: Option<PathBuf>,

    /// tmux を介さず前面で実行 (デバッグ用)。
    #[arg(long)]
    pub no_tmux: bool,

    /// agent 終了後の自動 commit / push / PR 作成 (or push) を無効化する。
    #[arg(long)]
    pub no_auto_publish: bool,

    /// agent (`claude`) に渡す `--permission-mode` を明示する。
    #[arg(long, value_name = "MODE", value_enum)]
    pub permission_mode: Option<PermissionMode>,
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
    pub fn as_arg(self) -> &'static str {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Flavor {
    Issue,
    Pr,
}

impl Flavor {
    pub fn label(self) -> &'static str {
        match self {
            Flavor::Issue => "issue",
            Flavor::Pr => "pr",
        }
    }
}

#[derive(Debug)]
pub struct Worktree {
    pub path: PathBuf,
    pub created: bool,
}

/// tmux セッション + agent 実行に必要な入力。Issue / PR で共通。
pub struct LaunchContext<'a> {
    pub repo: &'a str,
    pub branch: &'a str,
    pub flavor: Flavor,
    pub number: u64,
    pub prompt: &'a str,
    pub finalizer: Option<&'a str>,
    pub agent: &'a AgentArgs,
}

pub fn read_prompt_template(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .with_context(|| format!("failed to read prompt template: {}", path.display()))
}

pub fn build_engine_cmd(engine_cmd: &str, permission_mode: Option<PermissionMode>) -> String {
    if engine_cmd.contains("{PERMISSION_MODE}") {
        // `--verbose {PERMISSION_MODE} -- …` のようにプレースホルダの前後にスペースが
        // 入っているテンプレートで、permission_mode = None のとき素朴に空文字へ置換
        // すると `--verbose  -- …` のように二重スペースが残る。連続するスペースを
        // 単一に正規化したうえで、行頭にプレースホルダがあった場合に残る先頭スペースも
        // `trim_start` で取り除いてから返す (例: `{PERMISSION_MODE} ccs c1 -- …` で
        // permission_mode=None なら ` ccs c1 -- …` → `ccs c1 -- …`)。
        let replaced = match permission_mode {
            Some(mode) => engine_cmd.replace(
                "{PERMISSION_MODE}",
                &format!("--permission-mode {}", mode.as_arg()),
            ),
            None => collapse_spaces(&engine_cmd.replace("{PERMISSION_MODE}", ""))
                .trim_start()
                .to_string(),
        };
        return replaced;
    }
    if let Some(mode) = permission_mode {
        format!("{engine_cmd} --permission-mode {}", mode.as_arg())
    } else {
        engine_cmd.to_string()
    }
}

/// 連続するスペースを 1 つに畳む。`--verbose  -- foo` → `--verbose -- foo`。
/// `{PERMISSION_MODE}` を空に置換した直後のクリーンアップ専用。改行は保持する。
fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

pub fn build_agent_shell_command(
    engine_cmd: &str,
    prompt: &str,
    rai_exe: &str,
    finalizer: Option<&str>,
    shell_kind: Shell,
) -> String {
    let quote = shell::quote_for(shell_kind);
    // `{PROMPT}` プレースホルダの有無だけで「テンプレ模式 vs legacy append 模式」を
    // 切り替える。`{RAI}` は legacy append でも同じく置換しておく。以前は
    // `{PROMPT} || {RAI}` のいずれかがあれば placeholder 模式に倒していたため、
    // `--engine-cmd 'my-cmd {RAI}'` のような prompt を含まないテンプレートを与えると
    // プロンプトが本文に入らないバグになっていた。
    let with_rai = engine_cmd.replace("{RAI}", &quote(rai_exe));
    let agent = if with_rai.contains("{PROMPT}") {
        with_rai.replace("{PROMPT}", &quote(prompt))
    } else {
        format!("{} {}", with_rai, quote(prompt))
    };
    match shell_kind {
        Shell::Posix => build_posix_agent_block(&agent, finalizer),
        Shell::Fish => build_fish_agent_block(&agent, finalizer),
    }
}

fn build_posix_agent_block(agent: &str, finalizer: Option<&str>) -> String {
    let agent_block = format!("set -o pipefail; ({agent})");
    match finalizer {
        Some(finalizer) => format!(
            "{agent_block}; __rai_agent_status=$?; if [ \"$__rai_agent_status\" -ne 0 ]; then echo \"rai: agent exited with status $__rai_agent_status; skip auto publish\" >&2; exit \"$__rai_agent_status\"; fi; {finalizer}"
        ),
        None => agent_block,
    }
}

fn build_fish_agent_block(agent: &str, finalizer: Option<&str>) -> String {
    let pipefail = "set -l __rai_pipe $pipestatus; set -l __rai_agent_status 0; for s in $__rai_pipe; if test $s -ne 0; set __rai_agent_status $s; end; end";
    let agent_block = format!("begin; {agent}; end; {pipefail}");
    match finalizer {
        Some(finalizer) => format!(
            "{agent_block}; if test $__rai_agent_status -ne 0; echo \"rai: agent exited with status $__rai_agent_status; skip auto publish\" >&2; exit $__rai_agent_status; end; {finalizer}"
        ),
        None => agent_block,
    }
}

pub fn wrap_with_log(inner: &str, log_path: &Path, shell_kind: Shell) -> String {
    let log = shell::quote_path(shell_kind, log_path);
    match shell_kind {
        Shell::Posix => format!("({inner}) 2>&1 | tee -a {log}"),
        Shell::Fish => format!("begin; {inner}; end 2>&1 | tee -a {log}"),
    }
}

pub fn engine_log_path(session: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("rai-develop");
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create log dir: {}", dir.display()))?;
    Ok(dir.join(format!("{session}.log")))
}

pub fn launch(ctx: &LaunchContext, wt: &Worktree) -> Result<()> {
    let (shell_path, shell_kind) = shell::detect_user_shell();
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let rai_exe = exe.display().to_string();
    let engine_cmd = build_engine_cmd(&ctx.agent.engine_cmd, ctx.agent.permission_mode);
    let full_cmd =
        build_agent_shell_command(&engine_cmd, ctx.prompt, &rai_exe, ctx.finalizer, shell_kind);

    if ctx.agent.no_tmux {
        let status = shell::shell_command(&shell_path, &full_cmd)
            .current_dir(&wt.path)
            .status()
            .with_context(|| format!("failed to spawn `{shell_path} -c`"))?;
        if !status.success() {
            bail!("engine_cmd exited with {:?}", status.code());
        }
        return Ok(());
    }

    let ts = Local::now().format("%Y%m%d-%H%M%S");
    // 防御的に `/` を `-` に正規化する。現在の呼び出し側はリポジトリ名のみを渡している
    // (例: `rai`) が、将来 `OWNER/REPO` 形式が渡された場合に tmux session 名や log
    // パスが破綻するのを避ける。tmux session 名は `:` も禁止だが現在の構成では出現
    // しないので扱わない。
    let repo_safe = ctx.repo.replace('/', "-");
    let session = format!("{repo_safe}-{}-{}-{ts}", ctx.flavor.label(), ctx.number);
    let log_path = engine_log_path(&session)?;
    let wrapped_cmd = wrap_with_log(&full_cmd, &log_path, shell_kind);

    // tmux の `[shell-command]` 位置に渡す `wrapped_cmd` は、`begin; cmd | other; end
    // 2>&1 | tee …` のようにシェルメタ文字を含む **シェルスクリプト文字列**。
    // - これをクォートせずに `$SHELL -c "<line>"` に渡すと外側シェルが `;` や `|` を
    //   先に解釈してしまい、tmux に届くのは先頭の `begin` だけになる (シェル二重
    //   解釈バグ)。
    // - 単独引数として `user_shell_argv` 経由で渡しても、外側シェル → tmux → tmux の
    //   default-shell とコンテキストが切り替わる際に「結果的には動くが直感に反する」
    //   形になる。
    // したがってここでは **wrapped_cmd を 1 トークンとしてユーザーシェルでクォート**
    // した上で、ユーザーシェルに 1 つのコマンドラインとして渡す。tmux が
    // shell-command を default-shell -c に内部で渡してくれるため、メタ文字は最後の
    // 1 段でだけ解釈される。
    let q = shell::quote_for(shell_kind);
    let tmux_cmdline = format!(
        "tmux new-session -d -s {} -c {} {}",
        q(&session),
        q(&wt.path.display().to_string()),
        q(&wrapped_cmd),
    );
    let spawn = match shell::user_shell_command(&tmux_cmdline).status() {
        Ok(s) => s,
        Err(e) => {
            if wt.created {
                rollback_worktree(ctx.branch);
            }
            return Err(anyhow::Error::new(e).context("failed to spawn tmux"));
        }
    };
    if !spawn.success() {
        if wt.created {
            rollback_worktree(ctx.branch);
        }
        bail!("tmux new-session exited with {:?}", spawn.code());
    }

    thread::sleep(Duration::from_millis(750));
    if !tmux_has_session(&session) {
        let tail = read_log_tail(&log_path, 40).unwrap_or_default();
        if wt.created {
            rollback_worktree(ctx.branch);
        }
        bail!(
            "tmux session `{session}` exited immediately. log: {}\n--- last lines ---\n{}",
            log_path.display(),
            if tail.trim().is_empty() {
                "(empty log)".to_string()
            } else {
                tail
            }
        );
    }

    println!("tmux session: {session}");
    println!("cwd: {}", wt.path.display());
    println!("log: {}", log_path.display());
    println!("attach: tmux attach -t {session}");
    Ok(())
}

pub fn rollback_worktree(branch: &str) {
    eprintln!("tmux start failed; rolling back worktree");
    shell::user_shell_argv(&["gwq", "remove", "--force", branch])
        .status()
        .ok();
}

fn tmux_has_session(session: &str) -> bool {
    shell::user_shell_argv(&["tmux", "has-session", "-t", session])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn read_log_tail(path: &Path, max_lines: usize) -> Option<String> {
    let body = fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = body.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    Some(lines[start..].join("\n"))
}

/// 既存 worktree がある場合は無人で `git reset --hard` + `git clean -fd` +
/// `git pull --rebase` を流して最新化し、無ければ作成する。
///
/// `gwq_add` は閉じた callable で渡すことで Issue (新規 branch) と PR (既存 ref)
/// の両方に対応できる。
pub fn ensure_worktree<F>(branch: &str, gwq_add: F) -> Result<Worktree>
where
    F: FnOnce(&str) -> Result<PathBuf>,
{
    let wt = if let Ok(path) = gwq_get(branch) {
        eprintln!("rai: worktree for `{branch}` exists; resetting + pulling before work");
        refresh_existing_worktree(&path)?;
        Worktree {
            path,
            created: false,
        }
    } else {
        let path = gwq_add(branch)?;
        Worktree {
            path,
            created: true,
        }
    };
    maybe_mise_install(&wt.path);
    maybe_node_install(&wt.path);
    Ok(wt)
}

/// worktree 直下に `mise.toml` / `.mise.toml` があれば `mise install` を流す。
///
/// mise 未インストールや install 失敗で worktree 作成自体を巻き戻すのは過剰なので、
/// 失敗は stderr に通知するだけでエラー伝播はしない。
fn maybe_mise_install(path: &Path) {
    let candidates = ["mise.toml", ".mise.toml"];
    if !candidates.iter().any(|name| path.join(name).exists()) {
        return;
    }
    eprintln!(
        "rai: mise config detected at {}; running `mise install`",
        path.display()
    );
    if let Err(e) = run_in(path, &["mise", "install"]) {
        eprintln!("rai: `mise install` failed: {e}");
    }
}

/// worktree 直下に `package.json` + lockfile があれば対応する package manager で
/// 依存関係 install を流す。`mise install` の後に呼ぶことで mise 経由でインストール
/// された node / pm を使える。
///
/// lockfile が見つからない `package.json` 単独の状態は skip する (どの pm を使う
/// か rai 側で勝手に決めない)。失敗は mise と同じく stderr 通知のみ。
fn maybe_node_install(path: &Path) {
    if !path.join("package.json").exists() {
        return;
    }
    let Some((pm, lockfile)) = detect_node_package_manager(path) else {
        return;
    };
    eprintln!(
        "rai: {lockfile} detected at {}; running `{pm} install`",
        path.display()
    );
    if let Err(e) = run_in(path, &[pm, "install"]) {
        eprintln!("rai: `{pm} install` failed: {e}");
    }
}

/// lockfile を bun → pnpm → yarn → npm の順で検査し、最初にヒットした pm 名と
/// lockfile 名を返す。bun は `bun.lock` (text) と `bun.lockb` (binary) の両方を見る。
fn detect_node_package_manager(path: &Path) -> Option<(&'static str, &'static str)> {
    const CANDIDATES: &[(&str, &str)] = &[
        ("bun", "bun.lock"),
        ("bun", "bun.lockb"),
        ("pnpm", "pnpm-lock.yaml"),
        ("yarn", "yarn.lock"),
        ("npm", "package-lock.json"),
    ];
    CANDIDATES
        .iter()
        .find(|(_, lock)| path.join(lock).exists())
        .map(|(pm, lock)| (*pm, *lock))
}

fn refresh_existing_worktree(path: &Path) -> Result<()> {
    run_in(path, &["git", "reset", "--hard", "HEAD"])
        .context("failed to `git reset --hard HEAD` on existing worktree")?;
    run_in(path, &["git", "clean", "-fd"])
        .context("failed to `git clean -fd` on existing worktree")?;
    run_in(path, &["git", "pull", "--rebase"])
        .context("failed to `git pull --rebase` on existing worktree")?;
    Ok(())
}

fn run_in(cwd: &Path, argv: &[&str]) -> Result<()> {
    let st = shell::user_shell_argv(argv)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("failed to spawn `{}`", argv.join(" ")))?;
    if !st.success() {
        bail!("`{}` failed (status {:?})", argv.join(" "), st.code());
    }
    Ok(())
}

pub fn gwq_get(branch: &str) -> Result<PathBuf> {
    let out = shell::user_shell_argv(&["gwq", "get", branch]).output()?;
    if !out.status.success() {
        bail!("gwq get {branch} not found");
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(PathBuf::from(s))
}

/// `develop resume` 用。既存 worktree のパスを取り出すだけで、`git reset` 等の
/// 破壊的な refresh は **行わない**。`ensure_worktree` と違って、見つからない
/// 場合は新規作成も試みずにエラーで終わる。
pub fn find_existing_worktree(branch: &str) -> Result<Worktree> {
    let path =
        gwq_get(branch).with_context(|| format!("no existing worktree for branch `{branch}`"))?;
    Ok(Worktree {
        path,
        created: false,
    })
}

/// `develop/issue-<N>-*` パターンに合致するローカル branch を列挙する。
/// `develop resume` で issue 番号から worktree を探すために使う。
pub fn issue_branches_for(number: u64) -> Result<Vec<String>> {
    let pattern = format!("refs/heads/develop/issue-{number}-*");
    let raw = match git_capture(&["for-each-ref", "--format=%(refname:short)", &pattern]) {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()),
    };
    Ok(raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn gwq_add_new_branch(branch: &str) -> Result<PathBuf> {
    let st = shell::user_shell_argv(&["gwq", "add", "-b", branch])
        .status()
        .context("failed to spawn gwq add")?;
    if !st.success() {
        bail!("gwq add -b {branch} failed");
    }
    gwq_get(branch).context("failed to resolve gwq path after add")
}

pub fn gwq_add_existing_branch(branch: &str) -> Result<PathBuf> {
    let st = shell::user_shell_argv(&["gwq", "add", branch])
        .status()
        .context("failed to spawn gwq add")?;
    if !st.success() {
        bail!("gwq add {branch} failed");
    }
    gwq_get(branch).context("failed to resolve gwq path after add")
}

pub fn resolve_repo(repo_override: Option<&str>) -> Result<(String, String)> {
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

pub fn gh_capture(args: &[&str]) -> Result<String> {
    let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 1);
    argv.push("gh");
    argv.extend_from_slice(args);
    let out = shell::user_shell_argv(&argv)
        .output()
        .context("failed to spawn `gh` via user shell")?;
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

pub fn git_capture(args: &[&str]) -> Result<String> {
    let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 1);
    argv.push("git");
    argv.extend_from_slice(args);
    let out = shell::user_shell_argv(&argv)
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

/// 単純な文字列リストを fzf で 1 つ選ばせるヘルパ。`pick_with_fzf` は `(number,
/// title)` 形式を前提にしているので、`#0\t<value>` のような番号合成を避けたい
/// 場面 (= branch 名や任意ラベル) で使う。キャンセル時は `UserCancelled` を返す。
pub fn pick_string_with_fzf(items: impl IntoIterator<Item = String>) -> Result<String> {
    let items: Vec<String> = items.into_iter().collect();
    if items.is_empty() {
        bail!("nothing to pick");
    }
    let mut fzf = shell::user_shell_argv(&["fzf"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn `fzf`")?;
    {
        let mut stdin = fzf.stdin.take().ok_or_else(|| anyhow!("fzf stdin"))?;
        for v in &items {
            writeln!(stdin, "{v}").ok();
        }
    }
    let out = fzf.wait_with_output()?;
    if !out.status.success() {
        return Err(anyhow::Error::new(UserCancelled));
    }
    let picked = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if picked.is_empty() {
        return Err(anyhow::Error::new(UserCancelled));
    }
    Ok(picked)
}

pub fn pick_with_fzf(items: impl IntoIterator<Item = (u64, String)>) -> Result<Vec<(u64, String)>> {
    let items: Vec<_> = items.into_iter().collect();
    if items.is_empty() {
        bail!("nothing to pick");
    }
    let mut fzf = shell::user_shell_argv(&["fzf", "--multi"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn `fzf`")?;
    {
        let mut stdin = fzf.stdin.take().ok_or_else(|| anyhow!("fzf stdin"))?;
        for (n, title) in &items {
            writeln!(stdin, "#{n}\t{title}").ok();
        }
    }
    let out = fzf.wait_with_output()?;
    if !out.status.success() {
        // fzf を Esc / Ctrl-C で抜けた = ユーザーキャンセル。
        // process::exit(130) を直接呼ぶ代わりに UserCancelled を返し、トップレベル
        // で exit code に変換する (Result の destructors を巻き戻すため)。
        return Err(anyhow::Error::new(UserCancelled));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    parse_selected(&s)
}

fn parse_selected(s: &str) -> Result<Vec<(u64, String)>> {
    let mut selected = Vec::new();
    for line in s.lines() {
        selected.push(parse_one(line)?);
    }
    if selected.is_empty() {
        // fzf が success かつ stdout 空 = Enter だけで抜けたケース。これも
        // ユーザーキャンセル扱い。
        return Err(anyhow::Error::new(UserCancelled));
    }
    Ok(selected)
}

fn parse_one(line: &str) -> Result<(u64, String)> {
    let (left, title) = line
        .split_once('\t')
        .ok_or_else(|| anyhow!("invalid fzf output"))?;
    let n: u64 = left
        .trim_start_matches('#')
        .parse()
        .map_err(|_| anyhow!("could not parse number from {line}"))?;
    Ok((n, title.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rai_core::shell::Shell;
    use std::path::Path;

    #[test]
    fn agent_shell_command_runs_finalizer_only_after_success() {
        let cmd = build_agent_shell_command(
            "agent --flag",
            "hello world",
            "/opt/rai/rai",
            Some("rai finalize"),
            Shell::Posix,
        );
        // テンプレート展開も含めて返り値が一意に決まるので、`contains` / `starts_with`
        // ではなく `assert_eq!` で完全一致を取る (AGENTS.md Testing ガイドライン)。
        assert_eq!(
            cmd,
            "set -o pipefail; (agent --flag 'hello world'); __rai_agent_status=$?; \
             if [ \"$__rai_agent_status\" -ne 0 ]; then \
             echo \"rai: agent exited with status $__rai_agent_status; skip auto publish\" >&2; \
             exit \"$__rai_agent_status\"; fi; rai finalize"
        );
    }

    #[test]
    fn agent_shell_command_substitutes_placeholders() {
        let cmd = build_agent_shell_command(
            "ccs c1 --print -- {PROMPT} | {RAI} claude format",
            "hello world",
            "/opt/rai/rai",
            None,
            Shell::Posix,
        );
        assert_eq!(
            cmd,
            "set -o pipefail; (ccs c1 --print -- 'hello world' | /opt/rai/rai claude format)"
        );
    }

    #[test]
    fn agent_shell_command_appends_prompt_when_template_has_only_rai_placeholder() {
        // `{RAI}` だけを含み `{PROMPT}` を持たないテンプレートでも、プロンプトを
        // append する legacy fallback に倒れることを確認する。以前は has_placeholder
        // が true になる結果 prompt 本文が消える回帰があった。
        let cmd = build_agent_shell_command(
            "{RAI} claude format",
            "hello world",
            "/opt/rai/rai",
            None,
            Shell::Posix,
        );
        assert_eq!(
            cmd,
            "set -o pipefail; (/opt/rai/rai claude format 'hello world')"
        );
    }

    #[test]
    fn agent_shell_command_emits_fish_block_for_fish_shell() {
        let cmd = build_agent_shell_command(
            "ccs c1 -- {PROMPT} | {RAI} claude format",
            "hello world",
            "/opt/rai/rai",
            Some("rai finalize"),
            Shell::Fish,
        );
        // fish 分岐も入出力が一意なので exact match。
        assert_eq!(
            cmd,
            "begin; ccs c1 -- 'hello world' | '/opt/rai/rai' claude format; end; \
             set -l __rai_pipe $pipestatus; set -l __rai_agent_status 0; \
             for s in $__rai_pipe; if test $s -ne 0; set __rai_agent_status $s; end; end; \
             if test $__rai_agent_status -ne 0; \
             echo \"rai: agent exited with status $__rai_agent_status; skip auto publish\" >&2; \
             exit $__rai_agent_status; end; rai finalize"
        );
    }

    #[test]
    fn build_engine_cmd_substitutes_permission_mode_placeholder() {
        assert_eq!(
            build_engine_cmd(
                "ccs c1 {PERMISSION_MODE} -- {PROMPT}",
                Some(PermissionMode::DontAsk)
            ),
            "ccs c1 --permission-mode dontAsk -- {PROMPT}"
        );
    }

    #[test]
    fn build_engine_cmd_appends_when_no_placeholder() {
        assert_eq!(
            build_engine_cmd("claude", Some(PermissionMode::BypassPermissions)),
            "claude --permission-mode bypassPermissions"
        );
        assert_eq!(build_engine_cmd("claude", None), "claude");
    }

    #[test]
    fn build_engine_cmd_no_permission_mode_collapses_double_spaces_around_placeholder() {
        // デフォルト DEFAULT_ENGINE_CMD と同じ形 (--verbose の直後にプレースホルダが
        // あり、その後 -- が続く) で `permission_mode = None` のとき、二重スペースを
        // 残さない。
        assert_eq!(
            build_engine_cmd(
                "ccs c1 --print --output-format stream-json --verbose {PERMISSION_MODE} -- {PROMPT}",
                None,
            ),
            "ccs c1 --print --output-format stream-json --verbose -- {PROMPT}"
        );
    }

    #[test]
    fn build_engine_cmd_no_permission_mode_trims_leading_space_when_placeholder_is_first() {
        // `{PERMISSION_MODE}` が行頭にあるユーザーテンプレートで permission_mode = None
        // のとき、先頭にスペースが残らないこと。
        assert_eq!(
            build_engine_cmd("{PERMISSION_MODE} ccs c1 -- {PROMPT}", None),
            "ccs c1 -- {PROMPT}"
        );
    }

    #[test]
    fn collapse_spaces_squashes_runs_but_preserves_newlines() {
        assert_eq!(collapse_spaces("a  b   c"), "a b c");
        assert_eq!(collapse_spaces("a\n\nb"), "a\n\nb");
        assert_eq!(collapse_spaces("  leading"), " leading");
        assert_eq!(collapse_spaces("trailing  "), "trailing ");
    }

    #[test]
    fn wrap_with_log_uses_quoted_paths() {
        let p = wrap_with_log(
            "agent --x",
            Path::new("/tmp/has space/run.log"),
            Shell::Posix,
        );
        assert_eq!(p, "(agent --x) 2>&1 | tee -a '/tmp/has space/run.log'");

        let f = wrap_with_log("inner", Path::new("/tmp/x.log"), Shell::Fish);
        assert_eq!(f, "begin; inner; end 2>&1 | tee -a '/tmp/x.log'");
    }

    #[test]
    fn flavor_label_matches_session_naming() {
        assert_eq!(Flavor::Issue.label(), "issue");
        assert_eq!(Flavor::Pr.label(), "pr");
    }

    #[test]
    fn detect_node_package_manager_prefers_bun_then_pnpm_yarn_npm() {
        use std::fs;
        let tmp = std::env::temp_dir().join(format!("rai-pm-detect-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // empty
        assert!(detect_node_package_manager(&tmp).is_none());

        // npm only
        fs::write(tmp.join("package-lock.json"), "{}").unwrap();
        assert_eq!(
            detect_node_package_manager(&tmp),
            Some(("npm", "package-lock.json"))
        );

        // yarn beats npm
        fs::write(tmp.join("yarn.lock"), "").unwrap();
        assert_eq!(
            detect_node_package_manager(&tmp),
            Some(("yarn", "yarn.lock"))
        );

        // pnpm beats yarn
        fs::write(tmp.join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(
            detect_node_package_manager(&tmp),
            Some(("pnpm", "pnpm-lock.yaml"))
        );

        // bun beats pnpm
        fs::write(tmp.join("bun.lock"), "").unwrap();
        assert_eq!(detect_node_package_manager(&tmp), Some(("bun", "bun.lock")));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn default_engine_cmd_uses_real_binaries_only() {
        assert!(DEFAULT_ENGINE_CMD.starts_with("ccs c1"));
        assert!(DEFAULT_ENGINE_CMD.contains("{PROMPT}"));
        assert!(DEFAULT_ENGINE_CMD.contains("{RAI} claude format"));
        assert!(!DEFAULT_ENGINE_CMD.contains("ccs_print"));
    }
}
