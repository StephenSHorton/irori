//! `irori-tools`: the one implementation of Irori's agent tools
//! (I:internal/tools; surface.md §2.8, §7.10).
//!
//! The Core serves them at `POST /v1/tools/{name}` and lists them at
//! `GET /v1/tools`; the mod registers them as `mcp__irori__<name>` and
//! `irori mcp` as `irori_<name>`. Both front ends are thin proxies, so the
//! names, schemas, descriptions and every result text live only here.
//!
//! The spec list is `specs.json`, a copy of Go's
//! `internal/mcpserver/fallback_tools.json` (39 tools in front-end order,
//! each with its clients); the golden test pins it against Go's
//! `Specs("")`.
//!
//! | Module | Go |
//! |---|---|
//! | [`Set`], dispatch, errors, names | `tools.go`, `set.go`, `route.go` |
//! | `format` | `format.go` |
//! | `args` | `set.go` `decode` |
//! | `observe` | `observe.go` (agents, events, [`redact`]) |
//! | `messages` | `messages.go` (send, reply, inbox, ack, defer, skip) |
//! | [`notice`] | `notice.go`, `cardthread.go` |
//! | `board` | `board.go` |
//! | `room` | `room.go` (ask_claim, ask_close) |
//! | `explore` | `explore.go` (overview, search, show, commits, prs) |
//! | `capturetools` | `capture.go` (timeline, why, agent resolution) |
//! | `attachments` | `attachments.go` |
//! | [`notify`] | `notify.go` |
//! | [`talk`] | (Rust only, card #389) talk_post, talk_thread (#631), talk_read, talk_list, talk_join, talk_leave |
//! | [`huddle`] (private) | (Rust only, card #1055) huddle_status, huddle_canvas, huddle_note |
//!
//! Wiring notes (for the merge):
//! - The Core implements [`Backend`] (Go's `core.Core` does). Optional
//!   parts default to off: [`Backend::capture`] (`None` answers "capture
//!   is off"), [`Backend::notifier`] (`None` is headless, as Go's nil
//!   `*Notifier`), [`Backend::asks`] (the ask book: the notify tool's
//!   buttons become ask rows), [`Backend::attacher`] (Go
//!   `core/attachments.go`, not yet in the Rust core: `None` refuses
//!   attach), and the shared-board hooks [`Backend::router`] /
//!   [`Backend::board_viewer`] (M9).
//! - [`notice`] is the one port of `notice.go`/`cardthread.go`;
//!   `irori-core::notice` re-exports it (plus the inbox's `Stale` hook).
//!   The Core's `ToolsView` implements [`Backend`] and `Core::tools` is
//!   the [`Set`].
//! - The API's `GET /v1/tools?client=` answers [`Set::specs_json`];
//!   `POST /v1/tools/{name}` calls [`Set::call`] with the [`Caller`] the
//!   attach or member secret resolved to.

mod args;
pub mod assistant_writes;
mod attachments;
mod avatar;
mod board;
mod capturetools;
mod devicefree;
mod explore;
mod format;
mod huddle;
mod later;
mod mention;
mod messages;
mod notes;
pub mod notice;
pub mod notify;
mod observe;
mod review_claim;
mod room;
mod shared;
mod slots;
mod specs;
mod started;
pub mod talk;
mod waits;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use gocompat::time::GoTime;

pub use attachments::Attacher;
pub use explore::spawned_lines;
pub use format::{clip, clip_mcp, one_line};
pub use notes::NotesBackend;
pub use notify::{Action, Note, Notifier};
pub use observe::redact;
pub use shared::{
    BoardInfo, CHOICE_CREATE, CHOICE_NO, CHOICE_REVIEW, CHOICE_SHARE, CHOICE_WITHDRAW,
    PERSONAL_DEFAULT, PersonInfo, PersonalBoard, REQUEST_WAIT, SharedBoards,
};
pub use specs::{
    ASK_INSTRUCTIONS, BOARD_INSTRUCTIONS, INSTRUCTIONS, Spec, compact_value, fallback_specs,
    parse_specs,
};

use crate::args::{Args, Ty};
use crate::format::{UNTRUSTED, trim};

/// Every tool result is capped at this many bytes (about 2k tokens).
pub const MAX_RESULT: usize = 8 << 10;

/// The client kind of the Irori Claude Code mod (`mcp__irori__<name>`).
pub const CLIENT_CLAUDE: &str = "claude";
/// The client kind of `irori mcp` (`irori_<name>`).
pub const CLIENT_GROK: &str = "grok";
/// The window's assistant (card #783): `irori mcp` run by a headless
/// `claude -p` for the corner chat, reading as the person. It lists and
/// may call only [`ASSISTANT_TOOLS`].
pub const CLIENT_ASSISTANT: &str = "assistant";

