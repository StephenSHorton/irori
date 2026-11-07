//! What the sidebar lists (the React app's app-sidebar.tsx and App.tsx's
//! `boardDecide`): the Spaces count, and the pinned spaces and cards with
//! the marks and names their rows show.

use irori_window::boards::BoardSummary;
use irori_window::cards::CardSummary;

use super::plain::plain_line;
use super::spaces::{boards_in_space, card_in_space};

/// A card on a shared board waiting on the person: an open blocker, a
/// wait on a PR's merge (card #509) or a wait no agent will end (#520).
pub fn board_decides(c: &CardSummary) -> bool {
    c.scope == "board" && (c.open_blockers > 0 || c.merge_pr.is_some() || c.gone_wait.is_some())
}

/// The red count by Spaces: the picked space's board cards waiting on the
/// person.
pub fn boards_need_you(cards: &[CardSummary], space: &str) -> usize {
    cards
        .iter()
        .filter(|c| card_in_space(c, space) && board_decides(c))
        .count()
}

/// The pinned spaces the picked space shows, in pin order.
pub fn pinned_spaces<'a>(boards: &'a [BoardSummary], space: &str) -> Vec<&'a BoardSummary> {
    let mut v: Vec<_> = boards_in_space(boards, space)
        .into_iter()
        .filter(|b| b.pinned)
        .collect();
    v.sort_by(|a, b| a.pin_rank.total_cmp(&b.pin_rank));
    v
}

/// The pinned cards, in pin order.
pub fn pinned_cards(cards: &[CardSummary]) -> Vec<&CardSummary> {
    let mut v: Vec<_> = cards.iter().filter(|c| c.pinned).collect();
    v.sort_by(|a, b| a.pin_rank.total_cmp(&b.pin_rank));
    v
}

/// A pin row's words: the short mark in its chip, the name, the tooltip.
#[derive(Debug, PartialEq)]
pub struct Pin {
    pub mark: String,
    pub label: String,
    pub tip: String,
}

/// A pinned space: its tag's first two letters, its name.
pub fn space_pin(b: &BoardSummary) -> Pin {
    let name = plain_line(&b.name);
    let tag = plain_line(&b.tag);
    Pin {
        mark: tag.chars().take(2).collect(),
        label: if name.is_empty() {
            "(unnamed)".into()
        } else {
            name.clone()
        },
        tip: format!("{name} ({tag}#)"),
    }
}

/// A pinned card: "tag#12" (or "#12"), its title.
pub fn card_pin(c: &CardSummary) -> Pin {
    let title = plain_line(&c.title);
    Pin {
        mark: format!("{}#{}", plain_line(&c.tag), plain_line(&c.short)),
        label: title.clone(),
        tip: title,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::spaces::{ALL_SPACES, PERSONAL};

    fn card(id: &str, board: &str, scope: &str, blockers: i64) -> CardSummary {
        CardSummary {
            id: id.into(),
            board: board.into(),
            scope: scope.into(),
            open_blockers: blockers,
            ..Default::default()
        }
    }

    #[test]
    fn spaces_count_is_the_picked_spaces_board_decisions() {
        let cards = vec![
            card("a", "b1", "board", 1),
            card("b", "b2", "board", 2),
            card("c", "", "mine", 1),
            card("d", "b1", "board", 0),
        ];
        assert_eq!(boards_need_you(&cards, ALL_SPACES), 2);
        assert_eq!(boards_need_you(&cards, "b1"), 1);
        assert_eq!(boards_need_you(&cards, PERSONAL), 0);
    }

    #[test]
    fn pins_sort_by_rank_and_follow_the_space() {
        let b = |id: &str, pinned: bool, rank: f64| BoardSummary {
            board_id: id.into(),
            name: id.into(),
            pinned,
            pin_rank: rank,
            ..Default::default()
        };
        let boards = vec![b("x", true, 2.0), b("y", false, 0.0), b("z", true, 1.0)];
        let ids: Vec<_> = pinned_spaces(&boards, ALL_SPACES)
            .iter()
            .map(|b| b.board_id.as_str())
            .collect();
        assert_eq!(ids, ["z", "x"]);
        assert_eq!(pinned_spaces(&boards, "x").len(), 1);
        assert!(pinned_spaces(&boards, PERSONAL).is_empty());

        let mut c1 = card("1", "", "mine", 0);
        c1.pinned = true;
        c1.pin_rank = 3.0;
        let mut c2 = card("2", "", "mine", 0);
        c2.pinned = true;
        c2.pin_rank = 1.0;
        let cards = vec![c1, card("3", "", "mine", 0), c2];
        let ids: Vec<_> = pinned_cards(&cards).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["2", "1"]);
    }

    #[test]
    fn pin_marks() {
        let b = BoardSummary {
            name: "Dent\u{200b}Charter".into(),
            tag: "dent".into(),
            ..Default::default()
        };
        assert_eq!(
            space_pin(&b),
            Pin {
                mark: "de".into(),
                label: "Dent Charter".into(),
                tip: "Dent Charter (dent#)".into()
            }
        );
        assert_eq!(space_pin(&BoardSummary::default()).label, "(unnamed)");
        let c = CardSummary {
            tag: "mlg".into(),
            short: "12".into(),
            title: "Fix it".into(),
            ..Default::default()
        };
        assert_eq!(card_pin(&c).mark, "mlg#12");
        let c = CardSummary {
            short: "7".into(),
            ..Default::default()
        };
        assert_eq!(card_pin(&c).mark, "#7");
    }
}
