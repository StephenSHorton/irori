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
use std::sync::{Arc, Mutex, OnceLock};
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

pub mod boards;
mod boards_own;
mod bridge;
mod mirror;
pub use mirror::BoardReviewer;
pub mod invites;

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
    /// Builds the transport pairing runs on (M7, [`crate::devices`]);
    /// `None` is iroh on the device key, bound when a window opens.
    pub pair_net: Option<crate::devices::PairNetFn>,
    /// How long a pairing window stays open (tests); zero is
    /// `irori_pair::WINDOW` (Go `Options.PairWindow`).
    pub pair_window: Duration,
    /// Remove device (Go `Mesh().Revoke`); `None` is the mesh Node's.
    pub revoke: Option<crate::devices::RevokeFn>,
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
    closed: AtomicBool,
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
    members: Arc<Members>,
    names_kick: Arc<Kick>,
    colors: coord::devcolor::Picks,
    files: blob::Files,
    scrubber: Arc<scrub::Scrubber>,
    secrets: Mutex<Vec<String>>,
    token: String,
    reviver: Arc<Mutex<Option<ReviveFn>>>,
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
    fn agents(&self) -> Vec<presence::AgentCard> {
        self.0
            .roster
            .agents()
            .into_iter()
            .map(presence_card)
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
        device_state: c.device_state,
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
        device_state: c.device_state,
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

/// The `device.color` payload (a Go map: keys sorted).
#[derive(Serialize)]
struct DeviceColor<'a> {
    at_device: &'a str,
    color: &'a str,
    #[serde(rename = "deviceId")]
    device_id: &'a str,
}

