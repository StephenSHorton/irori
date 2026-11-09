//! A huddle's canvas: its rows and the writes onto it (card #1051,
//! docs/design/call-agents.md §7 "Output", §8).
//!
//! Three writers put `call.item` rows on the canvas:
//!
//! - **a person**, through the window's commands ([`Services::huddle_item_add`],
//!   [`Services::huddle_item_set`], [`Services::huddle_title_set`]);
//! - **their assistant**, whose reply is parsed here ([`parse_blocks`]) into
//!   rows written under the person's own key with `assistant: true`. No block
//!   carries a status, so every item an assistant writes is `open`;
//! - **Ask**, a `question` item the person writes with `why: "ask"`
//!   ([`Services::huddle_ask`]). Whichever device hosts their assistant sees
//!   the row arrive, so Ask works from a phone whose assistant runs on a
//!   computer, and gets a look.
//!
//! Nothing here makes a card, task or anything else: the blocks have no such
//! kind, the look has no tools, and the write path refuses `carded` (and a
//! card ref) while the huddle is live. The fold never asks whether a call is
//! live (that is per-device memory); only this write path does.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coord::talk::{CanvasItem, NewItem, NewItemSet, is_person_key};

use super::Services;
use super::huddle_packet::{self, DATA_TAG, Packet};
use super::huddle_rules::{Block, Kind, Layer, Look, admit, is_duplicate};
use crate::events::CoreEvent;

/// `logf` with format arguments.
macro_rules! logf {
    ($($t:tt)*) => { crate::logf(format_args!($($t)*)) };
}

/// What a placeholder says between the assistant's name and the words it is
/// looking into. A new host closes the old host's placeholders that still
/// say it.
pub(super) const LOOKING: &str = " is looking: ";
/// An Ask row older than this wakes nothing (a burst after a reconnect).
const ASK_FRESH: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------- the parser

/// Splits a look's reply into blocks (§7 "Output"): `<<item kind="…"
/// reply="…" refs="a,b">>text<</item>>` and `<<title>>text<</title>>`.
/// Anything else is ignored, so a chatty reply writes nothing. An item with
/// an unknown kind, no closing tag or no text is dropped. Limits (five items,
/// 2000 characters, who may write a title or summary) are
/// [`admit`]'s; this only reads.
pub(super) fn parse_blocks(reply: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut rest = reply;
    loop {
        let item = find_open(rest, "<<item");
        let title = rest.find("<<title>>");
        let (at, is_item) = match (item, title) {
            (Some(i), Some(t)) if t < i => (t, false),
            (Some(i), _) => (i, true),
            (None, Some(t)) => (t, false),
            (None, None) => break,
        };
        if is_item {
            let after = &rest[at + "<<item".len()..];
            let Some(head_end) = after.find(">>") else {
                break;
            };
            let attrs = &after[..head_end];
            let body = &after[head_end + 2..];
            let Some(close) = body.find("<</item>>") else {
                break;
            };
            let text = body[..close].trim();
            let attrs = parse_attrs(attrs);
            let kind = attrs
                .get("kind")
                .and_then(|k| Kind::parse(&k.trim().to_ascii_lowercase()));
            if let Some(kind) = kind
                && !text.is_empty()
            {
                out.push(Block::Item {
                    kind,
                    reply: attrs
                        .get("reply")
                        .map(|r| r.trim().to_string())
                        .filter(|r| !r.is_empty()),
                    refs: attrs
                        .get("refs")
                        .map(|r| {
                            r.split(',')
                                .map(|x| x.trim().to_string())
                                .filter(|x| !x.is_empty())
                                .collect()
                        })
                        .unwrap_or_default(),
                    text: text.to_string(),
                });
            }
            rest = &body[close + "<</item>>".len()..];
        } else {
            let body = &rest[at + "<<title>>".len()..];
            let Some(close) = body.find("<</title>>") else {
                break;
            };
            let text = body[..close].trim();
            if !text.is_empty() {
                out.push(Block::Title(text.to_string()));
            }
            rest = &body[close + "<</title>>".len()..];
        }
    }
    out
}

