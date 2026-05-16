//! claude CLI (`claude`) と共有する小さな型定義。
//!
//! `rai claude print` と `rai develop` の両方が `--permission-mode` をパススルー
//! するので、`PermissionMode` enum は両 subcommand crate から共通に使えるよう
//! ここに置く。値の意味は `claude --help` を参照し、`#[value(name = "...")]` で
//! `claude --permission-mode` がそのまま受け取る文字列にシリアライズされる
//! ことを保証する (`docs/specs/16-shell-execution-policy.md` の関連項目)。

use clap::ValueEnum;

/// claude の `--permission-mode` に渡す値。`claude --permission-mode` の choices
/// (acceptEdits / auto / bypassPermissions / default / dontAsk / plan) と一致。
///
/// clap の snake_case 自動シリアライズは claude が受け取る文字列と一致しないので、
/// 各 variant に `#[value(name = "...")]` を明示すること。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PermissionMode {
    #[value(name = "acceptEdits")]
    AcceptEdits,
    #[value(name = "auto")]
    Auto,
    #[value(name = "bypassPermissions")]
    BypassPermissions,
    #[value(name = "default")]
    Default,
    #[value(name = "dontAsk")]
    DontAsk,
    #[value(name = "plan")]
    Plan,
}

impl PermissionMode {
    /// `claude --permission-mode <X>` の `<X>` 文字列を返す。
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::AcceptEdits => "acceptEdits",
            Self::Auto => "auto",
            Self::BypassPermissions => "bypassPermissions",
            Self::Default => "default",
            Self::DontAsk => "dontAsk",
            Self::Plan => "plan",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_arg_serializes_all_variants() {
        for (m, want) in [
            (PermissionMode::AcceptEdits, "acceptEdits"),
            (PermissionMode::Auto, "auto"),
            (PermissionMode::BypassPermissions, "bypassPermissions"),
            (PermissionMode::Default, "default"),
            (PermissionMode::DontAsk, "dontAsk"),
            (PermissionMode::Plan, "plan"),
        ] {
            assert_eq!(m.as_arg(), want);
        }
    }
}
