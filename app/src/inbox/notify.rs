//! The Notifications page's words (card #377), the React app's
//! lib/notifications.ts without its sample: what an item is about, and
//! how the thing it names ended, from that thing as it stands now (the
//! card's blocker, the message, the agent's ask).

use irori_window::cards::CardBlocker;
use irori_window::notifications::{Notification, NotificationAsk};
use irori_window::rune_inbox::{Delivery, st};

/// What an item is about (crates/coord notifications.rs).
pub mod kind {
    pub const BLOCKER: &str = "blocker";
    pub const DONE_CHECK: &str = "done_check";
    pub const INPUT: &str = "input";
    pub const HANDED: &str = "handed";
    pub const RETURNED: &str = "returned";
    pub const UNCLAIMED: &str = "unclaimed";
    pub const MENTION: &str = "mention";
    pub const ASK: &str = "ask";
    pub const MESSAGE: &str = "message";
    pub const HELD: &str = "held";
    pub const AGENT: &str = "agent";
    /// The one item for several messages that waited (card #482).
    pub const WAITED: &str = "waited";
    /// No device can start an agent for a card or thread (card #541).
    pub const NO_AGENT: &str = "no_agent";
}

/// The kind's badge.
pub fn kind_label(k: &str) -> &'static str {
    match k {
        kind::BLOCKER => "Decision",
        kind::DONE_CHECK => "Done check",
        kind::INPUT => "Question",
        kind::HANDED => "Handed to you",
        kind::RETURNED => "Came back",
        kind::UNCLAIMED => "Nobody took it",
        kind::MENTION => "Mention",
        kind::ASK => "Agent asks",
        kind::MESSAGE => "Message",
        kind::HELD => "Held",
        kind::AGENT => "Agent",
        kind::WAITED => "Messages",
        kind::NO_AGENT => "No agent",
        _ => "Notification",
    }
}

/// The badge's tone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Success,
    Destructive,
    Warning,
    Info,
    Secondary,
}

pub fn kind_tone(k: &str) -> Tone {
    match k {
        kind::BLOCKER | kind::INPUT | kind::ASK => Tone::Destructive,
        kind::DONE_CHECK => Tone::Success,
        kind::HANDED | kind::RETURNED | kind::UNCLAIMED | kind::HELD | kind::NO_AGENT => Tone::Warning,
        kind::MENTION | kind::MESSAGE | kind::WAITED => Tone::Info,
        _ => Tone::Secondary,
    }
}

/// The item is about a card's blocker, done check or question.
pub fn asks_on_card(n: &Notification) -> bool {
    !n.task_id.is_empty() && matches!(n.kind.as_str(), kind::BLOCKER | kind::DONE_CHECK | kind::INPUT)
}

/// The item is the "N messages waited for you" one, which stands for
/// several messages (card #482); before #482 it was a message about
/// delivery "waiting".
pub fn waited_summary(n: &Notification) -> bool {
    n.kind == kind::WAITED || (n.kind == kind::MESSAGE && n.delivery_id == "waiting")
}

/// Which items the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    Unread,
    All,
}

/// The list a filter shows, newest first as it comes.
pub fn shown(list: &[Notification], f: Filter) -> Vec<&Notification> {
    list.iter().filter(|n| f == Filter::All || !n.read).collect()
}

pub fn unread_count(list: &[Notification]) -> usize {
    list.iter().filter(|n| !n.read).count()
}

fn who(name: &str) -> String {
    match name.trim() {
        "" => "someone".into(),
        "You" => "you".into(),
        n => n.to_string(),
    }
}

/// How an agent's next done check resolves the one it replaces (coord's
/// ASKED_AGAIN).
pub const ASKED_AGAIN: &str = "asked again: ready for the person";

/// How a resolved blocker or done check ended, in the card page's words:
/// "" while it still waits on the person.
pub fn blocker_outcome(b: &CardBlocker) -> String {
    let Some(r) = &b.resolved else {
        if b.state == "answered" {
            return "Answered; it's with the agent now".into();
        }
        return String::new();
    };
    let by = who(&r.by.name);
    let note = r.note.trim();
    if r.closed {
        return if note.is_empty() { format!("Closed by {by}") } else { format!("Closed by {by}: {note}") };
    }
    let answer = r.answer.trim();
    // The agent's next done check replaced this one (card #476): the
    // reason on it is the person's.
    if b.done_check && note == ASKED_AGAIN {
        return if !answer.is_empty() && answer != ASKED_AGAIN {
            format!("You sent it back: “{answer}”; {by} asked again")
        } else {
            format!("{by} asked again")
        };
    }
    if b.done_check {
        return if r.option >= 0 {
            format!("Marked done by {by}")
        } else if answer.is_empty() {
            format!("Sent back by {by}")
        } else {
            format!("Sent back by {by}: “{answer}”")
        };
    }
    if r.option >= 0 {
        return format!("Answered “{answer}” by {by}");
    }
    if answer.is_empty() { format!("Answered by {by}") } else { format!("Answered by {by}: “{answer}”") }
}

