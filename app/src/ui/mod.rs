//! The Rune window (card #674): one app [`State`], the [`Route`] it shows,
//! the event bus from crates/window's emitter, and the root view that puts
//! the sidebar, the page, the card page and the dialogs together.
//!
//! The UI is split into packages, one directory each under `app/src/`
//! (home, board, card, canvas, talk, call, spaces, notes, settings,
//! profile, inbox, palette, desk, term, status), plus the shared pieces
//! (`text` for rich text and the composer, `widgets` and `logic` for card
//! and agent bits). Each package owns:
//! - `pub struct Model` (its part of [`State`], a field named after it),
//! - `pub fn view(s: &State, f: &mut Frame) -> El` for its page or panel,
//! - `pub fn on_event(s: &mut State, ev: &Event)` for live updates,
//! - `pub fn scene(s: &mut State)`: the headless scene that shows it.
//!
//! Handlers are closures on `&mut State`, so a package changes its own
//! `s.<pkg>` and may call [`State::go`], [`State::open_card`] or
//! [`State::toast`]. Logic stays in crates/window; call it through
//! [`State::win`] (fast reads) or [`State::spawn`] (anything slow).

pub mod data;
pub mod icons;
pub mod keys;
pub mod nav;
pub mod prefs;
pub mod sidebar;
pub mod theme;

use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use irori_window::Window;
use rune::{AppHandle, Cx, El, col, el, row, text};
use rune_kit::{Kit, Toast, ToastRegion};

pub use nav::{Dialog, Route};

/// One event from crates/window's emitter (`Window::emit`), with its
/// payload as JSON: `ev.parse::<T>()` reads it as the backend's type.
#[derive(Clone, Debug)]
pub struct Event {
    pub name: String,
    pub payload: serde_json::Value,
}

impl Event {
    pub fn is(&self, name: &str) -> bool {
        self.name == name
    }
    pub fn parse<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_value(self.payload.clone()).ok()
    }
}

/// What a view gets besides the state: Rune's context, the theme kit and
/// the overlay layer (popovers, menus and dialogs mount there, above the
/// page; `rune_kit::root` draws it).
pub struct Frame<'a> {
    pub cx: &'a mut Cx<State>,
    pub k: &'a Kit,
    pub layer: &'a mut Vec<El>,
    pub dark: bool,
}

/// A short message at the bottom of the window: an action's failure, or
/// what it did.
#[derive(Clone, Debug)]
pub struct Note {
    pub id: u64,
    pub title: String,
    pub body: String,
    pub error: bool,
    pub until: Instant,
}

#[derive(Default)]
pub struct State {
    /// The window backend. None only in a bare `State::default()`.
    pub w: Option<Arc<Window>>,
    /// Posts work back onto the UI thread ([`State::spawn`]).
    pub handle: Option<AppHandle<State>>,
    /// A headless render (`--snap`, `--script`): [`State::spawn`] runs
    /// inline so the frame shows its result.
    pub headless: bool,
    pub data_dir: PathBuf,
    /// Shared reads every page uses: cards, agents, spaces, devices, counts.
    pub data: data::Data,
    pub nav: nav::Nav,
    pub dialog: Option<Dialog>,
    pub prefs: prefs::Prefs,
    pub notes: Vec<Note>,
    pub next_note: u64,
    /// The window has focus (App::on_focus).
    pub focused: bool,
    /// The tray, kept alive with the state (shell.rs).
    pub tray: Option<Box<dyn Any>>,

    // The packages' own state.
    pub home: crate::home::Model,
    pub board: crate::board::Model,
    pub card: crate::card::Model,
    pub canvas: crate::canvas::Model,
    pub talk: crate::talk::Model,
    pub call: crate::call::Model,
    pub spaces: crate::spaces::Model,
    pub notes_page: crate::notes::Model,
    pub settings: crate::settings::Model,
    pub profile: crate::profile::Model,
    pub inbox: crate::inbox::Model,
    pub palette: crate::palette::Model,
    pub desk: crate::desk::Model,
    pub term: crate::term::Model,
    pub status: crate::status::Model,
    pub text: crate::text::Model,
    pub widgets: crate::widgets::Model,
}

impl State {
    pub fn new(w: Arc<Window>, data_dir: PathBuf, headless: bool) -> State {
        let prefs = prefs::Prefs::load(&data_dir);
        let mut s = State {
            w: Some(w),
            headless,
            data_dir,
            prefs,
            focused: true,
            ..State::default()
        };
        s.nav = nav::Nav::restore(&s.prefs);
        s.data.refresh_all(s.w.as_deref());
        let ev = Event {
            name: "app:start".into(),
            payload: serde_json::Value::Null,
        };
        dispatch(&mut s, &ev);
        s
    }

    /// The window backend. Panics only on a bare `State::default()`, which
    /// never reaches a view.
    pub fn win(&self) -> &Arc<Window> {
        self.w.as_ref().expect("the window backend")
    }

