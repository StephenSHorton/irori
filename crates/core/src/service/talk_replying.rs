//! Talk's replying row (card #645): the agents a post in a channel or DM
//! woke that haven't answered there yet, so the person sees they heard.
//!
//! An agent counts from the wake (a DM to it, or an @mention) until it
//! posts in that conversation, the turn the wake started ends, or the wake
//! is held, deferred, skipped or moved; and never past [`REPLYING_CAP`],
//! or once it is gone, not answering or on a device that's offline.
//! Another person's agents are woken on their own device, so they never
//! show here.

use std::collections::BTreeMap;
use std::time::Duration;

use coord::talk::Post;
use gocompat::time::GoTime;

use super::{Core, ToolsView};

/// The longest an agent shows as replying after its wake.
pub const REPLYING_CAP: Duration = Duration::from_secs(30 * 60);

/// A Talk post and its wake share the poster's clock, but the two rows
/// are stamped apart: this much either side still matches.
const WAKE_SLACK: Duration = Duration::from_secs(10);

/// The agents replying in one conversation, the earliest woken first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalkReplying {
    pub conv_id: String,
    pub agents: Vec<Replier>,
}

/// One agent replying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replier {
    pub key: String,
    /// Its name, never its key.
    pub name: String,
    /// When the post that woke it was sent.
    pub since: GoTime,
}

impl Core {
    /// The agents replying in each conversation `reader` may read
    /// ([`talk_replying`](self)), conversations with none left out.
    pub fn talk_replying(&self, reader: &str) -> Vec<TalkReplying> {
        let now = (self.s.now)();
        let since = sub(&now, REPLYING_CAP);
        let wakes = self.s.inbox.talk_wakes(since);
        if wakes.is_empty() {
            return vec![];
        }
        let cards: BTreeMap<String, coord::roster::AgentCard> = self
            .mesh_agents()
            .into_iter()
            .map(|c| (c.key.clone(), c))
            .collect();
        let view = ToolsView(self.s.clone());
        let mut posts: BTreeMap<String, Option<Vec<Post>>> = BTreeMap::new();
        let mut out: BTreeMap<String, Vec<Replier>> = BTreeMap::new();
        for w in wakes {
            let Some(card) = cards.get(&w.agent) else {
                continue;
            };
            if !coord::roster::answers(&card.status)
                || card.device_state == coord::roster::OFFLINE
                || done_by_status(&w, card)
            {
                continue;
            }
            let ps = posts
                .entry(w.conv.clone())
                .or_insert_with(|| tools::talk::posts(&view, reader, &w.conv).ok());
            let Some(ps) = ps else { continue };
            if posted_since(ps, &w) {
                continue;
            }
            let name = self.agent_name(&w.agent);
            let name = if name.trim().is_empty() {
                card.name.clone()
            } else {
                name
            };
            if name.trim().is_empty() {
                continue; // never a key on the row
            }
            out.entry(w.conv).or_default().push(Replier {
                key: w.agent,
                name,
                since: w.woken_at,
            });
        }
        out.into_iter()
            .map(|(conv_id, mut agents)| {
                agents.sort_by(|a, b| {
                    ns(&a.since)
                        .cmp(&ns(&b.since))
                        .then_with(|| a.name.cmp(&b.name))
                });
                TalkReplying { conv_id, agents }
            })
            .collect()
    }
}

/// Whether the agent's own status says it is done with `w` though its
/// inbox doesn't: taken up on the channel path (no turn rows), it is on
/// only while busy or waiting on a prompt, except a Grok member, whose card
/// always says idle; with turn rows, a turn its session never reported
/// ending (it crashed) is over once the agent went idle after the wake.
fn done_by_status(w: &inbox::TalkWake, card: &coord::roster::AgentCard) -> bool {
    if w.waiting {
        return false;
    }
    if w.no_turn_rows {
        return card.session_kind != "member"
            && !matches!(card.status.as_str(), "busy" | "waiting");
    }
    card.status == "idle"
        && !card.status_since.is_zero()
        && ns(&card.status_since) > ns(&add(&w.woken_at, WAKE_SLACK))
}

/// Whether the agent of `w` has posted in its conversation since the post
/// that woke it. Posts are in the log's order, so the agent's device's
/// clock doesn't matter once the waking post is found: a DM's post has
/// the message's id; a mention's is the poster's post naming the agent
/// nearest the wake's time. Without it, the agent's posts are timed.
fn posted_since(posts: &[Post], w: &inbox::TalkWake) -> bool {
    let by_agent = |p: &Post| coord::talk::canon(&p.by) == w.agent;
    let from = coord::talk::canon(&w.from);
    let (early, late) = (sub(&w.woken_at, WAKE_SLACK), add(&w.woken_at, WAKE_SLACK));
    let waking = posts
        .iter()
        .rposition(|p| p.post_id == w.message_id)
        .or_else(|| {
            posts
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    coord::talk::canon(&p.by) == from
                        && p.mentions.iter().any(|m| m == &w.agent)
                        && ns(&p.at) >= ns(&early)
                        && ns(&p.at) <= ns(&late)
                })
                .min_by_key(|(_, p)| (ns(&p.at) - ns(&w.woken_at)).abs())
                .map(|(i, _)| i)
        });
    match waking {
        Some(i) => posts[i + 1..].iter().any(by_agent),
        None => posts
            .iter()
            .any(|p| by_agent(p) && ns(&p.at) > ns(&w.woken_at)),
    }
}

fn ns(t: &GoTime) -> i128 {
    i128::from(t.secs) * 1_000_000_000 + i128::from(t.nanos)
}

fn add(t: &GoTime, d: Duration) -> GoTime {
    from_ns(ns(t) + d.as_nanos() as i128, t)
}

fn sub(t: &GoTime, d: Duration) -> GoTime {
    from_ns(ns(t) - d.as_nanos() as i128, t)
}

fn from_ns(n: i128, like: &GoTime) -> GoTime {
    GoTime {
        secs: n.div_euclid(1_000_000_000) as i64,
        nanos: n.rem_euclid(1_000_000_000) as u32,
        offset: like.offset,
    }
}
