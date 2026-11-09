//! The Core itself (I:internal/core/core.go, colors.go).
//!
//! Wiring notes (what a merge or a later milestone plugs in):
//! - API: listen on 127.0.0.1:0, then [`Core::write_lockfile`] with the
//!   port; [`Core::token`] is the bearer token (already in capture's exact
//!   scrubber). Go writes the lockfile at the end of Start; here the API
//!   host does it once it listens. [`Core::close`] removes it.
//! - API / window reads: [`Core::subscribe_rows`] (the events stream),
//!   [`Core::feed`] (every service's changes), [`crate::events::coalesce`].
//! - Tools: [`Core::tools`] is the registry over the Core (Go
//!   `tools.New(meshView{c})`); the API answers `GET /v1/tools` with
//!   `Set::specs_json(client)` and `POST /v1/tools/{name}` with
//!   `Set::call` (`UnknownTool` 404, `NotObject` 400). Notices and the
//!   card-thread helpers are `tools::notice`'s, re-exported in
//!   [`crate::notice`]. The attacher is [`crate::attachments`]'s
//!   ([`Core::attach_files`], [`Core::get_attachment`]). Off until their
//!   milestones: the notifier (M6) and shared boards (M9).
//! - Search (capture's FTS5 index, Show, Overview, Why, RecordDecision):
//!   capture's sources are the Core's coord and inbox
//!   ([`Core::coord`], [`Core::inbox`]), added at start. The one decision
//!   fold is capture's (`dview`): RecordDecision, Why and Show go through
//!   capture, [`Core::list_decisions`] is `Capture::decisions`, and the
//!   Core's loop folds other devices' rows into it live
//!   (`Capture::follow_decision`, `refold_decisions` on resync).
//! - Grok members (`core/members.go`, [`crate::members`]):
//!   [`Core::join_member`], [`Core::member_by_secret`],
//!   [`Core::member_stream`]; their cards are in [`Core::local_agents`] and
//!   the roster coord and capture read.
//! - Mesh (M5): the roster's liveness, `Kick` on each follower, and the
//!   transport; pairing's `LearnSaved` before the services open.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use coord::roster::{AgentCard, Agents, Roster};
use coord::{Coord, Notice};
use gocompat::time::GoTime;
use inbox::notice::NoticeResult;
use inbox::{Inbox, Sender};
use irori_identity::keystore::Keystore;
use irori_identity::{Chain, DeviceJson, Identity};
use serde::Serialize;
use store::{Event, Filter, Store, kinds};
use sys::cancel::Cancel;
use tokio_util::sync::CancellationToken;

use crate::events::{CoreEvent, Feed, Hub, Topic};
use crate::lockfile::{self, Lockfile};
use crate::members::{Member, Members};
use crate::namer::Namer;
use crate::notice;
use crate::timers::{self, Kick, Ticker};
use crate::{CLOSE_WAIT, COORD_SWEEP_INTERVAL, logf};

pub mod assistant_answers;
pub mod assistant_chat;
pub mod assistant_home;
mod assistant_name;
pub mod assistant_packet;
mod assistant_turns;
pub mod boards;
mod boards_members;
mod boards_own;
pub mod boards_presence;
pub mod boards_status;
mod bridge;
pub mod call_settings;
pub(crate) mod calls;
mod calls_over;
pub(crate) mod calls_share;
pub mod card_weights;
mod deskmerge;
mod dm_spaces;
pub mod freshness;
pub mod huddle_canvas;
mod huddle_end;
pub mod huddle_feed;
pub mod huddle_host;
pub mod huddle_packet;
mod huddle_rules;
pub mod huddle_session;
mod huddle_settings;
mod liveness;
mod mirror;
pub mod name_themes;
pub mod notes;
mod notifications;
mod prcard;
mod review_claim;
pub mod space_move;
pub mod spaces;
pub mod spawn;
pub mod spawn_agents;
pub mod spawn_boards;
pub mod spawn_capable;
pub mod spawn_limits;
pub mod spawn_pause;
pub mod spawn_permissions;
pub mod spawn_remote;
pub mod spawn_stop;
pub mod spawn_waiting;
pub mod spawn_why;
pub mod storage;
pub mod sync_out;
pub mod thread_names;
pub use mirror::BoardReviewer;
pub mod invites;
mod talk;
pub mod talk_replying;
pub mod trees;

use boards::Boards;

/// The Core's clock: local time, as Go's `time.Now()` shows it.
pub type NowFn = Arc<dyn Fn() -> GoTime + Send + Sync>;
/// The desktop app's Revive (`claude --bg --resume <session>`).
pub type ReviveFn = Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;
/// A capture options edit (tests: intervals, git).
pub type CaptureEdit = Arc<dyn Fn(&mut capture::Options) + Send + Sync>;
/// A presence options edit (tests: intervals, liveness).
pub type PresenceEdit = Arc<dyn Fn(&mut presence::Options) + Send + Sync>;
/// A home server config edit (Go `Options.Home`; tests and dev builds:
/// 127.0.0.1 listeners, a fake router, no awake hold).
pub type HomeEdit = Arc<dyn Fn(&mut home::Config) + Send + Sync>;

/// Configures [`Core::start`] (Go `core.Options`, the parts M4 has).
#[derive(Default)]
pub struct Options {
    pub data_dir: PathBuf,
    /// `~/.claude`.
    pub claude_dir: PathBuf,
    pub version: String,
    pub headless: bool,
    /// Seals the person and device keys; `None` is the OS default (DPAPI,
    /// the Keychain). Tests pass a file keystore.
    pub keystore: Option<Box<dyn Keystore + Send + Sync>>,
    /// The inbox's reclaim of unconfirmed wakes; zero is
    /// [`inbox::REATTACH_GRACE`].
    pub reattach_grace: Duration,
    /// How long a Grok member's card outlives its last stream; zero is
    /// [`inbox::REATTACH_GRACE`]. Tests shorten it.
    pub member_grace: Duration,
    /// The wall clock of the log's HLC, the Board, the inbox and the asks;
    /// `None` is local time now. Tests set a device's clock off.
    pub now: Option<NowFn>,
    /// The runtime presence runs on; `None` gives it its own thread.
    pub runtime: Option<tokio::runtime::Handle>,
    /// Edits capture's options before it opens.
    pub capture: Option<CaptureEdit>,
    /// Edits presence's options before it opens.
    pub presence: Option<PresenceEdit>,
    /// Edits the home server's config before it is built.
    pub home: Option<HomeEdit>,
    /// Builds no home server (Go: a passed-in test transport has none);
    /// [`Core::set_home_server`] then answers [`NO_HOME_SERVER`].
    pub no_home: bool,
    /// The desktop cards (Go `Options.Notifier`): the app passes one made
    /// for this data dir. `None` is headless (Go's nil Notifier): no card
    /// is ever shown and no Watch runs, so tests never put one on screen.
    pub notifier: Option<notify::Notifier>,
    /// The phone's side of a call (card #654): Android's microphone
    /// service. `None` on desktops and in tests.
    pub call_platform: Option<Arc<dyn call::Platform>>,
    /// The mic and speaker a call opens; `None` is the default devices.
    /// Tests pass fake ones.
    pub call_audio: Option<call::device::OpenerFn>,
    /// Writes down what this device's person says in calls (card #656);
    /// `None`: this device can't.
    pub call_transcriber: Option<call::TranscriberFn>,
    /// Builds the transport pairing runs on (M7, [`crate::devices`]);
    /// `None` is iroh on the device key, bound when a window opens.
    pub pair_net: Option<crate::devices::PairNetFn>,
    /// How long a pairing window stays open (tests); zero is
    /// `irori_pair::WINDOW` (Go `Options.PairWindow`).
    pub pair_window: Duration,
    /// Remove device (Go `Mesh().Revoke`); `None` is the mesh Node's.
    pub revoke: Option<crate::devices::RevokeFn>,
    /// Asks GitHub how the PRs this device's agents' cards link stand
    /// (card #208): the app passes [`crate::prwatch::GhChecker`]; `None`
    /// asks nobody (tests pass a fake when they want it).
    pub pr_checker: Option<Arc<dyn crate::prwatch::PrChecker>>,
    /// Lists a merge desk's open PRs on GitHub (card #415): the app passes
    /// [`crate::deskwatch::GhLister`]; `None` polls no desk.
    pub desk_lister: Option<Arc<dyn crate::deskwatch::PrLister>>,
    /// Card #427: clear the `target/` of settled cards' worktrees on this
    /// device ([`crate::buildclean`]) once untouched this long. The app
    /// passes [`crate::buildclean::IDLE`]; `None` (tests, rehearsals)
    /// never deletes a build.
    pub clear_builds: Option<Duration>,
    /// Card #580: fetch every file up to this many bytes another device
    /// offers, so this device holds it too ([`crate::prefetch`]). The
    /// desktop app passes [`crate::prefetch::SMALL_FILE`]; `None` (phones,
    /// tests) fetches only what is asked for.
    pub prefetch_files: Option<u64>,
}

/// Go `core.ErrNoHomeServer`'s text.
pub const NO_HOME_SERVER: &str = "this device can't run a home server";

/// Why [`Core::start`] failed.
#[derive(Debug)]
pub enum StartError {
    /// Go `ErrAlreadyRunning`: another Irori holds the lockfile.
    AlreadyRunning {
        pid: i64,
    },
    Other(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::AlreadyRunning { pid } => {
                write!(f, "{} (pid {pid})", crate::ALREADY_RUNNING)
            }
            StartError::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for StartError {}

fn other(prefix: &str, e: impl std::fmt::Display) -> StartError {
    if prefix.is_empty() {
        StartError::Other(e.to_string())
    } else {
        StartError::Other(format!("{prefix}: {e}"))
    }
}

/// One running Irori.
pub struct Core {
    s: Arc<Services>,
    /// The agent tools over [`ToolsView`] (Go `c.tools`).
    tools: tools::Set,
    stop: Cancel,
    stop_async: CancellationToken,
    threads: Mutex<Vec<JoinHandle<()>>>,
    /// The cards' Watch over the inbox, Board and asks (desktop only).
    watch: Mutex<Option<notify::WatchHandle>>,
    /// Card terminals (card #369): the Core owns the shells, so they
    /// outlive the page that shows them.
    terms: Arc<term::Terms>,
    closed: AtomicBool,
}

/// What a card's buttons act through (notify's room): the private board,
/// and a finished card's Mark Done tells the agent that finished it, as the
/// window's Done does (card #140).
struct WatchRoom(Arc<Services>);

impl notify::watch::RoomSource for WatchRoom {
    fn ask_again(&self, by: &str, id: &str) -> Result<(), String> {
        notify::watch::RoomSource::ask_again(&*self.0.coord, by, id)
    }
    fn set_task_state(&self, by: &str, id: &str, state: &str, note: &str) -> Result<(), String> {
        notify::watch::RoomSource::set_task_state(&*self.0.coord, by, id, state, note)
    }
    fn mark_done(&self, by: &str, id: &str) -> Result<(), String> {
        let ns = notify::watch::mark_done(&self.0.coord, by, id)?;
        let from = self.0.user_sender();
        for n in &ns {
            if let Some(e) = notice::notice_err(&self.0.route_notice(&from, n)) {
                eprintln!("notify: mark done {id}: telling the agent: {e}");
            }
        }
        Ok(())
    }
}

/// Everything the loops share.
struct Services {
    data_dir: PathBuf,
    version: String,
    headless: bool,
    now: NowFn,
    device: DeviceJson,
    ident: Arc<Identity>,
    chain: Arc<Chain>,
    store: Arc<Store>,
    presence: Arc<presence::Watcher>,
    roster: Arc<Roster>,
    inbox: Arc<Inbox>,
    coord: Arc<Coord>,
    capture: capture::Capture,
    asks: Arc<coord::approvals::Book>,
    names: Arc<Namer>,
    /// The last name every agent went by, kept past its going (card #98).
    agent_names: Arc<crate::agentnames::AgentNames>,
    /// Claude usage left, as the mod last reported it (card #599).
    usage: crate::usage::Usage,
    /// Card #208: who asks GitHub about PRs, and when each was last asked.
    pr_checker: Option<Arc<dyn crate::prwatch::PrChecker>>,
    pr_watch: crate::prwatch::PrWatch,
    /// Card #415: who lists merge desks' PRs, and each desk's polling.
    desk_lister: Option<Arc<dyn crate::deskwatch::PrLister>>,
    desk_watch: crate::deskwatch::DeskWatch,
    /// Card #421: merged PRs the desk already settled, and the newest
    /// `gh pr merge` heard.
    desk_merges: crate::deskmerge::MergeHeard,
    /// A `gh pr merge` was heard: the desks thread polls every desk this
    /// device polls at its next turn (one thread polls desks, card #970).
    desks_due: std::sync::atomic::AtomicBool,
    /// Card #427: whether to clear settled cards' builds, when it last
    /// looked, and whether a pass is deleting now.
    clear_builds: Option<Duration>,
    builds_looked: Arc<Mutex<crate::buildclean::Looked>>,
    builds_busy: Arc<std::sync::atomic::AtomicBool>,
    /// These Services, once built (card #565: a sweep's thread asks them
    /// again just before a tree goes).
    this: Arc<OnceLock<Weak<Services>>>,
    members: Arc<Members>,
    names_kick: Arc<Kick>,
    /// This device's agents that stopped answering their wakes (#110).
    answering: Arc<crate::answering::Answering>,
    /// Kicked by roster and Board changes: asks are handed out and on by
    /// these events, never on a timer (#91).
    asks_kick: Arc<Kick>,
    colors: coord::devcolor::Picks,
    /// The person's own colour on shared boards (card #385).
    person_color: coord::devcolor::PersonPick,
    /// Agents' faces by name (card #407).
    faces: coord::avatar::Faces,
    /// The person's own avatar on shared spaces (card #393).
    person_avatar: coord::person_avatar::PersonAvatar,
    assistant_name: coord::assistant_name::AssistantName,
    /// The person's names for their devices (card #184).
    device_names: Arc<coord::devname::Names>,
    /// Talk's channels, DMs and read cursors (card #389).
    talk: Arc<coord::talk::Talk>,
    /// The person's notification history (card #377).
    notifications: Arc<coord::notifications::History>,
    /// The Office Assistant's one conversation (card #1102).
    assistant_chat: Arc<assistant_chat::Fold>,
    files: blob::Files,
    scrubber: Arc<scrub::Scrubber>,
    secrets: Mutex<Vec<String>>,
    token: String,
    /// The window assistant's per-run secret (card #783): the API reads a
    /// call carrying it as the person, through its read tools, and its
    /// two writes only per grant (card #987).
    /// Only in memory and in the assistant's own 0600 MCP config.
    assistant: String,
    /// What the window last said it shows (card #1009), for the
    /// assistant's `screen` tool. In memory only; the newest ask wins.
    assistant_screen: Mutex<String>,
    /// What the person's message lets the assistant change (card #987).
    assistant_grants: tools::Grants,
    reviver: Arc<Mutex<Option<ReviveFn>>>,
    /// Spawned agents (card #446): the window's spawner, the boards'
    /// switches and the devices' caps.
    spawn: spawn::SpawnState,
    hub: Hub,
    rearm: Arc<Kick>,
    /// This device's home server; `None` with [`Options::no_home`].
    home: Option<home::Server>,
    notifier: CoreNotifier,
    /// The pairing flow (M7).
    pairing: crate::devices::Pairing,
    revoke: Option<crate::devices::RevokeFn>,
    mesh: Arc<crate::meshnode::Mesh>,
    /// The shared boards (Go `c.boards`).
    boards: Arc<Boards>,
    /// The hidden DM spaces' markers (card #538).
    dm_spaces: Arc<boards_own::DmSpaces>,
    /// The DM hand-offs' work (card #538).
    dms: dm_spaces::Dms,
    /// Calls (card #654).
    calls: calls::Calls,
    /// "Transcribe what I say in calls" (card #656).
    call_settings: call_settings::CallSettings,
    /// Irori's assistant naming the person's new threads (card #782).
    naming: thread_names::Naming,
    /// The Office Assistant's computer, folded from `assistant.home` rows
    /// (one-assistant card 1).
    assistant_home: assistant_home::HomeState,
    /// The queue of questions this computer answers (one-assistant card 3).
    chat: assistant_turns::ChatQueue,
    /// Irori's assistant weighing new cards (card #965).
    weighing: card_weights::Weighing,
    /// The two huddle switches (card #1053).
    huddle_switches: huddle_settings::HuddleSwitches,
    /// Office Assistants answering other people (assistants-talk card 4).
    asst: assistant_answers::Answering,
    /// The assistant's held session per huddle (card #1046).
    held: huddle_session::HeldSessions,
    /// Which device hosts each person's assistant per huddle (card #1049).
    hosting: huddle_host::Hosting,
    /// The live lines of the huddles this device serves (card #1050).
    feed: huddle_feed::Feed,
    /// The themes new agents' names come from (card #702), shared with
    /// `names`.
    name_themes: Arc<name_themes::NameThemes>,
    /// The open board invite (card #44).
    invites: invites::Invites,
}

/// The notify crate's [`notify::Notifier`] as the tools' seam
/// (`tools::Notifier`); notify sits below tools, so the adapter is here.
struct CoreNotifier(notify::Notifier);

impl tools::Notifier for CoreNotifier {
    fn headless(&self) -> bool {
        self.0.is_headless()
    }
    fn enabled(&self) -> bool {
        self.0.enabled()
    }
    fn post(&self, n: tools::Note) -> bool {
        self.0.post(notify::Note {
            id: n.id,
            title: n.title,
            body: n.body,
            level: notify::parse_level(&n.level).unwrap_or_default(),
            expire: n.expire,
            actions: n
                .actions
                .into_iter()
                .map(|a| notify::Action {
                    label: a.label,
                    run: Some(a.run),
                })
                .collect(),
            on_dismiss: n.on_dismiss,
            on_expire: n.on_expire,
            about: Some(coord::notifications::PostedRow {
                kind: coord::notifications::KIND_AGENT.into(),
                from: n.from,
                ..Default::default()
            }),
            ..notify::Note::default()
        })
    }
}

// ---- the tools' backend -----------------------------------------------------

/// The Core as the tools see it (Go `meshView`): `agents` is the mesh
/// roster; the attacher is the Core's ([`crate::attachments`]). The
/// optional parts left at their defaults are off: the shared-board router
/// and viewer (M9).
struct ToolsView(Arc<Services>);

impl tools::Attacher for ToolsView {
    fn attach_files(&self, by: &str, paths: &[String]) -> Result<Vec<blob::FileRef>, blob::Error> {
        self.0.attach_files(by, paths)
    }
    fn get_attachment(
        &self,
        reader: &str,
        sha: &str,
        to: &str,
    ) -> Result<(String, String), String> {
        self.0.get_attachment(reader, sha, to)
    }
}

impl tools::Backend for ToolsView {
    fn assistant_screen(&self) -> String {
        self.0
            .assistant_screen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn device_id(&self) -> String {
        self.0.device.id.clone()
    }
    fn device_free(&self, path: &str) -> Option<tools::DeviceFree> {
        let dir = if path.is_empty() {
            self.0.data_dir.clone()
        } else {
            PathBuf::from(path)
        };
        let q = sys::meters::quick(&dir);
        Some(tools::DeviceFree {
            dir: dir.to_string_lossy().into_owned(),
            disk: q.disk.map(|d| (d.free_bytes, d.total_bytes)),
            mem: q
                .mem
                .map(|m| (m.total_bytes.saturating_sub(m.used_bytes), m.total_bytes)),
            cpu: q.cpu.and_then(|c| c.pct.map(|p| (p, c.cores))),
        })
    }
    fn agents(&self) -> Vec<presence::AgentCard> {
        self.0
            .roster
            .agents()
            .into_iter()
            .map(presence_card)
            .collect()
    }
    fn attached(&self, key: &str) -> Option<bool> {
        Some(self.0.inbox.reachable(key).0)
    }
    fn room_agents(&self) -> Vec<AgentCard> {
        self.0.ask_agents()
    }
    fn board_agents(&self) -> Vec<tools::BoardAgent> {
        self.0
            .board_others()
            .into_iter()
            .map(|a| tools::BoardAgent {
                board: a.board,
                person: a.person,
                card: presence_card(a.card),
                assistant: a.assistant,
            })
            .collect()
    }
    fn events(&self, f: &Filter) -> Result<Vec<Event>, String> {
        self.0.store.list(f).map_err(|e| e.to_string())
    }
    fn inbox(&self) -> &Inbox {
        &self.0.inbox
    }
    fn coord(&self) -> &Arc<Coord> {
        &self.0.coord
    }
    fn known_names(&self) -> std::collections::BTreeMap<String, String> {
        self.0.note_agent_names();
        let mut out = self.0.agent_names.all();
        // Other people and their agents in a space's channels (card #392).
        for (k, v) in self.0.space_names() {
            out.entry(k).or_insert(v);
        }
        out
    }
    fn device_labels(&self) -> std::collections::BTreeMap<String, String> {
        self.0.device_labels()
    }
    fn agent_start(&self, caller: &str, task_id: &str) -> Result<String, String> {
        self.0.agent_start(caller, task_id)
    }
    fn agent_end(&self, caller: &str, key: &str) -> Result<String, String> {
        self.0.agent_end(caller, key)
    }
    fn thread_agent_check(&self, caller: &str) -> Result<(), String> {
        self.0.thread_agent_check(caller)
    }
    fn thread_new_agent(&self, caller: &str, task_id: &str, watch: bool) -> Result<String, String> {
        self.0.thread_new_agent(caller, task_id, watch)
    }
    fn agents_pause(&self, caller: &str, on: bool) -> Result<String, String> {
        self.0.agent_set_paused(caller, on)
    }
    fn agents_paused(&self, caller: &str) -> Option<tools::AgentsPaused> {
        let (device, p) = self.0.agent_pause_of(caller);
        Some(tools::AgentsPaused {
            device,
            on: p.on,
            since: if p.on {
                p.since.format_rfc3339()
            } else {
                String::new()
            },
            by: self.0.pause_by_name(&p.by),
        })
    }
    fn agent_limit(&self) -> Option<tools::AgentLimit> {
        let u = self.0.agent_starts_use();
        Some(tools::AgentLimit {
            on: u.on,
            max: u.max,
            used: u.used,
        })
    }
    fn spawn_wait(&self, task_id: &str, asked: bool) -> String {
        self.0.spawn_wait(task_id, asked)
    }
    fn recheck_prs(&self, co: &Arc<Coord>, caller: &str, task_id: &str) {
        self.0.recheck_prs(co, caller, task_id)
    }
    fn avatar(&self, name: &str) -> String {
        self.0.faces.get(name)
    }
    fn set_avatar(&self, name: &str, face: Option<&coord::avatar::Avatar>) -> Result<(), String> {
        set_avatar(&self.0, name, face)
    }
    fn capture(&self) -> Option<&capture::Capture> {
        Some(&self.0.capture)
    }
    fn asks(&self) -> Option<&Arc<coord::approvals::Book>> {
        Some(&self.0.asks)
    }
    fn notifier(&self) -> Option<&dyn tools::Notifier> {
        // Go's nil Notifier on a headless Core.
        (!self.0.notifier.0.is_headless()).then_some(&self.0.notifier as &dyn tools::Notifier)
    }
    fn attacher(&self) -> Option<&dyn tools::Attacher> {
        Some(self)
    }
    fn router(&self) -> Option<&dyn tools::TaskRouter> {
        Some(self)
    }
    fn board_viewer(&self) -> Option<&dyn tools::BoardViewer> {
        Some(self)
    }
    fn shared_boards(&self) -> Option<&dyn tools::SharedBoards> {
        Some(self)
    }
    fn personal_boards(&self) -> Vec<tools::PersonalBoard> {
        self.0
            .personal_boards()
            .unwrap_or_default()
            .into_iter()
            .map(|b| tools::PersonalBoard {
                board_id: b.board_id,
                name: b.name,
                tag: b.tag,
                description: b.description,
                closed: b.closed,
            })
            .collect()
    }
    fn talk(&self) -> Option<&Arc<coord::talk::Talk>> {
        Some(&self.0.talk)
    }
    fn huddle_now(&self, call: &str) -> Option<tools::HuddleNow> {
        self.0.huddle_now(call)
    }
    fn notes(&self) -> Option<&dyn tools::NotesBackend> {
        Some(self)
    }
}

// The Core holds the private log and every board's (v0.5 §2.1); a tools
// backend without these would silently act on the private log alone.
impl tools::TaskRouter for ToolsView {
    fn task_coord(
        &self,
        task_id: &str,
        keys: &[String],
    ) -> Result<(Arc<Coord>, Vec<String>), coord::Error> {
        self.0.task_coord(task_id, keys)
    }
    fn private_notice(&self, n: Notice) -> Notice {
        self.0.private_notice(n)
    }
    fn task_ref(&self, r: &str) -> Result<String, coord::Error> {
        self.0.task_ref(r)
    }
    fn private_key(&self, key: &str) -> String {
        self.0.private_key(key)
    }
    fn mirror_acquire(
        &self,
        key: &str,
        resource: &str,
        ttl: std::time::Duration,
        steal: bool,
    ) -> Result<Vec<String>, coord::Error> {
        self.0.mirror_acquire(key, resource, ttl, steal)
    }
    fn mirror_release(&self, key: &str, resource: &str) {
        self.0.mirror_release(key, resource)
    }
    fn request_release(
        &self,
        by: &str,
        resource: &str,
        why: &str,
    ) -> Result<(String, coord::Comment, Notice), coord::Error> {
        self.0.request_release(by, resource, why)
    }
    fn review_claim(
        &self,
        key: &str,
        pr: &str,
        action: &str,
        reason: &str,
    ) -> Result<tools::ClaimAnswer, String> {
        self.0.review_claim(key, pr, action, reason)
    }
    fn comment_coord(&self, id: &str) -> Option<Arc<Coord>> {
        // Not a hidden DM space's (card #538): no agent works there.
        self.0
            .boards
            .list()
            .into_iter()
            .filter(|b| !self.0.is_dm_space(&b.id))
            .map(|b| b.coord().clone())
            .find(|co| co.get_comment(id).is_some())
    }
}

impl tools::BoardViewer for ToolsView {
    fn board_views(&self, f: &coord::BoardFilter) -> Vec<coord::BoardView> {
        self.0.board_views(f)
    }
}

/// The mesh roster's card as presence holds it ([`roster_card`] undone).
fn presence_card(c: AgentCard) -> presence::AgentCard {
    presence::AgentCard {
        key: c.key,
        device_id: c.device_id,
        agent: c.agent,
        session_id: c.session_id,
        pid: c.pid,
        name: c.name,
        title: c.title,
        name_source: c.name_source,
        ai_title: c.ai_title,
        cwd: c.cwd,
        status: c.status,
        session_kind: c.session_kind,
        entrypoint: c.entrypoint,
        version: c.version,
        peer_features: c.peer_features,
        started_at: c.started_at,
        updated_at: c.updated_at,
        last_activity: c.last_activity,
        last_text: c.last_text,
        transcript: c.transcript,
        repo_key: c.repo_key,
        permission_mode: c.permission_mode,
        last_tool: c.last_tool,
        device_state: c.device_state,
        waiting_for: c.waiting_for,
        device_seen: c.device_seen,
        device_via: c.device_via,
        status_since: c.status_since,
    }
}

// ---- the crates' roster projections ---------------------------------------

/// presence's card as the mesh roster holds it.
fn roster_card(c: presence::AgentCard) -> AgentCard {
    AgentCard {
        key: c.key,
        device_id: c.device_id,
        agent: c.agent,
        session_id: c.session_id,
        pid: c.pid,
        name: c.name,
        title: c.title,
        name_source: c.name_source,
        ai_title: c.ai_title,
        cwd: c.cwd,
        status: c.status,
        session_kind: c.session_kind,
        entrypoint: c.entrypoint,
        version: c.version,
        peer_features: c.peer_features,
        started_at: c.started_at,
        updated_at: c.updated_at,
        last_activity: c.last_activity,
        last_text: c.last_text,
        transcript: c.transcript,
        repo_key: c.repo_key,
        permission_mode: c.permission_mode,
        last_tool: c.last_tool,
        device_state: c.device_state,
        waiting_for: c.waiting_for,
        device_seen: c.device_seen,
        device_via: c.device_via,
        status_since: c.status_since,
    }
}

fn inbox_card(c: AgentCard) -> inbox::AgentCard {
    inbox::AgentCard {
        key: c.key,
        device_id: c.device_id,
        session_id: c.session_id,
        agent: c.agent,
        name: c.name,
        status: c.status,
        repo_key: c.repo_key,
        permission_mode: c.permission_mode,
    }
}

fn capture_card(c: AgentCard) -> capture::AgentCard {
    capture::AgentCard {
        key: c.key,
        session_id: c.session_id,
        cwd: c.cwd,
        status: c.status,
        name: c.name,
        title: c.title,
        repo_key: c.repo_key,
    }
}

/// This device's roster (Go `LocalAgents`): presence, plus Grok members
/// once the API brings them.
type LocalFn = Arc<dyn Fn() -> Vec<AgentCard> + Send + Sync>;

struct LocalAgents(LocalFn);
struct MeshAgents(Arc<Roster>);

/// What coord reads: this device's agents, and names from the whole mesh
/// and the name book, so its notice texts name agents (card #98).
struct CoordAgents {
    local: LocalFn,
    mesh: Arc<Roster>,
    names: Arc<crate::agentnames::AgentNames>,
}

impl Agents for CoordAgents {
    fn agents(&self) -> Vec<AgentCard> {
        (self.local)()
    }
    fn name_of(&self, key: &str) -> String {
        let live = self
            .mesh
            .agents()
            .into_iter()
            .find(|a| a.key == key)
            .map(|a| a.name)
            .unwrap_or_default();
        if !live.trim().is_empty() {
            return live;
        }
        self.names.get(key)
    }
}

impl Agents for LocalAgents {
    fn agents(&self) -> Vec<AgentCard> {
        (self.0)()
    }
}

impl capture::Roster for LocalAgents {
    fn agents(&self) -> Vec<capture::AgentCard> {
        (self.0)().into_iter().map(capture_card).collect()
    }
}

impl inbox::Roster for MeshAgents {
    fn agents(&self) -> Vec<inbox::AgentCard> {
        self.0.agents().into_iter().map(inbox_card).collect()
    }
}

struct Scrub(Arc<scrub::Scrubber>);

impl capture::Scrubber for Scrub {
    fn string(&self, s: &str) -> String {
        self.0.string(s)
    }
    fn skip_file(&self, path: &str) -> bool {
        scrub::skip_file(path)
    }
}

struct Blobs(blob::Store);

impl capture::Blobs for Blobs {
    fn put(&self, data: &[u8]) -> Result<String, String> {
        self.0.put(data).map_err(|e| e.to_string())
    }
    fn get(&self, id: &str) -> Result<Vec<u8>, String> {
        self.0.get(id).map_err(|e| e.to_string())
    }
}

/// sys::fswatch as presence's file events (Go passes `fswatch.Watch`).
/// The window assistant's run dir under data dir `dir` (card #783).
pub fn assistant_dir(dir: &Path) -> PathBuf {
    dir.join(ASSISTANT_DIR)
}

/// The folders presence leaves off the roster for the assistant: its run
/// dir as named and, off Windows, as resolved (Claude Code records the
/// real path: `/var` is `/private/var` on the Mac).
fn assistant_skip(dir: &Path) -> Vec<String> {
    let d = assistant_dir(dir);
    let mut v = vec![d.to_string_lossy().into_owned()];
    if !cfg!(windows)
        && let Ok(real) = std::fs::canonicalize(dir)
    {
        let r = real.join(ASSISTANT_DIR).to_string_lossy().into_owned();
        if r != v[0] {
            v.push(r);
        }
    }
    v
}

/// The most characters of the window's snapshot the Core keeps (card #1009).
pub const ASSISTANT_SCREEN_MAX: usize = 4000;

/// The window assistant's run dir's name in the data dir (card #783).
pub const ASSISTANT_DIR: &str = "assistant";

fn fs_watch(root: &Path, recursive: bool) -> Result<presence::FsSource, presence::WatchError> {
    let cancel = Cancel::new();
    let w = sys::fswatch::watch(&cancel, &root.to_string_lossy(), recursive, Duration::ZERO)
        .map_err(|e| match e {
            sys::fswatch::Error::Unsupported => presence::WatchError::Unsupported,
            sys::fswatch::Error::Io(e) => presence::WatchError::Other(e.to_string()),
        })?;
    let (tx, rx) = tokio::sync::mpsc::channel(sys::fswatch::QUEUE);
    std::thread::spawn(move || {
        while let Some(ev) = w.recv() {
            let e = presence::FsEvent {
                path: ev.path,
                overflow: ev.overflow,
            };
            if tx.blocking_send(e).is_err() {
                break;
            }
        }
        w.close();
    });
    Ok(presence::FsSource::new(rx, move || cancel.cancel()))
}

/// The `device.started` payload (a Go map: keys sorted).
#[derive(Serialize)]
struct DeviceStarted<'a> {
    headless: bool,
    name: &'a str,
    os: &'a str,
    version: &'a str,
}

/// The `agent.status` payload (a Go map: keys sorted).
#[derive(Serialize)]
struct AgentStatus<'a> {
    key: &'a str,
    name: &'a str,
    #[serde(rename = "nameSource", skip_serializing_if = "str::is_empty")]
    name_source: &'a str,
    #[serde(rename = "permissionMode", skip_serializing_if = "str::is_empty")]
    permission_mode: &'a str,
    status: &'a str,
}

/// The `agent.gone` payload.
#[derive(Serialize)]
struct AgentGone<'a> {
    key: &'a str,
    reason: &'a str,
}