    /// Runs `work` off the UI thread, then `done` with its result on it.
    /// Headless, both run now, so a snapshot shows the result.
    pub fn spawn<T: Send + 'static>(
        &mut self,
        work: impl FnOnce(&Window) -> T + Send + 'static,
        done: impl FnOnce(&mut State, T) + Send + 'static,
    ) {
        let Some(w) = self.w.clone() else { return };
        match (&self.handle, self.headless) {
            (Some(h), false) => {
                let h = h.clone();
                std::thread::spawn(move || {
                    let r = work(&w);
                    h.update(move |s: &mut State, _| done(s, r));
                });
            }
            _ => {
                let r = work(&w);
                done(self, r);
            }
        }
    }

    /// Runs a backend call that answers Result<_, String> off the UI
    /// thread; a failure becomes an error note, a success runs `done`.
    pub fn act<T: Send + 'static>(
        &mut self,
        what: &'static str,
        work: impl FnOnce(&Window) -> Result<T, String> + Send + 'static,
        done: impl FnOnce(&mut State, T) + Send + 'static,
    ) {
        self.spawn(work, move |s, r| match r {
            Ok(v) => done(s, v),
            Err(e) => s.error(what, &e),
        });
    }

    /// Shows a page, remembering where it came from (Back).
    pub fn go(&mut self, r: Route) {
        self.nav.go(r);
        self.nav.save(&mut self.prefs);
        // The page's package reads what it shows (profile: your picture).
        let ev = Event { name: "app:route-changed".into(), payload: serde_json::Value::Null };
        dispatch(self, &ev);
    }

    pub fn back(&mut self) {
        self.nav.back();
    }

    /// Opens a card's page over the current page (`task:` optional).
    pub fn open_card(&mut self, id: &str) {
        let id = id.trim_start_matches("task:").to_string();
        self.nav.open_card(id.clone());
        let ev = Event {
            name: "app:card-opened".into(),
            payload: serde_json::Value::String(id),
        };
        dispatch(self, &ev);
    }

    pub fn close_card(&mut self) {
        self.nav.card = None;
    }

    /// Opens a dialog; its package hears `app:dialog-opened` (to load).
    pub fn open_dialog(&mut self, d: Dialog) {
        self.dialog = Some(d);
        // The dialog's package reads what it shows (settings: its pane).
        let ev = Event { name: "app:dialog-opened".into(), payload: serde_json::Value::Null };
        dispatch(self, &ev);
    }

    pub fn close_dialog(&mut self) {
        self.dialog = None;
    }

    /// A note at the bottom of the window for a few seconds.
    pub fn toast(&mut self, title: &str, body: &str) {
        self.push_note(title, body, false, Duration::from_secs(4));
    }

    /// An action failed: says what and why, a little longer.
    pub fn error(&mut self, what: &str, why: &str) {
        self.push_note(what, why, true, Duration::from_secs(8));
    }

    fn push_note(&mut self, title: &str, body: &str, error: bool, life: Duration) {
        self.next_note += 1;
        self.notes.push(Note {
            id: self.next_note,
            title: title.to_string(),
            body: body.to_string(),
            error,
            until: Instant::now() + life,
        });
        if self.notes.len() > 4 {
            self.notes.remove(0);
        }
    }

    /// A notification's Open (or the tray's): the window's page for its
    /// target (the React App's onOpen).
    pub fn open_target(&mut self, tab: &str, r: &str) {
        match tab {
            "wind-down" => self.open_dialog(Dialog::WindDown),
            "held" => self.open_dialog(Dialog::Held {
                from: (!r.is_empty()).then(|| r.to_string()),
            }),
            "talk" => self.go(Route::Talk((!r.is_empty()).then(|| r.to_string()))),
            "inbox" if r.is_empty() => self.go(Route::Talk(None)),
            "inbox" => self.open_dialog(Dialog::Message { id: r.to_string() }),
            _ if r.is_empty() => {}
            _ => {
                // task:<id>, or task:<id>/comment:<id>.
                let (card, comment) = match r.split_once('/') {
                    Some((c, rest)) => (c, Some(rest.trim_start_matches("comment:"))),
                    None => (r, None),
                };
                self.card.focus_comment = comment.map(str::to_string);
                self.open_card(card);
            }
        }
    }
}

/// Every package hears every event, after the shared data.
pub fn dispatch(s: &mut State, ev: &Event) {
    let w = s.w.clone();
    s.data.on_event(w.as_deref(), ev);
    crate::home::on_event(s, ev);
    crate::board::on_event(s, ev);
    crate::card::on_event(s, ev);
    crate::canvas::on_event(s, ev);
    crate::talk::on_event(s, ev);
    crate::call::on_event(s, ev);
    crate::spaces::on_event(s, ev);
    crate::notes::on_event(s, ev);
    crate::settings::on_event(s, ev);
    crate::profile::on_event(s, ev);
    crate::inbox::on_event(s, ev);
    crate::palette::on_event(s, ev);
    crate::desk::on_event(s, ev);
    crate::term::on_event(s, ev);
    crate::status::on_event(s, ev);
    crate::text::on_event(s, ev);
    crate::widgets::on_event(s, ev);
}