impl Core {
    /// Brings everything up (Go `Start` without the net, pairing, boards,
    /// home server and API listener). The lockfile is written by
    /// [`Core::write_lockfile`] once the API listens.
    pub fn start(mut opts: Options) -> Result<Core, StartError> {
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
        let mut popts = presence::Options {
            claude_dir: opts.claude_dir.to_string_lossy().into_owned(),
            device_id: me.clone(),
            repo_key: Some(repo_key.clone()),
            names: Some(names.clone()),
            watch: Some(Arc::new(fs_watch)),
            ..Default::default()
        };
        if let Some(edit) = &opts.presence {
            edit(&mut popts);
        }
        let presence = Arc::new(presence::Watcher::new(popts));

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
            revoked: Some(revoked.clone()),
        })
        .map_err(|e| fail(&st, other("roster", e)))?;
        {
            let r = roster.clone();
            names.set_mesh(move || r.agents());
        }

        // The inbox's hooks reach coord and capture, which open after it.
        let coord_cell: Arc<OnceLock<Arc<Coord>>> = Arc::new(OnceLock::new());
        let capture_cell: Arc<OnceLock<capture::Capture>> = Arc::new(OnceLock::new());
        let iopts = inbox::Options {
            device_id: me.clone(),
            now: Some(now.clone()),
            pr_owners: Some({
                let c = capture_cell.clone();
                Arc::new(move |k: &str| c.get().map(|c| c.pr_owners(k)).unwrap_or_default())
            }),
            is_owner: Some(is_owner.clone()),
            revoked: Some(revoked.clone()),
            on_recipient_gone: Some({
                let c = coord_cell.clone();
                Arc::new(move |m: &inbox::Message, to: &str| match c.get() {
                    Some(co) => notice::record_gone(co, m, to),
                    None => Err(inbox::Error::sentinel(inbox::ErrorKind::NoCard)),
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
                    c.get()
                        .map(|co| notice::stale_notice(co, d))
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

        let co = Arc::new(
            Coord::new(
                st.clone(),
                Some(Arc::new(LocalAgents(local.clone()))),
                coord::Options {
                    now: Some(now.clone()),
                    is_owner: Some(is_owner.clone()),
                    revoked: Some(revoked.clone()),
                    ..Default::default()
                },
            )
            .map_err(|e| fail(&st, other("coord", e)))?,
        );
        let _ = coord_cell.set(co.clone());

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

        let files =
            blob::Files::open(dir.join("files")).map_err(|e| fail(&st, other("attachments", e)))?;
        let colors = coord::devcolor::Picks::new();
        let colors_sub = Arc::new(st.subscribe());
        load_colors(&st, &colors);
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

        let home = (!opts.no_home).then(|| {
            let mut hc = home::Config::new(dir.clone(), ident.clone(), chain.clone(), st.clone());
            let node = mesh.node.clone();
            let pr = pairing.pairer.clone();
            // Go core.go:290: another device, or an open pairing window;
            // stage 2 (B1): or a shared board with another person on it,
            // whose members this server relays for.
            let held = board_set.clone();
            hc.wanted = Some(Arc::new(move || {
                pr.status().open() || node.has_peer() || held.shared_with_others()
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
            members,
            names_kick: Kick::new(),
            colors,
            files,
            scrubber,
            secrets: Mutex::new(Vec::new()),
            token: token.clone(),
            reviver,
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
            invites: Default::default(),
        });
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
        // A pairing window starts the home server at once (Go Notify).
        if let Some(h) = s.home.clone() {
            s.pairing
                .pairer
                .set_notify(Some(Arc::new(move || h.kick())));
        }
        s.add_secret(&token);

        let tools = tools::Set::new(Arc::new(ToolsView(s.clone())), Some(s.now.clone()));
        let core = Core {
            s: s.clone(),
            tools,
            stop,
            stop_async: CancellationToken::new(),
            threads: Mutex::new(Vec::new()),
            watch: Mutex::new(None),
            closed: AtomicBool::new(false),
        };
        // Other devices' queued deliveries get this device's own hold
        // decision before anything serves an attach stream that could wake
        // them.
        s.inbox.catch_up();
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
                    let r = s.roster.clone();
                    Arc::new(move |key: &str| {
                        r.agents()
                            .into_iter()
                            .find(|a| a.key == key)
                            .map(|a| a.name)
                            .unwrap_or_default()
                    })
                }),
                room: Some(s.coord.clone()),
            },
        );
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
        // followOwnBoards).
        s.open_boards();
        {
            let sub = Arc::new(s.store.subscribe());
            let reg = self.on_stop(&sub, |s| s.cancel());
            let s = s.clone();
            self.spawn("boards", move || {
                let _reg = reg;
                s.follow_own_boards(&sub);
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
                        load_colors(&s.store, &s.colors);
                    } else if ev.kind == coord::devcolor::KIND {
                        s.colors.apply(&ev);
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
            self.spawn("names-roster", move || {
                let _reg = reg;
                while rq.recv().is_some() {
                    kick.kick();
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
                    if ct.take()
                        && let Err(e) = s.sweep_coord()
                    {
                        logf(format_args!("coord sweep: {e}"));
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
                    s.hub.publish(CoreEvent::Inbox(ch));
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
                while let Some(ch) = q.recv() {
                    s2.hub.publish(CoreEvent::Board(ch));
                }
            });
        }
        {
            let sub = Arc::new(s.capture.subscribe());
            let reg = self.on_stop(&sub, |s| s.cancel());
            let s = s.clone();
            self.spawn("feed-capture", move || {
                let _reg = reg;
                while let Some(ch) = sub.recv() {
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

    /// Gives capture's scrubber one more exact value to redact, kept for the
    /// Core's life (the lockfile token, member secrets).
    pub fn add_secret(&self, v: &str) {
        self.s.add_secret(v);
    }

    // ---- the services ----------------------------------------------------

    pub fn data_dir(&self) -> &Path {
        &self.s.data_dir
    }
    pub fn version(&self) -> &str {
        &self.s.version
    }
    pub fn headless(&self) -> bool {
        self.s.headless
    }
    /// This device (`device.json`).
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
    /// The home server itself, for the net and mesh wiring.
    pub fn home(&self) -> Option<&home::Server> {
        self.s.home.as_ref()
    }
    pub fn person(&self) -> irori_identity::Person {
        self.s.ident.current()
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
    /// sent or received, or on any Board comment) and writes it to `to`, or
    /// by default to `<data>/received/<reader>/<name>`. Returns the local
    /// path and the device it came from ("" when it was already here). A
    /// `to` inside the data dir is refused, and an existing file is never
    /// overwritten. [`crate::attachments::NO_ATTACHMENT`] when no message
    /// or card the reader may read carries it. Blocks on the mesh's runtime
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

    /// An agent key's card name in the mesh roster, or `""`.
    pub fn agent_name(&self, key: &str) -> String {
        self.mesh_agents()
            .into_iter()
            .find(|a| a.key == key)
            .map(|a| a.name)
            .unwrap_or_default()
    }

    /// Gives the Core the desktop app's Revive, which a revive ask answered
    /// from any device runs here. A headless Core has none.
    pub fn set_reviver(&self, f: Option<ReviveFn>) {
        *self.s.reviver.lock().unwrap_or_else(|e| e.into_inner()) = f;
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
        if let Some(m) = self.s.mesh.board(board_id) {
            m.recheck();
        }
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

fn load_colors(st: &Store, colors: &coord::devcolor::Picks) {
    let mut f = Filter {
        kinds: vec![coord::devcolor::KIND.into()],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    };
    loop {
        let Ok(evs) = st.list(&f) else { return };
        for ev in &evs {
            colors.apply(ev);
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
    fn user_sender(&self) -> Sender {
        Sender::new(&format!("user:{}", self.device.id), "you", "user")
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
        let r = match self.inbox.attachment(reader, &sha) {
            Some(f) => blob::FileRef {
                sha256: f.sha256,
                name: f.name,
                size: f.size,
                mime: f.mime,
                device: f.device,
            },
            // The private board's comments; shared boards' (#49) are not
            // looked up yet (crate::attachments).
            None => match self.coord.comment_attachment(&sha) {
                Some(f) => blob::FileRef {
                    sha256: f.sha256,
                    name: f.name,
                    size: f.size,
                    mime: f.mime,
                    device: f.device,
                },
                None => return Err(at::NO_ATTACHMENT.into()),
            },
        };
        let name = blob::safe_name(&r.name, &sha);
        let to = trim_space(to);
        let dst = at::attachment_dst(&self.data_dir, reader, &name, to, || {
            self.local_agents()
                .into_iter()
                .rfind(|a| a.key == reader)
                .map(|a| a.cwd)
                .unwrap_or_default()
        })?;
        let from = crate::meshview::fetch_file_blocking(
            &self.mesh.node,
            &self.pairing.handle,
            &self.device.id,
            &self.chain,
            &r,
        )
        .map_err(|e| match e {
            FileFetchError::Offline(on) => at::offline_text(&on),
            FileFetchError::TimedOut => at::TIMED_OUT.into(),
            FileFetchError::Other(s) => s,
        })?;
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

    fn sweep_coord(&self) -> Result<(), String> {
        let (ns, err) = self.coord.sweep_notices_partial();
        let (asks, aerr) = self.coord.sweep_asks_partial(&self.ask_agents());
        for n in ns.iter().chain(asks.iter()) {
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
    fn record(&self, mut next: impl FnMut() -> Option<presence::Change>) {
        let mut last: std::collections::HashMap<String, presence::AgentCard> =
            std::collections::HashMap::new();
        while let Some(ch) = next() {
            let card = ch.card;
            let res = match ch.kind {
                presence::ChangeKind::Added => {
                    last.insert(card.key.clone(), card.clone());
                    self.names_kick.kick();
                    self.store
                        .append("irori", kinds::AGENT_SEEN, &self.row_card(card))
                }
                presence::ChangeKind::Updated => {
                    let prev = last.insert(card.key.clone(), card.clone());
                    if let Some(p) = &prev
                        && p.status == card.status
                        && p.name == card.name
                        && p.name_source == card.name_source
                        && p.permission_mode == card.permission_mode
                    {
                        continue;
                    }
                    if let Some(p) = &prev
                        && (p.name != card.name || p.name_source != card.name_source)
                    {
                        self.names_kick.kick();
                    }
                    self.store.append(
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
                }
                presence::ChangeKind::Removed => {
                    last.remove(&card.key);
                    self.store.append(
                        "irori",
                        kinds::AGENT_GONE,
                        &AgentGone {
                            key: &card.key,
                            reason: &card.status,
                        },
                    )
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
        card.transcript.clear();
        card.pid = 0;
        card.device_state.clear();
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
