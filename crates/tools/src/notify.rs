//! The notify tool (I:internal/tools/notify.go): a card for the person at
//! this device. With actions, the person's answer goes back to the caller
//! as a next_turn notice from Irori.
//!
//! The card engine is M6's `notify` crate; here it is the [`Notifier`]
//! trait. A backend with none is headless (Go's nil `*Notifier`). A card
//! with buttons is an ask row when the backend keeps an ask book, so a
//! headless device can ask too.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gocompat::time::GoTime;

use crate::args::Ty;
use crate::format::{add, human_wait, one_line, quoted_all, sub, trim};
use crate::{Caller, Inner, decode, err_text, map_err};

/// Cards per agent per [`NOTIFY_WINDOW`].
pub const NOTIFY_LIMIT: usize = 5;
/// The sliding rate window.
pub const NOTIFY_WINDOW: Duration = Duration::from_secs(10 * 60);
/// How long a card with buttons waits for an answer.
pub const NOTIFY_ANSWER: Duration = Duration::from_secs(5 * 60);
/// A card without buttons.
const NOTIFY_INFO_LIFE: Duration = Duration::from_secs(12);
const MAX_NOTIFY_TEXT: usize = 500;
const MAX_NOTIFY_TITLE: usize = 80;
const MAX_ACTION_LABEL: usize = 24;
/// The most buttons a card shows (go-toast's limit).
pub const MAX_ACTIONS: usize = 2;

/// How the answer names who was asked.
const NOTIFY_PERSON: &str = "the person";

/// A card button. `run` is called once, after the card closes.
#[derive(Clone)]
pub struct Action {
    pub label: String,
    pub run: Arc<dyn Fn() + Send + Sync>,
}

/// One card (Go `notify.Note`, the parts the tool sets).
#[derive(Clone, Default)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub body: String,
    /// "info", "success", "warn" or "error".
    pub level: String,
    pub expire: Duration,
    pub actions: Vec<Action>,
    pub on_dismiss: Option<Arc<dyn Fn() + Send + Sync>>,
    pub on_expire: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// The cards the person sees on this device (M6's notify crate).
pub trait Notifier: Send + Sync {
    /// No desktop app runs here.
    fn headless(&self) -> bool;
    /// Not headless and the person has notifications on.
    fn enabled(&self) -> bool;
    /// Shows `note`; false when it wasn't shown.
    fn post(&self, note: Note) -> bool;
}

/// Go `notify.ParseLevel`.
fn parse_level(s: &str) -> Option<&'static str> {
    match gocompat::text::to_lower_str(trim(s)).as_str() {
        "" | "info" => Some("info"),
        "success" => Some("success"),
        "warn" => Some("warn"),
        "error" => Some("error"),
        _ => None,
    }
}

/// Go `notify.AskID`.
fn ask_id(id: &str) -> String {
    format!("ask:{id}")
}

/// The per-agent rate window and the card id counter.
#[derive(Default)]
pub(crate) struct NotifyState {
    posted: HashMap<String, Vec<GoTime>>,
    seq: u64,
    /// [`NOTIFY_ANSWER`] when zero; tests shorten it.
    pub(crate) answer: Duration,
}

impl NotifyState {
    /// Takes one of `key`'s slots in the window ending at `now`.
    fn reserve(&mut self, key: &str, now: &GoTime) -> bool {
        let win = NOTIFY_WINDOW.as_nanos() as i128;
        let kept: Vec<GoTime> = self
            .posted
            .get(key)
            .map(|v| v.iter().copied().filter(|t| sub(now, t) < win).collect())
            .unwrap_or_default();
        if kept.len() >= NOTIFY_LIMIT {
            self.posted.insert(key.into(), kept);
            return false;
        }
        let mut kept = kept;
        kept.push(*now);
        self.posted.insert(key.into(), kept);
        true
    }

    /// Gives back a slot taken at `now` (its card wasn't shown).
    fn unreserve(&mut self, key: &str, now: &GoTime) {
        if let Some(ts) = self.posted.get_mut(key)
            && let Some(i) = ts.iter().rposition(|t| t.equal(now))
        {
            ts.remove(i);
        }
    }
}

