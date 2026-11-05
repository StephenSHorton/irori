//! `irori-notify`: Irori's desktop notifications, a port of
//! I:internal/notify (inbox.md §3.11-3.12): native cards from the `toast`
//! crate that sit above every other window, never take focus, and carry up
//! to two buttons.
//!
//! A [`Notifier`] is made once per Core. The desktop app's shows cards; a
//! headless Core's (`serve --headless`) never does: [`Notifier::post`]
//! reports false and nothing reaches the screen. Every card has an Irori
//! id ([`held_id`], [`needs_id`], [`message_id`], [`ask_id`],
//! [`unclaimed_id`], [`returned_id`], [`TEST_ID`]), so the same thing never
//! stacks twice and [`Notifier::clear`] takes it down once it is resolved
//! elsewhere.
//!
//! [`watch()`] turns the inbox, Board and ask change streams into cards;
//! the notify tool (crates/tools) posts a card for an agent.
//!
//! Wiring notes (for the merge):
//! - core: `tools::Notifier` is the tools crate's seam (crates/tools
//!   notify.rs). notify sits below tools in the layering, so the adapter
//!   lives in core: `headless()`/`enabled()` map one to one, and `post`
//!   copies `tools::Note` into [`Note`] (level via [`parse_level`], the
//!   `Arc` callbacks as they are, `expire` and `on_expire` too). Core makes
//!   one [`Notifier`] per data dir ([`Config::dir`] = the data dir,
//!   `headless` for `serve --headless`) and starts [`watch()`] after its
//!   services open, with [`Sources`] = its inbox, coord (Board and Room),
//!   ask book, `user:<device>`, the Chain's `IsOwner` and the roster's
//!   display names; it stops the [`WatchHandle`] on close.
//! - M6 shell: [`Notifier::set_opener`] with a function that shows the
//!   window at a [`Target`] (tab `inbox`/`board`/`held`, ref).
//! - M6 commands: `settings_get_notifications` is [`Notifier::options`],
//!   `settings_set_notifications` [`Notifier::set_options`] (announce with
//!   [`Notifier::on_options`]), `settings_test_notification`
//!   [`Notifier::post_test`].

pub mod look;
mod settings;
pub mod thumb;
pub mod watch;

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

pub use inbox::watch::{
    Level, SeenStore, TAB_BOARD, TAB_HELD, TAB_INBOX, TAB_TALK, Target, WAITING_ID, ask_id,
    held_id, message_id, needs_id, parse_level, returned_id, talk_conv, talk_id, unclaimed_id,
};
pub use look::{CardColors, CardLook};
pub use settings::{SEEN_FILE, SETTINGS_FILE, SeenFile, load, load_look, save, save_look};
pub use thumb::{attachment_thumbnail, thumbnail};
pub use watch::{
    AskSource, BoardSource, FileFn, InboxFeed, RoomSource, Sources, WatchHandle, hold_reason,
    preview, upper, watch,
};

/// Whether a card is one a phone shows (card #395): a Talk post that
/// mentions the person or is a DM to them, or an agent's message to the
/// person (Talk shows it as a DM). On a phone only mentions and DMs
/// notify: asks, held messages and cards waiting on you stay off it.
pub fn on_phone(id: &str) -> bool {
    talk_conv(id).is_some() || id.starts_with("msg:")
}

/// Where a phone notification `id` opens when tapped: a Talk post its
/// conversation, an agent's message its DM (by its delivery).
pub fn phone_target(id: &str) -> Option<Target> {
    if let Some(conv) = talk_conv(id) {
        return Some(Target {
            tab: TAB_TALK.into(),
            ref_: conv.into(),
        });
    }
    id.strip_prefix("msg:")
        .filter(|d| !d.is_empty())
        .map(|d| Target {
            tab: TAB_INBOX.into(),
            ref_: d.into(),
        })
}

/// The most buttons a card shows (the toast engine's limit).
pub const MAX_ACTIONS: usize = 2;

/// The settings' "Send a test notification" card.
pub const TEST_ID: &str = "test";

/// Set to 1, screenshots and recordings see the cards (they are hidden
/// from capture by default). It is for checking a card's look.
pub const CAPTURE_ENV: &str = "IRORI_TOAST_CAPTURE";