/// The `device.name` payload (keys sorted, as the colour row's).
#[derive(Serialize)]
struct DeviceName<'a> {
    at_device: &'a str,
    #[serde(rename = "deviceId")]
    device_id: &'a str,
    name: &'a str,
}

/// The `device.color` payload (a Go map: keys sorted).
#[derive(Serialize)]
struct DeviceColor<'a> {
    at_device: &'a str,
    color: &'a str,
    #[serde(rename = "deviceId")]
    device_id: &'a str,
}

/// Writes an `agent.avatar` row and folds it straight away.
fn set_avatar(
    s: &Services,
    name: &str,
    face: Option<&coord::avatar::Avatar>,
) -> Result<(), String> {
    let name = coord::avatar::name_key(name);
    if name.is_empty() {
        return Err("a face needs an agent name".into());
    }
    let json = face.map(coord::avatar::to_json).unwrap_or_default();
    let ev = s
        .store
        .append(
            "irori",
            coord::avatar::KIND,
            &coord::avatar::RowOut {
                avatar: &json,
                name: &name,
            },
        )
        .map_err(|e| e.to_string())?;
    if s.faces.apply(&ev) {
        s.hub.publish(CoreEvent::Devices);
    }
    // Shared boards' agent cards carry the new face. Always: the store's
    // subscriber may have folded the row first, and it writes no cards
    // when the row is its own, so a skipped sync here lost the face.
    s.sync_card_status();
    Ok(())
}

/// The `person.color` payload (card #385).
#[derive(Serialize)]
struct PersonColor<'a> {
    at_device: &'a str,
    color: &'a str,
}

/// The `assistant.name` payload (card #1045).
#[derive(Serialize)]
struct AssistantNameRow<'a> {
    name: &'a str,
    by: &'a str,
    at: i64,
}

/// The `person.avatar` payload (card #393).
#[derive(Serialize)]
struct PersonAvatarRow<'a> {
    #[serde(rename = "atDevice")]
    at_device: &'a str,
    avatar: &'a str,
}

