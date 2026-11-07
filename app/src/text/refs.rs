//! Card refs in text (cards #114, #144, #343): what a "#12" or "mlg#12"
//! names, its status as the chip shows it (lib/cards.ts `refStatus`,
//! `cardStatus`), and the backend lookup for a ref the window doesn't hold
//! (a done card, a card on a board it isn't showing, an old thread).
//!
//! The lookup runs off the UI thread ([`prefetch_refs`]); its answers stay
//! on `s.text.refs` until the board changes.

use irori_window::threads::RefTarget;

use super::mentions::{CardRef, RefRow, local_ref, ref_text};
use crate::ui::State;

// One source for a ref's status: logic::cards (lib/cards.ts refStatus,
// cardStatus), with a chip's tones.
pub use crate::logic::cards::{RefOf, RefStatus, card_status, ref_status};
// One source for the device palette: the faces' (coord::devcolor).
pub use crate::widgets::faces::device_color;

/// [`ref_status`] for what the backend's lookup found.
pub fn target_status(t: &RefTarget) -> RefStatus {
    ref_status(&RefOf {
        thread: t.thread,
        done: t.done,
        state: t.state.clone(),
        blocked: t.blocked,
        ready: t.ready,
        working: None,
        progress: Some((t.progress.done, t.progress.total)),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::threads::RefTarget;

    #[test]
    fn a_looked_up_ref_reads_like_the_chip() {
        let t = |f: &dyn Fn(&mut RefTarget)| {
            let mut x = RefTarget::default();
            f(&mut x);
            target_status(&x)
        };
        assert_eq!(t(&|x| x.thread = true).label, "Open thread");
        assert_eq!(t(&|x| x.state = "completed".into()).label, "Done");
        assert!(t(&|x| x.state = "canceled".into()).closed());
        // Not known whether anyone works on it: a working card says so.
        assert_eq!(t(&|x| x.state = "working".into()).label, "Working");
    }
}
