//! `rai claude print` — `claude --print` を session-id 単位で「初回 → 継続」
//! 自動切替しながら呼ぶラッパー。
//!
//! 仕様: `docs/specs/21-claude-print.md` 参照。

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context as _};
use clap::{Args, ValueEnum};
use rai_core::{cli::Run, proc, shell, Ctx, Result};

/// claude の `--permission-mode` に渡す値。`rai develop` と同じ 6 種。
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

/// `--output-format` に渡す値。claude の値 (text / json / stream-json) と一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    #[value(name = "text")]
    Text,
    #[value(name = "json")]
    Json,
    #[value(name = "stream-json")]
    StreamJson,
}

impl OutputFormat {
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::StreamJson => "stream-json",
        }
    }
}

/// `rai claude print [OPTIONS] --session-id <UUID> <PROMPT>`
#[derive(Debug, Args)]
pub struct Cmd {
    /// claude に渡す session-id (= 継続用 resume id)。
    #[arg(long = "session-id", value_name = "UUID")]
    session_id: String,

    /// `claude --permission-mode` にパススルー。
    #[arg(long = "permission-mode", value_name = "MODE")]
    permission_mode: Option<PermissionMode>,

    /// `claude --output-format` にパススルー。
    #[arg(long = "output-format", value_name = "FMT")]
    output_format: Option<OutputFormat>,

    /// `claude --verbose` を付ける (stream-json 併用時に必須)。
    /// global `-v/--verbose` と衝突しないよう `--claude-verbose` に分けてある。
    #[arg(long = "claude-verbose")]
    claude_verbose: bool,

    /// `claude --fork-session` を付ける (継続時のみ意味あり)。
    #[arg(long = "fork-session")]
    fork_session: bool,

    /// 起動する claude バイナリ。デフォルトは `claude`。
    #[arg(long = "claude-bin", value_name = "PATH", default_value = "claude")]
    claude_bin: String,

    /// claude に渡すプロンプト本文 (1 つの文字列)。
    #[arg(value_name = "PROMPT")]
    prompt: String,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        validate(
            &self.session_id,
            &self.prompt,
            self.output_format,
            self.claude_verbose,
        )?;
        let dir = marker_dir_from_env(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"));
        let code = execute(
            &self.claude_bin,
            &self.session_id,
            &dir,
            self.permission_mode,
            self.output_format,
            self.claude_verbose,
            self.fork_session,
            &self.prompt,
            // ユーザーフェイシングな起動は **常に tmux 経由**。終了後も pane が残るので
            // 後追いで `tmux attach -t <name>` して確認できる。Direct 起動はテスト用の
            // 内部分岐のみで使う (CLI からは到達できない)。
            Launch::Tmux,
        )?;
        if code != 0 {
            std::process::exit(code);
        }
        Ok(())
    }
}

/// `execute` が claude をどう起動するかを指定する内部 enum。
/// CLI からは常に `Tmux`。`Direct` はユニットテストの統合シナリオ専用なので、
/// 非 test ビルドで dead_code 扱いされないよう明示的に allow している。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Launch {
    /// `claude` を子プロセスとして直接実行する (テスト用)。
    #[cfg_attr(not(test), allow(dead_code))]
    Direct,
    /// `tmux new-session -d` で新しい detached セッションを 1 つ立て、その中で
    /// claude を実行する。claude 終了後も `sleep infinity` で pane を保持する。
    Tmux,
}

fn validate(
    session_id: &str,
    prompt: &str,
    output_format: Option<OutputFormat>,
    claude_verbose: bool,
) -> Result<()> {
    if prompt.is_empty() {
        bail!("<PROMPT> must not be empty");
    }
    if session_id.trim().is_empty() {
        bail!("--session-id must not be empty");
    }
    // claude 自身も同じ組み合わせを拒否するが、shell + claude のクラッシュメッセージは
    // 分かりづらいので rai 側で先に弾く。
    if matches!(output_format, Some(OutputFormat::StreamJson)) && !claude_verbose {
        bail!("--output-format stream-json requires --claude-verbose (claude rejects this combination)");
    }
    Ok(())
}

