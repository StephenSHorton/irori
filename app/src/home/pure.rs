//! Home's pure pieces: the instant-answer keys and outcome words (the
//! React app's lib/optimistic.ts, card #644), the wind-down line
//! (lib/wind-down.ts, card #484), "Agents answer now"'s default
//! (lib/agents-now.ts, card #391), and the needs-you rows' small rules
//! (needs-you.tsx waitsOnYou, trustFromReason; card-detail.tsx
//! byRecommendation; App.tsx BoardsNeedYou).

use irori_window::agents_trust::TrustNeed;
use irori_window::cards::{CardBlocker, CardOption, CardSummary};
use irori_window::wind_down::WindDownStatus;

use crate::logic::cards::blocker;
use crate::logic::spawn_line::NOT_TRUSTED;
use crate::logic::thread::answerable;

// ---- instant actions (optimistic.ts) -----------------------------------

/// The keys actions go by: one per thing acted on, the same wherever it
/// is answered, so a second click on it anywhere waits for the first.
pub fn answer_key(card_id: &str, blocker_id: &str) -> String {
    format!("answer:{card_id}/{blocker_id}")
}
#[allow(dead_code)] // The card page (P3) and Notifications (P9) answer by these too.
pub fn close_key(card_id: &str) -> String {
    format!("close:{card_id}")
}
#[allow(dead_code)]
pub fn ask_key(ask_id: &str) -> String {
    format!("ask:{ask_id}")
}

/// The coord's Done answer: the done check's option that closes the card.
pub const DONE_ANSWER: &str = "Done";

/// `option` is Done on a done check (the pick the host closes the card
/// on); any other option goes back to the agent.
pub fn is_done(b: &CardBlocker, option: i64) -> bool {
    b.done_check
        && option >= 0
        && b.options.get(option as usize).is_some_and(|o| o.answer == DONE_ANSWER)
}

/// What an answer on a row says at once, while it saves.
pub fn answer_said(b: &CardBlocker, option: i64) -> &'static str {
    if !b.done_check {
        "Answer sent."
    } else if is_done(b, option) {
        "Marked done."
    } else {
        "Sent back to the agent."
    }
}

/// The toast's title when an answer is refused.
pub fn answer_failed(b: &CardBlocker) -> &'static str {
    if b.done_check {
        "Your answer to the done check wasn't saved"
    } else {
        "Your answer wasn't saved"
    }
}

/// The open item counts toward the card's open count (cards.rs
/// card_summary): open or replied back to the person, not another
/// member's question nor a wait. An answered one already isn't.
#[allow(dead_code)]
pub fn counted(b: &CardBlocker) -> bool {
    b.resolved.is_none()
        && b.decider.is_none()
        && b.waits_on.is_none()
        && (b.state.is_empty() || b.state == blocker::OPEN || b.state == blocker::REPLIED)
}

// ---- needs-you rows ---------------------------------------------------------

/// The blockers of a card the person answers now: open, or replied back
/// to them, and not another member's question nor a wait on other work.
pub fn waits_on_you(b: &CardBlocker) -> bool {
    let st = if b.state.is_empty() { blocker::OPEN } else { b.state.as_str() };
    b.decider.is_none() && b.waits_on.is_none() && answerable(st) && b.state != blocker::ANSWERED
}

/// Options, recommended first, with their index.
pub fn by_recommendation(options: &[CardOption]) -> Vec<(&CardOption, usize)> {
    let all = options.iter().enumerate().map(|(i, o)| (o, i));
    all.clone().filter(|(o, _)| o.recommended).chain(all.filter(|(o, _)| !o.recommended)).collect()
}

/// Reads `not trusted: <folder> on <device>` (a failed spawn's reason)
/// when the host can't be asked.
pub fn trust_from_reason(reason: &str) -> TrustNeed {
    let rest = reason.strip_prefix(NOT_TRUSTED).unwrap_or(reason);
    let folder = match rest.rfind(" on ") {
        Some(at) if at > 0 => &rest[..at],
        _ => rest,
    };
    TrustNeed {
        folder: folder.to_string(),
        device: "this computer".into(),
        here: true,
        what: format!(
            "Claude Code will run agents in {folder} on this computer without asking you first. It's the trust Claude Code asks for the first time you open a folder."
        ),
        ..TrustNeed::default()
    }
}

/// A board card waiting on the person (Home's BoardsNeedYou, cards #237,
/// #509, #520): a decision, a PR merge or a wait no agent will end.
pub fn board_decides(c: &CardSummary) -> bool {
    c.scope == "board" && (c.open_blockers > 0 || c.merge_pr.is_some() || c.gone_wait.is_some())
}

