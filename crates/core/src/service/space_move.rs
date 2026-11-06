//! Moving a one-person space into Personal (card #640,
//! docs/design/space-to-personal.md).
//!
//! The space's own board becomes a Personal board with the same name and
//! tag, and every card row of its log is written again into the private
//! log, in the log's order, with the same task ids, numbers and times. The
//! copy is folded and compared card by card with the space's fold, and only
//! then is the space forgotten on every device (`board.forgot`).
//!
//! No new row kind: the rows are the private log's own, which every Irori
//! from #558 on folds. Each copied row carries one extra field, `moved`
//! (`<spaceId>/<device>/<seq>` of the row it copies), which older decoders
//! drop, as they drop the space's `hop`. It lets a move that stopped part
//! way go on from where it stopped without writing a row twice.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use coord::ErrorKind as CK;
use serde::Serialize;
use serde_json::{Map, Value};
use store::{Event, Filter, MAX_LIST_LIMIT, Store, kinds};

use super::Services;
use super::boards::{BoardLog, Error, Result};
use super::spaces::board_tag_ids;

/// The extra field each copied row carries: the row it copies.
pub const MOVED_FIELD: &str = "moved";

/// How long the move waits for the private fold to take in the copy.
const FOLD_WAIT: Duration = Duration::from_secs(60);

/// Space kinds that are the private log's under another name
/// (`Fold::ask_kind`), with `board.ask.moved`, which `take_kind` leaves.
const RENAMED: &[(&str, &str)] = &[
    (kinds::BOARD_ASK_EDITED, kinds::TASK_EDITED),
    (kinds::BOARD_ASK_REQUESTED, kinds::TAKE_REQUESTED),
    (kinds::BOARD_ASK_STATE, kinds::TAKE_STATE),
    (kinds::BOARD_ASK_MOVED, kinds::ASK_MOVED),
];

/// Card kinds that are hand-out state or the space's own, never the card.
const LEFT_OUT: &[&str] = &[kinds::TASK_OFFERED, kinds::TASK_OWNER];

/// What a move did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceMove {
    pub space_id: String,
    /// The Personal board the cards are on now.
    pub board_id: String,
    pub name: String,
    pub tag: String,
    pub cards: usize,
    /// Rows written now, and rows an earlier run had already written.
    pub rows_copied: usize,
    pub rows_already: usize,
    /// Rows of the space left behind, by kind: its members, devices, agent
    /// sessions, hand-out state and the PR states older than the newest.
    pub rows_left: BTreeMap<String, usize>,
    /// Files the cards attach, and those no device could give.
    pub files: usize,
    pub files_missing: Vec<String>,
    /// Open cards the person made: Irori hands these out in Personal.
    pub open_by_person: Vec<String>,
    /// Whether the space was forgotten (false for a dry run).
    pub forgotten: bool,
}

/// How to run a move.
#[derive(Debug, Clone, Default)]
pub struct MoveOpts {
    /// Copy and compare, but keep the space.
    pub keep_space: bool,
    /// Go on when a file can't be fetched from any device.
    pub allow_missing_files: bool,
}

/// The per-card facts the move compares between the space and the copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardFacts {
    pub num: String,
    pub title: String,
    pub state: String,
    pub comments: i64,
    pub steps: Vec<(String, bool)>,
    pub blockers: usize,
    pub open_blockers: usize,
    pub subtasks: usize,
    pub parent: String,
    pub thread: bool,
}

impl CardFacts {
    pub fn of(t: &coord::Task) -> CardFacts {
        CardFacts {
            num: t.short.clone(),
            title: t.title.clone(),
            state: t.state.clone(),
            comments: t.comments,
            steps: t.steps.iter().map(|s| (s.text.clone(), s.done)).collect(),
            blockers: t.blockers.len(),
            open_blockers: t.blockers.iter().filter(|b| b.open()).count(),
            subtasks: t.subtasks.len(),
            parent: t.parent.clone(),
            thread: t.ask.as_ref().is_some_and(|a| a.thread),
        }
    }
}

