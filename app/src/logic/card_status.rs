//! A card's group on the Board and its status words and colour (the
//! React app's needs.ts/cards.ts `cardStatus`, cut down; F0c ports the
//! rest).

use irori_window::cards::CardSummary;
use rune::Color;

use crate::ui::theme::status;

/// The Board's groups, in the order the page shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    NeedsYou,
    Working,
    Waiting,
    Open,
    NotYet,
}

pub const GROUPS: [(Group, &str); 5] = [
    (Group::NeedsYou, "Needs you"),
    (Group::Working, "Working"),
    (Group::Waiting, "Waiting"),
    (Group::Open, "Open"),
    (Group::NotYet, "Not yet"),
];

pub fn group(c: &CardSummary) -> Group {
    if c.open_blockers > 0 || c.ready {
        Group::NeedsYou
    } else if c.later.is_some() {
        Group::NotYet
    } else if c.waiting_on > 0 || c.state == "input_required" {
        Group::Waiting
    } else if c.state == "working" || c.state == "in_review" {
        Group::Working
    } else {
        Group::Open
    }
}

/// A state's colour: amber for a decision, green for work, blue for
/// review.
pub fn color(c: &CardSummary) -> Color {
    match group(c) {
        Group::NeedsYou => status::decide(),
        Group::Working if c.state == "in_review" => status::review(),
        Group::Working => status::working(),
        Group::Waiting => status::waiting(),
        Group::Open | Group::NotYet => status::idle(),
    }
}

pub fn label(c: &CardSummary) -> &'static str {
    if c.ready {
        return "Ready for you";
    }
    if c.open_blockers > 0 {
        return "Needs a decision";
    }
    match c.state.as_str() {
        "working" => "Working",
        "in_review" => "In review",
        "input_required" => "Waiting",
        "completed" => "Done",
        "failed" => "Failed",
        "canceled" => "Canceled",
        "submitted" if c.later.is_some() => "Not yet",
        _ => "Open",
    }
}

/// "tag#12", or "new" before the card has a short ref.
pub fn card_ref(c: &CardSummary) -> String {
    if c.short.is_empty() {
        return "new".into();
    }
    format!("{}#{}", c.tag, c.short)
}
