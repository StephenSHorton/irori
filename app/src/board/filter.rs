//! What the Board shows (the React app's card-grid.tsx and board-view.tsx
//! logic, pure): the filter chips, which cards a scope holds, the search,
//! the families the grid draws, Closed's outcomes and a member's presence.

use std::collections::HashSet;

use irori_window::boards::{BoardSummary, SummaryMember};
use irori_window::cards::CardSummary;

use crate::logic::cards::{over, state};
use crate::logic::owners::owned_by;
use crate::logic::search::card_matches;
use crate::logic::spaces::boards_of;

/// Home's scope: your own cards. Any other scope is a board (space) id.
pub const SCOPE_OWN: &str = "own";

/// A filter chip: All, Needs a decision, In review (card #191), My
/// devices, Closed (done, canceled and failed cards, which load on their
/// own; its id stays "done"), one device (card #22), one shared board
/// (v0.5 §8), Home's Private (card #199) or one member (card #268).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Filter {
    #[default]
    All,
    Decide,
    Review,
    Mine,
    Done,
    Private,
    Device(String),
    Board(String),
    Owner(String),
}

/// The tabs, in order, with their labels.
pub const FILTERS: [(Filter, &str); 5] = [
    (Filter::All, "All"),
    (Filter::Decide, "Needs a decision"),
    (Filter::Review, "In review"),
    (Filter::Mine, "My devices"),
    (Filter::Done, "Closed"),
];

impl Filter {
    /// Its id as the React app wrote it ("all", "device:<id>", ...).
    pub fn id(&self) -> String {
        match self {
            Filter::All => "all".into(),
            Filter::Decide => "decide".into(),
            Filter::Review => "review".into(),
            Filter::Mine => "mine".into(),
            Filter::Done => "done".into(),
            Filter::Private => "private".into(),
            Filter::Device(d) => format!("device:{d}"),
            Filter::Board(b) => format!("board:{b}"),
            Filter::Owner(p) => format!("owner:{p}"),
        }
    }
}

/// A filter id from outside (prefs, a link), else All (asFilter).
pub fn as_filter(id: Option<&str>) -> Filter {
    let Some(id) = id else { return Filter::All };
    if let Some(b) = id.strip_prefix("board:") {
        return Filter::Board(b.into());
    }
    if let Some(p) = id.strip_prefix("owner:") {
        return Filter::Owner(p.into());
    }
    match id {
        "private" => Filter::Private,
        "closed" | "done" => Filter::Done,
        "decide" => Filter::Decide,
        "review" => Filter::Review,
        "mine" => Filter::Mine,
        _ => Filter::All,
    }
}

/// Whether card `c` is under filter `f`.
pub fn matches(c: &CardSummary, f: &Filter) -> bool {
    if *f == Filter::Done {
        return over(&c.state);
    }
    if over(&c.state) {
        return false;
    }
    match f {
        Filter::Decide => c.open_blockers > 0,
        Filter::Review => c.state == state::IN_REVIEW,
        Filter::Mine => c.scope == "mine",
        Filter::Private => c.scope != "board",
        Filter::Device(d) => c.devices.iter().any(|x| x == d),
        Filter::Board(b) => c.board == *b,
        Filter::Owner(_) => owned_by(c, &f.id()),
        _ => true,
    }
}

/// The grid under filter `f` and the search box's `query` (card #171). A
/// subtask that matches brings its parent's tile along, before it, so it
/// is seen in context; `context` holds the ids shown only for that.
pub fn shown_cards<'a>(cards: &[&'a CardSummary], f: &Filter, query: &str) -> (Vec<&'a CardSummary>, HashSet<String>) {
    let keep: HashSet<&str> = cards
        .iter()
        .filter(|c| matches(c, f) && card_matches(c, query))
        .map(|c| c.id.as_str())
        .collect();
    let mut context = HashSet::new();
    for c in cards {
        if let Some(p) = &c.parent
            && keep.contains(c.id.as_str())
            && !keep.contains(p.id.as_str())
        {
            context.insert(p.id.clone());
        }
    }
    let shown = cards
        .iter()
        .copied()
        .filter(|c| keep.contains(c.id.as_str()) || context.contains(&c.id))
        .collect();
    (shown, context)
}

