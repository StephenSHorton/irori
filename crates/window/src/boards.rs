//! Shared boards (I:boards_service.go): the Boards dialog's list and
//! actions, Share to board and Withdraw on a card's page, and the cards of
//! every board in the grid (design v0.5-shared-boards.md §8, step 4).

use coord::{Activity, Comment, Task};
use gocompat::time::GoTime;
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::cards::{CardBoard, CardDetail, CardSummary, SharedCopy};

/// A card on a shared board; its scope label is the board's name.
pub const SCOPE_BOARD: &str = "board";

/// One shared board in the list (Go `core.BoardSummary`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BoardSummary {
    pub board_id: String,
    pub name: String,
    pub tag: String,
    pub people: Vec<String>,
    /// The same members with their person ids, this person first (card
    /// #268): a card's owner chip and its holders' people come from them.
    pub members: Vec<SummaryMember>,
    pub owner: bool,
    pub devices: i64,
    pub cards: i64,
    /// Cards completed in the last seven days (card #126).
    pub done_week: i64,
    /// When anything last happened on the board; left out with no cards.
    #[serde(skip_serializing_if = "GoTime::is_zero")]
    #[specta(optional, type = crate::ts::Time)]
    pub last_at: GoTime,
    pub ready: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub tag_clash: bool,
    /// The person left and no other member holds the leave yet:
    /// "Leaving…", with Forget now (§4.4).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub leaving: bool,
    /// The board's warning lines (§8: only when something is wrong).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub warnings: Vec<String>,
    /// The person pinned it to the sidebar (card #135): a pin row in their
    /// own log, so it follows them to their devices; theirs alone.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub pinned: bool,
    /// Its place among the pinned boards, lowest first.
    #[serde(skip_serializing_if = "is_zero_rank")]
    #[specta(optional)]
    pub pin_rank: f64,
    /// The boards in this space (card #392, decision 46), its own first:
    /// a card's `board` names one of them (empty: the space's own).
    pub boards: Vec<SpaceBoard>,
}

/// One board of a space (card #392).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpaceBoard {
    /// Its id: the space's own id for its first board.
    pub board_id: String,
    pub name: String,
    pub tag: String,
}

/// One current member in a board's summary (card #268).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SummaryMember {
    pub person_id: String,
    pub name: String,
    /// The person at this device.
    pub me: bool,
    /// online, recent or offline on this board (card #371).
    pub presence: String,
    /// When they were last seen here; left out when unknown, and for this
    /// person.
    #[serde(skip_serializing_if = "GoTime::is_zero")]
    #[specta(optional, type = crate::ts::Time)]
    pub last_seen: GoTime,
    /// Their agents on the board that are working, and idle.
    pub working: i64,
    pub free: i64,
    /// Their colour (card #385): another person's as this screen places
    /// it (their pick, nudged off this person's devices and other people);
    /// this person's own pick, "" for none.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub color: String,
    /// Their avatar (card #393): a small image data URL, "" for none.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub avatar: String,
}

fn is_zero_rank(r: &f64) -> bool {
    *r == 0.0
}

/// The pin row key of board `id` in the person's own log (card #135):
/// beside card pins, in one rank space, never a card id.
pub fn board_pin_key(id: &str) -> String {
    format!("board:{id}")
}

impl From<irori_core::BoardSummary> for BoardSummary {
    fn from(s: irori_core::BoardSummary) -> BoardSummary {
        BoardSummary {
            board_id: s.board_id,
            name: s.name,
            tag: s.tag,
            people: s.people,
            members: s
                .members
                .into_iter()
                .map(|m| SummaryMember {
                    person_id: m.person_id,
                    name: m.name,
                    me: m.me,
                    presence: m.presence,
                    last_seen: m.last_seen,
                    working: m.working,
                    free: m.free,
                    color: m.color,
                    avatar: if coord::person_avatar::valid(&m.avatar) {
                        m.avatar
                    } else {
                        String::new()
                    },
                })
                .collect(),
            owner: s.owner,
            devices: s.devices,
            cards: s.cards,
            done_week: s.done_week,
            last_at: s.last_at,
            ready: s.ready,
            tag_clash: s.tag_clash,
            leaving: s.leaving,
            warnings: s.warnings,
            pinned: false,
            pin_rank: 0.0,
            boards: s
                .boards
                .into_iter()
                .map(|b| SpaceBoard {
                    board_id: b.board_id,
                    name: b.name,
                    tag: b.tag,
                })
                .collect(),
        }
    }
}