/// Each space with board decisions Home doesn't list (not ours), in the
/// order they first appear, with how many.
pub fn boards_need_you(cards: &[CardSummary]) -> Vec<(String, usize)> {
    let mut by: Vec<(String, usize)> = Vec::new();
    for c in cards.iter().filter(|c| board_decides(c) && !c.ours && !c.board.is_empty()) {
        match by.iter_mut().find(|(b, _)| *b == c.board) {
            Some((_, n)) => *n += 1,
            None => by.push((c.board.clone(), 1)),
        }
    }
    by
}

// ---- Agents answer now (agents-now.ts) ----------------------------------

/// A picked mention is a person, not an agent.
pub fn names_person(key: &str) -> bool {
    key.starts_with("person:") || key.starts_with("user:")
}

/// The toggle's starting point: on when the text names nobody or an
/// agent, off when it names only people.
pub fn agents_now_default<'a>(keys: impl IntoIterator<Item = &'a str>) -> bool {
    let keys: Vec<&str> = keys.into_iter().map(str::trim).filter(|k| !k.is_empty() && *k != "irori").collect();
    keys.is_empty() || keys.iter().any(|k| !names_person(k))
}

// ---- Wind down (wind-down.ts) -------------------------------------------

fn agents(n: usize) -> String {
    format!("{n} agent{}", if n == 1 { "" } else { "s" })
}

/// Home's line for a run: how many agents it reached and how many
/// answered "wound down". Empty when there is no run.
pub fn wind_down_line(st: &WindDownStatus) -> String {
    if st.id.is_empty() || st.rows.is_empty() {
        return String::new();
    }
    let n = st.rows.len();
    let done = st.rows.iter().filter(|r| r.state == "done").count();
    // Every send failed: the run never starts, so it never finishes.
    if st.rows.iter().all(|r| r.delivery_id.is_empty()) {
        return format!("Wind down reached none of {}", agents(n));
    }
    if !st.finished {
        return format!("Winding down {}, {done} confirmed so far", agents(n));
    }
    let tail = if st.paused { " · Pause all is on" } else { "" };
    format!("Wound down {}, {done} confirmed{tail}", agents(n))
}

