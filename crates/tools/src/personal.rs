//! Personal's boards in the tools (card #708): board_create `board` takes
//! one of them by tag or id, board_edit `board` moves a private card (and
//! its subtasks) between them, and the board tool lists them. Personal is
//! the person's own space (card #558): its cards are private, so an agent
//! may place and move them there as freely as it edits them. A card in a
//! shared space is never moved by an agent: its number changes for
//! everyone in the space, so the person moves it in the window (#563).

use crate::format::{one_line, q, trim};
use crate::shared::BoardInfo;
use crate::{Inner, err_text};

/// What the person's Personal default board is called in a `board`
/// argument: `irori_core::PERSONAL_BOARD`, which coord stores as `""`.
pub const PERSONAL_DEFAULT: &str = "personal";

/// One of Personal's boards as the tools show it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersonalBoard {
    /// Its id; [`PERSONAL_DEFAULT`] for the default board.
    pub board_id: String,
    pub name: String,
    /// Its short handle in refs (`wheels#3`); empty for the default board.
    pub tag: String,
}

impl PersonalBoard {
    /// Whether this is Personal's default board.
    pub fn is_default(&self) -> bool {
        self.tag.is_empty() || self.board_id == PERSONAL_DEFAULT
    }

    /// The board as a private card's row names it: `""` for the default.
    pub fn coord_id(&self) -> &str {
        if self.is_default() {
            ""
        } else {
            &self.board_id
        }
    }

    /// What an agent passes as `board` for it.
    fn handle(&self) -> &str {
        if self.is_default() {
            PERSONAL_DEFAULT
        } else {
            &self.tag
        }
    }

    pub(crate) fn label(&self) -> String {
        if self.is_default() {
            return format!("{} (Personal's default board)", one_line(&self.name, 80));
        }
        format!("{} ({}) in Personal", one_line(&self.name, 80), self.tag)
    }
}

/// The board a `board` argument names: one of Personal's, or one in a
/// shared space.
#[derive(Debug, Clone)]
pub(crate) enum Target {
    Personal(PersonalBoard),
    Shared,
}

impl Inner {
    /// Personal's boards, its default first.
    pub(crate) fn personal_boards(&self) -> Vec<PersonalBoard> {
        self.b.personal_boards()
    }

    /// The board `v` names, by tag or id, among Personal's boards and the
    /// shared spaces' (card #708). Tags are unique across both (a new
    /// board's tag is refused when any board here has it).
    pub(crate) fn find_any_board(&self, v: &str) -> Result<Target, String> {
        let v = trim(v).trim_start_matches('#');
        let personal = self.personal_boards();
        let mine: Vec<&PersonalBoard> = personal
            .iter()
            .filter(|b| {
                b.board_id == v
                    || (!b.tag.is_empty() && b.tag.eq_ignore_ascii_case(v))
                    || (b.is_default() && v.eq_ignore_ascii_case(PERSONAL_DEFAULT))
            })
            .collect();
        let mut shared: Vec<BoardInfo> = self.shared_hits(v);
        // A board's id is unique, so it wins over another board's tag:
        // "personal" is the default's id even if some board took it as a
        // tag, and a boardId given after "more than one" always settles it.
        let mut mine = mine;
        let by_id = |b: &BoardInfo| {
            if b.space_board.is_empty() {
                b.board_id == v
            } else {
                b.space_board == v
            }
        };
        if mine.len() + shared.len() > 1 {
            if let Some(one) = mine.iter().find(|b| b.board_id == v).copied() {
                mine = vec![one];
                shared.clear();
            } else if let Some(one) = shared.iter().find(|b| by_id(b)).cloned() {
                mine.clear();
                shared = vec![one];
            }
        }
        match (mine.as_slice(), shared.as_slice()) {
            ([one], []) => Ok(Target::Personal((*one).clone())),
            ([], [_]) => Ok(Target::Shared),
            ([], []) => {
                let own: Vec<String> = personal.iter().map(|b| b.handle().to_string()).collect();
                let spaces: Vec<String> = self.shared_tags();
                Err(err_text(&format!(
                    "no board {} here. Personal's: {}. {}",
                    q(v),
                    if own.is_empty() {
                        PERSONAL_DEFAULT.to_string()
                    } else {
                        own.join(", ")
                    },
                    if spaces.is_empty() {
                        "You aren't in any shared space.".to_string()
                    } else {
                        format!("Shared spaces': {}.", spaces.join(", "))
                    }
                )))
            }
            _ => {
                let mut ids: Vec<String> =
                    mine.iter().map(|b| b.board_id.clone()).collect::<Vec<_>>();
                ids.extend(shared.iter().map(|b| {
                    if b.space_board.is_empty() {
                        b.board_id.clone()
                    } else {
                        b.space_board.clone()
                    }
                }));
                Err(err_text(&format!(
                    "more than one board has the tag {}: give its boardId ({}).",
                    q(v),
                    ids.join(", ")
                )))
            }
        }
    }

    /// The board tool's list of Personal's boards (card #708), when it has
    /// more than its default one.
    pub(crate) fn personal_section(&self) -> Option<String> {
        let all = self.personal_boards();
        if all.len() < 2 {
            return None;
        }
        let boards: Vec<String> = all
            .iter()
            .map(|b| format!("{} {}", b.handle(), q(&one_line(&b.name, 80))))
            .collect();
        Some(format!(
            "Personal's boards (private: board_create board=<tag> puts a card there, board_edit board=<tag> moves one with its subtasks): {}",
            boards.join(", ")
        ))
    }
}
