//! `rai claude pair` — 2 つのプロンプト (A / B) を交互に `claude --print` で回す。
//!
//! 仕様: `docs/specs/22-claude-pair.md` 参照。
//!
//! 内部実装は `rai pair` を **子プロセス** として呼ぶ。`rai-cmd-pair` を直接 link
//! しないのはワークスペースの「Subcommand crate どうしを横断して depend しない」
//! ポリシーに従うため (`AGENTS.md` の Important Instructions 参照)。

use std::fs::File;
use std::io::Read;

use anyhow::Context as _;
use clap::Args;
use rai_core::{cli::Run, proc, shell, Ctx, Result};

use crate::print::PermissionMode;

#[derive(Debug, Args)]
pub struct Cmd {
    /// A 役に毎サイクル渡すプロンプト本文。
    #[arg(long = "prompt-a", value_name = "STR")]
    prompt_a: String,

    /// B 役に毎サイクル渡すプロンプト本文。
    #[arg(long = "prompt-b", value_name = "STR")]
    prompt_b: String,

    /// 最大サイクル数 (A→B で 1 サイクル)。
    #[arg(long = "max-cycles", default_value_t = 10)]
    max_cycles: u32,

    /// 累積最大実行時間 (時間)。0 で無制限。
    #[arg(long = "max-hours", default_value_t = 48)]
    max_hours: u32,

    /// `rai claude print` 経由で claude にパススルーする permission mode。
    #[arg(long = "permission-mode", value_name = "MODE")]
    permission_mode: Option<PermissionMode>,

    /// A 用の session-id。未指定なら新規 UUID を 1 度だけ生成する。
    #[arg(long = "id-a", value_name = "UUID")]
    id_a: Option<String>,

    /// B 用の session-id。未指定なら新規 UUID を 1 度だけ生成する。
    #[arg(long = "id-b", value_name = "UUID")]
    id_b: Option<String>,

    /// 各プロンプトの先頭に付与する文字列 (空文字なら付与しない)。
    #[arg(long = "prepend", default_value = "/goal")]
    prepend: String,

    /// 下部固定ステータスバーを無効化 (`rai pair --no-status-bar` をそのまま透過)。
    #[arg(long = "no-status-bar")]
    no_status_bar: bool,

    /// 子プロセスとして呼ぶ `rai` バイナリ。未指定時は `current_exe()`。
    #[arg(long = "rai-bin", value_name = "PATH")]
    rai_bin: Option<String>,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let id_a = self.id_a.clone().unwrap_or_else(random_uuid_v4);
        let id_b = self.id_b.clone().unwrap_or_else(random_uuid_v4);

        let rai_bin = match self.rai_bin.clone() {
            Some(p) => p,
            None => std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "rai".to_string()),
        };

        let (_shell_path, shell_kind) = shell::detect_user_shell();
        let q = shell::quote_for(shell_kind);

        let cmd_a = build_print_invocation(
            &rai_bin,
            &id_a,
            self.permission_mode,
            &self.prepend,
            &self.prompt_a,
            q,
        );
        let cmd_b = build_print_invocation(
            &rai_bin,
            &id_b,
            self.permission_mode,
            &self.prepend,
            &self.prompt_b,
            q,
        );

        eprintln!("rai claude pair: session A={id_a} session B={id_b}");

        let max_cycles = self.max_cycles.to_string();
        let max_hours = self.max_hours.to_string();
        let mut argv: Vec<&str> = vec![
            rai_bin.as_str(),
            "pair",
            "--command-a",
            cmd_a.as_str(),
            "--command-b",
            cmd_b.as_str(),
            "--max-cycles",
            max_cycles.as_str(),
            "--max-hours",
            max_hours.as_str(),
        ];
        if self.no_status_bar {
            argv.push("--no-status-bar");
        }

        let status = shell::user_shell_argv(&argv)
            .status()
            .with_context(|| format!("failed to spawn `rai pair` ({argv:?})"))?;
        let code = proc::shell_exit_code(&status);
        if code != 0 {
            std::process::exit(code);
        }
        Ok(())
    }
}

/// `rai claude print --session-id <UUID> [--permission-mode X] -- <PROMPT>`
/// 形のシェル文字列を組み立てる。プロンプトは `<prepend> <prompt>` (prepend が
/// 空なら本文だけ)。クォーティング関数 `q` は呼び出し側のシェル種別に応じて与える。
fn build_print_invocation(
    rai_bin: &str,
    session_id: &str,
    permission_mode: Option<PermissionMode>,
    prepend: &str,
    prompt: &str,
    q: fn(&str) -> String,
) -> String {
    let full_prompt = if prepend.is_empty() {
        prompt.to_string()
    } else {
        format!("{prepend} {prompt}")
    };
    let mut parts: Vec<String> = Vec::with_capacity(8);
    parts.push(q(rai_bin));
    parts.push("claude".into());
    parts.push("print".into());
    parts.push("--session-id".into());
    parts.push(q(session_id));
    if let Some(mode) = permission_mode {
        parts.push("--permission-mode".into());
        parts.push(mode.as_arg().to_string());
    }
    parts.push("--".into());
    parts.push(q(&full_prompt));
    parts.join(" ")
}

/// `/dev/urandom` から 16 byte 読んで RFC 4122 v4 形式の UUID 文字列を作る。
pub(crate) fn random_uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    let mut f = File::open("/dev/urandom").expect("open /dev/urandom");
    f.read_exact(&mut bytes).expect("read /dev/urandom");
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant RFC 4122
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn posix_q(s: &str) -> String {
        shell::quote_posix(s)
    }

    #[test]
    fn build_invocation_minimal() {
        let got = build_print_invocation(
            "/opt/rai/rai",
            "11111111-2222-3333-4444-555555555555",
            None,
            "/goal",
            "plan the migration",
            posix_q,
        );
        assert_eq!(
            got,
            "/opt/rai/rai claude print --session-id 11111111-2222-3333-4444-555555555555 -- '/goal plan the migration'"
        );
    }

    #[test]
    fn build_invocation_with_permission_mode() {
        let got = build_print_invocation(
            "rai",
            "abc",
            Some(PermissionMode::BypassPermissions),
            "/goal",
            "do it",
            posix_q,
        );
        assert_eq!(
            got,
            "rai claude print --session-id abc --permission-mode bypassPermissions -- '/goal do it'"
        );
    }

    #[test]
    fn build_invocation_empty_prepend_drops_prefix() {
        let got = build_print_invocation("rai", "abc", None, "", "raw prompt", posix_q);
        assert_eq!(got, "rai claude print --session-id abc -- 'raw prompt'");
    }

    #[test]
    fn build_invocation_quotes_shell_metas() {
        let got = build_print_invocation(
            "rai",
            "abc",
            None,
            "/goal",
            "echo $HOME && rm -rf /",
            posix_q,
        );
        assert_eq!(
            got,
            "rai claude print --session-id abc -- '/goal echo $HOME && rm -rf /'"
        );
    }

    #[test]
    fn random_uuid_v4_has_version_and_variant_bits() {
        for _ in 0..32 {
            let u = random_uuid_v4();
            assert_eq!(u.len(), 36, "got {u}");
            let bytes = u.as_bytes();
            assert_eq!(bytes[14] as char, '4', "version bit: {u}");
            let v = bytes[19] as char;
            assert!(matches!(v, '8' | '9' | 'a' | 'b'), "variant bit: {u}");
        }
    }
}
