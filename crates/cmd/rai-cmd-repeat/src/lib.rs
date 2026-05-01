//! `rai repeat` — run a shell command repeatedly under count / duration limits.
//!
//! The command body is passed verbatim to the user's login shell as a single
//! `$SHELL -c <CMD>` invocation, so fish functions / zsh aliases / etc. work
//! the same way they do when typed interactively.

use std::process::ExitStatus;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context as _};
use clap::Args;
use rai_core::{cli::Run, shell, Ctx, Result};

/// `rai repeat [OPTIONS] <COMMAND>`
#[derive(Debug, Args)]
#[command(group(
    clap::ArgGroup::new("limit")
        .required(true)
        .multiple(true)
        .args(["count", "duration"]),
))]
pub struct Cmd {
    /// Maximum number of iterations (must be >= 1). Stops as soon as this many
    /// runs have completed successfully. Combined with `--duration` as OR.
    #[arg(short = 'n', long, value_name = "N")]
    count: Option<u32>,

    /// Maximum elapsed wall-clock time from the start of the first iteration.
    /// Accepts forms like `30s`, `5m`, `1h30m`, `500ms`. Combined with
    /// `--count` as OR.
    #[arg(short = 'd', long, value_name = "DURATION", value_parser = parse_duration)]
    duration: Option<Duration>,

    /// Sleep this long between iterations. Same format as `--duration`.
    #[arg(short = 'i', long, value_name = "DURATION", value_parser = parse_duration)]
    interval: Option<Duration>,

    /// Shell command to run on each iteration. Passed to `$SHELL -c` as-is.
    #[arg(value_name = "COMMAND")]
    command: String,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        if matches!(self.count, Some(0)) {
            bail!("--count must be at least 1");
        }
        if self.command.trim().is_empty() {
            bail!("<COMMAND> must not be empty");
        }

        let started = Instant::now();
        let mut iter: u32 = 0;
        loop {
            iter += 1;
            let elapsed = started.elapsed();
            eprintln!(
                "rai repeat: iteration {iter} (elapsed={:.1}s)",
                elapsed.as_secs_f64()
            );

            let status = run_once(&self.command)?;
            if !status.success() {
                let code = exit_code_of(status);
                eprintln!("rai repeat: command failed with exit code {code}; stopping");
                std::process::exit(code);
            }

            if let Some(max) = self.count {
                if iter >= max {
                    return Ok(());
                }
            }
            if let Some(limit) = self.duration {
                if started.elapsed() >= limit {
                    return Ok(());
                }
            }

            if let Some(gap) = self.interval {
                if let Some(remaining) = remaining_budget(self.duration, started.elapsed()) {
                    let nap = gap.min(remaining);
                    if nap.is_zero() {
                        return Ok(());
                    }
                    thread::sleep(nap);
                } else {
                    thread::sleep(gap);
                }
            }

            if let Some(limit) = self.duration {
                if started.elapsed() >= limit {
                    return Ok(());
                }
            }
        }
    }
}

fn run_once(cmd: &str) -> Result<ExitStatus> {
    shell::user_shell_command(cmd)
        .status()
        .with_context(|| format!("failed to spawn shell command: {cmd}"))
}

/// Returns the remaining time budget if `--duration` is set, otherwise None.
fn remaining_budget(limit: Option<Duration>, elapsed: Duration) -> Option<Duration> {
    limit.map(|l| l.saturating_sub(elapsed))
}

fn exit_code_of(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        code
    } else {
        // Killed by signal on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(sig) = status.signal() {
                return 128 + sig;
            }
        }
        1
    }
}

/// Parse durations like `30s`, `5m`, `1h30m`, `500ms`, `2d`. Bare numbers are
/// rejected on purpose — the unit must be explicit.
fn parse_duration(input: &str) -> std::result::Result<Duration, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("duration must not be empty".to_string());
    }

    let mut total = Duration::ZERO;
    let mut bytes = s.as_bytes();
    let mut saw_unit = false;

    while !bytes.is_empty() {
        let num_end = bytes
            .iter()
            .position(|b| !b.is_ascii_digit())
            .ok_or_else(|| format!("duration `{input}` must include a unit (e.g. 30s)"))?;
        if num_end == 0 {
            return Err(format!("duration `{input}` must start with a number"));
        }
        let num_str = std::str::from_utf8(&bytes[..num_end]).expect("ascii digits");
        let value: u64 = num_str
            .parse()
            .map_err(|e| format!("invalid number in duration `{input}`: {e}"))?;

        let rest = &bytes[num_end..];
        let (unit_len, multiplier_ms) = match rest {
            [b'm', b's', ..] => (2, 1_u64),
            [b's', ..] => (1, 1_000),
            [b'm', ..] => (1, 60 * 1_000),
            [b'h', ..] => (1, 60 * 60 * 1_000),
            [b'd', ..] => (1, 24 * 60 * 60 * 1_000),
            _ => {
                return Err(format!(
                    "unknown unit in duration `{input}` (expected ms/s/m/h/d)"
                ))
            }
        };

        let ms = value
            .checked_mul(multiplier_ms)
            .ok_or_else(|| format!("duration `{input}` overflows"))?;
        total = total
            .checked_add(Duration::from_millis(ms))
            .ok_or_else(|| format!("duration `{input}` overflows"))?;
        saw_unit = true;
        bytes = &rest[unit_len..];
    }

    if !saw_unit {
        return Err(format!("duration `{input}` must include a unit"));
    }
    if total.is_zero() {
        return Err(format!("duration `{input}` must be greater than zero"));
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> Duration {
        parse_duration(s).unwrap_or_else(|e| panic!("parse {s}: {e}"))
    }

    #[test]
    fn parse_simple_units() {
        assert_eq!(ok("500ms"), Duration::from_millis(500));
        assert_eq!(ok("30s"), Duration::from_secs(30));
        assert_eq!(ok("5m"), Duration::from_secs(5 * 60));
        assert_eq!(ok("2h"), Duration::from_secs(2 * 60 * 60));
        assert_eq!(ok("1d"), Duration::from_secs(24 * 60 * 60));
    }

    #[test]
    fn parse_compound() {
        assert_eq!(
            ok("1h30m"),
            Duration::from_secs(60 * 60) + Duration::from_secs(30 * 60)
        );
        assert_eq!(
            ok("2m500ms"),
            Duration::from_secs(120) + Duration::from_millis(500)
        );
    }

    #[test]
    fn parse_rejects_bare_number() {
        assert!(parse_duration("30").is_err());
    }

    #[test]
    fn parse_rejects_unknown_unit() {
        assert!(parse_duration("30x").is_err());
        assert!(parse_duration("1xz").is_err());
    }

    #[test]
    fn parse_rejects_zero() {
        assert!(parse_duration("0s").is_err());
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn remaining_budget_caps() {
        assert_eq!(
            remaining_budget(Some(Duration::from_secs(10)), Duration::from_secs(3)),
            Some(Duration::from_secs(7))
        );
        assert_eq!(
            remaining_budget(Some(Duration::from_secs(10)), Duration::from_secs(20)),
            Some(Duration::ZERO)
        );
        assert_eq!(remaining_budget(None, Duration::from_secs(3)), None);
    }
}
