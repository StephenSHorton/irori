//! The Board's canvas as data (cards #80, #93, #587; the React app's
//! components/canvas/data.ts): the board as a graph (areas, cards,
//! blockers, agents and the edges between them) and where each node sits.
//! It reads what the grid already has (the cards), plus a card's
//! [`CardDetail`] when one is to hand, for one blocker node per open
//! blocker. A card the person dragged keeps its spot (`CardSummary::pos`,
//! kept in coord so every device shows it); the rest are laid out by
//! [`auto_layout`]. Nothing here calls the backend.
//!
//! The layout says who is doing what (card #587): each live agent is a hub
//! with the cards it holds in a small grid under it, the cards nobody live
//! is on sit in their own area, and done cards are tucked in a faded area
//! at the end. Nearness means something; nothing grows into a tall column.
//!
//! Every list here keeps its input order (no hash-map iteration reaches the
//! graph or the layout), so the same cards always give the same picture.

use std::collections::{HashMap, HashSet};

use irori_window::cards::{AgentRef, CardDetail, CardSummary};

use crate::logic::agent_status::{Tone as StatusTone, Who, agent_status, now_ms};
use crate::logic::cards::{Tone, blocker, card_tone, nobody_on, open_ask, over};
use crate::logic::names::display_name;
use crate::logic::needs::needs_you;

// ---- the graph --------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

const fn pt(x: f32, y: f32) -> Point {
    Point { x, y }
}

/// One face on a card: who, and in which role.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceRef {
    pub key: String,
    pub name: String,
    pub watching: bool,
    pub live: bool,
    pub person: bool,
    pub color: String,
    pub agent: AgentRef,
}

/// What an area holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    /// An agent's hub: the agent and the cards it holds first.
    Agent,
    /// The cards nobody is on.
    Unheld,
    /// The agents here with no card in a hub of their own (watching,
    /// talking, a second holder).
    Others,
    /// The done cards.
    Done,
}

/// An area on the canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupNode {
    /// "group:agent:<key>", "group:unheld", "group:others" or "group:done".
    pub id: String,
    /// "working · 3 cards", "Nobody on these · 4", "Done · 2".
    pub label: String,
    pub by: By,
    /// The agent's key, "" for the other areas.
    pub key: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CardNode {
    /// The card id, so open_card(id) works as is.
    pub id: String,
    pub label: String,
    pub short: String,
    pub state: String,
    /// The grid's status colour (card_tone).
    pub tone: Tone,
    pub done_steps: i64,
    pub total_steps: i64,
    pub open_blockers: i64,
    /// Answered blockers waiting on an agent.
    pub waiting: i64,
    /// The parent card's id, when it is on the canvas too.
    pub parent: Option<String>,
    pub subtasks: i64,
    /// Its area's id.
    pub group: String,
    pub repo: String,
    pub board: String,
    /// The board's name or the repo's, as a person reads it.
    pub place: String,
    // Markers (card #587): it waits on the person, nobody live is on it,
    // nothing happened on it for a while (the host's stale), it is over.
    pub needs: bool,
    pub nobody: bool,
    pub stale: bool,
    pub done: bool,
    pub faces: Vec<FaceRef>,
    /// The source card, for the real Irori bits (state pill, faces).
    pub card: CardSummary,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockerNode {
    /// "blocker:<cardId>:<blockerId>" ("open" when only the summary is known).
    pub id: String,
    /// The question.
    pub label: String,
    /// The blocked card's id.
    pub card: String,
    pub state: String,
    pub group: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentNode {
    /// "agent:<key>".
    pub id: String,
    pub label: String,
    pub agent: AgentRef,
    pub live: bool,
    pub person: bool,
    /// Its hub's id, or "group:others".
    pub group: String,
    /// agent_status's words: "working", "idle", ...
    pub status: String,
    /// agent_status's tone: working pulses.
    pub tone: StatusTone,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Group(GroupNode),
    Card(Box<CardNode>),
    Blocker(BlockerNode),
    Agent(Box<AgentNode>),
}

/// A node that isn't an area: its kind for sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Card,
    Blocker,
    Agent,
}

impl Node {
    pub fn id(&self) -> &str {
        match self {
            Node::Group(n) => &n.id,
            Node::Card(n) => &n.id,
            Node::Blocker(n) => &n.id,
            Node::Agent(n) => &n.id,
        }
    }

    /// The area it sits in (an area's own id for an area).
    pub fn group(&self) -> &str {
        match self {
            Node::Group(n) => &n.id,
            Node::Card(n) => &n.group,
            Node::Blocker(n) => &n.group,
            Node::Agent(n) => &n.group,
        }
    }

    pub fn kind(&self) -> Option<Kind> {
        match self {
            Node::Group(_) => None,
            Node::Card(_) => Some(Kind::Card),
            Node::Blocker(_) => Some(Kind::Blocker),
            Node::Agent(_) => Some(Kind::Agent),
        }
    }
}

/// talk: two agents wrote to each other on a card in the last few minutes
/// (CardSummary.talking, the host's talk window).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Subtask,
    Blocker,
    Holder,
    Watcher,
    Talk,
}

impl EdgeKind {
    fn word(self) -> &'static str {
        match self {
            EdgeKind::Subtask => "subtask",
            EdgeKind::Blocker => "blocker",
            EdgeKind::Holder => "holder",
            EdgeKind::Watcher => "watcher",
            EdgeKind::Talk => "talk",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// "<kind>:<source>-><target>".
    pub id: String,
    pub kind: EdgeKind,
    /// Parent card / blocker / agent node id.
    pub source: String,
    /// Child card / blocked card / held card / agent id.
    pub target: String,
    /// A talk line's cards: "#12, #14".
    pub label: String,
    /// A holder line to a card in that agent's own hub: the box already
    /// says it, so the canvas shows it only on hover.
    pub in_hub: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Graph {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id() == id)
    }

