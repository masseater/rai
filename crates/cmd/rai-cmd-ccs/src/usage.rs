//! `rai ccs usage` — ccs の全 Claude プロファイルについて Anthropic 側の
//! 5h / 7d レートリミット枠を 1 つの表にまとめて表示する。
//!
//! 仕様: `docs/specs/23-ccs-usage.md` 参照。

use std::fs;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context as _};
use chrono::{DateTime, Local, TimeZone, Utc};
use clap::Args;
use rai_core::{cli::Run, shell, Ctx, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const USAGE_ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";

/// `rai ccs usage [OPTIONS]`
#[derive(Debug, Args)]
pub struct Cmd {
    /// 対象プロファイル名 (繰り返し可)。未指定なら ccs の type=="account" を全件。
    #[arg(long = "profile", value_name = "NAME", action = clap::ArgAction::Append)]
    profile: Vec<String>,

    /// 機械可読 JSON を stdout に出す。
    #[arg(long, conflicts_with = "watch")]
    json: bool,

    /// 指定秒ごとに再取得して同じ画面を更新する。Ctrl-C で抜ける。
    #[arg(long, value_name = "SECS", num_args = 0..=1, default_missing_value = "60")]
    watch: Option<u64>,

    /// 1 プロファイルあたりの HTTP タイムアウト秒数。
    #[arg(long, value_name = "SECS", default_value_t = 8)]
    timeout: u64,

    /// ccs 実行ファイルを差し替える (テスト用)。
    #[arg(long = "ccs-bin", value_name = "PATH", default_value = "ccs")]
    ccs_bin: String,
}

impl Run for Cmd {
    fn run(self, _ctx: &Ctx) -> Result<()> {
        let client = UreqClient::new(Duration::from_secs(self.timeout));
        match self.watch {
            Some(sec) if sec > 0 => loop {
                let snap = collect(&self.ccs_bin, &self.profile, &client)?;
                print!("\x1b[2J\x1b[H");
                render_table(&snap, io::stdout().is_terminal());
                thread::sleep(Duration::from_secs(sec));
            },
            _ => {
                let snap = collect(&self.ccs_bin, &self.profile, &client)?;
                if self.json {
                    println!("{}", serde_json::to_string_pretty(&snap.to_json())?);
                } else {
                    render_table(&snap, io::stdout().is_terminal());
                }
                if snap.has_fatal_error() {
                    std::process::exit(1);
                }
                Ok(())
            }
        }
    }
}

/// 1 プロファイル分の取得結果。
#[derive(Debug, Clone)]
pub struct ProfileUsage {
    pub name: String,
    pub is_default: bool,
    pub tier: Option<String>,
    pub five_hour: Option<RateWindow>,
    pub seven_day: Option<RateWindow>,
    pub error: Option<ProfileError>,
}

#[derive(Debug, Clone, Copy)]
pub struct RateWindow {
    pub used_percentage: f64,
    pub resets_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    NoCredentials,
    Expired,
    AuthFailed,
    Timeout,
    Http(String),
}

impl ProfileError {
    fn note(&self) -> String {
        match self {
            Self::NoCredentials => "no credentials".to_string(),
            Self::Expired => "expired — run ccs <profile>".to_string(),
            Self::AuthFailed => "auth failed".to_string(),
            Self::Timeout => "timeout".to_string(),
            Self::Http(msg) => format!("error: {msg}"),
        }
    }

    fn as_json_str(&self) -> &'static str {
        match self {
            Self::NoCredentials => "no_credentials",
            Self::Expired => "expired",
            Self::AuthFailed => "auth_failed",
            Self::Timeout => "timeout",
            Self::Http(_) => "http_error",
        }
    }

    /// 全体 exit code を非 0 に倒すべきエラーか。`NoCredentials` 単独 (= ccs に登録は
    /// されているが credentials ファイル未作成) は致命扱いするが、表示は止めない。
    fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::Expired | Self::AuthFailed | Self::Timeout | Self::Http(_) | Self::NoCredentials
        )
    }
}

#[derive(Debug)]
pub struct Snapshot {
    pub fetched_at: DateTime<Utc>,
    pub profiles: Vec<ProfileUsage>,
}

