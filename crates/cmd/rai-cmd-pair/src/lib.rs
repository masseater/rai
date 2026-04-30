//! `rai pair` — 2 つのコマンドを交互に回し続けるループ + 下部固定ステータスバー。
//!
//! 仕様: `docs/specs/01-pair.md` 参照。

use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use clap::Args;
use rai_core::{
    cli::Run,
    proc, shell, signals,
    term::{install_panic_restore, StatusBar},
    ts, Ctx, Result,
};

const EXIT_TIMED_OUT: i32 = 124;
const STATUS_LINES: u16 = 4;

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
        let shell = self.shell.clone().unwrap_or_else(shell::user_shell_path);
        let timeout_bin = proc::find_timeout_bin();

        let mut bar = if self.no_status_bar {
            None
        } else {
            StatusBar::enable(STATUS_LINES)?
        };

        let started_at = Instant::now();
        let max_seconds: u64 = u64::from(self.max_hours).saturating_mul(3600);

        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".to_string());
        log(
            &mut bar,
            format!(
                "started cwd={cwd} cmd_a={:?} cmd_b={:?} max_cycles={} max_hours={}",
                self.command_a, self.command_b, self.max_cycles, self.max_hours,
            ),
        );

        let mut exit_code: i32 = 0;

        'outer: for cycle in 1..=self.max_cycles {
            log(
                &mut bar,
                format!("cycle {cycle}/{} started", self.max_cycles),
            );

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
                    log(&mut bar, "max-hours reached before command");
                    exit_code = EXIT_TIMED_OUT;
                    break 'outer;
                }

                log(
                    &mut bar,
                    format!(
                        "command {label} starting cycle={cycle} elapsed={elapsed}s remaining={remaining}s cmd={cmd:?}"
                    ),
                );

                if let Some(b) = bar.as_mut() {
                    let _ = draw_status(
                        b,
                        cycle,
                        self.max_cycles,
                        label,
                        &self.command_a,
                        &self.command_b,
                        &cwd,
                        started_at,
                        max_seconds,
                    );
                }

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
                    &self.command_a,
                    &self.command_b,
                    &cwd,
                    started_at,
                    max_seconds,
                )?;

                // 子の終了直後はスクロール領域が壊れている可能性があるので再適用。
                if let Some(b) = bar.as_mut() {
                    let _ = b.resume();
                }

                let cmd_elapsed = cmd_started.elapsed().as_secs();
                log(
                    &mut bar,
                    format!(
                        "command {label} exited status={status_int} cmd_elapsed={cmd_elapsed}s"
                    ),
                );

                if status_int != 0 {
                    if status_int == EXIT_TIMED_OUT {
                        log(&mut bar, "max-hours reached during command");
                    }
                    exit_code = status_int;
                    break 'outer;
                }
            }

            log(
                &mut bar,
                format!("cycle {cycle}/{} completed", self.max_cycles),
            );
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

fn log(bar: &mut Option<StatusBar>, msg: impl AsRef<str>) {
    if let Some(b) = bar.as_ref() {
        let _ = b.prepare_output();
    }
    ts::println(msg);
}

fn spawn_child(
    inner_shell: &str,
    cmd: &str,
    timeout_bin: Option<&Path>,
    max_seconds: u64,
    remaining_secs: u64,
) -> Result<Child> {
    let user_shell_path = shell::user_shell_path();
    let kind = shell::detect_shell_kind(&user_shell_path);
    let q = shell::quote_for(kind);

    // 内側: `<inner_shell> -c <cmd>` を 1 引数として組み立てる。
    let inner = format!("{} -c {}", q(inner_shell), q(cmd));

    // 外側にユーザーシェルを噛ませる。timeout がある場合は `<timeout_bin> <secs> <inner...>`
    // を外側シェルの -c 引数として渡す。timeout バイナリ自体もユーザーシェル経由で
    // 解決させる (alias / function でも動く)。
    let outer_cmd = match (timeout_bin, max_seconds) {
        (Some(bin), s) if s != 0 => {
            format!(
                "{} {} {}",
                q(&bin.display().to_string()),
                remaining_secs,
                inner
            )
        }
        _ => inner,
    };

    let mut command = shell::shell_command(&user_shell_path, &outer_cmd);
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
    command_a: &str,
    command_b: &str,
    cwd: &str,
    started_at: Instant,
    max_seconds: u64,
) -> Result<i32> {
    let mut next_draw = Instant::now();
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

        let now = Instant::now();
        if now >= next_draw {
            if let Some(b) = bar.as_mut() {
                let _ = draw_status(
                    b,
                    cycle,
                    max_cycles,
                    label,
                    command_a,
                    command_b,
                    cwd,
                    started_at,
                    max_seconds,
                );
            }
            next_draw = now + Duration::from_secs(1);
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_status(
    bar: &mut StatusBar,
    cycle: u32,
    max_cycles: u32,
    label: &str,
    command_a: &str,
    command_b: &str,
    cwd: &str,
    started_at: Instant,
    max_seconds: u64,
) -> std::io::Result<()> {
    let elapsed = started_at.elapsed().as_secs();
    let remaining_str = if max_seconds == 0 {
        "∞".to_string()
    } else {
        format!("{}s", max_seconds.saturating_sub(elapsed))
    };
    let mark_a = if label == "A" { "▶" } else { " " };
    let mark_b = if label == "B" { "▶" } else { " " };
    let line0 = format!(" cwd: {cwd} ");
    let line1 =
        format!(" cycle {cycle}/{max_cycles} | elapsed={elapsed}s remaining={remaining_str} ");
    let line2 = format!("{mark_a} command-a: {command_a} ");
    let line3 = format!("{mark_b} command-b: {command_b} ");
    bar.draw(&[&line0, &line1, &line2, &line3])
}