    pub fn cards(&self) -> impl Iterator<Item = &CardNode> {
        self.nodes.iter().filter_map(|n| match n {
            Node::Card(c) => Some(&**c),
            _ => None,
        })
    }
}

/// Laid out: every node but an area has a top-left position and a size; an
/// area has the rect around its members. Positions are absolute.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub pos: HashMap<String, Point>,
    pub size: HashMap<String, Size>,
    pub groups: HashMap<String, Rect>,
}

// ---- cards to graph ---------------------------------------------------------

/// A repo key as a person reads it: "remote:github.com/org/repo" to
/// "org/repo", a local path to its last folder.
pub fn repo_label(repo_key: &str) -> String {
    let k = repo_key.strip_prefix("remote:").unwrap_or(repo_key);
    let k = k.strip_prefix("local:").unwrap_or(k);
    if let Some((host, rest)) = k.split_once('/') {
        let dotted = host.find('.').is_some_and(|i| i > 0 && i + 1 < host.len());
        if dotted && !rest.is_empty() {
            return rest.to_string();
        }
    }
    k.split(['/', '\\'])
        .rfind(|s| !s.is_empty())
        .unwrap_or(k)
        .to_string()
}

fn where_of(c: &CardSummary) -> String {
    if !c.board.is_empty() {
        return if c.scope_label.is_empty() { c.board.clone() } else { c.scope_label.clone() };
    }
    if !c.repo_key.is_empty() {
        return repo_label(&c.repo_key);
    }
    String::new()
}

fn face_of(a: &AgentRef, watching: bool) -> FaceRef {
    FaceRef {
        key: a.key.clone(),
        name: display_name(a),
        watching,
        live: a.live,
        person: a.status == "person",
        color: a.color.clone(),
        agent: a.clone(),
    }
}

/// An agent is there to carry a card: running, on a device that's online,
/// not gone. The person always is.
pub fn here(a: &AgentRef) -> bool {
    if a.status == "person" {
        return true;
    }
    a.live && a.status != "gone" && !a.offline && a.status != "unknown"
}

pub const UNHELD_ID: &str = "group:unheld";
pub const OTHERS_ID: &str = "group:others";
pub const DONE_ID: &str = "group:done";

pub fn hub_id(key: &str) -> String {
    format!("group:agent:{key}")
}

/// Where a card sits: done cards in Done, else under its first holder that
/// is here, else with the cards nobody is on.
pub fn hub_of(c: &CardSummary) -> String {
    if over(&c.state) {
        return DONE_ID.into();
    }
    match c.working.iter().find(|a| here(a)) {
        Some(h) => hub_id(&h.key),
        None => UNHELD_ID.into(),
    }
}

fn plural(n: usize, w: &str) -> String {
    format!("{n} {w}{}", if n == 1 { "" } else { "s" })
}

struct Edges {
    out: Vec<Edge>,
    seen: HashSet<String>,
}

impl Edges {
    fn add(&mut self, kind: EdgeKind, source: &str, target: &str, label: &str) {
        let id = format!("{}:{source}->{target}", kind.word());
        if !self.seen.insert(id.clone()) {
            return;
        }
        self.out.push(Edge {
            id,
            kind,
            source: source.into(),
            target: target.into(),
            label: label.into(),
            in_hub: false,
        });
    }
}

/// The agents' nodes, in the order they first show up.
#[derive(Default)]
struct Agents {
    list: Vec<AgentNode>,
}

impl Agents {
    fn node(&mut self, a: &AgentRef, now: i64) -> &AgentNode {
        let id = format!("agent:{}", a.key);
        let i = match self.list.iter().position(|n| n.id == id) {
            Some(i) => i,
            None => {
                let st = agent_status(&Who::from(a), None, now);
                self.list.push(AgentNode {
                    id,
                    label: display_name(a),
                    agent: a.clone(),
                    live: a.live,
                    person: a.status == "person",
                    group: hub_id(&a.key),
                    status: st.text,
                    tone: st.tone,
                });
                self.list.len() - 1
            }
        };
        &self.list[i]
    }
}

/// The board as a graph: a hub per agent that is here (the person too,
/// when they hold a card), an area for the cards nobody is on and one for
/// done cards; a node per card and per open blocker; edges parent ->
/// subtask, blocker -> card, agent -> card (holder for working, watcher for
/// watching) and agent -> agent for agents talking on a card. `details`
/// supplies each card's own blockers; without one a card with open
/// blockers gets one blocker node carrying its blocker question. Agents
/// that aren't here get no node: their faces stay on the card.
pub fn to_graph(cards: &[&CardSummary], details: &HashMap<String, CardDetail>) -> Graph {
    to_graph_at(cards, details, now_ms())
}

