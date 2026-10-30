//! The Board (I:internal/coord; coord.md): a fold over the replicated log
//! of `task.*`, `lease.*`, `blocker.*` and `take.*` rows.
//!
//! [`Coord`] holds the fold behind one lock. Every mutation checks, appends
//! its rows through the store and folds them before it returns; changes go
//! to subscribers when the lock is released. coord never routes a message:
//! calls someone should hear about return [`Notice`]s for the caller to
//! send (coord.md §1.1).
//!
//! What runs on a clock is a plain call here, reading the time from
//! `Options::now` (M4 wires the timers): [`Coord::handle`] is the Follow
//! loop's body, [`Coord::tick`] its 1 s holder duties, [`Coord::next_expiry`]
//! and [`Coord::expire`] the expiry timer, [`Coord::sweep_notices`] and
//! [`Coord::sweep_asks`] the Core's sweeps.

mod asks;
mod board_search;
mod cards;
mod comments;
mod error;
mod fold;
mod links;
mod mirror;
mod resource;
mod review;
pub mod rows;
mod short;
mod subtasks;
mod tasks;
mod types;
mod util;

use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use gocompat::sub::Queue;
use gocompat::time::GoTime;
use store::{Event, Store, kinds};

pub use asks::{
    ASK_UNCLAIMED, ASK_WAIT, MAX_OUTCOME, OUTCOME_ANSWERED, OUTCOME_CARD, OUTCOME_EDIT,
    OUTCOME_NONE, TAKE_CLAIM, TAKE_GONE, TAKE_POSTED, TAKE_RELEASE, TAKE_RELEASED, TAKE_REOPEN,
    TAKE_REOPENED, TAKE_TAKEN, TAKE_UNCLAIMED, ask_title,
};
pub use board_search::{DEFAULT_DOC_PAGE, SHOW_COMMENT_CUT, SHOW_COMMENTS};
pub use cards::{merge_watch, watch_text};
pub use error::{Error, ErrorKind, Result};
pub use fold::{
    FOLLOW_KINDS, Fold, LEFT_GONE, LEFT_REMOVED, REASON_CONFLICT, REASON_EXPIRED,
    REASON_HOLDER_GONE, REASON_RELEASED, REASON_STOLEN, SUB_BUFFER, followed,
};
pub use links::{MAX_LINKS, link_label, parse_link, pr_link};
pub use mirror::RELEASE_REQUEST;
pub use resource::{
    RESOURCE_NAMED, RESOURCE_PATH, RESOURCE_PORT, Resource, check_repo_key, normalize_glob,
    normalize_resource, overlaps, parse_resource,
};
pub use review::{NOTICE_REVIEW, ReviewRequest};
pub use short::{parse_ref, parse_short, short_suffix};
pub use types::*;

use crate::roster::{AgentCard, Agents};
use fold::Sub;

pub const DEFAULT_TTL: Duration = Duration::from_secs(30 * 60);
pub const MAX_TTL: Duration = Duration::from_secs(4 * 3600);
/// A holder gone this long releases its leases.
pub const GONE_GRACE: Duration = Duration::from_secs(3 * 60);
/// A holder gone this long is taken off its cards.
pub const HOLDER_GRACE: Duration = Duration::from_secs(10 * 60);

pub const MAX_COMMENT: usize = 4000;
pub const MAX_ATTACHMENTS: usize = 10;
pub const MAX_REASON: usize = 500;
pub const MAX_REF: usize = 300;
pub const MAX_KEY: usize = 200;
pub const DEFAULT_DONE_CARD: i64 = 20;
pub const MAX_DONE_CARD: i64 = 200;

pub const MAX_QUESTION: usize = 200;
pub const MAX_CONTEXT: usize = 800;
pub const MAX_ANSWER: usize = 200;
pub const MAX_WHY: usize = 400;
pub const MAX_OWN_ANSWER: usize = 800;
pub const MAX_OPTIONS: usize = 6;
pub const MAX_OPEN_BLOCKERS: usize = 8;
pub const KEEP_RESOLVED: usize = 10;
pub const MAX_STEPS: usize = 20;
pub const MAX_STEP_TEXT: usize = 200;
pub const MAX_ACTIVITY: usize = 500;
pub const MAX_MISSING: usize = 6;
pub const MAX_MISSING_TEXT: usize = 200;
pub const MAX_BLOCKER_LINKS: usize = 8;
/// Rows a board coord sets aside at most.
pub const MAX_ASIDE: usize = 2048;