/// The blocker still waits on this person: open or replied back to them,
/// not another member's question nor a wait on other work (card #481).
pub fn blocker_open(b: &CardBlocker) -> bool {
    b.resolved.is_none()
        && b.decider.is_none()
        && b.waits_on.is_none()
        && matches!(b.state.as_str(), "" | "open" | "replied")
}

/// An agent's message to the person, not answered or read.
pub fn message_open(d: &Delivery) -> bool {
    matches!(d.state.as_str(), st::QUEUED | st::DELIVERED | st::ENGAGED | st::DEFERRED)
}

/// What became of a message or a held one: "" while it still waits on the
/// person.
pub fn delivery_outcome(d: &Delivery, held: bool) -> &'static str {
    if held {
        return match d.state.as_str() {
            st::HELD => "",
            st::DISMISSED => "Dismissed",
            st::REJECTED => "Refused",
            _ => "Released",
        };
    }
    if message_open(d) {
        return "";
    }
    match d.state.as_str() {
        st::ANSWERED => "Replied",
        st::SKIPPED => "Marked read",
        st::DISMISSED => "Dismissed",
        st::HELD => "Held by Irori",
        st::MOVED => "Moved to another agent",
        _ => "Closed",
    }
}

/// How an agent's ask ended: "" while it's open.
pub fn ask_outcome(a: &NotificationAsk) -> String {
    if a.open {
        String::new()
    } else if a.answered && !a.choice.is_empty() {
        format!("Answered “{}”", a.choice)
    } else if a.answered {
        "Dismissed".into()
    } else {
        "Closed: no answer in time".into()
    }
}

// ---- a permission prompt (cards/permission-prompt.tsx, card #682) -------------

pub const ALLOW_ONCE: &str = "Allow once";
pub const ALWAYS_ALLOW: &str = "Always allow";
pub const DENY: &str = "Deny";

/// A permission prompt's buttons, in the order they come in whatever
/// order the item's options are, each with its option's index. Always
/// allow only when a rule helps (`rule` non-empty).
pub fn permission_answers(b: &CardBlocker, rule: &str) -> Vec<(&'static str, usize)> {
    [ALLOW_ONCE, ALWAYS_ALLOW, DENY]
        .into_iter()
        .filter(|a| *a != ALWAYS_ALLOW || !rule.is_empty())
        .filter_map(|a| b.options.iter().position(|o| o.answer == a).map(|i| (a, i)))
        .collect()
}

