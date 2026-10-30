//! The in-memory view (I:internal/ledger/store.go).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::RwLock;

use gocompat::text;
use gocompat::time::GoTime;

use crate::keys::trim_space;
use crate::{Entry, instant, keep_cutoff};

#[derive(Default)]
struct Inner {
    entries: Vec<Entry>,
    /// identity -> position in entries.
    index: HashMap<String, usize>,
}

/// The ledger view. Safe for concurrent use; every view returns copies.
#[derive(Default)]
pub struct Ledger {
    inner: RwLock<Inner>,
}

fn identity(e: &Entry) -> String {
    format!("{}\0{}\0{}", e.key, e.session, e.role.as_str())
}

fn before(a: &GoTime, b: &GoTime) -> bool {
    instant(a) < instant(b)
}

/// Whether e is still in the view at now. Like tsukumo, an entry exactly
/// Keep old is kept.
fn live(e: &Entry, now: &GoTime) -> bool {
    instant(&e.at) >= keep_cutoff(now)
}

/// The Owners order: role rank, earliest At, then Session.
fn owner_cmp(a: &Entry, b: &Entry) -> Ordering {
    a.role
        .rank_or_zero()
        .cmp(&b.role.rank_or_zero())
        .then_with(|| instant(&a.at).cmp(&instant(&b.at)))
        .then_with(|| a.session.cmp(&b.session))
}

impl Ledger {
    /// An empty ledger.
    pub fn new() -> Ledger {
        Ledger::default()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|p| p.into_inner())
    }

    /// Merges e on the identity (key, session, role): a new identity is
    /// added; an existing one keeps the earliest `at` and the first non-empty
    /// URL. Reports whether the view changed. Entries with an empty key or
    /// session, a zero `at` (it could never show in the view) or an unknown
    /// role are ignored.
    pub fn add(&self, e: Entry) -> bool {
        if e.key.is_empty() || e.session.is_empty() || e.at.is_zero() {
            return false;
        }
        if e.role.rank().is_none() {
            return false;
        }
        let mut g = self.inner.write().unwrap_or_else(|p| p.into_inner());
        let id = identity(&e);
        let Some(&i) = g.index.get(&id) else {
            let n = g.entries.len();
            g.index.insert(id, n);
            g.entries.push(e);
            return true;
        };
        let old = &mut g.entries[i];
        let mut changed = false;
        if before(&e.at, &old.at) {
            old.at = e.at;
            changed = true;
        }
        if old.url.is_empty() && !e.url.is_empty() {
            old.url = e.url;
            changed = true;
        }
        changed
    }

    fn collect(&self, keep: impl Fn(&Entry) -> bool, now: &GoTime) -> Vec<Entry> {
        self.read()
            .entries
            .iter()
            .filter(|e| keep(e) && live(e, now))
            .cloned()
            .collect()
    }

    /// A PR's entries younger than Keep at now, best first: by role rank,
    /// then earliest `at`, then session for a stable order.
    pub fn owners(&self, key: &str, now: &GoTime) -> Vec<Entry> {
        let mut out = self.collect(|e| e.key == key, now);
        out.sort_by(owner_cmp);
        out
    }

    /// Every entry younger than Keep for a repo (`"owner/name"`,
    /// case-insensitive; `""` means all), newest PR number first, then key,
    /// then as [`Ledger::owners`].
    pub fn for_repo(&self, repo: &str, now: &GoTime) -> Vec<Entry> {
        let repo = trim_space(repo);
        let mut out = self.collect(|e| repo.is_empty() || text::equal_fold(&e.repo, repo), now);
        out.sort_by(|a, b| {
            b.number
                .cmp(&a.number)
                .then_with(|| a.key.cmp(&b.key))
                .then_with(|| owner_cmp(a, b))
        });
        out
    }

    /// A session's entries younger than Keep, newest `at` first, then key,
    /// then role rank.
    pub fn for_session(&self, session: &str, now: &GoTime) -> Vec<Entry> {
        let mut out = self.collect(|e| e.session == session, now);
        out.sort_by(|a, b| {
            instant(&b.at)
                .cmp(&instant(&a.at))
                .then_with(|| a.key.cmp(&b.key))
                .then_with(|| a.role.rank_or_zero().cmp(&b.role.rank_or_zero()))
        });
        out
    }
}