pub(crate) const MAX_TITLE: usize = 200;
pub(crate) const MAX_OBJECTIVE: usize = 8000;
pub(crate) const MAX_SCOPE: usize = 64;

/// The state row's note when a card is reopened.
pub const REOPEN_NOTE: &str = "reopened";
pub const REOPEN_COMMENT_NOTE: &str = "reopened by the person's comment";

type KeyFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;
type StrFn = Arc<dyn Fn(&str) -> String + Send + Sync>;
/// [`Options::hop_of`].
pub type HopFn = Arc<dyn Fn(&str) -> i64 + Send + Sync>;
/// Whether a person owned the board at a row's place (`None`: now).
pub type OwnerAtFn = Arc<dyn Fn(&str, Option<&RowPos>) -> bool + Send + Sync>;

/// Configures a [`Coord`] (Go `coord.Options`).
#[derive(Clone, Default)]
pub struct Options {
    /// `None` is the system clock.
    pub now: Option<Arc<dyn Fn() -> GoTime + Send + Sync>>,
    /// Zero means [`GONE_GRACE`].
    pub gone_grace: Duration,
    /// Zero means [`HOLDER_GRACE`].
    pub holder_grace: Duration,
    /// The person's keys; `None` accepts exactly `user:<store device>` (on
    /// a board, exactly `person`).
    pub is_owner: Option<KeyFn>,
    /// A removed device: its leases end everywhere. `None` revokes nobody.
    pub revoked: Option<KeyFn>,
    /// Fold a shared board's log.
    pub board: bool,
    /// On a board, the person at this device's key.
    pub person: String,
    /// On a board, the member a `person:`/`agent:` key is.
    pub key_person: Option<StrFn>,
    /// On a board, the member a writing device is certified to.
    pub device_person: Option<StrFn>,
    /// A board's short handle, read as rows fold.
    pub tag: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    /// A person who owned the board just before the row at that place,
    /// or now when there is no row (the cancel rule reads the owners at
    /// the row's time, v0.5 §5.3).
    pub board_owner: Option<OwnerAtFn>,
    /// On a board, the hop an agent's row carries (v0.5 §7.1): its running
    /// turn's hop plus one, by the actor as the board knows it. The rows
    /// in [`HOP_KINDS`] get it as `hop` when it is above zero.
    pub hop_of: Option<HopFn>,
}

/// The board rows that carry their writer's hop across the bridge.
pub const HOP_KINDS: &[&str] = &[
    store::kinds::TASK_COMMENT,
    store::kinds::TASK_ASSIGNED,
    store::kinds::TASK_STATUS,
    store::kinds::BLOCKER_CLOSED,
    store::kinds::BLOCKER_OPTION,
    store::kinds::BLOCKER_RESOLVED,
];

/// Whether a board row of kind carries `hop` (board asks too, §9).
pub fn carries_hop(kind: &str) -> bool {
    HOP_KINDS.contains(&kind) || kind.starts_with("board.ask.")
}

/// A board row's `hop` (0 when it has none), for the bridge.
pub fn row_hop(payload: &[u8]) -> i64 {
    serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("hop").and_then(serde_json::Value::as_i64))
        .unwrap_or(0)
        .max(0)
}

/// The lock on the fold; changes go out when it drops (Go's `unlock`).
struct Locked<'a>(MutexGuard<'a, Fold>);

impl std::ops::Deref for Locked<'_> {
    type Target = Fold;
    fn deref(&self) -> &Fold {
        &self.0
    }
}

impl std::ops::DerefMut for Locked<'_> {
    fn deref_mut(&mut self) -> &mut Fold {
        &mut self.0
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        self.0.flush();
        if let Some(f) = &self.0.on_unlock {
            f();
        }
    }
}