/// The first `<<item` in `s` that is the whole tag name (`<<item>>` or
/// `<<item kind=…`), not `<<items`.
fn find_open(s: &str, tag: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = s[from..].find(tag) {
        let at = from + i;
        match s[at + tag.len()..].chars().next() {
            Some(c) if c.is_whitespace() || c == '>' => return Some(at),
            _ => from = at + tag.len(),
        }
    }
    None
}

/// `name="value"` pairs (double or single quotes).
fn parse_attrs(s: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut rest = s.trim_start();
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim().to_ascii_lowercase();
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            break;
        };
        let inner = &after[1..];
        let Some(end) = inner.find(q) else { break };
        if !name.is_empty() {
            out.entry(name).or_insert_with(|| inner[..end].to_string());
        }
        rest = inner[end + 1..].trim_start();
    }
    out
}

// ------------------------------------------------------------- what is kept

/// What the held session has been shown of the canvas and the packet, so a
/// look carries only what changed since (§8: "each look carries cards
/// changed and items added since the last").
#[derive(Debug, Default)]
pub(crate) struct CanvasMem {
    /// Item ids the model has been shown (or wrote itself).
    known_items: BTreeSet<String>,
    /// The packet's cards as the model last saw them: ref to signature.
    known_cards: BTreeMap<String, String>,
    /// Whether anything has been shown yet.
    primed: bool,
    /// Placeholders this device wrote and hasn't resolved.
    looking: BTreeSet<String>,
    /// Ask items already given a look (the follow also delivers own rows).
    asked: BTreeSet<String>,
}

fn card_sig(c: &huddle_packet::PacketCard) -> String {
    format!("{}|{}|{}", c.title, c.state, c.holders.join(","))
}

fn cards_of(p: &Packet) -> BTreeMap<String, String> {
    p.cards
        .iter()
        .map(|c| (c.card_ref.clone(), card_sig(c)))
        .collect()
}

/// `text` with whitespace collapsed to single spaces, at most `n`
/// characters.
fn one_line(text: &str, n: usize) -> String {
    huddle_packet::clip(text, n)
}

// ---------------------------------------------------------- the Core's side

/// A canvas item with the names the page shows it by (card #1051).
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasEntry {
    pub item: CanvasItem,
    /// Who signed it.
    pub by_name: String,
    /// "Juno · Addam's assistant" when `item.assistant`, else "".
    pub label: String,
    /// Who started the look that wrote it, when it says.
    pub asker_name: String,
    /// The signer is this device's person.
    pub mine: bool,
    /// The asker is this device's person.
    pub asker_mine: bool,
}

impl Services {
    /// The key this device's person writes under.
    fn canvas_me(&self) -> String {
        self.user_sender().key
    }

    fn canvas_conv(&self, call: &str) -> Result<String, String> {
        self.talk
            .call_conv(call)
            .ok_or_else(|| format!("no huddle {call} here"))
    }

    /// Logs a refused write, so a space that isn't ready leaves a trace.
    fn canvas_done<T>(
        &self,
        call: &str,
        what: &str,
        r: Result<T, coord::Error>,
    ) -> Result<T, String> {
        match r {
            Ok(v) => {
                self.hub.publish(CoreEvent::Talk);
                Ok(v)
            }
            Err(e) => {
                logf!("call {call}: {what}: {e}");
                Err(e.to_string())
            }
        }
    }

    /// Whether a change would card something (or name its card) while the
    /// huddle is still live: refused (§8). The fold never asks.
    fn canvas_carding_refused(&self, call: &str, carded: bool) -> Result<(), String> {
        if carded && !self.call_over(call) {
            return Err("nothing is carded while the huddle is still live".into());
        }
        Ok(())
    }