/// A callback, run once after its card closes, on its own thread.
pub type Callback = Arc<dyn Fn() + Send + Sync>;

/// A button. `None` only closes the card.
#[derive(Clone)]
pub struct Action {
    pub label: String,
    pub run: Option<Callback>,
}

impl Action {
    pub fn new(label: impl Into<String>, run: impl Fn() + Send + Sync + 'static) -> Action {
        Action {
            label: label.into(),
            run: Some(Arc::new(run)),
        }
    }

    pub fn close(label: impl Into<String>) -> Action {
        Action {
            label: label.into(),
            run: None,
        }
    }
}

/// One card.
#[derive(Clone, Default)]
pub struct Note {
    /// Required: a post without one is refused, so nothing stacks twice.
    pub id: String,
    pub title: String,
    /// Up to two lines on the card.
    pub body: String,
    /// A thumbnail under the body ([`thumbnail`] makes one from a PNG).
    pub image: Option<toast::Image>,
    pub level: Level,
    pub silent: bool,
    /// Stays until answered, dismissed or cleared.
    pub sticky: bool,
    /// How long the card lives. With `on_expire`, the Notifier owns the
    /// clock: when it passes with no button, body click or dismiss, the
    /// card is closed and `on_expire` runs. Zero is the engine's default
    /// lifetime and no `on_expire`.
    pub expire: Duration,
    /// At most [`MAX_ACTIONS`]; the first is the primary.
    pub actions: Vec<Action>,
    /// A body click; `None` makes a body click a dismiss.
    pub on_click: Option<Callback>,
    /// The person dismissed it (the x, a right click).
    pub on_dismiss: Option<Callback>,
    pub on_expire: Option<Callback>,
    /// What it is about, for the person's notification history (card
    /// #377): a post with it is recorded through [`Notifier::set_recorder`]
    /// even when notifications are off. `None` is not kept (the test card,
    /// the updater's).
    pub about: Option<coord::notifications::PostedRow>,
}

impl fmt::Debug for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Note")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("body", &self.body)
            .field("image", &self.image)
            .field("level", &self.level)
            .field("sticky", &self.sticky)
            .field("expire", &self.expire)
            .field(
                "actions",
                &self.actions.iter().map(|a| &a.label).collect::<Vec<_>>(),
            )
            .finish()
    }
}

pub const CORNER_TOP_RIGHT: &str = toast::CORNER_TOP_RIGHT;
pub const CORNER_BOTTOM_RIGHT: &str = toast::CORNER_BOTTOM_RIGHT;
pub const CORNER_TOP_LEFT: &str = toast::CORNER_TOP_LEFT;
pub const CORNER_BOTTOM_LEFT: &str = toast::CORNER_BOTTOM_LEFT;

/// The person's notification settings, kept in settings.json. Field order
/// is Go's (the JSON the window and settings.json read).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Options {
    pub enabled: bool,
    pub sound: bool,
    /// 0..1.
    pub volume: f64,
    /// One of the `CORNER_*` values.
    pub corner: String,
    /// A confetti burst from each success card (off by default). Left out
    /// of settings.json while off, so the file stays as Go wrote it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub confetti: bool,
}

impl Default for Options {
    /// On, with sound, top-right.
    fn default() -> Self {
        from_toast(toast::Options::default())
    }
}

impl Options {
    /// Snaps hand-edited values the way the engine does.
    pub fn normalized(self) -> Options {
        from_toast(to_toast(&self).normalized())
    }
}

fn to_toast(o: &Options) -> toast::Options {
    toast::Options {
        enabled: o.enabled,
        sound: o.sound,
        volume: o.volume,
        corner: o.corner.clone(),
        confetti: o.confetti,
    }
}

fn from_toast(o: toast::Options) -> Options {
    Options {
        enabled: o.enabled,
        sound: o.sound,
        volume: o.volume,
        corner: o.corner,
        confetti: o.confetti,
    }
}

/// The cards' setup for a look (none: [`look::base_config`]).
fn config_for(l: Option<&CardLook>, capture_exclude: bool) -> toast::Config {
    let base = look::base_config(capture_exclude);
    match l {
        Some(l) => l.apply(base),
        None => base,
    }
}

fn kind_of(l: Level) -> toast::Kind {
    match l {
        Level::Success => toast::Kind::Success,
        Level::Warn => toast::Kind::Warn,
        Level::Error => toast::Kind::Error,
        Level::Info => toast::Kind::Info,
    }
}

