//! 旧 API 互換: 端末復元用の panic フック。
//!
//! 実体は `term::install_panic_restore` に移譲しているが、
//! 「panic フック」という分類で discoverable にするためエイリアスを置いておく。

pub use crate::term::install_panic_restore as install;
