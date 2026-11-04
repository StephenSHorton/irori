//! Cards that wait on cards (card #481): a loop of them never moves, and
//! neither does a chain that ends at a card nobody holds and nothing will
//! hand out. Both are found at read time from the waits as they stand, so
//! the window's one Needs you item for them goes away once a wait in the
//! loop closes or someone takes the card.

use std::collections::{HashMap, HashSet};

use super::offers::offerable;
use super::types::*;

/// How many cards a walk looks at before it stops.
const MAX_CARDS: usize = 64;

/// The cards `t` waits on: its open task waits with no options (one with
/// options is a question for the person, card #347).
pub fn task_waits(t: &Task) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for b in t
        .blockers
        .iter()
        .filter(|b| b.open() && b.options.is_empty())
    {
        if let Some(on) = b.on.as_ref().filter(|on| on.kind == BLOCKED_ON_TASK)
            && !on.reference.is_empty()
            && on.reference != t.id
            && !out.contains(&on.reference)
        {
            out.push(on.reference.clone());
        }
    }
    out
}

/// Where a card's waits on cards lead.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WaitChain {
    /// The cards that wait on each other, `start` among them: every card
    /// of the loop gets the same list, in wait order from the lowest id
    /// for a plain ring (#392 waits on #396, which waits on #394), else
    /// by id. Empty when `start` is in no loop.
    pub loop_ids: Vec<String>,
    /// The first card along the waits that nobody holds and nothing will
    /// move: not handed out, not starting an agent, and not waiting on
    /// a lease, something outside or the person. Empty when there is
    /// none, or `start` is in a loop.
    pub stuck: String,
}

/// Follows card `start`'s waits on other cards (card #481) through
/// `get`, a lookup in the log the card lives in.
pub fn wait_chain(get: &dyn Fn(&str) -> Option<Task>, start: &str) -> WaitChain {
    let mut cards: HashMap<String, Option<Task>> = HashMap::new();
    let mut look = |id: &str| -> Option<Task> {
        if !cards.contains_key(id) {
            if cards.len() >= MAX_CARDS {
                return None;
            }
            let t = get(id).filter(|t| !is_terminal(&t.state));
            cards.insert(id.to_string(), t);
        }
        cards.get(id).cloned().flatten()
    };
    let Some(first) = look(start) else {
        return WaitChain::default();
    };
    // What start reaches, breadth first, and who reaches whom.
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    let mut order = vec![start.to_string()];
    let mut seen: HashSet<String> = HashSet::from([start.to_string()]);
    let mut i = 0;
    let mut node = Some(first);
    while let Some(t) = node {
        let next: Vec<String> = task_waits(&t)
            .into_iter()
            .filter(|id| look(id).is_some())
            .collect();
        for n in &next {
            if seen.insert(n.clone()) {
                order.push(n.clone());
            }
        }
        edges.insert(t.id.clone(), next);
        i += 1;
        node = order.get(i).and_then(|id| look(id));
    }
    // The loop: the cards start reaches that reach start back.
    let back = |from: &str| -> bool {
        let mut stack = vec![from.to_string()];
        let mut been: HashSet<String> = HashSet::new();
        while let Some(id) = stack.pop() {
            if !been.insert(id.clone()) {
                continue;
            }
            for n in edges.get(&id).into_iter().flatten() {
                if n == start {
                    return true;
                }
                stack.push(n.clone());
            }
        }
        false
    };
    let ring: Vec<String> = order.iter().filter(|id| back(id)).cloned().collect();
    if !ring.is_empty() {
        return WaitChain {
            loop_ids: ring_order(ring, &edges),
            stuck: String::new(),
        };
    }
    let stuck = order
        .iter()
        .skip(1)
        .find(|id| {
            edges.get(*id).is_none_or(|e| e.is_empty()) && look(id).is_some_and(|t| stuck_end(&t))
        })
        .cloned()
        .unwrap_or_default();
    WaitChain {
        loop_ids: Vec::new(),
        stuck,
    }
}

/// A loop's cards in wait order from the lowest id when each waits on
/// exactly one other of them, else sorted.
fn ring_order(mut ring: Vec<String>, edges: &HashMap<String, Vec<String>>) -> Vec<String> {
    ring.sort();
    let inside: HashSet<&String> = ring.iter().collect();
    let next = |id: &String| -> Vec<&String> {
        edges
            .get(id)
            .into_iter()
            .flatten()
            .filter(|n| inside.contains(n))
            .collect()
    };
    if ring.iter().any(|id| next(id).len() != 1) {
        return ring;
    }
    let mut out = vec![ring[0].clone()];
    while out.len() < ring.len() {
        let n = next(out.last().unwrap())[0].clone();
        if out.contains(&n) {
            return ring;
        }
        out.push(n);
    }
    out
}

/// Whether a chain's last card stays where it is: nobody holds it or was
/// handed it, Irori won't hand it out or start an agent for it, it isn't
/// a parent (its subtasks move it) or an ask, and it has nothing open
/// that someone else ends: a wait (on a lease, a card, an agent or
/// something outside) or a question for the person (the window names
/// that as the person's answer, card #347). An item answered to an agent
/// that left is no help either.
fn stuck_end(t: &Task) -> bool {
    t.holders.is_empty()
        && t.co_holders.is_empty()
        && t.assigned.is_empty()
        && t.subtasks.is_empty()
        && t.ask.is_none()
        && t.state != INPUT_REQUIRED
        && !t.spawn_pending()
        && !offerable(t)
        && t.blockers
            .iter()
            .all(|b| !b.open() || b.status == BLOCKER_ANSWERED)
}
