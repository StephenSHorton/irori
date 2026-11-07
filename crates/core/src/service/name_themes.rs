//! Which themes new agents' names come from (card #702): every theme is on
//! until the person turns one off. Each change is a `names.themes` row in the
//! private log with the themes turned off, so it reaches all their devices
//! and a theme added later starts on (decision 52). Only `actor == "user"`
//! rows count and the latest `at` wins. It is set in the app only; no tool
//! reaches it. It only changes which words new names come from: nobody is
//! renamed.

use std::sync::RwLock;

use gocompat::time::GoTime;
use serde::{Deserialize, Serialize};
use store::{Filter, Store, kinds};

use super::Services;

/// A `names.themes` row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamesThemes {
    /// [`coord::names::THEMES`] ids turned off. An id this build doesn't
    /// know (a newer device's) is kept, and ignored here.
    #[serde(default)]
    pub off: Vec<String>,
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub at: GoTime,
}

/// The themes turned off, as last read from the log, shared with the
/// [`crate::namer::Namer`].
#[derive(Default)]
pub(crate) struct NameThemes {
    off: RwLock<Vec<String>>,
}

impl NameThemes {
    /// Read from the log once, at load.
    pub(crate) fn load(st: &Store) -> NameThemes {
        NameThemes {
            off: RwLock::new(newest_off(st)),
        }
    }

    /// Reads it again: a row came from another device, or the log resynced.
    pub(crate) fn reload(&self, st: &Store) {
        *self.off.write().unwrap_or_else(|e| e.into_inner()) = newest_off(st);
    }

    /// The ids turned off; empty when every theme is on.
    pub(crate) fn off(&self) -> Vec<String> {
        self.off.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// The themes off on the `user` row with the largest `at`, ties to the
/// larger device id, so every device settles on the same row whatever order
/// rows arrived in; none with no row.
fn newest_off(st: &Store) -> Vec<String> {
    let mut f = Filter {
        kinds: vec![kinds::NAMES_THEMES.into()],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    };
    let mut best: Option<((i64, u32), String, Vec<String>)> = None;
    loop {
        let Ok(page) = st.list(&f) else {
            return Vec::new();
        };
        for ev in &page {
            if ev.actor != "user" {
                continue;
            }
            let Ok(r) = serde_json::from_slice::<NamesThemes>(&ev.payload) else {
                continue;
            };
            let at = (r.at.secs, r.at.nanos);
            let newer = best.as_ref().is_none_or(|(b_at, b_dev, _)| {
                (at, ev.device_id.as_str()) > (*b_at, b_dev.as_str())
            });
            if newer {
                best = Some((at, ev.device_id.clone(), r.off));
            }
        }
        match page.last() {
            Some(last) if page.len() as i64 >= f.limit => f.after = last.cursor(),
            _ => break,
        }
    }
    best.map(|(_, _, off)| off).unwrap_or_default()
}

impl Services {
    pub(crate) fn names_themes_off(&self) -> Vec<String> {
        self.name_themes.off()
    }

    /// The person turns one theme on or off, from the app only.
    pub(crate) fn set_names_theme(&self, id: &str, on: bool) -> Result<(), String> {
        if coord::names::theme(id).is_none() {
            return Err(format!("no names theme {id:?}"));
        }
        let mut off = self.name_themes.off();
        off.retain(|o| o != id);
        if !on {
            off.push(id.into());
        }
        let row = NamesThemes {
            off,
            by: format!("user:{}", self.device.id),
            at: (self.now)(),
        };
        self.store
            .append("user", kinds::NAMES_THEMES, &row)
            .map_err(|e| e.to_string())?;
        self.name_themes.reload(&self.store);
        Ok(())
    }

    /// A `names.themes` row from the private log's follower: this device's
    /// own was read when it was written. Settings reads it when it opens,
    /// so nothing is published.
    pub(crate) fn follow_names_themes(&self) {
        self.name_themes.reload(&self.store);
    }
}