    /// A person adds an item to huddle `call`'s canvas.
    pub(crate) fn huddle_item_add(
        &self,
        call: &str,
        kind: &str,
        text: &str,
        status: &str,
    ) -> Result<CanvasItem, String> {
        let conv = self.canvas_conv(call)?;
        self.canvas_carding_refused(call, status.trim() == "carded")?;
        let r = self.talk.write_item(
            &conv,
            call,
            NewItem {
                kind: kind.trim().into(),
                text: text.into(),
                status: status.trim().into(),
                ..NewItem::default()
            },
            &self.canvas_me(),
        );
        let item = self.canvas_done(call, "item", r)?;
        self.huddle_route_question(call, &item);
        Ok(item)
    }

    /// A person changes an item: its text (their own), its status, its
    /// position, or the card it became. `carded` and `card` are refused
    /// while the huddle is live.
    pub(crate) fn huddle_item_set(
        &self,
        call: &str,
        set: NewItemSet,
    ) -> Result<CanvasItem, String> {
        let conv = self.canvas_conv(call)?;
        self.canvas_carding_refused(
            call,
            set.status.as_deref() == Some("carded")
                || set.card.as_deref().is_some_and(|c| !c.trim().is_empty()),
        )?;
        let r = self.talk.write_item_set(
            &conv,
            call,
            NewItemSet {
                assistant: false,
                ..set
            },
            &self.canvas_me(),
        );
        self.canvas_done(call, "item set", r)
    }

    /// The Ask box: the person puts a question to their assistant. It is a
    /// `question` item of their own (why `ask`), which whichever device
    /// hosts their assistant sees and gives a look.
    pub(crate) fn huddle_ask(&self, call: &str, text: &str) -> Result<CanvasItem, String> {
        let me_id = self.person_id();
        let mine = self.call_assistants(call).into_iter().any(|a| {
            a.state == "connected" && (is_person_key(&a.person) || a.person.ends_with(&me_id))
        });
        if !mine {
            return Err("your assistant isn't connected to this huddle".into());
        }
        if self.call_over(call) {
            return Err("this huddle is over".into());
        }
        let conv = self.canvas_conv(call)?;
        let me = self.canvas_me();
        let item = self.canvas_done(
            call,
            "ask",
            self.talk.write_item(
                &conv,
                call,
                NewItem {
                    kind: "question".into(),
                    text: text.into(),
                    asker: Some(me.clone()),
                    why: Some("ask".into()),
                    ..NewItem::default()
                },
                &me,
            ),
        )?;
        // The hosting device's follow delivers the row too; this makes sure
        // it is not missed when the host is this one.
        self.huddle_item_landed(call, &item);
        self.huddle_route_question(call, &item);
        Ok(item)
    }

    // ------------------------------------------------------ assistant writes

    /// What the canvas shows `call`'s texts as (for the duplicate check).
    fn canvas_texts(&self, call: &str) -> Vec<String> {
        self.talk
            .call_items(call)
            .into_iter()
            .map(|i| i.text)
            .collect()
    }

    /// The assistant's short name for a placeholder: "Juno", else "Addam's
    /// assistant".
    fn assistant_short(&self) -> String {
        let label = self.assistant_label_text();
        label.split(" \u{b7} ").next().unwrap_or("").to_string()
    }

    fn assistant_label_text(&self) -> String {
        coord::assistant_name::label(&self.ident.current().name, &self.assistant_name.chosen())
    }

    fn canvas_write_assistant(&self, call: &str, conv: &str, n: NewItem) -> Option<CanvasItem> {
        let me = self.canvas_me();
        match self.talk.write_item(
            conv,
            call,
            NewItem {
                assistant: true,
                ..n
            },
            &me,
        ) {
            Ok(i) => {
                self.hub.publish(CoreEvent::Talk);
                self.canvas_mem(call, |m| {
                    m.known_items.insert(i.item_id.clone());
                });
                // A question goes to whoever knows (L3b).
                self.huddle_route_question(call, &i);
                Some(i)
            }
            Err(e) => {
                logf!("huddle {call}: canvas write refused: {e}");
                None
            }
        }
    }