/// 実際の起動ロジック。`Run::run` から呼ばれるが、テストからも `marker_dir` /
/// `claude_bin` を直接差し替えて呼べるよう独立関数にしている。
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute(
    claude_bin: &str,
    session_id: &str,
    marker_dir: &Path,
    permission_mode: Option<PermissionMode>,
    output_format: Option<OutputFormat>,
    claude_verbose: bool,
    fork_session: bool,
    prompt: &str,
    launch: Launch,
) -> Result<i32> {
    let marker = marker_dir.join(session_id);
    let mode = if marker.exists() {
        SessionMode::Resume
    } else {
        SessionMode::Initial
    };

    // 重要: claude を spawn する **前** に marker を書く。
    // claude は `--session-id <UUID>` を投げた瞬間に session を「登録済」にする。
    // 応答途中で SIGKILL / OOM / 親死で rai が殺された場合、後から marker を書く
    // 旧実装では「session は登録されたが marker は無い」状態になり、次回呼び出しが
    // また `--session-id` で起動して claude に "already in use" で殺される。
    // 先に marker を書いておけば、たとえ claude バイナリが PATH に無くて起動その
    // ものに失敗するケース (= session は登録されていない) でも、ユーザーは
    // `rm <marker>` で復帰できる。fail-safe 側に倒す。
    if matches!(mode, SessionMode::Initial) {
        fs::create_dir_all(marker_dir)
            .with_context(|| format!("failed to create marker dir {}", marker_dir.display()))?;
        fs::write(&marker, b"")
            .with_context(|| format!("failed to create marker file {}", marker.display()))?;
    }

    let argv = build_argv(
        claude_bin,
        session_id,
        mode,
        permission_mode,
        output_format,
        claude_verbose,
        fork_session,
        prompt,
    );

    match launch {
        Launch::Direct => {
            let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();
            let status = shell::user_shell_argv(&argv_ref)
                .status()
                .with_context(|| format!("failed to spawn claude: {argv:?}"))?;
            Ok(proc::shell_exit_code(&status))
        }
        Launch::Tmux => execute_in_tmux(&argv, marker_dir, session_id),
    }
}