impl Services {
    /// Moves space `space_id` into Personal (card #640): see the module
    /// doc. Refused for a space anyone else is in, a hidden one, or one
    /// with boards of its own inside.
    pub(crate) fn move_space_to_personal(
        self: &Arc<Self>,
        space_id: &str,
        o: &MoveOpts,
    ) -> Result<SpaceMove> {
        let b = self.board(space_id)?;
        if self.is_hidden_space(space_id) {
            return Err(Error::coord(
                CK::NotAllowed,
                "a conversation's space can't move into Personal",
            ));
        }
        let me = self.person_id();
        let m = b.verifier.membership();
        if m.members
            .iter()
            .any(|p| p.person_id != me && m.active(&p.person_id))
        {
            return Err(Error::coord(
                CK::NotAllowed,
                "only a space nobody else is in can move into Personal",
            ));
        }
        let boards = self.space_boards(space_id)?;
        if boards.len() > 1 {
            return Err(Error::coord(
                CK::NotAllowed,
                format_args!(
                    "this space has {} boards; only a space with one board moves into Personal",
                    boards.len()
                ),
            ));
        }
        let (name, tag) = boards
            .first()
            .map(|x| (x.name.clone(), x.tag.clone()))
            .unwrap_or_else(|| (m.name.clone(), m.tag.clone()));

        let rows = all_rows(&b.store)?;
        let done = self.moved_rows(space_id)?;
        let resume = done.board.clone();
        self.check_move_tag(space_id, &tag, resume.as_deref())?;

        // Files first: once the copy is written, a refusal here would
        // leave both behind.
        let files = file_refs(&rows);
        let files_missing = self.hold_files(&files);
        if !files_missing.is_empty() && !o.allow_missing_files {
            return Err(Error::coord(
                CK::BadState,
                format_args!(
                    "{} file(s) the cards attach are on no device that answered: {}",
                    files_missing.len(),
                    files_missing.join(", ")
                ),
            ));
        }

        let mut out = SpaceMove {
            space_id: space_id.into(),
            name: name.clone(),
            tag: tag.clone(),
            files: files.len(),
            files_missing,
            ..SpaceMove::default()
        };
        self.note_space_agent_names(&b);

        out.board_id = match resume {
            Some(id) => id,
            None => self.create_moved_board(space_id, &name, &tag)?,
        };

        let mapper = KeyMap {
            person: format!("{}{me}", kinds::PERSON_ACTOR_PREFIX),
            user: format!("user:{}", self.device.id),
        };
        let newest_pr = newest_pr_rows(&rows);
        let mut last_seq = 0;
        for ev in &rows {
            let Some(kind) = copied_kind(ev, &newest_pr) else {
                *out.rows_left.entry(ev.kind.clone()).or_default() += 1;
                continue;
            };
            let mark = format!("{space_id}/{}/{}", ev.device_id, ev.seq);
            if done.rows.contains(&mark) {
                out.rows_already += 1;
                continue;
            }
            let Ok(mut v) = serde_json::from_slice::<Value>(&ev.payload) else {
                *out.rows_left.entry(ev.kind.clone()).or_default() += 1;
                continue;
            };
            rewrite(&mut v, &kind, &out.board_id, &mapper);
            if let Value::Object(map) = &mut v {
                map.insert(MOVED_FIELD.into(), Value::String(mark));
            }
            let actor = if ev.actor == mapper.person {
                "user".to_string()
            } else {
                ev.actor.clone()
            };
            let w = self.store.append(&actor, &kind, &v)?;
            last_seq = w.seq;
            out.rows_copied += 1;
        }
        self.wait_folded(last_seq)?;

        let diffs = self.compare_move(&b, &mut out);
        if !diffs.is_empty() {
            return Err(Error::coord(
                CK::BadState,
                format_args!(
                    "the copy in Personal differs from the space on {} card(s), so the space is kept: {}",
                    diffs.len(),
                    diffs.join("; ")
                ),
            ));
        }
        if !o.keep_space {
            self.forget_everywhere(space_id)?;
            out.forgotten = true;
        }
        Ok(out)
    }