    fn canvas_set_assistant(&self, call: &str, conv: &str, id: &str, text: &str) {
        let me = self.canvas_me();
        let r = self.talk.write_item_set(
            conv,
            call,
            NewItemSet {
                item_id: id.into(),
                text: Some(text.into()),
                assistant: true,
                ..NewItemSet::default()
            },
            &me,
        );
        match r {
            Ok(_) => self.hub.publish(CoreEvent::Talk),
            Err(e) => logf!("huddle {call}: placeholder {id}: {e}"),
        }
    }

    fn canvas_mem<R>(&self, call: &str, f: impl FnOnce(&mut CanvasMem) -> R) -> R {
        let mut g = self.feed.canvas.lock().unwrap_or_else(|e| e.into_inner());
        f(g.entry(call.to_string()).or_default())
    }

    /// On an L1 match, before the model runs (§7 "Fast acknowledgement"):
    /// an item that says the assistant is looking. The answer replaces it.
    /// Returns its id when written.
    pub(super) fn huddle_placeholder(&self, call: &str, look: &Look) -> Option<String> {
        if look.layer != Layer::Named {
            return None;
        }
        let conv = self.talk.call_conv(call)?;
        let line = self.huddle_look_text(call, look);
        let text = format!(
            "{}{LOOKING}{}",
            self.assistant_short(),
            one_line(&line, 300)
        );
        let item = self.canvas_write_assistant(
            call,
            &conv,
            NewItem {
                kind: "answer".into(),
                text,
                asker: look.caller.clone(),
                why: Some("name".into()),
                ..NewItem::default()
            },
        )?;
        self.canvas_mem(call, |m| m.looking.insert(item.item_id.clone()));
        Some(item.item_id)
    }

    /// Sets a placeholder to `text` (an answer, or why there is none), and
    /// forgets it.
    pub(super) fn huddle_placeholder_close(&self, call: &str, id: &str, text: &str) {
        if let Some(conv) = self.talk.call_conv(call) {
            self.canvas_set_assistant(call, &conv, id, text);
        }
        self.canvas_mem(call, |m| m.looking.remove(id));
    }

    /// A new host closes the old host's placeholders that still say they are
    /// looking (§7), when it connects: this person's assistant items with
    /// the looking words, none of them one of this device's own in flight.
    pub(super) fn huddle_close_stale_placeholders(&self, call: &str) {
        let Some(conv) = self.talk.call_conv(call) else {
            return;
        };
        let short = self.assistant_short();
        let mine = self.canvas_mem(call, |m| m.looking.clone());
        for i in self.talk.call_items(call) {
            if i.assistant
                && is_person_key(&i.by)
                && !mine.contains(&i.item_id)
                && i.text.contains(LOOKING)
                && i.kind == "answer"
            {
                let who = i.text.split(LOOKING).next().unwrap_or(&short).to_string();
                self.canvas_set_assistant(
                    call,
                    &conv,
                    &i.item_id,
                    &format!("{who} couldn't answer"),
                );
            }
        }
    }

