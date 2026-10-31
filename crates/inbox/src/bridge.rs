//! The bridge from shared boards into the inbox (v0.5 §7.1), and the two
//! settings that govern work crossing between people: the receive setting
//! (§7.2, decision 24, B2) and the agents' autonomy (§7.3, decisions 23
//! and 26).
//!
//! Board rows wake nothing on their own. The device that hosts an agent
//! turns a board row about a card the agent holds, watches or is assigned
//! into an ordinary private message.sent plus a delivery, from a sender of
//! its own kind ([`BOARD_KIND`], key `board:<boardId>/<cardId>` or
//! `board:<boardId>/person:<personId>`). Such a key names no device and is
//! never `user:`, so isOwner, the person check and decision 34's own-agent
//! skip can't match it. Each bridged message carries the row it came from,
//! and a (row, recipient) pair is bridged once, whichever of the person's
//! devices folds it. Both settings are private rows written by the person
//! in the app, replicated among the person's own devices.

use std::collections::BTreeSet;

use gocompat::json::{self, Out, Schema};
use gocompat::quote::quote;
use hlc::Timestamp;
use irori_identity::owner::device_of;
use serde::Serialize;
use store::Event;

use crate::messages::clip;
use crate::policy::{Cards, placed};
use crate::state::{DeliveryRow, Inner, b, pair_key, s, str_fields, strs, t, trim};
use crate::{
    Address, DIRECT_LIMIT, DIRECT_WINDOW, Delivery, Error, FileRef, Inbox, MAX_REF, MAX_TEXT,
    Message, Result, SYSTEM_KEY, Sender, addr, hold, kinds, mode, tm, trust,
};

/// The sender kind of a bridged board row: another person or their agent.
pub const BOARD_KIND: &str = "board";

/// Receive presets (§7.2).
pub mod receive {
    pub const CLOSED: &str = "closed";
    pub const ASK: &str = "ask";
    pub const MEMBERS: &str = "members";
    pub const OPEN: &str = "open";

    pub fn valid(p: &str) -> bool {
        matches!(p, CLOSED | ASK | MEMBERS | OPEN)
    }
}

/// Autonomy presets (§7.3).
pub mod autonomy {
    pub const ASK: &str = "ask";
    pub const TRUSTED: &str = "trusted";
    pub const AUTONOMOUS: &str = "autonomous";

    pub fn valid(p: &str) -> bool {
        matches!(p, ASK | TRUSTED | AUTONOMOUS)
    }
}

/// The receive preset with no row: Open (§7.2).
pub const RECEIVE_DEFAULT: &str = receive::OPEN;
/// The autonomy preset with no row: Trusted (§7.3).
pub const AUTONOMY_DEFAULT: &str = autonomy::TRUSTED;

/// What a bridged board row was, as the receive table sorts them.
pub mod row_kind {
    /// A comment, or anything else said on a card.
    pub const COMMENT: &str = "comment";
    /// A blocker raised, answered or closed.
    pub const BLOCKER: &str = "blocker";
    /// Work assigned to the recipient.
    pub const ASSIGN: &str = "assign";
    /// A board ask.
    pub const ASK: &str = "ask";
    /// A lease release request (§5.4).
    pub const LEASE: &str = "lease";
}

/// The reason on a wake request the receive setting turned into next turn.
pub const ASKED_TO_WAKE: &str = "asked to wake";

/// Where a bridged message came from (the message.sent `board` object).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BoardFrom {
    #[serde(rename = "boardId")]
    pub board_id: String,
    /// The board's tag, for the `board=` fence attribute (B3).
    pub tag: String,
    /// The other person, or the person whose agent wrote the row.
    #[serde(rename = "personId")]
    pub person_id: String,
    /// The writing agent's card; empty when the person wrote it.
    #[serde(rename = "cardId", skip_serializing_if = "String::is_empty")]
    pub card_id: String,
    /// The board row, `<deviceId>/<seq>` in the board log.
    pub row: String,
    /// One of [`row_kind`].
    pub kind: String,
    /// The row asked to wake the recipient.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub wake: bool,
}

impl BoardFrom {
    pub fn is_empty(&self) -> bool {
        self.board_id.is_empty()
    }
}

/// `board:<boardId>/<cardId>` for another person's agent,
/// `board:<boardId>/person:<personId>` for the person.
pub fn board_sender_key(board_id: &str, card_id: &str, person_id: &str) -> String {
    if card_id.is_empty() {
        format!("board:{board_id}/person:{person_id}")
    } else {
        format!("board:{board_id}/{card_id}")
    }
}

