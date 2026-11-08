//! The person's mentions (card #77; board.md "Mentions"): the picker's
//! candidates, the keys the picker recorded checked against the roster,
//! and what each mention came to. The window never parses the text.

use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::board::repo_label;
use crate::cards::AgentRef;
use crate::ts;

/// The picker lists at most this many gone agents.
const MAX_GONE_CANDIDATES: usize = 20;

pub const WOKE: &str = "woke";
pub const HELD: &str = "held";
pub const GONE: &str = "gone";
pub const UNKNOWN: &str = "unknown";
pub const FAILED: &str = "failed";

/// One mention's outcome after a send.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct MentionOutcome {
    /// "" for an unknown key.
    pub key: String,
    pub name: String,
    /// "woke" | "held" | "gone" | "unknown" | "failed"
    pub state: String,
    /// The hold reason or the error.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub note: String,
}

/// One row of the mention picker.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct MentionCandidate {
    /// Name, status, live, where, device, colour.
    pub who: AgentRef,
    pub cwd: String,
    /// repo_label(repoKey), or "".
    pub repo: String,
    /// Set for another person on a shared board, or one of their agents:
    /// the board they can be mentioned on (card #177). The picker offers
    /// them only in a box on that board's cards.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub board: String,
    /// `assistant` for an Office Assistant (kind assistant and its owner's
    /// profile points at it); the picker offers one only in
    /// #office-assistants. "" for everything else.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub kind: String,
}

/// The person's mentions checked: what the log stores, and the outcome of
/// each one that can't be delivered.
#[derive(Debug, Clone, Default)]
pub(crate) struct Plan {
    /// As this device knows them (gone set).
    pub private: Vec<coord::Mention>,
    /// As the card's log writes them.
    pub log: Vec<coord::Mention>,
    /// Unknown keys.
    pub unknown: Vec<MentionOutcome>,
}

impl Plan {
    /// Every mention's outcome: `heard` holds the mention notice's results
    /// (private keys).
    pub(crate) fn outcomes(&self, heard: &[inbox::notice::NoticeResult]) -> Vec<MentionOutcome> {
        let mut out = Vec::new();
        for m in &self.private {
            let mut o = MentionOutcome {
                key: m.key.clone(),
                name: m.name.clone(),
                ..MentionOutcome::default()
            };
            if m.gone {
                o.state = GONE.into();
                o.note = format!("not delivered: {} is gone", m.name);
            } else {
                match heard.iter().find(|r| r.to == m.key) {
                    None => {
                        o.state = GONE.into();
                        o.note = format!("not delivered: {} is gone", m.name);
                    }
                    Some(r) => match &r.err {
                        Some(e) => {
                            o.state = FAILED.into();
                            o.note = e.to_string();
                        }
                        None if r.delivery.state == inbox::st::HELD => {
                            o.state = HELD.into();
                            o.note = inbox::watch::hold_reason(&r.delivery.hold);
                        }
                        None => o.state = WOKE.into(),
                    },
                }
            }
            out.push(o);
        }
        out.extend(self.unknown.iter().cloned());
        out
    }

    /// The outcomes of a message's copies (`skip`: the message's own
    /// recipient, who has it already).
    pub(crate) fn copy_outcomes(
        &self,
        rs: &[(inbox::Mention, inbox::Result<inbox::Delivery>)],
        skip: &str,
    ) -> Vec<MentionOutcome> {
        let mut out = Vec::new();
        for m in self.private.iter().filter(|m| m.key != skip) {
            let mut o = MentionOutcome {
                key: m.key.clone(),
                name: m.name.clone(),
                ..MentionOutcome::default()
            };
            match rs.iter().find(|(x, _)| x.key == m.key) {
                _ if m.gone => {
                    o.state = GONE.into();
                    o.note = format!("not delivered: {} is gone", m.name);
                }
                None => o.state = FAILED.into(),
                Some((_, Err(e))) => {
                    o.state = FAILED.into();
                    o.note = e.to_string();
                }
                Some((_, Ok(d))) if d.state == inbox::st::HELD => {
                    o.state = HELD.into();
                    o.note = inbox::watch::hold_reason(&d.hold);
                }
                Some(_) => o.state = WOKE.into(),
            }
            out.push(o);
        }
        out.extend(self.unknown.iter().cloned());
        out
    }

    /// The private mentions as the inbox stores them.
    pub(crate) fn inbox(&self) -> Vec<inbox::Mention> {
        self.private
            .iter()
            .map(|m| inbox::Mention {
                key: m.key.clone(),
                name: m.name.clone(),
                gone: m.gone,
            })
            .collect()
    }
}

