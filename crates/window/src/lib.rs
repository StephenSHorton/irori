//! `irori-window`: the window's backend (architecture.md 4.3, 8 M6; D10).
//!
//! Each Go service the frontend calls (I:*_service.go; surface.md 2.9) is
//! a set of methods on [`Window`] over the Core, and each of those is one
//! `#[tauri::command] #[specta::specta]` function in [`commands`], named
//! `<service>_<method>`. [`builder`] collects them for the app's
//! invoke handler and for `cargo xtask bindings`, which writes
//! `frontend/src/gen/bindings.ts`. [`events`] runs Go's window pumps
//! (surface.md 2.10) and [`attachments`] serves the `irori` URI scheme.
//!
//! The commands live here, not in `app`, so the bindings export needs no
//! `frontend/dist` (tauri-build wants it for the app crate) and this
//! crate's tests run every service on a scratch Core with no webview.
//!
//! | Module | Go |
//! |---|---|
//! | [`roster`] | roster_service.go |
//! | [`inbox`] | inbox_service.go |
//! | [`cards`], [`board`] | cards_service.go, board_service.go |
//! | [`boards`] | boards_service.go, board_route.go |
//! | [`devices`] | devices_service.go |
//! | [`settings`] | settings_service.go, internal/notify/settings.go |
//! | [`attachments`] | attachments_service.go, internal/core/attachview.go |
//! | [`update`] | internal/update/service.go (the window's face) |
//! | [`ts`] | the TS shapes of the area types sent as they are |
//!
//! Wiring notes (what a merge plugs in, all in [`seams`]):
//! - Mesh and pairing: [`devices::CoreMesh`] and [`devices::CorePairing`]
//!   are the seams on the Core; [`events::start`] forwards the Pairer's
//!   changes as `devices:pairing`. `CoreMesh` reads the Core's running
//!   mesh (`irori_core::meshview`): the device rows' live state and
//!   warnings, `elsewhere`, `lan` and `remote` in the home server view, and
//!   `fetch_file`/`offer`. `roster:changed` fires on this device's presence
//!   only, as Go's (`Core.Changes` is `presence.Subscribe`).
//! - Notifier: [`settings::NotifierSeam`] over the notify crate (its
//!   settings.json writer); the default never shows a card, the app
//!   passes its own.
//! - Launch at login and the updater: the app fills [`seams::LoginItem`]
//!   and [`seams::Updater`]; [`settings::NoLoginItem`] and
//!   [`update::NoUpdater`] are the defaults.
//! - The tray takes its counts from [`Window::on_inbox_counts`] and
//!   [`Window::on_decide_count`], fed by the pumps (Go `onCounts`,
//!   `board.pump(.., tray.SetDecide)`).

pub mod attachments;
pub mod board;
pub mod boards;
pub mod cards;
pub mod commands;
pub mod devices;
pub mod events;
pub mod inbox;
pub mod roster;
pub mod seams;
pub mod settings;
pub mod ts;
pub mod update;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use irori_core::{Core, CoreEvent};

pub use crate::seams::Seams;

/// Trims Go's `strings.TrimSpace` set.
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(gocompat::text::is_space)
}

/// Go `filepath.Ext`: from the last dot of the last element.
pub(crate) fn ext(p: &str) -> &str {
    let seps: &[char] = if cfg!(windows) {
        &['/', '\\', ':']
    } else {
        &['/']
    };
    let base = p.rfind(seps).map(|i| &p[i + 1..]).unwrap_or(p);
    base.rfind('.').map(|i| &base[i..]).unwrap_or("")
}

/// The area crates each keep a copy of Go's `blob.FileRef` (their merge
/// notes say so); these move one to another.
pub(crate) fn blob_ref(r: &coord::FileRef) -> blob::FileRef {
    blob::FileRef {
        sha256: r.sha256.clone(),
        name: r.name.clone(),
        size: r.size,
        mime: r.mime.clone(),
        device: r.device.clone(),
    }
}

pub(crate) fn coord_ref(r: &blob::FileRef) -> coord::FileRef {
    coord::FileRef {
        sha256: r.sha256.clone(),
        name: r.name.clone(),
        size: r.size,
        mime: r.mime.clone(),
        device: r.device.clone(),
    }
}

pub(crate) fn inbox_blob_ref(r: &::inbox::FileRef) -> blob::FileRef {
    blob::FileRef {
        sha256: r.sha256.clone(),
        name: r.name.clone(),
        size: r.size,
        mime: r.mime.clone(),
        device: r.device.clone(),
    }
}

/// What the window's services need beside the Core.
pub struct Config {
    /// Go `paths.ClaudeDir` (Revive looks for transcripts there).
    pub claude_dir: PathBuf,
    /// The folder Save writes to.
    pub downloads: Box<dyn Fn() -> Result<PathBuf, String> + Send + Sync>,
}

