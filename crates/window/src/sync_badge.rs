//! The header's sync badge (card #1089): `sync:changed` carries `true` when
//! this device starts exchanging rows or blobs with a peer and `false` once
//! it has been quiet for a second (the hold is the mesh's, see
//! `mesh::activity`). Sent on change only; the page starts from `false`.

use std::sync::Arc;
use std::time::Duration;

use sys::cancel::Cancel;

use crate::Window;

pub const EVENT_SYNC_CHANGED: &str = "sync:changed";

/// How often the flag is read. The mesh holds it a full second, so this
/// can be slow without a short sync going unseen.
const EVERY: Duration = Duration::from_millis(150);

impl Window {
    /// Whether this device is syncing with a peer now.
    pub fn syncing(&self) -> bool {
        self.core.mesh().syncing()
    }
}

/// Emits [`EVENT_SYNC_CHANGED`] when the flag flips.
pub(crate) fn start(w: Arc<Window>, stop: Cancel) -> Option<std::thread::JoinHandle<()>> {
    let spawned = std::thread::Builder::new()
        .name("irori-window-sync".into())
        .spawn(move || {
            let mut last = false;
            while !stop.sleep(EVERY) {
                let now = w.syncing();
                if now != last {
                    w.emit(EVENT_SYNC_CHANGED, &now);
                    last = now;
                }
            }
        });
    match spawned {
        Ok(h) => Some(h),
        Err(e) => {
            irori_core::log(&format!("sync badge: no thread: {e}"));
            None
        }
    }
}