pub fn to_graph_at(cards: &[&CardSummary], details: &HashMap<String, CardDetail>, now: i64) -> Graph {
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges = Edges {
        out: Vec::new(),
        seen: HashSet::new(),
    };
    let mut agents = Agents::default();
    // Hub id -> open cards in it.
    let mut held: HashMap<String, usize> = HashMap::new();
    let ids: HashSet<&str> = cards.iter().map(|c| c.id.as_str()).collect();

    let mut unheld = 0;
    let mut done = 0;
    // (a, b, cards) per pair of agents talking, in the order first seen.
    let mut talk: Vec<(String, String, Vec<String>)> = Vec::new();
    for c in cards {
        let g = hub_of(c);
        let is_done = g == DONE_ID;
        if g == UNHELD_ID {
            unheld += 1;
        } else if is_done {
            done += 1;
        } else {
            *held.entry(g.clone()).or_default() += 1;
        }
        let faces: Vec<FaceRef> = c
            .working
            .iter()
            .map(|a| face_of(a, false))
            .chain(c.watching.iter().map(|a| face_of(a, true)))
            .collect();
        let parent = c
            .parent
            .as_ref()
            .filter(|p| ids.contains(p.id.as_str()))
            .map(|p| p.id.clone());
        nodes.push(Node::Card(Box::new(CardNode {
            id: c.id.clone(),
            label: c.title.clone(),
            short: c.short.clone(),
            state: c.state.clone(),
            tone: card_tone(c),
            done_steps: c.progress.done,
            total_steps: c.progress.total,
            open_blockers: c.open_blockers,
            waiting: c.waiting,
            parent: parent.clone(),
            subtasks: c.subtasks,
            group: g.clone(),
            repo: c.repo_key.clone(),
            board: c.board.clone(),
            place: where_of(c),
            needs: needs_you(c),
            // The grid's rule for its Nobody on it badge.
            nobody: !is_done && !open_ask(c.ask.as_ref()) && nobody_on(c) && c.starting.is_empty(),
            stale: !is_done && c.stale,
            done: is_done,
            faces: faces.clone(),
            card: (*c).clone(),
        })));
        if let Some(p) = &parent {
            edges.add(EdgeKind::Subtask, p, &c.id, "");
        }
        // A done card is tucked away: no blockers, no agent lines.
        if is_done {
            continue;
        }

        let open: Vec<_> = details
            .get(&c.id)
            .map(|d| {
                d.blockers
                    .iter()
                    .filter(|b| {
                        b.resolved.is_none()
                            // A wait on other work isn't a question (card #481).
                            && b.waits_on.is_none()
                            && b.state != blocker::RESOLVED
                            && b.state != blocker::CLOSED
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !open.is_empty() {
            for b in open {
                let id = format!("blocker:{}:{}", c.id, b.id);
                nodes.push(Node::Blocker(BlockerNode {
                    id: id.clone(),
                    label: b.question.clone(),
                    card: c.id.clone(),
                    state: b.state.clone(),
                    group: g.clone(),
                }));
                edges.add(EdgeKind::Blocker, &id, &c.id, "");
            }
        } else if c.open_blockers > 0 || c.waiting > 0 {
            let id = format!("blocker:{}:open", c.id);
            let label = if c.blocker_question.is_empty() {
                format!(
                    "{} answer{} waiting on an agent",
                    c.waiting,
                    if c.waiting == 1 { "" } else { "s" }
                )
            } else {
                c.blocker_question.clone()
            };
            nodes.push(Node::Blocker(BlockerNode {
                id: id.clone(),
                label,
                card: c.id.clone(),
                state: if c.open_blockers > 0 { blocker::OPEN } else { blocker::ANSWERED }.into(),
                group: g.clone(),
            }));
            edges.add(EdgeKind::Blocker, &id, &c.id, "");
        }

        for f in &faces {
            if !here(&f.agent) {
                continue;
            }
            // You watching every card would be a line to each: your face
            // on the card says it.
            if f.person && f.watching {
                continue;
            }
            let a = agents.node(&f.agent, now).id.clone();
            let kind = if f.watching { EdgeKind::Watcher } else { EdgeKind::Holder };
            edges.add(kind, &a, &c.id, "");
        }

        // Agents writing to each other on this card: a line per pair.
        let talking: Vec<&AgentRef> = c.talking.iter().filter(|a| here(a) && a.status != "person").collect();
        for i in 0..talking.len() {
            for j in i + 1..talking.len() {
                let (mut a, mut b) = (talking[i].key.clone(), talking[j].key.clone());
                if b < a {
                    std::mem::swap(&mut a, &mut b);
                }
                if a == b {
                    continue;
                }
                let r = format!("#{}", c.short.trim_start_matches('#'));
                match talk.iter_mut().find(|t| t.0 == a && t.1 == b) {
                    Some(t) => t.2.push(r),
                    None => talk.push((a, b, vec![r])),
                }
            }
        }
        for a in talking {
            agents.node(a, now);
        }
    }
    for (a, b, cs) in &talk {
        edges.add(EdgeKind::Talk, &format!("agent:{a}"), &format!("agent:{b}"), &cs.join(", "));
    }

    // An agent that is a card's first holder here (you too) has a hub; the
    // rest that are here (watching, talking, holding beside another) share
    // one strip, their lines going to the cards.
    let mut hubs: Vec<Node> = Vec::new();
    let mut others = 0;
    for a in agents.list.iter_mut() {
        let n = held.get(&a.group).copied().unwrap_or(0);
        if n == 0 {
            a.group = OTHERS_ID.into();
            others += 1;
            continue;
        }
        let head = if a.person { "You".to_string() } else { a.status.clone() };
        let label = [head, plural(n, "card")]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        hubs.push(Node::Group(GroupNode {
            id: a.group.clone(),
            label,
            by: By::Agent,
            key: a.agent.key.clone(),
        }));
    }
    let group_of: HashMap<&str, &str> = nodes
        .iter()
        .filter_map(|n| match n {
            Node::Card(c) => Some((c.id.as_str(), c.group.as_str())),
            _ => None,
        })
        .collect();
    let mut edges = edges.out;
    for e in edges.iter_mut() {
        if e.kind == EdgeKind::Holder {
            let agent_group = agents.list.iter().find(|a| a.id == e.source).map(|a| a.group.as_str());
            if agent_group.is_some() && group_of.get(e.target.as_str()).copied() == agent_group {
                e.in_hub = true;
            }
        }
    }
    let mut areas: Vec<Node> = Vec::new();
    if unheld > 0 {
        areas.push(Node::Group(GroupNode {
            id: UNHELD_ID.into(),
            label: format!("Nobody on these · {unheld}"),
            by: By::Unheld,
            key: String::new(),
        }));
    }
    if others > 0 {
        areas.push(Node::Group(GroupNode {
            id: OTHERS_ID.into(),
            label: format!("Also here · {others}"),
            by: By::Others,
            key: String::new(),
        }));
    }
    if done > 0 {
        areas.push(Node::Group(GroupNode {
            id: DONE_ID.into(),
            label: format!("Done · {done}"),
            by: By::Done,
            key: String::new(),
        }));
    }
    let mut all = hubs;
    all.extend(areas);
    all.extend(nodes);
    all.extend(agents.list.into_iter().map(|a| Node::Agent(Box::new(a))));
    Graph { nodes: all, edges }
}

/// The cards whose live holder changed between two graphs: a card that
/// came to an agent, or moved from one to another.
pub fn hand_offs(prev: &HashMap<String, String>, graph: &Graph) -> Vec<String> {
    graph
        .cards()
        .filter(|n| !n.done)
        .filter(|n| {
            prev.get(&n.id)
                .is_some_and(|was| *was != n.group && n.group.starts_with("group:agent:"))
        })
        .map(|n| n.id.clone())
        .collect()
}

/// Each card's area, for [`hand_offs`] next time.
pub fn holders(graph: &Graph) -> HashMap<String, String> {
    graph.cards().map(|n| (n.id.clone(), n.group.clone())).collect()
}

// ---- default layout ---------------------------------------------------------

pub const CARD: Size = Size { w: 280.0, h: 132.0 };
pub const BLOCKER: Size = Size { w: 240.0, h: 72.0 };
pub const AGENT: Size = Size { w: 180.0, h: 40.0 };

pub fn node_size(k: Kind) -> Size {
    match k {
        Kind::Card => CARD,
        Kind::Blocker => BLOCKER,
        Kind::Agent => AGENT,
    }
}

/// Between cards in a grid.
const GAP: f32 = 24.0;
/// Inside an area's rect.
const PAD: f32 = 24.0;
/// An area's label strip.
pub const HEAD: f32 = 32.0;
/// Between areas.
const AREA_GAP: f32 = 72.0;
/// Areas wrap into rows about this many times wider than tall, like the
/// window the canvas sits in, so the map fills it instead of a column.
const ASPECT: f32 = 2.2;
/// A small board stays on one row.
const MIN_ROW: f32 = 1600.0;
/// A blocker under its card, set in a little.
const BLOCKER_INSET: f32 = 20.0;
/// Between a card and its blockers, and between blockers.
const BLOCKER_GAP: f32 = 8.0;
/// An agent's grid is at most this wide.
const HUB_COLS: usize = 4;
/// The unheld and done grids, about twice as wide as tall.
const AREA_COLS: usize = 10;
/// Agents in the others strip.
const PILL_COLS: usize = 6;
const PILL_GAP: f32 = 12.0;

/// How wide a grid of n cards is: about square, so a hub of 16 cards is 4
/// by 4, never 16 tall.
pub fn cols_for(n: usize, max: usize, wide: usize) -> usize {
    ((n * wide) as f64).sqrt().ceil().min(max as f64).max(1.0) as usize
}

/// Where a card's i-th blocker sits: under the card, so the grid's next
/// column stays clear.
fn blocker_spot(card: Point, i: usize) -> Point {
    pt(
        card.x + BLOCKER_INSET,
        card.y + CARD.h + BLOCKER_GAP + i as f32 * (BLOCKER.h + BLOCKER_GAP),
    )
}

/// An area's cards ordered so each subtask follows its parent, keeping the
/// input order otherwise.
fn depth_first<'a>(cards: &[&'a CardNode]) -> Vec<&'a CardNode> {
    let in_set: HashSet<&str> = cards.iter().map(|c| c.id.as_str()).collect();
    let kids_of = |id: &str| -> Vec<&'a CardNode> {
        cards
            .iter()
            .copied()
            .filter(|c| c.parent.as_deref() == Some(id))
            .collect()
    };
    let mut out = Vec::new();
    fn visit<'a>(c: &'a CardNode, kids_of: &dyn Fn(&str) -> Vec<&'a CardNode>, out: &mut Vec<&'a CardNode>, depth: usize) {
        out.push(c);
        // A parent loop can't recurse forever.
        if depth > 64 {
            return;
        }
        for k in kids_of(&c.id) {
            visit(k, kids_of, out, depth + 1);
        }
    }
    for c in cards {
        if c.parent.as_deref().is_none_or(|p| !in_set.contains(p)) {
            visit(c, &kids_of, &mut out, 0);
        }
    }
    out
}