impl Core {
    /// Brings everything up (Go `Start` without the net, pairing, boards,
    /// home server and API listener). The lockfile is written by
    /// [`Core::write_lockfile`] once the API listens.
    pub fn start(mut opts: Options) -> Result<Core, StartError> {
        // Card #232: how long each part of the start takes, in one log line.
        let mut laps = StartLaps::new();
        let dir = opts.data_dir.clone();
        lockfile::create_dir_all_private(&dir).map_err(|e| other("", e))?;
        if let Ok(lf) = lockfile::read(&dir)
            && lf.pid != lockfile::own_pid()
        {
            return Err(StartError::AlreadyRunning { pid: lf.pid });
        }
        match crate::reset::reset_if_asked(&dir) {
            Ok(Some(dst)) => logf(format_args!(
                "reset: this device's log and identity moved to {}; it starts as a new device",
                dst.display()
            )),
            Ok(None) => {}
            Err(e) => return Err(other("", e)),
        }
        let dev = DeviceJson::load_or_create(&dir).map_err(|e| other("", e))?;
        // The device key signs every row; the person and its cert chain
        // decide which "user:" keys are the owner's.
        let ident = Arc::new(
            Identity::load(&dir, dev, opts.keystore.take()).map_err(|e| other("identity", e))?,
        );
        laps.lap("identity");
        let device = ident.self_device();
        let me = device.id.clone();
        let person = ident.current();
        let chain = Arc::new(Chain::new(&person.id, &person.root_device_id, &me));

        let now: NowFn = opts
            .now
            .clone()
            .unwrap_or_else(|| Arc::new(presence::now_local));
        let wall = {
            let n = now.clone();
            move || n().to_system()
        };
        let clock = Arc::new(store::Clock::with_now(Box::new(wall.clone())));
        let st = Arc::new(
            Store::open_with(
                dir.join("irori.db"),
                &me,
                clock,
                store::OpenOptions {
                    signer: Some(ident.clone()),
                    verifier: Some(chain.clone()),
                    now: opts.now.as_ref().map(|_| Arc::new(wall) as store::WallFn),
                    ..Default::default()
                },
            )
            .map_err(|e| other("open event log", e))?,
        );
        laps.lap("store");
        let role = if opts.headless {
            irori_identity::objects::ROLE_NODE
        } else {
            irori_identity::objects::ROLE_DESKTOP
        };
        if let Err(e) = ident.ensure_self_cert(&*st, &chain, role) {
            let _ = st.close();
            return Err(other("identity", e));
        }
        // A device that paired but stopped before its first sync learns the
        // person rows it was handed again (pair-rows.json, Go core.go:158).
        if let Err(e) = irori_pair::learn_saved(&dir, &st, &chain) {
            logf(format_args!("pairing rows: {e}"));
        }
        laps.lap("cert");
        let fail = |st: &Store, e: StartError| {
            let _ = st.close();
            e
        };

        let resolver = Arc::new(capture::repo::Resolver::new());
        let repo_key: Arc<dyn Fn(&str) -> String + Send + Sync> = {
            let r = resolver.clone();
            Arc::new(move |d: &str| r.key(d))
        };
        let names = Arc::new(Namer::new(&dir, &me));
        let name_themes = Arc::new(name_themes::NameThemes::load(&st));
        {
            let t = name_themes.clone();
            names.set_themes_off(move || t.off());
        }
        let agent_names = Arc::new(crate::agentnames::AgentNames::new(&dir));
        let usage = crate::usage::Usage::new(&dir);
        // Idle sessions whose wakes go unread show as not answering (#110).
        let answering = Arc::new(crate::answering::Answering::default());
        let mut popts = presence::Options {
            claude_dir: opts.claude_dir.to_string_lossy().into_owned(),
            device_id: me.clone(),
            repo_key: Some(repo_key.clone()),
            names: Some(names.clone()),
            status: Some({
                let a = answering.clone();
                Arc::new(move |c: &presence::AgentCard| a.status(c))
            }),
            watch: Some(Arc::new(fs_watch)),
            // Card #783: the window assistant's headless `claude -p` writes
            // a session file too; it is no agent.
            skip_cwd: assistant_skip(&dir),
            ..Default::default()
        };
        if let Some(edit) = &opts.presence {
            edit(&mut popts);
        }
        let presence = Arc::new(presence::Watcher::new(popts));
        laps.lap("presence");

        st.append(
            "irori",
            kinds::DEVICE_STARTED,
            &DeviceStarted {
                headless: opts.headless,
                name: &device.name,
                os: &device.os,
                version: &opts.version,
            },
        )
        .map_err(|e| fail(&st, other("", e)))?;
        laps.lap("started-row");

        let revoked = {
            let ch = chain.clone();
            Arc::new(move |d: &str| ch.cut(d).is_some()) as Arc<dyn Fn(&str) -> bool + Send + Sync>
        };
        let is_owner = {
            let ch = chain.clone();
            Arc::new(move |k: &str| ch.is_owner(k)) as Arc<dyn Fn(&str) -> bool + Send + Sync>
        };
        let members = Arc::new(if opts.member_grace.is_zero() {
            Members::new(&me)
        } else {
            Members::with_clock(&me, opts.member_grace, presence::now_local)
        });
        let local: LocalFn = {
            let p = presence.clone();
            let mb = members.clone();
            Arc::new(move || {
                let mut cs = p.snapshot();
                cs.extend(mb.cards());
                cs.into_iter().map(roster_card).collect()
            })
        };

        // One roster across devices (Go core.go:188): this device's presence
        // plus every other device's agent rows. The mesh node is built below;
        // its liveness is read per call, offline until it exists.
        let roster_sub = Arc::new(st.subscribe());
        let node_cell: Arc<OnceLock<Arc<mesh::Node>>> = Arc::new(OnceLock::new());
        let device_names = Arc::new(coord::devname::Names::new());
        load_device_names(&st, &device_names);
        let roster = Roster::new(coord::roster::Options {
            store: st.clone(),
            self_id: me.clone(),
            local: Some(local.clone()),
            liveness: Some({
                let cell = node_cell.clone();
                Arc::new(move |dev: &str| match cell.get() {
                    Some(n) => n.liveness(dev).to_string(),
                    None => coord::roster::OFFLINE.to_string(),
                })
            }),
            seen: Some({
                let cell = node_cell.clone();
                Arc::new(move |dev: &str| {
                    cell.get()
                        .and_then(|n| n.last_seen(dev))
                        .map(gocompat::time::GoTime::from_system)
                        .unwrap_or_default()
                })
            }),
            via: Some({
                let cell = node_cell.clone();
                let chain = chain.clone();
                let names = device_names.clone();
                Arc::new(move |dev: &str| {
                    let via = cell.get().map(|n| n.device_read(dev).via);
                    via_name(&chain, &names, &via.unwrap_or_default())
                })
            }),
            revoked: Some(revoked.clone()),
        })
        .map_err(|e| fail(&st, other("roster", e)))?;
        laps.lap("roster");
        {
            let r = roster.clone();
            names.set_mesh(move || r.agents());
            // Names agents went by lately, gone ones too (card #224).
            let an = agent_names.clone();
            names.set_seen(move || {
                an.seen()
                    .into_iter()
                    .map(|(k, n, at)| (k, n, gocompat::time::GoTime::unix(at, 0)))
                    .collect()
            });
        }

        // The inbox's hooks reach coord and capture, which open after it.
        let coord_cell: Arc<OnceLock<Arc<Coord>>> = Arc::new(OnceLock::new());
        // The inbox its own on_recipient_gone routes a notice through (card
        // #452), weak so the inbox doesn't keep itself alive.
        let inbox_cell: Arc<OnceLock<std::sync::Weak<Inbox>>> = Arc::new(OnceLock::new());
        // The Services, once built: the inbox's Notes hook reads notes from
        // every log (card #412).
        let services_cell: Arc<OnceLock<std::sync::Weak<Services>>> = Arc::new(OnceLock::new());
        let capture_cell: Arc<OnceLock<capture::Capture>> = Arc::new(OnceLock::new());
        let iopts = inbox::Options {
            device_id: me.clone(),
            now: Some(now.clone()),
            pr_owners: Some({
                let c = capture_cell.clone();
                Arc::new(move |k: &str| c.get().map(|c| c.pr_owners(k)).unwrap_or_default())
            }),
            is_owner: Some(is_owner.clone()),
            spawned: Some({
                let c = coord_cell.clone();
                Arc::new(move |k: &str| {
                    coord::with_inbox_lock_held(|| c.get().is_some_and(|co| co.is_spawned(k)))
                })
            }),
            on_recipient_gone: Some({
                let c = coord_cell.clone();
                let ibc = inbox_cell.clone();
                let an = agent_names.clone();
                Arc::new(move |m: &inbox::Message, to: &str| {
                    let Some(co) = c.get() else {
                        return Err(inbox::Error::sentinel(inbox::ErrorKind::NoCard));
                    };
                    match notice::record_gone(co, m, to) {
                        Err(e) if e.is(inbox::ErrorKind::NoCard) => {}
                        r => return r,
                    }
                    // A spawned agent that has finished (card #452): its
                    // messages land on the card it was spawned for and
                    // wake that card's holders; nothing starts an agent.
                    let name = an
                        .seen()
                        .into_iter()
                        .find(|(k, _, _)| k == to)
                        .map(|(_, n, _)| n)
                        .unwrap_or_default();
                    match notice::record_for_finished(co, m, to, &name)? {
                        Some((tid, n)) => {
                            if !n.to.is_empty()
                                && let Some(ib) = ibc.get().and_then(|w| w.upgrade())
                            {
                                let _ = notice::route(&ib, &inbox::system_sender(), &n);
                            }
                            Ok(tid)
                        }
                        // A comment or blocker bridged from a shared board
                        // is on that board's card already (card #478): it
                        // closes there, never moved to the person, where
                        // each agent's copy showed as a DM.
                        None => tools::notice::closed_on_board(m)
                            .ok_or_else(|| inbox::Error::sentinel(inbox::ErrorKind::NoCard)),
                    }
                })
            }),
            on_passed: Some({
                let c = coord_cell.clone();
                Arc::new(move |m: &inbox::Message, to: &str| match c.get() {
                    Some(co) => notice::record_passed(co, m, to),
                    None => Ok(()),
                })
            }),
            stale: Some({
                let c = coord_cell.clone();
                Arc::new(move |d: &inbox::Delivery| {
                    coord::with_inbox_lock_held(|| {
                        c.get()
                            .map(|co| notice::stale_notice(co, d))
                            .unwrap_or_default()
                    })
                })
            }),
            off_side: Some({
                let c = coord_cell.clone();
                Arc::new(move |m: &inbox::Message, to: &str| {
                    coord::with_inbox_lock_held(|| {
                        c.get()
                            .is_some_and(|co| co.off_side(&m.reason, &m.ref_, to))
                    })
                })
            }),
            notes: Some({
                let c = services_cell.clone();
                Arc::new(move |agent: &str, refs: &[String]| {
                    c.get()
                        .and_then(|w| w.upgrade())
                        .map(|s| coord::with_inbox_lock_held(|| s.wake_notes(agent, refs)))
                        .unwrap_or_default()
                })
            }),
            reattach_grace: opts.reattach_grace,
            ..Default::default()
        };
        let ib = Arc::new(
            Inbox::new(st.clone(), Arc::new(MeshAgents(roster.clone())), iopts)
                .map_err(|e| fail(&st, other("inbox", e)))?,
        );
        let _ = inbox_cell.set(Arc::downgrade(&ib));
        laps.lap("inbox");

        // The Services the private coord's busy hook reads the boards
        // through (card #422), set once they exist; until then only the
        // private board counts.
        let svc_cell: Arc<OnceLock<Weak<Services>>> = Arc::new(OnceLock::new());
        let spawn_state = spawn::SpawnState::default();
        let _ = spawn::load_spawn(&st, &spawn_state);
        ib.set_reviews_unasked(
            !spawn_state
                .reviews_off
                .load(std::sync::atomic::Ordering::SeqCst),
        );
        let call_settings = call_settings::CallSettings::load(&st);
        let naming = thread_names::Naming::load(&st);
        let weighing = card_weights::Weighing::load(&st);
        let huddle_switches = huddle_settings::HuddleSwitches::load(&st);
        let co = Arc::new(
            Coord::new(
                st.clone(),
                Some(Arc::new(CoordAgents {
                    local: local.clone(),
                    mesh: roster.clone(),
                    names: agent_names.clone(),
                })),
                coord::Options {
                    now: Some(now.clone()),
                    is_owner: Some(is_owner.clone()),
                    revoked: Some(revoked.clone()),
                    done_check: true,
                    // Whether an agent read another person's text (card
                    // #1075): the inbox keeps the flag beside its lock, so
                    // this write-time check never waits on it.
                    others_of: Some({
                        let sawn = ib.saw_others_handle();
                        Arc::new(move |actor: &str| {
                            actor
                                .strip_prefix(store::kinds::AGENT_ACTOR_PREFIX)
                                .is_some_and(|k| sawn.has(k))
                        })
                    }),
                    // The private board's spawned-agent switch (card #446).
                    spawning: Some({
                        let on = spawn_state.on.clone();
                        let def = spawn_state.private_default.clone();
                        Arc::new(move || spawn::SpawnState::switch(&on, &def, ""))
                    }),
                    agent_devices: Some(spawn_state.agent_devices_fn()),
                    // The limit the coord reads (card #576): from the
                    // same map, so a change holds at once.
                    crash_limit: Some(spawn::SpawnState::limit_fn(&spawn_state, "", |s| {
                        s.crash_limit
                    })),
                    // A terminal agent takes unheld cards, asks and threads
                    // only with the person's "Offer it cards" on (card
                    // #453); a spawned agent's rule is the coord's.
                    offers: Some({
                        let offers = spawn_state.offers.clone();
                        Arc::new(move |key: &str| {
                            offers
                                .read()
                                .unwrap_or_else(|e| e.into_inner())
                                .get(key)
                                .copied()
                                .unwrap_or(false)
                        })
                    }),
                    // A working card on a shared board counts here too:
                    // one card at a time across logs (card #422).
                    busy_elsewhere: Some({
                        let cell = svc_cell.clone();
                        Arc::new(move |agent: &str| {
                            cell.get()?.upgrade()?.busy_outside(agent, None)
                        })
                    }),
                    // Personal's boards (card #558), read from this log as
                    // a space's are from its own.
                    board_tag: Some({
                        let st = Arc::downgrade(&st);
                        Arc::new(move |board: &str| {
                            let st = st.upgrade()?;
                            spaces::board_tag_in(&st, board, &|_| true)
                        })
                    }),
                    board_tags: Some({
                        let st = Arc::downgrade(&st);
                        Arc::new(move || {
                            st.upgrade()
                                .map_or_else(Vec::new, |st| spaces::board_tags_in(&st))
                        })
                    }),
                    board_view: Some({
                        let st = Arc::downgrade(&st);
                        Arc::new(move || {
                            let st = st.upgrade()?;
                            spaces::with_personal_rules(|r| {
                                spaces::board_list_in(&st, spaces::PERSONAL_BOARD, r)
                            })
                        })
                    }),
                    ..Default::default()
                },
            )
            .map_err(|e| fail(&st, other("coord", e)))?,
        );
        let _ = coord_cell.set(co.clone());
        laps.lap("coord");

        let scrubber = Arc::new(scrub::Scrubber::new::<&str>(&[]));
        let blobs =
            blob::Store::open(dir.join("blobs")).map_err(|e| fail(&st, other("blobs", e)))?;
        let mut copts = capture::Options::new(
            &opts.claude_dir.to_string_lossy(),
            &dir.to_string_lossy(),
            &me,
            Arc::new(Scrub(scrubber.clone())),
            Arc::new(Blobs(blobs)),
        );
        copts.now = now.clone();
        copts.repo_key = repo_key.clone();
        if let Some(edit) = &opts.capture {
            edit(&mut copts);
        }
        let cap = capture::Capture::new(
            st.clone(),
            Some(Arc::new(LocalAgents(local.clone()))),
            copts,
        )
        .map_err(|e| fail(&st, other("capture", e)))?;
        let _ = capture_cell.set(cap.clone());
        laps.lap("capture");
        // Capture's search dispatches the Board and inbox kinds to them
        // (capture.md §2.7).
        cap.add_source(co.clone());
        cap.add_source(ib.clone());

        // Approvals as rows: the asking device decides, and a revive from
        // another device waits for one.
        let reviver: Arc<Mutex<Option<ReviveFn>>> = Arc::new(Mutex::new(None));
        let mut aopts = coord::approvals::Options::new(st.clone(), &me);
        aopts.is_owner = Some(is_owner.clone());
        aopts.now = Some(now.clone());
        aopts.revive = Some({
            let rv = reviver.clone();
            let me = me.clone();
            Arc::new(move |target: &str| revive_for_ask(&rv, &me, target))
        });
        aopts.name = Some({
            let r = roster.clone();
            Arc::new(move |key: &str| {
                r.agents()
                    .into_iter()
                    .find(|a| a.key == key)
                    .map(|a| a.name)
                    .unwrap_or_default()
            })
        });
        let asks_sub = Arc::new(st.subscribe());
        let asks = coord::approvals::Book::new(aopts).map_err(|e| fail(&st, other("asks", e)))?;
        laps.lap("asks");

        let files =
            blob::Files::open(dir.join("files")).map_err(|e| fail(&st, other("attachments", e)))?;
        let colors = coord::devcolor::Picks::new();
        let person_color = coord::devcolor::PersonPick::new();
        let faces = coord::avatar::Faces::new();
        let person_avatar = coord::person_avatar::PersonAvatar::new();
        let assistant_name = coord::assistant_name::AssistantName::new();
        let colors_sub = Arc::new(st.subscribe());
        load_colors(
            &st,
            &colors,
            &person_color,
            &faces,
            &person_avatar,
            &assistant_name,
        );
        let talk = Arc::new(coord::talk::Talk::new(Some(st.clone()), {
            let now = now.clone();
            Some(Arc::new(move || now().utc()))
        }));
        // Before Talk folds a row and before any space opens (card #538).
        let dm_spaces = Arc::new(boards_own::DmSpaces::default());
        if let Err(e) = dm_spaces.load(&st) {
            logf(format_args!("dm spaces: load: {e}"));
        }
        if let Err(e) = talk.load() {
            logf(format_args!("talk: load: {e}"));
        }
        let notifications = Arc::new(coord::notifications::History::new());
        load_notifications(&st, &notifications);
        let assistant_chat = Arc::new(assistant_chat::Fold::new());
        // What the chat worker counts as folded: every row before the load.
        let chat_heard = st.vector().unwrap_or_default();
        assistant_chat::load(&st, &assistant_chat);
        laps.lap("colors+names");
        let decisions_sub = Arc::new(st.subscribe());

        // The Core's stop; the shared boards' threads end with it too.
        let stop = Cancel::new();
        let board_set = Arc::new(Boards::new(stop.clone()));

        // The mesh and pairing share one iroh endpoint (crate::meshnode).
        // The home server (mesh.md §3.3) runs only where it is switched on,
        // and only while the mesh has a peer or a pairing window is open (Go
        // core.go:290); its relay port feeds the endpoint (Go Net.SetLocal).
        // Pairing (Go core.go:284-288) opens its own room on the transport,
        // only while a window is open. Add is window-only.
        // The home server comes after the Pairer (it asks whether a window
        // is open); the Pairer reads it back through this cell.
        let home_cell: Arc<OnceLock<home::Server>> = Arc::new(OnceLock::new());
        let pairing = {
            let own_rt = match &opts.runtime {
                Some(_) => None,
                None => Some(
                    tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .thread_name("irori-net")
                        .enable_all()
                        .build()
                        .map_err(|e| fail(&st, other("net runtime", e)))?,
                ),
            };
            let handle = opts
                .runtime
                .clone()
                .unwrap_or_else(|| own_rt.as_ref().expect("own runtime").handle().clone());
            let _enter = handle.enter();
            let guests = irori_net::guest::GuestWindows::default();
            let net = match &opts.pair_net {
                Some(f) => f(&ident, &chain, &guests),
                None => crate::devices::default_net(&ident, &chain, &guests),
            };
            // n0's public relays (card #328): on unless this device turned
            // them off; set before anything binds the endpoint. A test's own
            // transport (`pair_net`) stays off the internet.
            if opts.pair_net.is_none() {
                net.public_relays_flag()
                    .set(crate::publicrelays::load(&dir));
            }
            // The mesh Node on the same endpoint (crate::meshnode).
            let mesh = crate::meshnode::Mesh::new(
                net.clone(),
                crate::meshnode::Parts {
                    ident: ident.clone(),
                    chain: chain.clone(),
                    store: st.clone(),
                    files: blob::Files::open(dir.join("files")).ok().map(Arc::new),
                    data_dir: dir.clone(),
                    version: opts.version.clone(),
                    now: {
                        let n = now.clone();
                        Some(Arc::new(move || n().to_system()))
                    },
                    ignored: {
                        let (ib, co, asks, roster) =
                            (ib.clone(), co.clone(), asks.clone(), roster.clone());
                        Some(Arc::new(move || ignored_by(&ib, &co, &asks, &roster)))
                    },
                },
            );
            let mut po =
                irori_pair::Options::new(Arc::new(net), ident.clone(), chain.clone(), st.clone());
            po.data_dir = Some(dir.clone());
            po.kick = Some({
                let n = mesh.node.clone();
                Arc::new(move || n.kick())
            });
            po.servers = Some(mesh.hints());
            po.server_addr = Some({
                let (h, servers) = (home_cell.clone(), mesh.servers.clone());
                Arc::new(move || server_addr(h.get(), &servers))
            });
            po.window = crate::devices::window_or_default(opts.pair_window);
            po.logf = Some(Arc::new(|l: &str| crate::log(l)));
            (
                crate::devices::Pairing {
                    pairer: irori_pair::Pairer::new(po),
                    guests,
                    handle: handle.clone(),
                    own_rt: Mutex::new(own_rt),
                },
                mesh,
            )
        };
        let (pairing, mesh) = pairing;
        let _ = node_cell.set(mesh.node.clone());
        laps.lap("mesh");

        let invites = invites::Invites::default();
        let home = (!opts.no_home).then(|| {
            let mut hc = home::Config::new(dir.clone(), ident.clone(), chain.clone(), st.clone());
            let node = mesh.node.clone();
            let pr = pairing.pairer.clone();
            // Go core.go:290: another device, or an open pairing window;
            // stage 2 (B1): or a shared board with another person on it,
            // whose members this server relays for.
            // And while this device is the inviter of an open board invite,
            // so the link can name its relay.
            let (held, hosting) = (board_set.clone(), invites.hosting.clone());
            hc.wanted = Some(Arc::new(move || {
                pr.status().open()
                    || node.has_peer()
                    || held.shared_with_others()
                    || hosting.load(std::sync::atomic::Ordering::SeqCst)
            }));
            hc.board_admit = Some(mesh.net.board_members().relay_admit());
            let m = mesh.clone();
            hc.on_relay = Some(Arc::new(move |up| m.set_local_relay(up)));
            hc.guests = Some(pairing.guests.clone());
            if opts.now.is_some() {
                let n = now.clone();
                hc.now = Some(Arc::new(move || n().to_system()));
            }
            hc.logf = Some(Arc::new(|l: &str| crate::log(l)));
            if let Some(edit) = &opts.home {
                edit(&mut hc);
            }
            home::Server::new(hc)
        });
        if let Some(h) = &home {
            let _ = home_cell.set(h.clone());
            mesh.set_home(h.clone());
        }

        let token = lockfile::new_token().map_err(|e| fail(&st, other("", e)))?;
        let assistant = lockfile::new_token().map_err(|e| fail(&st, other("", e)))?;
        let s = Arc::new(Services {
            data_dir: dir,
            version: opts.version.clone(),
            headless: opts.headless,
            now,
            device,
            ident,
            chain,
            store: st,
            presence,
            roster,
            inbox: ib,
            coord: co,
            capture: cap,
            asks,
            names,
            agent_names,
            usage,
            pr_checker: opts.pr_checker.clone(),
            pr_watch: Default::default(),
            desk_lister: opts.desk_lister.clone(),
            desk_watch: Default::default(),
            desk_merges: Default::default(),
            desks_due: Default::default(),
            clear_builds: opts.clear_builds,
            builds_looked: Default::default(),
            builds_busy: Default::default(),
            this: services_cell.clone(),
            members,
            names_kick: Kick::new(),
            answering,
            asks_kick: Kick::new(),
            colors,
            person_color,
            faces,
            person_avatar,
            assistant_name,
            device_names,
            talk,
            notifications,
            assistant_chat,
            files,
            scrubber,
            secrets: Mutex::new(Vec::new()),
            token: token.clone(),
            assistant: assistant.clone(),
            assistant_screen: Mutex::new(String::new()),
            assistant_grants: Default::default(),
            reviver,
            spawn: spawn_state,
            hub: Hub::new(),
            rearm: Kick::new(),
            home,
            notifier: CoreNotifier(
                opts.notifier
                    .clone()
                    .unwrap_or_else(notify::Notifier::headless),
            ),
            pairing,
            revoke: Some(opts.revoke.clone().unwrap_or_else(|| mesh.revoke_fn())),
            mesh,
            boards: board_set,
            dm_spaces,
            dms: dm_spaces::Dms::default(),
            calls: calls::Calls::new(
                opts.call_platform.clone(),
                opts.call_audio.clone(),
                opts.call_transcriber.clone(),
            ),
            call_settings,
            naming,
            assistant_home: assistant_home::HomeState::default(),
            chat: assistant_turns::ChatQueue::new(chat_heard),
            weighing,
            huddle_switches,
            asst: Default::default(),
            held: Default::default(),
            hosting: Default::default(),
            feed: Default::default(),
            name_themes,
            invites,
        });
        let _ = svc_cell.set(Arc::downgrade(&s));
        let _ = services_cell.set(Arc::downgrade(&s));
        // A new name never takes a word another member's live agent on a
        // shared board has (card #472).
        {
            let w = Arc::downgrade(&s);
            s.names.set_others(move || {
                w.upgrade()
                    .map(|s| s.board_others().into_iter().map(|a| a.card).collect())
                    .unwrap_or_default()
            });
        }
        // The requests naming this device its log holds before any sync
        // are never started (card #455).
        s.hold_requests_at_start();
        // A space's channels go in its log (card #392).
        s.talk
            .set_spaces(Arc::new(spaces::SpaceTalk(Arc::downgrade(&s))));
        let _ = s.boards.kicker.set({
            let m = Arc::downgrade(&s.mesh);
            Arc::new(move || {
                if let Some(m) = m.upgrade() {
                    m.kick_boards();
                    // A board opened or its secret moved: the person's
                    // other devices get it on board.secret.
                    m.node.boards_changed();
                }
            })
        });
        s.mesh
            .node
            .set_own_boards(Some(Arc::new(invites::OwnBoardSecrets(Arc::downgrade(&s)))));
        // Before any space opens: every mesh carries the DM hooks (card #538).
        {
            let w = Arc::downgrade(&s);
            s.mesh.set_dms(
                Arc::new(dm_spaces::DmBridge(w.clone())),
                Arc::new(move |id: &str| w.upgrade().is_some_and(|s| s.is_dm_space(id))),
            );
            // And the call hooks (card #654).
            s.mesh
                .set_calls(Arc::new(calls::CallBridge(Arc::downgrade(&s))));
            // A Personal channel's huddle rings on the private mesh (#663).
            s.mesh
                .node
                .set_own_calls(Some(Arc::new(calls::CallBridge(Arc::downgrade(&s)))));
        }
        // A pairing window starts the home server at once (Go Notify).
        if let Some(h) = s.home.clone() {
            s.pairing
                .pairer
                .set_notify(Some(Arc::new(move || h.kick())));
        }
        s.add_secret(&token);
        s.add_secret(&assistant);
        laps.lap("services");

        let tools = tools::Set::new(Arc::new(ToolsView(s.clone())), Some(s.now.clone()));
        let core = Core {
            s: s.clone(),
            tools,
            stop,
            stop_async: CancellationToken::new(),
            threads: Mutex::new(Vec::new()),
            watch: Mutex::new(None),
            terms: Arc::new(term::Terms::new()),
            closed: AtomicBool::new(false),
        };
        // Other devices' queued deliveries get this device's own hold
        // decision before anything serves an attach stream that could wake
        // them.
        s.inbox.catch_up();
        s.follow_assistant_home();
        laps.lap("catch-up");
        let changes = Arc::new(s.presence.subscribe());
        core.start_loops(
            &opts,
            changes,
            Subs {
                roster: roster_sub,
                asks: asks_sub,
                colors: colors_sub,
                decisions: decisions_sub,
            },
        );
        laps.lap("loops");
        // Every notification shown is kept in the history (card #377). Weak:
        // the Notifier lives in the services.
        {
            let w = Arc::downgrade(&s);
            s.notifier.0.set_recorder(move |n| {
                if let Some(s) = w.upgrade() {
                    s.record_notification(n);
                }
            });
        }
        // The cards' Watch once the services run (Go core.go: notify.Watch
        // after Start); `None` when headless.
        *core.watch.lock().unwrap_or_else(|e| e.into_inner()) = notify::watch(
            &s.notifier.0,
            notify::Sources {
                inbox: Some(s.inbox.clone()),
                board: Some(s.coord.clone()),
                asks: Some(s.asks.clone()),
                user: s.user_sender().key,
                is_owner: Some({
                    let ch = s.chain.clone();
                    Arc::new(move |k: &str| ch.is_owner(k))
                }),
                name: Some({
                    // A gone agent keeps the last name it went by (cards
                    // #98, #483): a returned card's agent has left the roster.
                    let s = s.clone();
                    Arc::new(move |key: &str| s.agent_name(key))
                }),
                repo_name: Some({
                    let s = s.clone();
                    Arc::new(move |key: &str| {
                        let roots = s.capture.repo_roots(key);
                        inbox::watch::repo_short_name(key, roots.first().map(String::as_str))
                    })
                }),
                room: Some(Arc::new(WatchRoom(s.clone()))),
                file: Some({
                    let f = s.files.clone();
                    Arc::new(move |id: &str| f.has(id).then(|| f.path(id)))
                }),
                seen: Some(Arc::new(notify::SeenFile::new(&s.data_dir))),
            },
        );
        laps.lap("watch");
        laps.report();
        Ok(core)
    }

    fn spawn(&self, name: &str, f: impl FnOnce() + Send + 'static) {
        let h = std::thread::Builder::new()
            .name(format!("irori-{name}"))
            .spawn(f)
            .expect("spawn a core thread");
        self.threads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(h);
    }

