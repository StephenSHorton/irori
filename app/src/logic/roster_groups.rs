//! The sidebar's agents split in two (the React app's lib/roster-groups.ts,
//! cards #456, #450, #512, #558): Yours, the terminal agents the person
//! started, and On cards, the agents Irori started for a card, grouped
//! under the card when one card has several. Only live agents are listed.

use std::collections::{HashMap, HashSet};

use irori_window::cards::CardSummary;
use irori_window::roster::RosterAgent;

use super::spawn_line::{SpawnState, queued};

/// The roster status of an agent on a permission prompt (card #445).
pub const ON_PROMPT: &str = "waiting";

/// The agents Irori started for one card.
#[derive(Clone, Debug)]
pub struct OnCard<'a> {
    pub card_id: String,
    pub short: String,
    pub tag: String,
    pub title: String,
    pub agents: Vec<&'a RosterAgent>,
}

#[derive(Clone, Debug, Default)]
pub struct RosterGroups<'a> {
    pub yours: Vec<&'a RosterAgent>,
    pub on_cards: Vec<OnCard<'a>>,
    /// Agents on cards, cards starting one, cards queued for one.
    pub spawned: usize,
    pub starting: usize,
    pub queued: usize,
    /// Live agents that are blocked: on a permission prompt, or idle on a
    /// card with an open blocker that isn't only its done check (#512).
    pub blocked: HashSet<String>,
}