    /// Writes a finished look's reply onto the canvas: the blocks of `text`,
    /// admitted by [`admit`] (chair rules are card 10's, so no title or
    /// summary yet), then each item as an assistant row of this person's with
    /// `asker` and `why` from the look. An item whose text is on the canvas
    /// already is dropped. The first answer resolves a `placeholder`, which
    /// says "found nothing" when there is no answer.
    pub(super) fn huddle_write_look(
        &self,
        call: &str,
        look: &Look,
        text: &str,
        placeholder: Option<&str>,
    ) {
        let Some(conv) = self.talk.call_conv(call) else {
            return;
        };
        // The chair may add a title and a summary; the end's writer is the
        // chair for that look even when it is only covering for it (§3).
        let chair = look.layer == Layer::End || self.huddle_is_chair(call);
        let blocks = admit(look.layer, chair, parse_blocks(text));
        let mut existing = self.canvas_texts(call);
        let ids: BTreeSet<String> = self
            .talk
            .call_items(call)
            .into_iter()
            .map(|i| i.item_id)
            .collect();
        let mut placeholder = placeholder;
        let short = self.assistant_short();
        let mut wrote = 0;
        for b in blocks {
            let (kind, reply, refs, text) = match b {
                Block::Title(t) => {
                    self.huddle_propose_title(call, &t);
                    continue;
                }
                Block::Item {
                    kind,
                    reply,
                    refs,
                    text,
                } => (kind, reply, refs, text),
            };
            // The first answer takes a placeholder's place.
            if kind == Kind::Answer
                && let Some(p) = placeholder.take()
            {
                if is_duplicate(existing.iter().map(String::as_str), &text) {
                    self.huddle_placeholder_close(call, p, &format!("{short} found nothing new"));
                } else {
                    existing.push(text.clone());
                    self.huddle_placeholder_close(call, p, &text);
                    wrote += 1;
                }
                continue;
            }
            if is_duplicate(existing.iter().map(String::as_str), &text) {
                continue;
            }
            // The model sometimes writes the placeholder `ITEM-ID` literally.
            // A reply it can't resolve is dropped, the item kept.
            let reply = reply.filter(|r| ids.contains(r));
            let refs: Vec<String> = refs.into_iter().filter(|r| ids.contains(r)).collect();
            existing.push(text.clone());
            if self
                .canvas_write_assistant(
                    call,
                    &conv,
                    NewItem {
                        kind: kind.as_str().into(),
                        text,
                        reply,
                        refs,
                        asker: look.caller.clone(),
                        why: Some(look.layer.why().into()),
                        ..NewItem::default()
                    },
                )
                .is_some()
            {
                wrote += 1;
            }
        }
        if let Some(p) = placeholder {
            self.huddle_placeholder_close(call, p, &format!("{short} found nothing"));
        }
        // A question put on the page that the packet can't answer gets an
        // open item saying so, for after the call (§7).
        if look.layer == Layer::Ask && wrote == 0 {
            let q = self.huddle_look_text(call, look);
            self.canvas_write_assistant(
                call,
                &conv,
                NewItem {
                    kind: "note".into(),
                    text: format!("{short} found nothing to answer: {}", one_line(&q, 300)),
                    asker: look.caller.clone(),
                    why: Some("ask".into()),
                    ..NewItem::default()
                },
            );
        }
    }

    /// The chair's title: written as the assistant's, which a person's title
    /// always beats. Not when a person has set one.
    fn huddle_propose_title(&self, call: &str, title: &str) {
        let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
        let title: String = title.chars().take(80).collect();
        if title.is_empty() {
            return;
        }
        let person_set = self
            .talk
            .calls()
            .into_iter()
            .find(|c| c.call_id == call)
            .is_some_and(|c| !c.title.is_empty() && !c.title_by_assistant);
        if person_set {
            return;
        }
        let _ = self.call_title_set(call, &title, true);
    }

    /// A look that timed out or failed: its placeholder says so.
    pub(super) fn huddle_look_failed(&self, call: &str, placeholder: Option<&str>) {
        if let Some(p) = placeholder {
            let short = self.assistant_short();
            self.huddle_placeholder_close(call, p, &format!("{short} couldn't answer"));
        }
    }

    /// The cap's note (§6): once, as the assistant's.
    pub(super) fn huddle_cap_note(&self, call: &str) {
        let Some(conv) = self.talk.call_conv(call) else {
            return;
        };
        let short = self.assistant_short();
        self.canvas_write_assistant(
            call,
            &conv,
            NewItem {
                kind: "note".into(),
                text: format!("{short} has done its looks for this huddle."),
                ..NewItem::default()
            },
        );
    }

    // ----------------------------------------------------------- what it sees