    /// The tag must be free but for the space itself (which holds it until
    /// it's forgotten) and the board an earlier run of this move made.
    fn check_move_tag(&self, space_id: &str, tag: &str, resume: Option<&str>) -> Result<()> {
        let clash = |other: &str| {
            Err(Error::coord(
                CK::Invalid,
                format_args!(
                    "the tag {} is already board {other}'s",
                    gocompat::quote::quote(tag)
                ),
            ))
        };
        let held = self.tag_holder(tag);
        if !held.is_empty() && held != space_id {
            return clash(&held);
        }
        if let Some((_, id)) = board_tag_ids(&self.store)
            .into_iter()
            .find(|(t, id)| t == tag && Some(id.as_str()) != resume)
        {
            return clash(&id);
        }
        for other in self.visible_boards() {
            if other.id == space_id {
                continue;
            }
            if other.verifier.membership().tag == tag {
                return clash(&other.id);
            }
            if let Some((_, id)) = board_tag_ids(&other.store)
                .into_iter()
                .find(|(t, _)| t == tag)
            {
                return clash(&id);
            }
        }
        Ok(())
    }

    /// Writes the Personal board for the move, marked as the space's.
    fn create_moved_board(&self, space_id: &str, name: &str, tag: &str) -> Result<String> {
        let id = super::boards::new_board_id()?;
        let row = serde_json::json!({
            "boardId": id,
            "name": name,
            "tag": tag,
            "by": format!("user:{}", self.device.id),
            "at": (self.now)().format_rfc3339_nano(),
            MOVED_FIELD: space_id,
        });
        self.store
            .append("user", kinds::SPACE_BOARD_CREATED, &row)?;
        self.coord.refresh_boards();
        self.boards.moved();
        Ok(id)
    }

    /// What an earlier run of moving `space_id` wrote from this device: its
    /// board, and each row it copied.
    fn moved_rows(&self, space_id: &str) -> Result<Moved> {
        let mut out = Moved::default();
        let prefix = format!("{space_id}/");
        each_page(
            &self.store,
            Filter {
                device_id: self.device.id.clone(),
                ..Filter::default()
            },
            |ev| {
                let Ok(v) = serde_json::from_slice::<Value>(&ev.payload) else {
                    return;
                };
                let Some(mark) = v.get(MOVED_FIELD).and_then(Value::as_str) else {
                    return;
                };
                if ev.kind == kinds::SPACE_BOARD_CREATED {
                    if mark == space_id
                        && let Some(id) = v.get("boardId").and_then(Value::as_str)
                    {
                        out.board = Some(id.to_string());
                    }
                } else if mark.starts_with(&prefix) {
                    out.rows.insert(mark.to_string());
                }
            },
        )?;
        // A board an earlier run made and someone deleted since is not
        // gone on with: its tag is burnt, so the move is refused.
        if let Some(id) = &out.board
            && !self.personal_boards()?.iter().any(|x| &x.board_id == id)
        {
            return Err(Error::coord(
                CK::BadState,
                format_args!("the board an earlier move made, {id}, was deleted since"),
            ));
        }
        Ok(out)
    }

    /// Makes every file the cards attach held here and offered in the
    /// private log, so the person's other devices can fetch it once the
    /// space is gone. Returns the ones no device could give.
    fn hold_files(&self, files: &[blob::FileRef]) -> Vec<String> {
        let mut missing = Vec::new();
        for r in files {
            if !self.files.has(&r.sha256) {
                let got = crate::meshview::fetch_file_blocking(
                    &self.mesh,
                    &self.pairing.handle,
                    &self.device.id,
                    &self.chain,
                    &self.device_names.chosen(),
                    r,
                );
                if got.is_err() || !self.files.has(&r.sha256) {
                    missing.push(format!("{} ({})", r.name, &r.sha256[..r.sha256.len().min(8)]));
                    continue;
                }
            }
            if let Err(e) = self.mesh.node.offer(r) {
                logf(format_args!("move: offer {}: {e}", r.sha256));
            }
        }
        missing
    }

    /// The space's agents' names, kept under their space keys, which the
    /// copied rows go on naming them by.
    fn note_space_agent_names(&self, b: &BoardLog) {
        let cards: Vec<coord::roster::AgentCard> = super::boards::lock(&b.maps)
            .cards
            .values()
            .filter(|c| !c.name.trim().is_empty())
            .map(|c| coord::roster::AgentCard {
                key: format!("{}{}", kinds::AGENT_ACTOR_PREFIX, c.card_id),
                name: c.name.clone(),
                ..coord::roster::AgentCard::default()
            })
            .collect();
        self.agent_names
            .note(&cards, presence::now_local().secs);
    }

