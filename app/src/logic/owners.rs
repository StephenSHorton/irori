//! Whose a shared board's card is (the React app's lib/owners.ts, card
//! #268): the member who made it, by themselves or through an agent; they
//! answer for it, whoever holds it.

use irori_window::boards::{BoardSummary, SummaryMember};
use irori_window::cards::CardSummary;

pub const FORMER_MEMBER: &str = "Former member";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub person_id: String,
    pub name: String,
    /// The person at this device.
    pub me: bool,
    /// A current member; false for one who left (named "Former member").
    pub known: bool,
}

/// Person id `person_id` among a board's members: `None` for no id, else
/// that member, or "Former member" when they aren't on it now.
pub fn member_of(person_id: &str, members: &[SummaryMember]) -> Option<Owner> {
    if person_id.is_empty() {
        return None;
    }
    Some(match members.iter().find(|m| m.person_id == person_id) {
        None => Owner {
            person_id: person_id.into(),
            name: FORMER_MEMBER.into(),
            me: false,
            known: false,
        },
        Some(m) => Owner {
            person_id: person_id.into(),
            name: if m.name.trim().is_empty() {
                "Someone".into()
            } else {
                m.name.trim().into()
            },
            me: m.me,
            known: true,
        },
    })
}

/// Card `c`'s board when other people are on it (two members or more).
pub fn shared_board<'a>(
    scope: &str,
    board: &str,
    boards: &'a [BoardSummary],
) -> Option<&'a BoardSummary> {
    if scope != "board" || board.is_empty() {
        return None;
    }
    boards
        .iter()
        .find(|b| b.board_id == board)
        .filter(|b| b.members.len() >= 2)
}

/// The member card `c` belongs to on a shared board, `None` off one.
pub fn card_owner(c: &CardSummary, boards: &[BoardSummary]) -> Option<Owner> {
    let b = shared_board(&c.scope, &c.board, boards)?;
    member_of(&c.owner, &b.members)
}

/// The "by member" filter's value for person id `p`.
pub fn owner_filter(p: &str) -> String {
    format!("owner:{p}")
}

/// Whether card `c` is owned by the person in filter value `f`.
pub fn owned_by(c: &CardSummary, f: &str) -> bool {
    !c.owner.is_empty() && f == owner_filter(&c.owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, name: &str, me: bool) -> SummaryMember {
        SummaryMember {
            person_id: id.into(),
            name: name.into(),
            me,
            ..SummaryMember::default()
        }
    }
    fn boards() -> Vec<BoardSummary> {
        vec![
            BoardSummary {
                board_id: "b-mlg".into(),
                members: vec![
                    m("p-stephen", "Stephen", true),
                    m("p-blake", "Blake", false),
                ],
                ..BoardSummary::default()
            },
            BoardSummary {
                board_id: "b-solo".into(),
                members: vec![m("p-stephen", "Stephen", true)],
                ..BoardSummary::default()
            },
        ]
    }
    fn card(owner: &str, board: &str, scope: &str) -> CardSummary {
        CardSummary {
            owner: owner.into(),
            board: board.into(),
            scope: scope.into(),
            ..CardSummary::default()
        }
    }

    #[test]
    fn owners() {
        let b = boards();
        assert_eq!(
            card_owner(&card("p-blake", "b-mlg", "board"), &b),
            Some(Owner {
                person_id: "p-blake".into(),
                name: "Blake".into(),
                me: false,
                known: true
            })
        );
        let mine = card_owner(&card("p-stephen", "b-mlg", "board"), &b).unwrap();
        assert!(mine.me && mine.name == "Stephen");
        assert_eq!(
            card_owner(&card("p-gone", "b-mlg", "board"), &b),
            Some(Owner {
                person_id: "p-gone".into(),
                name: FORMER_MEMBER.into(),
                me: false,
                known: false
            })
        );
        assert_eq!(card_owner(&card("", "b-mlg", "board"), &b), None);
        assert_eq!(card_owner(&card("p-stephen", "", "mine"), &b), None);
        assert_eq!(card_owner(&card("p-stephen", "b-solo", "board"), &b), None);
        assert_eq!(card_owner(&card("p-blake", "b-other", "board"), &b), None);
        let x = [m("p-stephen", "Stephen", true), m("p-x", " ", false)];
        assert_eq!(member_of("p-x", &x).unwrap().name, "Someone");
    }

    #[test]
    fn shared_and_owned_by() {
        let b = boards();
        assert_eq!(
            shared_board("board", "b-mlg", &b).map(|x| x.board_id.as_str()),
            Some("b-mlg")
        );
        assert!(shared_board("board", "b-solo", &b).is_none());
        assert!(owned_by(
            &card("p-blake", "b-mlg", "board"),
            "owner:p-blake"
        ));
        assert!(!owned_by(
            &card("p-blake", "b-mlg", "board"),
            "owner:p-stephen"
        ));
        assert!(!owned_by(&card("", "b-mlg", "board"), "owner:"));
    }
}
