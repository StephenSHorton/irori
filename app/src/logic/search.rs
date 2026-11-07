//! The Board's search box (the React app's lib/search.ts, card #171): it
//! narrows the loaded cards to the ones every typed word matches. A word
//! with a # is a card ref (#12, mlg#12) and matches that card exactly; any
//! other word matches anywhere in its ref, title, open question, its
//! searchText (objective, done-when, steps) or the names of who holds, is
//! offered, watches or made it.

use irori_window::cards::{AgentRef, CardSummary};

use super::names::display_name;

/// A query's words, lower-cased; a blank query has none.
pub fn query_words(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split_whitespace()
        .filter(|w| *w != "#")
        .map(str::to_string)
        .collect()
}

fn haystack(c: &CardSummary) -> String {
    let mut parts = vec![
        format!("#{}", c.short),
        if c.tag.is_empty() {
            String::new()
        } else {
            format!("{}#{}", c.tag, c.short)
        },
        c.title.clone(),
        c.blocker_question.clone(),
        c.search_text.clone(),
    ];
    let mut add = |r: &AgentRef| {
        parts.push(display_name(r));
        parts.push(r.name.clone());
    };
    c.working.iter().for_each(&mut add);
    c.watching.iter().for_each(&mut add);
    add(&c.holder.who);
    if let Some(t) = c.ask.as_ref().and_then(|a| a.taker.as_ref()) {
        add(t);
    }
    if let Some(r) = &c.returned {
        add(r);
    }
    parts.join("\n").to_lowercase()
}

/// The ways a card's ref can be typed: #12, mlg#12 and the short it had.
fn refs(c: &CardSummary) -> Vec<String> {
    let mut out = vec![format!("#{}", c.short)];
    if !c.tag.is_empty() {
        out.push(format!("{}#{}", c.tag, c.short));
    }
    if !c.short_was.is_empty() {
        out.push(format!("#{}", c.short_was));
    }
    out.into_iter().map(|r| r.to_lowercase()).collect()
}

/// Whether every word of `query` matches `c`.
pub fn card_matches(c: &CardSummary, query: &str) -> bool {
    let words = query_words(query);
    if words.is_empty() {
        return true;
    }
    let mut hay: Option<String> = None;
    words.iter().all(|w| {
        if w.contains('#') {
            return refs(c).contains(w);
        }
        hay.get_or_insert_with(|| haystack(c)).contains(w.as_str())
    })
}

/// The cards every word of `query` matches, in their order.
pub fn matching<'a>(cards: &[&'a CardSummary], query: &str) -> Vec<&'a CardSummary> {
    if query_words(query).is_empty() {
        return cards.to_vec();
    }
    cards
        .iter()
        .copied()
        .filter(|c| card_matches(c, query))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::cards::{CardAsk, CardHolder};

    fn agent(key: &str, name: &str, status: &str) -> AgentRef {
        AgentRef {
            key: key.into(),
            name: name.into(),
            status: status.into(),
            live: true,
            where_: "Macbook Pro (this one)".into(),
            here: true,
            ..AgentRef::default()
        }
    }
    fn card() -> CardSummary {
        let heron = agent("dev/heron-key", "sample-heron", "busy");
        CardSummary {
            id: "a".into(),
            short: "12".into(),
            title: "Fix the boat clipping through the dock".into(),
            blocker_question: "Clamp the roll at 30°?".into(),
            search_text: "Boats stop sinking into the pier\nNo clipping in the test scene\nTune the hull collider\nYou".into(),
            working: vec![heron.clone()],
            watching: vec![agent("dev/wren-key", "sample-wren", "idle")],
            holder: CardHolder {
                who: heron,
                ..CardHolder::default()
            },
            ..CardSummary::default()
        }
    }

    #[test]
    fn query_words_split_and_lower() {
        assert_eq!(query_words("  Boat   DOCK # "), ["boat", "dock"]);
        assert!(query_words("").is_empty());
    }

    #[test]
    fn matches() {
        let c = card();
        assert!(card_matches(&c, ""));
        assert!(card_matches(&c, "   "));
        assert!(card_matches(&c, "DOCK boat"));
        assert!(card_matches(&c, "clip"));
        assert!(!card_matches(&c, "boat release"));
        for q in [
            "pier",
            "test scene",
            "collider",
            "you",
            "clamp roll",
            "heron",
            "wren",
        ] {
            assert!(card_matches(&c, q), "{q}");
        }
        assert!(!card_matches(&c, "willow"));
    }

    #[test]
    fn an_ask_matches_its_taker() {
        let ask = CardSummary {
            working: vec![],
            watching: vec![],
            holder: CardHolder::default(),
            ask: Some(CardAsk {
                taker: Some(agent("dev/wren-key", "willow", "idle")),
                ..CardAsk::default()
            }),
            ..card()
        };
        assert!(card_matches(&ask, "willow"));
    }

    #[test]
    fn a_hash_word_is_the_ref_exactly() {
        let c = card();
        assert!(card_matches(&c, "#12"));
        assert!(!card_matches(&c, "#1"));
        assert!(!card_matches(&c, "#120"));
        assert!(card_matches(&c, "#12 dock"));
        assert!(!card_matches(&c, "#12 release"));
        assert!(card_matches(&c, "12"));
    }

    #[test]
    fn a_shared_card_matches_tag_and_old_short() {
        let shared = CardSummary {
            tag: "mlg".into(),
            short: "3".into(),
            short_was: "12".into(),
            ..card()
        };
        for q in ["mlg#3", "MLG#3", "#3", "#12"] {
            assert!(card_matches(&shared, q), "{q}");
        }
        assert!(!card_matches(&shared, "abc#3"));
    }

    #[test]
    fn bare_cards_and_order() {
        let bare = CardSummary {
            id: "b".into(),
            short: "7".into(),
            title: "Ship the release".into(),
            ..CardSummary::default()
        };
        assert!(card_matches(&bare, "release"));
        assert!(!card_matches(&bare, "pier"));
        let c = card();
        let other = CardSummary {
            id: "b".into(),
            short: "7".into(),
            title: "Boat physics pass".into(),
            ..CardSummary::default()
        };
        let all = [&c, &other];
        let ids = |v: Vec<&CardSummary>| v.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(matching(&all, "boat")), ["a", "b"]);
        assert_eq!(ids(matching(&all, "dock")), ["a"]);
        assert_eq!(matching(&all, "").len(), 2);
        assert!(matching(&all, "nothing-like-it").is_empty());
    }
}