/// A run row's state in words.
pub fn wind_down_state(state: &str) -> &str {
    match state {
        "waiting" => "Not reached yet",
        "winding" => "Winding down",
        "done" => "Wound down",
        "no_answer" => "No answer",
        s => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::cards::GoneWait;
    use irori_window::wind_down::WindDownRow;

    fn row(state: &str) -> WindDownRow {
        WindDownRow {
            key: "k".into(),
            name: "n".into(),
            delivery_id: "d".into(),
            state: state.into(),
            ..WindDownRow::default()
        }
    }

    fn run(states: &[&str], f: impl FnOnce(&mut WindDownStatus)) -> WindDownStatus {
        let mut st = WindDownStatus {
            id: "wd_1".into(),
            scope: "this".into(),
            pause_after: true,
            paused: false,
            finished: true,
            rows: states.iter().map(|s| row(s)).collect(),
        };
        f(&mut st);
        st
    }

    #[test]
    fn one_line_for_the_whole_run() {
        let mut states = vec!["done"; 23];
        states.extend(vec!["no_answer"; 20]);
        assert_eq!(wind_down_line(&run(&states, |_| {})), "Wound down 43 agents, 23 confirmed");
        assert_eq!(
            wind_down_line(&run(&states, |s| s.paused = true)),
            "Wound down 43 agents, 23 confirmed · Pause all is on"
        );
    }

    #[test]
    fn while_it_runs_it_says_so() {
        assert_eq!(
            wind_down_line(&run(&["done", "winding", "waiting"], |s| s.finished = false)),
            "Winding down 3 agents, 1 confirmed so far"
        );
        assert_eq!(wind_down_line(&run(&["done"], |_| {})), "Wound down 1 agent, 1 confirmed");
    }

    #[test]
    fn a_run_none_of_whose_sends_went_out() {
        let failed = WindDownRow {
            delivery_id: String::new(),
            error: "no route".into(),
            ..row("no_answer")
        };
        let st = run(&[], |s| {
            s.finished = false;
            s.rows = vec![failed.clone(), failed];
        });
        assert_eq!(wind_down_line(&st), "Wind down reached none of 2 agents");
    }

    #[test]
    fn no_run_no_line() {
        assert_eq!(wind_down_line(&run(&[], |s| s.id.clear())), "");
        assert_eq!(wind_down_line(&run(&[], |_| {})), "");
    }

    fn opt(answer: &str, recommended: bool) -> CardOption {
        CardOption {
            answer: answer.into(),
            recommended,
            ..CardOption::default()
        }
    }

    #[test]
    fn done_is_the_done_checks_done_option() {
        let check = CardBlocker {
            id: "b1".into(),
            done_check: true,
            options: vec![opt("Done", true), opt("Not yet", false)],
            ..CardBlocker::default()
        };
        assert!(is_done(&check, 0));
        assert!(!is_done(&check, 1));
        assert!(!is_done(&check, -1));
        assert!(!is_done(&check, 7));
        assert_eq!(answer_said(&check, 0), "Marked done.");
        assert_eq!(answer_said(&check, -1), "Sent back to the agent.");
        assert_eq!(answer_failed(&check), "Your answer to the done check wasn't saved");
        let q = CardBlocker {
            done_check: false,
            ..check.clone()
        };
        assert!(!is_done(&q, 0));
        assert_eq!(answer_said(&q, 0), "Answer sent.");
        assert_eq!(answer_failed(&q), "Your answer wasn't saved");
    }

    #[test]
    fn keys_are_one_per_thing() {
        assert_eq!(answer_key("c1", "b2"), "answer:c1/b2");
        assert_eq!(close_key("c1"), "close:c1");
        assert_eq!(ask_key("a1"), "ask:a1");
    }

    #[test]
    fn counted_items() {
        let b = |f: fn(&mut CardBlocker)| {
            let mut b = CardBlocker::default();
            f(&mut b);
            b
        };
        assert!(counted(&b(|_| {})));
        assert!(counted(&b(|b| b.state = "replied".into())));
        assert!(!counted(&b(|b| b.state = "answered".into())));
        assert!(!counted(&b(|b| b.decider = Some(Default::default()))));
        assert!(!counted(&b(|b| b.waits_on = Some(Default::default()))));
    }

    #[test]
    fn what_waits_on_you() {
        let b = |f: fn(&mut CardBlocker)| {
            let mut b = CardBlocker::default();
            f(&mut b);
            b
        };
        assert!(waits_on_you(&b(|_| {})));
        assert!(waits_on_you(&b(|b| b.state = "open".into())));
        assert!(waits_on_you(&b(|b| b.state = "replied".into())));
        assert!(!waits_on_you(&b(|b| b.state = "answered".into())));
        assert!(!waits_on_you(&b(|b| b.state = "resolved".into())));
        assert!(!waits_on_you(&b(|b| b.state = "closed".into())));
        assert!(!waits_on_you(&b(|b| b.decider = Some(Default::default()))));
        assert!(!waits_on_you(&b(|b| b.waits_on = Some(Default::default()))));
    }

    #[test]
    fn recommended_first() {
        let os = vec![opt("a", false), opt("b", true), opt("c", false), opt("d", true)];
        let order: Vec<usize> = by_recommendation(&os).into_iter().map(|(_, i)| i).collect();
        assert_eq!(order, vec![1, 3, 0, 2]);
    }

    #[test]
    fn trust_from_a_reason() {
        let t = trust_from_reason("not trusted: /r/api on dev1");
        assert_eq!(t.folder, "/r/api");
        assert!(t.here);
        assert_eq!(t.device, "this computer");
        assert!(t.what.starts_with("Claude Code will run agents in /r/api on this computer"));
        assert_eq!(trust_from_reason("/x").folder, "/x");
    }

    #[test]
    fn agents_now_starts_off_for_people_only() {
        assert!(agents_now_default([]));
        assert!(agents_now_default(["irori"]));
        assert!(agents_now_default(["dev/oak"]));
        assert!(agents_now_default(["person:p1", "dev/oak"]));
        assert!(!agents_now_default(["person:p1"]));
        assert!(!agents_now_default(["user:u1", " person:p2 "]));
    }

    #[test]
    fn boards_waiting_on_you() {
        let c = |id: &str, board: &str, f: fn(&mut CardSummary)| {
            let mut c = CardSummary {
                id: id.into(),
                board: board.into(),
                scope: "board".into(),
                ..CardSummary::default()
            };
            f(&mut c);
            c
        };
        let cards = vec![
            c("1", "b1", |c| c.open_blockers = 1),
            c("2", "b2", |c| c.gone_wait = Some(GoneWait::default())),
            c("3", "b1", |c| c.open_blockers = 2),
            c("4", "b1", |c| {
                c.open_blockers = 1;
                c.ours = true;
            }),
            c("5", "b3", |_| {}),
            c("6", "", |c| {
                c.open_blockers = 1;
                c.scope = "private".into();
            }),
        ];
        assert_eq!(boards_need_you(&cards), vec![("b1".to_string(), 2), ("b2".to_string(), 1)]);
        // Home's own scope is the board list's (board::filter::in_scope).
        let own = crate::board::SCOPE_OWN;
        assert!(crate::board::filter::in_scope(&cards[3], own));
        assert!(!crate::board::filter::in_scope(&cards[0], own));
        assert!(crate::board::filter::in_scope(&cards[5], own));
    }
}
