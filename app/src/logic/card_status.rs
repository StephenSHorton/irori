//! A card's group on the Board (the React app's needs.ts grouping, cut
//! down); its status words and colour are logic::cards `card_status`.

use irori_window::cards::CardSummary;

/// The Board's groups, in the order the page shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    NeedsYou,
    Working,
    Waiting,
    Open,
    NotYet,
}

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

/// "tag#12", or "new" before the card has a short ref.
pub fn card_ref(c: &CardSummary) -> String {
    if c.short.is_empty() {
        return "new".into();
    }
    format!("{}#{}", c.tag, c.short)
}