pub(crate) fn board_from_schema() -> Schema {
    let mut f = str_fields(&["boardId", "tag", "personId", "cardId", "row", "kind"]);
    f.push(("wake", Schema::Bool));
    Schema::object(f)
}

pub(crate) fn decode_board_from(o: &Out) -> BoardFrom {
    BoardFrom {
        board_id: s(o, "boardId"),
        tag: s(o, "tag"),
        person_id: s(o, "personId"),
        card_id: s(o, "cardId"),
        row: s(o, "row"),
        kind: s(o, "kind"),
        wake: b(o, "wake"),
    }
}

/// One board row to bridge, as the hosting Core read it from a board log.
#[derive(Debug, Clone, Default)]
pub struct BridgeRow {
    pub from: BoardFrom,
    /// How the sender shows: `blake` or `blake's agent mlg-lead`.
    pub name: String,
    /// The local agents the row is for: holders, watchers, the assignee.
    pub to: Vec<String>,
    pub text: String,
    /// The card, `task:<id>`, so it lands on the card's thread.
    pub ref_: String,
    /// The row's hop: its writer's running hop plus one.
    pub hop: i64,
    pub attachments: Vec<FileRef>,
}

/// One receive.set row (§7.2): a preset for a board, a person, both, or
/// neither (everything). `people` are the picked people under Members.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReceiveSetting {
    #[serde(rename = "boardId", skip_serializing_if = "String::is_empty")]
    pub board_id: String,
    #[serde(rename = "personId", skip_serializing_if = "String::is_empty")]
    pub person_id: String,
    /// One of [`receive`]; `""` clears the row back to what it inherits.
    pub preset: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<String>,
}

/// One autonomy.set row (§7.3): a preset for a board, or for every board.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AutonomySetting {
    #[serde(rename = "boardId", skip_serializing_if = "String::is_empty")]
    pub board_id: String,
    /// One of [`autonomy`]; `""` clears the row.
    pub preset: String,
}

#[derive(Serialize)]
struct ReceiveRow<'a> {
    #[serde(flatten)]
    s: &'a ReceiveSetting,
    by: &'a str,
    at: gocompat::time::GoTime,
}

#[derive(Serialize)]
struct AutonomyRow<'a> {
    #[serde(flatten)]
    s: &'a AutonomySetting,
    by: &'a str,
    at: gocompat::time::GoTime,
}

/// A folded setting row: the newest by HLC wins, in any arrival order.
#[derive(Debug, Clone)]
pub(crate) struct Setting<T> {
    pub hlc: Timestamp,
    pub dev: String,
    pub val: T,
}

fn newer<T>(cur: Option<&Setting<T>>, ev: &Event) -> bool {
    cur.is_none_or(|c| (ev.hlc, ev.device_id.as_str()) > (c.hlc, c.dev.as_str()))
}

/// Whether key runs with bypassed permissions (Grok and people never do):
/// the receive setting's Ask me and Closed hold other people's rows to
/// such a session (§7.2). Under the default Open nothing is held, and the
/// private wake policy has no bypass hold at all (decision 37).
pub(crate) fn bypass(key: &str, cards: &Cards) -> bool {
    cards
        .get(key)
        .is_some_and(|c| c.agent != "grok" && c.permission_mode == crate::BYPASS_MODE)
}

/// The receive table (§7.2) for one row to one recipient: the delivery
/// mode, the hold reason (`""` for none) and whether a wake request was
/// turned into next turn.
pub fn receive_decision(
    set: &ReceiveSetting,
    f: &BoardFrom,
    bypass_recipient: bool,
) -> (&'static str, &'static str, bool) {
    let mut preset = set.preset.as_str();
    if !receive::valid(preset) {
        preset = RECEIVE_DEFAULT;
    }
    if preset == receive::MEMBERS {
        // Picked people are let through as on Open, the rest asked first.
        preset = if set.people.contains(&f.person_id) {
            receive::OPEN
        } else {
            receive::ASK
        };
    }
    let wake_mode = if f.wake { mode::WAKE } else { mode::NEXT_TURN };
    let agent_assign = !f.card_id.is_empty() && f.kind == row_kind::ASSIGN;
    match preset {
        receive::CLOSED => {
            let h = if bypass_recipient {
                hold::CROSS_PERSON
            } else {
                ""
            };
            (mode::NEXT_TURN, h, f.wake)
        }
        receive::ASK => {
            let h = if bypass_recipient || agent_assign {
                hold::CROSS_PERSON
            } else if f.wake {
                hold::MEMBER_WAKE
            } else {
                ""
            };
            (wake_mode, h, false)
        }
        // Open: B2 lets rows through to bypass sessions too.
        _ => (wake_mode, "", false),
    }
}