fn to_base36(mut n: u128) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut out = vec![];
    while n > 0 {
        out.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// Go `strconv.FormatInt(n, 36)`.
fn base36_i64(n: i64) -> String {
    if n < 0 {
        return format!("-{}", to_base36(n.unsigned_abs() as u128));
    }
    to_base36(n as u128)
}

const NOTIFY_ARGS: &[(&str, Ty)] = &[
    ("text", Ty::Str),
    ("title", Ty::Str),
    ("level", Ty::Str),
    ("actions", Ty::Strs),
];

struct Slot<'a> {
    s: &'a Inner,
    key: String,
    now: GoTime,
    shown: bool,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        if !self.shown {
            let mut st = self
                .s
                .notify_state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            st.unreserve(&self.key, &self.now);
        }
    }
}

pub(crate) fn notify(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, NOTIFY_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let text = trim(a.str("text")).to_string();
    let title = one_line(a.str("title"), MAX_NOTIFY_TITLE + 1);
    if text.is_empty() {
        return err_text("give text: what the person must see or decide.");
    }
    let tl = text.chars().count();
    if tl > MAX_NOTIFY_TEXT {
        return err_text(&format!(
            "text is {tl} characters; a card holds at most {MAX_NOTIFY_TEXT}. Say less, and put detail in a message or on the Board."
        ));
    }
    if title.chars().count() > MAX_NOTIFY_TITLE {
        return err_text(&format!("title is at most {MAX_NOTIFY_TITLE} characters."));
    }
    let Some(level) = parse_level(a.str("level")) else {
        return err_text("level is info, success, warn or error.");
    };
    let actions = a.strs("actions");
    if actions.len() > MAX_ACTIONS {
        return err_text(&format!("at most {MAX_ACTIONS} actions (button labels)."));
    }
    let mut labels = vec![];
    for l in &actions {
        let l = one_line(l, MAX_ACTION_LABEL + 1);
        if l.is_empty() {
            return err_text("an action label is empty.");
        }
        if l.chars().count() > MAX_ACTION_LABEL {
            return err_text(&format!(
                "an action label is at most {MAX_ACTION_LABEL} characters: one or two words."
            ));
        }
        labels.push(l);
    }

    let n = s.b.notifier();
    let headless = n.is_none_or(|n| n.headless());
    let book = s.b.asks();
    let rows = book.is_some() && !labels.is_empty();
    if headless && !rows {
        return err_text(
            "no desktop app runs on this device (Irori is headless), so there is nobody to show a card to. Send a message or use the Board.",
        );
    }
    if !headless && !n.is_some_and(|n| n.enabled()) {
        return err_text(
            "the person has notifications turned off. Send a message or use the Board instead.",
        );
    }

    let now = s.now();
    let key = c.sender.key.clone();
    let (id, wait) = {
        let mut st = s.notify_state.lock().unwrap_or_else(|e| e.into_inner());
        if !st.reserve(&key, &now) {
            return err_text(&format!(
                "you have shown the person {NOTIFY_LIMIT} notification cards in the last {} minutes, which is the limit. \
Only notify for what they must see or decide now; put the rest in a message or on the Board.",
                NOTIFY_WINDOW.as_secs() / 60
            ));
        }
        st.seq += 1;
        let id = ask_id(&format!(
            "{}-{}",
            base36_i64(now.unix_nano()),
            to_base36(u128::from(st.seq))
        ));
        let wait = if st.answer.is_zero() {
            NOTIFY_ANSWER
        } else {
            st.answer
        };
        (id, wait)
    };
    // The slot is taken from here on; a card that isn't shown gives it back.
    let mut slot = Slot {
        s,
        key: key.clone(),
        now,
        shown: false,
    };

    let mut who = trim(&c.sender.name).to_string();
    if who.is_empty() {
        who = s.roster().who(&c.sender.key);
    }
    let card_title = if title.is_empty() {
        who.clone()
    } else {
        format!("{who}: {title}")
    };
    if rows {
        if let Err(e) = s.b.inbox().record_roster() {
            // The answer may come after the caller has gone.
            return map_err(e);
        }
        let book = book.expect("rows has a book");
        let ask = coord::approvals::Ask {
            by: key.clone(),
            name: who.clone(),
            title: title.clone(),
            text: text.clone(),
            level: level.into(),
            actions: labels.clone(),
            until: add(&now, wait),
            ..coord::approvals::Ask::default()
        };
        let posted = match book.post(ask) {
            Ok(p) => p,
            Err(e) => return err_text(&format!("the card couldn't be recorded: {e}")),
        };
        slot.shown = true;
        return format!(
            "Showed the person a card ({}) on their desktop devices with {}. Their answer (the button, a dismiss, or no answer within {}) reaches you at your next turn as a message from Irori; carry on meanwhile and don't ask again.",
            ask_id(&posted.id),
            quoted_all(&labels).join(" and "),
            human_wait(wait)
        );
    }
    let n = n.expect("not headless");
    let mut note = Note {
        id: id.clone(),
        title: card_title,
        body: text.clone(),
        level: level.into(),
        ..Note::default()
    };
    if labels.is_empty() {
        note.expire = NOTIFY_INFO_LIFE;
    } else {
        let asked = if title.is_empty() {
            one_line(&text, 80)
        } else {
            title.clone()
        };
        let once = Arc::new(Mutex::new(false));
        let answer = {
            let ib = s.b.clone();
            let to = c.sender.clone();
            let id = id.clone();
            Arc::new(move |how: String| {
                let mut done = once.lock().unwrap_or_else(|e| e.into_inner());
                if *done {
                    return;
                }
                *done = true;
                notify_answer(
                    ib.inbox(),
                    &to,
                    &id,
                    &format!("You asked {NOTIFY_PERSON}: {asked}. {how}"),
                );
            })
        };
        for l in &labels {
            let (ans, l2) = (answer.clone(), l.clone());
            note.actions.push(Action {
                label: l.clone(),
                run: Arc::new(move || ans(format!("They chose: {l2}"))),
            });
        }
        let ans = answer.clone();
        note.on_dismiss = Some(Arc::new(move || {
            ans("They dismissed it without choosing.".into())
        }));
        let ans = answer.clone();
        let w = human_wait(wait);
        note.on_expire = Some(Arc::new(move || {
            ans(format!(
                "No answer: the card went away after {w} without a choice."
            ))
        }));
        note.expire = wait;
        // The answer may come after the caller has gone; record it now so
        // its key still resolves then.
        if let Err(e) = s.b.inbox().record_roster() {
            return map_err(e);
        }
    }
    if !n.post(note) {
        return err_text(
            "the card couldn't be shown (notifications may have just been turned off).",
        );
    }
    slot.shown = true;
    if labels.is_empty() {
        return format!(
            "Showed the person a card ({id}). No answer comes back for a card without actions."
        );
    }
    format!(
        "Showed the person a card ({id}) with {}. Their answer (the button, a dismiss, or no answer within {}) reaches you at your next turn as a message from Irori; carry on meanwhile and don't ask again.",
        quoted_all(&labels).join(" and "),
        human_wait(wait)
    )
}