struct Placed {
    w: f32,
    h: f32,
    at: Vec<(String, Point, Size)>,
}

/// One area laid out from (0, 0): its label strip, its agents (a hub's
/// one, or the others strip's several, in rows), then its cards in a grid,
/// each card's blockers under it.
fn place_area(agents: &[&AgentNode], cards: &[&CardNode], blockers_of: &dyn Fn(&str) -> Vec<String>, max_cols: usize) -> Placed {
    let mut at = Vec::new();
    let mut y = HEAD + PAD;
    let mut w: f32 = 0.0;
    let pill_cols = PILL_COLS.min(agents.len());
    for chunk in agents.chunks(pill_cols.max(1)) {
        for (i, a) in chunk.iter().enumerate() {
            at.push((a.id.clone(), pt(PAD + i as f32 * (AGENT.w + PILL_GAP), y), AGENT));
        }
        w = w.max(pill_cols as f32 * AGENT.w + (pill_cols as f32 - 1.0) * PILL_GAP);
        y += AGENT.h + PILL_GAP;
    }
    if !agents.is_empty() {
        y += GAP - PILL_GAP;
    }
    // An agent's hub is about square; the big areas lie wide, like the window.
    let cols = cols_for(cards.len(), max_cols, if max_cols == HUB_COLS { 1 } else { 2 });
    for row in cards.chunks(cols) {
        let mut row_h: f32 = 0.0;
        for (i, c) in row.iter().enumerate() {
            let p = pt(PAD + i as f32 * (CARD.w + GAP), y);
            at.push((c.id.clone(), p, CARD));
            let bs = blockers_of(&c.id);
            for (j, b) in bs.iter().enumerate() {
                at.push((b.clone(), blocker_spot(p, j), BLOCKER));
            }
            row_h = row_h.max(CARD.h + bs.len() as f32 * (BLOCKER.h + BLOCKER_GAP));
        }
        w = w.max(row.len() as f32 * CARD.w + (row.len() as f32 - 1.0) * GAP);
        y += row_h + GAP;
    }
    if !cards.is_empty() || !agents.is_empty() {
        y -= GAP;
    }
    Placed {
        w: w + 2.0 * PAD,
        h: y + PAD,
        at,
    }
}