/// The tools the window's assistant may read with (card #783), all
/// read-only (its two writes are [`ASSISTANT_WRITE_TOOLS`], card #987):
/// the API refuses any other for its caller, and `GET
/// /v1/tools?client=assistant` lists exactly these. None of them marks
/// anything delivered or read (inbox and talk_* are left out for that).
pub const ASSISTANT_TOOLS: [&str; 15] = [
    "search",
    "show",
    "board",
    "overview",
    "agents",
    "agents_started",
    "commits",
    "prs",
    "notes_search",
    "timeline",
    "why",
    "leases",
    "waits",
    "events",
    // Card #1009: what the window is showing (the window sends it with each ask).
    "screen",
];

pub use assistant_writes::{ASSISTANT_WRITE_TOOLS, Grant, Grants, grants_for};

/// Who is calling, established by the transport (attach secret or member
/// secret), never by the arguments.
#[derive(Debug, Clone, Default)]
pub struct Caller {
    pub sender: inbox::Sender,
    /// `inbox::path::MOD` or `inbox::path::CHANNEL`.
    pub path: String,
}

/// Why a call was malformed (Go `Call`'s non-nil error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// Go `ErrUnknownTool` wrapped with the name.
    UnknownTool(String),
    /// Go `ErrNotObject`.
    NotObject,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::UnknownTool(n) => write!(f, "tools: no such tool: {}", format::q(n)),
            CallError::NotObject => f.write_str("tools: args must be a JSON object"),
        }
    }
}

impl std::error::Error for CallError {}

/// The log a card lives in (Go `TaskRouter`): a backend holding shared
/// boards (v0.5 §2.1) gives each card's coord and the caller's keys as
/// that log knows them.
pub trait TaskRouter: Send + Sync {
    fn task_coord(
        &self,
        task_id: &str,
        keys: &[String],
    ) -> Result<(Arc<coord::Coord>, Vec<String>), coord::Error>;
    fn private_notice(&self, n: coord::Notice) -> coord::Notice;
    fn task_ref(&self, r: &str) -> Result<String, coord::Error>;
    /// A key a shared log names as this device knows it (`agent:<cardId>`
    /// of one of this person's agents, their `person:<id>`); any other key
    /// as given. A read: it mints no card, unlike the keys
    /// [`Self::task_coord`] gives.
    fn private_key(&self, key: &str) -> String {
        key.to_string()
    }
    /// Mirrors an agent's lease into every shared board where it works a
    /// shared card in the resource's repo (v0.5 §5.4, `Coord::mirror_acquire`
    /// with its card there), before the private lease is taken. An error
    /// (another person's lease holds it) refuses the lease; `steal` takes
    /// only the leases of the caller's own person. Returns the
    /// tags of the boards it went to.
    fn mirror_acquire(
        &self,
        key: &str,
        resource: &str,
        ttl: std::time::Duration,
        steal: bool,
    ) -> Result<Vec<String>, coord::Error> {
        let _ = (key, resource, ttl, steal);
        Ok(Vec::new())
    }
    /// Mirrors the release of an agent's lease wherever it was mirrored.
    fn mirror_release(&self, key: &str, resource: &str) {
        let _ = (key, resource);
    }
    /// Asks the holder of another person's lease on resource to release it
    /// (v0.5 §5.4, `Coord::request_release`): a comment by `by` on the
    /// shared card the lease mirrors through. Returns the board's id, the
    /// comment and its notice. Without shared boards there is none to ask.
    fn request_release(
        &self,
        by: &str,
        resource: &str,
        why: &str,
    ) -> Result<(String, coord::Comment, coord::Notice), coord::Error> {
        let _ = (by, why);
        Err(coord::Error::new(
            coord::ErrorKind::NotFound,
            format_args!("no shared space holds a lease on {resource}"),
        ))
    }
    /// `review_claim` (merge desk §12.3): the one claim per PR. The Core
    /// finds the PR's card on a shared board the caller's person belongs
    /// to, checks who may claim, and writes the `pr/` lease straight to
    /// that board's log. An `Err` is a refusal, in fixed words.
    fn review_claim(
        &self,
        key: &str,
        pr: &str,
        action: &str,
        reason: &str,
    ) -> Result<ClaimAnswer, String> {
        let _ = (key, pr, action, reason);
        Err("review claims need a shared space with a card for this PR.".into())
    }
    /// Where each PR's review stands in `repo` (`owner/name`) on the
    /// shared boards (merge desk §12.9, card #1080): who claims it,
    /// who is related, GitHub's review state and why nothing is picked.
    /// Needs no capture.
    fn pr_reviews(&self, repo: &str) -> Vec<PrReviewLine> {
        let _ = repo;
        Vec::new()
    }
    /// The shared board's coord holding comment `id`, or `None` when no
    /// shared board has it (card #189: show reads a shared card's comments
    /// from its board's log, which capture doesn't index).
    fn comment_coord(&self, id: &str) -> Option<Arc<coord::Coord>> {
        let _ = id;
        None
    }
}