    /// Runs `fut` on `rt`, or on a current-thread runtime of its own; a
    /// thread waits for it either way, so [`Core::close`] waits for it.
    fn spawn_async(
        &self,
        name: &str,
        rt: Option<&tokio::runtime::Handle>,
        fut: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        match rt {
            Some(h) => {
                let (tx, rx) = std::sync::mpsc::channel::<()>();
                h.spawn(async move {
                    fut.await;
                    let _ = tx.send(());
                });
                self.spawn(name, move || {
                    let _ = rx.recv();
                });
            }
            None => {
                let label = name.to_string();
                self.spawn(
                    name,
                    move || match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt.block_on(fut),
                        Err(e) => logf(format_args!("{label}: {e}")),
                    },
                )
            }
        }
    }

    /// Ends `sub` when the Core stops.
    fn on_stop<T: Send + Sync + 'static>(
        &self,
        sub: &Arc<T>,
        end: impl Fn(&T) + Send + 'static,
    ) -> sys::cancel::Registration {
        let s = sub.clone();
        self.stop.on_cancel(move || end(&s))
    }

    fn start_loops(&self, opts: &Options, changes: Arc<presence::Subscription>, subs: Subs) {
        let s = &self.s;

        // Presence.
        {
            let w = s.presence.clone();
            let ct = self.stop_async.clone();
            self.spawn_async(
                "presence",
                opts.runtime.as_ref(),
                async move { w.run(ct).await },
            );
        }

        // The home server; it removes its port mappings when it stops, and
        // close waits for it, so node.addr is written before the log closes.
        if let Some(h) = s.home.clone() {
            let ct = self.stop_async.clone();
            self.spawn_async(
                "home",
                opts.runtime.as_ref(),
                async move { h.run(ct).await },
            );
        }

        // The shared boards on this device, and the person's board.joined
        // and board.forgot rows from every device (Go openBoards,
        // followOwnBoards). Card #232: the boards open on this thread, not
        // in the start, so the window shows the private cards first and
        // each board fills in as it opens (a big board takes seconds on a
        // phone). The subscription is taken first, so no row is missed.
        // Card #392: v0.10's boards/ becomes spaces/ before anything can
        // open, create or join one (renames only: quick even on a phone).
        if let Err(e) = boards::migrate_board_files(&s.data_dir) {
            logf(format_args!("spaces: moving the board logs: {e}"));
        }
        s.boards.opening_pending();
        {
            let sub = Arc::new(s.store.subscribe());
            let reg = self.on_stop(&sub, |s| s.cancel());
            let s = s.clone();
            self.spawn("boards", move || {
                let _reg = reg;
                let t = Instant::now();
                s.open_boards();
                logf(format_args!(
                    "start: boards opened in {} ms",
                    t.elapsed().as_millis()
                ));
                s.follow_own_boards(&sub);
            });
        }

        // Card #580: small files other devices offer, fetched unasked.
        if let Some(cap) = opts.prefetch_files {
            let sub = Arc::new(s.store.subscribe());
            let reg = self.on_stop(&sub, |s| s.cancel());
            let p = crate::prefetch::Prefetch {
                mesh: s.mesh.clone(),
                store: s.store.clone(),
                files: s.files.clone(),
                now: s.now.clone(),
                me: s.device.id.clone(),
                cap,
            };
            let ct = self.stop_async.clone();
            let rt = s.pairing.handle.clone();
            self.spawn_async("prefetch", Some(&rt), async move {
                let _reg = reg;
                p.run(sub, ct).await
            });
        }

        // The DM hand-offs (card #538): the mesh's hooks only queue work.
        {
            let s = s.clone();
            let reg = self.stop.on_cancel({
                let s = s.clone();
                move || s.dms.close()
            });
            self.spawn("dms", move || {
                let _reg = reg;
                s.run_dm_jobs();
            });
        }

        // The mesh Node, its Servers fold and the address book, on the
        // net runtime; it joins nothing until there is a second device.
        {
            let (m, st, ct) = (s.mesh.clone(), s.store.clone(), self.stop_async.clone());
            let rt = s.pairing.handle.clone();
            self.spawn_async("mesh", Some(&rt), async move { m.run(st, ct).await });
        }

        // Capture tails and imports; backfill runs inside run.
        {
            let cap = s.capture.clone();
            let stop = self.stop.child();
            self.spawn("capture", move || cap.run(&stop));
        }

        // Presence changes become log rows.
        {
            let reg = self.on_stop(&changes, |c| c.cancel());
            let s = s.clone();
            self.spawn("record", move || {
                let _reg = reg;
                s.record(|| changes.recv());
            });
        }

        // The mesh roster folds other devices' agent rows.
        {
            let sub = subs.roster;
            let reg = self.on_stop(&sub, |s| s.cancel());
            let r = s.roster.clone();
            self.spawn("roster", move || {
                let _reg = reg;
                while let Some(ev) = sub.recv() {
                    r.handle(&ev);
                }
            });
        }

        // The asks: other devices' rows, and the asking device's duties.
        {
            let sub = subs.asks;
            let reg = self.on_stop(&sub, |s| s.cancel());
            let b = s.asks.clone();
            self.spawn("asks", move || {
                let _reg = reg;
                b.tick();
                let mut t = Ticker::new(Duration::from_secs(1));
                loop {
                    match sub.recv_timeout(t.until()) {
                        Ok(None) => return,
                        Ok(Some(ev)) => b.handle(&ev),
                        Err(()) => {}
                    }
                    if t.take() {
                        b.tick();
                    }
                }
            });
        }

        // coord's Follow: other devices' rows, and its 1 s holder duties.
        {
            let sub = Arc::new(s.store.subscribe());
            let reg = self.on_stop(&sub, |s| s.cancel());
            let co = s.coord.clone();
            self.spawn("coord", move || {
                let _reg = reg;
                if let Err(e) = co.catch_up() {
                    logf(format_args!("coord: {e}"));
                }
                let mut t = Ticker::new(Duration::from_secs(1));
                loop {
                    match sub.recv_timeout(t.until()) {
                        Ok(None) => return,
                        Ok(Some(ev)) => {
                            if let Err(e) = co.handle(&ev) {
                                logf(format_args!("coord: {e}"));
                            }
                        }
                        Err(()) => {}
                    }
                    if t.take() {
                        co.tick();
                    }
                }
            });
        }

        // coord's expiry timer.
        {
            timers::arm(&s.coord, &s.rearm);
            let s = s.clone();
            let stop = self.stop.clone();
            self.spawn("expiry", move || {
                let now = s.now.clone();
                timers::run_expiry(&s.coord, &s.rearm, &*now, &stop);
            });
        }

        // The inbox's Follow.
        {
            let sub = Arc::new(s.store.subscribe());
            let reg = self.on_stop(&sub, |s| s.cancel());
            let ib = s.inbox.clone();
            self.spawn("inbox", move || {
                let _reg = reg;
                ib.follow(&sub);
            });
        }

        // Device colours.
        {
            let sub = subs.colors;
            let reg = self.on_stop(&sub, |s| s.cancel());
            let s = s.clone();
            self.spawn("colors", move || {
                let _reg = reg;
                while let Some(ev) = sub.recv() {
                    if ev.is_resync() {
                        load_colors(
                            &s.store,
                            &s.colors,
                            &s.person_color,
                            &s.faces,
                            &s.person_avatar,
                            &s.assistant_name,
                        );
                        load_device_names(&s.store, &s.device_names);
                        s.follow_person_name();
                        s.notifications.reset();
                        load_notifications(&s.store, &s.notifications);
                        s.settle_notifications();
                        s.hub.publish(CoreEvent::Devices);
                        s.hub
                            .publish(CoreEvent::Notifications(crate::events::NotifChange::all()));
                        let before = s.store.vector().unwrap_or_default();
                        s.assistant_chat.reload(&s.store);
                        s.hub.publish(CoreEvent::AssistantChat);
                        // Card #538: this thread doesn't wait for the
                        // reconcile, which reads the markers too.
                        if let Err(e) = s.dm_spaces.load(&s.store) {
                            logf(format_args!("dm spaces: load: {e}"));
                        }
                        let _ = s.talk.load();
                        s.hub
                            .publish(CoreEvent::Talk(crate::events::TalkChange::all()));
                        s.follow_call_settings();
                        s.follow_names_themes();
                        s.follow_names_on();
                        s.follow_weighs_on();
                        s.follow_assistant_home();
                        // Questions past the buffer are lost to the
                        // chat's queue: its scan finds them (card 3).
                        s.chat_resynced(&before);
                    } else if ev.kind == kinds::ASSISTANT_HOME
                        || ev.kind == kinds::PERSON_DEVICE_REVOKED
                    {
                        // The recorded assistant's computer, or a removal
                        // that may end its rows (one-assistant card 1).
                        s.follow_assistant_home();
                    } else if ev.kind == kinds::ASSISTANT_NAMES_SET {
                        // Another device's "Name new threads" (card #782).
                        s.follow_names_on();
                    } else if ev.kind == kinds::ASSISTANT_HUDDLE_NOTES_SET
                        || ev.kind == kinds::ASSISTANT_HUDDLE_CHAIR_SET
                    {
                        // Another device's huddle switches (card #1053).
                        s.follow_huddle_switches();
                    } else if ev.kind == kinds::ASSISTANT_WEIGHS_SET {
                        // Another device's "Weigh new cards" (card #965).
                        s.follow_weighs_on();
                    } else if ev.kind == kinds::TRANSCRIBE_SET {
                        // Another device's switch (card #656).
                        s.follow_call_settings();
                    } else if ev.kind == kinds::NAMES_THEMES {
                        // Another device's switches (card #702).
                        s.follow_names_themes();
                    } else if coord::talk::KINDS.contains(&ev.kind.as_str()) {
                        // A write here was folded when it was written; the
                        // window still hears it (card #389).
                        s.talk.apply(&ev);
                        s.hub
                            .publish(CoreEvent::Talk(crate::events::TalkChange::of_row(&ev)));
                        if ev.kind == kinds::TALK_POST {
                            s.talk_note(&ev);
                        }
                        // A live line feeds the assistant (card #1050).
                        s.huddle_line_row(&ev);
                    } else if assistant_chat::KINDS.contains(&ev.kind.as_str()) {
                        // The assistant's conversation (card #1102); a row
                        // written here was folded when it was written.
                        if s.assistant_chat.apply(&ev) {
                            s.hub.publish(CoreEvent::AssistantChat);
                        }
                    } else if ev.kind == coord::notifications::POSTED
                        || ev.kind == coord::notifications::READ
                    {
                        // This device's own rows were folded when written;
                        // folding again changes nothing.
                        if s.notifications.apply(&ev) {
                            s.hub.publish(notifications::notif_change(&ev));
                            // Another device kept one this one can see is
                            // already resolved.
                            if ev.kind == coord::notifications::POSTED {
                                s.settle_notifications();
                            }
                        }
                    } else if ev.kind == coord::devcolor::KIND {
                        // Cards and the header show device colours
                        // (list-signals.md §3.2).
                        if s.colors.apply(&ev) {
                            s.hub.publish(CoreEvent::Devices);
                        }
                    } else if ev.kind == coord::avatar::KIND {
                        if s.faces.apply(&ev) {
                            s.hub.publish(CoreEvent::Devices);
                            // A face picked on another of the person's
                            // devices reaches this device's board cards.
                            s.sync_card_status();
                        }
                    } else if ev.kind == coord::devcolor::PERSON_KIND
                        || ev.kind == coord::person_avatar::KIND
                        || ev.kind == coord::assistant_name::KIND
                    {
                        // The pick reaches every shared board on the
                        // person's member.profile (cards #385, #393).
                        if s.person_color.apply(&ev)
                            || s.person_avatar.apply(&ev)
                            || s.assistant_name.apply(&ev)
                        {
                            s.follow_person_name();
                            s.hub.publish(CoreEvent::Devices);
                        }
                    } else if ev.kind == coord::devname::KIND {
                        // A rename from this device was folded when it was
                        // written; the window still hears it here.
                        s.device_names.apply(&ev);
                        s.hub.publish(CoreEvent::Devices);
                    } else if ev.kind == coord::sides::KIND {
                        // The sides a device runs (card #494): folded
                        // into the coord now, so Settings → Devices reads
                        // them when it hears this; its follow loop folding
                        // the row again changes nothing.
                        s.coord.apply_sides(&ev);
                        s.hub.publish(CoreEvent::Devices);
                    }
                    if !ev.is_resync() {
                        // After the folds: the chat worker's live way in,
                        // its Stop, and how far the folds have got.
                        s.chat_heard(&ev);
                    }
                }
            });
        }

        // Decisions from every device.
        {
            let sub = subs.decisions;
            let reg = self.on_stop(&sub, |s| s.cancel());
            let s = s.clone();
            self.spawn("decisions", move || {
                let _reg = reg;
                while let Some(ev) = sub.recv() {
                    if ev.is_resync() {
                        if let Err(e) = s.capture.refold_decisions() {
                            logf(format_args!("decisions: {e}"));
                        }
                    } else if let Err(e) = s.capture.follow_decision(&ev) {
                        logf(format_args!("decisions: {e}"));
                    }
                }
            });
        }

        // This device's agents' cards on shared boards follow their
        // sessions' status (v0.5 §2.4); only the hosting device writes it.
        {
            let sub = Arc::new(s.presence.subscribe());
            let reg = self.on_stop(&sub, |s| s.cancel());
            let s = s.clone();
            self.spawn("board-card-status", move || {
                let _reg = reg;
                while sub.recv().is_some() {
                    s.sync_card_status();
                }
            });
        }

        // Name clashes settle when this device's cards or another's change.
        {
            let rq = s.roster.subscribe();
            let reg = self.on_stop(&rq, |q| q.close());
            let kick = s.names_kick.clone();
            let asks = s.asks_kick.clone();
            self.spawn("names-roster", move || {
                let _reg = reg;
                while rq.recv().is_some() {
                    kick.kick();
                    asks.kick();
                }
            });
            let s = s.clone();
            self.spawn("names", move || {
                // One settle at start: a kick before this thread ran
                // would otherwise be lost (Go's channel keeps it).
                let mut seen = s.names_kick.generation().wrapping_sub(1);
                while s.names_kick.wait(&mut seen) {
                    if s.names.settle() {
                        s.presence.rescan();
                    }
                    s.note_agent_names();
                }
            });
        }

        // Asks go to one idle agent and move on by events: an agent going
        // idle, busy or away (the roster), or the ask's card changing (a
        // give-back, a claim, a comment). No timer decides a hand-on.
        {
            let s = s.clone();
            self.spawn("asks", move || {
                let mut seen = s.asks_kick.generation().wrapping_sub(1);
                while s.asks_kick.wait(&mut seen) {
                    s.hand_out_asks();
                }
            });
        }

        // Linked PRs on GitHub (card #208).
        if s.pr_checker.is_some() {
            let s = s.clone();
            let stop = self.stop.clone();
            self.spawn("prs", move || {
                loop {
                    if stop.sleep(crate::prwatch::TICK) {
                        return;
                    }
                    s.check_prs(std::time::Instant::now());
                }
            });
        }

        // A `gh pr merge` heard on any device (cards #421, #970): this
        // device's open PRs and its desks are asked about at once.
        {
            let s = s.clone();
            let stop = self.stop.clone();
            self.spawn("merges", move || {
                loop {
                    s.merge_heard();
                    if stop.sleep(crate::prwatch::MERGE_TICK) {
                        return;
                    }
                }
            });
        }

        // Merge desks' PR lists (card #415), and which repos are the
        // person's (card #625): a device that polls none still hides
        // stray desks.
        {
            let s = s.clone();
            let stop = self.stop.clone();
            self.spawn("desks", move || {
                loop {
                    s.desks_heard_merge();
                    s.check_desks();
                    if stop.sleep(crate::deskwatch::DESK_TICK) {
                        return;
                    }
                }
            });
        }

        // The sweeps.
        {
            let s = s.clone();
            let stop = self.stop.clone();
            self.spawn("sweep", move || {
                let mut it = Ticker::new(inbox::SWEEP_INTERVAL);
                let mut ct = Ticker::new(COORD_SWEEP_INTERVAL);
                loop {
                    if stop.sleep(it.until().min(ct.until())) {
                        return;
                    }
                    if it.take()
                        && let Err(e) = s.inbox.sweep()
                    {
                        logf(format_args!("inbox sweep: {e}"));
                    }
                    if ct.take() {
                        s.check_answering();
                        if let Err(e) = s.sweep_coord() {
                            logf(format_args!("coord sweep: {e}"));
                        }
                    }
                }
            });
        }

        self.start_pumps();
    }

    /// Republishes each service's changes into the hub.
    fn start_pumps(&self) {
        let s = &self.s;
        {
            let sub = Arc::new(s.presence.subscribe());
            let reg = self.on_stop(&sub, |s| s.cancel());
            let s = s.clone();
            self.spawn("feed-presence", move || {
                let _reg = reg;
                while let Some(ch) = sub.recv() {
                    s.hub.publish(CoreEvent::Presence(Box::new(ch)));
                }
            });
        }
        {
            let q = s.roster.subscribe();
            let reg = self.on_stop(&q, |q| q.close());
            let s = s.clone();
            self.spawn("feed-roster", move || {
                let _reg = reg;
                while q.recv().is_some() {
                    s.hub.publish(CoreEvent::Roster);
                }
            });
        }
        {
            let sub = s.inbox.subscribe();
            let q = sub.queue().clone();
            let reg = self.on_stop(&q, |q| q.close());
            let s = s.clone();
            self.spawn("feed-inbox", move || {
                let _reg = reg;
                while let Some(ch) = sub.recv() {
                    // An agent's message shows as a DM from its delivery's
                    // first row (list-signals.md §3.2).
                    // Only a first row or a move can show one: no lookup
                    // for the rest.
                    let d = (!ch.delivery_id.is_empty()
                        && (ch.created || ch.kind == inbox::ChangeKind::MovedAway))
                        .then(|| s.inbox.get(&ch.delivery_id))
                        .flatten();
                    if let Some(t) = dm_change(&ch, d.as_ref()) {
                        s.hub.publish(CoreEvent::Talk(t));
                    }
                    s.hub.publish(CoreEvent::Inbox(ch));
                    // A message replied to, read or released (card #482).
                    s.settle_notifications();
                }
            });
        }
        {
            let q = s.coord.subscribe();
            let reg = self.on_stop(&q, |q| q.close());
            let s2 = s.clone();
            // This subscription keeps coord's expiry timer armed for the
            // Core's life, as the window's Board pump does in Go's GUI.
            self.spawn("feed-board", move || {
                let _reg = reg;
                // What was resolved while Irori was off reads now (card
                // #482).
                s2.settle_notifications();
                while let Some(ch) = q.recv() {
                    if s2
                        .coord
                        .get_task(&ch.task_id)
                        .is_some_and(|t| t.open_ask().is_some() || t.live_offer().is_some())
                    {
                        s2.asks_kick.kick();
                    }
                    s2.hub.publish(CoreEvent::Board(ch));
                    // A blocker answered or a done check marked, by anyone.
                    s2.settle_notifications();
                }
            });
        }
        {
            // A shared board opened, went, was renamed or changed members,
            // or a DM space was hidden (`Boards::moved`): an opened board's
            // cards come by replay, which notes nothing, so every card list
            // reads again (list-signals.md §3.1). Through the hub, so a
            // full feed's lost flag covers it.
            let q = s.boards.subscribe();
            let reg = self.on_stop(&q, |q| q.close());
            let s = s.clone();
            self.spawn("feed-board-set", move || {
                let _reg = reg;
                while q.recv().is_some() {
                    s.hub.publish(CoreEvent::Board(coord::Change {
                        kind: coord::RESYNC.into(),
                        ..coord::Change::default()
                    }));
                }
            });
        }
        {
            // Irori's assistant names the person's new threads (card
            // #782): one loop hears the Board, another runs the turns one
            // at a time, so a slow turn never holds the Board's feed.
            let q = s.coord.subscribe();
            let reg = self.on_stop(&q, |q| q.close());
            s.names_seen_all();
            let (tx, rx) = std::sync::mpsc::channel();
            s.names_queue(Some(tx));
            let s2 = s.clone();
            self.spawn("assistant-names", move || {
                let _reg = reg;
                while let Some(ch) = q.recv() {
                    s2.names_hear(&ch);
                }
                s2.names_queue(None);
            });
            let s3 = s.clone();
            self.spawn("assistant-names-run", move || s3.names_loop(rx));
        }
        {
            // The assistant answers other people's questions (assistants
            // talk, card 4): one worker takes the checked questions.
            let (tx, rx) = std::sync::mpsc::channel();
            s.answers_queue(Some(tx));
            let s2 = s.clone();
            let reg = self.on_stop(&s2, |s| s.answers_queue(None));
            let s3 = s.clone();
            self.spawn("assistant-answers", move || {
                let _reg = reg;
                s3.answers_loop(rx);
            });
        }
        {
            // The assistant's computer answers the chat's questions from
            // rows (one-assistant card 3): one worker, oldest first.
            let s2 = s.clone();
            let reg = self.on_stop(&s2, |s| s.chat_close());
            let s3 = s.clone();
            self.spawn("assistant-chat", move || {
                let _reg = reg;
                s3.chat_loop();
            });
        }
        {
            // Irori's assistant weighs new cards (card #965), as above.
            let q = s.coord.subscribe();
            let reg = self.on_stop(&q, |q| q.close());
            s.weighs_seen_all();
            let (tx, rx) = std::sync::mpsc::channel();
            s.weighs_queue(Some(tx));
            let s2 = s.clone();
            self.spawn("assistant-weighs", move || {
                let _reg = reg;
                while let Some(ch) = q.recv() {
                    s2.weighs_hear(&ch);
                }
                s2.weighs_queue(None);
            });
            let s3 = s.clone();
            self.spawn("assistant-weighs-run", move || s3.weighs_loop(rx));
        }
        {
            let sub = Arc::new(s.capture.subscribe());
            let reg = self.on_stop(&sub, |s| s.cancel());
            let s = s.clone();
            self.spawn("feed-capture", move || {
                let _reg = reg;
                while let Some(ch) = sub.recv() {
                    if ch.kind == capture::ChangeKind::Pr {
                        s.link_opened_prs(&ch.key);
                    }
                    s.hub.publish(CoreEvent::Capture(ch));
                }
            });
        }
        {
            let q = s.asks.subscribe();
            let reg = self.on_stop(&q, |q| q.close());
            let s = s.clone();
            self.spawn("feed-asks", move || {
                let _reg = reg;
                while let Some(id) = q.recv() {
                    s.hub.publish(CoreEvent::Asks(id));
                    s.settle_notifications();
                }
            });
        }
    }

    /// Stops the services, removes the lockfile if it is still ours and
    /// closes the log. Waits for the loops at most [`CLOSE_WAIT`].
    pub fn close(&self) -> store::Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(w) = self.watch.lock().unwrap_or_else(|e| e.into_inner()).take() {
            w.stop();
        }
        self.s.pairing.pairer.cancel();
        self.stop.cancel();
        self.stop_async.cancel();
        self.s.rearm.close();
        self.s.names_kick.close();
        self.s.asks_kick.close();
        let threads = std::mem::take(&mut *self.threads.lock().unwrap_or_else(|e| e.into_inner()));
        let deadline = Instant::now() + CLOSE_WAIT;
        for h in threads {
            while !h.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if h.is_finished() {
                let _ = h.join();
            }
        }
        // Killed shells remove their bootstrap folders from the temp dir as
        // they exit: wait for them, a stuck one at most CLOSE_WAIT (#960).
        self.terms.close_all();
        self.terms.settle(CLOSE_WAIT);
        self.s.boards.close_all();
        self.s.hub.close();
        self.s.pairing.shutdown();
        let _ = lockfile::remove(&self.s.data_dir, lockfile::own_pid());
        self.s.store.close()
    }

    // ---- the lockfile ----------------------------------------------------

    /// Writes `irori.json` for an API listening on `127.0.0.1:<port>`.
    pub fn write_lockfile(&self, port: u16) -> std::io::Result<()> {
        lockfile::write(
            &self.s.data_dir,
            &Lockfile {
                pid: lockfile::own_pid(),
                port: i64::from(port),
                token: self.s.token.clone(),
                version: self.s.version.clone(),
                device_id: self.s.device.id.clone(),
                headless: self.s.headless,
                started_at: GoTime::from_system(SystemTime::now()).utc(),
            },
        )
    }

    /// The desktop cards (headless unless [`Options::notifier`] gave one).
    pub fn notifier(&self) -> &notify::Notifier {
        &self.s.notifier.0
    }

    /// The API's bearer token. A secret: never log it.
    pub fn token(&self) -> &str {
        &self.s.token
    }

    /// The window assistant's secret for this run (card #783): sent as
    /// `X-Irori-Assistant`, it reads as the person through
    /// `tools::ASSISTANT_TOOLS` (writes per grant, card #987). A secret:
    /// never log it.
    pub fn assistant_secret(&self) -> &str {
        &self.s.assistant
    }

    /// Keeps the window's plain-text snapshot of what it shows (card
    /// #1009) for the assistant's `screen` tool, cut to
    /// [`ASSISTANT_SCREEN_MAX`] characters. An empty one clears it.
    pub fn set_assistant_screen(&self, text: &str) {
        let t: String = text.trim().chars().take(ASSISTANT_SCREEN_MAX).collect();
        *self
            .s
            .assistant_screen
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = t;
    }

    /// What the person's message lets the window assistant change, for
    /// the run going now (card #987).
    pub fn assistant_grants(&self) -> &tools::Grants {
        &self.s.assistant_grants
    }

    /// Where the window assistant runs (card #783): its `claude -p` cwd,
    /// which presence leaves off the roster.
    pub fn assistant_dir(&self) -> PathBuf {
        assistant_dir(&self.s.data_dir)
    }

    /// Gives capture's scrubber one more exact value to redact, kept for the
    /// Core's life (the lockfile token, member secrets).
    pub fn add_secret(&self, v: &str) {
        self.s.add_secret(v);
    }

    // ---- the services ----------------------------------------------------

    pub fn data_dir(&self) -> &Path {
        &self.s.data_dir
    }

    /// Claude usage left (card #599): the plan's rate-limit windows as this
    /// device's sessions last reported them, without any past its reset.
    pub fn usage_limits(&self) -> crate::usage::UsageLimits {
        self.s.usage.get((self.s.now)().unix_milli_value())
    }

    /// Takes one session's rate-limit windows (`POST /v1/usage/limits`):
    /// checked, merged into the kept reading and saved. Every reading taken
    /// is `usage:changed`, so the window's "as of" moves too; one with no
    /// window left after the checks changes nothing and is no event. `Err`
    /// is why the reading was refused. `account` is the Claude account the
    /// session is signed in as, when the mod could tell (card #1040): a
    /// different one replaces the kept windows.
    pub fn record_usage_limits(
        &self,
        input: &[crate::usage::RateLimit],
        account: Option<&str>,
    ) -> Result<(), String> {
        let now = (self.s.now)().unix_milli_value();
        if self.s.usage.record(input, account, now)? {
            self.s.hub.publish(CoreEvent::Usage);
        }
        Ok(())
    }
    pub fn version(&self) -> &str {
        &self.s.version
    }
    pub fn headless(&self) -> bool {
        self.s.headless
    }
    /// This device (`device.json`).
    /// This device's terminals (card #369).
    pub fn terms(&self) -> &Arc<term::Terms> {
        &self.terms
    }

    pub fn device(&self) -> &DeviceJson {
        &self.s.device
    }
    pub fn device_id(&self) -> &str {
        &self.s.device.id
    }
    pub fn identity(&self) -> &Arc<Identity> {
        &self.s.ident
    }
    pub fn chain(&self) -> &Arc<Chain> {
        &self.s.chain
    }

    // ---- the home server (Go core/home.go) -----------------------------

    /// Whether this device can run a home server (Go `CanServe`).
    pub fn can_serve(&self) -> bool {
        self.s.home.is_some()
    }
    /// This device's home server status (Go `HomeServer`); zero without one.
    pub fn home_server(&self) -> home::Status {
        self.s.home.as_ref().map(|h| h.status()).unwrap_or_default()
    }
    /// Switches the home server on or off, with "Reachable from outside
    /// home" and an optional public address (Go `SetHomeServer`). GUI only.
    pub fn set_home_server(&self, on: bool, outside: bool, public: &str) -> Result<(), String> {
        match &self.s.home {
            Some(h) => h.set(on, outside, public),
            None => Err(NO_HOME_SERVER.to_string()),
        }
    }
    /// "Keep this PC awake while it's the server" (Go `SetHomeServerAwake`).
    /// GUI only.
    pub fn set_home_server_awake(&self, on: bool) -> Result<(), String> {
        match &self.s.home {
            Some(h) => h.set_awake(on),
            None => Err(NO_HOME_SERVER.to_string()),
        }
    }
    /// "Meet through public relays" (card #328): n0's iroh relays, so
    /// devices and other people reach this one from any network.
    pub fn public_relays(&self) -> bool {
        self.net().public_relays()
    }
    /// Turns public relays on or off: saved first, then the live endpoint.
    pub fn set_public_relays(&self, on: bool) -> Result<(), String> {
        crate::publicrelays::save(self.data_dir(), on).map_err(|e| e.to_string())?;
        let net = self.net().clone();
        self.pairing_runtime()
            .block_on(async move { net.set_public_relays(on).await });
        Ok(())
    }
    /// The home server itself, for the net and mesh wiring.
    pub fn home(&self) -> Option<&home::Server> {
        self.s.home.as_ref()
    }
    pub fn person(&self) -> irori_identity::Person {
        self.s.ident.current()
    }

    /// The person renames themself (card #183): person.json here, then a
    /// person.profile row that carries the name to their other devices;
    /// the boards' member.profile rows follow as it folds.
    pub fn set_person_name(&self, name: &str) -> Result<irori_identity::Person, String> {
        let s = &self.s;
        if !s.ident.set_name(name).map_err(|e| e.to_string())? {
            return Ok(s.ident.current());
        }
        let prof = s.ident.profile_row().map_err(|e| e.to_string())?;
        s.store
            .append("irori", store::kinds::PERSON_PROFILE, &prof)
            .map_err(|e| e.to_string())?;
        Ok(s.ident.current())
    }
    pub fn certified_devices(&self) -> Vec<irori_identity::DeviceState> {
        self.s.chain.devices()
    }
    pub fn store(&self) -> &Arc<Store> {
        &self.s.store
    }
    pub fn inbox(&self) -> &Arc<Inbox> {
        &self.s.inbox
    }
    pub fn coord(&self) -> &Arc<Coord> {
        &self.s.coord
    }
    pub fn capture(&self) -> &capture::Capture {
        &self.s.capture
    }
    pub fn roster(&self) -> &Arc<Roster> {
        &self.s.roster
    }
    pub fn asks(&self) -> &Arc<coord::approvals::Book> {
        &self.s.asks
    }
    /// The agent tool registry the API serves at `/v1/tools` (Go
    /// `Tools`).
    pub fn tools(&self) -> &tools::Set {
        &self.tools
    }
    pub fn presence(&self) -> &Arc<presence::Watcher> {
        &self.s.presence
    }

    /// Activity from this device's Claude session `key` (an attach stream's
    /// hello or keepalive, a tool call through its attach secret) while
    /// presence doesn't list it: presence reads the session files again now
    /// (card #433). A session whose process lives comes back (its Added
    /// change writes `agent.seen`, so every device lists it again); one
    /// whose process ended stays gone. Whether it is listed afterwards.
    pub fn revive_agent(&self, key: &str) -> bool {
        let listed = |p: &presence::Watcher| p.snapshot().iter().any(|c| c.key == key);
        if listed(&self.s.presence) {
            return true;
        }
        if irori_identity::owner::device_of(key) != self.s.device.id {
            return false;
        }
        self.s.presence.rescan();
        let ok = listed(&self.s.presence);
        if ok {
            logf(format_args!("presence: {key} is active again; listed"));
        }
        ok
    }
    pub fn names(&self) -> &Arc<Namer> {
        &self.s.names
    }
    /// Attachments (`files/`).
    pub fn files(&self) -> &blob::Files {
        &self.s.files
    }

    /// The person at this device's window.
    pub fn user_sender(&self) -> Sender {
        self.s.user_sender()
    }

    /// Whether `key` is the person's.
    pub fn is_owner(&self, key: &str) -> bool {
        self.s.chain.is_owner(key)
    }

    /// This device's roster (Go `LocalAgents`, what `Agents` serves to
    /// attach and the API's caller checks): presence, then a synthetic card
    /// for each Grok member whose stream is open or closed less than
    /// [`inbox::REATTACH_GRACE`] ago.
    pub fn local_agents(&self) -> Vec<presence::AgentCard> {
        self.s.local_agents()
    }

    // ---- attachments (core/attachments.go, crate::attachments) -----------

    /// Go `AttachFiles` (v0.4 §6.6): checks and stores files an agent
    /// attaches to a message or a comment. Each path resolves to its
    /// canonical form, following symlinks, and anything the never-leave
    /// rules cover is refused with the reason ([`blob::ErrorKind::NeverLeaves`]);
    /// the file is copied into the attachment store and offered to the
    /// person's other devices. A relative path is taken against `by`'s
    /// working directory. Nothing inside a file is scrubbed: it is the
    /// person's.
    pub fn attach_files(
        &self,
        by: &str,
        paths: &[String],
    ) -> Result<Vec<blob::FileRef>, blob::Error> {
        self.s.attach_files(by, paths)
    }

    /// Go `GetAttachment`: fetches a file `reader` may read (on a message it
    /// sent or received, on any Board comment, or on a Talk post in a
    /// channel or DM it may read, card #643) and writes it to `to`, or
    /// by default to `<data>/received/<reader>/<name>`. Returns the local
    /// path and the device it came from ("" when it was already here). A
    /// `to` inside the data dir is refused, and an existing file is never
    /// overwritten. [`crate::attachments::NO_ATTACHMENT`] when no message,
    /// card or post the reader may read carries it. Blocks on the mesh's runtime
    /// when another device holds it: call it off that runtime's workers.
    pub fn get_attachment(
        &self,
        reader: &str,
        sha: &str,
        to: &str,
    ) -> Result<(String, String), String> {
        self.s.get_attachment(reader, sha, to)
    }

    // ---- Grok members (core/members.go) ---------------------------------

    /// Binds a Grok member (`POST /v1/members`): the same non-empty session
    /// gets the same key; every join mints a new secret, the old one stops
    /// working. Logs `member.joined {key, name}` (device-local); the secret
    /// is never logged, and goes to capture's scrubber.
    pub fn join_member(&self, name: &str, session: &str) -> Result<Member, String> {
        let m = self.s.members.join(name, session)?;
        self.s.add_secret(&m.secret);
        #[derive(Serialize)]
        struct Joined<'a> {
            key: &'a str,
            name: &'a str,
        }
        self.s
            .store
            .append_local(
                "irori",
                kinds::MEMBER_JOINED,
                &Joined {
                    key: &m.key,
                    name: &m.name,
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(m)
    }

    /// The member a secret (`X-Irori-Member`, `?member=`) belongs to.
    pub fn member_by_secret(&self, secret: &str) -> Option<Member> {
        self.s.members.by_secret(secret)
    }

    /// A member stream for `key` opened (true) or closed (false).
    pub fn member_stream(&self, key: &str, open: bool) {
        self.s.members.stream(key, open);
    }

    /// How many member streams `key` has open (Go reads
    /// `members.byKey[key].open` in its tests).
    pub fn member_open_streams(&self, key: &str) -> u32 {
        self.s.members.open_count(key)
    }

    /// The roster across the person's devices (Go `MeshAgents`): inbox
    /// routing, the tools and the window.
    pub fn mesh_agents(&self) -> Vec<AgentCard> {
        self.s.roster.agents()
    }

    /// The word a new agent started from `seed` would get now (card #472):
    /// the name pick a spawn makes, for tests.
    #[doc(hidden)]
    pub fn pick_agent_name(&self, seed: &str) -> String {
        self.s.names.pick_new(seed, &|_| false)
    }

    /// An agent key's name: its card's in the mesh roster, else the last
    /// name it went by (gone agents, Grok members after a restart; card
    /// #98), or `""`.
    pub fn agent_name(&self, key: &str) -> String {
        self.s.agent_name(key)
    }

    /// Every agent key → the last name it went by, gone ones included
    /// (card #98).
    pub fn known_agent_names(&self) -> std::collections::BTreeMap<String, String> {
        self.s.note_agent_names();
        let mut names = self.s.agent_names.all();
        self.s.card_key_names(&mut names);
        names
    }

    /// Gives the Core the desktop app's Revive, which a revive ask answered
    /// from any device runs here. A headless Core has none.
    pub fn set_reviver(&self, f: Option<ReviveFn>) {
        *self.s.reviver.lock().unwrap_or_else(|e| e.into_inner()) = f;
    }

    /// Gives the Core the desktop app's spawner (card #446): `claude --bg`
    /// for a card's agent, and stop and rm to end one. A headless Core has
    /// none and starts no agent.
    pub fn set_spawner(&self, s: Option<spawn::Spawner>) {
        self.s.set_spawner(s);
        self.s.asks_kick.kick();
    }

    /// Gives the Core the desktop app's assistant turn (card #782): one
    /// `claude -p` on Haiku that names the person's new threads. A
    /// headless Core, or the phone, has none and names nothing.
    pub fn set_namer(&self, n: Option<thread_names::Namer>) {
        self.s.set_namer(n);
    }

    /// Gives the Core the desktop app's chat turn (one-assistant card 3):
    /// the window runs each question this computer answers, blocking until
    /// it ends. Installing it answers the questions that came before it,
    /// and says on `spawn.host` that this computer can answer the person's
    /// other devices (`assistantChat`). A headless Core, or the phone, has
    /// none and answers nothing.
    pub fn set_chat_turn(&self, f: Option<assistant_chat::ChatTurnFn>) {
        self.s.set_chat_turn(f);
    }

    /// How many times the kind loop has resynced (tests; card 3's scan).
    #[doc(hidden)]
    pub fn chat_resyncs(&self) -> u64 {
        self.s.chat_resyncs()
    }

    /// Questions each catch-up scan queued: on install, on becoming the
    /// assistant's computer, on a resync, from the sweep (tests).
    #[doc(hidden)]
    pub fn chat_scan_counts(&self) -> [u64; 4] {
        self.s.chat_scan_counts()
    }

    /// Whether the kind loop has folded every row the store holds (tests).
    #[doc(hidden)]
    pub fn chat_heard_all(&self) -> bool {
        self.s.chat_heard_all()
    }

    /// Whether question `turn` is queued or running here (tests).
    #[doc(hidden)]
    pub fn chat_queued(&self, turn: &str) -> bool {
        self.s.chat_queued(turn)
    }

    /// Installs the turn that weighs a new card (card #965).
    pub fn set_weigher(&self, n: Option<thread_names::Namer>) {
        self.s.set_weigher(n);
    }

    /// Installs the turn that answers other people's questions to the
    /// person's Office Assistant (assistants talk, card 4); a desktop with
    /// Claude Code has one. Installing it answers the questions that came
    /// before it.
    pub fn set_answerer(&self, n: Option<assistant_answers::Answerer>) {
        self.s.set_answerer(n);
    }

    /// What the assistant held back and didn't post, oldest first.
    pub fn assistant_held(&self) -> Vec<assistant_answers::Held> {
        self.s.assistant_held()
    }

    /// Writes a raw `talk.post` row to space `space` as `by`, folded and
    /// shown at once. The assistant's answers go in this way: `PostRow`
    /// carries the hop and `replyTo` that `talk_post` has no place for.
    pub fn space_talk_post(
        &self,
        space: &str,
        by: &str,
        payload: serde_json::Value,
    ) -> Result<(), String> {
        self.s.space_talk_post(space, by, payload)
    }

    /// Gives the Core the desktop app's held huddle sessions (card #1046):
    /// one `claude -p` per huddle, kept up between looks. A headless Core,
    /// or the phone, has none and runs no looks.
    pub fn set_huddle_runner(&self, r: Option<Arc<dyn huddle_session::HuddleRunner>>) {
        self.s.set_huddle_runner(r);
    }

    /// The packet huddle `seed` starts a session with (tests; card #1046).
    #[doc(hidden)]
    pub fn huddle_packet_text(&self, seed: &huddle_session::HuddleSeed) -> Result<String, String> {
        self.s.huddle_packet_text(seed)
    }

    /// Opens `seed`'s huddle's held session with its priming turn (tests;
    /// card 7a calls it when the person joins).
    #[doc(hidden)]
    pub fn huddle_open(&self, seed: &huddle_session::HuddleSeed) -> Result<(), String> {
        self.s.huddle_open(seed)
    }

    /// One look of `seed`'s huddle's assistant (tests; card 7b's feed
    /// calls it).
    #[doc(hidden)]
    pub fn huddle_look(
        &self,
        seed: &huddle_session::HuddleSeed,
        message: &str,
    ) -> Result<huddle_session::Look, String> {
        self.s.huddle_look(seed, message)
    }

    /// Ends `call`'s held session (tests; card 7a).
    #[doc(hidden)]
    pub fn huddle_close(&self, call: &str) {
        self.s.huddle_close(call)
    }

    /// Whether `call` has an open held session (tests).
    #[doc(hidden)]
    pub fn huddle_is_open(&self, call: &str) -> bool {
        self.s.huddle_is_open(call)
    }

    /// "Weigh new cards" (card #965): on unless the person turned it off.
    pub fn weighs_on(&self) -> bool {
        self.s.weighs_on()
    }

    /// The person turns "Weigh new cards" on or off, from the app only.
    pub fn set_weighs_on(&self, on: bool) -> Result<(), String> {
        self.s.set_weighs_on(on)
    }

    /// The Office Assistant's computer, folded from the person's
    /// `assistant.home` rows; `None` before one claims itself.
    pub fn assistant_home(&self) -> Option<String> {
        self.s.assistant_home()
    }

    /// Whether this device is the recorded assistant's computer. It reads
    /// the fold, so it never moves for a device that is offline or a Claude
    /// Code that is missing.
    pub fn is_assistant_home(&self) -> bool {
        self.s.is_assistant_home()
    }

    /// Where the assistant runs and whether it can be reached, for the chat
    /// and Settings.
    pub fn assistant_home_status(&self) -> assistant_home::HomeStatus {
        self.s.assistant_home_status()
    }

    /// The person picks which computer runs the assistant. From any of
    /// their devices, the phone included.
    pub fn set_assistant_home(&self, device: &str) -> Result<(), String> {
        self.s.set_assistant_home(device)
    }

    /// Runs `f` with the new answer whenever the assistant's computer
    /// changes. The chat worker doesn't use it: `follow_assistant_home`
    /// runs its catch-up scan itself (one-assistant card 3).
    pub fn set_assistant_home_listener(&self, f: Option<assistant_home::HomeListener>) {
        self.s.set_assistant_home_listener(f);
    }

    /// A crashed huddle's end mark check at `now` (tests move the clock).
    #[doc(hidden)]
    pub fn huddle_crash_check_at(&self, now: std::time::SystemTime) {
        self.s.huddle_crash_check_at(now)
    }

    /// How long a huddle runs before its chair proposes a title (tests).
    #[doc(hidden)]
    pub fn set_huddle_title_after(&self, d: std::time::Duration) {
        self.s.set_huddle_title_after(d)
    }

    /// "Let my assistant add notes from what I say" (card #1053): on unless
    /// the person turned it off.
    pub fn huddle_notes_on(&self) -> bool {
        self.s.huddle_notes_on()
    }

    /// Turns the notes switch on or off, on all the person's devices.
    pub fn set_huddle_notes_on(&self, on: bool) -> Result<(), String> {
        self.s.set_huddle_notes_on(on)
    }

    /// "Let my assistant chair huddles I start" (card #1053): on unless the
    /// person turned it off.
    pub fn huddle_chair_on(&self) -> bool {
        self.s.huddle_chair_on()
    }

    /// Turns the chair switch on or off, on all the person's devices.
    pub fn set_huddle_chair_on(&self, on: bool) -> Result<(), String> {
        self.s.set_huddle_chair_on(on)
    }

    /// "Name new threads" (card #782): on unless the person turned it off.
    pub fn names_on(&self) -> bool {
        self.s.names_on()
    }

    /// The person turns "Name new threads" on or off, from the app only:
    /// no tool reaches this.
    pub fn set_names_on(&self, on: bool) -> Result<(), String> {
        self.s.set_names_on(on)
    }

    /// The `irori` whose `permission-hook` role each spawned agent's
    /// settings run on a permission prompt (card #682); `None` (the
    /// default) starts agents without the hook.
    pub fn set_permission_hook(&self, exe: Option<std::path::PathBuf>) {
        self.s.set_permission_hook(exe);
    }

    /// A spawned agent's permission hook reports its prompt and waits up
    /// to `wait` for the person's pick on the card (card #682).
    pub fn permission_wait(
        &self,
        ask: &spawn_waiting::PermissionAsk,
        wait: std::time::Duration,
    ) -> spawn_waiting::PermissionDecision {
        self.s.permission_wait(ask, wait)
    }

    /// A board's spawned-agent settings (`""`: the private board).
    pub fn spawn_setting(&self, board_id: &str) -> spawn::SpawnSetting {
        self.s.spawn_setting(board_id)
    }

    /// The desktop app turns the private board's spawning on by default
    /// (card #448, decision 1 (a)): only where agents can be started. A
    /// switch he set wins either way.
    pub fn set_spawn_default_on(&self, on: bool) {
        self.s
            .spawn
            .private_default
            .store(on, std::sync::atomic::Ordering::Relaxed);
        self.s.asks_kick.kick();
    }

    /// Whether he has set board `board_id`'s switch himself.
    pub fn spawn_switch_set(&self, board_id: &str) -> bool {
        self.s.spawn.switch_set(board_id)
    }

    /// The person turns a board's spawned agents on or off: the app only,
    /// never a tool.
    pub fn set_spawn_setting(&self, s: spawn::SpawnSetting) -> Result<(), String> {
        self.s.set_spawn_setting(s)
    }

    /// The person sets a board's permission choice, extra allow and deny
    /// rules and per-repo merge policies (card #449): the app only, never
    /// a tool. The switch and the board's other spawn fields stay.
    pub fn set_spawn_permissions(
        &self,
        board_id: &str,
        permission: &str,
        allow: Vec<String>,
        deny: Vec<String>,
        merge: std::collections::BTreeMap<String, String>,
    ) -> Result<spawn::SpawnSetting, String> {
        self.s
            .set_spawn_permissions(board_id, permission, allow, deny, merge)
    }

    /// This device's cap on spawned agents.
    pub fn spawn_cap(&self) -> i64 {
        self.s.spawn_cap(&self.s.device.id)
    }

    /// This device's spawned agents (live, and starting) and its cap, for
    /// the card line "waiting: the PC is at its cap" (card #448).
    /// What card `id` waits on for an agent (card #574); `asked`: the caller
    /// just asked for one.
    pub fn why_no_agent(&self, id: &str, asked: bool) -> Option<spawn_why::Why> {
        self.s.why_no_agent(id, asked)
    }

    pub fn spawn_slots(&self) -> (i64, i64) {
        (self.s.spawned_here(), self.s.spawn_cap(&self.s.device.id))
    }

    /// Whether the person turned on "Offer it cards" for agent `key`: a
    /// terminal agent is handed unheld cards, asks and threads only then
    /// (card #453).
    pub fn agent_offered(&self, key: &str) -> bool {
        self.s.agent_offered(key)
    }

    /// Whether Irori started agent `key` for a card (its spawned mark).
    pub fn agent_spawned(&self, key: &str) -> bool {
        self.s.coord.is_spawned(key)
    }

    /// The open cards and threads no agent can start for now, each with
    /// why (card #541), on the private board and the shared ones.
    pub fn no_agent(&self) -> std::collections::HashMap<String, String> {
        self.s.no_agent_all()
    }

    /// This device never runs agents (a phone, card #541).
    pub fn set_runs_no_agents(&self) {
        self.s.set_runs_no_agents();
        self.s.asks_kick.kick();
    }

    /// The devices that can't run agents (card #541).
    pub fn cant_run_devices(&self) -> std::collections::HashSet<String> {
        self.s.cant_run_devices()
    }

    /// The person's "Offer it cards" on one of their terminal agents: the
    /// app only, never a tool (card #453).
    pub fn set_agent_offer(&self, key: &str, on: bool) -> Result<(), String> {
        self.s.set_agent_offer(key, on)
    }

    /// The person sets this device's cap: the app only.
    pub fn set_spawn_cap(&self, cap: i64) -> Result<(), String> {
        self.s.set_spawn_cap(&self.s.device.id.clone(), cap)
    }

    /// Claude Code's folder (its `projects/` holds the transcripts).
    pub fn claude_dir(&self) -> PathBuf {
        PathBuf::from(self.s.presence.claude_dir())
    }

    /// Device `device`'s cap (card #450): his pick, else its OS's default.
    pub fn spawn_cap_of(&self, device: &str) -> i64 {
        self.s.cap_on(device, self.s.hosts().get(device))
    }

    /// Whether he picked device `device`'s cap himself.
    pub fn spawn_cap_set(&self, device: &str) -> bool {
        self.s
            .spawn
            .caps
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(device)
    }

    /// The person sets device `device`'s cap from Settings › Agents, on any
    /// of his devices (the row replicates; each device reads its own).
    pub fn set_spawn_cap_of(&self, device: &str, cap: i64) -> Result<(), String> {
        if device.trim().is_empty() {
            return Err("spawn: no device given".into());
        }
        if !(0..=spawn::MAX_CAP).contains(&cap) {
            return Err(format!("a cap is 0 to {}, not {cap}", spawn::MAX_CAP));
        }
        self.s.set_spawn_cap(device, cap)
    }

    /// Where the spawn queue stands here (card #450): each waiting card's
    /// place, a usage-limit hold.
    pub fn spawn_queue(&self) -> spawn_limits::SpawnQueue {
        self.s.spawn_queue()
    }

    /// The person ends this device's usage hold now (card #576): if the
    /// limit still stands, the next spawn fails on it and holds again. App
    /// only.
    pub fn release_usage_hold(&self) {
        self.s.release_usage_hold()
    }

    /// The agents Irori started today for the private board, every
    /// device's, oldest first: their keys and marks (card #450).
    pub fn spawned_today(&self) -> Vec<(String, coord::Spawned)> {
        self.s.spawned_today()
    }

    /// The person's End on an agent Irori started: its hand-off, then
    /// stop and rm.
    pub fn end_spawned(&self, key: &str) -> Result<(), String> {
        self.s.end_spawned(key)
    }

    /// The person asks for an agent on card `id` now, with `note` (their
    /// comment) in its brief: "Start an agent with this" (§4). App only.
    /// "My agents may start agents on my cards" (card #575).
    pub fn agent_starts(&self) -> spawn_agents::AgentStartsUse {
        self.s.agent_starts_use()
    }

    /// The person sets it, from the app only: no tool reaches this.
    pub fn set_agent_starts(&self, on: bool, max: i64) -> Result<(), String> {
        self.s.set_agent_starts(on, max)
    }

    /// "Agents review other people's PRs without asking" (merge-desk
    /// §12.8): on unless the person turned it off.
    pub fn reviews_unasked(&self) -> bool {
        self.s.reviews_unasked()
    }

    /// The person sets it, from the app only: no tool reaches this.
    pub fn set_reviews_unasked(&self, on: bool) -> Result<(), String> {
        self.s.set_reviews_unasked(on)
    }

    /// "Transcribe what I say in calls" (card #656): on unless the person
    /// turned it off.
    pub fn call_transcribe(&self) -> bool {
        self.s.call_transcribe()
    }

    /// The person sets it, from the app only: no tool reaches this.
    pub fn set_call_transcribe(&self, on: bool) -> Result<(), String> {
        self.s.set_call_transcribe(on)
    }

    /// The themes new agents' names don't come from (card #702): every
    /// theme is on until the person turns it off.
    pub fn names_themes_off(&self) -> Vec<String> {
        self.s.names_themes_off()
    }

    /// The person turns a `coord::names::THEMES` theme on or off, from the
    /// app only: no tool reaches this.
    pub fn set_names_theme(&self, id: &str, on: bool) -> Result<(), String> {
        self.s.set_names_theme(id, on)
    }

    pub fn start_agent(&self, id: &str, note: &str) -> Result<(), String> {
        self.s.start_agent(id, note)
    }

    /// Pause agents (card #665): whether this device starts no agents now.
    pub fn agents_pause(&self) -> spawn_pause::AgentsPause {
        self.s.agents_pause()
    }

    /// Pause agents on device `dev`.
    pub fn agents_pause_on(&self, dev: &str) -> spawn_pause::AgentsPause {
        self.s.agents_pause_on(dev)
    }

    /// Who set Pause agents, for people (`by` from [`Self::agents_pause`]).
    pub fn agents_pause_by(&self, by: &str) -> String {
        self.s.pause_by_name(by)
    }

    /// Whether device `dev` starts no agents now: its own pause, or Stop
    /// everywhere's (card #787).
    pub fn agents_paused_on(&self, dev: &str) -> bool {
        self.s.agents_paused_on(dev)
    }

    /// Every device's Pause agents, by device id (card #787), and Stop
    /// everywhere's own under [`spawn_pause::EVERYWHERE`].
    pub fn agents_pauses(&self) -> std::collections::HashMap<String, spawn_pause::AgentsPause> {
        self.s.agents_pauses()
    }

    /// Wind down on (card #787): Pause agents on each of `devs` (any of the
    /// person's devices), marked as Wind down's. Devices already paused are
    /// left as they are. Says which it paused. App only.
    pub fn wind_down_pause(&self, devs: &[String]) -> Result<Vec<String>, String> {
        self.s.wind_down_pause(devs)
    }

    /// Wind down off (card #787): lifts the pauses Wind down set, on every
    /// device, and no other. Says which. App only.
    pub fn wind_down_lift(&self) -> Result<Vec<String>, String> {
        self.s.wind_down_lift()
    }

    /// The devices that can run agents: every device that said so (its
    /// `spawn.host`), and this one (card #787).
    pub fn agent_devices(&self) -> Vec<String> {
        let mut v: Vec<String> = self.s.hosts().into_keys().collect();
        v.push(self.s.device.id.clone());
        v.sort();
        v.dedup();
        v
    }

    /// The person turns Pause agents on or off for this device, from the
    /// app (their agents use the `agents_pause` tool).
    pub fn set_agents_paused(&self, on: bool) -> Result<spawn_pause::AgentsPause, String> {
        let me = self.s.device.id.clone();
        let by = format!("user:{me}");
        self.s.set_agents_paused(&me, on, &by)
    }

    /// "Hand to an agent" (card #485): the person moves a card whose items
    /// wait on gone agents back on the Board, and Irori starts an agent for
    /// it (or hands it to the next free one). Says which. App only.
    ///
    /// With this computer's Agents switch off the card still goes back on
    /// the Board: start_agent's switch refusal (card #1066) isn't for this
    /// button, whose card has already moved by then (card #1096). A card
    /// another computer hands out is still asked for there, since that
    /// computer goes by its own switch (cards #492, #995).
    pub fn hand_on(&self, id: &str) -> Result<String, String> {
        let person = format!("user:{}", self.s.device.id);
        let (t, _) = self
            .s
            .coord
            .hand_on(&person, id)
            .map_err(|e| e.to_string())?;
        let here = self.s.coord.hands_out_here(id);
        let spawning = self.s.coord.spawning();
        if spawning {
            self.s.start_agent(id, "")?;
        } else if !here {
            self.s.request_start(id, "")?;
        }
        if let Some(why) = self.s.no_agent_all().remove(id) {
            return Ok(format!("{} is back on the Board. {why}", t.short_ref()));
        }
        // The device that made the card runs its hand-out (card #485: the
        // PC's cards go out from the PC).
        if !here {
            let dev = irori_identity::owner::device_of(&t.created_by).to_string();
            let mut name = self.device_label(&dev);
            if self.s.cant_run_devices().contains(&dev) {
                name = "one of your computers".into();
            } else if name.is_empty() {
                name = "the device that made it".into();
            }
            return Ok(format!(
                "{} is back on the Board: {name} starts its agent or hands it to a free one.",
                t.short_ref()
            ));
        }
        Ok(if spawning {
            format!(
                "{} is back on the Board: Irori starts an agent for it.",
                t.short_ref()
            )
        } else {
            format!(
                "{} is back on the Board. Agents is switched off on this computer (Settings › Agents), so it waits for a free agent.",
                t.short_ref()
            )
        })
    }

    /// The person's words for card `id`'s next spawned agent (a "Not
    /// done", a blocker answer): quoted in its brief (card #447).
    pub fn set_brief_answer(&self, id: &str, text: &str) {
        self.s.set_brief_answer(id, text)
    }

    /// The person cancels card `id`'s starting agent.
    pub fn cancel_spawn(&self, id: &str) -> Result<bool, String> {
        self.s.cancel_spawn(id)
    }

    /// The person's End on spawned agent `key` from any of their devices
    /// (card #455): ended here when it runs here, else a replicated request
    /// the device it runs on acts on. App only.
    pub fn request_end(&self, key: &str) -> Result<(), String> {
        self.s.request_end(key)
    }

    /// The person asks the device card `id`'s agent must start on to trust
    /// its repo (card #455): that device asks them in its own window. The
    /// device's id. App only; nothing here writes trust.
    pub fn request_trust(&self, id: &str) -> Result<String, String> {
        self.s.request_trust(id)
    }

    /// The device that asked this one to trust card `id`'s repo, if any.
    pub fn trust_asked_by(&self, id: &str) -> Option<String> {
        self.s.trust_asked_by(id)
    }

    /// What each of the person's devices can start agents in, by device
    /// (card #455).
    pub fn spawn_hosts(&self) -> std::collections::HashMap<String, spawn_remote::SpawnHost> {
        self.s.hosts()
    }

    /// Log rows appended or replicated from now on, with a resync signal
    /// (`Event::is_resync`) when the reader fell behind (Go
    /// `SubscribeEvents`; 256 slots, store.md §3.9).
    pub fn subscribe_rows(&self) -> store::Subscription {
        self.s.store.subscribe()
    }

    /// Log rows (Go `Events`).
    pub fn events(&self, f: &Filter) -> store::Result<Vec<Event>> {
        self.s.store.list(f)
    }

    /// A feed of the services' changes (`topics` empty: all of them).
    pub fn feed(&self, topics: &[Topic]) -> Feed {
        self.s.hub.subscribe(topics)
    }

    /// This device's presence changes (Go `Changes`).
    pub fn changes(&self) -> presence::Subscription {
        self.s.presence.subscribe()
    }

    // ---- notices and sweeps ---------------------------------------------

    /// Tells a notice's recipients through the inbox (`from` is whoever
    /// acted: [`Core::user_sender`] for the window, the agent for a tool).
    pub fn route_notice(&self, from: &Sender, n: &Notice) -> Vec<NoticeResult> {
        self.s.route_notice(from, n)
    }

    /// Every recipient of every notice is tried; failures come back joined.
    pub fn route_notices(&self, from: &Sender, ns: &[Notice]) -> Result<(), String> {
        let errs: Vec<String> = ns
            .iter()
            .filter_map(|n| notice::notice_err(&self.route_notice(from, n)))
            .collect();
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs.join("\n"))
        }
    }

    /// Go `PrivateNotice`: a notice with board keys as this device's inbox
    /// knows them.
    pub fn private_notice(&self, n: Notice) -> Notice {
        self.s.private_notice(n)
    }

    /// The coord sweep (every [`COORD_SWEEP_INTERVAL`]): sweep notices and
    /// the asks' timed part, routed from Irori (or the person).
    pub fn sweep_coord(&self) -> Result<(), String> {
        self.s.sweep_coord()
    }

    /// Asks GitHub about the linked PRs that are due now (card #208); the
    /// Core's own loop does every [`crate::prwatch::TICK`].
    pub fn check_prs(&self) {
        self.s.check_prs(std::time::Instant::now())
    }

    /// [`Self::check_prs`] as if it were `now`: its throttles are minutes.
    pub fn check_prs_at(&self, now: std::time::Instant) {
        self.s.check_prs(now)
    }

    /// Polls the merge desks this device polls that are due now (card
    /// #415); the Core's own loop does every [`crate::deskwatch::DESK_TICK`].
    pub fn check_desks(&self) {
        self.s.check_desks()
    }

    /// Polls one desk's PR list now, whatever its cadence (after a desk
    /// event: a merge, a hold, a settings change). Whether a snapshot row
    /// was written; an error when this device doesn't poll it or GitHub
    /// couldn't be asked.
    pub fn poll_desk(&self, repo_key: &str) -> Result<bool, String> {
        self.s.poll_desk(repo_key)
    }

    /// How polling `repo_key`'s desk stands on this device: the last poll,
    /// and since when GitHub has been unreachable.
    pub fn desk_poll(&self, repo_key: &str) -> crate::deskwatch::DeskPoll {
        self.s.desk_watch.poll(repo_key)
    }

    /// Maps each PR in `repo_key`'s merge desk snapshot to its author card
    /// (merge desk slice 2): links it, makes a "PR #n" card for a trusted
    /// PR of ours with none, leaves anyone else's alone. The desk poller calls it
    /// after each poll; each PR's verdict.
    pub fn map_desk(&self, repo_key: &str) -> Vec<(i64, crate::deskmap::Verdict)> {
        self.s.map_desk(repo_key)
    }

    /// The author card of each of our PRs on `repo_key`'s desk, from the
    /// latest snapshot; anyone else's PR has none and isn't shown (card
    /// #625). Read-only: the desk page's view; [`Self::map_desk`] is the
    /// poller's write.
    pub fn desk_verdicts(&self, repo_key: &str) -> Vec<crate::deskmap::DeskAuthor> {
        self.s.desk_verdicts(repo_key)
    }

    /// The inbox sweep (every [`inbox::SWEEP_INTERVAL`]).
    pub fn sweep_inbox(&self) -> inbox::Result<()> {
        self.s.inbox.sweep()
    }

    /// The roster an ask is told to: the mesh roster without this
    /// device's agents while Pause all is on.
    pub fn ask_agents(&self) -> Vec<AgentCard> {
        self.s.ask_agents()
    }

    /// Go `recordGone` (the inbox's `on_recipient_gone`).
    pub fn record_gone(&self, m: &inbox::Message, to: &str) -> inbox::Result<String> {
        notice::record_gone(&self.s.coord, m, to)
    }

    /// Go `recordPassed` (the inbox's `on_passed`).
    pub fn record_passed(&self, m: &inbox::Message, to: &str) -> inbox::Result<()> {
        notice::record_passed(&self.s.coord, m, to)
    }

    /// Turns presence changes into log rows until `changes` ends (Go
    /// `record`; the Core runs it on its own presence subscription).
    pub fn record_from(&self, changes: &gocompat::sub::Queue<presence::Change>) {
        self.s.record(|| changes.recv());
    }

    /// Rows every fold ignored for the writer rules, per writing device.
    pub fn ignored_by(&self) -> std::collections::BTreeMap<String, i64> {
        ignored_by(&self.s.inbox, &self.s.coord, &self.s.asks, &self.s.roster)
    }

    // ---- decisions, colours, reset --------------------------------------

    /// Go `capture.Decisions(f)`: anchors re-checked, the fallback root a
    /// live local card's known root in the same repo.
    pub fn list_decisions(&self, f: &coord::decision::Filter) -> Vec<crate::DecisionView> {
        self.s.capture.decisions(f)
    }

    /// Each of the person's devices' colour by id (removed devices left
    /// out).
    pub fn device_colors(&self) -> std::collections::BTreeMap<String, String> {
        let mut ds: Vec<coord::devcolor::Device> = self
            .s
            .chain
            .devices()
            .into_iter()
            .filter(|d| !d.revoked)
            .map(|d| coord::devcolor::Device {
                id: d.cert.device_id,
                at: d.cert.at,
            })
            .collect();
        if ds.is_empty() {
            ds.push(coord::devcolor::Device {
                id: self.s.device.id.clone(),
                at: GoTime::ZERO,
            });
        }
        coord::devcolor::assign(&ds, &self.s.colors.chosen())
    }

    /// Records the person's colour pick for a device (`""` goes back to the
    /// default). Only the window calls it.
    pub fn set_device_color(&self, device_id: &str, color: &str) -> Result<(), String> {
        if !color.is_empty() && !coord::devcolor::valid(color) {
            return Err(format!(
                "{} isn't one of the device colours",
                gocompat::quote::quote(color)
            ));
        }
        if !self.device_colors().contains_key(device_id) {
            return Err("that isn't one of your devices".into());
        }
        self.s
            .store
            .append(
                "user",
                coord::devcolor::KIND,
                &DeviceColor {
                    at_device: &self.s.device.id,
                    color,
                    device_id,
                },
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Agents' faces by name key (card #407): validated open-bot-avatar
    /// JSON.
    pub fn agent_avatars(&self) -> std::collections::HashMap<String, String> {
        self.s.faces.all()
    }

    /// Keeps an agent name's face, or clears it (`None`). Agents set their
    /// own through the avatar tool.
    pub fn set_agent_avatar(
        &self,
        name: &str,
        face: Option<&coord::avatar::Avatar>,
    ) -> Result<(), String> {
        set_avatar(&self.s, name, face)
    }

    /// The person's own colour on shared boards, "" when they haven't
    /// picked one (card #385).
    pub fn person_color(&self) -> String {
        self.s.person_color.chosen()
    }

    /// Records the person's colour on shared boards (`""` goes back to the
    /// default each viewer picks). Every shared board hears it on the
    /// person's member.profile. Only the window calls it.
    pub fn set_person_color(&self, color: &str) -> Result<(), String> {
        if !color.is_empty() && !coord::devcolor::valid(color) {
            return Err(format!(
                "{} isn't one of the colours",
                gocompat::quote::quote(color)
            ));
        }
        self.s
            .store
            .append(
                "user",
                coord::devcolor::PERSON_KIND,
                &PersonColor {
                    at_device: &self.s.device.id,
                    color,
                },
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// The person's own avatar on shared spaces, "" for none (card #393).
    pub fn person_avatar(&self) -> String {
        self.s.person_avatar.chosen()
    }

    /// Records the person's avatar (`""` takes it off): a PNG, JPEG or
    /// WebP data URL of at most `coord::person_avatar::MAX_LEN` bytes. Every shared
    /// space hears it on the person's member.profile. Only the window
    /// calls it.
    pub fn set_person_avatar(&self, avatar: &str) -> Result<(), String> {
        if !coord::person_avatar::valid(avatar) {
            return Err(if avatar.len() > coord::person_avatar::MAX_LEN {
                format!(
                    "the picture is too big ({} KB; at most {} KB)",
                    avatar.len() / 1024,
                    coord::person_avatar::MAX_LEN / 1024
                )
            } else {
                "an avatar is a PNG, JPEG or WebP picture".into()
            });
        }
        self.s
            .store
            .append(
                "user",
                coord::person_avatar::KIND,
                &PersonAvatarRow {
                    at_device: &self.s.device.id,
                    avatar,
                },
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// What each of the person's devices is called, by id: the person's
    /// name for it (card #184), else its certificate's label (the
    /// computer's name when it joined), else this device's own name.
    /// Removed devices keep their name, so old lines still read.
    pub fn device_labels(&self) -> std::collections::BTreeMap<String, String> {
        self.s.device_labels()
    }

    /// The person's renames alone, by device id (card #184).
    pub(crate) fn device_renames(&self) -> std::collections::HashMap<String, String> {
        self.s.device_names.chosen()
    }

    /// What device id is called ([`Core::device_labels`]); `""` for one
    /// this device doesn't know.
    pub fn device_label(&self, id: &str) -> String {
        self.device_labels().remove(id).unwrap_or_default()
    }

    /// Renames one of the person's devices for them (`""` goes back to the
    /// computer's name). The row replicates to their other devices and
    /// never to a shared board. Only the window calls it.
    pub fn set_device_name(&self, device_id: &str, name: &str) -> Result<String, String> {
        if !self.device_colors().contains_key(device_id) {
            return Err("that isn't one of your devices".into());
        }
        let name = coord::devname::clean(name);
        let ev = self
            .s
            .store
            .append(
                "user",
                coord::devname::KIND,
                &DeviceName {
                    at_device: &self.s.device.id,
                    device_id,
                    name: &name,
                },
            )
            .map_err(|e| e.to_string())?;
        // Folded now, so the window's next read has it; the follower
        // folding it again is a no-op.
        self.s.device_names.apply(&ev);
        Ok(name)
    }

    /// The work/personal sides each pinned device runs (card #494), by
    /// device id; a device not in it runs every side.
    pub fn device_sides(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        self.s.coord.device_sides()
    }

    /// Every side named on a device's pin or a card (card #494).
    pub fn side_names(&self) -> Vec<String> {
        self.s.coord.side_names()
    }

    /// Pins the sides this device runs, the first the one its new cards
    /// take; empty runs every side (card #494). Only this device: a pin
    /// one device writes for another counts nowhere. Only the window
    /// calls it.
    pub fn set_device_sides(&self, sides: &[String]) -> Result<Vec<String>, String> {
        let sides = self
            .s
            .coord
            .set_device_sides(sides)
            .map_err(|e| e.to_string())?;
        self.s.hub.publish(CoreEvent::Devices);
        Ok(sides)
    }

    /// Marks this device for Reset at its next start.
    pub fn request_reset(&self) -> std::io::Result<()> {
        crate::reset::request_reset(&self.s.data_dir)
    }

    // ---- devices (M7, crate::devices) -------------------------------------

    /// The pairing flow (Go `Pairing`). The window reaches all of it; the
    /// loopback API only the joining side.
    pub fn pairing(&self) -> &irori_pair::Pairer {
        &self.s.pairing.pairer
    }

    /// The runtime pairing and its transport run on.
    pub fn pairing_runtime(&self) -> &tokio::runtime::Handle {
        &self.s.pairing.handle
    }

    /// The guest windows on this device's relay.
    pub fn guest_windows(&self) -> &irori_net::guest::GuestWindows {
        &self.s.pairing.guests
    }

    /// The mesh Node (Go `Mesh()`): `status()` and `devices(detail)` for
    /// `/v1/devices`, `connected_count()` for health, `liveness` for the
    /// roster, `fetch_file` for attachments held on another device.
    pub fn mesh(&self) -> &Arc<mesh::Node> {
        &self.s.mesh.node
    }

    /// A file on a comment or blocker the window shows: the private
    /// board's, else a shared board's (v0.5 §5.5).
    pub fn comment_attachment(&self, sha: &str) -> Option<coord::FileRef> {
        self.s.comment_attachment(sha)
    }

    /// A file on a Talk post (card #643) in a channel or DM `reader` may
    /// read, the earliest such post's; None when there is none.
    pub fn post_attachment(&self, reader: &str, sha: &str) -> Option<blob::FileRef> {
        self.s.post_attachment(reader, sha)
    }

    /// The Core's network side: the Node and every open board's mesh.
    pub(crate) fn mesh_net(&self) -> &Arc<crate::meshnode::Mesh> {
        &self.s.mesh
    }

    /// The iroh endpoint the mesh and pairing share (status lines, tests).
    pub fn net(&self) -> &irori_net::transport::IrohNet {
        &self.s.mesh.net
    }

    /// Opens a shared board on the network (boards stage 2): its member
    /// devices count at the endpoint gate for `irori/board/1` only, and
    /// its rows move on that board's connections. The board-open path
    /// (invites, the M9 router) calls it once the board's log, verifier
    /// and secrets are open.
    pub async fn open_board_net(&self, p: crate::BoardParts) -> Result<(), String> {
        self.s.mesh.open_board(p).await
    }

    /// Opens a board this device holds on the network again, after
    /// [`Core::close_board_net`]: a member coming back online (tests).
    pub fn reopen_board_net(&self, board_id: &str) -> Result<(), String> {
        let b = self.s.board(board_id).map_err(|e| e.to_string())?;
        self.s.open_board_net(&b);
        Ok(())
    }

    /// Closes a shared board on the network (left, removed, forgotten).
    pub fn close_board_net(&self, board_id: &str) {
        self.s
            .mesh
            .close_board(board_id, irori_meshseam::reason::LEAVING);
    }

    /// Direct addresses for another member's device (an invite ticket's
    /// `direct`), dialled for every shared board it is a member device of.
    pub fn board_addr_hint(&self, device_id: &str, addrs: Vec<std::net::SocketAddr>) {
        self.s.mesh.board_hint(device_id, addrs);
    }

    /// After a local write to a shared board: its members pull now.
    pub fn kick_board(&self, board_id: &str) {
        if let Some(m) = self.s.mesh.board(board_id) {
            m.kick();
        }
    }

    /// After a local membership change on a shared board (a removal, a
    /// revoke, a rotation): connections it no longer admits close, and a
    /// newer secret goes to the members still on the older epoch.
    pub fn recheck_board(&self, board_id: &str) {
        self.s.recheck_board_mesh(board_id);
    }

    /// The mesh with its address book (crate::meshview).
    pub(crate) fn mesh_parts(&self) -> &Arc<crate::meshnode::Mesh> {
        &self.s.mesh
    }

    /// Go `RemoveDevice`: revokes a device; its later rows are refused
    /// everywhere and the person key rotates.
    pub async fn remove_device(&self, device_id: &str) -> Result<(), String> {
        match &self.s.revoke {
            Some(f) => f(device_id.to_string(), crate::devices::REMOVED_REASON.into()).await,
            None => Err("this Core has no mesh to remove a device with".into()),
        }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

struct Subs {
    roster: Arc<store::Subscription>,
    asks: Arc<store::Subscription>,
    colors: Arc<store::Subscription>,
    decisions: Arc<store::Subscription>,
}

/// Whether device `dev` wrote an `attach.offered` for `sha` in `st` (card
/// #659): what lets a shared board's row lend a file.
fn offered_in(st: &store::Store, dev: &str, sha: &str) -> bool {
    !dev.is_empty() && mesh::files::holders_in(st, sha).is_ok_and(|h| h.iter().any(|d| d == dev))
}

/// Go `ignoredBy`: the rows every fold ignored for the writer rules, per
/// writing device (Devices' "N of its rows were ignored").
fn ignored_by(
    ib: &Inbox,
    co: &Coord,
    asks: &coord::approvals::Book,
    roster: &Roster,
) -> std::collections::BTreeMap<String, i64> {
    let mut out = std::collections::BTreeMap::new();
    for (d, n) in ib.ignored_by() {
        *out.entry(d).or_insert(0) += n;
    }
    for m in [co.ignored_by(), asks.ignored_by(), roster.ignored_by()] {
        for (d, n) in m {
            *out.entry(d).or_insert(0) += n as i64;
        }
    }
    out
}

/// The DMs an inbox change shows an agent's message in (list-signals.md
/// §3.2), as Talk lists them (`tools::talk::mirrored`): only on the
/// delivery's first row, or a move, which takes the message from the old
/// recipient's DM to the new one's. A notice, a stop or a mention's wake
/// never shows. `d` is the delivery now.
pub(crate) fn dm_change(
    ch: &inbox::Change,
    d: Option<&inbox::Delivery>,
) -> Option<crate::events::TalkChange> {
    use crate::events::TalkChange;
    use inbox::ChangeKind as K;
    // Lost changes may have held a delivery's first row.
    if ch.kind == K::Resync {
        return Some(TalkChange::all());
    }
    if ch.delivery_id.is_empty() || !(ch.created || ch.kind == K::MovedAway) {
        return None;
    }
    let d = d?;
    let m = &d.message;
    // The delivery came from another device before its message did.
    if m.id.is_empty() {
        return Some(TalkChange::all());
    }
    if !tools::talk::mirrors(m) {
        return None;
    }
    let dm = |to: &str| {
        (!to.is_empty() && coord::talk::canon(&m.from.key) != coord::talk::canon(to))
            .then(|| coord::talk::dm_id(&m.from.key, to))
    };
    let mut convs: Vec<String> = dm(&d.to).into_iter().collect();
    if ch.kind == K::MovedAway {
        // moved_to is the new recipient's key.
        convs.extend(dm(&d.moved_to));
    }
    (!convs.is_empty()).then(|| TalkChange::convs(convs))
}

fn load_notifications(st: &Store, h: &coord::notifications::History) {
    let mut f = Filter {
        kinds: vec![
            coord::notifications::POSTED.into(),
            coord::notifications::READ.into(),
        ],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    };
    loop {
        let Ok(evs) = st.list(&f) else { return };
        for ev in &evs {
            h.apply(ev);
        }
        if (evs.len() as i64) < f.limit {
            return;
        }
        f.after = evs[evs.len() - 1].cursor();
    }
}

/// What the device `id` a row came through is called (card #544): the
/// person's name for it, else the computer's, else its id's first letters.
pub(crate) fn via_name(chain: &Chain, names: &coord::devname::Names, id: &str) -> String {
    if id.is_empty() {
        return String::new();
    }
    if let Some(n) = names.chosen().remove(id).filter(|n| !n.is_empty()) {
        return n;
    }
    chain
        .cert(id)
        .map(|c| c.label)
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| id.chars().take(8).collect())
}

fn load_device_names(st: &Store, names: &coord::devname::Names) {
    let mut f = Filter {
        kinds: vec![coord::devname::KIND.into()],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    };
    loop {
        let Ok(evs) = st.list(&f) else { return };
        for ev in &evs {
            names.apply(ev);
        }
        if (evs.len() as i64) < f.limit {
            return;
        }
        f.after = evs[evs.len() - 1].cursor();
    }
}

fn load_colors(
    st: &Store,
    colors: &coord::devcolor::Picks,
    person: &coord::devcolor::PersonPick,
    faces: &coord::avatar::Faces,
    avatar: &coord::person_avatar::PersonAvatar,
    assistant: &coord::assistant_name::AssistantName,
) {
    let mut f = Filter {
        kinds: vec![
            coord::devcolor::KIND.into(),
            coord::devcolor::PERSON_KIND.into(),
            coord::avatar::KIND.into(),
            coord::person_avatar::KIND.into(),
            coord::assistant_name::KIND.into(),
        ],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    };
    loop {
        let Ok(evs) = st.list(&f) else { return };
        for ev in &evs {
            colors.apply(ev);
            person.apply(ev);
            faces.apply(ev);
            avatar.apply(ev);
            assistant.apply(ev);
        }
        if (evs.len() as i64) < f.limit {
            return;
        }
        f.after = evs[evs.len() - 1].cursor();
    }
}

/// Go `reviveForAsk`: revives `target`, a session on this device, once
/// its revive ask was answered for exactly it.
fn revive_for_ask(rv: &Mutex<Option<ReviveFn>>, me: &str, target: &str) -> Result<(), String> {
    let f = rv.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let Some(f) = f else {
        return Err("no desktop app runs on this device to revive it".into());
    };
    match target.split_once('/') {
        Some((dev, session)) if dev == me => f(session).map(|_| ()),
        _ => Err(format!("{target} is not a session on this device")),
    }
}

impl Services {
    /// An agent key's name: its card's in the mesh roster, else the last
    /// name it went by (gone agents, Grok members after a restart; cards
    /// #98, #483), or `""`.
    fn agent_name(&self, key: &str) -> String {
        let live = self
            .roster
            .agents()
            .into_iter()
            .find(|a| a.key == key)
            .map(|a| a.name)
            .unwrap_or_default();
        if !live.trim().is_empty() {
            return live;
        }
        let known = self.agent_names.get(key);
        if known.is_empty() && key.starts_with(kinds::AGENT_ACTOR_PREFIX) {
            let mut names = self.agent_names.all();
            self.card_key_names(&mut names);
            return names.remove(key).unwrap_or_default();
        }
        known
    }

    /// See [`Core::device_labels`].
    pub(crate) fn device_labels(&self) -> std::collections::BTreeMap<String, String> {
        let mut out: std::collections::BTreeMap<String, String> = self
            .chain
            .devices()
            .into_iter()
            .map(|d| (d.cert.device_id, d.cert.label))
            .collect();
        let me = &self.device;
        let mine = out.entry(me.id.clone()).or_default();
        if mine.is_empty() {
            *mine = me.name.clone();
        }
        for (id, name) in self.device_names.chosen() {
            if let Some(l) = out.get_mut(&id) {
                *l = name;
            }
        }
        out
    }
}

impl Services {
    /// Records the names on the mesh roster in the name book (card #98).
    fn note_agent_names(&self) {
        self.agent_names
            .note(&self.roster.agents(), presence::now_local().secs);
    }

    fn user_sender(&self) -> Sender {
        Sender::new(&format!("user:{}", self.device.id), "you", "user")
    }

    /// A Talk post that mentions the person or is a DM to them, posted in
    /// the last few minutes, becomes a notification here (card #395,
    /// design §8); a plain channel post only bumps the unread count. A
    /// tap opens the conversation.
    pub(crate) fn talk_note(&self, ev: &Event) {
        const FRESH: Duration = Duration::from_secs(10 * 60);
        let Ok(r) = serde_json::from_slice::<coord::talk::PostRow>(&ev.payload) else {
            return;
        };
        let Some(p) = self
            .talk
            .notifies(&r.conv_id, &r.post_id, &self.user_sender().key)
        else {
            return;
        };
        // A device catching up on old posts doesn't ring for each.
        let age = (self.now)().unix_milli_value() - p.at.unix_milli_value();
        if age >= FRESH.as_millis() as i64 {
            return;
        }
        let who = if coord::talk::is_person_key(&p.by) {
            "You".to_string()
        } else {
            // As Talk's list names it: the live card's, else the last name
            // the agent went by.
            self.local_agents()
                .into_iter()
                .map(|a| (a.key, a.name))
                .chain(self.roster.agents().into_iter().map(|a| (a.key, a.name)))
                .find(|(k, n)| *k == p.by && !n.trim().is_empty())
                .map(|(_, n)| n)
                .or_else(|| {
                    self.note_agent_names();
                    self.agent_names.all().remove(&p.by)
                })
                .filter(|n| !n.trim().is_empty())
                // Another person or their agent in a space's channel; a
                // key never stands in for a name (card #478).
                .or_else(|| self.space_names().remove(&p.by))
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| "An agent".to_string())
        };
        let title = match self.talk.channel(&p.conv_id) {
            Some(c) => format!("{who} in #{}", c.name),
            None => who,
        };
        let n = self.notifier.0.clone();
        let conv = p.conv_id.clone();
        self.notifier.0.post(notify::Note {
            id: notify::talk_id(&p.conv_id, &p.post_id),
            title,
            body: notify::preview(&tools::talk::post_line(&p)),
            on_click: Some(Arc::new(move || {
                n.open(notify::Target {
                    tab: notify::TAB_TALK.into(),
                    ref_: conv.clone(),
                })
            })),
            ..notify::Note::default()
        });
    }

    fn local_agents(&self) -> Vec<presence::AgentCard> {
        let mut cs = self.presence.snapshot();
        cs.extend(self.members.cards());
        cs
    }

    /// Go `AttachFiles` ([`Core::attach_files`]).
    fn attach_files(&self, by: &str, paths: &[String]) -> Result<Vec<blob::FileRef>, blob::Error> {
        use crate::attachments::{base, trim_space};
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        if paths.len() > inbox::MAX_ATTACHMENTS {
            return Err(blob::Error::new(
                blob::ErrorKind::Other,
                format!("at most {} attachments", inbox::MAX_ATTACHMENTS),
            ));
        }
        let cwd = self
            .local_agents()
            .into_iter()
            .find(|a| a.key == by)
            .map(|a| a.cwd)
            .unwrap_or_default();
        let data = self.data_dir.to_string_lossy();
        let mut out = Vec::with_capacity(paths.len());
        for p in paths {
            let real = blob::check_attach_path(p, &cwd, &data)?;
            let mut r = self
                .files
                .put_file(&real)
                .map_err(|e| blob::Error::new(e.kind(), format!("{}: {e}", base(&real))))?;
            // The name the agent gave, reduced to one safe file name.
            r.name = blob::safe_name(&base(trim_space(p)), &r.sha256);
            r.device = self.device.id.clone();
            self.mesh
                .node
                .offer(&r)
                .map_err(|e| blob::Error::new(blob::ErrorKind::Other, e))?;
            out.push(r);
        }
        Ok(out)
    }

    /// A file on a comment or blocker: the private board's first, then
    /// each open shared board's (v0.5 §5.5).
    pub(crate) fn comment_attachment(&self, sha: &str) -> Option<coord::FileRef> {
        self.comment_files(sha).into_iter().next().map(|(f, _)| f)
    }

    /// [`Self::comment_attachment`]'s candidates, each with where it was
    /// found: None for the private board, which only this person's
    /// devices write, else the shared board (its earliest row that
    /// counts). On a shared board a comment or blocker counts only where
    /// the device whose row put the file there offered it in that board's
    /// log (card #659): a member's row naming a hash it never held lends
    /// nothing.
    fn comment_files(&self, sha: &str) -> Vec<(coord::FileRef, Option<String>)> {
        let mut out = Vec::new();
        if let Some(f) = self.coord.comment_attachment(sha) {
            out.push((f, None));
        }
        for b in self
            .boards
            .list()
            .iter()
            .filter(|b| !self.is_dm_space(&b.id))
        {
            if let Some((f, _)) = b.coord().attachment_rows(sha).into_iter().find(|(f, dev)| {
                offered_in(&b.store, dev, sha) || offered_in(&b.store, &f.device, sha)
            }) {
                out.push((f, Some(b.id.clone())));
            }
        }
        out
    }

    /// A file on a Talk post (card #643) in a channel or DM `reader` may
    /// read ([`tools::talk::can_read`], [`tools::talk::posts`]'s rule:
    /// anyone reads a channel this device folds, a DM its parties and the
    /// person): the earliest such post's. A post in a conversation the
    /// reader can't read never lends its file, even when the same file is
    /// on one they can. It asks per image the window shows, so it never
    /// lists the conversation's posts to decide.
    pub(crate) fn post_attachment(&self, reader: &str, sha: &str) -> Option<blob::FileRef> {
        self.post_files(reader, sha)
            .into_iter()
            .next()
            .map(|(f, _)| f)
    }

    /// [`Self::post_attachment`]'s candidates, earliest first, each with
    /// where it was found: None for a post in the private log, else the
    /// space whose log holds it (once per place). A space's post counts
    /// only where the device that wrote it offered the file in that
    /// space's log (card #659).
    fn post_files(&self, reader: &str, sha: &str) -> Vec<(blob::FileRef, Option<String>)> {
        let found = self.talk.attachments(sha);
        if found.is_empty() {
            return Vec::new();
        }
        let Some(me) = self.this.get().and_then(Weak::upgrade) else {
            return Vec::new();
        };
        let view = ToolsView(me);
        let mut out: Vec<(blob::FileRef, Option<String>)> = Vec::new();
        for pf in found {
            let space = (!pf.space.is_empty()).then(|| pf.space.clone());
            if out.iter().any(|(_, s)| *s == space) {
                continue;
            }
            if let Some(id) = &space {
                let Ok(b) = self.board(id) else { continue };
                if !offered_in(&b.store, &pf.device, sha)
                    && !offered_in(&b.store, &pf.file.device, sha)
                {
                    continue;
                }
            }
            if tools::talk::can_read(&view, reader, &pf.conv) {
                out.push((pf.file, space));
            }
        }
        out
    }

    /// Go `GetAttachment` ([`Core::get_attachment`]).
    fn get_attachment(
        &self,
        reader: &str,
        sha: &str,
        to: &str,
    ) -> Result<(String, String), String> {
        use crate::attachments::{self as at, trim_space};
        use crate::meshview::FileFetchError;
        let sha = gocompat::text::to_lower_str(trim_space(sha));
        if !blob::valid_id(&sha) {
            return Err("blob: id must be 64 lowercase hex characters".into());
        }
        // Every place the reader may read it, in order: its messages, the
        // private board's comments, each shared board's, then Talk posts
        // (card #643). A shared board's or space's goes through that board
        // alone (card #659), so each is tried until one brings the file: a
        // forged row elsewhere never stands in the way of a real one.
        let to_ref = |f: coord::FileRef| blob::FileRef {
            sha256: f.sha256,
            name: f.name,
            size: f.size,
            mime: f.mime,
            device: f.device,
        };
        let mut cands: Vec<(blob::FileRef, Option<String>)> = Vec::new();
        if let Some(f) = self.inbox.attachment(reader, &sha) {
            let r = blob::FileRef {
                sha256: f.sha256,
                name: f.name,
                size: f.size,
                mime: f.mime,
                device: f.device,
            };
            cands.push((r, None));
        }
        cands.extend(
            self.comment_files(&sha)
                .into_iter()
                .map(|(f, b)| (to_ref(f), b)),
        );
        cands.extend(self.post_files(reader, &sha));
        if cands.is_empty() {
            return Err(at::NO_ATTACHMENT.into());
        }
        let to = trim_space(to);
        let dst_for = |r: &blob::FileRef| {
            at::attachment_dst(
                &self.data_dir,
                reader,
                &blob::safe_name(&r.name, &sha),
                to,
                || {
                    self.local_agents()
                        .into_iter()
                        .rfind(|a| a.key == reader)
                        .map(|a| a.cwd)
                        .unwrap_or_default()
                },
            )
        };
        // A `to` it refuses is refused before anything is fetched.
        dst_for(&cands[0].0)?;
        let mut got: Option<(blob::FileRef, String)> = None;
        let mut last = String::new();
        for (r, board) in &cands {
            match crate::meshview::fetch_file_blocking(
                &self.mesh,
                &self.pairing.handle,
                &self.device.id,
                &self.chain,
                &self.device_names.chosen(),
                r,
                board.as_deref(),
            ) {
                Ok(from) => {
                    got = Some((r.clone(), from));
                    break;
                }
                Err(e) => {
                    last = match e {
                        FileFetchError::Offline(on) => at::offline_text(&on),
                        FileFetchError::TimedOut => at::TIMED_OUT.into(),
                        FileFetchError::Other(s) => s,
                    }
                }
            }
        }
        let Some((r, from)) = got else {
            return Err(last);
        };
        let dst = dst_for(&r)?;
        let shown = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let dst = if to.is_empty() {
            let dst = at::free_name(&dst, &sha);
            if blob::same_content(&dst, &sha) {
                return Ok((shown(&dst), from));
            }
            dst
        } else {
            if std::fs::symlink_metadata(&dst).is_ok() {
                if blob::same_content(&dst, &sha) {
                    return Ok((shown(&dst), from));
                }
                return Err(format!("{} already exists; give another to", shown(&dst)));
            }
            dst
        };
        self.files.copy_out(&sha, &dst).map_err(|e| e.to_string())?;
        Ok((shown(&dst), from))
    }

    fn add_secret(&self, v: &str) {
        if v.is_empty() {
            return;
        }
        let mut s = self.secrets.lock().unwrap_or_else(|e| e.into_inner());
        s.push(v.to_string());
        self.scrubber.set_exact(&s);
    }

    fn route_notice(&self, from: &Sender, n: &Notice) -> Vec<NoticeResult> {
        notice::route(&self.inbox, from, &self.private_notice(n.clone()))
    }

    fn ask_agents(&self) -> Vec<AgentCard> {
        let all = self.roster.agents();
        if !self.inbox.policy().paused {
            return all;
        }
        let me = self.device.id.as_str();
        all.into_iter()
            .filter(|a| {
                let dev = a.device_id.as_str();
                !(dev == me || dev.is_empty() && irori_identity::owner::device_of(&a.key) == me)
            })
            .collect()
    }

    /// Hands out and on this device's asks, the private Board's and each
    /// shared board's, and tells the agents they went to.
    fn hand_out_asks(self: &Arc<Self>) {
        self.sweep_spawns();
        let (asks, err) = self.coord.sweep_asks_partial(&self.ask_agents());
        for n in &asks {
            let from = if n.person {
                self.user_sender()
            } else {
                inbox::system_sender()
            };
            if let Some(e) = notice::notice_err(&self.route_notice(&from, n)) {
                logf(format_args!("ask notice {} {}: {e}", n.kind, n.reference));
            }
        }
        if let Err(e) = err {
            logf(format_args!("asks: {e}"));
        }
        if let Err(e) = self.sweep_board_asks() {
            logf(format_args!("board asks: {e}"));
        }
        // A thread this pass found nobody free for starts its agent now,
        // not at the next kick (card #512), so the person never sees it
        // sit unclaimed.
        self.sweep_spawns();
    }

    /// Looks for this device's idle agents whose wakes went unread (#110)
    /// and has presence show them as not answering, so the roster, the
    /// hand-outs and other devices see it. Clears as soon as one acts.
    fn check_answering(&self) {
        let (newly, changed) = self.answering.refresh(
            &self.presence.snapshot(),
            inbox::NOT_ANSWERING_AFTER,
            |k, after| self.inbox.unanswered_since(k, after),
        );
        for k in &newly {
            logf(format_args!(
                "{k} isn't answering: a wake waited on it unread"
            ));
        }
        if changed {
            self.presence.rescan();
        }
    }

    /// Every card this device can see, in its private log and on the
    /// boards it shows, with keys as this device knows them, and the log
    /// each one lives in (by card id).
    fn desk_cards(
        &self,
        open_only: bool,
    ) -> (
        Vec<coord::Task>,
        std::collections::HashMap<String, Arc<Coord>>,
    ) {
        let f = coord::TaskFilter {
            open: open_only,
            ..coord::TaskFilter::default()
        };
        let mut coords = vec![self.coord.clone()];
        coords.extend(self.visible_boards().iter().map(|b| b.coord().clone()));
        let mut tasks = Vec::new();
        let mut home = std::collections::HashMap::new();
        for co in coords {
            for t in co.list_tasks(&f) {
                home.entry(t.id.clone()).or_insert_with(|| co.clone());
                tasks.push(self.private_task(t));
            }
        }
        (tasks, home)
    }

    /// The author of each of our PRs on `repo_key`'s desk, worked out from
    /// the latest snapshot without writing anything (the desk page's
    /// read). A PR that isn't ours has no entry: the page doesn't list it
    /// (card #625).
    fn desk_verdicts(&self, repo_key: &str) -> Vec<crate::deskmap::DeskAuthor> {
        use crate::deskmap::{DeskAuthor, PrRef, Trust, Verdict, judge};
        let Some(desk) = self.coord.desk(repo_key) else {
            return Vec::new();
        };
        let (Some(snap), Some(repo)) = (desk.prs.as_ref(), coord::github_repo(repo_key)) else {
            return Vec::new();
        };
        let own = self.people_agent();
        let ledger = self.capture.prs(&repo);
        let (tasks, _) = self.desk_cards(false);
        let logins = crate::deskmap::desk_logins(&desk, &self.people_logins());
        snap.prs
            .iter()
            .filter_map(|p| {
                let author = ledger
                    .iter()
                    .find(|e| e.number == p.number && e.role.as_str() == "author")
                    .map(|e| e.session.clone())
                    .unwrap_or_default();
                let trust = Trust {
                    own_agent: &own,
                    logins: &logins,
                };
                let mut a = DeskAuthor {
                    pr: p.number,
                    agent: author.clone(),
                    review_note: self.desk_watch.note(repo_key, p.number),
                    ..DeskAuthor::default()
                };
                match judge(&PrRef::of(&repo, p), &tasks, &author, &trust) {
                    Verdict::NotOurs => return None,
                    Verdict::NewCard(_) => {}
                    Verdict::Card(m) => {
                        if let Some(t) = tasks.iter().find(|t| t.id == m.task_id) {
                            if a.agent.is_empty() {
                                a.agent = t.holders.first().cloned().unwrap_or_default();
                            }
                            a.card_ref = t.short_ref();
                        }
                        a.card = m.task_id;
                    }
                }
                Some(a)
            })
            .collect()
    }

    /// Maps each PR in a desk's latest snapshot to its author card (merge
    /// desk §3, slice 2): links the card that rule 1-3 found, makes a
    /// "PR #n" card for one of our PRs with none, and leaves anyone else's
    /// alone (not ours: no card, no wake, not shown; card #625). Returns
    /// each PR's verdict.
    fn map_desk(&self, repo_key: &str) -> Vec<(i64, crate::deskmap::Verdict)> {
        use crate::deskmap::{PrRef, Trust, Verdict, judge, new_card_title};
        let Some(desk) = self.coord.desk(repo_key) else {
            return Vec::new();
        };
        let (Some(snap), Some(repo)) = (desk.prs.as_ref(), coord::github_repo(repo_key)) else {
            return Vec::new();
        };
        let own = self.people_agent();
        let ledger = self.capture.prs(&repo);
        let (mut tasks, home) = self.desk_cards(false);
        let logins = crate::deskmap::desk_logins(&desk, &self.people_logins());
        // Other members' logins: a PR of theirs gets its card from their
        // device, in the space we share (merge desk §12.2, card #1077).
        let mine = [self.own_github_login(), snap.viewer.clone()];
        let others: Vec<String> = self
            .people_logins()
            .into_iter()
            .filter(|l| !mine.iter().any(|m| m.eq_ignore_ascii_case(l)))
            .collect();
        let mut out = Vec::new();
        for p in &snap.prs {
            let pr = PrRef::of(&repo, p);
            let author = ledger
                .iter()
                .find(|e| e.number == p.number && e.role.as_str() == "author")
                .map(|e| e.session.clone())
                .unwrap_or_default();
            let trust = Trust {
                own_agent: &own,
                logins: &logins,
            };
            let v = judge(&pr, &tasks, &author, &trust);
            let made = match &v {
                Verdict::Card(m) => Some(m.task_id.clone()),
                // A member's PR: their device makes its PR card in the space
                // we share, and a private card made here would be one nobody
                // else can claim on (merge desk §12.2, card #1077).
                Verdict::NewCard(_)
                    if !p.author.is_empty()
                        && !desk.settings.reviews_off
                        && crate::prcard::needs_card(p, &[])
                        && others.iter().any(|l| l.eq_ignore_ascii_case(&p.author)) =>
                {
                    None
                }
                Verdict::NewCard(m) => {
                    let nt = coord::NewTask {
                        title: new_card_title(&pr),
                        objective: format!(
                            "PR #{} on {repo} {}; this is its thread on the merge desk.",
                            p.number,
                            coord::DESK_PR_CARD_MARK
                        ),
                        repo_key: repo_key.into(),
                        ..coord::NewTask::default()
                    };
                    match self.coord.create_task("irori", nt) {
                        Ok(t) => {
                            let to = &m.author;
                            if !to.is_empty()
                                && let Err(e) = self.coord.assign_task("irori", &t.id, to)
                            {
                                logf(format_args!("desk: PR #{} card {}: {e}", p.number, t.id));
                            }
                            Some(t.id)
                        }
                        Err(e) => {
                            logf(format_args!("desk: PR #{} card: {e}", p.number));
                            None
                        }
                    }
                }
                Verdict::NotOurs => None,
            };
            // A PR card already links its PR, and is its author's to write.
            let own_link = made
                .as_ref()
                .is_some_and(|id| tasks.iter().any(|t| &t.id == id && coord::pr_card(t)));
            if let Some(id) = made.filter(|_| !own_link) {
                let co = home.get(&id).unwrap_or(&self.coord);
                match co.add_link("irori", &id, &p.url) {
                    Ok(t) => {
                        // Rule 1 finds it next time, and in this pass.
                        tasks.retain(|x| x.id != t.id);
                        tasks.push(self.private_task(t));
                    }
                    Err(e) => logf(format_args!("desk: link {} to {id}: {e}", p.url)),
                }
            }
            out.push((p.number, v));
        }
        out
    }

    /// Links a PR an agent on this device just opened to the one card it
    /// holds (merge desk §3, slice 2), so the desk's rule 1 finds it.
    fn link_opened_prs(&self, session: &str) {
        let here = self
            .presence
            .snapshot()
            .into_iter()
            .any(|c| c.key == session);
        if !here {
            return;
        }
        let (tasks, home) = self.desk_cards(true);
        let now = (self.now)().unix_milli_value();
        for e in self.capture.prs("") {
            if e.session != session || e.role.as_str() != "author" || e.source == "pr-link" {
                continue;
            }
            let o = crate::deskmap::Opened {
                session: e.session.clone(),
                repo: e.repo.clone(),
                url: e.url.clone(),
                at_ms: e.at.unix_milli_value(),
            };
            if let Some(id) = crate::deskmap::opened_card(&o, &tasks, now)
                && let Err(err) = home
                    .get(&id)
                    .unwrap_or(&self.coord)
                    .add_link("irori", &id, &e.url)
            {
                logf(format_args!("desk: link {} to {id}: {err}", e.url));
            }
        }
    }

    /// Asks GitHub about the PRs that are due (card #208) and routes what
    /// coord says changed.
    fn check_prs(self: &Arc<Self>, now: std::time::Instant) {
        // First, while another person's row is still theirs alone: a poll
        // here hearing the same would count as this person's word for it.
        self.tell_pr_news();
        if let Some(checker) = self.pr_checker.clone().filter(|c| c.ready()) {
            self.ask_github(checker.as_ref(), now);
        }
        // And what folded while gh answered.
        self.tell_pr_news();
    }

    /// [`tools::Backend::recheck_prs`] (card #970): asks GitHub about each
    /// PR card `task_id` links that `co` last heard open, and records a
    /// merge or a closing. The caller, completing the card, isn't told
    /// what it just did; anyone else on the card is. An open answer or a
    /// failure changes nothing here: the poller goes on as before.
    fn recheck_prs(self: &Arc<Self>, co: &Arc<Coord>, caller: &str, task_id: &str) {
        let Some(checker) = self.pr_checker.clone().filter(|c| c.ready()) else {
            return;
        };
        for p in co.pr_states(task_id) {
            if p.state != coord::PR_OPEN {
                continue;
            }
            let c = checker.check(&p.pr);
            // A failure is the poller's to note and say, not this one's.
            if !c.error.is_empty() {
                continue;
            }
            self.pr_watch.answered(task_id, &p.pr, true);
            if c.state != coord::PR_MERGED && c.state != coord::PR_CLOSED {
                continue;
            }
            // Overtaken while gh answered: what folded since stands.
            if co.pr_state(task_id, &p.pr).map(|q| q.state).as_deref() != Some(coord::PR_OPEN) {
                continue;
            }
            match co.record_pr_state(task_id, c) {
                Ok((_, ns)) => {
                    for mut n in ns {
                        // A board card's recipients are in its board form.
                        n.to.retain(|k| self.private_key(k) != caller);
                        if n.to.is_empty() {
                            continue;
                        }
                        if let Some(e) =
                            notice::notice_err(&self.route_notice(&inbox::system_sender(), &n))
                        {
                            logf(format_args!("pr notice {}: {e}", n.reference));
                        }
                    }
                }
                Err(e) => logf(format_args!("pr {} on {task_id}: {e}", p.pr)),
            }
        }
    }

    fn ask_github(
        self: &Arc<Self>,
        checker: &dyn crate::prwatch::PrChecker,
        now: std::time::Instant,
    ) {
        let local: std::collections::HashSet<String> = self
            .presence
            .snapshot()
            .into_iter()
            .map(|c| c.key)
            .collect();
        let mut coords = vec![self.coord.clone()];
        coords.extend(self.visible_boards().iter().map(|b| b.coord().clone()));
        let due = self
            .pr_watch
            .due(&coords, &local, &|k| self.private_key(k), now);
        for d in due {
            let known = |co: &coord::Coord| {
                co.pr_state(&d.task_id, &d.url)
                    .filter(|p| !p.state.is_empty())
                    .map(|p| (p.state, p.review))
            };
            let before = known(&d.co);
            let c = checker.check(&d.url);
            self.pr_watch
                .answered(&d.task_id, &d.url, c.error.is_empty());
            // Overtaken while gh answered (card #548): an OPEN from before
            // a closing that just folded is no reopen. The next poll asks.
            if known(&d.co) != before {
                continue;
            }
            match d.co.record_pr_state(&d.task_id, c) {
                Ok((_, ns)) => {
                    for n in ns {
                        if let Some(e) =
                            notice::notice_err(&self.route_notice(&inbox::system_sender(), &n))
                        {
                            logf(format_args!("pr notice {}: {e}", n.reference));
                        }
                    }
                }
                Err(e) => logf(format_args!("pr {} on {}: {e}", d.url, d.task_id)),
            }
        }
    }

    /// The desks this device polls: enabled, taking PRs, on GitHub, and
    /// naming this device as `pollDevice`.
    fn polled_desks(&self) -> Vec<coord::Desk> {
        self.coord
            .desks()
            .into_iter()
            .filter(|d| {
                d.on()
                    && d.settings.poll_device == self.device.id
                    && coord::github_repo(&d.settings.repo_key).is_some()
            })
            .collect()
    }

    /// The person's GitHub login: as this run's gh said it, else as the
    /// newest snapshot from any of their devices heard it; "" unknown.
    pub(crate) fn own_github_login(&self) -> String {
        let v = self.desk_watch.known_viewer();
        if !v.is_empty() {
            return v;
        }
        self.coord
            .desks()
            .into_iter()
            .filter_map(|d| d.prs)
            .filter(|s| !s.viewer.trim().is_empty())
            .max_by_key(|s| s.at.unix_milli_value())
            .map(|s| s.viewer.trim().to_string())
            .unwrap_or_default()
    }

    /// The GitHub logins whose PRs are ours (card #625): the person's own
    /// and each active member's of the spaces they are active in, as each
    /// member's profile says. Lower case, once each. A hidden DM space's
    /// rows reach nobody here (card #538), so its other member doesn't
    /// count through it.
    fn people_logins(&self) -> Vec<String> {
        let mut out = std::collections::BTreeSet::new();
        let own = self.own_github_login();
        if !own.is_empty() {
            out.insert(own.to_ascii_lowercase());
        }
        let me = self.person_id();
        for b in self.visible_boards() {
            let m = b.verifier.membership();
            if !m.active(&me) {
                continue;
            }
            for p in m.members.iter().filter(|p| !p.person_id.is_empty()) {
                if !m.active(&p.person_id) {
                    continue;
                }
                let l = b.verifier.profile_github_login(&p.person_id);
                if !l.trim().is_empty() {
                    out.insert(l.trim().to_ascii_lowercase());
                }
            }
        }
        out.into_iter().collect()
    }

    /// Whether an agent key on a card this device sees is the person's
    /// agent or a space member's (card #625). The person's own agents read
    /// `<device>/<session>`; a member's stay `agent:<id>` on the shared
    /// board's card, and only members write there.
    fn people_agent(&self) -> impl Fn(&str) -> bool + '_ {
        move |k: &str| {
            k.starts_with(store::kinds::AGENT_ACTOR_PREFIX)
                || k.split_once('/')
                    .is_some_and(|(dev, _)| self.chain.is_owner(&format!("user:{dev}")))
        }
    }

    /// What says which repos are the person's (card #625). `lister` also
    /// searches GitHub for each login's PRs, at most every
    /// [`crate::deskwatch::AUTHORED_EVERY`].
    fn desk_evidence(
        &self,
        lister: Option<&dyn crate::deskwatch::PrLister>,
    ) -> crate::deskmap::RepoEvidence {
        let logins = self.people_logins();
        let mut pr_repos: Vec<String> = Vec::new();
        // The ledger: PRs the person's agents opened (gh pr create).
        let own = self.people_agent();
        for e in self.capture.prs("") {
            if e.role.as_str() == "author"
                && e.source != "pr-link"
                && !e.session.starts_with(store::kinds::AGENT_ACTOR_PREFIX)
                && own(&e.session)
            {
                pr_repos.push(e.repo.clone());
            }
        }
        // Cards the person and their members made that link a PR; the
        // desk's own "PR #n" cards (made by Irori) don't count.
        let (tasks, _) = self.desk_cards(false);
        for t in tasks.iter().filter(|t| t.created_by != "irori") {
            for l in &t.links {
                if let Some((repo, _)) = coord::pr_link(l) {
                    pr_repos.push(repo);
                }
            }
        }
        // GitHub: the repos each login opened PRs in.
        if let Some(lister) = lister {
            let now = std::time::Instant::now();
            for l in &logins {
                pr_repos.extend(self.desk_watch.authored(lister, l, now));
            }
        }
        pr_repos.sort_by_key(|r| r.to_ascii_lowercase());
        pr_repos.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        crate::deskmap::RepoEvidence { logins, pr_repos }
    }

    /// Tells coord which desks' repos are the person's (card #625); any
    /// other desk is stray: off, never polled or listed. No rows: older
    /// devices keep their own view until they update.
    fn check_desk_repos(&self, lister: Option<&dyn crate::deskwatch::PrLister>) {
        let ev = self.desk_evidence(lister);
        let desks = self.coord.desks();
        let ours: Vec<String> = desks
            .iter()
            .filter(|d| crate::deskmap::our_repo(d, &ev))
            .map(|d| d.settings.repo_key.clone())
            .collect();
        let judged = desks.into_iter().map(|d| d.settings.repo_key);
        if self.coord.judge_desk_repos(judged, ours) {
            logf(format_args!(
                "desks: {} of the person's repos",
                self.coord.desks().iter().filter(|d| !d.stray).count()
            ));
        }
    }

    fn check_desks(self: &Arc<Self>) {
        let Some(lister) = self.desk_lister.clone() else {
            self.check_desk_repos(None);
            return;
        };
        // The person's login, asked of gh once a run, before any desk is
        // on: it is what makes a repo theirs. Once learned, the spaces'
        // profiles get it so members' desks show this person's PRs.
        let had = self.desk_watch.known_viewer();
        let login = self.desk_watch.viewer(lister.as_ref());
        if had.is_empty() && !login.is_empty() {
            for b in self.boards.list() {
                if let Err(e) = self.keep_current(&b) {
                    logf(format_args!("board {}: github login: {e}", b.id));
                }
            }
        }
        self.check_desk_repos(Some(lister.as_ref()));
        let now = std::time::Instant::now();
        for d in self.polled_desks() {
            let k = d.settings.repo_key;
            if !self.desk_watch.due(&k, now) {
                continue;
            }
            if let Err(e) = self.poll_desk(&k) {
                logf(format_args!("desk {k}: {e}"));
            }
        }
    }

    fn poll_desk(self: &Arc<Self>, repo_key: &str) -> Result<bool, String> {
        let Some(lister) = self.desk_lister.clone() else {
            return Err("this Irori polls no merge desk".into());
        };
        if !self
            .polled_desks()
            .iter()
            .any(|d| coord::same_repo_key(&d.settings.repo_key, repo_key))
        {
            return Err(format!(
                "this device doesn't poll {}'s merge desk",
                coord::repo_name(repo_key)
            ));
        }
        let repo = coord::github_repo(repo_key).unwrap_or_default();
        let limit = crate::deskwatch::LIMIT;
        let heard = lister.list(&repo, limit);
        let at = gocompat::time::GoTime::from_system(std::time::SystemTime::now()).utc();
        let now = std::time::Instant::now();
        match heard {
            Ok(prs) => {
                let truncated = prs.len() >= limit;
                // The person's own login, which the desk trusts (card #521).
                let viewer = self.desk_watch.viewer(lister.as_ref());
                let wrote = self
                    .coord
                    .record_desk_snapshot(repo_key, prs, truncated, &viewer)
                    .map_err(|e| e.to_string());
                self.desk_watch.heard(
                    repo_key,
                    wrote.as_ref().map(|_| ()).map_err(|e| e.clone()),
                    now,
                    at,
                );
                // Every good poll, not only a changed one: a card or a ledger
                // row may have changed what a PR maps to.
                if wrote.is_ok() {
                    let v = self.map_desk(repo_key);
                    self.post_desk(repo_key, &v);
                    // Merge desk §12.2 (card #1077): the PR card in a
                    // shared space, and what GitHub says on the PR.
                    self.pr_cards(repo_key, &v);
                    // Card #421: a PR that left the list may have merged.
                    self.settle_gone_prs(repo_key);
                }
                wrote
            }
            Err(e) => {
                // The previous snapshot stays: the queue never reads empty
                // because GitHub couldn't be asked.
                self.desk_watch.heard(repo_key, Err(e.clone()), now, at);
                Err(e)
            }
        }
    }

    /// Irori's posts on each mapped PR's author card after a desk poll
    /// (merge desk slice 6, card #420; [`crate::deskpost`]). What was
    /// already said is read from the card's own Irori comments. A PR with
    /// no card of ours yet (not ours, or a "PR #n" card made this pass)
    /// waits for the next poll.
    fn post_desk(&self, repo_key: &str, verdicts: &[(i64, crate::deskmap::Verdict)]) {
        use crate::deskpost::{self, IRORI, Post};
        let Some(desk) = self.coord.desk(repo_key) else {
            return;
        };
        let (Some(snap), Some(repo)) = (desk.prs.as_ref(), coord::github_repo(repo_key)) else {
            return;
        };
        let (_, home) = self.desk_cards(false);
        for (n, v) in verdicts {
            let crate::deskmap::Verdict::Card(m) = v else {
                continue;
            };
            let Some(p) = snap.prs.iter().find(|p| p.number == *n) else {
                continue;
            };
            // A shared PR card (merge desk §12.2) is nobody's to post on from
            // here: its board takes no Irori comment from another member's
            // device, and nobody holds it to hear one.
            if home
                .get(&m.task_id)
                .and_then(|c| c.get_task(&m.task_id))
                .is_some_and(|t| coord::pr_card(&t))
            {
                continue;
            }
            let co = home
                .get(&m.task_id)
                .cloned()
                .unwrap_or_else(|| self.coord.clone());
            let said: Vec<String> = co
                .comments(&m.task_id)
                .unwrap_or_default()
                .into_iter()
                .filter(|c| c.by == IRORI)
                .map(|c| c.text)
                .collect();
            let said: Vec<&str> = said.iter().map(String::as_str).collect();
            for post in deskpost::due(p, &said) {
                if self.desk_watch.fell_back(&m.task_id, &post.tag(p)) {
                    continue;
                }
                let (text, files) = if post == Post::ChecksFailed {
                    let got = match self.desk_lister.as_ref() {
                        Some(l) => l.failed_checks(&repo, p.number),
                        None => Ok(Vec::new()),
                    };
                    // The log is what lets the author fix it from the post
                    // alone: a fetch that fell short waits a poll or two.
                    let whole = got.as_ref().is_ok_and(|cs| {
                        !cs.is_empty()
                            && cs.iter().take(deskpost::MAX_LOGS).all(|c| {
                                deskpost::actions_job(&c.url).is_none() || !c.log.trim().is_empty()
                            })
                    });
                    if whole {
                        self.desk_watch.fetched(repo_key, p.number, &p.head_ref_oid);
                    } else if self
                        .desk_watch
                        .fetch_again(repo_key, p.number, &p.head_ref_oid)
                    {
                        if let Err(e) = &got {
                            logf(format_args!("desk: PR #{} checks: {e}", p.number));
                        }
                        continue;
                    }
                    let checks = got.unwrap_or_default();
                    let files = self.desk_logs(p, &checks);
                    (deskpost::failed_text(p, &checks, !files.is_empty()), files)
                } else {
                    (deskpost::text(post, p), Vec::new())
                };
                if !self.desk_post(&co, &m.task_id, p.number, &text, post.wakes(), files) {
                    self.desk_watch.fall_back(&m.task_id, &post.tag(p));
                }
            }
        }
    }

    /// Puts one of Irori's posts on card `id` in `co`, and wakes its
    /// holders when `wake`. When the card's board refuses Irori's comment,
    /// its holders get the same text as an inbox notice instead, so the
    /// author still hears it. False when the comment didn't go up.
    fn desk_post(
        &self,
        co: &Arc<Coord>,
        id: &str,
        pr: i64,
        text: &str,
        wake: bool,
        files: Vec<coord::FileRef>,
    ) -> bool {
        let from = inbox::system_sender();
        match co.add_comment_with(crate::deskpost::IRORI, id, text, wake, files.clone()) {
            Ok((_, n)) => {
                // Opened and pushed are the thread's record: nobody is told.
                if wake && let Some(e) = notice::notice_err(&self.route_notice(&from, &n)) {
                    logf(format_args!("desk: PR #{pr} post on {id}: {e}"));
                }
                true
            }
            Err(e) => {
                logf(format_args!("desk: PR #{pr} post on {id}: {e}"));
                let Some(t) = co.get_task(id) else {
                    return false;
                };
                let to: Vec<String> = t
                    .holders
                    .iter()
                    .chain(&t.co_holders)
                    .filter(|h| !h.starts_with("user:") && *h != crate::deskpost::IRORI)
                    .cloned()
                    .collect();
                if to.is_empty() {
                    return false;
                }
                let n = Notice {
                    kind: coord::NOTICE_COMMENT.into(),
                    task_id: id.into(),
                    reference: format!("task:{id}"),
                    title: t.title.clone(),
                    to,
                    by: crate::deskpost::IRORI.into(),
                    text: format!(
                        "Irori, about PR #{pr} on {} ({}); its board didn't take this as a comment:\n{text}",
                        gocompat::quote::quote(&t.title),
                        t.short_ref()
                    ),
                    wake,
                    attachments: files,
                    ..Notice::default()
                };
                if let Some(e) = notice::notice_err(&self.route_notice(&from, &n)) {
                    logf(format_args!("desk: PR #{pr} notice for {id}: {e}"));
                }
                false
            }
        }
    }

    /// The failing checks' full logs as files for the post, best effort:
    /// a log that can't be stored is only left out.
    fn desk_logs(
        &self,
        p: &coord::DeskPr,
        checks: &[crate::deskpost::FailedCheck],
    ) -> Vec<coord::FileRef> {
        /// The most of one log kept: its end, where the failure is.
        const MAX_LOG: usize = 1 << 20;
        let head = crate::deskpost::short(&p.head_ref_oid);
        let mut out = Vec::new();
        for (i, c) in checks.iter().enumerate() {
            if c.log.trim().is_empty() {
                continue;
            }
            let log = c.log.as_bytes();
            let log = &log[log.len().saturating_sub(MAX_LOG)..];
            let tmp = std::env::temp_dir().join(format!(
                "irori-desk-{}-{}-{head}-{i}.log",
                self.device.id, p.number
            ));
            let r = std::fs::write(&tmp, log)
                .map_err(|e| e.to_string())
                .and_then(|_| {
                    self.files
                        .put_file(&tmp.to_string_lossy())
                        .map_err(|e| e.to_string())
                });
            let _ = std::fs::remove_file(&tmp);
            let mut r = match r {
                Ok(r) => r,
                Err(e) => {
                    logf(format_args!("desk: PR #{} log {i}: {e}", p.number));
                    continue;
                }
            };
            let name: String = c
                .name
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
                .take(60)
                .collect();
            r.name = blob::safe_name(&format!("pr-{}-{head}-{name}.log", p.number), &r.sha256);
            r.device = self.device.id.clone();
            if let Err(e) = self.mesh.node.offer(&r) {
                logf(format_args!("desk: PR #{} log {i}: {e}", p.number));
                continue;
            }
            out.push(coord::FileRef {
                sha256: r.sha256,
                name: r.name,
                size: r.size,
                mime: r.mime,
                device: r.device,
            });
        }
        out
    }

    /// Waits on shared boards' cards (card #574): the wait is in this log
    /// and the card in the board's, so its state is read here, as a branch
    /// wait's refs are. It sees the state at each sweep, so a move and
    /// back between two sweeps goes unseen. A card that's gone or withdrawn,
    /// or a board this person left, ends the wait.
    fn check_board_waits(&self) {
        for w in self.coord.board_waits() {
            let b = match self.board(&w.board) {
                Ok(b) if !self.board_leaving(&b) => b,
                _ => {
                    self.coord.board_wait_lost(&w.id);
                    continue;
                }
            };
            let co = b.coord();
            let Some(t) = co.get_task(&w.task_id) else {
                self.coord.board_wait_lost(&w.id);
                continue;
            };
            if let Some(was) = self.coord.board_wait_moved(&w.id, &t.state) {
                let what = co.wait_what(&t.id, &was, &t.state);
                self.coord.fire_wait(&w.id, &what);
            } else if t.withdrawn {
                self.coord.board_wait_lost(&w.id);
            }
        }
    }

    fn sweep_coord(self: &Arc<Self>) -> Result<(), String> {
        check_branch_waits(&self.coord);
        self.check_board_waits();
        self.sweep_where();
        self.sweep_builds();
        let (ns, err) = self.coord.sweep_notices_partial();
        self.sweep_spawns();
        let (asks, aerr) = self.coord.sweep_asks_partial(&self.ask_agents());
        // Card #372: an agent here hears when one it works with drops off.
        let me = self.device.id.clone();
        let away = self.coord.sweep_presence(&self.roster.agents(), &|k| {
            irori_identity::owner::device_of(k) == me
        });
        for n in ns.iter().chain(asks.iter()).chain(away.iter()) {
            let from = if n.person {
                self.user_sender()
            } else {
                inbox::system_sender()
            };
            if let Some(e) = notice::notice_err(&self.route_notice(&from, n)) {
                logf(format_args!("coord notice {} {}: {e}", n.kind, n.reference));
            }
        }
        let mut errs: Vec<String> = [err.err(), aerr.err()]
            .into_iter()
            .flatten()
            .map(|e| e.to_string())
            .collect();
        self.sweep_leaving();
        if let Err(e) = self.sweep_boards() {
            errs.push(e);
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs.join("\n"))
        }
    }

    /// Go `record`: status rows only when status, name, name source or
    /// permission mode changed, the mode riding on the status row.
    ///
    /// Card #433: other devices list a card only from its `agent.seen` row
    /// (an `agent.status` for a card they don't hold is dropped), so a key
    /// whose `agent.seen` never landed (the append failed on a full disk,
    /// or the Added change was dropped from a full queue) gets one on its
    /// next change instead of a status row.
    fn record(&self, mut next: impl FnMut() -> Option<presence::Change>) {
        let mut last: std::collections::HashMap<String, presence::AgentCard> =
            std::collections::HashMap::new();
        // Keys whose agent.seen row is in the log since they were last gone.
        let mut written: std::collections::HashSet<String> = std::collections::HashSet::new();
        while let Some(ch) = next() {
            let card = ch.card;
            let key = card.key.clone();
            let res = match ch.kind {
                presence::ChangeKind::Added => {
                    last.insert(key.clone(), card.clone());
                    self.names_kick.kick();
                    let r = self
                        .store
                        .append("irori", kinds::AGENT_SEEN, &self.row_card(card));
                    if r.is_ok() {
                        written.insert(key);
                    }
                    r.map(|_| ())
                }
                presence::ChangeKind::Updated => {
                    let prev = last.insert(key.clone(), card.clone());
                    if let Some(p) = &prev
                        && (p.name != card.name || p.name_source != card.name_source)
                    {
                        self.names_kick.kick();
                    }
                    if !written.contains(&key) {
                        let r = self
                            .store
                            .append("irori", kinds::AGENT_SEEN, &self.row_card(card));
                        if r.is_ok() {
                            written.insert(key);
                        }
                        r.map(|_| ())
                    } else {
                        if let Some(p) = &prev
                            && p.status == card.status
                            && p.name == card.name
                            && p.name_source == card.name_source
                            && p.permission_mode == card.permission_mode
                        {
                            continue;
                        }
                        self.store
                            .append(
                                "irori",
                                kinds::AGENT_STATUS,
                                &AgentStatus {
                                    key: &card.key,
                                    name: &card.name,
                                    name_source: &card.name_source,
                                    permission_mode: &card.permission_mode,
                                    status: &card.status,
                                },
                            )
                            .map(|_| ())
                    }
                }
                presence::ChangeKind::Removed => {
                    last.remove(&key);
                    written.remove(&key);
                    self.store
                        .append(
                            "irori",
                            kinds::AGENT_GONE,
                            &AgentGone {
                                key: &card.key,
                                reason: &card.status,
                            },
                        )
                        .map(|_| ())
                }
            };
            if let Err(e) = res {
                logf(format_args!("record {:?}: {e}", ch.kind));
            }
        }
    }

    /// The `agent.seen` card as it replicates: no transcript path or pid,
    /// no last assistant text, no device state or AI title, and the title
    /// scrubbed (a replicated row can't be scrubbed later).
    fn row_card(&self, mut card: presence::AgentCard) -> presence::AgentCard {
        card.last_text.clear();
        card.last_tool.clear();
        card.transcript.clear();
        card.pid = 0;
        card.device_state.clear();
        card.device_via.clear();
        card.ai_title.clear();
        card.title = self.scrubber.string(&card.title);
        card
    }
}

/// The address a device away from home dials to join, shown under the
/// pairing code (Go core `serverAddr`, home.go:104-123): this device's own
/// server's while it runs, else the first other server reachable from
/// outside.
fn server_addr(home: Option<&home::Server>, servers: &mesh::Servers) -> String {
    if let Some(h) = home {
        let st = h.status();
        if st.running && !st.address.is_empty() {
            return st.address;
        }
    }
    servers
        .list()
        .iter()
        .filter_map(|s| servers.row(&s.id))
        .map(|row| row.outside())
        .find(|a| !a.is_empty())
        .unwrap_or_default()
}

/// Card #87's branch waits: reads each waited branch's refs (a file read,
/// no git) and queues the wait to fire when one moved, with the new tip's
/// subject so the waiter needn't go and look. The coord sweep that
/// follows writes wait.fired and wakes the waiter.
impl Services {
    /// Where each card this device's agents hold is (card #137): for every
    /// open private card a local session holds, its latest working
    /// directory (capture's, else the session's start one) read as a git
    /// place, written when it changed. So a card says where its work is
    /// even if its agent never says, and whoever picks it up after a crash
    /// can find the branch.
    fn sweep_where(&self) {
        for c in self.presence.snapshot() {
            if c.status == "gone" {
                continue;
            }
            let dir = self
                .capture
                .session_dir(&c.key)
                .unwrap_or_else(|| c.cwd.clone());
            if dir.is_empty() {
                continue;
            }
            let held = self.coord.list_tasks(&coord::TaskFilter {
                holder: c.key.clone(),
                open: true,
                ..coord::TaskFilter::default()
            });
            for t in held {
                // One writer per card: its holder, not a co-holding step
                // owner, so two sessions never take turns rewriting it.
                if t.holder != c.key || t.shared || t.open_ask().is_some() {
                    continue;
                }
                // Only a git place is worth a row (see Where's line).
                let Some(g) = capture::gitlink::place(&dir) else {
                    continue;
                };
                let w = coord::Where {
                    who: c.key.clone(),
                    cwd: dir.clone(),
                    worktree: g.worktree,
                    branch: g.branch,
                    head: g.head,
                    pushed: g.pushed,
                    ..coord::Where::default()
                };
                if let Err(e) = self.coord.set_where(&t.id, w) {
                    logf(format_args!("where {}: {e}", t.id));
                }
            }
        }
    }
}

impl Services {
    /// Cards #427 and #565: when a card settles, and every
    /// [`crate::buildclean::EVERY`], removes each settled card's worktree
    /// here that nothing uses and holds no work, and clears the `target/`
    /// of one that holds work, on a thread of its own (a build folder
    /// takes a while to delete).
    fn sweep_builds(&self) {
        let Some(idle) = self.clear_builds else {
            return;
        };
        let last = self.builds_looked.lock().unwrap().listed;
        if last.is_some_and(|t| t.elapsed() < crate::buildclean::LOOK) {
            return;
        }
        if self.builds_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        self.note_held_trees();
        let (tasks, homes) = self.all_tasks();
        self.restore_probed(&tasks);
        let book = self.booked_trees();
        let mut trees = crate::buildclean::settled_worktrees(&tasks, &self.device.id, &book);
        self.forget_gone_trees(&trees);
        let names: Vec<String> = trees.iter().map(|t| t.worktree.clone()).collect();
        {
            // At once when a card has just settled, else every EVERY (a
            // build kept as recent may since have gone idle).
            let mut l = self.builds_looked.lock().unwrap();
            let now = Instant::now();
            l.listed = Some(now);
            if l.seen == names
                && l.cleared
                    .is_some_and(|t| t.elapsed() < crate::buildclean::EVERY)
            {
                self.builds_busy.store(false, Ordering::SeqCst);
                return;
            }
            l.cleared = Some(now);
            l.seen = names;
            // A tree kept for its work rests; one that isn't a worktree
            // any more isn't looked at again.
            trees.retain(|t| match l.rest.get(&t.worktree) {
                Some(Some(until)) => *until <= now,
                Some(None) => false,
                None => true,
            });
        }
        let pass = trees::Pass {
            trees,
            homes,
            live: self.session_dirs(),
            list: self
                .spawn
                .spawner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .and_then(|s| s.list.clone()),
            idle,
            looked: self.builds_looked.clone(),
            recheck: self.this.get().cloned().map(|w| -> trees::Recheck {
                Arc::new(move |tree: &str| match w.upgrade() {
                    Some(s) => s.tree_recheck(tree),
                    None => Err("Irori is stopping".into()),
                })
            }),
        };
        let busy = self.builds_busy.clone();
        let spawned = std::thread::Builder::new()
            .name("clear-builds".into())
            .spawn(move || {
                pass.run();
                busy.store(false, Ordering::SeqCst);
            });
        if let Err(e) = spawned {
            self.builds_busy.store(false, Ordering::SeqCst);
            logf(format_args!("clear builds: {e}"));
        }
    }
}

fn check_branch_waits(co: &coord::Coord) {
    for w in co.branch_waits() {
        let Some(now) = capture::gitlink::branch_tips(&w.root, &w.branch) else {
            continue;
        };
        let Some((name, old, new)) = capture::gitlink::moved_ref(&w.refs, &now) else {
            continue;
        };
        let short = |s: &str| s[..s.len().min(7)].to_string();
        let subject = commit_subject(&w.root, &new);
        let which = name
            .trim_start_matches("refs/")
            .replacen("heads/", "", 1)
            .replacen("remotes/", "", 1);
        let mut what = if old.is_empty() {
            format!("{which} appeared at {}", short(&new))
        } else {
            format!("{which} moved from {} to {}", short(&old), short(&new))
        };
        if !subject.is_empty() {
            what.push_str(&format!(" ({subject})"));
        }
        co.fire_wait(&w.id, &what);
    }
}

/// The subject line of `sha` in the repo at `root`, "" when git can't say.
fn commit_subject(root: &str, sha: &str) -> String {
    let args: Vec<String> = ["-C", root, "log", "-1", "--format=%s", sha]
        .iter()
        .map(|s| s.to_string())
        .collect();
    match capture::gitexec::run(&Cancel::new(), Duration::from_secs(5), &args, &[]) {
        Ok((out, _)) => String::from_utf8_lossy(&out)
            .trim()
            .chars()
            .take(120)
            .collect(),
        Err(_) => String::new(),
    }
}

/// Card #232: times the parts of [`Core::start`] and logs them as one line,
/// so a slow start (a phone with a fully synced log) shows where it went.
struct StartLaps {
    t0: Instant,
    last: Instant,
    parts: Vec<(&'static str, Duration)>,
}

impl StartLaps {
    fn new() -> Self {
        let now = Instant::now();
        StartLaps {
            t0: now,
            last: now,
            parts: Vec::new(),
        }
    }

    fn lap(&mut self, what: &'static str) {
        let now = Instant::now();
        self.parts.push((what, now - self.last));
        self.last = now;
    }

    /// "N ms (part a, part b, ...)".
    fn summary(&self) -> String {
        let parts: Vec<String> = self
            .parts
            .iter()
            .map(|(w, d)| format!("{w} {}", d.as_millis()))
            .collect();
        format!(
            "{} ms ({})",
            (self.last - self.t0).as_millis(),
            parts.join(", ")
        )
    }

    fn report(&self) {
        logf(format_args!("start: core ready in {}", self.summary()));
    }
}
