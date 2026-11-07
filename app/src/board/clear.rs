//! Clear done cards by age (clear-done.tsx, card #138): pick an age, see
//! how many it takes, confirm. Cleared cards leave the app's lists, in
//! your spaces too; agents still find them with search and show. On the
//! Closed tab (compact) and in Settings > Maintenance.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use rune::{El, Tx, col, row, text};

use super::bars::{Look, chip};
use crate::ui::theme::status;
use crate::ui::{Frame, State};
use crate::widgets::confirm::{Ask, confirm_button};

const DAY: i64 = 24 * 60 * 60;

/// The ages a clear can take: id, label, seconds, phrase.
pub const AGES: [(&str, &str, i64, &str); 4] = [
    ("day", "Older than a day", DAY, "closed more than a day ago"),
    ("week", "Older than a week", 7 * DAY, "closed more than a week ago"),
    ("month", "Older than a month", 30 * DAY, "closed more than a month ago"),
    ("all", "All", 0, ""),
];

/// One Clear done's state, by its key.
#[derive(Clone, Debug, Default)]
pub struct Clear {
    /// The age picked (an index into [`AGES`]; week until picked).
    pub age: Option<usize>,
    pub scope: String,
    /// How many it would clear (None while counting).
    pub count: Option<i64>,
    /// What the last clear cleared.
    pub last: Option<i64>,
    pub error: Option<String>,
}

impl Clear {
    fn age(&self) -> usize {
        self.age.unwrap_or(1)
    }
}

#[derive(Default)]
pub struct Model {
    pub by_key: HashMap<String, Clear>,
}

fn plural(n: i64) -> String {
    format!("{n} done {}", if n == 1 { "card" } else { "cards" })
}

/// Counts what the picked age would clear, off the UI thread.
pub fn count(s: &mut State, key: &str) {
    let Some(c) = s.board.clear.by_key.get(key) else { return };
    let (secs, scope, k) = (AGES[c.age()].2, c.scope.clone(), key.to_string());
    let age = c.age();
    s.spawn(
        move |w| w.clear_done_count(secs, &scope),
        move |s, n| {
            if let Some(c) = s.board.clear.by_key.get_mut(&k)
                && c.age() == age
            {
                c.count = Some(n);
                c.error = None;
            }
        },
    );
}

/// Counts every Clear done again (a board change).
pub fn recount(s: &mut State) {
    let keys: Vec<String> = s.board.clear.by_key.keys().cloned().collect();
    for k in keys {
        count(s, &k);
    }
}

/// The age chips, the count, and Clear with its confirm step. `scope`:
/// whose done cards ("" all, "own" Home's, else a space id).
pub fn clear_done(s: &State, f: &Frame, key: &str, scope: &str, compact: bool) -> El {
    let k = f.k;
    let c = s.board.clear.by_key.get(key).filter(|c| c.scope == scope).cloned().unwrap_or_else(|| Clear {
        scope: scope.to_string(),
        ..Clear::default()
    });
    let pick = AGES[c.age()];
    let n = c.count.unwrap_or(0);
    let what = if pick.3.is_empty() { format!("all {}", plural(n)) } else { format!("{} {}", plural(n), pick.3) };
    let mut ages = row().key(format!("{key}/ages")).role("group").label("Clear done cards by age").gap(4.0).center_y().flex_wrap();
    if compact {
        ages = ages.child(text("Clear done:").font(k.ty.text_xsmall).color(k.c.text_muted).mr(4.0));
    }
    for (i, (id, label, _, _)) in AGES.iter().enumerate() {
        let (kk, sc) = (key.to_string(), scope.to_string());
        ages = ages.child(
            chip(f, &format!("{key}/age-{id}"), label, c.age() == i, Look::Secondary, "", None, None, move |s: &mut State| {
                let e = s.board.clear.by_key.entry(kk.clone()).or_default();
                e.scope = sc.clone();
                e.age = Some(i);
                e.count = None;
                e.last = None;
                count(s, &kk);
            })
            .h(24.0)
            .px(8.0),
        );
    }
    let mut line = row().key(format!("{key}/line")).gap(8.0).center_y().flex_wrap();
    if !compact {
        line = line.child(
            text(match c.count {
                None => "Counting…".to_string(),
                Some(0) => "Nothing to clear.".to_string(),
                Some(_) => format!("Clears {what}."),
            })
            .font(rune::tok::TextStyle::new(12.5, 400.0))
            .color(k.c.text_muted),
        );
    }
    let cleared = Arc::new(AtomicI64::new(0));
    let (secs, sc, got) = (pick.2, scope.to_string(), cleared.clone());
    let kk = key.to_string();
    let ask = Ask::new(
        format!("Clear {what}? They leave the app's lists, in your spaces too. Agents still find them with search and show. Nothing is deleted."),
        if n > 0 { format!("Clear {n}") } else { "Clear".into() },
        move |w| {
            let n = w.clear_done(secs, &[], &sc)?;
            got.store(n, Ordering::Relaxed);
            Ok(())
        },
    )
    .rich()
    .done(move |s| {
        if let Some(c) = s.board.clear.by_key.get_mut(&kk) {
            c.last = Some(cleared.load(Ordering::Relaxed));
        }
        count(s, &kk);
        super::done::reload_all(s);
    });
    line = line.child(confirm_button(s, f, &format!("{key}/go"), &if n > 0 { format!("Clear {n}…") } else { "Clear".into() }, None, n == 0, ask));
    if let Some(l) = c.last {
        line = line.child(text(format!("Cleared {}.", plural(l))).role("status").font(k.ty.text_xsmall).color(k.c.text_muted));
    }
    if let Some(e) = &c.error {
        line = line.child(text(e.clone()).font(k.ty.text_xsmall).color(status::failed()));
    }
    let watch = {
        let (kk, sc) = (key.to_string(), scope.to_string());
        move |s: &mut State, on: bool, _tx: &mut Tx| {
            if !on {
                return;
            }
            let e = s.board.clear.by_key.entry(kk.clone()).or_default();
            if e.scope != sc || e.count.is_none() {
                e.scope = sc.clone();
                count(s, &kk);
            }
        }
    };
    let body = if compact {
        row().key(key.to_string()).gap(8.0).center_y().flex_wrap().child(ages).child(line)
    } else {
        col().key(key.to_string()).gap(10.0).child(ages).child(line)
    };
    body.on_visible(watch)
}