/// One PR's review as `prs` words it (merge desk §12.9).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrReviewLine {
    pub number: i64,
    pub url: String,
    /// `none`, `off`, `waiting`, `asked`, `claimed` or `reviewed`.
    pub pick: String,
    /// GitHub's review decision, "" none.
    pub review: String,
    /// The agent asked, while `pick` is `asked`.
    pub asked: String,
    /// `agent (person)`, "" none.
    pub claimer: String,
    /// When the claim lapses, RFC 3339.
    pub until: String,
    /// `agent (person): why` for each related agent.
    pub related: Vec<String>,
    pub why_none: String,
}

/// What a `review_claim` came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// The caller holds the claim (taken now, or renewed).
    Claimed,
    /// Someone else holds it: comment on the PR instead.
    Held,
    /// `confirm`: the caller still holds it.
    Yours,
    /// `confirm`: the caller does not hold it.
    NotYours,
    Released,
    /// `release`: it held none.
    NotHeld,
    Declined,
}

/// `review_claim`'s answer, as fields: the tool words it in fixed text and
/// never quotes a card or PR title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimAnswer {
    pub outcome: ClaimOutcome,
    /// The PR's full URL, as its card names it.
    pub pr: String,
    /// The PR card's short ref.
    pub card: String,
    /// The holder's name, clipped, for `Held`; empty otherwise.
    pub holder: String,
    /// When the claim lapses unless renewed, RFC 3339; empty if none.
    pub until: String,
    /// The On go-ahead (§12.8): the setting is on and the caller's person
    /// already works in this repo in that space.
    pub go_ahead: bool,
    /// `confirm`: the caller's own person signed off (the Off text).
    pub signed_off: bool,
}

/// Another member's agent on a shared board ([`Backend::board_agents`]).
#[derive(Debug, Clone, Default)]
pub struct BoardAgent {
    /// The board's tag, or its name.
    pub board: String,
    /// Its person's name on the board, "" when unknown.
    pub person: String,
    pub card: presence::AgentCard,
    /// Its owner's Office Assistant, verified (kind `assistant` and the
    /// owner's profile points at it): the chosen name, "" for the default.
    pub assistant: Option<String>,
}

/// "My agents may start agents on my cards", as an agent may read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentLimit {
    pub on: bool,
    pub max: i64,
    pub used: i64,
}

/// Pause agents (card #665), as an agent may read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentsPaused {
    /// The device's name for people; empty when it has none.
    pub device: String,
    pub on: bool,
    /// When it went on, RFC 3339; empty when off.
    pub since: String,
    /// Who set it last, for people: "the person" or an agent's name.
    pub by: String,
}

/// Shared boards to show (Go `BoardViewer`).
pub trait BoardViewer: Send + Sync {
    fn board_views(&self, f: &coord::BoardFilter) -> Vec<coord::BoardView>;
}

/// A huddle as the Core sees it now (card #1055).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HuddleNow {
    /// The huddle is over (call-agents.md §8): no more is being said.
    pub over: bool,
    /// The people in it now, by key in this device's form.
    pub people: Vec<String>,
    /// Each person's assistant: the person's key and its state
    /// (`connected`, `reconnecting`, `not connected`).
    pub assistants: Vec<(String, String)>,
}