/// The card system: the toast engine on the desktop, a fake in tests.
pub trait Toaster: Send + Sync {
    fn configure(&self, c: toast::Config);
    fn set_options(&self, o: toast::Options);
    fn post(&self, n: toast::Note);
    fn close(&self, id: &str);
}

/// The real cards.
pub struct Cards;

impl Toaster for Cards {
    fn configure(&self, c: toast::Config) {
        toast::configure(c);
    }
    fn set_options(&self, o: toast::Options) {
        toast::set_options(o);
    }
    fn post(&self, n: toast::Note) {
        toast::post(n);
    }
    fn close(&self, id: &str) {
        toast::close(id);
    }
}

/// Configures [`Notifier::new`].
#[derive(Default, Clone)]
pub struct Config {
    /// Irori's data folder; settings.json there holds the options. `None`
    /// keeps them in memory only.
    pub dir: Option<PathBuf>,
    /// Never show a card (`serve --headless`).
    pub headless: bool,
    /// Defaults to [`Cards`]. Tests pass a fake.
    pub toaster: Option<Arc<dyn Toaster>>,
    /// Lets screen capture see the cards. [`CAPTURE_ENV`]=1 does too.
    pub capture_visible: bool,
}

/// A cancellable one-shot timer (Go `time.AfterFunc` + `Stop`).
struct Timer {
    stopped: Mutex<bool>,
    cv: Condvar,
}

impl Timer {
    fn stop(&self) {
        *self.stopped.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.cv.notify_all();
    }
}

/// One posted card's bookkeeping: generation tells a replaced card's callbacks
/// from the live one's, so only one of a card's outcomes ever runs.
struct Entry {
    generation: u64,
    timer: Option<Arc<Timer>>,
}

type Opener = Arc<dyn Fn(Target) + Send + Sync>;
/// Keeps a posted note in the history (card #377).
pub type Recorder = Arc<dyn Fn(&Note) + Send + Sync>;
type OptionsFn = Arc<dyn Fn(&Options) + Send + Sync>;

struct State {
    opts: Options,
    /// The cards' look the window last sent (or the saved one).
    look: Option<CardLook>,
    live: HashMap<String, Entry>,
    generation: u64,
    opener: Option<Opener>,
    recorder: Option<Recorder>,
    on_change: Vec<OptionsFn>,
}

struct Inner {
    /// `None` when headless.
    t: Option<Arc<dyn Toaster>>,
    dir: Option<PathBuf>,
    /// Cards are hidden from screen capture.
    capture_exclude: bool,
    st: Mutex<State>,
}

/// Shows Irori's cards. Cheap to clone; safe for concurrent use.
#[derive(Clone)]
pub struct Notifier {
    inner: Arc<Inner>,
}

impl fmt::Debug for Notifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Notifier")
            .field("headless", &self.is_headless())
            .finish()
    }
}

impl Notifier {
    /// Loads the saved options and look and, unless headless, configures
    /// the cards: app name "Irori", the person's theme, light/dark and
    /// glass as the window last sent them ([`Notifier::set_look`]; dark,
    /// solid and rounded before it ever has), hidden from screen capture
    /// (see [`CAPTURE_ENV`]).
    pub fn new(c: Config) -> Notifier {
        let opts = c.dir.as_deref().map(load).unwrap_or_default();
        let look = c.dir.as_deref().and_then(load_look);
        let capture = c.capture_visible || std::env::var(CAPTURE_ENV).as_deref() == Ok("1");
        let t = if c.headless {
            None
        } else {
            let t = c.toaster.clone().unwrap_or_else(|| Arc::new(Cards));
            t.configure(config_for(look.as_ref(), !capture));
            t.set_options(to_toast(&opts));
            Some(t)
        };
        Notifier {
            inner: Arc::new(Inner {
                t,
                dir: c.dir,
                capture_exclude: !capture,
                st: Mutex::new(State {
                    opts,
                    look,
                    live: HashMap::new(),
                    generation: 0,
                    opener: None,
                    recorder: None,
                    on_change: vec![],
                }),
            }),
        }
    }

