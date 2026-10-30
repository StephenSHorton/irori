//! `irori-core`: the Core (I:internal/core, I:internal/discovery;
//! surface.md §1, §2.1-2.2, §2.13; architecture.md §2, §8 M4).
//!
//! One Core per data dir owns the signed log and every service built on
//! it: identity and the Chain, presence, the mesh roster, the inbox, the
//! Board (coord), capture, the asks, friendly names, device colours, the
//! attachments; the decisions live in capture's fold. [`Core::start`] opens them in Go's order
//! and starts their loops; [`Core::close`] stops the loops (5 s at most, as
//! Go waits), removes the lockfile if it is still ours and closes the log.
//!
//! | Module | Go | What |
//! |---|---|---|
//! | [`Core`] | `core.go`, `colors.go` | start/stop, the loops, presence → rows, sweeps |
//! | [`lockfile`] | `discovery` | `irori.json` |
//! | [`diag`] | `internal/diag` | crash.log, the rotated irori.log, the start line |
//! | [`members`] | `members.go` | bound Grok members and their cards |
//! | [`reset`] | `reset.go` | Reset and its marker |
//! | [`namer`] | `names.go` | friendly session names, clash settling |
//! | [`notice`] | `notice.go`, `board.go`, tools `notice.go`/`cardthread.go` | Board notices through the inbox |
//! | [`events`] | the services' Subscribe, root `*_service.go` pumps | the change feed |
//! | [`timers`] | coord `armExpiry`, Go tickers | the expiry timer |
//! | [`meshview`] | `home.go`, `attachview.go` | the running mesh as Devices and attachments see it |
//! | [`attachments`] | `attachments.go` | agents' attach and attachment_get (the tools' attacher) |
//!
//! Loops run on plain threads (every service API is synchronous); presence
//! runs on the tokio runtime given in [`Options::runtime`], or on a
//! current-thread runtime of its own. No loop holds a service lock across
//! a wait.
//!
//! Not here yet (M4 neighbours and later milestones; see the wiring notes
//! on [`Core`]): the loopback API listener (the API crate calls
//! [`Core::write_lockfile`] once it listens) and shared boards. The mesh
//! Node runs on the Core's iroh endpoint (`meshnode`, read through
//! [`meshview`]), pairing is [`devices`]. The home server ([`Core::home_server`],
//! `irori-home`) is built unless [`Options::no_home`], and binds nothing
//! until it is switched on and there is another device.

pub mod attachments;
pub mod devices;
pub mod diag;
pub mod events;
pub mod lockfile;
pub mod members;
mod meshnode;
pub mod meshview;
pub mod namer;
pub mod notice;
pub mod offline;
pub mod reset;
mod service;
pub mod timers;

pub use crate::members::Member;
pub use crate::meshnode::BoardParts;
pub use crate::service::boards::{
    BOARDS_DIR, BoardIdentity, BoardInfo, BoardSummary, Error as BoardError, SharePreview,
    SharedCopy, new_board_id, valid_board_id,
};
pub use crate::service::invites::{InviteStatus, InviteWatchFn};
pub use crate::service::{Core, HomeEdit, NO_HOME_SERVER, NowFn, Options, ReviveFn, StartError};
pub use capture::DecisionView;
pub use events::{CoreEvent, Feed, Hub, Topic};

/// Go `core.ErrAlreadyRunning`'s text.
pub const ALREADY_RUNNING: &str = "irori is already running on this device";

/// coord's sweeper starts a vanished holder's grace clock only when it
/// runs, so it runs more often than the inbox's.
pub const COORD_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
/// How long [`Core::close`] waits for the loops.
pub const CLOSE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

type LogFn = Box<dyn Fn(&str) + Send + Sync>;

static LOGGER: std::sync::RwLock<Option<LogFn>> = std::sync::RwLock::new(None);

/// Routes the Core's log lines (Go `log.Printf`; the app sends them to
/// `irori.log`). Unset, they go to stderr. Lines never carry secrets.
pub fn set_logger(f: impl Fn(&str) + Send + Sync + 'static) {
    *LOGGER.write().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
}

pub(crate) fn logf(args: std::fmt::Arguments<'_>) {
    log(&args.to_string());
}

/// Writes one line to the Core's log (see [`set_logger`]); the API and
/// other crates above the Core log through it. Never pass a secret.
/// Unrouted, it goes to stderr with Go `log`'s `2006/01/02 15:04:05 `
/// prefix; a logger gets the bare line.
pub fn log(line: &str) {
    let line = line.to_string();
    match LOGGER.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        Some(f) => f(&line),
        None => eprintln!("{}{line}", diag::log_stamp()),
    }
}