    /// One line per canvas item for the packet and a restart's seed:
    /// `<id> · <kind> · <status> · <who>: <text>`.
    pub(super) fn huddle_canvas_lines(&self, call: &str) -> Vec<String> {
        let items = self.talk.call_items(call);
        self.canvas_lines_of(call, &items)
    }

    fn canvas_lines_of(&self, call: &str, items: &[CanvasItem]) -> Vec<String> {
        if items.is_empty() {
            return Vec::new();
        }
        let entries = self.canvas_entries(call, items);
        entries
            .iter()
            .map(|e| {
                let who = if e.item.assistant {
                    e.label.clone()
                } else {
                    e.by_name.clone()
                };
                let mut line = format!("{} \u{b7} {}", e.item.item_id, e.item.kind);
                if !e.item.status.is_empty() {
                    line.push_str(&format!(" \u{b7} {}", e.item.status));
                }
                if let Some(r) = &e.item.reply {
                    line.push_str(&format!(" \u{b7} reply to {r}"));
                }
                line.push_str(&format!(" \u{b7} {who}: {}", e.item.text));
                line
            })
            .collect()
    }

    /// Marks what a new session was shown (its packet and canvas), so the
    /// next look carries only what changed.
    pub(crate) fn huddle_note_shown(&self, call: &str, p: &Packet) {
        let ids: Vec<String> = self
            .talk
            .call_items(call)
            .into_iter()
            .map(|i| i.item_id)
            .collect();
        let cards = cards_of(p);
        self.canvas_mem(call, |m| {
            m.known_items = ids.into_iter().collect();
            m.known_cards = cards;
            m.primed = true;
        });
    }

    /// What changed since the last look, fenced: the canvas items added by
    /// others, and the cards that are new, changed or gone. Updates what the
    /// session has been shown. None when nothing did.
    pub(super) fn huddle_delta(&self, call: &str, conv: &str) -> Option<String> {
        let items = self.talk.call_items(call);
        let (new_items, primed) = self.canvas_mem(call, |m| {
            let new: Vec<CanvasItem> = items
                .iter()
                .filter(|i| !m.known_items.contains(&i.item_id))
                .cloned()
                .collect();
            for i in &new {
                m.known_items.insert(i.item_id.clone());
            }
            (new, m.primed)
        });
        let mut out = String::new();
        if !new_items.is_empty() {
            let lines = self.canvas_lines_of(call, &new_items);
            out.push_str("Added to the canvas since your last look:\n");
            out.push_str(&huddle_packet::fence(DATA_TAG, "canvas", &lines.join("\n")));
        }
        if primed && let Ok(p) = self.huddle_packet(conv) {
            let now = cards_of(&p);
            let by_ref: BTreeMap<&str, &huddle_packet::PacketCard> =
                p.cards.iter().map(|c| (c.card_ref.as_str(), c)).collect();
            let (changed, gone) = self.canvas_mem(call, |m| {
                let changed: Vec<String> = now
                    .iter()
                    .filter(|(r, sig)| m.known_cards.get(*r) != Some(*sig))
                    .map(|(r, _)| r.clone())
                    .collect();
                let gone: Vec<String> = m
                    .known_cards
                    .keys()
                    .filter(|r| !now.contains_key(*r))
                    .cloned()
                    .collect();
                m.known_cards = now.clone();
                (changed, gone)
            });
            let mut lines: Vec<String> = Vec::new();
            for r in &changed {
                if let Some(c) = by_ref.get(r.as_str()) {
                    let mut l = format!(
                        "{} {} \u{b7} {}",
                        c.card_ref,
                        huddle_packet::clip(&c.title, huddle_packet::MAX_TITLE),
                        c.state
                    );
                    if !c.holders.is_empty() {
                        l.push_str(&format!(" \u{b7} held by {}", c.holders.join(", ")));
                    }
                    lines.push(l);
                }
            }
            for r in &gone {
                lines.push(format!("{r} \u{b7} no longer open"));
            }
            if !lines.is_empty() {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str("Cards that changed since your last look:\n");
                out.push_str(&huddle_packet::fence(
                    DATA_TAG,
                    "space-cards",
                    &lines.join("\n"),
                ));
            }
        } else if let Ok(p) = self.huddle_packet(conv) {
            // Nothing shown yet: this is the baseline.
            let cards = cards_of(&p);
            self.canvas_mem(call, |m| {
                m.known_cards = cards;
                m.primed = true;
            });
        }
        (!out.is_empty()).then_some(out)
    }

