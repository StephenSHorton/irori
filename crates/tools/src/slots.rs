//! The slot tools (card #1168): `slot_take` takes one of N named slots
//! (coord's slot leases, `<name>/slot-1`..`-N`) and `slot_release` gives
//! it back, so an agent needs no curl and token for `POST /v1/slots`
//! (crates/api/src/slots.rs, which scripts keep). The holder is the
//! calling agent, so the gone sweep frees its slots when it goes away;
//! while it lives, each of its tool calls renews them ([`renew`]).

use std::time::Duration;

use crate::args::Ty;
use crate::format::{MAX_FIELD_RUNES, in_, one_line, trim};
use crate::{Caller, Inner, decode, err_text, map_err};

/// How long a slot lasts without a renewal.
pub(crate) const SLOT_TTL: Duration = Duration::from_secs(30 * 60);
/// The longest `wait_seconds` a take blocks for.
const MAX_WAIT: i64 = 120;
const POLL: Duration = Duration::from_secs(1);

const TAKE_ARGS: &[(&str, Ty)] = &[
    ("name", Ty::Str),
    ("limit", Ty::OptInt),
    ("wait_seconds", Ty::OptInt),
];

pub(crate) fn slot_take(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, TAKE_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let name = trim(a.str("name"));
    if name.is_empty() {
        return err_text(
            "give name: a plain name like check/mac, shared by everyone who takes it.",
        );
    }
    let max = coord::MAX_SLOTS as i64;
    let Some(limit) = a.opt_int("limit") else {
        return err_text(&format!(
            "give limit: how many may hold {name} at once (1 to {max})."
        ));
    };
    if !(1..=max).contains(&limit) {
        return err_text(&format!("limit must be 1 to {max}, not {limit}."));
    }
    let wait = a.opt_int("wait_seconds").unwrap_or(0);
    if !(0..=MAX_WAIT).contains(&wait) {
        return err_text(&format!(
            "wait_seconds must be 0 to {MAX_WAIT}, not {wait}."
        ));
    }
    let co = s.b.coord();
    let mut waited = Duration::ZERO;
    loop {
        match co.acquire_slot(&c.sender.key, name, limit as usize, SLOT_TTL) {
            Ok(l) => {
                return format!(
                    "Took {} until {} ({}). It renews while you work; slot_release {} when done.",
                    one_line(&l.resource, MAX_FIELD_RUNES),
                    l.until.format_rfc3339(),
                    in_(&s.now(), &l.until),
                    one_line(name, MAX_FIELD_RUNES)
                );
            }
            Err(e) if e.kind() == coord::ErrorKind::Held => {
                if waited.as_secs() as i64 >= wait {
                    return err_text(&format!("{} {}", held_by(s, c, name), retry(wait)));
                }
                std::thread::sleep(POLL);
                waited += POLL;
            }
            Err(e) => return map_err(e),
        }
    }
}

/// "<name>: all slots are held by a, b." from the live leases.
fn held_by(s: &Inner, c: &Caller, name: &str) -> String {
    let base = format!("{}/slot-", name.trim());
    let n = s.roster();
    let mut who: Vec<String> =
        s.b.coord()
            .leases()
            .into_iter()
            .filter(|l| l.resource.starts_with(&base) && l.holder != c.sender.key)
            .map(|l| n.who(&l.holder))
            .collect();
    who.dedup();
    format!(
        "every {} slot is held by {}.",
        one_line(name, MAX_FIELD_RUNES),
        who.join(", ")
    )
}

fn retry(waited: i64) -> String {
    if waited > 0 {
        format!("Still held after {waited}s; try again, or work on something else.")
    } else {
        "Try again, or pass wait_seconds to wait for one.".into()
    }
}

const RELEASE_ARGS: &[(&str, Ty)] = &[("name", Ty::Str)];

pub(crate) fn slot_release(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, RELEASE_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let name = trim(a.str("name"));
    if name.is_empty() {
        return err_text("give name, as you gave it to slot_take.");
    }
    match s.b.coord().release_slot(&c.sender.key, name) {
        Ok(true) => format!("Released your {} slot.", one_line(name, MAX_FIELD_RUNES)),
        Ok(false) => format!("You held no {} slot.", one_line(name, MAX_FIELD_RUNES)),
        Err(e) => map_err(e),
    }
}

/// Renews the caller's slots that are under half spent; a failure is
/// left for the next call (the slot then ends at its until).
pub(crate) fn renew(s: &Inner, c: &Caller) {
    let _ = s.b.coord().renew_slots(&c.sender.key, SLOT_TTL);
}