/// What a permission pick says while the card catches up.
pub fn permission_said(answer: &str, name: &str, rule: &str) -> String {
    match answer {
        DENY => format!("Denied: {name} goes on without it."),
        ALWAYS_ALLOW => format!("Allowed, and {rule} is on the board's allow list."),
        _ => format!("Allowed once: {name} goes on."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::cards::{AgentRef, CardResolution, CardWaitOn};

    fn n(kind: &str, task: &str, delivery: &str) -> Notification {
        Notification {
            kind: kind.into(),
            task_id: task.into(),
            delivery_id: delivery.into(),
            ..Notification::default()
        }
    }

    fn agent(key: &str, name: &str) -> AgentRef {
        AgentRef {
            key: key.into(),
            name: name.into(),
            ..AgentRef::default()
        }
    }

    #[test]
    fn the_summary_card_is_one_row_for_several_messages_old_rows_too() {
        assert!(waited_summary(&n("waited", "", "d2")));
        assert!(waited_summary(&n("message", "", "waiting")));
        assert!(!waited_summary(&n("message", "", "d1")));
    }

    #[test]
    fn unread_shows_only_unread_ones_all_every_one_in_order() {
        let mk = |key: &str, read: bool| Notification {
            key: key.into(),
            read,
            ..Notification::default()
        };
        let list = vec![mk("a", false), mk("b", true), mk("c", false)];
        let keys = |f| shown(&list, f).iter().map(|n| n.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(Filter::Unread), ["a", "c"]);
        assert_eq!(keys(Filter::All), ["a", "b", "c"]);
        assert_eq!(unread_count(&list), 2);
    }

    #[test]
    fn only_a_cards_question_or_done_check_reads_the_cards_blocker() {
        assert!(asks_on_card(&n("blocker", "T", "")));
        assert!(asks_on_card(&n("done_check", "T", "")));
        assert!(!asks_on_card(&n("mention", "T", "")));
        assert!(!asks_on_card(&n("blocker", "", "")));
    }

    #[test]
    fn open_keeps_its_buttons_another_members_question_doesnt() {
        let b = |state: &str| CardBlocker {
            id: "b1".into(),
            state: state.into(),
            ..CardBlocker::default()
        };
        assert!(blocker_open(&b("open")));
        assert!(blocker_open(&b("replied")));
        let mut theirs = b("open");
        theirs.decider = Some(agent("dev/abc", "civet"));
        assert!(!blocker_open(&theirs));
    }

    #[test]
    fn a_wait_on_another_card_has_no_buttons() {
        let wait = CardBlocker {
            id: "b1".into(),
            state: "open".into(),
            waits_on: Some(CardWaitOn {
                kind: "task".into(),
                label: "#392".into(),
                ..CardWaitOn::default()
            }),
            ..CardBlocker::default()
        };
        assert!(!blocker_open(&wait));
    }

    #[test]
    fn resolved_by_anyone_no_buttons_the_outcome_and_who() {
        let you = agent("user:dev", "You");
        let civet = agent("dev/abc", "civet");
        let res = |answer: &str, option: i64, by: &AgentRef, note: &str, closed: bool| CardResolution {
            answer: answer.into(),
            option,
            by: by.clone(),
            note: note.into(),
            closed,
            ..CardResolution::default()
        };
        let done = CardBlocker {
            done_check: true,
            state: "resolved".into(),
            resolved: Some(res("Done", 0, &you, "", false)),
            ..CardBlocker::default()
        };
        assert!(!blocker_open(&done));
        assert_eq!(blocker_outcome(&done), "Marked done by you");
        let back = CardBlocker {
            done_check: true,
            resolved: Some(res("the far dock clips", -1, &you, "", false)),
            ..CardBlocker::default()
        };
        assert_eq!(blocker_outcome(&back), "Sent back by you: “the far dock clips”");
        let again = CardBlocker {
            done_check: true,
            resolved: Some(res("the far dock clips", -1, &civet, ASKED_AGAIN, false)),
            ..CardBlocker::default()
        };
        assert_eq!(blocker_outcome(&again), "You sent it back: “the far dock clips”; civet asked again");
        let picked = CardBlocker {
            resolved: Some(res("Wintun", 0, &civet, "", false)),
            ..CardBlocker::default()
        };
        assert_eq!(blocker_outcome(&picked), "Answered “Wintun” by civet");
        let closed = CardBlocker {
            resolved: Some(res("", -1, &civet, "the lease came back", true)),
            ..CardBlocker::default()
        };
        assert_eq!(blocker_outcome(&closed), "Closed by civet: the lease came back");
        let answered = CardBlocker {
            state: "answered".into(),
            ..CardBlocker::default()
        };
        assert_eq!(blocker_outcome(&answered), "Answered; it's with the agent now");
    }

    fn d(state: &str) -> Delivery {
        let mut d = Delivery::default();
        d.id = "d".into();
        d.state = state.into();
        d
    }

    #[test]
    fn a_message_to_the_person_waits_until_replied_or_read() {
        assert_eq!(delivery_outcome(&d("delivered"), false), "");
        assert_eq!(delivery_outcome(&d("answered"), false), "Replied");
        assert_eq!(delivery_outcome(&d("skipped"), false), "Marked read");
    }

    #[test]
    fn a_held_message_waits_until_released_or_dismissed() {
        assert_eq!(delivery_outcome(&d("held"), true), "");
        assert_eq!(delivery_outcome(&d("delivered"), true), "Released");
        assert_eq!(delivery_outcome(&d("dismissed"), true), "Dismissed");
    }

    #[test]
    fn an_agents_ask_open_answered_or_out_of_time() {
        let a = |open: bool, answered: bool, choice: &str| NotificationAsk {
            open,
            answered,
            choice: choice.into(),
            ..NotificationAsk::default()
        };
        assert_eq!(ask_outcome(&a(true, false, "")), "");
        assert_eq!(ask_outcome(&a(false, true, "Restart")), "Answered “Restart”");
        assert_eq!(ask_outcome(&a(false, false, "")), "Closed: no answer in time");
    }

    #[test]
    fn kinds_have_labels_and_tones() {
        assert_eq!(kind_label("done_check"), "Done check");
        assert_eq!(kind_label("zzz"), "Notification");
        assert_eq!(kind_tone("ask"), Tone::Destructive);
        assert_eq!(kind_tone("zzz"), Tone::Secondary);
    }

    #[test]
    fn a_permission_prompts_buttons_come_in_their_order() {
        use irori_window::cards::CardOption;
        let opt = |a: &str| CardOption {
            answer: a.into(),
            ..CardOption::default()
        };
        let b = CardBlocker {
            options: vec![opt(DENY), opt(ALWAYS_ALLOW), opt(ALLOW_ONCE)],
            ..CardBlocker::default()
        };
        assert_eq!(
            permission_answers(&b, "Bash(cargo test:*)"),
            vec![(ALLOW_ONCE, 2), (ALWAYS_ALLOW, 1), (DENY, 0)]
        );
        // No rule helps (a bypass agent's safety check): no Always allow.
        assert_eq!(permission_answers(&b, ""), vec![(ALLOW_ONCE, 2), (DENY, 0)]);
        assert_eq!(permission_said(DENY, "kestrel", ""), "Denied: kestrel goes on without it.");
        assert_eq!(
            permission_said(ALWAYS_ALLOW, "kestrel", "Bash(ls:*)"),
            "Allowed, and Bash(ls:*) is on the board's allow list."
        );
        assert_eq!(permission_said(ALLOW_ONCE, "kestrel", ""), "Allowed once: kestrel goes on.");
    }
}