impl Snapshot {
    fn has_fatal_error(&self) -> bool {
        self.profiles
            .iter()
            .any(|p| p.error.as_ref().is_some_and(ProfileError::is_fatal))
    }

    fn to_json(&self) -> serde_json::Value {
        let profiles: Vec<serde_json::Value> = self
            .profiles
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "is_default": p.is_default,
                    "tier": p.tier,
                    "five_hour": p.five_hour.map(|w| serde_json::json!({
                        "used_percentage": w.used_percentage,
                        "resets_at": w.resets_at,
                    })),
                    "seven_day": p.seven_day.map(|w| serde_json::json!({
                        "used_percentage": w.used_percentage,
                        "resets_at": w.resets_at,
                    })),
                    "error": p.error.as_ref().map(ProfileError::as_json_str),
                })
            })
            .collect();
        serde_json::json!({
            "fetched_at": self.fetched_at.to_rfc3339(),
            "profiles": profiles,
        })
    }
}

// ---- ccs auth list --json ----------------------------------------------------

#[derive(Debug, Deserialize)]
struct CcsAuthList {
    profiles: Vec<CcsProfile>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CcsProfile {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub is_default: bool,
    pub instance_path: String,
}

pub fn parse_ccs_auth_list(bytes: &[u8]) -> Result<Vec<CcsProfile>> {
    let parsed: CcsAuthList = serde_json::from_slice(bytes)
        .context("failed to parse `ccs auth list --json` output as JSON")?;
    Ok(parsed
        .profiles
        .into_iter()
        .filter(|p| p.kind == "account")
        .collect())
}

fn fetch_ccs_profiles(ccs_bin: &str) -> Result<Vec<CcsProfile>> {
    let out = shell::user_shell_argv(&[ccs_bin, "auth", "list", "--json"])
        .output()
        .with_context(|| format!("failed to spawn `{ccs_bin} auth list --json`"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!(
            "`{ccs_bin} auth list --json` failed (status {:?}): {}",
            out.status.code(),
            stderr.trim()
        );
    }
    parse_ccs_auth_list(&out.stdout)
}

// ---- credentials -------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<ClaudeAiOauth>,
}

#[derive(Debug, Deserialize, Clone)]
struct ClaudeAiOauth {
    #[serde(rename = "accessToken")]
    access_token: String,
    /// epoch milliseconds.
    #[serde(rename = "expiresAt", default)]
    expires_at: Option<i64>,
    #[serde(rename = "rateLimitTier", default)]
    rate_limit_tier: Option<String>,
}

/// expiresAt (epoch ms) が `now_ms` (epoch ms) 以下なら expired。
pub fn is_expired(expires_at_ms: Option<i64>, now_ms: i64) -> bool {
    matches!(expires_at_ms, Some(exp) if exp <= now_ms)
}

/// macOS keychain の service 名: `Claude Code-credentials-<sha256(instance_path)[0..8]>`。
/// Claude Code 本体が `CLAUDE_CONFIG_DIR` を sha256 して頭 8 hex を suffix に使う実装に
/// 揃えている。
pub fn keychain_service_name(instance_path: &str) -> String {
    use std::fmt::Write as _;
    let mut h = Sha256::new();
    h.update(instance_path.as_bytes());
    let digest = h.finalize();
    let mut hex = String::with_capacity(8);
    for b in digest.iter().take(4) {
        let _ = write!(&mut hex, "{b:02x}");
    }
    format!("Claude Code-credentials-{hex}")
}

/// keychain から得た文字列を `ClaudeAiOauth` にパース。
fn parse_credentials_str(raw: &str) -> std::result::Result<ClaudeAiOauth, ProfileError> {
    let parsed: CredentialsFile = serde_json::from_str(raw.trim())
        .map_err(|e| ProfileError::Http(format!("parse credentials failed: {e}")))?;
    parsed.claude_ai_oauth.ok_or(ProfileError::NoCredentials)
}

fn read_credentials(instance_path: &str) -> std::result::Result<ClaudeAiOauth, ProfileError> {
    // 1) instance 直下の .credentials.json (旧形式 / file-based)。
    let path = PathBuf::from(instance_path).join(".credentials.json");
    match fs::read(&path) {
        Ok(bytes) => {
            let s = std::str::from_utf8(&bytes)
                .map_err(|_| ProfileError::Http("credentials.json not utf8".into()))?;
            return parse_credentials_str(s);
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(ProfileError::Http(format!("read credentials failed: {e}"))),
    }

    // 2) macOS keychain フォールバック (新形式 / ccs 既定)。
    read_credentials_from_keychain(instance_path)
}

#[cfg(target_os = "macos")]
fn read_credentials_from_keychain(
    instance_path: &str,
) -> std::result::Result<ClaudeAiOauth, ProfileError> {
    let service = keychain_service_name(instance_path);
    let out = shell::user_shell_argv(&["security", "find-generic-password", "-s", &service, "-w"])
        .output()
        .map_err(|e| ProfileError::Http(format!("spawn security: {e}")))?;
    if !out.status.success() {
        // 44 = errSecItemNotFound — credentials 未作成扱いにする。
        return Err(ProfileError::NoCredentials);
    }
    let s = std::str::from_utf8(&out.stdout)
        .map_err(|_| ProfileError::Http("keychain output not utf8".into()))?;
    parse_credentials_str(s)
}

#[cfg(not(target_os = "macos"))]
fn read_credentials_from_keychain(
    _instance_path: &str,
) -> std::result::Result<ClaudeAiOauth, ProfileError> {
    Err(ProfileError::NoCredentials)
}

// ---- HTTP client trait + ureq impl ------------------------------------------

pub trait UsageClient: Sync {
    fn get_usage(&self, access_token: &str)
        -> std::result::Result<serde_json::Value, ProfileError>;
}

pub struct UreqClient {
    agent: ureq::Agent,
}

impl UreqClient {
    pub fn new(timeout: Duration) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(timeout)
            .user_agent(concat!("rai-ccs-usage/", env!("CARGO_PKG_VERSION")))
            .build();
        Self { agent }
    }
}

impl UsageClient for UreqClient {
    fn get_usage(
        &self,
        access_token: &str,
    ) -> std::result::Result<serde_json::Value, ProfileError> {
        let resp = self
            .agent
            .get(USAGE_ENDPOINT)
            .set("Authorization", &format!("Bearer {access_token}"))
            .set("anthropic-beta", "oauth-2025-04-20")
            .call();
        match resp {
            Ok(r) => {
                let v: serde_json::Value = r
                    .into_json()
                    .map_err(|e| ProfileError::Http(format!("decode body: {e}")))?;
                Ok(v)
            }
            Err(ureq::Error::Status(401, _)) => Err(ProfileError::AuthFailed),
            Err(ureq::Error::Status(code, r)) => {
                // body の内容は token を含み得るので参照しない。
                drop(r);
                Err(ProfileError::Http(format!("HTTP {code}")))
            }
            Err(ureq::Error::Transport(t)) if matches!(t.kind(), ureq::ErrorKind::Io) => {
                let msg = format!("{}", t.kind());
                if msg.to_lowercase().contains("time") {
                    Err(ProfileError::Timeout)
                } else {
                    Err(ProfileError::Http(format!("io: {}", t.kind())))
                }
            }
            Err(ureq::Error::Transport(t)) => {
                // io::Error::kind() などは accessToken を含まない。
                Err(ProfileError::Http(format!("{}", t.kind())))
            }
        }
    }
}

// ---- response -> RateWindow ------------------------------------------------

/// Anthropic 側のレスポンスから 5h / 7d の `RateWindow` を取り出す。
///
/// 実応答スキーマ (2026-05-19 ローカル実機で確認):
///   { "five_hour": {"utilization": 25.0, "resets_at": "2026-05-19T05:40:00.000Z"},
///     "seven_day": {"utilization": 84.0, "resets_at": "..." },
///     ... }
///
/// `utilization` は 0-100 の値。`resets_at` は ISO8601 文字列。
/// `null` / 欠落値もありうる (枠未設定 or 未消費)。
pub fn extract_windows(body: &serde_json::Value) -> (Option<RateWindow>, Option<RateWindow>) {
    let pick = |key: &str| -> Option<RateWindow> {
        let obj = body.get(key)?;
        if obj.is_null() {
            return None;
        }
        let used = obj.get("utilization").and_then(|v| v.as_f64())?;
        let resets = obj
            .get("resets_at")
            .and_then(|v| v.as_str())
            .and_then(parse_iso8601_to_epoch)?;
        Some(RateWindow {
            used_percentage: used,
            resets_at: resets,
        })
    };
    (pick("five_hour"), pick("seven_day"))
}

fn parse_iso8601_to_epoch(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

/// `rateLimitTier` を表示用に短縮する。`default_claude_max_20x` → `max_20x`。
/// 該当しない値はそのまま返す。
pub fn display_tier(raw: &str) -> String {
    raw.strip_prefix("default_claude_")
        .unwrap_or(raw)
        .to_string()
}

// ---- collect ----------------------------------------------------------------

fn collect(
    ccs_bin: &str,
    filter: &[String],
    client: &(dyn UsageClient + Send + Sync),
) -> Result<Snapshot> {
    let mut profiles = fetch_ccs_profiles(ccs_bin)?;
    if !filter.is_empty() {
        profiles.retain(|p| filter.iter().any(|n| n == &p.name));
    }
    let now_ms = now_unix_ms();
    let results: Vec<ProfileUsage> = run_in_chunks(profiles, 5, |p| fetch_one(&p, now_ms, client));
    Ok(Snapshot {
        fetched_at: Utc::now(),
        profiles: results,
    })
}

fn fetch_one(
    p: &CcsProfile,
    now_ms: i64,
    client: &(dyn UsageClient + Send + Sync),
) -> ProfileUsage {
    let mut out = ProfileUsage {
        name: p.name.clone(),
        is_default: p.is_default,
        tier: None,
        five_hour: None,
        seven_day: None,
        error: None,
    };
    let oauth = match read_credentials(&p.instance_path) {
        Ok(o) => o,
        Err(e) => {
            out.error = Some(e);
            return out;
        }
    };
    out.tier = oauth.rate_limit_tier.as_deref().map(display_tier);
    if is_expired(oauth.expires_at, now_ms) {
        out.error = Some(ProfileError::Expired);
        return out;
    }
    match client.get_usage(&oauth.access_token) {
        Ok(body) => {
            let (five, seven) = extract_windows(&body);
            out.five_hour = five;
            out.seven_day = seven;
        }
        Err(e) => out.error = Some(e),
    }
    out
}

fn run_in_chunks<T, R, F>(items: Vec<T>, chunk_size: usize, f: F) -> Vec<R>
where
    T: Send,
    R: Send,
    F: Fn(T) -> R + Sync,
{
    let mut results: Vec<R> = Vec::with_capacity(items.len());
    let mut buf = items;
    while !buf.is_empty() {
        let take = buf.len().min(chunk_size);
        let batch: Vec<T> = buf.drain(..take).collect();
        let mut chunk_results: Vec<R> = thread::scope(|s| {
            let handles: Vec<_> = batch.into_iter().map(|item| s.spawn(|| f(item))).collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("worker panicked"))
                .collect()
        });
        results.append(&mut chunk_results);
    }
    results
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---- rendering --------------------------------------------------------------

fn render_table(snap: &Snapshot, color: bool) {
    let header_ts = snap
        .fetched_at
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M %Z");
    println!("ccs Claude usage  (fetched {header_ts})");
    println!();
    let rows: Vec<Row> = snap.profiles.iter().map(Row::from_profile).collect();
    let widths = Widths::compute(&rows);
    widths.print_header();
    for row in &rows {
        row.print(&widths, color);
    }
}

struct Row {
    profile: String,
    tier: String,
    five_used: String,
    five_used_pct: Option<f64>,
    five_reset: String,
    seven_used: String,
    seven_reset: String,
    note: String,
}

impl Row {
    fn from_profile(p: &ProfileUsage) -> Self {
        let profile = if p.is_default {
            format!("{} *", p.name)
        } else {
            p.name.clone()
        };
        let tier = p.tier.clone().unwrap_or_default();
        let (five_used, five_used_pct, five_reset) = match p.five_hour {
            Some(w) => (
                format_pct(w.used_percentage),
                Some(w.used_percentage),
                format_reset(w.resets_at),
            ),
            None => ("—".to_string(), None, "—".to_string()),
        };
        let (seven_used, seven_reset) = match p.seven_day {
            Some(w) => (format_pct(w.used_percentage), format_reset(w.resets_at)),
            None => ("—".to_string(), "—".to_string()),
        };
        let note = match &p.error {
            Some(e) => e.note(),
            None => {
                if p.five_hour.is_none() && p.seven_day.is_none() {
                    "no usage yet".to_string()
                } else {
                    String::new()
                }
            }
        };
        Self {
            profile,
            tier,
            five_used,
            five_used_pct,
            five_reset,
            seven_used,
            seven_reset,
            note,
        }
    }

    fn print(&self, w: &Widths, color: bool) {
        let five_used = if color {
            highlight_pct(&self.five_used, self.five_used_pct)
        } else {
            self.five_used.clone()
        };
        // 色付きの幅は ANSI シーケンスを含むので、整形は色無しの幅を使う。
        let pad_five = " ".repeat(w.five_used.saturating_sub(self.five_used.chars().count()));
        println!(
            "{profile:<wp$}  {tier:<wt$}  {five_used}{pad_five}  {five_reset:<wfr$}  {seven_used:<wsu$}  {seven_reset:<wsr$}  {note}",
            profile = self.profile,
            tier = self.tier,
            five_used = five_used,
            pad_five = pad_five,
            five_reset = self.five_reset,
            seven_used = self.seven_used,
            seven_reset = self.seven_reset,
            note = self.note,
            wp = w.profile,
            wt = w.tier,
            wfr = w.five_reset,
            wsu = w.seven_used,
            wsr = w.seven_reset,
        );
    }
}

struct Widths {
    profile: usize,
    tier: usize,
    five_used: usize,
    five_reset: usize,
    seven_used: usize,
    seven_reset: usize,
}

impl Widths {
    fn compute(rows: &[Row]) -> Self {
        let mut w = Self {
            profile: "PROFILE".len(),
            tier: "TIER".len(),
            five_used: "5h USED".len(),
            five_reset: "5h RESET".len(),
            seven_used: "7d USED".len(),
            seven_reset: "7d RESET".len(),
        };
        for r in rows {
            w.profile = w.profile.max(r.profile.chars().count());
            w.tier = w.tier.max(r.tier.chars().count());
            w.five_used = w.five_used.max(r.five_used.chars().count());
            w.five_reset = w.five_reset.max(r.five_reset.chars().count());
            w.seven_used = w.seven_used.max(r.seven_used.chars().count());
            w.seven_reset = w.seven_reset.max(r.seven_reset.chars().count());
        }
        w
    }

    fn print_header(&self) {
        println!(
            "{:<wp$}  {:<wt$}  {:<wfu$}  {:<wfr$}  {:<wsu$}  {:<wsr$}  NOTE",
            "PROFILE",
            "TIER",
            "5h USED",
            "5h RESET",
            "7d USED",
            "7d RESET",
            wp = self.profile,
            wt = self.tier,
            wfu = self.five_used,
            wfr = self.five_reset,
            wsu = self.seven_used,
            wsr = self.seven_reset,
        );
    }
}

fn format_pct(pct: f64) -> String {
    format!("{:>3.0}%", pct)
}

fn format_reset(epoch_sec: i64) -> String {
    match Utc.timestamp_opt(epoch_sec, 0).single() {
        Some(dt) => dt
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        None => "—".to_string(),
    }
}

fn highlight_pct(s: &str, pct: Option<f64>) -> String {
    match pct {
        Some(p) if p >= 80.0 => format!("\x1b[31m{s}\x1b[0m"),
        Some(p) if p >= 60.0 => format!("\x1b[33m{s}\x1b[0m"),
        _ => s.to_string(),
    }
}

// ---- tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_account_profiles_only() {
        let body = br#"{
            "version": "x",
            "profiles": [
                {"name":"c1","type":"account","is_default":true,"instance_path":"/tmp/c1"},
                {"name":"kimi","type":"custom","is_default":false,"instance_path":"/tmp/kimi"},
                {"name":"c2","type":"account","is_default":false,"instance_path":"/tmp/c2"}
            ]
        }"#;
        let profs = parse_ccs_auth_list(body).expect("parse");
        let names: Vec<_> = profs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["c1", "c2"]);
        assert!(profs[0].is_default);
    }

    #[test]
    fn expired_judgment() {
        assert!(is_expired(Some(100), 200));
        assert!(is_expired(Some(200), 200));
        assert!(!is_expired(Some(300), 200));
        assert!(!is_expired(None, 200));
    }

    #[test]
    fn extracts_both_windows() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{
                "five_hour": {"utilization": 25.0, "resets_at": "2026-05-19T05:40:00.891874+00:00"},
                "seven_day": {"utilization": 84.0, "resets_at": "2026-05-20T19:00:00.891901+00:00"}
            }"#,
        )
        .unwrap();
        let (five, seven) = extract_windows(&v);
        let five = five.unwrap();
        assert!((five.used_percentage - 25.0).abs() < 1e-9);
        // 2026-05-19T05:40:00Z = 1779169200
        assert_eq!(five.resets_at, 1779169200);
        let seven = seven.unwrap();
        assert!((seven.used_percentage - 84.0).abs() < 1e-9);
    }

    #[test]
    fn extracts_missing_keys() {
        let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
        let (five, seven) = extract_windows(&v);
        assert!(five.is_none());
        assert!(seven.is_none());
    }

    #[test]
    fn extracts_null_value() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"five_hour": null, "seven_day": null}"#).unwrap();
        let (five, seven) = extract_windows(&v);
        assert!(five.is_none());
        assert!(seven.is_none());
    }

    #[test]
    fn extracts_partial_windows() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"five_hour": {"utilization": 10.0, "resets_at": "2026-05-19T05:40:00Z"}}"#,
        )
        .unwrap();
        let (five, seven) = extract_windows(&v);
        assert!(five.is_some());
        assert!(seven.is_none());
    }

    #[test]
    fn tier_normalization() {
        assert_eq!(display_tier("default_claude_max_20x"), "max_20x");
        assert_eq!(display_tier("max5x"), "max5x");
        assert_eq!(display_tier(""), "");
    }

    #[test]
    fn keychain_service_name_matches_claude_code_format() {
        // Claude Code が `CLAUDE_CONFIG_DIR` を sha256 して頭 8 hex 取った形式。
        // 実機 keychain と突き合わせて固定 (2026-05-19 確認):
        //   /Users/pc386/.ccs/instances/c1   → 65a88fb6
        //   /Users/pc386/.ccs/instances/c2   → 2d1633fa
        //   /Users/pc386/.ccs/instances/team → 25dad392
        assert_eq!(
            keychain_service_name("/Users/pc386/.ccs/instances/c1"),
            "Claude Code-credentials-65a88fb6"
        );
        assert_eq!(
            keychain_service_name("/Users/pc386/.ccs/instances/c2"),
            "Claude Code-credentials-2d1633fa"
        );
        assert_eq!(
            keychain_service_name("/Users/pc386/.ccs/instances/team"),
            "Claude Code-credentials-25dad392"
        );
    }

    #[test]
    fn parse_credentials_str_round_trip() {
        let raw = r#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":1700000000000,"rateLimitTier":"default_claude_max_20x"}}"#;
        let oauth = parse_credentials_str(raw).expect("parse");
        assert_eq!(oauth.access_token, "tok");
        assert_eq!(oauth.expires_at, Some(1_700_000_000_000));
        assert_eq!(
            oauth.rate_limit_tier.as_deref(),
            Some("default_claude_max_20x")
        );
    }

    #[test]
    fn parse_credentials_str_missing_oauth_is_no_credentials() {
        let raw = r#"{"foo":"bar"}"#;
        let err = parse_credentials_str(raw).unwrap_err();
        assert_eq!(err, ProfileError::NoCredentials);
    }

    #[test]
    fn snapshot_to_json_shape() {
        let snap = Snapshot {
            fetched_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            profiles: vec![
                ProfileUsage {
                    name: "c1".into(),
                    is_default: true,
                    tier: Some("max_20x".into()),
                    five_hour: Some(RateWindow {
                        used_percentage: 72.0,
                        resets_at: 1747691400,
                    }),
                    seven_day: Some(RateWindow {
                        used_percentage: 41.0,
                        resets_at: 1748008800,
                    }),
                    error: None,
                },
                ProfileUsage {
                    name: "foo".into(),
                    is_default: false,
                    tier: None,
                    five_hour: None,
                    seven_day: None,
                    error: Some(ProfileError::Expired),
                },
            ],
        };
        let v = snap.to_json();
        assert_eq!(v["profiles"][0]["name"], "c1");
        assert_eq!(v["profiles"][0]["is_default"], true);
        assert_eq!(v["profiles"][0]["five_hour"]["used_percentage"], 72.0);
        assert_eq!(v["profiles"][1]["error"], "expired");
        assert!(v["profiles"][1]["five_hour"].is_null());
    }

    #[test]
    fn fatal_error_classification() {
        assert!(ProfileError::Expired.is_fatal());
        assert!(ProfileError::AuthFailed.is_fatal());
        assert!(ProfileError::Timeout.is_fatal());
        assert!(ProfileError::NoCredentials.is_fatal());
    }

    /// 簡易 mock client: 渡された JSON をそのまま返す。
    struct MockClient(serde_json::Value);

    impl UsageClient for MockClient {
        fn get_usage(
            &self,
            _access_token: &str,
        ) -> std::result::Result<serde_json::Value, ProfileError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn fetch_one_uses_mock() {
        let dir = tempdir_or_skip();
        let inst = dir.join("c1");
        std::fs::create_dir_all(&inst).unwrap();
        let creds = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "tok",
                "expiresAt": 9_999_999_999_999_i64,
                "rateLimitTier": "default_claude_max_20x"
            }
        });
        std::fs::write(inst.join(".credentials.json"), creds.to_string()).unwrap();
        let prof = CcsProfile {
            name: "c1".into(),
            kind: "account".into(),
            is_default: true,
            instance_path: inst.to_string_lossy().into_owned(),
        };
        let body = serde_json::json!({
            "five_hour": {"utilization": 50.0, "resets_at": "2026-05-19T05:40:00Z"}
        });
        let client = MockClient(body);
        let res = fetch_one(&prof, 0, &client);
        assert_eq!(res.tier.as_deref(), Some("max_20x"));
        assert!(res.error.is_none());
        assert_eq!(res.five_hour.unwrap().used_percentage, 50.0);
        assert!(res.seven_day.is_none());
    }

    #[test]
    fn fetch_one_no_credentials() {
        let dir = tempdir_or_skip();
        let inst = dir.join("nope-unique-no-keychain-hit");
        std::fs::create_dir_all(&inst).unwrap();
        let prof = CcsProfile {
            name: "nope".into(),
            kind: "account".into(),
            is_default: false,
            instance_path: inst.to_string_lossy().into_owned(),
        };
        let body = serde_json::json!({});
        let client = MockClient(body);
        let res = fetch_one(&prof, 0, &client);
        assert_eq!(res.error.as_ref().unwrap(), &ProfileError::NoCredentials);
    }

    #[test]
    fn fetch_one_expired() {
        let dir = tempdir_or_skip();
        let inst = dir.join("expd");
        std::fs::create_dir_all(&inst).unwrap();
        let creds = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "tok",
                "expiresAt": 1_i64
            }
        });
        std::fs::write(inst.join(".credentials.json"), creds.to_string()).unwrap();
        let prof = CcsProfile {
            name: "expd".into(),
            kind: "account".into(),
            is_default: false,
            instance_path: inst.to_string_lossy().into_owned(),
        };
        let client = MockClient(serde_json::json!({}));
        let res = fetch_one(&prof, 999_999_999_999_i64, &client);
        assert_eq!(res.error.as_ref().unwrap(), &ProfileError::Expired);
    }

    /// std::env::temp_dir 下に test-uniq な dir を作る。
    fn tempdir_or_skip() -> PathBuf {
        let nano = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "rai-ccs-usage-test-{}-{}",
            std::process::id(),
            nano
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
