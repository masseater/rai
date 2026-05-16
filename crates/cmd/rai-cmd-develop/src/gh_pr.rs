//! `gh pr view` の JSON 表現と、PR が fork 由来かを判定するヘルパー。
//!
//! `rai develop pr` (full PR ビュー: mergeable / failures つき) と
//! `rai develop resume` (PrLite: 最低限) で同じ `gh pr` JSON を読むため、
//! deserialize 用の型と fork 判定をここに集約する。両モジュールでは
//! ローカルの `Pr` / `PrLite` 型に詰め替えてから使う。

use serde::Deserialize;

/// `gh pr view --json …` で取れる JSON の全フィールド。各サブコマンドが必要なものだけ
/// 取り出す。
#[derive(Debug, Deserialize)]
pub struct PrJson {
    pub number: u64,
    pub title: String,
    pub url: String,
    #[serde(rename = "headRefName")]
    pub head_ref_name: String,
    #[serde(rename = "baseRefName")]
    pub base_ref_name: String,
    #[serde(rename = "mergeable", default)]
    pub mergeable: Option<String>,
    #[serde(rename = "statusCheckRollup", default)]
    pub status_check_rollup: Vec<StatusCheck>,
    #[serde(rename = "headRepository", default)]
    pub head_repository: Option<HeadRepo>,
    #[serde(rename = "headRepositoryOwner", default)]
    pub head_repository_owner: Option<HeadOwner>,
}

#[derive(Debug, Deserialize)]
pub struct HeadRepo {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct HeadOwner {
    pub login: String,
}

#[derive(Debug, Deserialize)]
pub struct StatusCheck {
    /// CheckRun → name, StatusContext → context. どちらかが入る。
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    /// CheckRun: conclusion (SUCCESS / FAILURE / ...)
    /// StatusContext: state (SUCCESS / FAILURE / ERROR / PENDING)
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(rename = "detailsUrl", default)]
    pub details_url: Option<String>,
    #[serde(rename = "targetUrl", default)]
    pub target_url: Option<String>,
}

/// PR が fork から来ているかどうかを判定する。`head_owner` が `base_owner` と
/// 異なり、`head_repo` が Some の場合のみ fork と判定する。`head_repository` が
/// None のケース (権限が無くて gh から見えない等) は安全側で `false` (= 同一リポ
/// 扱い) にする。
pub fn is_fork(head_owner: Option<&str>, head_repo: Option<&str>, base_owner: &str) -> bool {
    match (head_owner, head_repo) {
        (Some(owner), Some(_)) => owner != base_owner,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_fork_true_when_owner_differs_and_repo_present() {
        assert!(is_fork(Some("alice"), Some("repo"), "bob"));
    }

    #[test]
    fn is_fork_false_when_owner_matches() {
        assert!(!is_fork(Some("bob"), Some("repo"), "bob"));
    }

    #[test]
    fn is_fork_false_when_head_repo_missing() {
        assert!(!is_fork(Some("alice"), None, "bob"));
        assert!(!is_fork(None, Some("repo"), "bob"));
        assert!(!is_fork(None, None, "bob"));
    }
}
