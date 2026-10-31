//! Tells Irori when files under a directory change, a port of
//! I:internal/fswatch (capture.md §4, §7 trap 12).
//!
//! On Windows one `ReadDirectoryChangesW` handle watches a whole tree, armed
//! before [`watch`] returns so a write right after it is seen. On macOS one
//! FSEvents stream does, started before [`watch`] returns. Elsewhere
//! [`watch`] returns [`Error::Unsupported`] and callers keep their interval
//! scan.
//!
//! Notifications are a hint, not a record: the OS can drop them (a full
//! buffer is reported as an Overflow event), and a reader that falls behind
//! loses events but always gets one Overflow for them, which callers treat
//! as "rescan everything now". Events for one path inside the debounce
//! window are merged.

mod coalesce;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::cancel::Cancel;

/// The default debounce (Go: 50 ms).
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(50);
/// The raw and the delivered queue's capacity (Go: 1024 each).
pub const QUEUE: usize = 1024;

/// One change. `path` is absolute; an Overflow means events were lost and
/// `path` is the watched root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub path: String,
    pub overflow: bool,
}

/// Why [`watch`] failed.
#[derive(Debug)]
pub enum Error {
    /// Go `ErrUnsupported`: no watcher on this OS.
    Unsupported,
    /// The directory could not be opened or watched.
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unsupported => f.write_str("fswatch: not supported on this OS"),
            Error::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

/// A wait that timed out with nothing to receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeout;

impl std::fmt::Display for Timeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("timed out")
    }
}

impl std::error::Error for Timeout {}

pub(crate) struct Shared {
    rx: Mutex<Receiver<Event>>,
    stop: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

/// Delivers events until closed or its token is canceled. Clones share one
/// watcher; [`Watcher::close`] from any of them stops it.
#[derive(Clone)]
pub struct Watcher {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Watcher")
    }
}

impl Watcher {
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        expect(dead_code, reason = "only the Windows and macOS watchers build one")
    )]
    pub(crate) fn new(rx: Receiver<Event>, stop: Box<dyn FnOnce() + Send>) -> Watcher {
        Watcher {
            shared: Arc::new(Shared {
                rx: Mutex::new(rx),
                stop: Mutex::new(Some(stop)),
            }),
        }
    }

    /// The next event; `None` once the watcher stopped and its queue is
    /// drained (Go's closed channel).
    pub fn recv(&self) -> Option<Event> {
        self.shared
            .rx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .recv()
            .ok()
    }

    /// [`Watcher::recv`] with a timeout.
    pub fn recv_timeout(&self, d: Duration) -> Result<Option<Event>, Timeout> {
        match self
            .shared
            .rx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .recv_timeout(d)
        {
            Ok(e) => Ok(Some(e)),
            Err(RecvTimeoutError::Disconnected) => Ok(None),
            Err(RecvTimeoutError::Timeout) => Err(Timeout),
        }
    }

    /// An event if one is queued: `Ok(None)` when stopped, `Err` when none
    /// is waiting.
    pub fn try_recv(&self) -> Result<Option<Event>, Timeout> {
        match self
            .shared
            .rx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .try_recv()
        {
            Ok(e) => Ok(Some(e)),
            Err(TryRecvError::Disconnected) => Ok(None),
            Err(TryRecvError::Empty) => Err(Timeout),
        }
    }

    /// Stops the watcher and waits for its threads. Safe to call twice.
    pub fn close(&self) {
        let stop = self
            .shared
            .stop
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(stop) = stop {
            stop();
        }
    }
}

/// Starts watching `root` (a directory), with its whole subtree when
/// `recursive`. Events for one path within `debounce` are merged (zero
/// means [`DEFAULT_DEBOUNCE`]). The watcher stops when `cancel` is canceled
/// or it is closed.
pub fn watch(
    cancel: &Cancel,
    root: &str,
    recursive: bool,
    debounce: Duration,
) -> Result<Watcher, Error> {
    let debounce = if debounce.is_zero() {
        DEFAULT_DEBOUNCE
    } else {
        debounce
    };
    imp(cancel, root, recursive, debounce)
}

#[cfg(windows)]
fn imp(cancel: &Cancel, root: &str, recursive: bool, debounce: Duration) -> Result<Watcher, Error> {
    windows::watch(cancel, root, recursive, debounce)
}

#[cfg(target_os = "macos")]
fn imp(cancel: &Cancel, root: &str, recursive: bool, debounce: Duration) -> Result<Watcher, Error> {
    macos::watch(cancel, root, recursive, debounce)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn imp(_: &Cancel, _: &str, _: bool, _: Duration) -> Result<Watcher, Error> {
    Err(Error::Unsupported)
}
