//! The space switcher's pure half (the React app's lib/spaces.ts, cards
//! #393, #461, #538, #558): All spaces, Personal, then each shared space,
//! and which cards, boards and conversations each pick shows. The pick
//! itself is the spaces package's (a pref).

use irori_window::boards::{BoardSummary, SpaceBoard};
use irori_window::cards::CardSummary;
use irori_window::talk::TalkConversation;

/// The two picks that aren't a shared space.
pub const ALL_SPACES: &str = "";
pub const PERSONAL: &str = "personal";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Space {
    /// [`ALL_SPACES`], [`PERSONAL`], or a shared space's id.
    pub id: String,
    pub name: String,
    /// The other people in it, for "with Blake".
    pub others: Vec<String>,
}

/// Case-insensitive order, as `localeCompare` sorts names.
pub fn name_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b))
}

/// The switcher's list: All spaces, Personal, then each shared space by name.
pub fn spaces_of(boards: &[BoardSummary]) -> Vec<Space> {
    let mut shared: Vec<&BoardSummary> = boards.iter().filter(|b| !b.leaving).collect();
    shared.sort_by(|a, b| name_order(&a.name, &b.name));
    let mut out = vec![
        Space {
            id: ALL_SPACES.into(),
            name: "All spaces".into(),
            others: vec![],
        },
        Space {
            id: PERSONAL.into(),
            name: "Personal".into(),
            others: vec![],
        },
    ];
    out.extend(shared.into_iter().map(|b| Space {
        id: b.board_id.clone(),
        name: [&b.name, &b.tag]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "(unnamed)".into()),
        others: b.members.iter().filter(|m| !m.me).map(|m| m.name.clone()).collect(),
    }));
    out
}

/// A space's boards, its default first; empty names fall back to the
/// space's (a joined space not ready yet).
pub fn boards_of(s: &BoardSummary) -> Vec<SpaceBoard> {
    if s.boards.is_empty() {
        return vec![SpaceBoard {
            board_id: s.board_id.clone(),
            name: s.name.clone(),
            tag: s.tag.clone(),
        }];
    }
    s.boards
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let mut x = x.clone();
            if i == 0 {
                if x.name.is_empty() {
                    x.name = s.name.clone();
                }
                if x.tag.is_empty() {
                    x.tag = s.tag.clone();
                }
            }
            x
        })
        .collect()
}

/// "with Blake", "with Blake and Casey", "with Blake +2"; "" alone.
pub fn with_line(others: &[String]) -> String {
    match others {
        [] => String::new(),
        [a] => format!("with {a}"),
        [a, b] => format!("with {a} and {b}"),
        [a, rest @ ..] => format!("with {a} +{}", rest.len()),
    }
}

/// A pick that still names a space; one the person left falls back to All.
pub fn known(id: &str, boards: &[BoardSummary]) -> String {
    if id == ALL_SPACES || id == PERSONAL {
        return id.into();
    }
    if boards.iter().any(|b| b.board_id == id && !b.leaving) {
        id.into()
    } else {
        ALL_SPACES.into()
    }
}

/// Personal's cards on one of its boards; a card with no inner board is on
/// the default.
pub fn cards_on_personal<'a>(cards: &'a [CardSummary], board_id: &str) -> Vec<&'a CardSummary> {
    cards
        .iter()
        .filter(|c| {
            c.board.is_empty()
                && (if c.space_board.is_empty() { PERSONAL } else { c.space_board.as_str() }) == board_id
        })
        .collect()
}

/// Whether a card shows in the picked space.
pub fn card_in_space(c: &CardSummary, space: &str) -> bool {
    match space {
        ALL_SPACES => true,
        PERSONAL => c.board.is_empty(),
        s => c.board == s,
    }
}

/// Whether a channel, DM or thread shows in the picked space; a DM with
/// another person shows under every pick (#538).
pub fn conv_in_space(space_id: &str, kind: &str, members: &[(String, String)], space: &str) -> bool {
    if space == ALL_SPACES {
        return true;
    }
    if kind == "dm"
        && members.iter().all(|(k, _)| k == "person")
        && members.iter().any(|(_, key)| key.starts_with("person:"))
    {
        return true;
    }
    if space == PERSONAL {
        return space_id.is_empty();
    }
    space_id == space
}

/// [`conv_in_space`] for a Talk conversation.
pub fn talk_in_space(c: &TalkConversation, space: &str) -> bool {
    let members: Vec<(String, String)> = c.members.iter().map(|m| (m.kind.clone(), m.key.clone())).collect();
    conv_in_space(&c.space_id, &c.kind, &members, space)
}

/// The Boards page in the picked space.
pub fn boards_in_space<'a>(boards: &'a [BoardSummary], space: &str) -> Vec<&'a BoardSummary> {
    match space {
        ALL_SPACES => boards.iter().collect(),
        PERSONAL => vec![],
        s => boards.iter().filter(|b| b.board_id == s).collect(),
    }
}