/// What the tools act on (Go `Backend`); the Core provides it.
pub trait Backend: Send + Sync {
    /// This device's id (`deviceId` in `irori.json`); empty when the backend
    /// has none, which turns heavy slots off.
    fn device_id(&self) -> String {
        String::new()
    }
    /// This device's free room for the folder `path` (empty: Irori's data
    /// dir), read now; `None` when this backend can't read it.
    fn device_free(&self, _path: &str) -> Option<DeviceFree> {
        None
    }
    /// This device's roster (presence plus Grok members).
    fn agents(&self) -> Vec<presence::AgentCard>;
    /// Whether this device's session `key` has a live attach stream
    /// (GitHub #57); `None` when this backend can't tell.
    fn attached(&self, _key: &str) -> Option<bool> {
        None
    }
    /// The agents Irori hands an unheld card to (#82): every device's, as
    /// asks go out, without this device's while Pause all is on. The
    /// default is [`Backend::agents`].
    fn room_agents(&self) -> Vec<coord::roster::AgentCard> {
        self.agents()
            .into_iter()
            .map(|a| coord::roster::AgentCard {
                key: a.key,
                device_id: a.device_id,
                status: a.status,
                repo_key: a.repo_key,
                device_state: a.device_state,
                updated_at: a.updated_at,
                last_activity: a.last_activity,
                ..coord::roster::AgentCard::default()
            })
            .collect()
    }
    /// Other members' agents on the shared boards this device shows, with
    /// their devices' presence (card #372). Default: none.
    fn board_agents(&self) -> Vec<BoardAgent> {
        Vec::new()
    }
    fn events(&self, f: &store::Filter) -> Result<Vec<store::Event>, String>;
    /// What the window last said it shows (card #1009): the plain-text
    /// snapshot the assistant's `screen` tool returns; empty when none.
    fn assistant_screen(&self) -> String {
        String::new()
    }
    fn inbox(&self) -> &inbox::Inbox;
    fn coord(&self) -> &Arc<coord::Coord>;
    /// Every agent key → the last name it went by, gone agents included
    /// (card #98); empty when the backend keeps none.
    fn known_names(&self) -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::new()
    }
    /// What each of the person's devices is called, by device id (card
    /// #485: a version wait names devices); empty when the backend keeps
    /// none.
    fn device_labels(&self) -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::new()
    }
    /// What card `task_id` waits on for an agent (card #574), one line, or
    /// `""` when it doesn't. `asked`: the caller just asked for one, so
    /// it's said even where the board's list would leave it out.
    fn spawn_wait(&self, _task_id: &str, _asked: bool) -> String {
        String::new()
    }
    /// Asks GitHub now about each PR card `task_id` (in `co`) links that is
    /// last heard open, and records a merge or a closing (card #970):
    /// agent `caller` is setting it completed, likely just after its own
    /// `gh pr merge`, which the poll may not have heard yet. Default: none.
    fn recheck_prs(&self, _co: &Arc<coord::Coord>, _caller: &str, _task_id: &str) {}
    /// Asks for an agent on card `task_id` for agent `caller` (card #575):
    /// what happens next, or why not.
    fn agent_start(&self, _caller: &str, _task_id: &str) -> Result<String, String> {
        Err("agents can't start agents here".into())
    }
    /// What `agent_start` would refuse before anything is made, for a
    /// thread `caller` is about to open for a fresh agent (card #1027).
    fn thread_agent_check(&self, _caller: &str) -> Result<(), String> {
        Err("agents can't start agents here".into())
    }
    /// Gives thread `task_id` that `caller` opened to a fresh agent, gated
    /// as `agent_start` is (card #1027): what happens next, or why not.
    fn thread_new_agent(
        &self,
        _caller: &str,
        _task_id: &str,
        _watch: bool,
    ) -> Result<String, String> {
        Err("agents can't start agents here".into())
    }
    /// Ends agent `key` for agent `caller`, with its hand-off: the card it
    /// was on, or why not.
    fn agent_end(&self, _caller: &str, _key: &str) -> Result<String, String> {
        Err("agents can't end agents here".into())
    }
    /// Pause agents (card #665): agent `caller` turns it on or off for its
    /// own device; what it says, or why not.
    fn agents_pause(&self, _caller: &str, _on: bool) -> Result<String, String> {
        Err("agents can't pause agents here".into())
    }
    /// Whether Pause agents is on on agent `caller`'s device; `None` when
    /// the backend keeps none.
    fn agents_paused(&self, _caller: &str) -> Option<AgentsPaused> {
        None
    }
    /// The person's limit on their agents' starts. There is no setter here,
    /// and none may be added: only the app changes it.
    fn agent_limit(&self) -> Option<AgentLimit> {
        None
    }
    /// Capture serves timeline, why, search, show, overview, commits and
    /// prs; `None` makes them answer "capture is off".
    fn capture(&self) -> Option<&capture::Capture> {
        None
    }
    /// Shows the notify tool's cards; `None` is a headless Core.
    fn notifier(&self) -> Option<&dyn Notifier> {
        None
    }
    /// The ask book, when the backend keeps asks as rows.
    fn asks(&self) -> Option<&Arc<coord::approvals::Book>> {
        None
    }
    fn attacher(&self) -> Option<&dyn Attacher> {
        None
    }
    fn router(&self) -> Option<&dyn TaskRouter> {
        None
    }
    fn board_viewer(&self) -> Option<&dyn BoardViewer> {
        None
    }
    /// Shared boards with other people (v0.5 §7.3); `None`: private only.
    fn shared_boards(&self) -> Option<&dyn SharedBoards> {
        None
    }
    /// Personal's boards (card #558), its default first (card #698: an
    /// agent's board_create and board_share take one by its tag). Empty:
    /// Personal has only its default.
    fn personal_boards(&self) -> Vec<PersonalBoard> {
        Vec::new()
    }
    /// Talk's channels, DMs and read cursors (card #389); `None`: the
    /// talk_* tools answer that Talk is off.
    fn talk(&self) -> Option<&Arc<coord::talk::Talk>> {
        None
    }
    /// What only the Core knows of huddle `call` now (card #1055): whether
    /// it is over, who is in it and whose assistant is connected. `None`:
    /// not known here; the tools then read the huddle's end mark.
    fn huddle_now(&self, _call: &str) -> Option<HuddleNow> {
        None
    }
    /// The face kept for an agent name (card #407), "" for none.
    fn avatar(&self, _name: &str) -> String {
        String::new()
    }
    /// Keeps (or with `None` clears) an agent name's face.
    fn set_avatar(&self, _name: &str, _face: Option<&coord::avatar::Avatar>) -> Result<(), String> {
        Err("faces aren't kept here".into())
    }
    /// The notes API (card #412); `None` answers "notes aren't available".
    fn notes(&self) -> Option<&dyn NotesBackend> {
        None
    }
}

