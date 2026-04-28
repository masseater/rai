//! queue.json + advisory file lock。

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Entry {
    pub head_sha: String,
    pub base_ref: String,
    pub head_ref: String,
    pub mergeable: String,
    pub title: String,
    pub url: String,
    pub status: String,
    #[serde(default)]
    pub attempts: u32,
    pub enqueued_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub finished_at: String,
    #[serde(default)]
    pub log_path: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Queue {
    pub version: u32,
    pub updated_at: String,
    pub entries: BTreeMap<String, Entry>,
}

impl Default for Queue {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            updated_at: now_iso(),
            entries: BTreeMap::new(),
        }
    }
}

pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub struct Paths {
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    pub fn new(state_dir: Option<PathBuf>, cache_dir: Option<PathBuf>) -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let state_dir = state_dir.unwrap_or_else(|| home.join(".local/state/resolve-conflicts"));
        let cache_dir = cache_dir.unwrap_or_else(|| home.join(".cache/resolve-conflicts"));
        Self {
            state_dir,
            cache_dir,
        }
    }

    pub fn queue_json(&self) -> PathBuf {
        self.state_dir.join("queue.json")
    }
    pub fn lock_file(&self) -> PathBuf {
        self.state_dir.join("queue.lock")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.state_dir.join("logs")
    }
    pub fn worktree_dir(&self, pr: u64) -> PathBuf {
        self.cache_dir.join("wt").join(pr.to_string())
    }
    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.state_dir)?;
        fs::create_dir_all(self.logs_dir())?;
        fs::create_dir_all(self.cache_dir.join("wt"))?;
        Ok(())
    }
}

pub struct Lock {
    file: File,
}

impl Lock {
    /// 排他 lock を取得する。既に他プロセスが持っているなら即エラー。
    pub fn try_acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .with_context(|| format!("failed to open lock file: {}", path.display()))?;
        file.try_lock_exclusive()
            .map_err(|e| anyhow::anyhow!("another instance is running ({e})"))?;
        Ok(Self { file })
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

pub fn load(path: &Path) -> Result<Queue> {
    if !path.exists() {
        return Ok(Queue::default());
    }
    let mut buf = String::new();
    File::open(path)
        .with_context(|| format!("failed to open {}", path.display()))?
        .read_to_string(&mut buf)?;
    if buf.trim().is_empty() {
        return Ok(Queue::default());
    }
    let q: Queue = serde_json::from_str(&buf)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(q)
}

pub fn save(path: &Path, q: &mut Queue) -> Result<()> {
    q.updated_at = now_iso();
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)
            .with_context(|| format!("failed to open temp file: {}", tmp.display()))?;
        let body = serde_json::to_string_pretty(q)?;
        f.write_all(body.as_bytes())?;
        f.flush()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}
