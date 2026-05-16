//! `rai claude pair` — 2 つのプロンプト (A / B) を交互に `claude --print` で回す。
//!
//! 仕様: `docs/specs/22-claude-pair.md` 参照。
//!
//! 内部実装は `rai pair` を **子プロセス** として呼ぶ。`rai-cmd-pair` を直接 link
//! しないのはワークスペースの「Subcommand crate どうしを横断して depend しない」
//! ポリシーに従うため (`AGENTS.md` の Important Instructions 参照)。

use anyhow::{anyhow, bail, Context as _};
use clap::Args;
use rai_core::{cli::Run, proc, shell, Ctx, Result};
use uuid::Uuid;

use crate::print::{OutputFormat, PermissionMode};

const LONG_ABOUT: &str = "\
2 つのプロンプト (A / B) を 1 サイクルとして `claude --print` で交互に回し続ける。

各サイクルで、A 用 / B 用に **1 度だけ生成した UUID** を `rai claude print` の
`--session-id` に与え、2 回目以降は自動で `--resume` に切り替わる。これにより
A 会話 / B 会話はそれぞれ独立して文脈を蓄積していく。

各プロンプトの先頭には既定で `/goal ` を自動付与する。`/goal` はユーザーの
claude 設定 (skill / slash command) 側で定義しておく前提。未定義の環境で使う
場合は `--prepend ''` で無効化するか、`--prepend <別の slash command>` で
差し替える。

ループ本体は `rai pair` を子プロセスとして呼ぶので、status bar / `--max-cycles`
/ `--max-hours` / SIGINT ハンドリング / 端末復元はそのまま継承される。";

#[derive(Debug, Args)]
#[command(long_about = LONG_ABOUT)]
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

    /// `rai claude print` 経由で claude にパススルーする output format。
    /// `stream-json` を選ぶ場合は `--claude-verbose` も必須。
    #[arg(long = "output-format", value_name = "FMT")]
    output_format: Option<OutputFormat>,

    /// `rai claude print` 経由で claude の `--verbose` を有効化 (stream-json 併用に必要)。
    #[arg(long = "claude-verbose")]
    claude_verbose: bool,

    /// A 用の session-id。未指定なら新規 UUID を 1 度だけ生成する。
    #[arg(long = "id-a", value_name = "UUID")]
    id_a: Option<String>,

    /// B 用の session-id。未指定なら新規 UUID を 1 度だけ生成する。
    #[arg(long = "id-b", value_name = "UUID")]
    id_b: Option<String>,

    /// 各プロンプトの先頭に付与する文字列 (空文字なら付与しない)。
    /// 既定の `/goal` はユーザーの claude 設定で定義されている前提。
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
        // print 側と同じ pre-validation。pair の場合は session id 生成 / eprintln /
        // rai pair の spawn など副作用が走った後に最初の `rai claude print` で死ぬのを
        // 避けたい。print に揃えて早期エラーする。
        if matches!(self.output_format, Some(OutputFormat::StreamJson)) && !self.claude_verbose {
            bail!("--output-format stream-json requires --claude-verbose (claude rejects this combination)");
        }

        let id_a = self
            .id_a
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let id_b = self
            .id_b
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        // current_exe() の失敗は通常起き得ない異常事態。テスト経路では `--rai-bin`
        // で明示できるので、フォールバックの `"rai"` 推測は廃止し、わかりやすい
        // エラーで停止する (spec 22 の意図)。
        let rai_bin = match self.rai_bin.clone() {
            Some(p) => p,
            None => std::env::current_exe()
                .map_err(|e| {
                    anyhow!(
                        "failed to resolve current rai executable (current_exe(): {e}). \
                     pass --rai-bin <PATH> to override."
                    )
                })?
                .display()
                .to_string(),
        };

        let (_shell_path, shell_kind) = shell::detect_user_shell();
        let q = shell::quote_for(shell_kind);

        let opts = PrintOpts {
            permission_mode: self.permission_mode,
            output_format: self.output_format,
            claude_verbose: self.claude_verbose,
        };
        let cmd_a =
            build_print_invocation(&rai_bin, &id_a, &opts, &self.prepend, &self.prompt_a, q);
        let cmd_b =
            build_print_invocation(&rai_bin, &id_b, &opts, &self.prepend, &self.prompt_b, q);

        eprintln!("rai claude pair: session A={id_a} session B={id_b}");

        // `cmd_a` / `cmd_b` は既にシェルコマンド文字列なので、これを `rai pair`
        // の `--command-a` / `--command-b` 値として **1 引数のまま** 渡したい。
        // `develop::common::launch` と同じパターン: argv を `user_shell_command`
        // 用に手でクォートして 1 行のシェルコマンドに組み立てる。
        let max_cycles = self.max_cycles.to_string();
        let max_hours = self.max_hours.to_string();
        let mut parts: Vec<String> = vec![
            q(&rai_bin),
            "pair".to_string(),
            "--command-a".to_string(),
            q(&cmd_a),
            "--command-b".to_string(),
            q(&cmd_b),
            "--max-cycles".to_string(),
            max_cycles,
            "--max-hours".to_string(),
            max_hours,
        ];
        if self.no_status_bar {
            parts.push("--no-status-bar".to_string());
        }
        let full_cmd = parts.join(" ");

        let status = shell::user_shell_command(&full_cmd)
            .status()
            .with_context(|| format!("failed to spawn `rai pair`: {full_cmd}"))?;
        let code = proc::shell_exit_code(&status);
        if code != 0 {
            std::process::exit(code);
        }
        Ok(())
    }
}