impl Window {
    /// BoardService.MentionCandidates: every device's agents, live ones
    /// first (this device's, then the others', by name), then the last
    /// gone ones.
    pub fn mention_candidates(&self) -> Vec<MentionCandidate> {
        let p = self.people();
        let me = self.core.device_id().to_string();
        let mut live: Vec<(bool, String, MentionCandidate)> = Vec::new();
        let mut gone: Vec<(i64, MentionCandidate)> = Vec::new();
        for a in self.core.mesh_agents() {
            if a.key.is_empty() || a.key == "irori" || a.key.starts_with("user:") {
                continue;
            }
            let c = MentionCandidate {
                who: p.ref_(&a.key),
                cwd: a.cwd.clone(),
                repo: if a.repo_key.is_empty() {
                    String::new()
                } else {
                    repo_label(&a.repo_key)
                },
                board: String::new(),
                kind: String::new(),
            };
            if a.status == "gone" {
                gone.push((a.updated_at.unix_milli_value(), c));
            } else {
                live.push((a.device_id != me, a.name.to_lowercase(), c));
            }
        }
        live.sort_by(|x, y| (x.0, &x.1, &x.2.who.key).cmp(&(y.0, &y.1, &y.2.who.key)));
        gone.sort_by(|x, y| y.0.cmp(&x.0).then_with(|| x.1.who.key.cmp(&y.1.who.key)));
        let mut out: Vec<MentionCandidate> = live.into_iter().map(|(_, _, c)| c).collect();
        out.extend(gone.into_iter().take(MAX_GONE_CANDIDATES).map(|(_, c)| c));
        out.extend(self.board_candidates(&p));
        out
    }

    /// The other people on each shared board and their live agents, by
    /// name, each tagged with its board (card #177).
    fn board_candidates(&self, p: &crate::cards::People) -> Vec<MentionCandidate> {
        let me = self.core.person().id;
        let mut out: Vec<(String, String, MentionCandidate)> = Vec::new();
        for m in self.core.board_people() {
            out.push((
                m.board_id.clone(),
                format!("0{}", m.name.to_lowercase()),
                MentionCandidate {
                    who: p.ref_(&m.key),
                    board: m.board_id,
                    ..MentionCandidate::default()
                },
            ));
        }
        for a in self.core.board_agents() {
            if a.person_id.is_empty() || a.person_id == me || a.status == "gone" {
                continue;
            }
            out.push((
                a.board_id.clone(),
                format!("1{}", a.name.to_lowercase()),
                MentionCandidate {
                    who: p.ref_(&a.key),
                    board: a.board_id,
                    kind: if a.assistant {
                        "assistant".into()
                    } else {
                        String::new()
                    },
                    ..MentionCandidate::default()
                },
            ));
        }
        out.sort_by(|x, y| (&x.0, &x.1, &x.2.who.key).cmp(&(&y.0, &y.1, &y.2.who.key)));
        out.into_iter().map(|(_, _, c)| c).collect()
    }

    /// Whether `key` is another person, or one of their agents, on board
    /// `board`: delivered by their own devices (card #177).
    fn on_board(&self, board: &str, key: &str) -> Option<String> {
        if board.is_empty() || !coord::board_form(key) {
            return None;
        }
        let me = self.core.person().id;
        if let Some(m) = self
            .core
            .board_people()
            .into_iter()
            .find(|m| m.board_id == board && m.key == key)
        {
            return Some(m.name);
        }
        self.core
            .board_agents()
            .into_iter()
            .find(|a| a.board_id == board && a.key == key && a.person_id != me)
            .map(|a| a.name)
    }

    /// Checks the keys the picker recorded against every device's roster.
    /// `id`: the card they go on, whose log may want board keys.
    pub(crate) fn plan_mentions(&self, id: &str, given: &[ts::Mention]) -> Plan {
        let board = if id.is_empty() {
            String::new()
        } else {
            self.core.route(id).map(|(b, _)| b).unwrap_or_default()
        };
        self.plan_mentions_on(&board, given)
    }

    /// [`Self::plan_mentions`] for something new on board `board` ("" your
    /// own), such as a thread (card #187): the log gets board keys.
    pub(crate) fn plan_mentions_on(&self, board: &str, given: &[ts::Mention]) -> Plan {
        let roster = self.core.mesh_agents();
        let board = board.to_string();
        let user = self.core.user_sender().key;
        let mut plan = Plan::default();
        for g in given {
            let key = crate::trim(&g.key).to_string();
            if key.is_empty()
                || key == user
                || key == "irori"
                || key.starts_with("user:")
                || plan.private.iter().any(|m| m.key == key)
                || plan.unknown.iter().any(|m| m.key == key)
            {
                continue;
            }
            if let Some(known) = self.on_board(&board, &key) {
                // Another person or their agent: the board row carries it,
                // and their devices deliver it; this device has no outcome
                // for it.
                let name = if g.name.trim().is_empty() {
                    known
                } else {
                    g.name.trim().to_string()
                };
                plan.log.push(coord::Mention {
                    key: key.clone(),
                    name,
                    gone: false,
                });
                continue;
            }
            let Some(a) = roster.iter().find(|a| a.key == key) else {
                plan.unknown.push(MentionOutcome {
                    key: String::new(),
                    name: g.name.clone(),
                    state: UNKNOWN.into(),
                    note: format!("no agent has key {key}"),
                });
                continue;
            };
            let name = if g.name.trim().is_empty() {
                a.name.clone()
            } else {
                g.name.trim().to_string()
            };
            let mut m = coord::Mention {
                key: key.clone(),
                name,
                gone: a.status == "gone",
            };
            let mut log = m.clone();
            if !board.is_empty() {
                match self.core.board_key(&board, &key) {
                    Ok(k) => log.key = k,
                    Err(_) => {
                        m.gone = true;
                        log.gone = true;
                    }
                }
            }
            plan.private.push(m);
            plan.log.push(log);
        }
        plan
    }
}
