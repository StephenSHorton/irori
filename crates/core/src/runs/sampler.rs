//! Process metering, tail tabs and Stop (card #1249, design §4, §5): the
//! `irori-runs` sampler thread, the read-only tail a background shell run
//! opens on its card, and [`Core::stop_run`]. Slice L7 fills this file in.

use crate::service::Core;

/// Starts the sampler. A no-op until slice L7.
pub(crate) fn start(core: &Core) {
    let _ = core;
}

impl Core {
    /// The person stops one of this device's runs (the window's
    /// `runs_stop`): marks it stopped and ends its process tree. Not wired
    /// until slice L7.
    pub fn stop_run(&self, key: &str, task_id: &str) -> Result<(), String> {
        let _ = (key, task_id);
        Err("stopping a run is not available yet".into())
    }
}
