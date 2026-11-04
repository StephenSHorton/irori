//! Work and personal sides (card #494, work-personal-separation §7.5).
//!
//! A private card carries a side: a short label such as "Work" or
//! "Personal", kept apart from the board it is on. Each device says which
//! sides it runs with a replicated `device.sides` row, set on that device
//! in Settings → Devices. The row counts only when the device that signed
//! it is the one it names, so one computer can't pin another. With no row
//! (or an empty one) a device runs every side, so nothing changes until
//! the person pins. The first side a device lists is the one new cards
//! typed on it take.
//!
//! The side is a label for who runs a card, not who reads it: every
//! device still holds every card.

use std::collections::{BTreeMap, HashMap};

use crate::gojson::{decode, omit};

/// The replicated row a pin writes: `{deviceId, sides, atDevice}`.
pub const KIND: &str = store::kinds::DEVICE_SIDES;

/// The longest side name, in characters.
pub const MAX_LEN: usize = 40;

/// The most sides one device lists.
pub const MAX_SIDES: usize = 8;

/// A side name as it is kept: one line, trimmed, at most [`MAX_LEN`]
/// characters. `""` is no side.
pub fn clean(side: &str) -> String {
    let one: String = side
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let words: Vec<&str> = one.split_whitespace().collect();
    words.join(" ").chars().take(MAX_LEN).collect()
}

/// Whether two side names are the same side: case doesn't count.
pub fn same(a: &str, b: &str) -> bool {
    clean(a).to_lowercase() == clean(b).to_lowercase()
}

/// A side list as it is kept: each cleaned, empty ones and repeats
/// (any case) dropped, at most [`MAX_SIDES`], order kept.
pub fn clean_list(sides: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in sides {
        let c = clean(s);
        if c.is_empty() || out.iter().any(|o| same(o, &c)) {
            continue;
        }
        out.push(c);
        if out.len() == MAX_SIDES {
            break;
        }
    }
    out
}

crate::go_struct! {
    /// A `device.sides` row.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct SidesRow {
        pub device_id: String = "deviceId",
        /// The sides it runs, the first its new cards'. Empty: every side.
        pub sides: Vec<String> = "sides" "omit",
        pub at_device: String = "atDevice" "omit",
    }
}

#[derive(Debug, Clone)]
struct Pick {
    sides: Vec<String>,
    hlc: u64,
    seq: i64,
}

/// Folds `device.sides` rows: each device's latest list, as that device
/// wrote it.
#[derive(Debug, Clone, Default)]
pub struct Sides {
    m: HashMap<String, Pick>,
}

impl Sides {
    /// An empty fold: every device runs every side.
    pub fn new() -> Sides {
        Sides::default()
    }

    /// Folds one row; reports whether a device's sides changed. Only the
    /// person pins (actor `user`), and only on the device the row names:
    /// a row one device signs about another is ignored.
    pub fn apply(&mut self, ev: &store::Event) -> bool {
        if ev.kind != KIND || ev.actor != "user" {
            return false;
        }
        let Some(r) = decode::<SidesRow>(&ev.payload) else {
            return false;
        };
        if r.device_id.is_empty() || r.device_id != ev.device_id {
            return false;
        }
        let p = Pick {
            sides: clean_list(&r.sides),
            hlc: ev.hlc.0,
            seq: ev.seq,
        };
        if let Some(cur) = self.m.get(&r.device_id)
            && (cur.hlc, cur.seq) >= (p.hlc, p.seq)
        {
            return false;
        }
        let changed = self.m.get(&r.device_id).map(|c| &c.sides) != Some(&p.sides);
        self.m.insert(r.device_id, p);
        changed
    }

    /// The sides `device` runs, or None when it runs every side (no row,
    /// or an empty one).
    pub fn of(&self, device: &str) -> Option<&[String]> {
        self.m
            .get(device)
            .map(|p| p.sides.as_slice())
            .filter(|s| !s.is_empty())
    }

    /// Whether `device` runs a card on `side`. A card with no side runs
    /// anywhere, and a device with no pin runs every side.
    pub fn runs(&self, device: &str, side: &str) -> bool {
        if clean(side).is_empty() {
            return true;
        }
        match self.of(device) {
            None => true,
            Some(list) => list.iter().any(|s| same(s, side)),
        }
    }

    /// The side a new card typed on `device` takes: the first it lists,
    /// or "" when it runs every side.
    pub fn home(&self, device: &str) -> String {
        self.of(device)
            .and_then(|s| s.first())
            .cloned()
            .unwrap_or_default()
    }

    /// Every pinned device's sides, by device id.
    pub fn all(&self) -> BTreeMap<String, Vec<String>> {
        self.m
            .iter()
            .filter(|(_, p)| !p.sides.is_empty())
            .map(|(d, p)| (d.clone(), p.sides.clone()))
            .collect()
    }

    /// Every side some device lists, in the order first seen by device id.
    pub fn names(&self) -> Vec<String> {
        let all: Vec<String> = self.all().into_values().flatten().collect();
        let mut out: Vec<String> = Vec::new();
        for s in all {
            if !out.iter().any(|o| same(o, &s)) {
                out.push(s);
            }
        }
        out
    }
}

/// What an agent on a device is told about sides (card #496): the sides
/// its device runs, and that cards on any other side are read, never
/// taken, unless the person asks. `runs` is the device's pin (None: it
/// runs every side, and there is nothing to say); `names` every side named
/// anywhere, for the ones it doesn't run.
pub fn agent_line(runs: Option<&[String]>, names: &[String]) -> String {
    let Some(runs) = runs.filter(|r| !r.is_empty()) else {
        return String::new();
    };
    let others: Vec<&str> = names
        .iter()
        .filter(|n| !runs.iter().any(|r| same(r, n)))
        .map(String::as_str)
        .collect();
    let what = if others.is_empty() {
        "Cards on any other side".to_string()
    } else {
        format!("Cards on {}", others.join(" or "))
    };
    format!(
        "This device runs the {} side{}. {what} are only read here, never taken: don't claim, join or assign them unless the person asks you to (they assign it to you, start an agent on it, or @-mention you on it); Irori refuses it otherwise.",
        runs.join(" and "),
        if runs.len() == 1 { "" } else { "s" },
    )
}