/// The clock the tools read.
pub type NowFn = Arc<dyn Fn() -> GoTime + Send + Sync>;

type Handler = fn(&Inner, &Caller, &[u8]) -> String;

/// The tool registry (Go `Set`).
pub struct Set {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) b: Arc<dyn Backend>,
    pub(crate) now: NowFn,
    specs: Vec<Spec>,
    handlers: HashMap<&'static str, Handler>,
    needs_caller: HashMap<String, bool>,
    pub(crate) notify_state: Mutex<notify::NotifyState>,
    /// Board requests waiting on the person, by ask id.
    pub(crate) board_requests: Mutex<shared::Requests>,
}

pub use devicefree::DeviceFree;

/// Heavy slots per device (device-free plan §2): advisory, fixed at two.
pub const HEAVY_SLOTS: usize = 2;

/// The name of this device's heavy family, `heavy/<first 8 hex of the
/// device id>`; `None` when the id is unknown.
pub fn heavy_family(device_id: &str) -> Option<String> {
    let id: String = device_id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    (id.len() == 8).then(|| format!("heavy/{id}"))
}

/// The read-only tools that read as the caller.
const CALLER_READS: [&str; 10] = [
    "huddle_status",
    "huddle_canvas",
    "inbox",
    "timeline",
    "overview",
    "search",
    "show",
    "board",
    "talk_read",
    "talk_list",
];

fn handlers() -> HashMap<&'static str, Handler> {
    let list: [(&'static str, Handler); 70] = [
        ("avatar", avatar::avatar),
        ("agent_start", started::agent_start),
        ("agents_started", started::agents_started),
        ("agent_end", started::agent_end),
        ("agents_pause", started::agents_pause),
        ("agents", observe::agents),
        ("events", observe::events),
        ("screen", observe::screen),
        ("send", messages::send),
        ("reply", messages::reply),
        ("inbox", messages::inbox),
        ("ack", messages::ack),
        ("defer", messages::defer),
        ("skip", messages::skip),
        ("overview", explore::overview),
        ("search", explore::search),
        ("show", explore::show),
        ("board", board::board),
        ("board_create", board::board_create),
        ("board_split", board::board_split),
        ("board_comment", board::board_comment),
        ("board_block", board::board_block),
        ("board_unblock", board::board_unblock),
        ("board_join", board::board_join),
        ("board_leave", board::board_leave),
        ("board_move", board::board_move),
        ("board_assign", board::board_assign),
        ("board_status", board::board_status),
        ("assistant_close", assistant_writes::assistant_close),
        ("assistant_tick", assistant_writes::assistant_tick),
        ("board_link", board::board_link),
        ("board_edit", board::board_edit),
        ("board_later", later::board_later),
        ("board_steps", board::board_steps),
        ("board_step", board::board_step),
        ("board_recommend", board::board_recommend),
        ("board_watch", board::board_watch),
        ("board_unwatch", board::board_unwatch),
        ("board_share", shared::board_share),
        ("board_review", shared::board_review),
        ("ask_claim", room::ask_claim),
        ("ask_close", room::ask_close),
        ("commits", explore::commits),
        ("prs", explore::prs),
        ("lease", board::lease),
        ("leases", board::leases),
        ("device_free", devicefree::device_free),
        ("release", board::release),
        ("slot_take", slots::slot_take),
        ("slot_release", slots::slot_release),
        ("review_claim", review_claim::review_claim),
        ("timeline", capturetools::timeline),
        ("why", capturetools::why),
        ("join", join),
        ("notify", notify::notify),
        ("attachment_get", attachments::attachment_get),
        ("wait", waits::wait),
        ("waits", waits::waits),
        ("wait_cancel", waits::wait_cancel),
        ("talk_post", talk::talk_post),
        ("talk_thread", talk::talk_thread),
        ("talk_read", talk::talk_read),
        ("talk_list", talk::talk_list),
        ("talk_join", talk::talk_join),
        ("talk_leave", talk::talk_leave),
        ("huddle_status", huddle::huddle_status),
        ("huddle_canvas", huddle::huddle_canvas),
        ("huddle_note", huddle::huddle_note),
        ("notes_propose", notes::notes_propose),
        ("notes_search", notes::notes_search),
    ];
    list.into_iter().collect()
}