/// The board id a new thread or channel starts on.
pub fn threads_space(space: &str) -> &str {
    if space == ALL_SPACES || space == PERSONAL { "" } else { space }
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::boards::SummaryMember;

    fn mlg() -> BoardSummary {
        let m = |id: &str, name: &str, me: bool| SummaryMember {
            person_id: id.into(),
            name: name.into(),
            me,
            ..SummaryMember::default()
        };
        BoardSummary {
            board_id: "b-mlg".into(),
            name: "MLG".into(),
            tag: "mlg".into(),
            members: vec![m("me", "Stephen", true), m("p1", "Blake", false)],
            ..BoardSummary::default()
        }
    }
    fn b(id: &str, name: &str, leaving: bool) -> BoardSummary {
        BoardSummary {
            board_id: id.into(),
            name: name.into(),
            tag: name.to_lowercase(),
            leaving,
            ..BoardSummary::default()
        }
    }
    fn c(id: &str, board: &str, space_board: &str) -> CardSummary {
        CardSummary {
            id: id.into(),
            board: board.into(),
            space_board: space_board.into(),
            ..CardSummary::default()
        }
    }

    #[test]
    fn the_switcher_lists_all_personal_then_by_name() {
        let s = spaces_of(&[mlg(), b("b-iro", "Irori", false), b("b-gone", "Gone", true)]);
        let ids: Vec<&str> = s.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, [ALL_SPACES, PERSONAL, "b-iro", "b-mlg"]);
        assert_eq!(s[3].others, ["Blake"]);
    }

    #[test]
    fn with_lines() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(with_line(&[]), "");
        assert_eq!(with_line(&v(&["Blake"])), "with Blake");
        assert_eq!(with_line(&v(&["Blake", "Addam"])), "with Blake and Addam");
        assert_eq!(with_line(&v(&["Blake", "Addam", "Casey"])), "with Blake +2");
    }

    #[test]
    fn a_pick_of_a_space_you_left_goes_back_to_all() {
        assert_eq!(known("b-mlg", &[mlg()]), "b-mlg");
        assert_eq!(known("b-gone", &[mlg(), b("b-gone", "Gone", true)]), ALL_SPACES);
        assert_eq!(known("nope", &[mlg()]), ALL_SPACES);
        assert_eq!(known(PERSONAL, &[]), PERSONAL);
    }

    #[test]
    fn cards_follow_the_pick() {
        let private = c("a", "", "");
        let on_mlg = CardSummary {
            scope: "board".into(),
            ..c("b", "b-mlg", "")
        };
        assert!(card_in_space(&private, ALL_SPACES) && card_in_space(&on_mlg, ALL_SPACES));
        assert!(card_in_space(&private, PERSONAL));
        assert!(!card_in_space(&on_mlg, PERSONAL));
        assert!(card_in_space(&on_mlg, "b-mlg"));
        assert!(!card_in_space(&private, "b-mlg"));
    }

    #[test]
    fn boards_and_threads_follow_the_pick() {
        let all = [mlg(), b("b-iro", "Irori", false)];
        assert_eq!(boards_in_space(&all, ALL_SPACES).len(), 2);
        assert!(boards_in_space(&all, PERSONAL).is_empty());
        assert_eq!(boards_in_space(&all, "b-iro")[0].board_id, "b-iro");
        assert_eq!(threads_space(ALL_SPACES), "");
        assert_eq!(threads_space(PERSONAL), "");
        assert_eq!(threads_space("b-mlg"), "b-mlg");
    }

    #[test]
    fn conversations_follow_the_pick() {
        let ids = |space: &str| {
            [("c1", "channel", ""), ("c2", "channel", "b-mlg"), ("d1", "dm", "")]
                .iter()
                .filter(|(_, kind, sp)| conv_in_space(sp, kind, &[], space))
                .map(|(id, ..)| *id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(ALL_SPACES), ["c1", "c2", "d1"]);
        assert_eq!(ids(PERSONAL), ["c1", "d1"]);
        assert_eq!(ids("b-mlg"), ["c2"]);
        assert!(ids("b-iro").is_empty());
        // Card #538: a DM with another person shows under every pick.
        let bob = [("person".to_string(), "user".to_string()), ("person".to_string(), "person:bob".to_string())];
        for space in [ALL_SPACES, PERSONAL, "b-mlg", "b-iro"] {
            assert!(conv_in_space("", "dm", &bob, space));
        }
    }

    #[test]
    fn cards_on_personal_boards() {
        let cards = [
            c("d", "", PERSONAL),
            c("o", "", ""),
            CardSummary {
                tag: "home".into(),
                ..c("h", "", "b-home")
            },
            CardSummary {
                tag: "mlg".into(),
                ..c("s", "b-mlg", "b-mlg")
            },
        ];
        let ids = |v: Vec<&CardSummary>| v.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(cards_on_personal(&cards, PERSONAL)), ["d", "o"]);
        assert_eq!(ids(cards_on_personal(&cards, "b-home")), ["h"]);
    }

    #[test]
    fn boards_of_falls_back_to_the_space() {
        let mut s = mlg();
        assert_eq!(boards_of(&s)[0].name, "MLG");
        s.boards = vec![
            SpaceBoard {
                board_id: "b-mlg".into(),
                ..SpaceBoard::default()
            },
            SpaceBoard {
                board_id: "b2".into(),
                name: "Art".into(),
                tag: "art".into(),
            },
        ];
        let got = boards_of(&s);
        assert_eq!((got[0].name.as_str(), got[0].tag.as_str()), ("MLG", "mlg"));
        assert_eq!(got[1].name, "Art");
    }
}
