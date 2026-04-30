//! `rai doctor` — diagnose whether the external CLIs that `rai` depends on are
//! installed and reachable on `PATH`.

use std::path::PathBuf;
use std::process::Command;

use anyhow::bail;
use clap::Args;
use rai_core::{cli::Run, Ctx, Result};

/// Tools that `rai` shells out to. Adding a new external dependency anywhere in
/// the workspace? Add it here too so `rai doctor` can verify it.
const REQUIRED_TOOLS: &[Tool] = &[
    Tool::new("git", "--version"),
    Tool::new("gh", "--version"),
    Tool::new("gwq", "--version"),
    Tool::new("tmux", "-V"),
    Tool::new("fzf", "--version"),
    Tool::new("claude", "--version"),
    Tool::new("ccs", "--version"),
    Tool::new("tee", "--version"),
];

#[derive(Debug, Args)]
pub struct Cmd {}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let report = run_doctor(REQUIRED_TOOLS, &Env::detect());
        print!("{}", report.render());
        if report.has_missing() {
            bail!("doctor found {} missing tool(s)", report.missing_count());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct Tool {
    name: &'static str,
    version_arg: &'static str,
}

impl Tool {
    const fn new(name: &'static str, version_arg: &'static str) -> Self {
        Self { name, version_arg }
    }
}

#[derive(Debug)]
struct Env {
    shell: String,
}

impl Env {
    fn detect() -> Self {
        Self {
            shell: std::env::var("SHELL").unwrap_or_else(|_| "(unset)".to_string()),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ToolStatus {
    name: &'static str,
    found: Option<String>,
}

#[derive(Debug)]
struct Report {
    shell: String,
    statuses: Vec<ToolStatus>,
}

impl Report {
    fn has_missing(&self) -> bool {
        self.statuses.iter().any(|s| s.found.is_none())
    }

    fn missing_count(&self) -> usize {
        self.statuses.iter().filter(|s| s.found.is_none()).count()
    }

    fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("shell: {}\n", self.shell));
        let name_width = self
            .statuses
            .iter()
            .map(|s| s.name.len())
            .max()
            .unwrap_or(0);
        for s in &self.statuses {
            let pad = " ".repeat(name_width.saturating_sub(s.name.len()));
            match &s.found {
                Some(v) => out.push_str(&format!("  {}{}  ok       {}\n", s.name, pad, v)),
                None => out.push_str(&format!("  {}{}  missing\n", s.name, pad)),
            }
        }
        out
    }
}

fn run_doctor(tools: &[Tool], env: &Env) -> Report {
    let statuses = tools.iter().map(|t| probe_tool(*t)).collect();
    Report {
        shell: env.shell.clone(),
        statuses,
    }
}

fn probe_tool(tool: Tool) -> ToolStatus {
    if find_in_path(tool.name).is_none() {
        return ToolStatus {
            name: tool.name,
            found: None,
        };
    }
    let version = Command::new(tool.name)
        .arg(tool.version_arg)
        .output()
        .ok()
        .and_then(|out| {
            if !out.status.success() {
                return None;
            }
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            let combined = if !stdout.trim().is_empty() {
                stdout
            } else {
                stderr
            };
            let line = first_line(&combined);
            if line.is_empty() {
                None
            } else {
                Some(line)
            }
        })
        .unwrap_or_else(|| "(installed)".to_string());
    ToolStatus {
        name: tool.name,
        found: Some(version),
    }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

#[allow(dead_code)]
fn shell_basename(shell: &str) -> String {
    PathBuf::from(shell)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{first_line, shell_basename, Report, ToolStatus};

    fn make_report() -> Report {
        Report {
            shell: "/opt/homebrew/bin/fish".to_string(),
            statuses: vec![
                ToolStatus {
                    name: "git",
                    found: Some("git version 2.45.0".to_string()),
                },
                ToolStatus {
                    name: "gh",
                    found: None,
                },
                ToolStatus {
                    name: "tmux",
                    found: Some("tmux 3.4".to_string()),
                },
            ],
        }
    }

    #[test]
    fn render_lists_each_tool_status_with_aligned_names() {
        let body = make_report().render();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines[0], "shell: /opt/homebrew/bin/fish");
        assert!(lines[1].starts_with("  git "));
        assert!(lines[1].contains("ok"));
        assert!(lines[1].ends_with("git version 2.45.0"));
        assert!(lines[2].contains("gh  "));
        assert!(lines[2].trim_end().ends_with("missing"));
        assert!(lines[3].starts_with("  tmux"));
    }

    #[test]
    fn has_missing_returns_true_when_any_tool_unfound() {
        let r = make_report();
        assert!(r.has_missing());
        assert_eq!(r.missing_count(), 1);
    }

    #[test]
    fn has_missing_returns_false_when_everything_found() {
        let r = Report {
            shell: "/bin/zsh".to_string(),
            statuses: vec![ToolStatus {
                name: "git",
                found: Some("git version 2.45.0".to_string()),
            }],
        };
        assert!(!r.has_missing());
        assert_eq!(r.missing_count(), 0);
    }

    #[test]
    fn first_line_strips_trailing_lines() {
        assert_eq!(
            first_line("git version 2.45.0\nfoo\n"),
            "git version 2.45.0"
        );
        assert_eq!(first_line(""), "");
        assert_eq!(first_line("   only-line   "), "only-line");
    }

    #[test]
    fn shell_basename_extracts_file_name() {
        assert_eq!(shell_basename("/opt/homebrew/bin/fish"), "fish");
        assert_eq!(shell_basename("/bin/zsh"), "zsh");
        assert_eq!(shell_basename(""), "");
    }
}