/// Delivers a resolved ask's answer to the agent that asked, on this
/// device (the asking device decides, §6.7).
pub(crate) fn ask_resolved(s: &Inner, a: &coord::approvals::Ask) {
    if a.by.is_empty() || a.by == "irori" {
        return;
    }
    let asked = if a.title.is_empty() {
        one_line(&a.text, 80)
    } else {
        a.title.clone()
    };
    let how = if !a.answered {
        format!(
            "No answer: the card went away after {} without a choice.",
            human_wait(NOTIFY_ANSWER)
        )
    } else if a.choice.is_empty() {
        "They dismissed it without choosing.".into()
    } else {
        format!("They chose: {}", a.choice)
    };
    notify_answer(
        s.b.inbox(),
        &inbox::Sender::new(&a.by, "", ""),
        &ask_id(&a.id),
        &format!("You asked {NOTIFY_PERSON}: {asked}. {how}"),
    );
}

/// Tells the agent that asked what the person did, at its next turn, from
/// Irori itself.
pub(crate) fn notify_answer(ib: &inbox::Inbox, to: &inbox::Sender, id: &str, text: &str) {
    let r = ib.send_with(
        &inbox::system_sender(),
        &inbox::Address::new(inbox::addr::AGENT, &to.key),
        text,
        inbox::SendOptions {
            mode: inbox::mode::NEXT_TURN.into(),
            notice: true,
            reason: "notify answer".into(),
            ..inbox::SendOptions::default()
        },
    );
    if let Err(e) = r {
        eprintln!("irori: notify {id}: answer to {}: {e}", to.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base36() {
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(36), "10");
        assert_eq!(base36_i64(-37), "-11");
    }
}