    // --------------------------------------------------------------- Ask

    /// A `call.item` row landed on a live path (the three follows, and an
    /// Ask this device wrote): if it is this person's own Ask, gives their
    /// assistant a look at it. Only the hosting device feeds; a row already
    /// handled, an old one and an assistant's wake nothing.
    pub(super) fn huddle_item_landed(&self, call: &str, item: &CanvasItem) {
        if item.kind != "question"
            || item.why.as_deref() != Some("ask")
            || item.assistant
            || !is_person_key(&item.by)
            || !self.huddle_in(call)
        {
            return;
        }
        let fresh = {
            let ms = item.at.unix_milli_value();
            let at = UNIX_EPOCH + Duration::from_millis(ms.max(0) as u64);
            SystemTime::now().duration_since(at).unwrap_or_default() <= ASK_FRESH
        };
        if !fresh {
            return;
        }
        if !self.canvas_mem(call, |m| m.asked.insert(item.item_id.clone())) {
            return;
        }
        self.huddle_ask_look(call, &item.item_id);
    }

    /// A row of a log, if it is a `call.item`: feeds it.
    pub(crate) fn huddle_item_row(&self, ev: &store::Event) {
        if ev.kind != store::kinds::CALL_ITEM {
            return;
        }
        let Ok(r) = serde_json::from_slice::<coord::talk::ItemRow>(&ev.payload) else {
            return;
        };
        // Only the huddle's own conversation counts.
        if self.talk.call_conv(&r.call_id).as_deref() != Some(r.conv_id.as_str()) {
            return;
        }
        if let Some(i) = self
            .talk
            .call_items(&r.call_id)
            .into_iter()
            .find(|i| i.item_id == r.item_id)
        {
            self.huddle_item_landed(&r.call_id, &i);
            // A question on the canvas, from anyone, goes to the assistants
            // whose people hold a matching card (L3b).
            self.huddle_route_question(&r.call_id, &i);
        }
    }

    // ---------------------------------------------------------- the page's view

    /// Huddle `call`'s canvas for the page: each item with the names it is
    /// shown by.
    pub fn call_canvas(&self, call: &str) -> Vec<CanvasEntry> {
        let items = self.talk.call_items(call);
        self.canvas_entries(call, &items)
    }