/// What sharing a card would put on a board (Go `core.SharePreview`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SharePreview {
    pub board_id: String,
    pub board_name: String,
    pub title: String,
    pub goal: String,
    pub done_when: String,
    pub steps: Vec<String>,
    pub links: Vec<String>,
    pub repo: String,
    pub visible_to: Vec<String>,
    pub dropped: Vec<String>,
    /// The open subtasks that go under it (card #180).
    pub subtasks: Vec<SubtaskPreview>,
    /// Its ended subtasks, which stay private: "#6 Old idea (canceled)".
    pub kept_private: Vec<String>,
    pub hash: String,
}

/// One subtask as Share would write it under its parent (card #180).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtaskPreview {
    pub task_id: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: String,
    pub goal: String,
    pub done_when: String,
    pub steps: Vec<String>,
    pub links: Vec<String>,
    pub repo: String,
}

impl From<irori_core::SubtaskPreview> for SubtaskPreview {
    fn from(s: irori_core::SubtaskPreview) -> SubtaskPreview {
        SubtaskPreview {
            task_id: s.task_id,
            reference: s.reference,
            title: s.title,
            goal: s.goal,
            done_when: s.done_when,
            steps: s.steps,
            links: s.links,
            repo: s.repo,
        }
    }
}

impl From<irori_core::SharePreview> for SharePreview {
    fn from(p: irori_core::SharePreview) -> SharePreview {
        SharePreview {
            board_id: p.board_id,
            board_name: p.board_name,
            title: p.title,
            goal: p.goal,
            done_when: p.done_when,
            steps: p.steps,
            links: p.links,
            repo: p.repo,
            visible_to: p.visible_to,
            dropped: p.dropped,
            subtasks: p.subtasks.into_iter().map(Into::into).collect(),
            kept_private: p.kept_private,
            hash: p.hash,
        }
    }
}

/// The boards:changed event.
pub const EVENT_BOARDS_CHANGED: &str = "boards:changed";

fn text(e: irori_core::BoardError) -> String {
    e.to_string()
}

impl Window {
    /// BoardsService.List: every board on this device, by name.
    pub fn boards_list(&self) -> Vec<BoardSummary> {
        let pins = self.core.coord().pinned_ranks();
        // Other people wear the colour this screen gives them (card #385),
        // the same one their face and agents wear on cards.
        let others = self.person_colors();
        self.core
            .board_list()
            .into_iter()
            .map(|s| {
                let mut b: BoardSummary = s.into();
                if let Some(r) = pins.get(&board_pin_key(&b.board_id)) {
                    b.pinned = true;
                    b.pin_rank = *r;
                }
                for m in b.members.iter_mut().filter(|m| !m.me) {
                    if let Some(c) = others.get(&m.person_id) {
                        m.color.clone_from(c);
                    }
                }
                b
            })
            .collect()
    }

    /// BoardsService.Pin: pins (or unpins) a board to the sidebar (card
    /// #135), in the person's own log. A board pin is no card, so the log
    /// notes no change for it: boards:changed is what tells every boards
    /// list (the sidebar's too) to read the pins again (#238).
    pub fn boards_pin(&self, id: &str, pinned: bool) -> Result<(), String> {
        let id = crate::trim(id);
        self.boards_find(id)?;
        self.core
            .coord()
            .pin_ref(&self.user_key(), &board_pin_key(id), pinned, "", false)
            .map_err(|e| e.to_string())?;
        self.emit(EVENT_BOARDS_CHANGED, &crate::events::Empty {});
        Ok(())
    }

    fn boards_find(&self, id: &str) -> Result<BoardSummary, String> {
        self.boards_list()
            .into_iter()
            .find(|b| b.board_id == id)
            .ok_or_else(|| format!("core: no such board on this device: {id}"))
    }

    /// BoardsService.Create: a board between this person's own devices,
    /// named `name` with the short tag its card refs carry (`mlg#12`).
    pub fn boards_create(&self, name: &str, tag: &str) -> Result<BoardSummary, String> {
        let id = self
            .core
            .create_board(name, &crate::trim(tag).to_lowercase())
            .map_err(text)?;
        self.boards_find(&id)
    }