/// `rai claude print` 起動時に乗せる claude 系オプション一式。
#[derive(Debug, Clone, Copy, Default)]
struct PrintOpts {
    permission_mode: Option<PermissionMode>,
    output_format: Option<OutputFormat>,
    claude_verbose: bool,
}

/// `rai claude print --session-id <UUID> [...] -- <PROMPT>` 形のシェル文字列を組み立てる。
fn build_print_invocation(
    rai_bin: &str,
    session_id: &str,
    opts: &PrintOpts,
    prepend: &str,
    prompt: &str,
    q: fn(&str) -> String,
) -> String {
    let full_prompt = if prepend.is_empty() {
        prompt.to_string()
    } else {
        format!("{prepend} {prompt}")
    };
    let mut parts: Vec<String> = Vec::with_capacity(12);
    parts.push(q(rai_bin));
    parts.push("claude".into());
    parts.push("print".into());
    parts.push("--session-id".into());
    parts.push(q(session_id));
    if let Some(mode) = opts.permission_mode {
        parts.push("--permission-mode".into());
        parts.push(mode.as_arg().to_string());
    }
    if let Some(fmt) = opts.output_format {
        parts.push("--output-format".into());
        parts.push(fmt.as_arg().to_string());
    }
    if opts.claude_verbose {
        parts.push("--claude-verbose".into());
    }
    parts.push("--".into());
    parts.push(q(&full_prompt));
    parts.join(" ")
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
            &PrintOpts::default(),
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
        let opts = PrintOpts {
            permission_mode: Some(PermissionMode::BypassPermissions),
            ..PrintOpts::default()
        };
        let got = build_print_invocation("rai", "abc", &opts, "/goal", "do it", posix_q);
        assert_eq!(
            got,
            "rai claude print --session-id abc --permission-mode bypassPermissions -- '/goal do it'"
        );
    }

    #[test]
    fn build_invocation_with_stream_json_and_verbose() {
        let opts = PrintOpts {
            output_format: Some(OutputFormat::StreamJson),
            claude_verbose: true,
            ..PrintOpts::default()
        };
        let got = build_print_invocation("rai", "abc", &opts, "/goal", "x", posix_q);
        assert_eq!(
            got,
            "rai claude print --session-id abc --output-format stream-json --claude-verbose -- '/goal x'"
        );
    }

    #[test]
    fn build_invocation_empty_prepend_drops_prefix() {
        let got = build_print_invocation(
            "rai",
            "abc",
            &PrintOpts::default(),
            "",
            "raw prompt",
            posix_q,
        );
        assert_eq!(got, "rai claude print --session-id abc -- 'raw prompt'");
    }

    #[test]
    fn build_invocation_quotes_shell_metas() {
        let got = build_print_invocation(
            "rai",
            "abc",
            &PrintOpts::default(),
            "/goal",
            "echo $HOME && rm -rf /",
            posix_q,
        );
        assert_eq!(
            got,
            "rai claude print --session-id abc -- '/goal echo $HOME && rm -rf /'"
        );
    }
}
