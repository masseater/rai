//! `rai claude print` — `claude --print` を session-id 単位で「初回 → 継続」
//! 自動切替しながら呼ぶラッパー。
//!
//! 仕様: `docs/specs/21-claude-print.md` 参照。

use std::fs;
use std::path::PathBuf;
use std::process::ExitStatus;

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
    fn as_arg(self) -> &'static str {
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

    /// `claude --verbose` を付ける (stream-json 併用時に必要)。
    /// 名前が global `-v/--verbose` と衝突しないよう `--claude-verbose` に分けてある。
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
        if self.prompt.is_empty() {
            bail!("<PROMPT> must not be empty");
        }
        if self.session_id.trim().is_empty() {
            bail!("--session-id must not be empty");
        }

        let marker_dir = marker_dir();
        let marker = marker_dir.join(&self.session_id);
        let mode = if marker.exists() {
            SessionMode::Resume
        } else {
            SessionMode::Initial
        };

        let argv = build_argv(
            &self.claude_bin,
            &self.session_id,
            mode,
            self.permission_mode,
            self.output_format,
            self.claude_verbose,
            self.fork_session,
            &self.prompt,
        );
        let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();

        let status: ExitStatus = shell::user_shell_argv(&argv_ref)
            .status()
            .with_context(|| format!("failed to spawn claude: {argv:?}"))?;

        // session 自体は初回呼び出しの時点で claude 側に作成済み (or 作成試行済み)。
        // 二度目以降に `--session-id` を当てると確実に "already in use" になるので、
        // 失敗終了でも marker は必ず作る。
        if matches!(mode, SessionMode::Initial) {
            fs::create_dir_all(&marker_dir)
                .with_context(|| format!("failed to create marker dir {}", marker_dir.display()))?;
            // 既存ファイル上書きは無害。エラーは無視せず伝播。
            fs::write(&marker, b"")
                .with_context(|| format!("failed to create marker file {}", marker.display()))?;
        }

        let code = proc::shell_exit_code(&status);
        if code != 0 {
            std::process::exit(code);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionMode {
    Initial,
    Resume,
}

/// marker ファイルを置くディレクトリ。
fn marker_dir() -> PathBuf {
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        let p = PathBuf::from(state);
        if !p.as_os_str().is_empty() {
            return p.join("rai").join("claude-print");
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local")
        .join("state")
        .join("rai")
        .join("claude-print")
}

/// 実行する argv を組み立てる。テストしやすいよう副作用なし。
#[allow(clippy::too_many_arguments)]
fn build_argv(
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
    fn marker_dir_honors_xdg_state_home() {
        let original = std::env::var_os("XDG_STATE_HOME");
        let home = std::env::var_os("HOME");
        std::env::set_var("XDG_STATE_HOME", "/tmp/xdg-state-rai-test");
        assert_eq!(
            marker_dir(),
            PathBuf::from("/tmp/xdg-state-rai-test/rai/claude-print")
        );
        std::env::remove_var("XDG_STATE_HOME");
        std::env::set_var("HOME", "/var/empty");
        assert_eq!(
            marker_dir(),
            PathBuf::from("/var/empty/.local/state/rai/claude-print")
        );
        // restore
        if let Some(v) = original {
            std::env::set_var("XDG_STATE_HOME", v);
        }
        if let Some(v) = home {
            std::env::set_var("HOME", v);
        } else {
            std::env::remove_var("HOME");
        }
    }
}