type EmitFn = Box<dyn Fn(&str, serde_json::Value) + Send + Sync>;
type OpenFn = Box<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;
/// Takes the inbox counts after every change (the tray).
pub type CountsFn = Arc<dyn Fn(inbox::InboxCounts) + Send + Sync>;
/// Takes the Needs-a-decision count after every board change (the tray).
pub type DecideFn = Arc<dyn Fn(usize) + Send + Sync>;

/// Every window service over one Core (the Go root services together).
pub struct Window {
    pub core: Arc<Core>,
    pub seams: Seams,
    cfg: Config,
    cards: Mutex<roster::CardIndex>,
    revives: Mutex<roster::ReviveGuard>,
    inbox_snap: Mutex<Option<inbox::Snapshot>>,
    inbox_kick: events::Kick,
    on_counts: Mutex<Vec<CountsFn>>,
    on_decide: Mutex<Vec<DecideFn>>,
    emitter: OnceLock<EmitFn>,
    open_file: OnceLock<OpenFn>,
    on_reset: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

impl Window {
    pub fn new(core: Arc<Core>, seams: Seams, cfg: Config) -> Window {
        Window {
            core,
            seams,
            cfg,
            cards: Mutex::default(),
            revives: Mutex::default(),
            inbox_snap: Mutex::default(),
            inbox_kick: events::Kick::default(),
            on_counts: Mutex::default(),
            on_decide: Mutex::default(),
            emitter: OnceLock::new(),
            open_file: OnceLock::new(),
            on_reset: OnceLock::new(),
        }
    }

    /// The defaults this build has for every seam.
    pub fn default_seams(core: &Arc<Core>) -> Seams {
        Seams {
            mesh: Arc::new(devices::CoreMesh::new(core.clone())),
            pairing: Arc::new(devices::CorePairing::new(core.clone())),
            notifier: Arc::new(settings::NotifierSeam::headless(core.data_dir())),
            login: Arc::new(settings::NoLoginItem),
            updater: Arc::new(update::NoUpdater),
        }
    }

    /// Where events go (the app's `Emitter::emit`); set once at setup.
    pub fn set_emitter(&self, f: impl Fn(&str, serde_json::Value) + Send + Sync + 'static) {
        let _ = self.emitter.set(Box::new(f));
    }

    /// The OS opener for Open (the opener plugin); set once at setup.
    pub fn set_open_file(&self, f: impl Fn(&Path) -> Result<(), String> + Send + Sync + 'static) {
        let _ = self.open_file.set(Box::new(f));
    }

    /// What ResetDevice runs after marking the Reset: the app quits.
    pub fn set_on_reset(&self, f: impl Fn() + Send + Sync + 'static) {
        let _ = self.on_reset.set(Box::new(f));
    }

    /// Emits a window event (dropped before setup).
    pub fn emit<T: serde::Serialize>(&self, name: &str, payload: &T) {
        if let (Some(f), Ok(v)) = (self.emitter.get(), serde_json::to_value(payload)) {
            f(name, v);
        }
    }

    /// The version the window shows (RosterService.Version).
    pub fn version(&self) -> String {
        self.core.version().to_string()
    }

    /// Go `InboxService.onCounts`: `f` gets the counts now and then from
    /// the inbox pump's own scan after every change, so it never reads a
    /// snapshot the pump hasn't refreshed yet.
    pub fn on_inbox_counts(&self, f: CountsFn) {
        f(self.inbox_counts());
        self.on_counts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(f);
    }

    /// The decide half of Go's `board.pump(ctx, app, tray.SetDecide)`: `f`
    /// gets the count now and after every board change.
    pub fn on_decide_count(&self, f: DecideFn) {
        f(self.decide_count());
        self.on_decide
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(f);
    }

    pub(crate) fn counts_changed(&self, c: inbox::InboxCounts) {
        let fns = self
            .on_counts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for f in fns {
            f(c);
        }
    }

    pub(crate) fn board_changed(&self) {
        let fns = self
            .on_decide
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if fns.is_empty() {
            return;
        }
        let n = self.decide_count();
        for f in fns {
            f(n);
        }
    }

    /// Hands the inbox pump a nudge (see [`events::Kick`]).
    pub(crate) fn kick_inbox(&self, q: &Arc<gocompat::sub::Queue<CoreEvent>>) {
        self.inbox_kick.set(q.clone());
    }
}

/// Every command, for the app's invoke handler and the bindings export.
pub fn builder() -> tauri_specta::Builder<tauri::Wry> {
    commands::builder()
}

/// Writes the TypeScript bindings (`cargo xtask bindings`).
pub fn export_bindings(path: &Path) -> Result<(), String> {
    builder()
        .export(bindings_config(), path)
        .map_err(|e| e.to_string())
}

/// The bindings as text, for `cargo xtask bindings --check`.
pub fn bindings_text() -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let p = dir.path().join("bindings.ts");
    export_bindings(&p)?;
    std::fs::read_to_string(&p).map_err(|e| e.to_string())
}

fn bindings_config() -> specta_typescript::Typescript {
    specta_typescript::Typescript::default()
        .header("// Generated by `cargo xtask bindings` from crates/window. DO NOT EDIT.\n/* eslint-disable */")
}