/// The starting layout, no library (card #587): a hub per agent (you
/// first, then biggest, then by name), then the cards nobody is on, packed
/// in rows that wrap; the done cards start a row of their own at the end.
/// Deterministic, so every device starts from the same picture.
pub fn auto_layout(g: &Graph) -> Layout {
    let mut l = Layout::default();
    let cards: Vec<&CardNode> = g.cards().collect();
    let blockers_of = |card: &str| -> Vec<String> {
        g.nodes
            .iter()
            .filter_map(|n| match n {
                Node::Blocker(b) if b.card == card => Some(b.id.clone()),
                _ => None,
            })
            .collect()
    };
    let agents_of = |group: &str| -> Vec<&AgentNode> {
        g.nodes
            .iter()
            .filter_map(|n| match n {
                Node::Agent(a) if a.group == group => Some(&**a),
                _ => None,
            })
            .collect()
    };
    let members = |group: &str| -> Vec<&CardNode> { cards.iter().copied().filter(|c| c.group == group).collect() };
    let count = |group: &str| cards.iter().filter(|c| c.group == group).count();
    let agent_of = |group: &str| agents_of(group).into_iter().next();

    let areas: Vec<&GroupNode> = g
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::Group(a) => Some(a),
            _ => None,
        })
        .collect();
    let mut hubs: Vec<&GroupNode> = areas.iter().copied().filter(|a| a.by == By::Agent).collect();
    hubs.sort_by(|a, b| {
        let pa = agent_of(&a.id).is_some_and(|x| x.person);
        let pb = agent_of(&b.id).is_some_and(|x| x.person);
        let la = agent_of(&a.id).map(|x| x.label.clone()).unwrap_or_default();
        let lb = agent_of(&b.id).map(|x| x.label.clone()).unwrap_or_default();
        // You first, then the bigger hub, then by name.
        pb.cmp(&pa)
            .then(count(&b.id).cmp(&count(&a.id)))
            .then_with(|| la.to_lowercase().cmp(&lb.to_lowercase()))
            .then_with(|| la.cmp(&lb))
            .then_with(|| a.key.cmp(&b.key))
    });
    let mut order = hubs;
    for by in [By::Unheld, By::Others, By::Done] {
        order.extend(areas.iter().copied().filter(|a| a.by == by));
    }

    let placed: Vec<(&GroupNode, Placed)> = order
        .iter()
        .map(|a| {
            let max = if a.by == By::Agent { HUB_COLS } else { AREA_COLS };
            (*a, place_area(&agents_of(&a.id), &depth_first(&members(&a.id)), &blockers_of, max))
        })
        .collect();
    let room: f32 = placed
        .iter()
        .filter(|(a, _)| a.by != By::Done)
        .map(|(_, p)| (p.w + AREA_GAP) * (p.h + AREA_GAP))
        .sum();
    let row_width = placed
        .iter()
        .map(|(_, p)| p.w)
        .fold(MIN_ROW.max((room * ASPECT).sqrt()), f32::max);

    let (mut x, mut y, mut row_h) = (0.0f32, 0.0f32, 0.0f32);
    for (a, p) in &placed {
        // Done starts its own row.
        let fresh = a.by == By::Done;
        if x > 0.0 && (fresh || x + p.w > row_width) {
            x = 0.0;
            y += row_h + AREA_GAP;
            row_h = 0.0;
        }
        l.groups.insert(
            a.id.clone(),
            Rect {
                x,
                y,
                w: p.w,
                h: p.h,
            },
        );
        for (id, q, s) in &p.at {
            l.pos.insert(id.clone(), pt(x + q.x, y + q.y));
            l.size.insert(id.clone(), *s);
        }
        x += p.w + AREA_GAP;
        row_h = row_h.max(p.h);
    }
    l
}

// ---- placed cards -----------------------------------------------------------

/// The layout with every placed card at its kept spot: its blockers move
/// with it (under it, as auto_layout puts them), and each area's rect grows
/// or shrinks to hold its members where they are now. Areas move as cards
/// change hands (card #587), so a kept spot counts only while it lies
/// inside the card's own area as laid out now: a card arranged within its
/// hub stays put, and one kept from an older layout, under another holder
/// or out in the open goes back to its area's grid rather than landing on
/// another hub's cards.
pub fn with_placed(g: &Graph, l: &Layout) -> Layout {
    let mut pos = l.pos.clone();
    for n in g.cards() {
        let Some(p) = &n.card.pos else { continue };
        let p = pt(p.x as f32, p.y as f32);
        if !within(p, l.groups.get(&n.group)) {
            continue;
        }
        pos.insert(n.id.clone(), p);
    }
    let pos = followers(g, &pos);
    let groups = group_rects(g, &pos, l, Some(&l.groups));
    Layout {
        pos,
        size: l.size.clone(),
        groups,
    }
}