/// The window: sidebar, then the page (or the open card), dialogs and
/// notes above.
pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let dark = theme::is_dark(s, cx);
    let k = theme::kit(dark, &s.prefs);
    let mut layer = Vec::new();
    let mut f = Frame {
        cx,
        k: &k,
        layer: &mut layer,
        dark,
    };
    let page = if s.text.demo {
        crate::text::view(s, &mut f)
    } else if let Some(g) = crate::widgets::page(s, &mut f) {
        g
    } else if let Some(c) = crate::canvas::page(s, &mut f) {
        c
    } else if s.nav.card.is_some() {
        crate::card::view(s, &mut f)
    } else {
        match &s.nav.route {
            Route::Home => crate::home::view(s, &mut f),
            Route::Talk(_) => crate::talk::view(s, &mut f),
            Route::Notes => crate::notes::view(s, &mut f),
            Route::Desk(_) => crate::desk::view(s, &mut f),
            Route::Spaces => crate::spaces::view(s, &mut f),
            Route::Board { .. } => crate::spaces::board_page(s, &mut f),
            Route::Profile => crate::profile::view(s, &mut f),
            Route::Notifications => crate::inbox::view(s, &mut f),
        }
    };
    let main = col()
        .key("main")
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .fill(k.c.bg_base)
        .child(crate::status::banner(s, &mut f))
        .child(page.grow(1.0).min_h(0.0).w_full());
    let body = row()
        .key("body")
        .w_full()
        .h_full()
        .fill(k.c.page)
        .child(sidebar::view(s, &mut f))
        .child(main);
    if let Some(d) = &s.dialog {
        match d {
            Dialog::NewCard { .. } => crate::card::new_card_dialog(s, &mut f),
            Dialog::Held { .. } | Dialog::Message { .. } => crate::inbox::dialog(s, &mut f),
            Dialog::Palette => crate::palette::dialog(s, &mut f),
            Dialog::WindDown => crate::home::wind_down_dialog(s, &mut f),
            Dialog::Settings(_) => crate::settings::dialog(s, &mut f),
            Dialog::Share { .. } => crate::card::share_dialog(s, &mut f),
        }
    }
    crate::call::overlay(s, &mut f);
    crate::text::overlay(s, &mut f);
    crate::widgets::overlay(s, &mut f);
    crate::canvas::overlay(s, &mut f);
    notes(s, &mut f);
    rune_kit::root(body, layer)
}

/// The notes (toasts), bottom right.
fn notes(s: &State, f: &mut Frame) {
    if s.notes.is_empty() {
        return;
    }
    let mut region = ToastRegion::new();
    for n in &s.notes {
        let id = n.id;
        let mut t = Toast::new(format!("note-{id}"), n.title.clone())
            .description(n.body.clone())
            .on_close(move |s: &mut State| s.notes.retain(|x| x.id != id));
        if n.error {
            t = t.icon(f.k, &rune_kit::glyphs::CIRCLE_X);
        }
        region = region.toast(t.build(f.k));
    }
    f.layer.push(region.build(f.k));
}

/// The window's keys (AppSpec::on_key): a key nothing focused took. The
/// one hook: each package's keys chain here, the first to take it wins.
pub fn on_key(s: &mut State, ev: &rune::KeyEvent, tx: &mut rune::Tx) -> bool {
    crate::palette::on_key(s, ev, tx)
        || crate::call::on_key(s, ev, tx)
        || crate::board::on_key(s, ev, tx)
        || keys::on_key(s, ev, tx)
}

/// Drops notes whose time is up (AppSpec::poll).
pub fn poll(s: &mut State, _t: f64, tx: &mut rune::Tx) -> bool {
    let now = Instant::now();
    let before = s.notes.len();
    s.notes.retain(|n| n.until > now);
    if let Some(next) = s.notes.iter().map(|n| n.until).min() {
        tx.wake_after(next.saturating_duration_since(now));
    }
    let widgets = crate::widgets::poll(s, tx);
    let card = crate::card::poll(s, tx);
    let talk = crate::talk::poll(s);
    before != s.notes.len() || widgets || card || talk
}

/// A page a package hasn't ported yet: says so, so a screenshot shows
/// where the port stands.
pub fn not_yet(f: &Frame, title: &str, what: &str) -> El {
    let k = f.k;
    col()
        .key(format!("not-yet-{title}"))
        .w_full()
        .h_full()
        .padding(32.0)
        .gap(8.0)
        .child(text(title.to_string()).font(k.ty.ui_medium).color(k.c.text_base))
        .child(
            text(what.to_string())
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        )
        .child(el().grow(1.0))
}

/// A page's title bar: the title, then whatever the page puts on the right.
pub fn page_header(f: &Frame, title: &str, right: Vec<El>) -> El {
    let k = f.k;
    let mut r = row()
        .key("page-header")
        .w_full()
        .h(48.0)
        .px(20.0)
        .gap(12.0)
        .center_y()
        .child(
            text(title.to_string())
                .font(rune::tok::TextStyle::new(15.0, 600.0))
                .color(k.c.text_base),
        )
        .child(el().grow(1.0));
    for e in right {
        r = r.child(e);
    }
    r
}