    fn canvas_entries(&self, call: &str, items: &[CanvasItem]) -> Vec<CanvasEntry> {
        let mut keys: Vec<String> = Vec::new();
        for i in items {
            for k in std::iter::once(&i.by).chain(i.asker.iter()) {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
        }
        let shown = self
            .this
            .get()
            .and_then(std::sync::Weak::upgrade)
            .map(|me| tools::talk::names(&super::ToolsView(me), &keys))
            .unwrap_or_default();
        let names: BTreeMap<String, String> = keys.iter().cloned().zip(shown).collect();
        let me_name = self.ident.current().name.clone();
        let space = self
            .talk
            .call_conv(call)
            .and_then(|c| self.talk.channel(&c))
            .map(|c| c.space_id)
            .filter(|s| !s.is_empty());
        let name_of = |k: &str| {
            if is_person_key(k) && !me_name.trim().is_empty() {
                me_name.clone()
            } else {
                names.get(k).cloned().unwrap_or_default()
            }
        };
        items
            .iter()
            .map(|i| {
                let label = if i.assistant {
                    let chosen = if is_person_key(&i.by) {
                        self.assistant_name.chosen()
                    } else {
                        self.profile_assistant_chosen(space.as_deref(), &i.by)
                    };
                    coord::assistant_name::label(&name_of(&i.by), &chosen)
                } else {
                    String::new()
                };
                CanvasEntry {
                    item: i.clone(),
                    by_name: name_of(&i.by),
                    label,
                    asker_name: i.asker.as_deref().map(&name_of).unwrap_or_default(),
                    mine: is_person_key(&i.by),
                    asker_mine: i.asker.as_deref().is_some_and(is_person_key),
                }
            })
            .collect()
    }

    /// Another person's chosen assistant name, from the signer's profile in
    /// the huddle's space (else any space the person shares).
    fn profile_assistant_chosen(&self, space: Option<&str>, key: &str) -> String {
        let pid = key
            .strip_prefix(store::kinds::PERSON_ACTOR_PREFIX)
            .unwrap_or(key);
        let boards = match space.and_then(|s| self.board(s).ok()) {
            Some(b) => vec![b],
            None => self.visible_boards(),
        };
        boards
            .iter()
            .map(|b| b.verifier.profile_assistant_name(pid))
            .find(|n| !n.trim().is_empty())
            .unwrap_or_default()
    }

    /// Forgets what `call`'s session was shown (the huddle isn't served any
    /// more).
    pub(crate) fn huddle_canvas_forget(&self, call: &str) {
        self.feed
            .canvas
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(call);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(b: &Block) -> (Kind, Option<&str>, &[String], &str) {
        match b {
            Block::Item {
                kind,
                reply,
                refs,
                text,
            } => (*kind, reply.as_deref(), refs.as_slice(), text.as_str()),
            Block::Title(_) => panic!("a title"),
        }
    }

    #[test]
    fn parses_items_and_titles_and_ignores_the_rest() {
        let r = "Sure!\n<<item kind=\"answer\" reply=\"i1\">>Addam has #412.<</item>>\nchatter\n\
                 <<title>>Release planning<</title>>\n<<item kind=\"summary\" refs=\"i1, i2\">>Done.<</item>>";
        let b = parse_blocks(r);
        assert_eq!(b.len(), 3);
        assert_eq!(
            item(&b[0]),
            (Kind::Answer, Some("i1"), &[][..], "Addam has #412.")
        );
        assert_eq!(b[1], Block::Title("Release planning".into()));
        let (k, _, refs, _) = item(&b[2]);
        assert_eq!(k, Kind::Summary);
        assert_eq!(refs, ["i1", "i2"]);
    }

    #[test]
    fn drops_what_it_cannot_read() {
        assert!(parse_blocks("nothing here").is_empty());
        // Unknown kind, empty text, no closing tag, a longer tag name.
        assert!(parse_blocks("<<item kind=\"chat\">>hi<</item>>").is_empty());
        assert!(parse_blocks("<<item kind=\"note\">>   <</item>>").is_empty());
        assert!(parse_blocks("<<item kind=\"note\">>never closed").is_empty());
        assert!(parse_blocks("<<items kind=\"note\">>x<</item>>").is_empty());
        // A status is never read: it is not an attribute.
        let b = parse_blocks("<<item kind=\"task\" status=\"settled\">>Do it.<</item>>");
        assert_eq!(b.len(), 1);
        // Single quotes and uppercase kinds read.
        let b = parse_blocks("<<item kind='NOTE'>>x<</item>>");
        assert_eq!(item(&b[0]).0, Kind::Note);
        // A bad block doesn't swallow the next.
        let b = parse_blocks("<<item kind=\"chat\">>a<</item>><<item kind=\"note\">>b<</item>>");
        assert_eq!(b.len(), 1);
        assert_eq!(item(&b[0]).3, "b");
    }

    #[test]
    fn a_block_carries_no_status_and_text_stays_whole() {
        let b =
            parse_blocks("<<item kind=\"decision\">>Keep the old updater <<until>> 0.16.<</item>>");
        assert_eq!(item(&b[0]).3, "Keep the old updater <<until>> 0.16.");
    }
}