/// Tasks, leases, comments and asks. Safe for concurrent use.
pub struct Coord {
    f: Mutex<Fold>,
    stale: Arc<RwLock<fold::StaleMap>>,
    board: bool,
}

impl Coord {
    /// Replays the store's log.
    pub fn new(st: Arc<Store>, roster: Option<Arc<dyn Agents>>, o: Options) -> Result<Coord> {
        let dev = st.device_id().to_string();
        let mut f = Fold::new(Some(st), &dev, roster, o);
        f.replay_store()?;
        Ok(Coord::wrap(f))
    }

    /// A read-only coord folded from `rows` in the order given (log order
    /// for a replay), as if this were device `device_id`. Writes fail.
    /// The replay goldens and the differential checks use it.
    pub fn from_rows(
        device_id: &str,
        rows: &[Event],
        roster: Option<Arc<dyn Agents>>,
        o: Options,
    ) -> Coord {
        let mut f = Fold::new(None, device_id, roster, o);
        f.replay_page(rows);
        f.end_replay();
        Coord::wrap(f)
    }

    /// Folds rows one at a time as they would arrive live (not a replay:
    /// short refs settle per row, links re-derive per row), with no duties
    /// run and no repeat check (Go's `impl.apply`). For checks that live
    /// and replay folds agree, and tests that hand the fold a row.
    pub fn fold_live(&self, rows: &[Event]) {
        let mut f = self.lock();
        for ev in rows {
            let last = f.last.entry(ev.device_id.clone()).or_default();
            *last = (*last).max(ev.seq);
            f.apply(ev);
        }
    }

    /// The board's set-aside gate alone (Go's `impl.asideGate`): `true`
    /// means the row would fold now. For tests of the cap.
    #[doc(hidden)]
    pub fn aside_gate(&self, ev: &Event) -> bool {
        self.lock().aside_gate(ev)
    }

    fn wrap(f: Fold) -> Coord {
        let stale = f.stale.clone();
        let board = f.on_board;
        Coord {
            f: Mutex::new(f),
            stale,
            board,
        }
    }

