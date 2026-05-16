//! `rai claude print` — `claude --print` を session-id 単位で「初回 → 継続」
//! 自動切替しながら呼ぶラッパー。
//!
//! 仕様: `docs/specs/21-claude-print.md` 参照。

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

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
        )?;
        if code != 0 {
            std::process::exit(code);
        }
        Ok(())
    }
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
    let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();

    let status = shell::user_shell_argv(&argv_ref)
        .status()
        .with_context(|| format!("failed to spawn claude: {argv:?}"))?;
    Ok(proc::shell_exit_code(&status))
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
        let msg = format!("{err}");
        assert!(
            msg.contains("--claude-verbose"),
            "unexpected message: {msg}"
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
        )
        .unwrap();
        assert_eq!(code, 0);
        assert!(
            markers.join(uuid).exists(),
            "marker should exist after 1st call"
        );
        let log1 = fs::read_to_string(&log).unwrap();
        assert!(
            log1.contains(&format!("--session-id {uuid}")),
            "first call should use --session-id: {log1:?}"
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
        )
        .unwrap();
        assert_eq!(code, 0);
        let log2 = fs::read_to_string(&log).unwrap();
        let second_line = log2.lines().nth(1).expect("two log lines");
        assert!(
            second_line.contains(&format!("--resume {uuid}")),
            "second call should use --resume: {second_line:?}"
        );
        assert!(
            !second_line.contains("--session-id"),
            "second call must not include --session-id: {second_line:?}"
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
        )
        .unwrap();
        assert_eq!(code, 1);
        let log2 = fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = log2.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[1].contains(&format!("--resume {uuid}")),
            "second call after failure must still use --resume: {:?}",
            lines[1]
        );
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
}
