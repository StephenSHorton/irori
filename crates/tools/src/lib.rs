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
mod attachments;
mod board;
mod capturetools;
mod explore;
mod format;
mod messages;
pub mod notice;
pub mod notify;
mod observe;
mod room;
mod shared;
mod specs;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use gocompat::time::GoTime;

pub use attachments::Attacher;
pub use format::{clip, clip_mcp, one_line};
pub use notify::{Action, Note, Notifier};
pub use observe::redact;
pub use shared::{
    BoardInfo, CHOICE_CREATE, CHOICE_NO, CHOICE_REVIEW, CHOICE_SHARE, CHOICE_WITHDRAW,
    REQUEST_WAIT, SharedBoards,
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
            format_args!("no shared board holds a lease on {resource}"),
        ))
    }
}

/// Shared boards to show (Go `BoardViewer`).
pub trait BoardViewer: Send + Sync {
    fn board_views(&self, f: &coord::BoardFilter) -> Vec<coord::BoardView>;
}

/// What the tools act on (Go `Backend`); the Core provides it.
pub trait Backend: Send + Sync {
    /// This device's roster (presence plus Grok members).
    fn agents(&self) -> Vec<presence::AgentCard>;
    fn events(&self, f: &store::Filter) -> Result<Vec<store::Event>, String>;
    fn inbox(&self) -> &inbox::Inbox;
    fn coord(&self) -> &Arc<coord::Coord>;
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

/// The read-only tools that read as the caller.
const CALLER_READS: [&str; 6] = ["inbox", "timeline", "overview", "search", "show", "board"];

fn handlers() -> HashMap<&'static str, Handler> {
    let list: [(&'static str, Handler); 41] = [
        ("agents", observe::agents),
        ("events", observe::events),
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
        ("board_assign", board::board_assign),
        ("board_status", board::board_status),
        ("board_link", board::board_link),
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
        ("release", board::release),
        ("timeline", capturetools::timeline),
        ("why", capturetools::why),
        ("join", join),
        ("notify", notify::notify),
        ("attachment_get", attachments::attachment_get),
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
    /// every tool.
    pub fn specs(&self, client: &str) -> Vec<Spec> {
        self.inner
            .specs
            .iter()
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
        if let Some(c) = self.cards.get(key) {
            let name = format::one_line(&c.name, 80);
            if !name.is_empty() {
                if c.status == "gone" {
                    return format!("{name} ({k}, gone)");
                }
                return format!("{name} ({k})");
            }
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