    fn lock(&self) -> Locked<'_> {
        Locked(self.f.lock().unwrap_or_else(|e| e.into_inner()))
    }

    // ---- tasks ----

    pub fn create_task(&self, by: &str, t: NewTask) -> Result<Task> {
        self.lock().create_task(by, t)
    }
    pub fn list_tasks(&self, f: &TaskFilter) -> Vec<Task> {
        self.lock().list_tasks(f)
    }
    pub fn get_task(&self, id: &str) -> Option<Task> {
        self.lock().tasks.get(id).cloned()
    }
    /// Makes `by` the holder; ErrHeld if another live holder has it.
    pub fn claim_task(&self, by: &str, id: &str) -> Result<Task> {
        self.lock().claim_task(by, id)
    }
    /// The v0.2 assign (the creator, a holder or a person).
    pub fn assign_task(&self, by: &str, id: &str, to: &str) -> Result<Task> {
        self.lock().assign(by, id, to, false).map(|(t, _)| t)
    }
    /// Assign with the wide rule, and the notice for `to`.
    pub fn assign(&self, by: &str, id: &str, to: &str) -> Result<(Task, Notice)> {
        self.lock().assign_notice(by, id, to)
    }
    pub fn set_task_state(&self, by: &str, id: &str, s: &str, note: &str) -> Result<Task> {
        self.lock().set_task_state(by, id, s, note)
    }
    pub fn reopen(&self, by: &str, id: &str) -> Result<(Task, Notice)> {
        self.lock().reopen(by, id, REOPEN_NOTE)
    }
    pub fn join(&self, by: &str, id: &str, who: &str) -> Result<Task> {
        self.lock().join(by, id, who)
    }
    pub fn leave(&self, by: &str, id: &str, who: &str) -> Result<Task> {
        self.lock().leave(by, id, who)
    }
    /// Takes board member `person_id`'s keys off every open card (v0.5
    /// §4.3): the remover's cleanup.
    pub fn take_off_person(&self, by: &str, person_id: &str) -> Result<usize> {
        self.lock().take_off_person(by, person_id)
    }
    pub fn set_shared(&self, by: &str, id: &str, shared: bool) -> Result<Task> {
        self.lock().set_shared(by, id, shared)
    }
    pub fn block(&self, by: &str, id: &str, reason: &str, bb: Option<BlockedBy>) -> Result<Task> {
        self.lock().block(by, id, reason, bb)
    }
    pub fn unblock(&self, by: &str, id: &str, note: &str) -> Result<(Task, Notice)> {
        self.lock().unblock(by, id, note)
    }
    /// Takes a card back off a shared board.
    pub fn withdraw(&self, by: &str, id: &str, note: &str) -> Result<Task> {
        self.lock().withdraw(by, id, note)
    }
    pub fn split(
        &self,
        by: &str,
        id: &str,
        subs: Vec<NewSubtask>,
        adopt: Vec<String>,
    ) -> Result<(Task, Vec<Task>)> {
        self.lock().split(by, id, subs, adopt)
    }
    /// Resolves `#12`, `12b`, `task:#12`, `mlg#12` or a full id.
    pub fn task_ref(&self, r: &str) -> Result<String> {
        self.lock().task_ref_locked(r)
    }
    pub fn is_person(&self, key: &str) -> bool {
        self.lock().is_owner(key)
    }
    pub fn ignored_rows(&self) -> usize {
        self.lock().ignored
    }
    pub fn ignored_by(&self) -> std::collections::BTreeMap<String, usize> {
        self.lock().ignored_by.clone()
    }
    pub fn aside_dropped(&self) -> usize {
        self.lock().aside_drop
    }
    /// What a board's fold kept out, for `GET /v1/boards?detail=1`.
    pub fn set_aside(&self) -> SetAside {
        let f = self.lock();
        SetAside {
            unresolved: f.aside.len(),
            dropped: f.aside_drop,
            ignored: f.ignored,
            ignored_by: f.ignored_by.clone(),
        }
    }
    pub fn unresolved(&self) -> usize {
        self.lock().aside.len()
    }
    pub fn is_board(&self) -> bool {
        self.board
    }

    // ---- cards ----

    pub fn raise(&self, by: &str, id: &str, nb: NewBlocker) -> Result<(Task, Blocker)> {
        self.lock().raise(by, id, nb)
    }
    pub fn recommend(
        &self,
        by: &str,
        id: &str,
        blocker: &str,
        answer: &str,
        why: &str,
    ) -> Result<Task> {
        self.lock().recommend(by, id, blocker, answer, why)
    }
    pub fn answer(
        &self,
        by: &str,
        id: &str,
        blocker: &str,
        a: BlockerAnswer,
    ) -> Result<(Task, Notice)> {
        self.lock().answer(by, id, blocker, a)
    }
    pub fn comment_on(&self, by: &str, id: &str, input: CommentInput) -> Result<(Comment, Notice)> {
        self.lock().comment_on(by, id, input)
    }
    pub fn set_steps(&self, by: &str, id: &str, steps: Vec<Step>) -> Result<Task> {
        self.lock().set_steps(by, id, steps)
    }
    pub fn set_step(&self, by: &str, id: &str, step: &str, u: StepUpdate) -> Result<Task> {
        self.lock().set_step(by, id, step, u)
    }
    pub fn watch(&self, by: &str, id: &str) -> Result<Task> {
        self.lock().watch(by, id, true)
    }
    pub fn unwatch(&self, by: &str, id: &str) -> Result<Task> {
        self.lock().watch(by, id, false)
    }
    /// A task's newest activity lines, newest first (`None`: no such task).
    pub fn activity(&self, id: &str) -> Option<Vec<Activity>> {
        self.lock().activity_of(id)
    }
    /// The person deletes a card (card #57); returns it as it was.
    pub fn delete_task(&self, by: &str, id: &str) -> Result<Task> {
        self.lock().delete_task(by, id)
    }
    pub fn pin(&self, by: &str, id: &str, pinned: bool) -> Result<Task> {
        self.lock().pin(by, id, pinned, "", false)
    }
    pub fn move_pin(&self, by: &str, id: &str, before: &str) -> Result<Task> {
        self.lock().pin(by, id, true, before, true)
    }

    // ---- comments ----

    pub fn add_comment(
        &self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
    ) -> Result<(Comment, Notice)> {
        self.lock().add_comment(by, id, text, wake, Vec::new())
    }
    pub fn add_comment_with(
        &self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
        files: Vec<FileRef>,
    ) -> Result<(Comment, Notice)> {
        self.lock().add_comment(by, id, text, wake, files)
    }
    /// The person's comment from the app. `around` runs under coord's lock.
    pub fn person_comment(
        &self,
        by: &str,
        id: &str,
        text: &str,
        wake: bool,
        around: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<(Comment, Vec<Notice>)> {
        self.lock().person_comment(by, id, text, wake, around)
    }
    pub fn record_message(&self, m: MessageEntry) -> Result<Comment> {
        self.lock().record_message(m)
    }
    pub fn comments(&self, id: &str) -> Option<Vec<Comment>> {
        self.lock().list_comments(id)
    }
    pub fn get_comment(&self, id: &str) -> Option<Comment> {
        self.lock().comments.get(id).cloned()
    }
    pub fn comment_attachment(&self, sha: &str) -> Option<FileRef> {
        self.lock().comment_attachment(sha)
    }

    // ---- links, leases ----

    pub fn add_link(&self, by: &str, id: &str, link: &str) -> Result<Task> {
        self.lock().add_link(by, id, link)
    }
    pub fn remove_link(&self, by: &str, id: &str, r: &str) -> Result<Task> {
        self.lock().remove_link(by, id, r)
    }
    /// Takes or renews a lease; ttl zero is DEFAULT_TTL, capped at MAX_TTL.
    pub fn acquire(
        &self,
        holder: &str,
        resource: &str,
        ttl: Duration,
        steal: bool,
    ) -> Result<Lease> {
        self.lock().acquire(holder, resource, ttl, steal)
    }
    pub fn release(&self, by: &str, resource: &str) -> Result<()> {
        self.lock().release(by, resource)
    }
    /// The person ending a lease held on any device.
    pub fn steal_lease(&self, by: &str, resource: &str) -> Result<()> {
        self.lock().steal_lease(by, resource)
    }
    pub fn leases(&self) -> Vec<Lease> {
        self.lock().leases()
    }
    /// The shared cards on this board that make card's lease on resource
    /// mirror here (v0.5 §5.4); `all` is the Autonomous preset's "every
    /// lease in shared tasks' repos".
    pub fn mirror_tasks(&self, card: &str, resource: &str, all: bool) -> Vec<String> {
        self.lock().mirror_tasks(card, resource, all)
    }
    /// Mirrors card's lease into this board when a shared card makes it
    /// mirror (`None`: nothing does). Another person's live lease refuses
    /// it, and the private lease must be refused with it.
    pub fn mirror_acquire(
        &self,
        card: &str,
        resource: &str,
        ttl: Duration,
        all: bool,
        steal: bool,
    ) -> Result<Option<Lease>> {
        self.lock().mirror_acquire(card, resource, ttl, all, steal)
    }
    /// Mirrors the release of card's lease; false when none is here.
    pub fn mirror_release(&self, card: &str, resource: &str) -> Result<bool> {
        self.lock().mirror_release(card, resource)
    }
    /// Asks another person's agents to review a PR the card links: `to`,
    /// or their dedicated reviewers (v0.5 §10). The notice goes to them.
    pub fn request_review(
        &self,
        by: &str,
        id: &str,
        pr: &str,
        to: &str,
    ) -> Result<(ReviewRequest, Notice)> {
        self.lock().request_review(by, id, pr, to)
    }
    /// The person dedicating one of their agents as reviewer for the board
    /// or one repo (empty reviewer: none).
    pub fn set_reviewer(&self, by: &str, reviewer: &str, repo: &str) -> Result<()> {
        self.lock().set_reviewer(by, reviewer, repo)
    }
    /// A card's review requests, oldest first.
    pub fn reviews(&self, id: &str) -> Vec<ReviewRequest> {
        self.lock().reviews.get(id).cloned().unwrap_or_default()
    }
    /// A member's dedicated reviewer for repo, else for the board.
    pub fn reviewer(&self, person: &str, repo: &str) -> String {
        self.lock().reviewer_of(person, repo)
    }
    /// Asks the holder of another person's lease to release it, on the
    /// shared card it mirrors through, in place of a steal.
    pub fn request_release(
        &self,
        by: &str,
        resource: &str,
        why: &str,
    ) -> Result<(Comment, Notice)> {
        let mut f = self.lock();
        let now = f.stamp();
        f.request_release(by, resource, why, &now)
    }
    /// Active refusals, newest first.
    pub fn refusals(&self) -> Vec<Refusal> {
        let mut f = self.lock();
        let now = f.stamp();
        f.prune(&now);
        f.refusals_list()
    }
    pub fn board(&self, filter: &BoardFilter) -> BoardView {
        self.lock().board(filter)
    }
    /// SweepNotices with the notices dropped.
    pub fn sweep(&self) -> Result<()> {
        self.sweep_notices().map(|_| ())
    }
    /// Releases gone holders' leases, returns cards whose holder went away,
    /// closes blockers that resolved by themselves, and drains the watch
    /// digests. Core calls it every few seconds and routes the notices.
    pub fn sweep_notices(&self) -> Result<Vec<Notice>> {
        let (out, r) = self.sweep_notices_partial();
        r.map(|_| out)
    }

    /// `sweep_notices` with what was built before a failed write kept:
    /// Core routes those notices (their rows are in the log) and reports
    /// the error, as Go's `SweepNotices` returns both.
    pub fn sweep_notices_partial(&self) -> (Vec<Notice>, Result<()>) {
        self.lock().sweep_notices()
    }

    // ---- asks ----

    pub fn post_ask(
        &self,
        by: &str,
        input: NewAsk,
        agents: &[AgentCard],
    ) -> Result<(Task, Notice)> {
        self.lock().post_ask(by, input, agents)
    }
    /// Asks for an ask's take and waits up to `wait` for the owner's answer.
    pub fn ask_claim(&self, by: &str, id: &str, wait: Duration) -> Result<ClaimResult> {
        let by = util::caller(by)?;
        let sub = self.subscribe();
        let req = {
            let mut f = self.lock();
            if f.is_owner(&by) || by == util::SWEEPER {
                return Err(error::invalid("an agent takes an ask"));
            }
            match f.ask_claim_start(&by, id)? {
                Ok(done) => return Ok(done),
                Err(i) => i,
            }
        };
        let deadline = Instant::now() + wait;
        loop {
            if let Some(r) = self.lock().ask_claim_poll(&by, id, req, false) {
                return r;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || !matches!(sub.recv_timeout(left), Ok(Some(_))) {
                break;
            }
        }
        self.lock()
            .ask_claim_poll(&by, id, req, true)
            .unwrap_or_else(|| Err(error::not_found(format!("task {id}"))))
    }
    pub fn ask_close(&self, by: &str, id: &str, o: AskOutcome) -> Result<Task> {
        self.lock().ask_close(by, id, o)
    }
    pub fn ask_again(&self, by: &str, id: &str) -> Result<()> {
        self.lock().ask_again(by, id)
    }
    pub fn sweep_asks(&self, agents: &[AgentCard]) -> Result<Vec<Notice>> {
        let (out, r) = self.sweep_asks_partial(agents);
        r.map(|_| out)
    }

    /// `sweep_asks` with the notices told before a failed write kept, as
    /// Go's `SweepAsks` returns both.
    pub fn sweep_asks_partial(&self, agents: &[AgentCard]) -> (Vec<Notice>, Result<()>) {
        self.lock().sweep_asks(agents)
    }
    /// The inbox's Stale hook: never takes coord's lock.
    pub fn stale_notice(&self, r: &str, to: &str) -> String {
        let Some(id) = r.strip_prefix("task:") else {
            return String::new();
        };
        let s = self.stale.read().unwrap_or_else(|e| e.into_inner());
        if s.closed.contains(id) {
            return "the ask closed".into();
        }
        match s.stale.get(id) {
            Some(t) if !t.is_empty() && t != to => format!("taken by {t}"),
            _ => String::new(),
        }
    }

    // ---- subscribe and the clock-driven parts ----

    /// A change after every mutation (buffer 64; a slow reader gets one
    /// resync). Dropping the queue unsubscribes.
    pub fn subscribe(&self) -> Arc<Queue<Change>> {
        let q = Arc::new(Queue::new(SUB_BUFFER));
        self.lock().subs.push(Sub {
            q: Arc::downgrade(&q),
            lost: false,
        });
        q
    }

    /// One row from the store's subscription (Go Follow's body).
    pub fn handle(&self, ev: &Event) -> Result<()> {
        let mut f = self.lock();
        if ev.is_resync() {
            return f.catch_up();
        }
        if ev.device_id == f.dev || !followed(&ev.kind) {
            if f.on_board {
                f.retry_aside();
            }
            return Ok(());
        }
        f.apply_remote(ev);
        Ok(())
    }

    /// Follow's 1 s tick: refusals for conflicts lost, expiry rows for own
    /// leases, and the ask owner's duties.
    pub fn tick(&self) {
        self.lock().holder_duties();
    }

    /// Rebuilds the fold when the log holds fewer of some device's rows
    /// than were folded (a revoke's cut moved them to refused_events).
    pub fn refold_if_cut(&self, vec: &store::Vector) -> Result<()> {
        self.lock().refold_if_cut(vec)
    }

    /// Whether a card's short ref moved at or after `since` (search
    /// re-reads those docs).
    pub fn short_moved_since(&self, id: &str, since: &GoTime) -> bool {
        self.lock().moved_since(id, since)
    }

    /// Re-reads other devices' rows after a store resync signal.
    pub fn catch_up(&self) -> Result<()> {
        self.lock().catch_up()
    }

    /// When the expiry timer should fire: the soonest Until of this
    /// device's leases while anyone is subscribed.
    pub fn next_expiry(&self) -> Option<GoTime> {
        self.lock().next_expiry()
    }

    /// Runs `f` after every unlock (Go's `unlock` re-arming its timer): the
    /// core's expiry timer re-reads [`Coord::next_expiry`] when it fires.
    /// `f` runs under the coord's lock, so it must only signal.
    pub fn set_on_unlock(&self, f: impl Fn() + Send + Sync + 'static) {
        self.lock().on_unlock = Some(Arc::new(f));
    }

    /// The expiry timer: prune, which notes each lease that ran out.
    pub fn expire(&self) {
        let mut f = self.lock();
        let now = f.stamp();
        f.prune(&now);
    }
}

impl Fold {
    /// Withdraw (boardlog.go): a board card canceled with withdrawn set.
    fn withdraw(&mut self, by: &str, id: &str, note: &str) -> Result<Task> {
        let by = crate::names::trim_space(by).to_string();
        let note = crate::names::trim_space(note).to_string();
        if !self.on_board {
            return Err(error::invalid(
                "only a card on a shared board is withdrawn; cancel a private card instead",
            ));
        }
        if util::runes(&note) > MAX_OBJECTIVE {
            return Err(error::invalid(format!(
                "note longer than {MAX_OBJECTIVE} characters"
            )));
        }
        let Some(t) = self.tasks.get(id) else {
            return Err(error::not_found(format!(
                "task {}",
                gocompat::quote::quote(id)
            )));
        };
        if t.withdrawn {
            return Ok(t.clone());
        }
        if !self.may_cancel(t, &self.dev, None) {
            return Err(error::not_allowed(format!(
                "only its creator or a board owner withdraws {}",
                t.short_ref()
            )));
        }
        let note = util::cmp_str(&note, "withdrawn from the board");
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_STATUS,
            &rows::TaskStateRow {
                task_id: id.into(),
                state: CANCELED.into(),
                note,
                withdrawn: true,
                by: by.clone(),
                at,
                ..rows::TaskStateRow::default()
            },
        )?;
        Ok(self.tasks[id].clone())
    }
}