    /// Waits until the private fold has taken in this device's rows up to
    /// `seq`.
    fn wait_folded(&self, seq: i64) -> Result<()> {
        let until = Instant::now() + FOLD_WAIT;
        while self.coord.folded_to(&self.device.id) < seq {
            if Instant::now() > until {
                return Err(Error::coord(
                    CK::BadState,
                    "the copy didn't fold in time; the space is kept, and moving it again goes on from here",
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }

    /// Adds a name for each space agent key (`agent:<cardId>`) the
    /// person's `board.cardof` rows map to a session with a known name, so
    /// cards moved out of a space still name their agents on every device.
    pub(crate) fn card_key_names(&self, names: &mut BTreeMap<String, String>) {
        let mut add = Vec::new();
        let _ = each_page(
            &self.store,
            Filter {
                kinds: vec![kinds::BOARD_CARD_OF.into()],
                ..Filter::default()
            },
            |ev| {
                let Ok(v) = serde_json::from_slice::<Value>(&ev.payload) else {
                    return;
                };
                let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let (card, agent) = (s("cardId"), s("agentKey"));
                if !card.is_empty() && !agent.is_empty() {
                    add.push((format!("{}{card}", kinds::AGENT_ACTOR_PREFIX), agent));
                }
            },
        );
        for (key, agent) in add {
            if names.contains_key(&key) {
                continue;
            }
            if let Some(n) = names.get(&agent).filter(|n| !n.trim().is_empty()).cloned() {
                names.insert(key, n);
            }
        }
    }

    /// Compares each of the space's cards with its copy, filling the
    /// counts in `out`; returns one line per card that differs.
    fn compare_move(&self, b: &BoardLog, out: &mut SpaceMove) -> Vec<String> {
        let mut diffs = Vec::new();
        let src = b.coord().list_tasks(&coord::TaskFilter::default());
        out.cards = src.len();
        for t in &src {
            let Some(c) = self.coord.get_task(&t.id) else {
                diffs.push(format!("{} is missing", t.short_ref()));
                continue;
            };
            let (a, z) = (CardFacts::of(t), CardFacts::of(&c));
            if a != z {
                diffs.push(format!("{}: {}", t.short_ref(), facts_diff(&a, &z)));
            }
            if c.board != out.board_id {
                diffs.push(format!("{} is on board {}", t.short_ref(), c.board));
            }
            if !coord::is_terminal(&c.state)
                && c.holders.is_empty()
                && self.coord.is_person(&c.created_by)
            {
                out.open_by_person.push(c.short_ref());
            }
        }
        diffs.truncate(20);
        diffs
    }
}

#[derive(Default)]
struct Moved {
    board: Option<String>,
    rows: HashSet<String>,
}

/// The person's key in the space and in the private log.
struct KeyMap {
    person: String,
    user: String,
}

/// Every row of `st`, in fold order.
fn all_rows(st: &Store) -> Result<Vec<Event>> {
    let mut out = Vec::new();
    each_page(st, Filter::default(), |ev| out.push(ev.clone()))?;
    Ok(out)
}

fn each_page(st: &Store, mut f: Filter, mut each: impl FnMut(&Event)) -> Result<()> {
    f.limit = MAX_LIST_LIMIT;
    loop {
        let evs = st.list(&f)?;
        for ev in &evs {
            each(ev);
        }
        if (evs.len() as i64) < f.limit {
            return Ok(());
        }
        f.after = evs[evs.len() - 1].cursor();
    }
}

/// The private kind a space row is copied as, or None when it stays.
fn copied_kind(ev: &Event, newest_pr: &HashSet<(String, i64)>) -> Option<String> {
    if let Some((_, k)) = RENAMED.iter().find(|(from, _)| *from == ev.kind) {
        return Some(k.to_string());
    }
    if ev.kind == kinds::PR_STATE {
        return newest_pr
            .contains(&(ev.device_id.clone(), ev.seq))
            .then(|| kinds::PR_STATE.to_string());
    }
    if LEFT_OUT.contains(&ev.kind.as_str()) {
        return None;
    }
    let card = ev.kind.starts_with("task.")
        || ev.kind.starts_with("blocker.")
        || ev.kind.starts_with("review.");
    card.then(|| ev.kind.clone())
}

/// The newest `pr.state` row of each (card, PR), by row.
fn newest_pr_rows(rows: &[Event]) -> HashSet<(String, i64)> {
    let mut newest: HashMap<(String, String), (String, i64)> = HashMap::new();
    for ev in rows.iter().filter(|e| e.kind == kinds::PR_STATE) {
        let Ok(v) = serde_json::from_slice::<Value>(&ev.payload) else {
            continue;
        };
        let get = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        newest.insert((get("taskId"), get("pr")), (ev.device_id.clone(), ev.seq));
    }
    newest.into_values().collect()
}

/// Every file a comment or a blocker attaches, once each.
fn file_refs(rows: &[Event]) -> Vec<blob::FileRef> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for ev in rows {
        if ev.kind != kinds::TASK_COMMENT && ev.kind != kinds::TASK_STATUS {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&ev.payload) else {
            continue;
        };
        let lists = [
            v.get("attachments"),
            v.get("blocker").and_then(|b| b.get("attachments")),
        ];
        for f in lists.into_iter().flatten().filter_map(Value::as_array).flatten() {
            let s = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let sha = s("sha256");
            if sha.is_empty() || !seen.insert(sha.clone()) {
                continue;
            }
            out.push(blob::FileRef {
                sha256: sha,
                name: s("name"),
                size: f.get("size").and_then(Value::as_i64).unwrap_or(0),
                mime: s("mime"),
                device: s("device"),
            });
        }
    }
    out
}

/// Makes a space row's payload the private log's: the board is the new
/// one, the person's key is this device's user key, and the space's own
/// fields go.
fn rewrite(v: &mut Value, kind: &str, board: &str, keys: &KeyMap) {
    swap_keys(v, keys);
    let Value::Object(map) = v else {
        return;
    };
    map.remove("hop");
    // A card's board, and a move's: the space's own board is "" there.
    if kind == kinds::TASK_CREATED || matches!(map.get("board"), Some(Value::String(_))) {
        map.insert("board".into(), Value::String(board.into()));
    }
}

/// Swaps the person's space key for their private one wherever it is a
/// whole string, and drops a person blocker's `ref` (private ones carry
/// none).
fn swap_keys(v: &mut Value, keys: &KeyMap) {
    match v {
        Value::String(s) if *s == keys.person => *s = keys.user.clone(),
        Value::Array(a) => a.iter_mut().for_each(|x| swap_keys(x, keys)),
        Value::Object(m) => {
            drop_person_ref(m);
            m.values_mut().for_each(|x| swap_keys(x, keys));
        }
        _ => {}
    }
}

fn drop_person_ref(m: &mut Map<String, Value>) {
    if m.get("kind").and_then(Value::as_str) == Some("person") && m.contains_key("ref") {
        m.remove("ref");
    }
}

fn facts_diff(a: &CardFacts, z: &CardFacts) -> String {
    let mut out = Vec::new();
    let mut f = |what: &str, x: String, y: String| {
        if x != y {
            out.push(format!("{what} {x} → {y}"));
        }
    };
    f("number", a.num.clone(), z.num.clone());
    f("title", a.title.clone(), z.title.clone());
    f("state", a.state.clone(), z.state.clone());
    f("comments", a.comments.to_string(), z.comments.to_string());
    f("steps", a.steps.len().to_string(), z.steps.len().to_string());
    f(
        "ticked steps",
        a.steps.iter().filter(|s| s.1).count().to_string(),
        z.steps.iter().filter(|s| s.1).count().to_string(),
    );
    f("blockers", a.blockers.to_string(), z.blockers.to_string());
    f("open blockers", a.open_blockers.to_string(), z.open_blockers.to_string());
    f("subtasks", a.subtasks.to_string(), z.subtasks.to_string());
    f("parent", a.parent.clone(), z.parent.clone());
    f("thread", a.thread.to_string(), z.thread.to_string());
    if out.is_empty() {
        out.push("step text".into());
    }
    out.join(", ")
}

fn logf(args: std::fmt::Arguments<'_>) {
    crate::logf(args);
}