impl Set {
    /// Builds the registry over `b`, reading the clock `now` (`None` is
    /// local time now).
    pub fn new(b: Arc<dyn Backend>, now: Option<NowFn>) -> Set {
        let now = now.unwrap_or_else(|| Arc::new(presence::now_local));
        let specs = specs::all();
        let handlers = handlers();
        let mut needs_caller = HashMap::new();
        for sp in &specs {
            assert!(
                handlers.contains_key(sp.name.as_str()),
                "tools: no handler for {}",
                sp.name
            );
            // Reads for the caller: inbox and timeline show its messages;
            // board groups "needs you" for it; overview, search and show
            // pass it as the Viewer, so they must never run as nobody.
            if !sp.read_only || CALLER_READS.contains(&sp.name.as_str()) {
                needs_caller.insert(sp.name.clone(), true);
            }
        }
        needs_caller.remove("join");
        let inner = Arc::new(Inner {
            b,
            now,
            specs,
            handlers,
            needs_caller,
            notify_state: Mutex::new(notify::NotifyState::default()),
            board_requests: Mutex::new(shared::Requests::default()),
        });
        // The notify tool's asks are rows when the backend keeps them (v0.4
        // §6.7): the answer comes back from whichever device the person used.
        if let Some(book) = inner.b.asks() {
            let weak: Weak<Inner> = Arc::downgrade(&inner);
            book.on_resolved(move |a| {
                if let Some(x) = weak.upgrade()
                    && !shared::request_resolved(&x, a)
                {
                    notify::ask_resolved(&x, a);
                }
            });
        }
        Set { inner }
    }

    /// The tools for a client kind (`"claude"` or `"grok"`); `""` lists
    /// every tool, and [`CLIENT_ASSISTANT`] lists [`ASSISTANT_TOOLS`].
    pub fn specs(&self, client: &str) -> Vec<Spec> {
        if client == CLIENT_ASSISTANT {
            return self
                .inner
                .specs
                .iter()
                .filter(|sp| {
                    ASSISTANT_TOOLS.contains(&sp.name.as_str())
                        || ASSISTANT_WRITE_TOOLS.contains(&sp.name.as_str())
                })
                .cloned()
                .collect();
        }
        self.inner
            .specs
            .iter()
            .filter(|sp| !ASSISTANT_WRITE_TOOLS.contains(&sp.name.as_str()))
            .filter(|sp| client.is_empty() || sp.clients.iter().any(|c| c == client))
            .cloned()
            .collect()
    }

    /// `json.Marshal(Specs(client))`, Go's bytes (no trailing newline).
    pub fn specs_json(&self, client: &str) -> Vec<u8> {
        specs::marshal(&self.specs(client))
    }

    /// The test clock override (Go tests set `s.impl.now`).
    pub fn set_now(&mut self, now: NowFn) {
        if let Some(x) = Arc::get_mut(&mut self.inner) {
            x.now = now;
        }
    }

    /// Runs one tool. The result is text for the model, at most
    /// [`MAX_RESULT`] bytes; tool-level failures are text starting
    /// "Error: ". An `Err` means the call itself was malformed.
    pub fn call(&self, c: &Caller, name: &str, args: &[u8]) -> Result<String, CallError> {
        let x = &self.inner;
        let Some(h) = x.handlers.get(name) else {
            return Err(CallError::UnknownTool(name.to_string()));
        };
        let mut args = trim_ascii_space(args);
        if args.is_empty() || args == b"null" {
            args = b"{}";
        }
        if args[0] != b'{' || !gocompat::json::valid(args) {
            return Err(CallError::NotObject);
        }
        if x.needs_caller.contains_key(name) && trim(&c.sender.key).is_empty() {
            return Ok(err_text(
                "this connection isn't bound to an agent yet. Grok: call join first. \
Claude Code: the Irori mod attaches your session; reload it if this persists.",
            ));
        }
        if !trim(&c.sender.key).is_empty() && name != "slot_release" {
            slots::renew(x, c);
        }
        let out = x.around_board(name, args, || h(x, c, args));
        Ok(clip(&out, MAX_RESULT))
    }
}

