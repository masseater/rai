//! 共通シグナルハンドリング。
//!
//! SIGINT / SIGTERM / SIGHUP を一括捕捉し、最後に受け取ったシグナル番号を
//! `Arc<AtomicI32>` に書き込む。メインループはこの値を polling して停止判断する。

use std::sync::atomic::AtomicI32;
use std::sync::Arc;
use std::thread;

use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

/// 補足対象のシグナル一覧。
pub const WATCHED: &[i32] = &[SIGINT, SIGTERM, SIGHUP];

/// 受信シグナル番号 (未受信は 0)。
pub type SigSlot = Arc<AtomicI32>;

/// バックグラウンドスレッドでシグナルを購読する。
///
/// 戻り値の `SigSlot` を `load(SeqCst)` することで「最後に届いたシグナル番号」が読める。
/// 既に何か届いている場合は上書きされ、最後の 1 つだけが残る。
pub fn install() -> std::io::Result<SigSlot> {
    let slot: SigSlot = Arc::new(AtomicI32::new(0));
    let slot_for_thread = slot.clone();
    let mut signals = Signals::new(WATCHED)?;
    thread::Builder::new()
        .name("rai-signals".into())
        .spawn(move || {
            for sig in signals.forever() {
                slot_for_thread.store(sig, std::sync::atomic::Ordering::SeqCst);
            }
        })?;
    Ok(slot)
}

/// シグナル番号 → 推奨終了コード (POSIX 慣習: 128 + signo)。
pub fn exit_code(sig: i32) -> i32 {
    128 + sig
}