/// The rate limit's sender for m: other people count per (person, board),
/// whichever of their agents wrote the row (§7.1).
pub(crate) fn rate_from(m: &Message) -> String {
    if m.from.kind == BOARD_KIND && !m.board.is_empty() && !m.board.person_id.is_empty() {
        return board_sender_key(&m.board.board_id, "", &m.board.person_id);
    }
    m.from.key.clone()
}

fn bridged_key(board_id: &str, row: &str, to: &str) -> String {
    format!("{board_id}/{row}>{to}")
}

impl Inner {
    /// The receive setting that applies to a row from person on board:
    /// (board, person), then board, then person, then everything, then
    /// Open.
    pub(crate) fn receive_for(&self, board_id: &str, person_id: &str) -> ReceiveSetting {
        for k in [
            (board_id, person_id),
            (board_id, ""),
            ("", person_id),
            ("", ""),
        ] {
            if let Some(r) = self.receive.get(&(k.0.to_string(), k.1.to_string()))
                && !r.val.preset.is_empty()
            {
                return r.val.clone();
            }
        }
        ReceiveSetting {
            board_id: board_id.into(),
            person_id: person_id.into(),
            preset: RECEIVE_DEFAULT.into(),
            people: vec![],
        }
    }

    pub(crate) fn autonomy_for(&self, board_id: &str) -> String {
        for k in [board_id, ""] {
            if let Some(r) = self.autonomy.get(k)
                && !r.val.preset.is_empty()
            {
                return r.val.preset.clone();
            }
        }
        AUTONOMY_DEFAULT.into()
    }