/// The Board's order (BoardService.Cards, crates/window cards_in): open
/// asks first, then cards waiting on a decision, then the newest change;
/// pinned cards lead in their pin order, each with its subtasks after it.
/// `s.data.cards` comes newest first, so the Board puts it back.
pub fn board_order<'a>(cards: &[&'a CardSummary]) -> Vec<&'a CardSummary> {
    let open_ask = |c: &CardSummary| c.ask.as_ref().is_some_and(|a| a.state != crate::logic::cards::ask::CLOSED);
    let mut out: Vec<&CardSummary> = cards.to_vec();
    out.sort_by(|a, b| {
        open_ask(b)
            .cmp(&open_ask(a))
            .then((b.open_blockers > 0).cmp(&(a.open_blockers > 0)))
            .then(b.updated_at.unix_nano().cmp(&a.updated_at.unix_nano()))
            .then(b.id.cmp(&a.id))
    });
    let mut pins: Vec<&CardSummary> = out.iter().copied().filter(|c| c.pinned).collect();
    pins.sort_by(|a, b| a.pin_rank.partial_cmp(&b.pin_rank).unwrap_or(std::cmp::Ordering::Equal));
    let mut lead: Vec<&CardSummary> = Vec::new();
    let mut used: HashSet<&str> = HashSet::new();
    for p in pins {
        if !used.insert(p.id.as_str()) {
            continue;
        }
        lead.push(p);
        for k in out.iter().copied().filter(|k| !k.pinned && k.parent.as_ref().is_some_and(|x| x.id == p.id)) {
            if used.insert(k.id.as_str()) {
                lead.push(k);
            }
        }
    }
    lead.extend(out.into_iter().filter(|c| !used.contains(c.id.as_str())));
    lead
}

/// Whether card `c` belongs on `scope`'s page. Home (card #199) is your
/// private cards and every shared board's card one of your agents holds
/// or watches.
pub fn in_scope(c: &CardSummary, scope: &str) -> bool {
    if scope == SCOPE_OWN {
        c.scope != "board" || c.ours
    } else {
        c.board == scope
    }
}

/// A space in Home's board row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowBoard {
    pub board_id: String,
    pub name: String,
    pub tag: String,
}

/// Home's board row (card #199): each space with a card on Home, in the
/// order they first appear, named and tagged as the space is (card #558).
pub fn home_boards(cards: &[&CardSummary], spaces: &[BoardSummary]) -> Vec<RowBoard> {
    let mut out: Vec<RowBoard> = Vec::new();
    for c in cards {
        if c.scope != "board" || c.board.is_empty() || out.iter().any(|b| b.board_id == c.board) {
            continue;
        }
        let s = spaces.iter().find(|x| x.board_id == c.board);
        out.push(RowBoard {
            board_id: c.board.clone(),
            name: s.map(|s| s.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| c.scope_label.clone()),
            tag: s.map(|s| s.tag.clone()).unwrap_or_default(),
        });
    }
    out
}

/// The filter as the page can show it: a device chip whose device has no
/// cards left, or Mine, fall back to All. Private and a board chip are
/// Home's (`boards`: its row), kept while that board has cards there. A
/// member chip holds only on a board that member is on.
pub fn active_filter(f: &Filter, chips: &[String], boards: Option<&[RowBoard]>, members: Option<&[SummaryMember]>) -> Filter {
    match f {
        Filter::Owner(p) => {
            let ms = members.unwrap_or(&[]);
            if ms.len() > 1 && ms.iter().any(|m| m.person_id == *p) {
                f.clone()
            } else {
                Filter::All
            }
        }
        Filter::Device(d) if !chips.iter().any(|c| c == d) => Filter::All,
        Filter::Private if boards.is_none() => Filter::All,
        Filter::Board(b) => {
            if boards.is_some_and(|bs| bs.iter().any(|x| x.board_id == *b)) {
                f.clone()
            } else {
                Filter::All
            }
        }
        Filter::Mine => Filter::All,
        _ => f.clone(),
    }
}