    /// Adds board `name` (tag `tag`) to space `space_id` (card #392). Until
    /// every member device runs an Irori that reads spaces, it's refused
    /// with the devices it waits on.
    pub fn boards_add_to_space(
        &self,
        space_id: &str,
        name: &str,
        tag: &str,
    ) -> Result<BoardSummary, String> {
        self.core
            .create_space_board(space_id, name, &crate::trim(tag).to_lowercase())
            .map_err(text)?;
        self.boards_find(space_id)
    }

    /// Renames a board of space `space_id`, its first (the space's id) too
    /// (card #558): any member may, and the space keeps its own name.
    /// Refused, like adding one, until every member device reads spaces.
    pub fn boards_rename_in_space(
        &self,
        space_id: &str,
        board_id: &str,
        name: &str,
    ) -> Result<BoardSummary, String> {
        self.core
            .rename_space_board(space_id, board_id, name)
            .map_err(text)?;
        self.boards_find(space_id)
    }

    /// BoardsService.Rename (owners): the space's own name.
    pub fn boards_rename(&self, board_id: &str, name: &str) -> Result<BoardSummary, String> {
        self.core.rename_board(board_id, name).map_err(text)?;
        self.boards_find(board_id)
    }

    /// BoardsService.Leave: off the board on every one of the person's
    /// devices; with other people on it, after "Leaving…" (§4.4).
    pub fn boards_leave(&self, board_id: &str) -> Result<(), String> {
        self.core.leave_board(board_id).map_err(text)
    }

    /// BoardsService.Forget: dropped on this device only.
    pub fn boards_forget(&self, board_id: &str) -> Result<(), String> {
        self.core.forget_board(board_id).map_err(text)
    }

    /// BoardsService.SharePreview: exactly what Share would write (§5.2).
    pub fn boards_share_preview(
        &self,
        task_id: &str,
        board_id: &str,
    ) -> Result<SharePreview, String> {
        self.core
            .preview_share(task_id, board_id)
            .map(Into::into)
            .map_err(text)
    }

    /// BoardsService.Share: the private card goes to the board as the
    /// confirmed preview showed; returns the shared card's id.
    pub fn boards_share(
        &self,
        task_id: &str,
        board_id: &str,
        hash: &str,
    ) -> Result<String, String> {
        self.core
            .share_to_board(task_id, board_id, hash)
            .map(|t| t.id)
            .map_err(text)
    }

    /// BoardsService.Withdraw: the shared card comes off its board (§5.3);
    /// returns the private card the work carries on in, or "".
    pub fn boards_withdraw(&self, task_id: &str) -> Result<String, String> {
        self.core
            .withdraw_card(task_id)
            .map(|t| t.map(|t| t.id).unwrap_or_default())
            .map_err(text)
    }

    /// Card #432: the person offers shared card `task_id` to member `to`
    /// (a person id) when they own it, or asks its owner for it when they
    /// don't. Returns the Accept/Decline item it raised.
    pub fn boards_transfer_owner(&self, task_id: &str, to: &str) -> Result<String, String> {
        self.core
            .transfer_owner(task_id, to)
            .map(|b| b.id)
            .map_err(text)
    }

    /// A board card with its board keys as this device knows them (Go
    /// `privTask`).
    pub(crate) fn priv_task(&self, t: Task) -> Task {
        self.core.private_task(t)
    }

    pub(crate) fn priv_comments(&self, mut cs: Vec<Comment>) -> Vec<Comment> {
        for c in &mut cs {
            c.by = self.core.private_key(&c.by);
        }
        cs
    }

    pub(crate) fn priv_activity(&self, mut acts: Vec<Activity>) -> Vec<Activity> {
        for a in &mut acts {
            a.by = self.core.private_key(&a.by);
        }
        acts
    }

    /// Card #268: board `board_id`'s card `raw` (its board keys, before
    /// [`Self::priv_task`]) belongs to the member who made it, by person id:
    /// they made it or one of their agents did, and they answer for it.
    pub(crate) fn board_owner(&self, board_id: &str, raw: &Task, cs: &mut CardSummary) {
        // A card handed over since (card #432) belongs to its new owner.
        cs.owner = if raw.owner.is_empty() {
            self.core.board_key_person(board_id, &raw.created_by)
        } else {
            raw.owner.clone()
        };
    }

