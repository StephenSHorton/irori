//! "Transcribe what I say in calls" (card #656): the person's switch, on
//! a `transcribe.set` row in the private log, so it reaches all their
//! devices. Only `actor == "user"` rows count, the latest `at` wins, and with
//! none it is on (decision 52). Set in the app only: no tool reaches it.

use std::sync::atomic::{AtomicBool, Ordering};

use gocompat::time::GoTime;
use serde::{Deserialize, Serialize};
use store::{Filter, Store, kinds};

use super::Services;
use crate::events::CoreEvent;

/// On for a new person (decision 52).
pub const TRANSCRIBE_ON: bool = true;

/// A `transcribe.set` row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeSet {
    pub on: bool,
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub at: GoTime,
}

/// The switch as last read from the log.
pub(crate) struct CallSettings {
    transcribe: AtomicBool,
}

impl CallSettings {
    /// Read from the log once, at load.
    pub(crate) fn load(st: &Store) -> CallSettings {
        CallSettings {
            transcribe: AtomicBool::new(newest_transcribe(st)),
        }
    }

    /// Reads it again: a row came from another device, or the log resynced.
    pub(crate) fn reload(&self, st: &Store) {
        self.transcribe
            .store(newest_transcribe(st), Ordering::SeqCst);
    }

    pub(crate) fn transcribe(&self) -> bool {
        self.transcribe.load(Ordering::SeqCst)
    }
}

/// The switch on the `user` row with the largest `at`, ties to the larger
/// device id, so every device settles on the same row whatever order rows
/// arrived in; on with none.
fn newest_transcribe(st: &Store) -> bool {
    newest_switch(st, kinds::TRANSCRIBE_SET, TRANSCRIBE_ON)
}

/// The newest `{on, by, at}` switch of `kind` in the log, as for
/// transcribe; `none` with no row (card #782 reads its own this way).
pub(crate) fn newest_switch(st: &Store, kind: &str, none: bool) -> bool {
    let mut f = Filter {
        kinds: vec![kind.into()],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    };
    let mut best: Option<((i64, u32), String, bool)> = None;
    loop {
        let Ok(page) = st.list(&f) else {
            return none;
        };
        for ev in &page {
            if ev.actor != "user" {
                continue;
            }
            let Ok(r) = serde_json::from_slice::<TranscribeSet>(&ev.payload) else {
                continue;
            };
            let at = (r.at.secs, r.at.nanos);
            let newer = best.as_ref().is_none_or(|(b_at, b_dev, _)| {
                (at, ev.device_id.as_str()) > (*b_at, b_dev.as_str())
            });
            if newer {
                best = Some((at, ev.device_id.clone(), r.on));
            }
        }
        match page.last() {
            Some(last) if page.len() as i64 >= f.limit => f.after = last.cursor(),
            _ => break,
        }
    }
    best.map_or(none, |(_, _, on)| on)
}

impl Services {
    pub(crate) fn call_transcribe(&self) -> bool {
        self.call_settings.transcribe()
    }

    /// The person sets it, from the app only. A call in progress hears it
    /// as a [`CoreEvent::Call`].
    pub(crate) fn set_call_transcribe(&self, on: bool) -> Result<(), String> {
        let row = TranscribeSet {
            on,
            by: format!("user:{}", self.device.id),
            at: (self.now)(),
        };
        self.store
            .append("user", kinds::TRANSCRIBE_SET, &row)
            .map_err(|e| e.to_string())?;
        self.call_settings.reload(&self.store);
        self.hub.publish(CoreEvent::Call);
        Ok(())
    }

    /// A `transcribe.set` row from the private log's follower: this
    /// device's own was read when it was written.
    pub(crate) fn follow_call_settings(&self) {
        let was = self.call_settings.transcribe();
        self.call_settings.reload(&self.store);
        if self.call_settings.transcribe() != was {
            self.hub.publish(CoreEvent::Call);
        }
    }
}