/// `argv` を tmux 内で実行し、claude の exit code を sentinel ファイル経由で受け取る。
///
/// 設計:
/// - `tmux new-session -d -s <name>` で detached セッションを 1 つ作り、その中で
///   POSIX shell スクリプトを 1 つ実行する。スクリプトは:
///   1. claude を実行
///   2. exit code を sentinel に書く
///   3. `exec tail -f /dev/null` で pane を保持
/// - claude が終了しても pane は `tail -f` で生き続けるので、ユーザーは後から
///   `tmux attach -t <name>` で出力を確認できる。
/// - `rai claude print` 本体は sentinel ファイルが現れるまでポーリングし、出てきた
///   exit code をそのまま返す。tmux セッションはそのまま残しておく (= 「print 終わっても
///   消えない」要件)。
/// - `sleep infinity` を使わないのは macOS の BSD `sleep` が `infinity` を受け
///   付けず即時 exit するため。pane が死ぬと session ごと消えてしまう。
///   `tail -f /dev/null` は GNU 拡張に依らず、シグナル待ちで CPU も使わない。
fn execute_in_tmux(argv: &[String], marker_dir: &Path, session_id: &str) -> Result<i32> {
    // tmux の `[shell-command]` は **単一文字列** で、`default-shell` (= 通常 fish や zsh)
    // に渡される。default-shell の引用ルールはユーザー環境次第なので、引用地獄を避ける
    // ため POSIX `/bin/sh` 用の小さなスクリプトを sidecar として書き出し、tmux には
    // そのパスだけ渡す。スクリプト内では POSIX で claude を呼び、exit code を sentinel
    // に書いてから `exec sleep infinity` で pane を保持する。

    let ts_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    fs::create_dir_all(marker_dir)
        .with_context(|| format!("failed to create marker dir {}", marker_dir.display()))?;
    let sentinel = marker_dir.join(format!("{session_id}.{ts_ns}.rc"));
    let _ = fs::remove_file(&sentinel);
    let sentinel_str = sentinel
        .to_str()
        .context("sentinel path is not valid UTF-8")?;

    let claude_cmdline = argv
        .iter()
        .map(|s| shell::quote_posix(s))
        .collect::<Vec<_>>()
        .join(" ");
    // `exec tail -f /dev/null` で pane を保持する。macOS の BSD `sleep` は
    // `infinity` を受け付けず即時 exit してしまうため (= pane が死んで session
    // ごと消える)、GNU 拡張に依らない `tail -f /dev/null` を使う。
    let script_body = format!(
        "#!/bin/sh\nset +e\n{claude_cmdline}\n__rai_rc=$?\nprintf '%d' \"$__rai_rc\" > {sentinel_q}\nprintf '\\n--- rai claude print: claude exited rc=%d. tmux session preserved. ---\\n' \"$__rai_rc\"\nexec tail -f /dev/null\n",
        sentinel_q = shell::quote_posix(sentinel_str),
    );
    let script_path = marker_dir.join(format!("{session_id}.{ts_ns}.sh"));
    fs::write(&script_path, script_body.as_bytes())
        .with_context(|| format!("failed to write tmux script {}", script_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("failed to chmod {}", script_path.display()))?;
    }

    // tmux session 名は session_id の先頭 8 文字 + タイムスタンプで衝突回避。
    // `.` と `:` は禁止文字なので、英数字とハイフンのみにする。
    let short_id = session_id
        .split('-')
        .next()
        .unwrap_or(session_id)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>();
    let tmux_session = format!("rai-claude-print-{short_id}-{ts_ns}");

    let script_path_str = script_path
        .to_str()
        .context("script path is not valid UTF-8")?;
    let tmux_argv: Vec<&str> = vec![
        "tmux",
        "new-session",
        "-d",
        "-s",
        tmux_session.as_str(),
        script_path_str,
    ];

    let status = shell::user_shell_argv(&tmux_argv)
        .status()
        .with_context(|| format!("failed to spawn tmux: {tmux_argv:?}"))?;
    if !status.success() {
        let code = proc::shell_exit_code(&status);
        bail!(
            "tmux new-session failed with exit code {code} \
             (is tmux installed? session name attempted: {tmux_session})"
        );
    }

    // tmux 経由で起動したセッション名を、テスト / 後追い用の sidecar ファイルに書き残す。
    // 出力には `<sentinel>` を共有するパス命名規則を使う (`<sentinel>.tmux`)。
    let session_file = sentinel.with_extension("tmux");
    fs::write(&session_file, tmux_session.as_bytes()).with_context(|| {
        format!(
            "failed to write tmux session sidecar file {}",
            session_file.display()
        )
    })?;

    eprintln!(
        "rai claude print: tmux session '{tmux_session}' is running. \
         attach: `tmux attach -t {tmux_session}` (it stays alive after claude exits)"
    );

    // sentinel をポーリング。claude の完了を待つだけで、tmux session は触らない。
    // NotFound の場合は tmux session の生存も同時にチェックし、session が消えていれば
    // (= OOM / SIGKILL / 手動 kill 等で claude が完了前に死んだ) bail する。これが
    // 無いと sentinel が永遠に出ない場合に `rai claude print` が無限ハングする。
    loop {
        match fs::read_to_string(&sentinel) {
            Ok(s) => {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    let code: i32 = trimmed.parse().unwrap_or(1);
                    return Ok(code);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !tmux_has_session(&tmux_session)? {
                    bail!(
                        "tmux session '{tmux_session}' disappeared before claude wrote \
                         its exit code to {sentinel_path}. claude was likely killed \
                         (SIGKILL / OOM / manual kill) before completing. \
                         Inspect `tmux ls` and remove the marker {marker} if you want \
                         to retry with a fresh --session-id.",
                        sentinel_path = sentinel.display(),
                        marker = marker_dir.join(session_id).display(),
                    );
                }
            }
            Err(e) => {
                return Err(e).with_context(|| format!("read sentinel {}", sentinel.display()))
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// `tmux has-session -t <name>` でセッションの生存を確認する。tmux 経由なので
/// ユーザーシェル (`user_shell_argv`) 越しに呼ぶ (rai 全体の shell ポリシーに準拠)。
fn tmux_has_session(name: &str) -> Result<bool> {
    let status = shell::user_shell_argv(&["tmux", "has-session", "-t", name])
        .stderr(std::process::Stdio::null())
        .status()
        .with_context(|| format!("failed to spawn `tmux has-session -t {name}`"))?;
    Ok(status.success())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionMode {
    Initial,
    Resume,
}

/// marker ファイルを置くディレクトリ。
fn marker_dir_from_env(xdg_state_home: Option<OsString>, home: Option<OsString>) -> PathBuf {
    if let Some(state) = xdg_state_home {
        let p = PathBuf::from(state);
        if !p.as_os_str().is_empty() {
            return p.join("rai").join("claude-print");
        }
    }
    let home = home
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local")
        .join("state")
        .join("rai")
        .join("claude-print")
}

/// 実行する argv を組み立てる。テストしやすいよう副作用なし。
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_argv(
    claude_bin: &str,
    session_id: &str,
    mode: SessionMode,
    permission_mode: Option<PermissionMode>,
    output_format: Option<OutputFormat>,
    verbose: bool,
    fork_session: bool,
    prompt: &str,
) -> Vec<String> {
    let mut argv: Vec<String> = Vec::with_capacity(10);
    argv.push(claude_bin.to_string());
    argv.push("--print".to_string());
    match mode {
        SessionMode::Initial => {
            argv.push("--session-id".to_string());
            argv.push(session_id.to_string());
        }
        SessionMode::Resume => {
            argv.push("--resume".to_string());
            argv.push(session_id.to_string());
        }
    }
    if let Some(mode) = permission_mode {
        argv.push("--permission-mode".to_string());
        argv.push(mode.as_arg().to_string());
    }
    if let Some(fmt) = output_format {
        argv.push("--output-format".to_string());
        argv.push(fmt.as_arg().to_string());
    }
    if verbose {
        argv.push("--verbose".to_string());
    }
    if fork_session {
        argv.push("--fork-session".to_string());
    }
    argv.push("--".to_string());
    argv.push(prompt.to_string());
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_initial_minimal() {
        let got = build_argv(
            "claude",
            "11111111-2222-3333-4444-555555555555",
            SessionMode::Initial,
            None,
            None,
            false,
            false,
            "hello",
        );
        assert_eq!(
            got,
            vec![
                "claude",
                "--print",
                "--session-id",
                "11111111-2222-3333-4444-555555555555",
                "--",
                "hello",
            ]
        );
    }

    #[test]
    fn argv_resume_with_all_flags() {
        let got = build_argv(
            "/usr/local/bin/claude",
            "abc",
            SessionMode::Resume,
            Some(PermissionMode::BypassPermissions),
            Some(OutputFormat::StreamJson),
            true,
            true,
            "do it",
        );
        assert_eq!(
            got,
            vec![
                "/usr/local/bin/claude",
                "--print",
                "--resume",
                "abc",
                "--permission-mode",
                "bypassPermissions",
                "--output-format",
                "stream-json",
                "--verbose",
                "--fork-session",
                "--",
                "do it",
            ]
        );
    }

    #[test]
    fn argv_permission_modes_serialize() {
        for (m, want) in [
            (PermissionMode::AcceptEdits, "acceptEdits"),
            (PermissionMode::Auto, "auto"),
            (PermissionMode::BypassPermissions, "bypassPermissions"),
            (PermissionMode::Default, "default"),
            (PermissionMode::DontAsk, "dontAsk"),
            (PermissionMode::Plan, "plan"),
        ] {
            assert_eq!(m.as_arg(), want);
        }
    }

    #[test]
    fn argv_output_formats_serialize() {
        for (fmt, want) in [
            (OutputFormat::Text, "text"),
            (OutputFormat::Json, "json"),
            (OutputFormat::StreamJson, "stream-json"),
        ] {
            assert_eq!(fmt.as_arg(), want);
        }
    }

    #[test]
    fn marker_dir_uses_xdg_state_home_when_set() {
        let got = marker_dir_from_env(
            Some(OsString::from("/tmp/xdg-state-rai-test")),
            Some(OsString::from("/var/empty")),
        );
        assert_eq!(
            got,
            PathBuf::from("/tmp/xdg-state-rai-test/rai/claude-print")
        );
    }

    #[test]
    fn marker_dir_falls_back_to_home_local_state_when_xdg_unset() {
        let got = marker_dir_from_env(None, Some(OsString::from("/var/empty")));
        assert_eq!(
            got,
            PathBuf::from("/var/empty/.local/state/rai/claude-print")
        );
    }

    #[test]
    fn marker_dir_falls_back_to_home_when_xdg_empty() {
        let got = marker_dir_from_env(Some(OsString::new()), Some(OsString::from("/h")));
        assert_eq!(got, PathBuf::from("/h/.local/state/rai/claude-print"));
    }

    #[test]
    fn marker_dir_falls_back_to_dot_when_no_home() {
        let got = marker_dir_from_env(None, None);
        assert_eq!(got, PathBuf::from("./.local/state/rai/claude-print"));
    }

    #[test]
    fn validate_rejects_stream_json_without_claude_verbose() {
        let err = validate("uuid", "p", Some(OutputFormat::StreamJson), false).unwrap_err();
        assert_eq!(
            err.to_string(),
            "--output-format stream-json requires --claude-verbose (claude rejects this combination)"
        );
    }

    #[test]
    fn validate_allows_stream_json_with_claude_verbose() {
        assert!(validate("uuid", "p", Some(OutputFormat::StreamJson), true).is_ok());
    }

    #[test]
    fn validate_rejects_empty_prompt() {
        assert!(validate("uuid", "", None, false).is_err());
    }

    #[test]
    fn validate_rejects_blank_session_id() {
        assert!(validate("   ", "p", None, false).is_err());
    }

    /// 統合テスト相当: stub の `claude` を被せて、同じ session-id への 1 回目 →
    /// 2 回目で `--session-id` から `--resume` に切替わり、各回の argv が
    /// stub のログに記録されることを確認する。`marker_dir` は temp に隔離。
    #[cfg(unix)]
    #[test]
    fn execute_switches_session_id_to_resume_between_invocations() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = std::env::temp_dir().join(format!(
            "rai-claude-print-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&tmp).unwrap();
        // scope guard for tempdir cleanup
        let _guard = scopeguard(&tmp);

        let log = tmp.join("argv.log");
        let stub = tmp.join("stub-claude.sh");
        fs::write(
            &stub,
            format!(
                "#!/bin/sh\necho \"$@\" >> {}\nexit 0\n",
                shell::quote_posix(log.to_str().unwrap())
            ),
        )
        .unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();

        let markers = tmp.join("markers");
        let uuid = "11111111-2222-3333-4444-555555555501";

        let code = execute(
            stub.to_str().unwrap(),
            uuid,
            &markers,
            None,
            None,
            false,
            false,
            "first",
            Launch::Direct,
        )
        .unwrap();
        assert_eq!(code, 0);
        assert!(
            markers.join(uuid).exists(),
            "marker should exist after 1st call"
        );
        let log_after_first: Vec<Vec<String>> = parse_stub_log(&log);
        assert_eq!(
            log_after_first,
            vec![vec![
                "--print".to_string(),
                "--session-id".to_string(),
                uuid.to_string(),
                "--".to_string(),
                "first".to_string(),
            ]]
        );

        let code = execute(
            stub.to_str().unwrap(),
            uuid,
            &markers,
            None,
            None,
            false,
            false,
            "second",
            Launch::Direct,
        )
        .unwrap();
        assert_eq!(code, 0);
        let log_after_second: Vec<Vec<String>> = parse_stub_log(&log);
        assert_eq!(
            log_after_second,
            vec![
                vec![
                    "--print".to_string(),
                    "--session-id".to_string(),
                    uuid.to_string(),
                    "--".to_string(),
                    "first".to_string(),
                ],
                vec![
                    "--print".to_string(),
                    "--resume".to_string(),
                    uuid.to_string(),
                    "--".to_string(),
                    "second".to_string(),
                ],
            ]
        );
    }

    /// 統合テスト: 起動 **前** に marker を書く挙動。stub claude を非 0 終了で
    /// 死なせても marker は残っており、次回呼び出しが `--resume` に倒れる。
    /// これは旧実装 (= claude 終了後に marker を書く) では再現できない動作。
    #[cfg(unix)]
    #[test]
    fn execute_writes_marker_before_spawn_so_failures_still_resume_next_time() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = std::env::temp_dir().join(format!(
            "rai-claude-print-fail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&tmp).unwrap();
        let _guard = scopeguard(&tmp);

        let log = tmp.join("argv.log");
        let stub = tmp.join("stub-claude.sh");
        // exit 1 で死ぬが、argv は記録する。
        fs::write(
            &stub,
            format!(
                "#!/bin/sh\necho \"$@\" >> {}\nexit 1\n",
                shell::quote_posix(log.to_str().unwrap())
            ),
        )
        .unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();

        let markers = tmp.join("markers");
        let uuid = "22222222-3333-4444-5555-666666666602";

        let code = execute(
            stub.to_str().unwrap(),
            uuid,
            &markers,
            None,
            None,
            false,
            false,
            "first",
            Launch::Direct,
        )
        .unwrap();
        assert_eq!(code, 1, "stub should exit 1");
        assert!(
            markers.join(uuid).exists(),
            "marker must be created even when claude exits non-zero"
        );

        let code = execute(
            stub.to_str().unwrap(),
            uuid,
            &markers,
            None,
            None,
            false,
            false,
            "second",
            Launch::Direct,
        )
        .unwrap();
        assert_eq!(code, 1);
        let parsed = parse_stub_log(&log);
        assert_eq!(
            parsed,
            vec![
                vec![
                    "--print".to_string(),
                    "--session-id".to_string(),
                    uuid.to_string(),
                    "--".to_string(),
                    "first".to_string(),
                ],
                vec![
                    "--print".to_string(),
                    "--resume".to_string(),
                    uuid.to_string(),
                    "--".to_string(),
                    "second".to_string(),
                ],
            ]
        );
    }

    /// stub claude が `echo "$@" >> <log>` で書き出した行を、空白区切りの token 列に
    /// パースする。各テストが期待 argv と `assert_eq!` で完全比較できるようにする。
    fn parse_stub_log(path: &Path) -> Vec<Vec<String>> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| line.split_whitespace().map(str::to_string).collect())
            .collect()
    }

    /// 簡易な tempdir cleanup ガード (`tempfile` クレートを workspace に足さずに済ます)。
    fn scopeguard(path: &Path) -> impl Drop + '_ {
        struct G<'a>(&'a Path);
        impl<'a> Drop for G<'a> {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(self.0);
            }
        }
        G(path)
    }

    /// `tmux -V` で tmux が PATH 上にあるかを確認するヘルパ。本番コードと揃えて
    /// ユーザーシェル経由で起動する (`Command::new("tmux")` 直叩きは shell policy 違反)。
    fn tmux_available() -> bool {
        shell::user_shell_argv(&["tmux", "-V"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// テスト終了時に対象 tmux session を kill するガード。テスト失敗時の取り残しを防ぐ。
    fn tmux_session_guard(name: String) -> impl Drop {
        struct G(String);
        impl Drop for G {
            fn drop(&mut self) {
                let _ = shell::user_shell_argv(&["tmux", "kill-session", "-t", &self.0]).output();
            }
        }
        G(name)
    }

    /// 統合テスト: `Launch::Tmux` で claude が tmux 内で実行され、claude 終了後も
    /// pane (= tmux session) が残り続け、`rai claude print` 本体は claude の exit
    /// code を sentinel 経由で受け取って正しく返すことを確認する。
    /// tmux が CI runner で利用できない場合はスキップ。
    #[cfg(unix)]
    #[test]
    fn execute_tmux_mode_returns_claude_rc_and_keeps_session_alive() {
        if !tmux_available() {
            eprintln!("tmux not installed; skipping execute_tmux_mode test");
            return;
        }

        use std::os::unix::fs::PermissionsExt;

        let tmp = std::env::temp_dir().join(format!(
            "rai-claude-print-tmux-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&tmp).unwrap();
        let _guard = scopeguard(&tmp);

        let log = tmp.join("argv.log");
        let stub = tmp.join("stub-claude.sh");
        // 0 で終了し、tmux 側の sleep infinity でセッションが残るパスを検証する。
        fs::write(
            &stub,
            format!(
                "#!/bin/sh\necho \"$@\" >> {}\nexit 0\n",
                shell::quote_posix(log.to_str().unwrap())
            ),
        )
        .unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();

        let markers = tmp.join("markers");
        let uuid = "ffffffff-ffff-4fff-bfff-ffffffffff01";

        let code = execute(
            stub.to_str().unwrap(),
            uuid,
            &markers,
            None,
            None,
            false,
            false,
            "tmux-first",
            Launch::Tmux,
        )
        .unwrap();
        assert_eq!(code, 0);

        // stub が argv を log に書いたことを確認 (= tmux 内で実際に起動された)。
        let parsed = parse_stub_log(&log);
        assert_eq!(
            parsed,
            vec![vec![
                "--print".to_string(),
                "--session-id".to_string(),
                uuid.to_string(),
                "--".to_string(),
                "tmux-first".to_string(),
            ]]
        );

        // execute_in_tmux が書き残した sidecar から、起動した tmux session 名を取る。
        // テスト終了時に確実に kill するため tmux_session_guard で保護する。
        let sidecar = fs::read_dir(&markers)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|s| s.to_str()) == Some("tmux"))
            .expect("execute_in_tmux must write a <sentinel>.tmux sidecar");
        let tmux_session = fs::read_to_string(&sidecar).unwrap().trim().to_string();
        let _session_kill_guard = tmux_session_guard(tmux_session.clone());

        // tmux ls にその session 名が含まれている = print 終了後も session が残っている。
        // ユーザーシェル経由 (rai 全体の shell policy に準拠)。
        let ls_out = shell::user_shell_argv(&["tmux", "ls", "-F", "#S"])
            .output()
            .expect("tmux ls");
        let ls_stdout = String::from_utf8_lossy(&ls_out.stdout);
        let ls_stderr = String::from_utf8_lossy(&ls_out.stderr);
        assert!(
            ls_stdout.lines().any(|l| l == tmux_session),
            "tmux ls must list the print session '{tmux_session}' (stdout={ls_stdout:?} stderr={ls_stderr:?})"
        );

        // 「print 終わっても消えない」を確認する: claude (stub) 終了から 1 秒経過後も
        // session がまだ alive であること。これで `exec tail -f /dev/null` の hold が
        // 効いていることが分かる。pane_current_command は default-shell (fish) が
        // 子プロセスを wait() しているため "fish" のままになるので、コマンド名では
        // なく **session の生存** だけを最終確認にする。
        std::thread::sleep(Duration::from_secs(1));
        let ls_after = shell::user_shell_argv(&["tmux", "ls", "-F", "#S"])
            .output()
            .expect("tmux ls (after wait)");
        let ls_after_stdout = String::from_utf8_lossy(&ls_after.stdout);
        assert!(
            ls_after_stdout.lines().any(|l| l == tmux_session),
            "tmux session '{tmux_session}' must still exist 1s after claude exited \
             (stdout={ls_after_stdout:?}). claude exit was sentinel-confirmed, so a missing \
             session here means the hold-after-exit logic broke."
        );
    }

    /// `tmux_has_session` のスモークテスト: 起動した session の有無で true/false が
    /// 切り替わることだけを確認する。tmux 経由ポリシーに準拠した起動になっていること
    /// 自体は `tmux_session_guard` も同じ helper を使っているので間接的に検証される。
    #[cfg(unix)]
    #[test]
    fn tmux_has_session_returns_true_only_when_session_exists() {
        if !tmux_available() {
            eprintln!("tmux not installed; skipping tmux_has_session test");
            return;
        }
        let name = format!(
            "rai-claude-print-test-has-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        assert!(
            !tmux_has_session(&name).unwrap(),
            "session that was never created must not exist"
        );
        let create = shell::user_shell_argv(&[
            "tmux",
            "new-session",
            "-d",
            "-s",
            name.as_str(),
            "tail -f /dev/null",
        ])
        .status()
        .expect("tmux new-session");
        assert!(create.success(), "tmux new-session must succeed");
        let _kill_guard = tmux_session_guard(name.clone());
        assert!(
            tmux_has_session(&name).unwrap(),
            "just-created session must be reported as existing"
        );
    }
}