    /// A card page's board line, or where a private card was shared to
    /// (Go `boardDetail`).
    pub(crate) fn board_detail(&self, t: &Task, d: &mut CardDetail) {
        if let Some(s) = self.core.board_of(&t.id) {
            d.summary.scope = SCOPE_BOARD.into();
            d.summary.scope_label = s.name.clone();
            d.summary.board = s.board_id.clone();
            let (tag, space_board) = crate::cards::card_place(t, &s);
            d.summary.tag = tag.clone();
            d.summary.space_board = space_board.clone();
            // The fold takes a cancel only from the creator's person or an
            // owner (§5.3); with one person on the board that is always
            // them.
            let user = self.core.user_sender().key;
            d.on_board = Some(CardBoard {
                id: s.board_id,
                name: s.name,
                tag,
                space_board,
                visible_to: s.people,
                can_withdraw: !t.withdrawn
                    && (s.owner || t.created_by.starts_with("user:") || t.created_by == user),
                // Card #432: one change of owner at a time, on an open card
                // of a board with someone to hand it to.
                can_transfer: !t.withdrawn
                    && !coord::is_terminal(&t.state)
                    && s.members.len() > 1
                    && !t.blockers.iter().any(|b| b.open() && b.transfer),
            });
            return;
        }
        if let Some(sc) = self.core.shared_copy_of(&t.id) {
            d.shared_to = Some(SharedCopy {
                board_id: sc.board_id,
                board_name: sc.board_name,
                task_id: sc.task_id,
                ref_: sc.reference,
                withdrawn: sc.withdrawn,
            });
        }
    }
}

// ---- routing an action to the log a card lives in (I:board_route.go) ----------

impl Window {
    /// The coord the person's action on card `id` goes to, with the
    /// person's key first in the keys returned and then each of `more`, as
    /// that log knows them (Go `on`).
    pub(crate) fn on(
        &self,
        id: &str,
        more: &[&str],
    ) -> Result<(std::sync::Arc<coord::Coord>, Vec<String>), String> {
        let mut keys = vec![self.core.user_sender().key];
        keys.extend(more.iter().map(|k| k.to_string()));
        self.core.task_coord(id, &keys).map_err(|e| e.to_string())
    }

    /// The coord holding card `id`, for reads: no key is translated, so
    /// nothing is written (Go `read`).
    pub(crate) fn read(&self, id: &str) -> std::sync::Arc<coord::Coord> {
        match self.core.route(id) {
            Ok((_, co)) => co,
            Err(_) => self.core.coord().clone(),
        }
    }
}

// ---- a board's settings: members, what reaches my agents, what they may do ----

/// One member of a board, for the Boards dialog's Members and the Members
/// receive preset's people picker (§8).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BoardMember {
    pub person_id: String,
    pub name: String,
    pub owner: bool,
    /// The person at this device.
    pub me: bool,
    /// Their devices certified on the board.
    pub devices: i64,
    /// Their person fingerprint, as the invite screens read it out (the
    /// key the board pinned for them).
    pub fingerprint: String,
}

/// One of this person's agents dedicated as a reviewer on a board (§10).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BoardReviewer {
    /// The repoKey it reviews, or "" for the whole board.
    pub repo: String,
    /// The agent's session key.
    pub agent: String,
    pub name: String,
}

/// One of this person's live agents, to pick a reviewer from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewerChoice {
    pub agent: String,
    pub name: String,
    pub repo: String,
}