/// Go `bytes.TrimSpace`.
fn trim_ascii_space(b: &[u8]) -> &[u8] {
    if let Ok(s) = std::str::from_utf8(b) {
        let t = trim(s);
        let start = t.as_ptr() as usize - s.as_ptr() as usize;
        return &b[start..start + t.len()];
    }
    let ws = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let lo = b.iter().position(|c| !ws(c)).unwrap_or(b.len());
    let hi = b.iter().rposition(|c| !ws(c)).map_or(lo, |i| i + 1);
    &b[lo..hi.max(lo)]
}

fn join(_: &Inner, _: &Caller, _: &[u8]) -> String {
    err_text("join is handled by irori mcp")
}

// ---- errors ---------------------------------------------------------------

/// `"Error: " + oneLine(msg, 600)`.
pub(crate) fn err_text(msg: &str) -> String {
    format!("Error: {}", format::one_line(msg, 600))
}

/// An error a tool turns into advice (Go's `mapErr` cases).
#[derive(Debug)]
pub(crate) enum ToolErr {
    Inbox(inbox::Error),
    Coord(coord::Error),
    Search(search::Error),
    Capture(capture::Error),
}

impl std::fmt::Display for ToolErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolErr::Inbox(e) => e.fmt(f),
            ToolErr::Coord(e) => e.fmt(f),
            ToolErr::Search(e) => e.fmt(f),
            ToolErr::Capture(e) => e.fmt(f),
        }
    }
}

impl From<inbox::Error> for ToolErr {
    fn from(e: inbox::Error) -> ToolErr {
        ToolErr::Inbox(e)
    }
}

impl From<coord::Error> for ToolErr {
    fn from(e: coord::Error) -> ToolErr {
        ToolErr::Coord(e)
    }
}

impl From<search::Error> for ToolErr {
    fn from(e: search::Error) -> ToolErr {
        ToolErr::Search(e)
    }
}

/// An inbox or coord error as advice. The error text can carry
/// caller-supplied values and agent names, so it is folded onto one line.
pub(crate) fn map_err(err: impl Into<ToolErr>) -> String {
    use coord::ErrorKind as C;
    use inbox::ErrorKind as I;
    let err = err.into();
    let e = err.to_string();
    match &err {
        ToolErr::Inbox(ie) => match ie.kind() {
            I::RateLimited => err_text(&format!(
                "rate limited: at most 8 messages per pair of agents per 10 minutes. Wait, or batch what you have into one message. ({e})"
            )),
            I::UnknownAgent => {
                err_text(&format!("{e}. Use the agents tool to see names and keys."))
            }
            I::Ambiguous => err_text(&format!("{e}. Use the agent's key from the agents tool.")),
            I::SelfAddress => err_text("that's you; address another agent (see the agents tool)."),
            I::Empty => err_text("give the message text."),
            I::NotYours | I::NotFound => err_text(&format!(
                "{e}. Use a delivery id addressed to you: the delivery=\"…\" value of an <irori-message> tag, or one the inbox tool lists."
            )),
            I::BadState => err_text(&format!(
                "{e}. The inbox tool (all = true) shows each delivery's state."
            )),
            _ => err_text(&e),
        },
        ToolErr::Coord(ce) => {
            if ce.is(C::Ambiguous) {
                err_text(&format!(
                    "{e}. Give the one you mean by its lettered ref (#12a) or its task:<id>."
                ))
            } else if ce.is(C::NotFound) {
                err_text(&format!(
                    "{e}. The board and leases tools show what exists."
                ))
            } else if ce.is(C::NotAllowed) && e.to_string().contains(coord::ASK_THE_PERSON) {
                // A card on another side (card #496): the refusal says what to do.
                err_text(&e)
            } else if ce.is(C::NotAllowed)
                && (e.contains("only a person brings it back")
                    || e.contains("when the person brings it back"))
            {
                // A Not yet card (card #570): the refusal says what brings
                // it back, and nobody else should be asked.
                err_text(&format!("{e}."))
            } else if ce.is(C::NotAllowed) {
                err_text(&format!(
                    "{e}. show #<n> (or task:<id>) lists its holders and creator; comment on it or message them instead."
                ))
            } else if ce.is(C::Held) {
                err_text(&format!(
                    "{e}. Comment on the card or message its holder; or wait for it to come free."
                ))
            } else if ce.is(C::Invalid) {
                err_text(&format!("{e}."))
            } else if ce.is(C::BadResource) {
                err_text(&format!(
                    "{e}. For files use path:<repoKey>:<glob>, with the repoKey from the agents tool."
                ))
            } else {
                err_text(&e)
            }
        }
        ToolErr::Search(se) => match se {
            search::Error::BadRef => err_text(&format!(
                "{e}. Copy a ref exactly as a list line printed it."
            )),
            s if s.is_not_found() => err_text(&format!(
                "{e} that you may read. search finds refs; messages show only to their sender and recipients."
            )),
            _ => err_text(&e),
        },
        ToolErr::Capture(ce) => match ce {
            capture::Error::NoRepo => err_text(&format!(
                "{e}. Give an absolute path, or repo = a repoKey from the agents tool."
            )),
            _ => err_text(&e),
        },
    }
}