    /// The receive setting's hold for a bridged message (B2).
    pub(crate) fn receive_hold(&self, m: &Message, recipient: &str, cards: &Cards) -> &'static str {
        let set = self.receive_for(&m.board.board_id, &m.board.person_id);
        receive_decision(&set, &m.board, bypass(recipient, cards)).1
    }

    /// Folds a receive.set or autonomy.set row: the person's only.
    pub(crate) fn apply_setting(&mut self, ev: &Event) -> std::result::Result<(), String> {
        let schema = |extra: &[&'static str]| {
            let mut f = str_fields(&["boardId", "personId", "preset", "by"]);
            for e in extra {
                f.push((e, Schema::Slice(Box::new(Schema::String))));
            }
            f.push(("at", Schema::Time));
            Schema::object(f)
        };
        let receive_row = ev.kind == kinds::RECEIVE_SET;
        let o = json::unmarshal(
            &ev.payload,
            &schema(if receive_row { &["people"] } else { &[] }),
        )
        .map_err(|e| e.to_string())?;
        let _ = t(&o, "at");
        if ev.actor != "user" || !self.is_owner(&s(&o, "by")) {
            self.ignore(&ev.device_id);
            return Ok(());
        }
        if receive_row {
            let val = ReceiveSetting {
                board_id: s(&o, "boardId"),
                person_id: s(&o, "personId"),
                preset: s(&o, "preset"),
                people: strs(&o, "people").unwrap_or_default(),
            };
            let k = (val.board_id.clone(), val.person_id.clone());
            if newer(self.receive.get(&k), ev) {
                self.receive.insert(
                    k,
                    Setting {
                        hlc: ev.hlc,
                        dev: ev.device_id.clone(),
                        val,
                    },
                );
            }
        } else {
            let val = AutonomySetting {
                board_id: s(&o, "boardId"),
                preset: s(&o, "preset"),
            };
            let k = val.board_id.clone();
            if newer(self.autonomy.get(&k), ev) {
                self.autonomy.insert(
                    k,
                    Setting {
                        hlc: ev.hlc,
                        dev: ev.device_id.clone(),
                        val,
                    },
                );
            }
        }
        self.note(crate::ChangeKind::Updated, "", "");
        Ok(())
    }

    /// Notes a folded message.sent that came over the bridge.
    pub(crate) fn fold_bridged(&mut self, m: &Message) {
        if !m.board.is_empty() && !m.board.row.is_empty() {
            self.bridged
                .insert(bridged_key(&m.board.board_id, &m.board.row, &m.to.value));
        }
    }

    fn bridge_locked(&mut self, r: &BridgeRow) -> Result<Vec<Delivery>> {
        let f = &r.from;
        if trim(&f.board_id).is_empty() || trim(&f.row).is_empty() {
            return Err(Error::other("inbox: a bridged row names its board and row"));
        }
        if trim(&f.person_id).is_empty() {
            return Err(Error::other("inbox: a bridged row names its person"));
        }
        let text = trim(&r.text);
        if text.is_empty() {
            return Err(Error::sentinel(crate::ErrorKind::Empty));
        }
        let now = self.now();
        let cards = self.live_cards()?;
        let from = Sender {
            key: board_sender_key(&f.board_id, &f.card_id, &f.person_id),
            name: if trim(&r.name).is_empty() {
                "another person".into()
            } else {
                trim(&r.name).into()
            },
            kind: BOARD_KIND.into(),
        };
        let set = self.receive_for(&f.board_id, &f.person_id);
        let mut seen = BTreeSet::new();
        let mut out = vec![];
        for to in &r.to {
            let to = trim(to);
            // Only the device hosting the agent bridges (§7.1); the person
            // sees the board in the window.
            if to.is_empty()
                || device_of(to) != self.dev
                || to.starts_with("user:")
                || self.is_owner(to)
                || !seen.insert(to.to_string())
            {
                continue;
            }
            if self.bridged.contains(&bridged_key(&f.board_id, &f.row, to)) {
                continue;
            }
            let (mut md, _, asked) = receive_decision(&set, f, bypass(to, &cards));
            let mut reason = if asked {
                ASKED_TO_WAKE
            } else {
                f.kind.as_str()
            };
            // Over the per-(person, board) rate, a wake waits for next turn.
            let rate_key = pair_key(&board_sender_key(&f.board_id, "", &f.person_id), to);
            let win = tm::d(DIRECT_WINDOW);
            let kept = self.pairs.entry(rate_key).or_default();
            kept.retain(|at| tm::sub(&now, at) < win);
            if kept.len() >= DIRECT_LIMIT && md == mode::WAKE {
                md = mode::NEXT_TURN;
                reason = "rate limited";
            }
            let m = Message {
                id: format!("msg_{}", self.ids.next(&now)),
                from: from.clone(),
                to: Address::new(addr::AGENT, to),
                text: clip(text, MAX_TEXT),
                reason: reason.into(),
                hop: r.hop.max(0),
                trust: trust::UNTRUSTED.into(),
                sent_at: now,
                ref_: clip(&r.ref_, MAX_REF),
                board: f.clone(),
                attachments: r.attachments.clone(),
                ..Message::default()
            };
            self.emit(SYSTEM_KEY, kinds::MESSAGE_SENT, &m)?;
            let h = self.hold_id(&m.id, to, &cards);
            let row = placed(
                DeliveryRow {
                    delivery_id: format!("dlv_{}", self.ids.next(&now)),
                    message_id: m.id.clone(),
                    to: to.into(),
                    mode: md.into(),
                    ..DeliveryRow::default()
                },
                h,
            );
            let did = row.delivery_id.clone();
            self.put_delivery(row, SYSTEM_KEY)?;
            out.push(self.copy_id(&did));
        }
        Ok(out)
    }
}

impl Inbox {
    /// Bridges one board row to the local agents it is for (§7.1). Agents
    /// hosted on other devices are left to their host, and a row already
    /// bridged to an agent (by this device or another of the person's) is
    /// not bridged again. The receive setting picks each delivery's mode
    /// and hold; block list, hop limit and Pause all apply as to any
    /// other sender.
    pub fn bridge(&self, r: &BridgeRow) -> Result<Vec<Delivery>> {
        self.mutate(|b| b.bridge_locked(r))
    }

    /// Marks a bridged delivery answered without a private reply: the
    /// agent's answer went on the card in the board log (§7.1).
    pub fn answered_on_card(&self, agent: &str, did: &str, note: &str) -> Result<()> {
        self.mutate(|b| {
            let d = b.mine(agent, did)?;
            if d.d.state == crate::st::ANSWERED {
                return Ok(());
            }
            if !crate::st::open(&d.d.state) || d.d.state == crate::st::HELD {
                return Err(crate::messages::bad_state(d, "answer"));
            }
            let mut r = d.row();
            r.state = crate::st::ANSWERED.into();
            r.note = clip(note, crate::MAX_NOTE);
            r.until = gocompat::time::GoTime::ZERO;
            b.put_delivery(r, agent)
        })
    }

