//! Card refs in text (cards #114, #144, #343): what a "#12" or "mlg#12"
//! names, its status as the chip shows it (lib/cards.ts `refStatus`,
//! `cardStatus`), and the backend lookup for a ref the window doesn't hold
//! (a done card, a card on a board it isn't showing, an old thread).
//!
//! The lookup runs off the UI thread ([`prefetch_refs`]); its answers stay
//! on `s.text.refs` until the board changes.

use irori_window::cards::CardSummary;
use irori_window::threads::RefTarget;
use rune::Color;

use super::mentions::{CardRef, RefRow, card_refs, local_ref, ref_text};
use crate::ui::State;
use crate::ui::theme::status;

/// A ref chip's colour family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Info,
    Destructive,
    Success,
    Warning,
    Slate,
}

/// A card's state as its ref chip shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefStatus {
    pub label: &'static str,
    pub tone: Tone,
    /// Steps done and in all, when it has steps.
    pub progress: Option<(i64, i64)>,
}

impl RefStatus {
    /// Done, failed, canceled or resolved: its title steps back.
    pub fn closed(&self) -> bool {
        matches!(self.label, "Done" | "Failed" | "Canceled" | "Resolved")
    }
}

/// lib/cards.ts `refStatus`.
#[allow(clippy::too_many_arguments)]
pub fn ref_status(
    thread: bool,
    done: bool,
    state: &str,
    blocked: bool,
    ready: bool,
    working: Option<bool>,
    progress: (i64, i64),
) -> RefStatus {
    let progress = (!thread && progress.1 > 0).then_some(progress);
    let s = |label, tone| RefStatus { label, tone, progress };
    if thread {
        return if done { s("Resolved", Tone::Slate) } else { s("Open thread", Tone::Info) };
    }
    match state {
        "completed" => s("Done", Tone::Success),
        "failed" => s("Failed", Tone::Warning),
        "canceled" => s("Canceled", Tone::Slate),
        _ if ready => s("Ready", Tone::Success),
        _ if blocked || state == "input_required" || state == "blocked" => s("Blocked", Tone::Destructive),
        "in_review" => s("In review", Tone::Warning),
        "working" if working != Some(false) => s("Working", Tone::Info),
        _ => s("Open", Tone::Slate),
    }
}

/// lib/cards.ts `cardStatus`: [`ref_status`] for a card the window holds.
pub fn card_status(c: &CardSummary) -> RefStatus {
    ref_status(
        false,
        false,
        &c.state,
        (c.open_blockers > 0 && !(c.ready && c.open_blockers == 1)) || c.state == "input_required",
        c.ready && c.open_blockers == 1,
        Some(!c.working.is_empty()),
        (c.progress.done, c.progress.total),
    )
}

/// [`ref_status`] for what the backend's lookup found.
pub fn target_status(t: &RefTarget) -> RefStatus {
    ref_status(
        t.thread,
        t.done,
        &t.state,
        t.blocked,
        t.ready,
        None,
        (t.progress.done, t.progress.total),
    )
}

/// A tone's dot colour.
pub fn tone_color(t: Tone) -> Color {
    match t {
        Tone::Info => status::review(),
        Tone::Destructive => status::failed(),
        Tone::Success => status::working(),
        Tone::Warning => status::decide(),
        Tone::Slate => status::idle(),
    }
}

/// A device's colour by its palette name (coord::devcolor::PALETTE; the
/// React app's `--device-*`), or None for none.
pub fn device_color(name: &str) -> Option<Color> {
    let (l, c, h) = match name {
        "teal" => (0.76, 0.12, 185.0),
        "orange" => (0.74, 0.14, 50.0),
        "pink" => (0.74, 0.14, 350.0),
        "lime" => (0.8, 0.15, 125.0),
        "cyan" => (0.78, 0.11, 215.0),
        "indigo" => (0.7, 0.13, 275.0),
        "sand" => (0.78, 0.06, 80.0),
        _ => return None,
    };
    Some(rune::color::oklch(l, c, h))
}

/// What a ref in text shows as: a card or thread to open, with its title
/// and status; None leaves it text.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub id: String,
    pub thread: bool,
    pub title: String,
    pub status: RefStatus,
}

/// Resolves `r` from the live cards, else the lookup's answer.
pub fn resolve(s: &State, r: &CardRef) -> Option<Resolved> {
    let rows: Vec<RefRow> = s.data.cards.iter().map(RefRow::from).collect();
    if let Some(hit) = local_ref(r, &rows, &[]) {
        let c = s.data.card(&hit.id)?;
        return Some(Resolved {
            id: hit.id,
            thread: false,
            title: hit.title,
            status: card_status(c),
        });
    }
    let t = s.text.refs.get(&ref_text(r))?.as_ref()?;
    Some(Resolved {
        id: t.id.clone(),
        thread: t.thread,
        title: t.title.clone(),
        status: target_status(t),
    })
}

/// The refs in `text` the window can't resolve by itself and hasn't
/// asked about yet.
pub fn unresolved(s: &State, text: &str) -> Vec<String> {
    if !text.contains('#') {
        return Vec::new();
    }
    let rows: Vec<RefRow> = s.data.cards.iter().map(RefRow::from).collect();
    let mut out: Vec<String> = Vec::new();
    for r in card_refs(text) {
        let t = ref_text(&r);
        if local_ref(&r, &rows, &[]).is_none()
            && !s.text.refs.contains_key(&t)
            && !s.text.refs_pending.contains(&t)
            && !out.contains(&t)
        {
            out.push(t);
        }
    }
    out
}

/// Looks up each ref in `refs` the window doesn't hold (off the UI thread),
/// so the rich text that shows it can make it a chip. Rich text calls this
/// itself when it comes on screen; a page may call it when it loads.
pub fn prefetch_refs(s: &mut State, refs: Vec<String>) {
    let refs: Vec<String> = refs
        .into_iter()
        .filter(|r| !s.text.refs.contains_key(r) && !s.text.refs_pending.contains(r))
        .collect();
    if refs.is_empty() {
        return;
    }
    s.text.refs_pending.extend(refs.iter().cloned());
    let rev = s.text.refs_rev;
    s.spawn(
        move |w| refs.into_iter().map(|r| (w.card_ref(&r), r)).collect::<Vec<_>>(),
        move |s, found| {
            for (t, r) in found {
                s.text.refs_pending.remove(&r);
                if s.text.refs_rev == rev {
                    s.text.refs.insert(r, t);
                }
            }
        },
    );
}

/// [`prefetch_refs`] for every ref in `text`.
pub fn prefetch_text(s: &mut State, text: &str) {
    let refs = unresolved(s, text);
    prefetch_refs(s, refs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_status_reads_like_the_chip() {
        assert_eq!(ref_status(true, false, "", false, false, None, (0, 0)).label, "Open thread");
        assert_eq!(ref_status(true, true, "", false, false, None, (1, 2)).progress, None);
        assert_eq!(ref_status(false, false, "completed", false, false, None, (0, 0)).label, "Done");
        assert_eq!(ref_status(false, false, "submitted", false, true, None, (0, 0)).label, "Ready");
        assert_eq!(ref_status(false, false, "submitted", true, false, None, (0, 0)).tone, Tone::Destructive);
        assert_eq!(ref_status(false, false, "working", false, false, Some(false), (0, 0)).label, "Open");
        let w = ref_status(false, false, "working", false, false, None, (2, 5));
        assert_eq!((w.label, w.progress), ("Working", Some((2, 5))));
        assert!(ref_status(false, false, "canceled", false, false, None, (0, 0)).closed());
    }
}
