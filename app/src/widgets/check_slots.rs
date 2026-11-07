//! The sidebar's line of check.sh runs holding this device's check slots
//! (the React app's cards/check-slots.tsx, card #437): at most two full
//! checks build at once. Nothing shows while no run holds one. The React
//! app polled every 10 s; here the slots are re-read on the board and
//! roster events and on the widgets' own 10 s tick ([`super::poll`]).

use irori_window::rune_widgets::CheckSlot;
use rune::{El, col, row, text};

use crate::ui::{Frame, State};

/// How often the slots are read again: check.sh renews its lease every
/// 30 s, so a run that ended shows within a poll.
pub const POLL: std::time::Duration = std::time::Duration::from_secs(10);

/// The N of check/<host>/slot-N.
pub fn slot_number(resource: &str) -> String {
    resource
        .rsplit_once("/slot-")
        .map(|(_, n)| n)
        .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or("?")
        .to_string()
}

/// Reads the slots off the UI thread.
pub fn refresh(s: &mut State) {
    s.widgets.slots_at = Some(std::time::Instant::now());
    s.spawn(
        |w| w.check_slots(),
        |s, v: Vec<CheckSlot>| s.widgets.check_slots = v,
    );
}

pub fn check_slots(s: &State, f: &Frame) -> Option<El> {
    if s.widgets.check_slots.is_empty() {
        return None;
    }
    let k = f.k;
    let mut c = col()
        .key("widgets-check-slots")
        .w_full()
        .px(10.0)
        .pt(8.0)
        .gap(2.0)
        .child(
            text("Check slots")
                .font(k.ty.text_xsmall_medium)
                .color(k.c.text_muted),
        );
    for sl in &s.widgets.check_slots {
        c = c.child(
            row()
                .key(format!("widgets-slot-{}", sl.resource))
                .w_full()
                .help(format!(
                    "{}: held by {} (scripts/check.sh)",
                    sl.resource, sl.run
                ))
                .child(
                    text(format!("{} · {}", slot_number(&sl.resource), sl.run))
                        .font(k.ty.text_xsmall)
                        .color(k.c.text_muted)
                        .max_lines(1),
                ),
        );
    }
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_numbers() {
        assert_eq!(slot_number("check/mac/slot-2"), "2");
        assert_eq!(slot_number("check/mac/slot-"), "?");
        assert_eq!(slot_number("check/mac"), "?");
    }
}