/// What a filter or search hides (card #237): the open cards All with no
/// search would show and this view doesn't, and how many of them wait on
/// a decision. None when nothing is hidden, on All with no search, or on
/// Closed.
pub fn hidden_by_filter(cards: &[&CardSummary], f: &Filter, query: &str) -> Option<(usize, usize)> {
    if *f == Filter::Done || (*f == Filter::All && query.trim().is_empty()) {
        return None;
    }
    let (shown, _) = shown_cards(cards, f, query);
    let shown: HashSet<&str> = shown.iter().map(|c| c.id.as_str()).collect();
    let hidden: Vec<_> = cards
        .iter()
        .filter(|c| matches(c, &Filter::All) && !shown.contains(c.id.as_str()))
        .collect();
    if hidden.is_empty() {
        return None;
    }
    Some((hidden.len(), hidden.iter().filter(|c| c.open_blockers > 0).count()))
}

/// Home remembers its last board pick (card #199) under this pref.
pub const HOME_FILTER_PREF: &str = "board.homeFilter";

/// Whether `f` is a pick Home keeps: All, Private or one board.
pub fn remembered_filter(f: &Filter) -> bool {
    matches!(f, Filter::All | Filter::Private | Filter::Board(_))
}

/// The pick Home starts on from what was saved.
pub fn saved_home_filter(saved: Option<&str>) -> Filter {
    let f = as_filter(saved);
    if remembered_filter(&f) { f } else { Filter::All }
}

/// The grid's families: each subtask whose parent is in the list joins
/// its parent, in list order, at the parent's place; any other card is a
/// family of its own.
pub fn family_groups<'a>(cards: &[&'a CardSummary]) -> Vec<(&'a CardSummary, Vec<&'a CardSummary>)> {
    let ids: HashSet<&str> = cards.iter().map(|c| c.id.as_str()).collect();
    let parent_in = |c: &CardSummary| c.parent.as_ref().filter(|p| p.id != c.id && ids.contains(p.id.as_str())).map(|p| p.id.clone());
    cards
        .iter()
        .filter(|c| parent_in(c).is_none())
        .map(|h| {
            let kids = cards.iter().copied().filter(|c| parent_in(c).as_deref() == Some(h.id.as_str())).collect();
            (*h, kids)
        })
        .collect()
}

/// Closed's outcome filter (card #57).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    #[default]
    Any,
    Completed,
    Canceled,
    Failed,
}

pub const OUTCOMES: [(Outcome, &str); 4] = [
    (Outcome::Any, "Any outcome"),
    (Outcome::Completed, "Done"),
    (Outcome::Canceled, "Canceled"),
    (Outcome::Failed, "Failed"),
];

impl Outcome {
    pub fn state(self) -> &'static str {
        match self {
            Outcome::Any => "any",
            Outcome::Completed => state::COMPLETED,
            Outcome::Canceled => state::CANCELED,
            Outcome::Failed => state::FAILED,
        }
    }
}

/// The closed cards that ended as `o`.
pub fn of_outcome<'a>(cs: &[&'a CardSummary], o: Outcome) -> Vec<&'a CardSummary> {
    cs.iter().copied().filter(|c| o == Outcome::Any || c.state == o.state()).collect()
}

