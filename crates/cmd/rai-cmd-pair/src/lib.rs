//! `rai pair` — 2 つのコマンドを交互に回し続けるループ + 下部固定ステータスバー。
//!
//! 仕様: `docs/specs/01-pair.md` 参照。

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use clap::Args;
use rai_core::{
    cli::Run,
    proc, signals,
    term::{install_panic_restore, StatusBar},
    ts, Ctx, Result,
};

const EXIT_TIMED_OUT: i32 = 124;

#[derive(Debug, Args)]
pub struct Cmd {
    /// 1 サイクル目に実行するコマンド (A 側)。`<shell> -c "<cmd>"` で起動される。
    #[arg(long = "command-a", value_name = "CMD")]
    command_a: String,

    /// 1 サイクル目に実行するコマンド (B 側)。
    #[arg(long = "command-b", value_name = "CMD")]
    command_b: String,

    /// 最大サイクル数 (A→B で 1 サイクル)。
    #[arg(long = "max-cycles", default_value_t = 10)]
    max_cycles: u32,

    /// 累積最大実行時間 (時間)。0 で無制限。
    #[arg(long = "max-hours", default_value_t = 48)]
    max_hours: u32,

    /// 下部固定ステータスバーを無効化 (現行 fish 版互換モード)。
    #[arg(long = "no-status-bar")]
    no_status_bar: bool,

    /// 子コマンド実行用シェル。未指定時は $SHELL → /bin/sh。
    #[arg(long = "shell")]
    shell: Option<String>,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        // panic でもターミナル復元を最優先で
        install_panic_restore();

        let signal_slot = signals::install()?;
        let shell = self.shell.clone().unwrap_or_else(proc::default_shell);
        let timeout_bin = proc::find_timeout_bin();

        let mut bar = if self.no_status_bar {
            None
        } else {
            StatusBar::enable()?
        };

        let started_at = Instant::now();
        let max_seconds: u64 = u64::from(self.max_hours).saturating_mul(3600);

        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".to_string());
        ts::println(format!(
            "started cwd={cwd} cmd_a={:?} cmd_b={:?} max_cycles={} max_hours={}",
            self.command_a, self.command_b, self.max_cycles, self.max_hours,
        ));

        let mut exit_code: i32 = 0;

        'outer: for cycle in 1..=self.max_cycles {
            ts::println(format!("cycle {cycle}/{} started", self.max_cycles));

            for (label, cmd) in [("A", &self.command_a), ("B", &self.command_b)] {
                if signal_slot.load(Ordering::SeqCst) != 0 {
                    break 'outer;
                }

                let elapsed = started_at.elapsed().as_secs();
                let remaining = if max_seconds == 0 {
                    u64::MAX
                } else {
                    max_seconds.saturating_sub(elapsed)
                };
                if max_seconds != 0 && remaining == 0 {
                    ts::println("max-hours reached before command");
                    exit_code = EXIT_TIMED_OUT;
                    break 'outer;
                }

                ts::println(format!(
                    "command {label} starting cycle={cycle} elapsed={elapsed}s remaining={remaining}s cmd={cmd:?}"
                ));

                let cmd_started = Instant::now();
                let mut child =
                    spawn_child(&shell, cmd, timeout_bin.as_deref(), max_seconds, remaining)?;

                let status_int = wait_with_bar(
                    &mut child,
                    &mut bar,
                    &signal_slot,
                    cycle,
                    self.max_cycles,
                    label,
                    cmd,
                    started_at,
                    max_seconds,
                )?;

                // 子の終了直後はスクロール領域が壊れている可能性があるので再適用。
                if let Some(b) = bar.as_ref() {
                    let _ = b.apply_region();
                }

                let cmd_elapsed = cmd_started.elapsed().as_secs();
                ts::println(format!(
                    "command {label} exited status={status_int} cmd_elapsed={cmd_elapsed}s"
                ));

                if status_int != 0 {
                    if status_int == EXIT_TIMED_OUT {
                        ts::println("max-hours reached during command");
                    }
                    exit_code = status_int;
                    break 'outer;
                }
            }

            ts::println(format!("cycle {cycle}/{} completed", self.max_cycles));
        }

        // ステータスバーを明示的に drop して端末を戻す
        drop(bar);

        let sig = signal_slot.load(Ordering::SeqCst);
        if sig != 0 && exit_code == 0 {
            exit_code = signals::exit_code(sig);
        }

        if exit_code == 0 {
            ts::println("completed");
        }

        std::process::exit(exit_code);
    }
}

fn spawn_child(
    shell: &str,
    cmd: &str,
    timeout_bin: Option<&Path>,
    max_seconds: u64,
    remaining_secs: u64,
) -> Result<Child> {
    let mut command = match (timeout_bin, max_seconds) {
        (Some(bin), s) if s != 0 => {
            let mut c = Command::new(bin);
            c.arg(remaining_secs.to_string())
                .arg(shell)
                .arg("-c")
                .arg(cmd);
            c
        }
        _ => proc::shell_command(shell, cmd),
    };
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    Ok(command.spawn()?)
}

#[allow(clippy::too_many_arguments)]
fn wait_with_bar(
    child: &mut Child,
    bar: &mut Option<StatusBar>,
    signal_slot: &signals::SigSlot,
    cycle: u32,
    max_cycles: u32,
    label: &str,
    cmd: &str,
    started_at: Instant,
    max_seconds: u64,
) -> Result<i32> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(proc::shell_exit_code(&status));
        }

        if signal_slot.load(Ordering::SeqCst) != 0 {
            #[cfg(unix)]
            // SAFETY: child.id() returns the live PID we just polled. SIGTERM is best-effort.
            unsafe {
                libc::kill(child.id() as i32, libc::SIGTERM);
            }
            let status = child.wait()?;
            return Ok(proc::shell_exit_code(&status));
        }

        if let Some(b) = bar.as_mut() {
            let elapsed = started_at.elapsed().as_secs();
            let remaining_str = if max_seconds == 0 {
                "∞".to_string()
            } else {
                format!("{}s", max_seconds.saturating_sub(elapsed))
            };
            let line = format!(
                " cycle {cycle}/{max_cycles} | {label} running | elapsed={elapsed}s remaining={remaining_str} | {cmd} "
            );
            let _ = b.draw(&line);
        }

        std::thread::sleep(Duration::from_millis(1000));
    }
}