/// Decodes a tool's args, or returns the model's error text.
pub(crate) fn decode(raw: &[u8], fields: &'static [(&'static str, Ty)]) -> Result<Args, String> {
    args::decode(raw, fields).map_err(|e| err_text(&e))
}

// ---- names ----------------------------------------------------------------

/// Agent keys as display names for results. `owner` is the person's rule
/// (coord's isOwner), so a made-up "user:" key is never shown as the
/// person.
pub(crate) struct Names<'a> {
    pub cards: HashMap<String, presence::AgentCard>,
    /// Last-known names of agents no longer on the roster (card #98).
    known: std::collections::BTreeMap<String, String>,
    co: &'a coord::Coord,
}

impl Inner {
    pub(crate) fn roster(&self) -> Names<'_> {
        let mut cards = HashMap::new();
        for c in self.b.agents() {
            cards.insert(c.key.clone(), c);
        }
        Names {
            cards,
            known: self.b.known_names(),
            co: self.b.coord(),
        }
    }

    pub(crate) fn is_owner(&self, key: &str) -> bool {
        self.b.coord().is_person(key)
    }

    pub(crate) fn now(&self) -> GoTime {
        (self.now)()
    }
}

impl Names<'_> {
    pub fn owner(&self, key: &str) -> bool {
        self.co.is_person(key)
    }

    /// An agent key as `name (key)`, folded.
    pub fn who(&self, key: &str) -> String {
        if key.is_empty() {
            return "nobody".into();
        }
        let k = format::one_line(key, 120);
        if self.owner(key) {
            return format!("the person ({k})");
        }
        // Another person (a huddle's speaker) is never a gone agent: they
        // print by name, or as "another person" when this device has none.
        if coord::talk::is_other_person_key(key) {
            return match self.known.get(key).filter(|n| !n.trim().is_empty()) {
                Some(name) => format::one_line(name, 80),
                None => "another person".into(),
            };
        }
        if let Some(c) = self.cards.get(key) {
            let name = format::one_line(&c.name, 80);
            if !name.is_empty() {
                if c.status == "gone" {
                    return format!("{name} ({k}, gone)");
                }
                if c.status == coord::roster::NOT_ANSWERING {
                    return format!("{name} ({k}, isn't answering)");
                }
                return format!("{name} ({k})");
            }
        }
        // Gone from the roster: the last name it went by (card #98).
        if let Some(name) = self.known.get(key).filter(|n| !n.trim().is_empty()) {
            return format!("{} ({k}, gone)", format::one_line(name, 80));
        }
        k
    }

    /// A message's sender: the card name when the agent is on the roster,
    /// else the name the Core recorded for it.
    pub fn sender(&self, f: &inbox::Sender) -> String {
        if self.cards.contains_key(&f.key) || self.owner(&f.key) {
            return self.who(&f.key);
        }
        let name = format::one_line(&f.name, 80);
        if !name.is_empty() {
            return format!("{name} ({})", format::one_line(&f.key, 120));
        }
        self.who(&f.key)
    }

    /// A key's name alone when the roster knows it, else the key.
    pub fn short(&self, key: &str) -> String {
        if self.owner(key) {
            return "the person".into();
        }
        if let Some(c) = self.cards.get(key) {
            let name = format::one_line(&c.name, 80);
            if !name.is_empty() {
                return name;
            }
        }
        format::one_line(key, 120)
    }

    pub fn live(&self, key: &str) -> bool {
        self.cards.get(key).is_some_and(|c| c.status != "gone")
    }
}

/// Said once per result that carries text other agents wrote.
pub(crate) fn untrusted() -> &'static str {
    UNTRUSTED
}