/// Each blocker under its card's current spot.
pub fn followers(g: &Graph, pos: &HashMap<String, Point>) -> HashMap<String, Point> {
    let mut out = pos.clone();
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for n in &g.nodes {
        let Node::Blocker(b) = n else { continue };
        let Some(c) = out.get(&b.card).copied() else { continue };
        let i = seen.entry(b.card.as_str()).or_default();
        out.insert(b.id.clone(), blocker_spot(c, *i));
        *i += 1;
    }
    out
}

/// A card at p lies wholly inside r, give or take a little.
fn within(p: Point, r: Option<&Rect>) -> bool {
    let Some(r) = r else { return false };
    let slack = GAP;
    p.x >= r.x - slack && p.y >= r.y - slack && p.x + CARD.w <= r.x + r.w + slack && p.y + CARD.h <= r.y + r.h + slack
}

/// How far outside its area's laid-out rect a kept card may sit and still
/// count in it: a card dragged within its hub grows the box; one dragged
/// far away (or kept from an older layout) floats on its own rather than
/// stretching the box across the canvas.
const REACH: f32 = 160.0;

fn near(p: Point, r: Option<&Rect>) -> bool {
    let Some(r) = r else { return true };
    p.x >= r.x - REACH - CARD.w && p.x <= r.x + r.w + REACH && p.y >= r.y - REACH - CARD.h && p.y <= r.y + r.h + REACH
}

