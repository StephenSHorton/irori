//! The mesh summary (card #1249, design §7): this device's `agent.run`
//! rows and the fold of other devices' ones. Slice L9 fills this file in.

use crate::service::Core;

/// Starts the writer and the fold. A no-op until slice L9.
pub(crate) fn start(core: &Core) {
    let _ = core;
}