/// A space's cards on one of its boards: a card with no inner board (or
/// the space's own id) is on the first board (space-boards.tsx
/// cardsOnBoard).
pub fn cards_on_board<'a>(b: &BoardSummary, cards: &'a [CardSummary], board_id: &str) -> Vec<&'a CardSummary> {
    let first = boards_of(b).first().map(|x| x.board_id.clone()).unwrap_or_else(|| b.board_id.clone());
    cards
        .iter()
        .filter(|c| {
            let on = if c.space_board.is_empty() || c.space_board == b.board_id {
                first.as_str()
            } else {
                c.space_board.as_str()
            };
            c.board == b.board_id && on == board_id
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::cards::CardRef;

    fn cs(id: &str, f: impl FnOnce(&mut CardSummary)) -> CardSummary {
        let mut c = CardSummary {
            id: id.into(),
            short: id.into(),
            state: state::WORKING.into(),
            scope: "mine".into(),
            ..CardSummary::default()
        };
        f(&mut c);
        c
    }
    fn card(st: &str, ob: i64) -> CardSummary {
        cs("c", |c| {
            c.state = st.into();
            c.open_blockers = ob;
        })
    }
    fn on_board(id: &str, board: &str, label: &str, tag: &str, ours: bool) -> CardSummary {
        cs(id, |c| {
            c.scope = "board".into();
            c.board = board.into();
            c.scope_label = label.into();
            c.tag = tag.into();
            c.ours = ours;
        })
    }
    fn space(id: &str, name: &str, tag: &str) -> BoardSummary {
        BoardSummary {
            board_id: id.into(),
            name: name.into(),
            tag: tag.into(),
            ..BoardSummary::default()
        }
    }
    fn rb(id: &str, name: &str, tag: &str) -> RowBoard {
        RowBoard {
            board_id: id.into(),
            name: name.into(),
            tag: tag.into(),
        }
    }
    fn ids(v: &[&CardSummary]) -> Vec<String> {
        v.iter().map(|c| c.id.clone()).collect()
    }

    // board-view.test.ts

    #[test]
    fn home_is_private_cards_and_board_cards_your_agents_are_on() {
        let p = cs("p", |_| {});
        let ours = on_board("o", "b-wind", "Wind Waker Multiplayer", "wind", true);
        let theirs = on_board("t", "b-wind", "Wind Waker Multiplayer", "wind", false);
        let horton = on_board("h", "b-horton", "horton", "horton", true);
        assert!(in_scope(&p, SCOPE_OWN));
        assert!(in_scope(&ours, SCOPE_OWN));
        assert!(!in_scope(&theirs, SCOPE_OWN));
        assert!(in_scope(&ours, "b-wind"));
        assert!(in_scope(&theirs, "b-wind"));
        assert!(!in_scope(&horton, "b-wind"));
        assert!(!in_scope(&p, "b-wind"));
        let spaces = [space("b-wind", "Wind Waker Multiplayer", "wind"), space("b-horton", "horton", "horton")];
        assert_eq!(
            home_boards(&[&p, &ours, &horton, &theirs], &spaces),
            vec![rb("b-wind", "Wind Waker Multiplayer", "wind"), rb("b-horton", "horton", "horton")]
        );
        assert_eq!(home_boards(&[&p], &[]), vec![]);
    }

    #[test]
    fn the_board_row_tags_a_space_as_the_space() {
        let ours = on_board("o", "b-wind", "Wind Waker Multiplayer", "wind", true);
        let net = on_board("n", "b-wind", "Wind · Netcode", "net", true);
        let spaces = [space("b-wind", "Wind Waker Multiplayer", "wind")];
        assert_eq!(home_boards(&[&net, &ours], &spaces), vec![rb("b-wind", "Wind Waker Multiplayer", "wind")]);
        assert_eq!(home_boards(&[&net], &[]), vec![rb("b-wind", "Wind · Netcode", "")]);
    }

    #[test]
    fn homes_board_filter() {
        let p = cs("p", |_| {});
        let ours = on_board("o", "b-wind", "Wind", "wind", true);
        let horton = on_board("h", "b-horton", "horton", "horton", true);
        let row = home_boards(&[&p, &ours, &horton], &[]);
        assert!(matches(&p, &Filter::Private));
        assert!(!matches(&ours, &Filter::Private));
        assert_eq!(as_filter(Some("private")), Filter::Private);
        assert!(matches(&ours, &Filter::Board("b-wind".into())));
        assert!(!matches(&horton, &Filter::Board("b-wind".into())));
        assert_eq!(active_filter(&Filter::Private, &[], Some(&row), None), Filter::Private);
        let bh = Filter::Board("b-horton".into());
        assert_eq!(active_filter(&bh, &[], Some(&row), None), bh);
        assert_eq!(active_filter(&Filter::Board("b-gone".into()), &[], Some(&row), None), Filter::All);
        assert_eq!(active_filter(&Filter::Private, &[], None, None), Filter::All);
        assert_eq!(active_filter(&Filter::Board("b-wind".into()), &[], None, None), Filter::All);
        assert_eq!(active_filter(&Filter::Mine, &[], Some(&row), None), Filter::All);
    }

    #[test]
    fn the_pick_home_remembers() {
        assert!(remembered_filter(&Filter::All));
        assert!(remembered_filter(&Filter::Private));
        assert!(remembered_filter(&Filter::Board("b-wind".into())));
        assert!(!remembered_filter(&Filter::Decide));
        assert!(!remembered_filter(&Filter::Done));
        assert_eq!(saved_home_filter(None), Filter::All);
        assert_eq!(saved_home_filter(Some("board:b-wind")), Filter::Board("b-wind".into()));
        assert_eq!(saved_home_filter(Some("private")), Filter::Private);
        assert_eq!(saved_home_filter(Some("done")), Filter::All);
        assert_eq!(saved_home_filter(Some("nonsense")), Filter::All);
    }

    #[test]
    fn hidden_by_filter_counts_what_a_filter_or_search_hides() {
        let waiting = cs("w", |c| c.open_blockers = 1);
        let review = cs("r", |c| c.state = state::IN_REVIEW.into());
        let plain = cs("x", |c| c.title = "plain card".into());
        let cards = [&waiting, &review, &plain];
        assert_eq!(hidden_by_filter(&cards, &Filter::All, ""), None);
        assert_eq!(hidden_by_filter(&cards, &Filter::Done, ""), None);
        assert_eq!(hidden_by_filter(&cards, &Filter::Review, ""), Some((2, 1)));
        assert_eq!(hidden_by_filter(&cards, &Filter::All, "plain"), Some((2, 1)));
        assert_eq!(hidden_by_filter(&[&waiting], &Filter::Decide, ""), None);
    }

    #[test]
    fn member_chips_hold_on_a_board_that_member_is_on() {
        let m = |id: &str, name: &str, me: bool| SummaryMember {
            person_id: id.into(),
            name: name.into(),
            me,
            ..SummaryMember::default()
        };
        let members = [m("p-me", "Stephen", true), m("p-blake", "Blake", false)];
        let blake = Filter::Owner("p-blake".into());
        assert_eq!(active_filter(&blake, &[], None, Some(&members)), blake);
        assert_eq!(active_filter(&Filter::Owner("p-gone".into()), &[], None, Some(&members)), Filter::All);
        assert_eq!(active_filter(&blake, &[], None, Some(&members[..1])), Filter::All);
        assert_eq!(active_filter(&blake, &[], None, None), Filter::All);
    }

    // card-grid.test.ts

    #[test]
    fn closed_is_the_last_tab_and_narrows_by_outcome() {
        let tabs: Vec<String> = FILTERS.iter().map(|(f, _)| f.id()).collect();
        assert_eq!(tabs, ["all", "decide", "review", "mine", "done"]);
        assert_eq!(FILTERS[4].1, "Closed");
        let labels: Vec<&str> = OUTCOMES.iter().map(|(_, l)| *l).collect();
        assert_eq!(labels, ["Any outcome", "Done", "Canceled", "Failed"]);
        let all: Vec<CardSummary> = [state::COMPLETED, state::CANCELED, state::FAILED]
            .iter()
            .map(|s| cs(s, |c| c.state = (*s).into()))
            .collect();
        let refs: Vec<&CardSummary> = all.iter().collect();
        assert_eq!(of_outcome(&refs, Outcome::Any).len(), 3);
        assert_eq!(ids(&of_outcome(&refs, Outcome::Completed)), [state::COMPLETED]);
        assert_eq!(ids(&of_outcome(&refs, Outcome::Canceled)), [state::CANCELED]);
        assert_eq!(ids(&of_outcome(&refs, Outcome::Failed)), [state::FAILED]);
    }

    #[test]
    fn closed_holds_over_cards_and_the_other_tabs_open_ones() {
        assert!(matches(&card(state::COMPLETED, 0), &Filter::Done));
        assert!(matches(&card(state::CANCELED, 0), &Filter::Done));
        assert!(matches(&card(state::FAILED, 0), &Filter::Done));
        assert!(!matches(&card(state::WORKING, 0), &Filter::Done));
        assert!(!matches(&card(state::BLOCKED, 1), &Filter::Done));
        assert!(matches(&card(state::WORKING, 0), &Filter::All));
        assert!(!matches(&card(state::COMPLETED, 0), &Filter::All));
        assert!(matches(&card(state::BLOCKED, 1), &Filter::Decide));
        assert!(matches(&card(state::WORKING, 0), &Filter::Mine));
        assert!(!matches(&card(state::CANCELED, 0), &Filter::Mine));
        assert!(matches(&card(state::IN_REVIEW, 0), &Filter::Review));
        assert!(!matches(&card(state::WORKING, 0), &Filter::Review));
        assert!(matches(&card(state::IN_REVIEW, 0), &Filter::All));
        assert_eq!(as_filter(Some("review")), Filter::Review);
    }

    #[test]
    fn as_filter_reads_a_known_id_else_all() {
        assert_eq!(as_filter(Some("done")), Filter::Done);
        assert_eq!(as_filter(Some("closed")), Filter::Done);
        assert_eq!(as_filter(Some("nope")), Filter::All);
        assert_eq!(as_filter(None), Filter::All);
        assert_eq!(as_filter(Some("board:b1")), Filter::Board("b1".into()));
        assert_eq!(as_filter(Some("owner:p-blake")), Filter::Owner("p-blake".into()));
        for f in [Filter::Device("d".into()), Filter::Board("b".into()), Filter::Owner("p".into()), Filter::Private] {
            assert_eq!(as_filter(Some(&f.id())), if matches!(f, Filter::Device(_)) { Filter::All } else { f.clone() });
        }
    }

    #[test]
    fn a_board_chip_holds_that_boards_cards() {
        let shared = on_board("s", "b1", "", "mlg", false);
        assert!(matches(&shared, &Filter::All));
        assert!(matches(&shared, &Filter::Board("b1".into())));
        assert!(!matches(&shared, &Filter::Board("b2".into())));
        assert!(!matches(&shared, &Filter::Mine));
        assert!(!matches(&card(state::WORKING, 0), &Filter::Board("b1".into())));
    }

    fn sub(id: &str, parent: &str, ob: i64) -> CardSummary {
        cs(id, |c| {
            c.parent = Some(CardRef {
                id: parent.into(),
                short: parent.into(),
                title: "p".into(),
                ..CardRef::default()
            });
            c.open_blockers = ob;
        })
    }

    #[test]
    fn shown_cards_brings_a_matching_subtasks_parent_along() {
        let parent = cs("7", |_| {});
        let blocked = sub("9", "7", 1);
        let quiet = sub("8", "7", 0);
        let other = cs("3", |_| {});
        let all = [&parent, &quiet, &blocked, &other];
        let (shown, context) = shown_cards(&all, &Filter::All, "");
        assert_eq!(ids(&shown), ["7", "8", "9", "3"]);
        assert!(context.is_empty());
        let (shown, context) = shown_cards(&all, &Filter::Decide, "");
        assert_eq!(ids(&shown), ["7", "9"]);
        assert_eq!(context.into_iter().collect::<Vec<_>>(), ["7"]);
    }

    #[test]
    fn family_groups_put_subtasks_under_their_parent() {
        let shape = |cs: &[&CardSummary]| -> Vec<Vec<String>> {
            family_groups(cs)
                .into_iter()
                .map(|(h, k)| std::iter::once(h.id.clone()).chain(k.iter().map(|c| c.id.clone())).collect())
                .collect()
        };
        let (a, p, k1, b, k2) = (cs("a", |_| {}), cs("p", |_| {}), sub("k1", "p", 0), cs("b", |_| {}), sub("k2", "p", 0));
        assert_eq!(shape(&[&a, &p, &k1, &b, &k2]), [vec!["a"], vec!["p", "k1", "k2"], vec!["b"]]);
        let k = sub("k", "gone", 0);
        assert_eq!(shape(&[&k, &a]), [vec!["k"], vec!["a"]]);
        let k = sub("k", "p", 0);
        assert_eq!(shape(&[&k, &p]), [vec!["p", "k"]]);
    }

    #[test]
    fn the_member_filter_keeps_that_members_open_cards() {
        let on = |owner: &str| {
            cs(owner, |c| {
                c.scope = "board".into();
                c.board = "b".into();
                c.owner = owner.into();
            })
        };
        let blake = Filter::Owner("p-blake".into());
        assert!(matches(&on("p-blake"), &blake));
        assert!(!matches(&on("p-me"), &blake));
        assert!(!matches(&on(""), &blake));
    }

    #[test]
    fn board_order_puts_asks_then_decisions_then_newest_with_pins_first() {
        let t = |ms: i64| gocompat::time::GoTime::unix_milli(ms);
        let old = cs("old", |c| c.updated_at = t(1_000));
        let new = cs("new", |c| c.updated_at = t(5_000));
        let dec = cs("dec", |c| {
            c.updated_at = t(500);
            c.open_blockers = 1;
        });
        let p2 = cs("p2", |c| {
            c.pinned = true;
            c.pin_rank = 2.0;
        });
        let p1 = cs("p1", |c| {
            c.pinned = true;
            c.pin_rank = 1.0;
        });
        let kid = sub("kid", "p2", 0);
        let got = board_order(&[&old, &kid, &new, &p2, &dec, &p1]);
        assert_eq!(ids(&got), ["p1", "p2", "kid", "dec", "new", "old"]);
    }

    // boards.test.ts presenceLine (the spaces package's copy)

    #[test]
    fn presence_line_says_how_a_member_stands() {
        use crate::spaces::logic::presence_line;
        let now = 1_800_000_000_000;
        let m = |presence: &str, seen_ago: Option<i64>, working: i64, free: i64| SummaryMember {
            person_id: "p".into(),
            name: "Blake".into(),
            presence: presence.into(),
            last_seen: seen_ago.map(|a| gocompat::time::GoTime::unix_milli(now - a)).unwrap_or_default(),
            working,
            free,
            ..SummaryMember::default()
        };
        assert_eq!(presence_line(&m("online", None, 2, 1), now), "online · 2 working");
        assert_eq!(presence_line(&m("online", None, 0, 1), now), "online");
        assert_eq!(presence_line(&m("online", None, 0, 0), now), "online · no agents");
        assert_eq!(presence_line(&m("recent", Some(5 * 60_000), 1, 0), now), "seen 5m ago · 1 working");
        assert_eq!(presence_line(&m("offline", Some(2 * 3_600_000), 3, 1), now), "offline · seen 2h ago");
        assert_eq!(presence_line(&m("offline", None, 0, 0), now), "offline");
        assert_eq!(presence_line(&m("", None, 0, 0), now), "offline");
    }
}