/// A board opened in the Boards dialog (§8): its members, the receive
/// preset that applies to other people's rows on it (§7.2) and the
/// autonomy preset for this person's agents there (§7.3).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BoardSettings {
    pub board_id: String,
    pub members: Vec<BoardMember>,
    /// closed, ask, members or open: what applies on this board.
    pub receive: String,
    /// Under members: the people whose rows wake agents.
    pub people: Vec<String>,
    /// The board has its own receive row (else it inherits).
    pub receive_own: bool,
    /// ask, trusted or autonomous: what applies on this board.
    pub autonomy: String,
    /// The board has its own autonomy row (else it inherits).
    pub autonomy_own: bool,
    /// shared or private: where this person's threads on the board live
    /// (card #185).
    pub threads: String,
    /// "Leaving…" (§4.4).
    pub leaving: bool,
    /// Owners only: each membership row the board ignored, in a line
    /// ("Stephen's role change for Blake didn't take: they are not on the
    /// board"), so an owner sees a promote or a removal that lost (§4.2).
    pub conflicts: Vec<String>,
    /// This person's dedicated reviewers on the board (§10), by repo.
    pub reviewers: Vec<BoardReviewer>,
    /// This person's live agents, to dedicate one as a reviewer.
    pub agents: Vec<ReviewerChoice>,
}

impl Window {
    /// BoardsService.Settings: one board's members and presets.
    pub fn boards_settings(&self, board_id: &str) -> Result<BoardSettings, String> {
        let info = self.core.board_info(board_id).map_err(text)?;
        let me = self.core.person().id;
        let mut members = Vec::new();
        for p in &info.members {
            if p.state != irori_identity::membership::MEMBER_ACTIVE {
                continue;
            }
            let m = self
                .core
                .board_member(board_id, &p.person_id)
                .map_err(text)?;
            members.push(BoardMember {
                person_id: p.person_id.clone(),
                name: if m.name.is_empty() {
                    p.name.clone()
                } else {
                    m.name
                },
                owner: p.role == irori_identity::membership::BOARD_OWNER,
                me: p.person_id == me,
                devices: m.devices.iter().filter(|d| !d.revoked).count() as i64,
                fingerprint: hex::decode(&p.pin.person_pub)
                    .map(|k| irori_pair::fingerprint(&k))
                    .unwrap_or_default(),
            });
        }
        members.sort_by_key(|m| !m.me);
        let owner = members.iter().any(|m| m.me && m.owner);
        let leaving = info
            .members
            .iter()
            .any(|p| p.person_id == me && p.state == irori_identity::membership::MEMBER_LEFT);
        let conflicts = if owner {
            info.conflicts
                .iter()
                .map(|c| conflict_line(c, &info.members, &me))
                .collect()
        } else {
            Vec::new()
        };
        let ib = self.core.inbox();
        let r = ib.receive_setting(board_id, "");
        Ok(BoardSettings {
            board_id: board_id.into(),
            members,
            receive: if r.preset.is_empty() {
                inbox::RECEIVE_DEFAULT.into()
            } else {
                r.preset
            },
            people: r.people,
            receive_own: ib
                .receive_settings()
                .iter()
                .any(|s| s.board_id == board_id && s.person_id.is_empty()),
            autonomy: ib.autonomy(board_id),
            autonomy_own: ib
                .autonomy_settings()
                .iter()
                .any(|s| s.board_id == board_id),
            threads: ib.threads_mode(board_id),
            leaving,
            conflicts,
            reviewers: self
                .core
                .board_reviewers(board_id)
                .into_iter()
                .map(|r| BoardReviewer {
                    repo: r.repo,
                    agent: r.agent,
                    name: r.name,
                })
                .collect(),
            agents: self.reviewer_choices(),
        })
    }

    /// BoardsService.RemoveMember (owners, §4.3).
    pub fn boards_remove_member(&self, board_id: &str, person_id: &str) -> Result<(), String> {
        self.core
            .remove_member(board_id, person_id, "")
            .map_err(text)?;
        Ok(())
    }

    /// BoardsService.SetRole (owners, §4.2): `owner` or `member`.
    pub fn boards_set_role(
        &self,
        board_id: &str,
        person_id: &str,
        role: &str,
    ) -> Result<(), String> {
        self.core
            .set_member_role(board_id, person_id, role)
            .map_err(text)?;
        Ok(())
    }

    /// BoardsService.ForgetNow: a board showing "Leaving…" is forgotten on
    /// every device of the person without waiting (§4.4).
    pub fn boards_forget_now(&self, board_id: &str) -> Result<(), String> {
        self.core.forget_left_board(board_id).map_err(text)?;
        Ok(())
    }

