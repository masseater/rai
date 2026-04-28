//! `rai completion` — emit a shell completion script to stdout.
//!
//! The completion definition is generated from the top-level `clap::Command`
//! of the `rai` binary, so it always reflects the current set of subcommands
//! and options without any hand-maintained completion table.

use clap::{Args, Command};
use clap_complete::{generate, Shell};
use rai_core::Result;
use std::io::{self, Write};

#[derive(Debug, Args)]
pub struct Cmd {
    /// Target shell.
    #[arg(value_enum)]
    shell: Shell,

    /// Emit a shell startup snippet that reloads completions from the current `rai`.
    #[arg(long)]
    source: bool,
}

impl Cmd {
    /// Write the completion script for the requested shell to stdout.
    ///
    /// `cmd` must be the top-level `clap::Command` of the binary (typically
    /// `Cli::command()` from `crates/rai/src/main.rs`).
    pub fn print(self, cmd: &mut Command) -> Result<()> {
        if self.source {
            writeln!(io::stdout(), "{}", source_snippet(self.shell))?;
            return Ok(());
        }

        let bin = cmd.get_name().to_string();
        generate(self.shell, cmd, bin, &mut io::stdout());
        Ok(())
    }
}

fn source_snippet(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => "source <(rai completion bash)",
        Shell::Elvish => "eval (rai completion elvish | slurp)",
        Shell::Fish => "rai completion fish | source",
        Shell::PowerShell => "rai completion powershell | Out-String | Invoke-Expression",
        Shell::Zsh => {
            "autoload -Uz compinit\nif ! typeset -f compdef >/dev/null; then\n  compinit\nfi\nsource <(rai completion zsh)"
        }
        _ => unreachable!("clap_complete::Shell gained an unsupported variant"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_snippets_reload_current_rai_binary() {
        for shell in [
            Shell::Bash,
            Shell::Elvish,
            Shell::Fish,
            Shell::PowerShell,
            Shell::Zsh,
        ] {
            let snippet = source_snippet(shell);
            assert!(snippet.contains("rai completion"));
            assert!(snippet.contains(&shell.to_string()));
        }
    }
}