/// Each area's rect around its agent, cards and blockers as they sit now;
/// an area with none keeps the rect it had. `anchor`, when given, is where
/// each area was laid out: a card far from it doesn't count (see REACH),
/// nor do its blockers.
pub fn group_rects(
    g: &Graph,
    pos: &HashMap<String, Point>,
    l: &Layout,
    anchor: Option<&HashMap<String, Rect>>,
) -> HashMap<String, Rect> {
    let mut out = l.groups.clone();
    let mut away: HashSet<&str> = HashSet::new();
    if let Some(anchor) = anchor {
        for n in g.cards() {
            if let Some(p) = pos.get(&n.id)
                && !near(*p, anchor.get(&n.group))
            {
                away.insert(n.id.as_str());
            }
        }
    }
    let mut boxes: HashMap<&str, [f32; 4]> = HashMap::new();
    for n in &g.nodes {
        let Some(kind) = n.kind() else { continue };
        let owner = match n {
            Node::Blocker(b) => b.card.as_str(),
            _ => n.id(),
        };
        if away.contains(owner) {
            continue;
        }
        let Some(p) = pos.get(n.id()) else { continue };
        let s = node_size(kind);
        let b = boxes
            .entry(n.group())
            .or_insert([f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY]);
        b[0] = b[0].min(p.x);
        b[1] = b[1].min(p.y);
        b[2] = b[2].max(p.x + s.w);
        b[3] = b[3].max(p.y + s.h);
    }
    for (id, b) in boxes {
        out.insert(
            id.to_string(),
            Rect {
                x: b[0] - PAD,
                y: b[1] - HEAD - PAD,
                w: b[2] - b[0] + 2.0 * PAD,
                h: b[3] - b[1] + HEAD + 2.0 * PAD,
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::cards::state;
    use irori_window::cards::{CanvasPos, CardHolder, CardProgress, Holding};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn agent(key: &str, status: &str, live: bool) -> AgentRef {
        AgentRef {
            key: key.into(),
            name: key.into(),
            status: status.into(),
            live,
            ..AgentRef::default()
        }
    }
    fn busy(key: &str) -> AgentRef {
        agent(key, "busy", true)
    }
    fn idle(key: &str) -> AgentRef {
        agent(key, "idle", true)
    }
    fn gone(key: &str) -> AgentRef {
        agent(key, "gone", false)
    }

    static N: AtomicUsize = AtomicUsize::new(0);

    fn card(edit: impl FnOnce(&mut CardSummary)) -> CardSummary {
        let n = N.fetch_add(1, Ordering::SeqCst) + 1;
        let mut c = CardSummary {
            id: format!("c{n}"),
            short: format!("{n}"),
            title: format!("card {n}"),
            state: state::WORKING.into(),
            repo_key: "remote:github.com/org/app".into(),
            progress: CardProgress { done: 0, total: 0 },
            holder: CardHolder {
                state: Holding::Active,
                ..CardHolder::default()
            },
            ..CardSummary::default()
        };
        edit(&mut c);
        c
    }

    fn graph(cards: &[CardSummary]) -> Graph {
        let refs: Vec<&CardSummary> = cards.iter().collect();
        to_graph(&refs, &HashMap::new())
    }

    fn inside(r: &Rect, p: Point, s: Size) -> bool {
        p.x >= r.x && p.y >= r.y && p.x + s.w <= r.x + r.w && p.y + s.h <= r.y + r.h
    }

    fn overlap(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    fn group_of(g: &Graph, id: &str) -> String {
        g.node(id).unwrap().group().to_string()
    }

    #[test]
    fn repo_labels() {
        assert_eq!(repo_label("remote:github.com/org/repo"), "org/repo");
        assert_eq!(repo_label("local:/Users/me/src/irori"), "irori");
        assert_eq!(repo_label("C:\\src\\irori\\"), "irori");
        assert_eq!(repo_label(""), "");
    }

    #[test]
    fn each_held_card_sits_in_its_agents_hub_the_rest_in_their_own_areas() {
        let cards = vec![
            card(|c| c.working = vec![busy("a")]),
            card(|c| c.working = vec![busy("a")]),
            card(|c| c.working = vec![idle("b")]),
            card(|c| {
                c.state = state::SUBMITTED.into();
                c.holder.state = Holding::Nobody;
            }),
            card(|c| {
                c.working = vec![gone("c")];
                c.holder.state = Holding::Gone;
            }),
            card(|c| {
                c.state = state::COMPLETED.into();
                c.working = vec![busy("a")];
            }),
        ];
        let g = graph(&cards);
        let l = auto_layout(&g);
        assert_eq!(group_of(&g, &cards[0].id), "group:agent:a");
        assert_eq!(group_of(&g, &cards[1].id), "group:agent:a");
        assert_eq!(group_of(&g, &cards[2].id), "group:agent:b");
        assert_eq!(group_of(&g, &cards[3].id), "group:unheld");
        // A gone holder doesn't keep a card: nobody is on it.
        assert_eq!(group_of(&g, &cards[4].id), "group:unheld");
        assert_eq!(group_of(&g, &cards[5].id), "group:done");
        // A gone agent gets no node; a done card no agent line.
        assert!(g.node("agent:c").is_none());
        assert!(!g.edges.iter().any(|e| e.target == cards[5].id));

        for n in &g.nodes {
            let Some(k) = n.kind() else { continue };
            let r = &l.groups[n.group()];
            assert!(inside(r, l.pos[n.id()], node_size(k)), "{} outside {}", n.id(), n.group());
        }
        // The agent's pill heads its own hub.
        assert!(inside(&l.groups["group:agent:a"], l.pos["agent:a"], AGENT));
    }

    #[test]
    fn no_tall_column_one_agent_with_16_cards_is_a_4_by_4_grid() {
        let cards: Vec<_> = (0..16).map(|_| card(|c| c.working = vec![busy("solo")])).collect();
        let l = auto_layout(&graph(&cards));
        let hub = l.groups["group:agent:solo"];
        assert_eq!(cols_for(16, 4, 1), 4);
        assert!(hub.h < 5.0 * (CARD.h + 24.0) + 2.0 * AGENT.h);
        assert!(hub.w > hub.h);
    }

    #[test]
    fn areas_dont_overlap_wrap_into_rows_and_done_starts_its_own_row() {
        let mut cards: Vec<_> = (0..30)
            .map(|i| card(|c| c.working = vec![busy(&format!("agent{}", i % 10))]))
            .collect();
        cards.extend((0..12).map(|_| card(|c| c.state = state::SUBMITTED.into())));
        cards.extend((0..5).map(|_| card(|c| c.state = state::COMPLETED.into())));
        let l = auto_layout(&graph(&cards));
        let rects: Vec<&Rect> = l.groups.values().collect();
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                assert!(!overlap(rects[i], rects[j]));
            }
        }
        let right = rects.iter().map(|r| r.x + r.w).fold(f32::MIN, f32::max);
        let bottom = rects.iter().map(|r| r.y + r.h).fold(f32::MIN, f32::max);
        // Wider than tall: a map, not a tower.
        assert!(right > bottom);
        let done = l.groups["group:done"];
        assert_eq!(done.x, 0.0);
        for (id, r) in &l.groups {
            if id != "group:done" {
                assert!(r.y + r.h <= done.y);
            }
        }
    }

    #[test]
    fn the_same_cards_give_the_same_layout() {
        let cards = vec![
            card(|c| c.working = vec![busy("z")]),
            card(|c| c.working = vec![busy("y")]),
            card(|c| {
                c.working = vec![busy("y")];
                c.open_blockers = 1;
                c.blocker_question = "Ship?".into();
            }),
        ];
        assert_eq!(auto_layout(&graph(&cards)), auto_layout(&graph(&cards)));
        // Bigger hub first.
        let l = auto_layout(&graph(&cards));
        assert!(l.groups["group:agent:y"].x < l.groups["group:agent:z"].x);
    }

    #[test]
    fn a_blocker_sits_under_its_card_inside_the_hub() {
        let c = card(|c| {
            c.working = vec![busy("a")];
            c.open_blockers = 1;
            c.blocker_question = "Ship?".into();
        });
        let l = auto_layout(&graph(std::slice::from_ref(&c)));
        let b = l.pos[&format!("blocker:{}:open", c.id)];
        assert!(b.y >= l.pos[&c.id].y + CARD.h);
        assert!(inside(&l.groups["group:agent:a"], b, BLOCKER));
    }

    #[test]
    fn a_kept_spot_counts_only_inside_the_cards_own_area() {
        let a1 = card(|c| c.working = vec![busy("a")]);
        let a2 = card(|c| c.working = vec![busy("a")]);
        let g0 = graph(&[a1.clone(), a2.clone()]);
        let p0 = auto_layout(&g0).pos[&a1.id];
        let spot = CanvasPos {
            x: p0.x as i64 + 10,
            y: p0.y as i64 + 6,
        };
        let spot_pt = pt(spot.x as f32, spot.y as f32);
        // Arranged within its hub: kept.
        let mut in_hub = a2.clone();
        in_hub.pos = Some(spot);
        let g = graph(&[a1.clone(), in_hub.clone()]);
        assert_eq!(with_placed(&g, &auto_layout(&g)).pos[&in_hub.id], spot_pt);
        // Far off (an older layout, or dropped in the open): back in the grid.
        let mut far = a2.clone();
        far.pos = Some(CanvasPos { x: 9000, y: 9000 });
        let g = graph(&[a1.clone(), far.clone()]);
        let auto = auto_layout(&g);
        assert_eq!(with_placed(&g, &auto).pos[&far.id], auto.pos[&far.id]);
        // While dragged far, it floats rather than stretching the box.
        let mut dragged = auto.pos.clone();
        dragged.insert(far.id.clone(), pt(9000.0, 9000.0));
        assert!(group_rects(&g, &dragged, &auto, Some(&auto.groups))["group:agent:a"].w < 2000.0);
        assert!(group_rects(&g, &dragged, &auto, None)["group:agent:a"].w > 9000.0);
        // Handed to b: its spot in a's hub no longer counts.
        let mut handed = a2.clone();
        handed.working = vec![busy("b")];
        handed.pos = Some(spot);
        let g = graph(&[a1, handed.clone()]);
        let l = auto_layout(&g);
        assert_eq!(with_placed(&g, &l).pos[&handed.id], l.pos[&handed.id]);
    }

    #[test]
    fn a_second_holder_sits_in_the_shared_strip_with_a_line_to_the_card() {
        let c = card(|c| c.working = vec![busy("a"), busy("b")]);
        let g = graph(&[c]);
        assert_eq!(group_of(&g, "agent:b"), "group:others");
        assert!(
            g.edges
                .iter()
                .any(|e| e.kind == EdgeKind::Holder && e.source == "agent:b" && !e.in_hub)
        );
    }

    #[test]
    fn agents_talking_on_a_card_get_one_line_per_pair_named_by_the_cards() {
        let (a, b) = (busy("a"), busy("b"));
        let c1 = card(|c| {
            c.working = vec![a.clone()];
            c.talking = vec![a.clone(), b.clone()];
        });
        let c2 = card(|c| {
            c.working = vec![b.clone()];
            c.talking = vec![b.clone(), a.clone()];
        });
        let g = graph(&[c1.clone(), c2.clone()]);
        let talk: Vec<&Edge> = g.edges.iter().filter(|e| e.kind == EdgeKind::Talk).collect();
        assert_eq!(talk.len(), 1);
        assert_eq!(talk[0].source, "agent:a");
        assert_eq!(talk[0].target, "agent:b");
        assert_eq!(talk[0].label, format!("#{}, #{}", c1.short, c2.short));
    }

    #[test]
    fn agents_with_no_card_of_their_own_share_one_strip_in_hub_holder_lines_are_marked() {
        let c = card(|c| {
            c.working = vec![busy("a")];
            c.watching = vec![busy("w1"), idle("w2")];
        });
        let g = graph(&[c]);
        let l = auto_layout(&g);
        let areas: Vec<&str> = g
            .nodes
            .iter()
            .filter(|n| matches!(n, Node::Group(_)))
            .map(|n| n.id())
            .collect();
        assert_eq!(areas, vec!["group:agent:a", "group:others"]);
        for k in ["agent:w1", "agent:w2"] {
            assert!(inside(&l.groups["group:others"], l.pos[k], AGENT));
        }
        assert!(g.edges.iter().find(|e| e.kind == EdgeKind::Holder).unwrap().in_hub);
    }

    #[test]
    fn agent_status_tone_comes_from_agent_status() {
        let g = graph(&[card(|c| c.working = vec![busy("a")]), card(|c| c.working = vec![idle("b")])]);
        let tone = |id: &str| match g.node(id) {
            Some(Node::Agent(a)) => a.tone,
            _ => panic!("{id}"),
        };
        assert_eq!(tone("agent:a"), StatusTone::Working);
        assert_eq!(tone("agent:b"), StatusTone::Idle);
    }

    #[test]
    fn a_card_that_changes_hands_is_a_hand_off() {
        let c = card(|c| c.working = vec![busy("a")]);
        let prev = holders(&graph(std::slice::from_ref(&c)));
        assert!(hand_offs(&prev, &graph(std::slice::from_ref(&c))).is_empty());
        let mut moved = c.clone();
        moved.working = vec![busy("b")];
        assert_eq!(hand_offs(&prev, &graph(&[moved])), vec![c.id]);
    }

    #[test]
    fn group_labels_name_the_agent_state_and_count() {
        let mut me = agent("me", "person", false);
        me.name = "You".into();
        let g = graph(&[
            card(|c| c.working = vec![busy("a")]),
            card(|c| c.working = vec![busy("a")]),
            card(|c| c.working = vec![me.clone()]),
            card(|c| c.state = state::SUBMITTED.into()),
        ]);
        let label = |id: &str| match g.node(id) {
            Some(Node::Group(n)) => n.label.clone(),
            _ => panic!("{id}"),
        };
        assert_eq!(label("group:agent:a"), "working · 2 cards");
        assert_eq!(label("group:agent:me"), "You · 1 card");
        assert_eq!(label(UNHELD_ID), "Nobody on these · 1");
        // You come first.
        let l = auto_layout(&g);
        assert!(l.groups["group:agent:me"].x < l.groups["group:agent:a"].x);
    }

    #[test]
    fn subtasks_follow_their_parent_with_a_line() {
        let p = card(|c| c.state = state::SUBMITTED.into());
        let other = card(|c| c.state = state::SUBMITTED.into());
        let kid = card(|c| {
            c.state = state::SUBMITTED.into();
            c.parent = Some(irori_window::cards::CardRef {
                id: p.id.clone(),
                ..Default::default()
            });
        });
        let g = graph(&[p.clone(), other.clone(), kid.clone()]);
        assert!(g.edges.iter().any(|e| e.kind == EdgeKind::Subtask && e.source == p.id && e.target == kid.id));
        let l = auto_layout(&g);
        // The kid comes right after its parent in the grid.
        assert!(l.pos[&kid.id].x < l.pos[&other.id].x);
    }
}
