//! The theme new agents' names come from (card #702): the person's pick, on
//! a `names.theme` row in the private log, so it reaches all their devices.
//! Only `actor == "user"` rows count, the latest `at` wins, and with none
//! it is `""`: every theme mixed (decision 52). Set in the app only: no tool
//! reaches it. A pick only narrows new names; nobody is renamed.

use std::sync::RwLock;

use gocompat::time::GoTime;
use serde::{Deserialize, Serialize};
use store::{Filter, Store, kinds};

use super::Services;

/// A `names.theme` row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamesTheme {
    /// A [`coord::names::THEMES`] id, or `""` for every theme mixed.
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub at: GoTime,
}

/// The theme as last read from the log, shared with the
/// [`crate::namer::Namer`].
#[derive(Default)]
pub(crate) struct NameTheme {
    theme: RwLock<String>,
}

impl NameTheme {
    /// Read from the log once, at load.
    pub(crate) fn load(st: &Store) -> NameTheme {
        NameTheme {
            theme: RwLock::new(newest_theme(st)),
        }
    }

    /// Reads it again: a row came from another device, or the log resynced.
    pub(crate) fn reload(&self, st: &Store) {
        *self.theme.write().unwrap_or_else(|e| e.into_inner()) = newest_theme(st);
    }

    /// The stored id; `""` for every theme mixed. An id this build doesn't
    /// know (a newer device's) reads as mixed where it is used.
    pub(crate) fn get(&self) -> String {
        self.theme.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// The theme on the `user` row with the largest `at`, ties to the larger
/// device id, so every device settles on the same row whatever order rows
/// arrived in; `""` with none.
fn newest_theme(st: &Store) -> String {
    let mut f = Filter {
        kinds: vec![kinds::NAMES_THEME.into()],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    };
    let mut best: Option<((i64, u32), String, String)> = None;
    loop {
        let Ok(page) = st.list(&f) else {
            return String::new();
        };
        for ev in &page {
            if ev.actor != "user" {
                continue;
            }
            let Ok(r) = serde_json::from_slice::<NamesTheme>(&ev.payload) else {
                continue;
            };
            let at = (r.at.secs, r.at.nanos);
            let newer = best.as_ref().is_none_or(|(b_at, b_dev, _)| {
                (at, ev.device_id.as_str()) > (*b_at, b_dev.as_str())
            });
            if newer {
                best = Some((at, ev.device_id.clone(), r.theme));
            }
        }
        match page.last() {
            Some(last) if page.len() as i64 >= f.limit => f.after = last.cursor(),
            _ => break,
        }
    }
    best.map(|(_, _, t)| t).unwrap_or_default()
}

impl Services {
    pub(crate) fn names_theme(&self) -> String {
        self.name_theme.get()
    }

    /// The person picks it, from the app only: `""` or a
    /// [`coord::names::THEMES`] id.
    pub(crate) fn set_names_theme(&self, theme: &str) -> Result<(), String> {
        if !theme.is_empty() && coord::names::theme(theme).is_none() {
            return Err(format!("no names theme {theme:?}"));
        }
        let row = NamesTheme {
            theme: theme.into(),
            by: format!("user:{}", self.device.id),
            at: (self.now)(),
        };
        self.store
            .append("user", kinds::NAMES_THEME, &row)
            .map_err(|e| e.to_string())?;
        self.name_theme.reload(&self.store);
        Ok(())
    }

    /// A `names.theme` row from the private log's follower: this device's
    /// own was read when it was written. Settings reads it when it opens,
    /// so nothing is published.
    pub(crate) fn follow_names_theme(&self) {
        self.name_theme.reload(&self.store);
    }
}