/// Sorts the live roster into Yours and On cards. Cards keep their order;
/// an agent spawned for two cards shows under the first.
pub fn roster_groups<'a>(agents: &'a [RosterAgent], cards: &[CardSummary]) -> RosterGroups<'a> {
    let live: Vec<&RosterAgent> = agents.iter().filter(|a| a.card.status != "gone").collect();
    let by_key: HashMap<&str, &RosterAgent> =
        live.iter().map(|a| (a.card.key.as_str(), *a)).collect();
    let mut placed: HashSet<String> = HashSet::new();
    let mut g = RosterGroups {
        blocked: live
            .iter()
            .filter(|a| a.card.status == ON_PROMPT)
            .map(|a| a.card.key.clone())
            .collect(),
        ..RosterGroups::default()
    };
    for c in cards {
        if !c.starting.is_empty() {
            g.starting += 1;
        } else if queued(&SpawnState::from(c)) {
            g.queued += 1;
        }
        if c.open_blockers - i64::from(c.ready) > 0 {
            for w in &c.working {
                if by_key
                    .get(w.key.as_str())
                    .is_some_and(|a| a.card.status == "idle")
                {
                    g.blocked.insert(w.key.clone());
                }
            }
        }
        let mut here = Vec::new();
        for w in &c.working {
            if !w.spawned || placed.contains(&w.key) {
                continue;
            }
            let Some(a) = by_key.get(w.key.as_str()) else {
                continue;
            };
            placed.insert(w.key.clone());
            here.push(*a);
        }
        if !here.is_empty() {
            g.on_cards.push(OnCard {
                card_id: c.id.clone(),
                short: c.short.clone(),
                tag: c.tag.clone(),
                title: c.title.clone(),
                agents: here,
            });
        }
    }
    g.yours = live
        .into_iter()
        .filter(|a| !placed.contains(&a.card.key))
        .collect();
    g.spawned = placed.len();
    g
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use irori_window::cards::AgentRef;

    pub(crate) fn agent(key: &str, status: &str) -> RosterAgent {
        let mut a = RosterAgent {
            card: Default::default(),
            offers_cards: false,
            spawned: false,
            here: true,
        };
        a.card.key = key.into();
        a.card.name = key.into();
        a.card.status = status.into();
        a
    }
    fn card(short: &str, working: &[(&str, bool)]) -> CardSummary {
        CardSummary {
            id: format!("c{short}"),
            short: short.into(),
            title: format!("card {short}"),
            working: working
                .iter()
                .map(|(k, sp)| AgentRef {
                    key: k.to_string(),
                    status: "busy".into(),
                    spawned: *sp,
                    ..AgentRef::default()
                })
                .collect(),
            ..CardSummary::default()
        }
    }
    fn keys(v: &[&RosterAgent]) -> Vec<String> {
        v.iter().map(|a| a.card.key.clone()).collect()
    }

    #[test]
    fn splits_terminal_agents_from_spawned() {
        let agents = [
            agent("juniper", "idle"),
            agent("thyme", "busy"),
            agent("basil", "busy"),
        ];
        let g = roster_groups(
            &agents,
            &[
                card("58", &[("thyme", true)]),
                card("57", &[("basil", false)]),
            ],
        );
        assert_eq!(keys(&g.yours), ["juniper", "basil"]);
        assert_eq!(g.on_cards.len(), 1);
        assert_eq!(g.on_cards[0].card_id, "c58");
        assert_eq!(keys(&g.on_cards[0].agents), ["thyme"]);
        assert_eq!(g.spawned, 1);
    }

    #[test]
    fn several_on_one_card_sit_together() {
        let agents = [
            agent("fern", "busy"),
            agent("clover", "busy"),
            agent("mallow", "busy"),
        ];
        let g = roster_groups(
            &agents,
            &[
                card("61", &[("fern", true), ("clover", true)]),
                card("66", &[("mallow", true)]),
            ],
        );
        let got: Vec<(String, Vec<String>)> = g
            .on_cards
            .iter()
            .map(|c| (c.short.clone(), keys(&c.agents)))
            .collect();
        assert_eq!(
            got,
            [
                (
                    "61".to_string(),
                    vec!["fern".to_string(), "clover".to_string()]
                ),
                ("66".to_string(), vec!["mallow".to_string()])
            ]
        );
        assert!(g.yours.is_empty());
    }

    #[test]
    fn an_ended_agent_is_in_neither_group() {
        let agents = [agent("sorrel", "gone"), agent("juniper", "idle")];
        let g = roster_groups(&agents, &[card("61", &[("sorrel", true)])]);
        assert!(g.on_cards.is_empty());
        assert_eq!(keys(&g.yours), ["juniper"]);
        assert_eq!(g.spawned, 0);
    }

    #[test]
    fn counts_starting_and_queued() {
        let mut a = card("64", &[]);
        a.starting = "Macbook Pro (this one)".into();
        let mut b = card("67", &[]);
        b.at_cap = "Macbook Pro (this one)".into();
        b.queue_place = 1;
        let mut c = card("68", &[]);
        c.usage_limit = gocompat::time::GoTime::parse_rfc3339("2026-11-04T23:30:00Z");
        let mut d = card("70", &[]);
        d.cant_start = "no copy of irori on PC".into();
        let g = roster_groups(&[], &[a, b, c, d]);
        assert_eq!((g.starting, g.queued), (1, 2));
    }

    #[test]
    fn blocked_on_a_prompt_or_idle_on_a_blocked_card() {
        let agents = [
            agent("fern", "waiting"),
            agent("clover", "idle"),
            agent("mallow", "idle"),
            agent("sorrel", "gone"),
            agent("thyme", "busy"),
            agent("basil", "idle"),
        ];
        let with = |mut c: CardSummary, ob: i64, ready: bool| {
            c.open_blockers = ob;
            c.ready = ready;
            c
        };
        let g = roster_groups(
            &agents,
            &[
                with(card("61", &[("clover", true)]), 1, false),
                card("66", &[("mallow", false)]),
                with(card("67", &[("sorrel", false)]), 2, false),
                with(card("68", &[("thyme", false)]), 1, false),
                with(card("69", &[("basil", false)]), 1, true),
            ],
        );
        let mut b: Vec<String> = g.blocked.into_iter().collect();
        b.sort();
        assert_eq!(b, ["clover", "fern"]);
    }

    #[test]
    fn a_group_carries_its_card_tag() {
        let agents = [agent("thyme", "busy")];
        let mut c = card("3", &[("thyme", true)]);
        c.tag = "net".into();
        assert_eq!(roster_groups(&agents, &[c]).on_cards[0].tag, "net");
    }
}