    /// Moves the bridged deliveries from board member `person_id` on board
    /// `board_id` that haven't reached their agent yet (queued or
    /// deferred) to held, so the person decides (v0.5 §4.3: they were
    /// removed or left). Returns how many moved.
    pub fn hold_from_member(&self, board_id: &str, person_id: &str) -> Result<usize> {
        self.mutate(|b| {
            let ids: Vec<String> = b
                .order
                .iter()
                .filter(|id| {
                    b.dels.get(*id).is_some_and(|d| {
                        matches!(d.d.state.as_str(), crate::st::QUEUED | crate::st::DEFERRED)
                            && b.msgs.get(&d.d.message_id).is_some_and(|m| {
                                m.board.board_id == board_id && m.board.person_id == person_id
                            })
                    })
                })
                .cloned()
                .collect();
            for id in &ids {
                let r = placed(b.dels[id].row(), hold::MEMBER_REMOVED);
                b.put_delivery(r, SYSTEM_KEY)?;
            }
            Ok(ids.len())
        })
    }

    /// Whether the board row was bridged to agent already.
    pub fn bridged(&self, board_id: &str, row: &str, agent: &str) -> bool {
        self.read(|b| b.bridged.contains(&bridged_key(board_id, row, agent)))
    }

    /// The hop a board row agent writes now carries: one more than the
    /// message its running turn woke on, 0 outside a woken turn (§7.1).
    pub fn current_hop(&self, agent: &str) -> i64 {
        self.read(|b| {
            b.running
                .get(agent)
                .and_then(|run| b.newest_in(&run.wake_id))
                .map_or(0, |m| m.hop + 1)
        })
    }

    /// The receive setting that applies to rows from person on board.
    pub fn receive_setting(&self, board_id: &str, person_id: &str) -> ReceiveSetting {
        self.read(|b| b.receive_for(board_id, person_id))
    }

    /// Every receive.set row in force, most general first.
    pub fn receive_settings(&self) -> Vec<ReceiveSetting> {
        self.read(|b| {
            b.receive
                .values()
                .filter(|r| !r.val.preset.is_empty())
                .map(|r| r.val.clone())
                .collect()
        })
    }

    /// Writes a receive.set row. The person only (by is an owner key): no
    /// tool changes it (§7.3).
    pub fn set_receive(&self, mut set: ReceiveSetting, by: &str) -> Result<()> {
        set.board_id = trim(&set.board_id).into();
        set.person_id = trim(&set.person_id).into();
        set.preset = trim(&set.preset).into();
        if !set.preset.is_empty() && !receive::valid(&set.preset) {
            return Err(Error::other(format!(
                "inbox: unknown receive setting {} (closed, ask, members or open)",
                quote(&set.preset)
            )));
        }
        let mut people: Vec<String> = vec![];
        for p in &set.people {
            let p = trim(p);
            if !p.is_empty() && !people.iter().any(|o| o == p) {
                people.push(p.into());
            }
        }
        set.people = people;
        self.mutate(|b| {
            if !b.is_owner(by) {
                return Err(Error::other(
                    "inbox: only the person changes the receive setting, in the app",
                ));
            }
            let at = b.now();
            b.emit(by, kinds::RECEIVE_SET, &ReceiveRow { s: &set, by, at })
        })
    }

    /// The autonomy preset for the person's agents on board.
    pub fn autonomy(&self, board_id: &str) -> String {
        self.read(|b| b.autonomy_for(board_id))
    }

    /// Every autonomy.set row in force.
    pub fn autonomy_settings(&self) -> Vec<AutonomySetting> {
        self.read(|b| {
            b.autonomy
                .values()
                .filter(|r| !r.val.preset.is_empty())
                .map(|r| r.val.clone())
                .collect()
        })
    }

    /// Writes an autonomy.set row; the person only.
    pub fn set_autonomy(&self, mut set: AutonomySetting, by: &str) -> Result<()> {
        set.board_id = trim(&set.board_id).into();
        set.preset = trim(&set.preset).into();
        if !set.preset.is_empty() && !autonomy::valid(&set.preset) {
            return Err(Error::other(format!(
                "inbox: unknown autonomy setting {} (ask, trusted or autonomous)",
                quote(&set.preset)
            )));
        }
        self.mutate(|b| {
            if !b.is_owner(by) {
                return Err(Error::other(
                    "inbox: only the person changes their agents' autonomy, in the app",
                ));
            }
            let at = b.now();
            b.emit(by, kinds::AUTONOMY_SET, &AutonomyRow { s: &set, by, at })
        })
    }
}
