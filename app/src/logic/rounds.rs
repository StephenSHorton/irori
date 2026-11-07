//! A done check the person sent back (the React app's lib/rounds.ts, card
//! #476): their reason, the agent's answers on it since, and whether the
//! agent heard it.

use irori_window::cards::CardThreadItem;

use super::thread::KIND_COMMENT;

#[derive(Clone, Debug, PartialEq)]
pub struct SentBack {
    /// The person's reason: their last own words on the check.
    pub reason: String,
    pub reason_at: Option<i64>,
    /// The agents' answers after the reason, oldest first.
    pub replies: Vec<CardThreadItem>,
    /// An agent read the reason (or answered it).
    pub heard: bool,
    pub answered: bool,
    /// Who has it but hasn't read it yet.
    pub pending: Vec<String>,
}

fn person(it: &CardThreadItem) -> bool {
    it.who.status == "person"
}

fn agent(it: &CardThreadItem) -> bool {
    it.who.status != "person" && it.who.status != "irori"
}

/// Reads a done check's thread: `None` when the person said nothing on
/// it. `fallback` is the reason when no comment carries it (a card from
/// before #476).
pub fn sent_back(thread: &[CardThreadItem], fallback: &str) -> Option<SentBack> {
    let comments: Vec<&CardThreadItem> = thread.iter().filter(|it| it.kind == KIND_COMMENT).collect();
    let Some(at) = comments.iter().rposition(|it| person(it)) else {
        if fallback.trim().is_empty() {
            return None;
        }
        let replies: Vec<CardThreadItem> = comments.into_iter().filter(|it| agent(it)).cloned().collect();
        let any = !replies.is_empty();
        return Some(SentBack {
            reason: fallback.into(),
            reason_at: None,
            replies,
            heard: any,
            answered: any,
            pending: vec![],
        });
    };
    let reason = comments[at];
    let replies: Vec<CardThreadItem> = comments[at + 1..].iter().filter(|it| agent(it)).map(|it| (*it).clone()).collect();
    let answered = !replies.is_empty();
    Some(SentBack {
        reason: reason.text.clone(),
        reason_at: super::agent_status::ms(&reason.at),
        replies,
        heard: answered || !reason.heard_by.is_empty(),
        answered,
        pending: if answered { vec![] } else { reason.pending.clone() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::cards::AgentRef;

    fn who(key: &str, status: &str) -> AgentRef {
        AgentRef {
            key: key.into(),
            name: key.into(),
            status: status.into(),
            ..AgentRef::default()
        }
    }
    fn reason() -> CardThreadItem {
        CardThreadItem {
            id: "c1".into(),
            kind: "comment".into(),
            who: who("person", "person"),
            text: "the others haven't been told the next steps".into(),
            blocker_id: "b1".into(),
            ..CardThreadItem::default()
        }
    }
    fn reply(id: &str, text: &str) -> CardThreadItem {
        CardThreadItem {
            id: id.into(),
            kind: "comment".into(),
            who: who("mac/card-81-1", "idle"),
            text: text.into(),
            ..CardThreadItem::default()
        }
    }

    #[test]
    fn nothing_said_no_round() {
        assert!(sent_back(&[], "").is_none());
        let ev = CardThreadItem {
            kind: "event".into(),
            who: who("irori", "irori"),
            ..CardThreadItem::default()
        };
        assert!(sent_back(&[ev], "").is_none());
    }

    #[test]
    fn sent_not_read_yet() {
        let mut r = reason();
        r.pending = vec!["sent to card-81-1, not read yet".into()];
        let s = sent_back(&[r], "").unwrap();
        assert_eq!(s.reason, "the others haven't been told the next steps");
        assert!(!s.heard && !s.answered);
        assert_eq!(s.pending, ["sent to card-81-1, not read yet"]);
    }

    #[test]
    fn read_heard_not_answered() {
        let mut r = reason();
        r.heard_by = vec!["mac/card-81-1".into()];
        let s = sent_back(&[r], "").unwrap();
        assert!(s.heard && !s.answered);
    }

    #[test]
    fn answered_after_the_reason() {
        let s = sent_back(&[reply("c0", "earlier"), reason(), reply("c2", "made #108")], "").unwrap();
        assert_eq!(s.replies.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["c2"]);
        assert!(s.heard && s.answered);
        assert!(s.pending.is_empty());
    }

    #[test]
    fn the_newest_reason_counts() {
        let mut again = reason();
        again.id = "c3".into();
        again.text = "still no note to the team".into();
        let s = sent_back(&[reason(), reply("c2", "on it"), again], "").unwrap();
        assert_eq!(s.reason, "still no note to the team");
        assert!(!s.answered);
    }

    #[test]
    fn an_older_card_reason_in_the_answer() {
        let s = sent_back(&[], "also the tests").unwrap();
        assert_eq!(s.reason, "also the tests");
        assert!(!s.answered);
        assert!(sent_back(&[], "  ").is_none());
    }
}