    /// A Notifier that never shows a card (Go's nil `*Notifier`).
    pub fn headless() -> Notifier {
        Notifier::new(Config {
            headless: true,
            ..Config::default()
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether this Notifier never shows cards.
    pub fn is_headless(&self) -> bool {
        self.inner.t.is_none()
    }

    /// Whether a post would show a card now: not headless, and the person
    /// has notifications on.
    pub fn enabled(&self) -> bool {
        !self.is_headless() && self.lock().opts.enabled
    }

    /// The person's settings.
    pub fn options(&self) -> Options {
        self.lock().opts.clone()
    }

    /// Applies and saves the person's settings. Turning notifications off
    /// takes every card down (their `on_expire` still runs when due, so an
    /// agent waiting on an answer hears that none came).
    pub fn set_options(&self, o: Options) -> (Options, Result<(), String>) {
        let o = o.normalized();
        let fns = {
            let mut st = self.lock();
            st.opts = o.clone();
            st.on_change.clone()
        };
        if let Some(t) = &self.inner.t {
            t.set_options(to_toast(&o));
        }
        let err = match &self.inner.dir {
            Some(d) => save(d, &o),
            None => Ok(()),
        };
        for f in fns {
            f(&o);
        }
        (o, err)
    }

    /// The cards' look in effect (`None`: the window hasn't sent one).
    pub fn look(&self) -> Option<CardLook> {
        self.lock().look.clone()
    }

    /// Restyles the cards, those on screen too, to the person's
    /// appearance and saves it so the next start shows it at once (card
    /// #527). The window sends it at startup and on every change; an
    /// unchanged look neither reconfigures nor writes.
    pub fn set_look(&self, l: CardLook) -> Result<(), String> {
        {
            // Configured under the lock, so two sends land in order.
            let mut st = self.lock();
            if st.look.as_ref() == Some(&l) {
                return Ok(());
            }
            st.look = Some(l.clone());
            if let Some(t) = &self.inner.t {
                t.configure(config_for(Some(&l), self.inner.capture_exclude));
            }
        }
        match &self.inner.dir {
            Some(d) => save_look(d, &l),
            None => Ok(()),
        }
    }

    /// Registers f for every [`Notifier::set_options`] (the window
    /// announces them).
    pub fn on_options(&self, f: impl Fn(&Options) + Send + Sync + 'static) {
        self.lock().on_change.push(Arc::new(f));
    }

    /// Sets what Open does: the desktop app shows its window at the
    /// target's tab. Headless leaves it unset.
    pub fn set_opener(&self, f: impl Fn(Target) + Send + Sync + 'static) {
        self.lock().opener = Some(Arc::new(f));
    }

    /// Shows the Irori window at t (a no-op with no opener).
    pub fn open(&self, t: Target) {
        let f = self.lock().opener.clone();
        if let Some(f) = f {
            f(t);
        }
    }

    /// Keeps every note posted with an `about` (card #377): the Core
    /// writes it to the person's notification history.
    pub fn set_recorder(&self, f: impl Fn(&Note) + Send + Sync + 'static) {
        self.lock().recorder = Some(Arc::new(f));
    }

    /// Shows note, replacing the live card with the same id. False when
    /// headless, when notifications are off, or when the note has no id or
    /// nothing to say.
    pub fn post(&self, mut note: Note) -> bool {
        note.id = note.id.trim().to_string();
        if note.id.is_empty() || format!("{}{}", note.title, note.body).trim().is_empty() {
            return false;
        }
        // Kept even with the popups off: the history is every notification.
        if note.about.is_some() {
            let rec = self.lock().recorder.clone();
            if let Some(rec) = rec {
                rec(&note);
            }
        }
        if !self.enabled() {
            return false;
        }
        let Some(t) = self.inner.t.clone() else {
            return false;
        };
        let owned = note.on_expire.is_some() && note.expire > Duration::ZERO;
        let generation = {
            let mut st = self.lock();
            st.generation += 1;
            let generation = st.generation;
            if let Some(old) = st.live.get(&note.id)
                && let Some(tm) = &old.timer
            {
                tm.stop();
            }
            let mut e = Entry {
                generation,
                timer: None,
            };
            if owned {
                let tm = Arc::new(Timer {
                    stopped: Mutex::new(false),
                    cv: Condvar::new(),
                });
                e.timer = Some(tm.clone());
                let (n, id, f, d) = (
                    self.clone(),
                    note.id.clone(),
                    note.on_expire.clone().expect("owned"),
                    note.expire,
                );
                let spawned = std::thread::Builder::new()
                    .name("notify-expire".into())
                    .spawn(move || {
                        let g = tm.stopped.lock().unwrap_or_else(|e| e.into_inner());
                        let (g, _) = tm
                            .cv
                            .wait_timeout_while(g, d, |stopped| !*stopped)
                            .unwrap_or_else(|e| e.into_inner());
                        if *g {
                            return;
                        }
                        drop(g);
                        if n.take(&id, generation) {
                            if let Some(t) = &n.inner.t {
                                t.close(&id);
                            }
                            f();
                        }
                    });
                if let Err(err) = spawned {
                    eprintln!("irori: notifications: expiry timer: {err}");
                }
            }
            st.live.insert(note.id.clone(), e);
            generation
        };

        // Each outcome runs at most once, and only for the card that is live.
        let wrap = |f: Option<Callback>| -> Option<toast::Callback> {
            let n = self.clone();
            let id = note.id.clone();
            Some(Arc::new(move || {
                if n.take(&id, generation)
                    && let Some(f) = &f
                {
                    f();
                }
            }))
        };
        let mut tn = toast::Note {
            id: note.id.clone(),
            title: note.title.clone(),
            body: note.body.clone(),
            image: note.image.clone(),
            kind: kind_of(note.level),
            silent: note.silent,
            sticky: note.sticky,
            expire: note.expire,
            on_dismiss: wrap(note.on_dismiss.clone()),
            ..toast::Note::default()
        };
        if owned {
            // The Notifier's timer closes the card; the engine keeps it.
            tn.sticky = true;
            tn.expire = Duration::ZERO;
        }
        if note.on_click.is_some() {
            tn.on_click = wrap(note.on_click.clone());
        }
        for a in note.actions.iter().take(MAX_ACTIONS) {
            tn.actions.push(toast::Action {
                label: a.label.clone(),
                run: wrap(a.run.clone()),
            });
        }
        t.post(tn);
        true
    }

    /// Ends card id's generation generation, reporting whether it was still live
    /// (so its outcome should run).
    fn take(&self, id: &str, generation: u64) -> bool {
        let mut st = self.lock();
        match st.live.get(id) {
            Some(e) if e.generation == generation => {
                if let Some(tm) = &e.timer {
                    tm.stop();
                }
                st.live.remove(id);
                true
            }
            _ => false,
        }
    }

    /// Takes card id down without running any of its callbacks: the thing
    /// it was about was resolved elsewhere. A no-op when none is up.
    pub fn clear(&self, id: &str) {
        let Some(t) = &self.inner.t else { return };
        if id.is_empty() {
            return;
        }
        let e = self.lock().live.remove(id);
        if let Some(e) = e {
            if let Some(tm) = &e.timer {
                tm.stop();
            }
            t.close(id);
        }
    }

    /// Whether card id is up as far as the Notifier knows (posted and not
    /// answered, dismissed, expired or cleared). A card the engine let
    /// expire on its own lifetime stays "live" until the next post or
    /// clear of that id; that only costs a no-op close.
    pub fn live(&self, id: &str) -> bool {
        self.lock().live.contains_key(id)
    }

    /// The settings' sample card with two buttons, with the current sound
    /// and corner (Go `SettingsService.TestNotification`).
    pub fn post_test(&self) -> Result<(), String> {
        if self.is_headless() {
            return Err("this Irori shows no notifications".into());
        }
        if !self.enabled() {
            return Err("notifications are off; turn them on first".into());
        }
        let n = self.clone();
        self.post(Note {
            id: TEST_ID.into(),
            title: "Irori notifications are on".into(),
            body: "Cards like this show what needs you: held messages, blocked cards, and agents asking you to choose.".into(),
            // With confetti on, the test card is good news, so it shows it.
            level: if self.options().confetti {
                Level::Success
            } else {
                Level::Info
            },
            actions: vec![
                Action::new("Open Board", move || {
                    n.open(Target {
                        tab: TAB_BOARD.into(),
                        ref_: String::new(),
                    })
                }),
                Action::close("Close"),
            ],
            ..Note::default()
        });
        Ok(())
    }
}