    /// The person's agents that may review: Claude sessions not gone.
    fn reviewer_choices(&self) -> Vec<ReviewerChoice> {
        let mut out: Vec<ReviewerChoice> = self
            .core
            .mesh_agents()
            .into_iter()
            .filter(|a| a.status != "gone" && a.agent != "grok")
            .map(|a| ReviewerChoice {
                name: if a.name.is_empty() {
                    a.key.clone()
                } else {
                    a.name.clone()
                },
                agent: a.key,
                repo: a.repo_key,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// BoardsService.SetReviewer: dedicates one of the person's agents as
    /// their reviewer on the board (§10), for one repo or ("" ) the whole
    /// board; agent "" takes the reviewer off.
    pub fn boards_set_reviewer(
        &self,
        board_id: &str,
        agent: &str,
        repo: &str,
    ) -> Result<BoardSettings, String> {
        self.core
            .set_board_reviewer(board_id, crate::trim(agent), crate::trim(repo))
            .map_err(text)?;
        self.boards_settings(board_id)
    }

    /// BoardsService.SetReceive: the receive preset for other people's rows
    /// on this board; "" goes back to what it inherits.
    pub fn boards_set_receive(
        &self,
        board_id: &str,
        preset: &str,
        people: Vec<String>,
    ) -> Result<BoardSettings, String> {
        self.core.board_info(board_id).map_err(text)?;
        self.core
            .inbox()
            .set_receive(
                inbox::ReceiveSetting {
                    board_id: board_id.into(),
                    preset: preset.into(),
                    people,
                    ..Default::default()
                },
                &self.core.user_sender().key,
            )
            .map_err(|e| e.to_string())?;
        self.boards_settings(board_id)
    }

    /// BoardsService.SetAutonomy: what this person's agents may do on this
    /// board without asking; "" goes back to what it inherits.
    pub fn boards_set_autonomy(
        &self,
        board_id: &str,
        preset: &str,
    ) -> Result<BoardSettings, String> {
        self.core.board_info(board_id).map_err(text)?;
        self.core
            .inbox()
            .set_autonomy(
                inbox::AutonomySetting {
                    board_id: board_id.into(),
                    preset: preset.into(),
                },
                &self.core.user_sender().key,
            )
            .map_err(|e| e.to_string())?;
        self.boards_settings(board_id)
    }

    /// BoardsService.SetThreads (card #185): this person's threads on the
    /// board are shared with every member or private to them and their own
    /// agents. Threads already started stay where they are.
    pub fn boards_set_threads(&self, board_id: &str, mode: &str) -> Result<BoardSettings, String> {
        self.core.board_info(board_id).map_err(text)?;
        self.core
            .inbox()
            .set_threads_mode(
                inbox::ThreadsSetting {
                    board_id: board_id.into(),
                    preset: mode.into(),
                },
                &self.core.user_sender().key,
            )
            .map_err(|e| e.to_string())?;
        self.boards_settings(board_id)
    }
}

/// One ignored membership row in the person's words.
fn conflict_line(
    c: &irori_identity::membership::Conflict,
    members: &[irori_identity::membership::BoardPerson],
    me: &str,
) -> String {
    let name = |id: &str| -> String {
        if id == me {
            return "your".into();
        }
        let n = members
            .iter()
            .find(|p| p.person_id == id)
            .map(|p| p.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "someone".into());
        format!("{n}'s")
    };
    let what = match c.kind.as_str() {
        store::kinds::BOARD_MEMBER_ROLE => "role change",
        store::kinds::BOARD_MEMBER_REMOVED => "removal",
        store::kinds::BOARD_MEMBER_ADDED => "invite",
        store::kinds::BOARD_MEMBER_LEFT => "leave",
        store::kinds::BOARD_RENAMED => "rename",
        store::kinds::BOARD_OPEN_CARDS => "open cards setting",
        _ => "change",
    };
    let by = if c.by.is_empty() {
        name(&c.person_id)
    } else {
        name(&c.by)
    };
    let of = if c.person_id.is_empty() || c.person_id == c.by {
        String::new()
    } else {
        let n = name(&c.person_id);
        format!(
            " for {}",
            n.strip_suffix("'s")
                .unwrap_or(if n == "your" { "you" } else { &n })
        )
    };
    let mut by = by;
    if let Some(f) = by.get(..1) {
        by = f.to_uppercase() + &by[1..];
    }
    format!("{by} {what}{of} didn't take: {}", c.reason)
}
