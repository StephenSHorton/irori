//! The tasks-folder scan (card #1249, design §2.2): the runs of sessions
//! with no mod, foreground until proven background, and the re-check of
//! saved runs on Core start. Slice L6 fills this file in.

use crate::service::Core;

/// Starts the scan loop. A no-op until slice L6.
pub(crate) fn start(core: &Core) {
    let _ = core;
}
