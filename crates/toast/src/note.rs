//! The OS-agnostic model (go-toast note.go): notes, the stack, expiry and
//! the person's options.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Kind;

/// A callback: run once, on its own thread, after its card closes.
pub type Callback = Arc<dyn Fn() + Send + Sync>;

/// A button on a card.
#[derive(Clone)]
pub struct Action {
    pub label: String,
    /// `None` only closes the card.
    pub run: Option<Callback>,
}

impl Action {
    pub fn new(label: impl Into<String>, run: impl Fn() + Send + Sync + 'static) -> Action {
        Action {
            label: label.into(),
            run: Some(Arc::new(run)),
        }
    }

    /// A button that only closes the card.
    pub fn close(label: impl Into<String>) -> Action {
        Action {
            label: label.into(),
            run: None,
        }
    }
}

impl fmt::Debug for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Action")
            .field("label", &self.label)
            .field("run", &self.run.is_some())
            .finish()
    }
}

/// One notification.
#[derive(Clone, Default)]
pub struct Note {
    /// When set, replaces the live card with the same id in place (and
    /// restarts its timer) instead of stacking a new one.
    pub id: String,
    pub title: String,
    /// Up to two lines; longer text is ellipsized.
    pub body: String,
    pub kind: Kind,
    /// Skips the sound.
    pub silent: bool,
    /// Overrides the default lifetime ([`default_lifetime`]). Ignored when
    /// sticky.
    pub expire: Duration,
    /// Stays until clicked, dismissed or closed.
    pub sticky: bool,
    /// Up to two buttons (extra ones are dropped); the first is primary.
    pub actions: Vec<Action>,
    /// A left click on the body. Without it a body click is a dismiss.
    pub on_click: Option<Callback>,
    /// The person dismissed it (right click, the x, or a body click with no
    /// `on_click`). Not when it expires, is replaced, pushed off a full
    /// stack, or closed with [`crate::close`].
    pub on_dismiss: Option<Callback>,
}

impl fmt::Debug for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Note")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("body", &self.body)
            .field("kind", &self.kind)
            .field("silent", &self.silent)
            .field("expire", &self.expire)
            .field("sticky", &self.sticky)
            .field("actions", &self.actions)
            .field("on_click", &self.on_click.is_some())
            .field("on_dismiss", &self.on_dismiss.is_some())
            .finish()
    }
}

pub const CORNER_TOP_RIGHT: &str = "top-right";
pub const CORNER_BOTTOM_RIGHT: &str = "bottom-right";
pub const CORNER_TOP_LEFT: &str = "top-left";
pub const CORNER_BOTTOM_LEFT: &str = "bottom-left";

/// The person's notification settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Off makes a post a no-op: no card and no sound. [`crate::play_sound`]
    /// still plays.
    pub enabled: bool,
    /// Off silences the cards' sounds.
    pub sound: bool,
    /// 0..1.
    pub volume: f64,
    /// One of the `CORNER_*` values.
    pub corner: String,
}

impl Default for Options {
    /// On, with sound, top-right.
    fn default() -> Self {
        Options {
            enabled: true,
            sound: true,
            volume: 0.6,
            corner: CORNER_TOP_RIGHT.into(),
        }
    }
}

impl Options {
    /// Snaps a hand-edited or partial value to valid ones.
    pub fn normalized(mut self) -> Options {
        if self.volume.is_nan() {
            self.volume = Options::default().volume;
        } else {
            self.volume = self.volume.clamp(0.0, 1.0);
        }
        if !matches!(
            self.corner.as_str(),
            CORNER_TOP_RIGHT | CORNER_BOTTOM_RIGHT | CORNER_TOP_LEFT | CORNER_BOTTOM_LEFT
        ) {
            self.corner = CORNER_TOP_RIGHT.into();
        }
        self
    }
}

pub const MAX_CARDS: usize = 5;
pub const MAX_ACTIONS: usize = 2;
pub const FADE_IN: Duration = Duration::from_millis(140);
pub const FADE_OUT: Duration = Duration::from_millis(320);
/// The least time a card has left once the cursor leaves the stack.
pub const AFTER_HOVER: Duration = Duration::from_millis(1500);
pub const FULL_ALPHA: u8 = 245;

/// How long a card stays fully visible before it fades (zero: sticky).
pub fn default_lifetime(n: &Note) -> Duration {
    if n.sticky {
        Duration::ZERO
    } else if n.expire > Duration::ZERO {
        n.expire
    } else if !n.actions.is_empty() {
        Duration::from_secs(10)
    } else if n.kind == Kind::Error || n.kind == Kind::Warn {
        Duration::from_secs(6)
    } else {
        Duration::from_secs(3)
    }
}

/// Trims a note to what a card shows; `None` for a note with nothing to
/// say. Go also replaces invalid UTF-8 with U+FFFD here; a Rust `String`
/// is always valid, so that half has nothing to do.
pub fn clean(mut n: Note) -> Option<Note> {
    n.title = n.title.trim().to_string();
    n.body = n.body.trim().to_string();
    if n.title.is_empty() {
        n.title = std::mem::take(&mut n.body);
    }
    n.actions.truncate(MAX_ACTIONS);
    if n.title.is_empty() { None } else { Some(n) }
}

/// A note on screen.
#[derive(Debug, Clone)]
pub struct Card {
    pub note: Note,
    /// Stable for the card's life, across same-id replaces.
    pub key: u64,
    /// Bumps on every replace, so the window repaints.
    pub rev: u64,
    /// First shown (fade-in).
    pub born: Instant,
    /// When it starts to fade; `None` = sticky.
    pub until: Option<Instant>,
    /// Fading out since.
    pub closing: Option<Instant>,
}

impl Card {
    /// The card's opacity now: fading in after it appears, out once it
    /// expires.
    pub fn alpha(&self, now: Instant) -> u8 {
        let mut f: f64 = 1.0;
        let age = now.saturating_duration_since(self.born);
        if age < FADE_IN {
            f = (age.as_secs_f64() / FADE_IN.as_secs_f64()).max(0.0);
        }
        if let Some(c) = self.closing {
            let gone = now.saturating_duration_since(c).as_secs_f64() / FADE_OUT.as_secs_f64();
            f = f.min(1.0 - gone);
        }
        if f <= 0.0 {
            return 0;
        }
        (f * FULL_ALPHA as f64 + 0.5) as u8
    }
}

/// The live cards, oldest first.
#[derive(Debug, Default)]
pub struct Stack {
    pub cards: Vec<Card>,
    next: u64,
}

impl Stack {
    /// Adds n, or replaces the card with the same id in place (keeping its
    /// slot). On a full stack the oldest non-sticky card (else the oldest)
    /// is pushed off and returned. Returns the posted card's key.
    pub fn post(&mut self, n: Note, now: Instant) -> (u64, Option<Card>) {
        let life = default_lifetime(&n);
        let until = (life > Duration::ZERO).then(|| now + life);
        if !n.id.is_empty()
            && let Some(c) = self.cards.iter_mut().find(|c| c.note.id == n.id)
        {
            c.note = n;
            c.until = until;
            c.closing = None;
            c.rev += 1;
            return (c.key, None);
        }
        let mut dropped = None;
        if self.cards.len() >= MAX_CARDS {
            let i = self.cards.iter().position(|c| !c.note.sticky).unwrap_or(0);
            dropped = Some(self.cards.remove(i));
        }
        self.next += 1;
        self.cards.push(Card {
            note: n,
            key: self.next,
            rev: 0,
            born: now,
            until,
            closing: None,
        });
        (self.next, dropped)
    }

    /// Removes and returns the card with key (a click or dismiss).
    pub fn take(&mut self, key: u64) -> Option<Card> {
        let i = self.cards.iter().position(|c| c.key == key)?;
        Some(self.cards.remove(i))
    }

    /// Removes the card with id at once, reporting whether one was live.
    pub fn close_id(&mut self, id: &str) -> bool {
        if id.is_empty() {
            return false;
        }
        match self.cards.iter().position(|c| c.note.id == id) {
            Some(i) => {
                self.cards.remove(i);
                true
            }
            None => false,
        }
    }

    /// Advances expiry. While the cursor is over the stack nothing expires
    /// and a fading card comes back; after it leaves every card keeps at
    /// least [`AFTER_HOVER`]. Cards whose fade-out finished are removed and
    /// returned.
    pub fn tick(&mut self, now: Instant, hovered: bool) -> Vec<Card> {
        if hovered {
            for c in &mut self.cards {
                c.closing = None;
                if let Some(u) = c.until
                    && u.saturating_duration_since(now) < AFTER_HOVER
                {
                    c.until = Some(now + AFTER_HOVER);
                }
            }
            return vec![];
        }
        let mut gone = vec![];
        let mut keep = Vec::with_capacity(self.cards.len());
        for mut c in self.cards.drain(..) {
            if c.closing.is_none()
                && let Some(u) = c.until
                && now >= u
            {
                c.closing = Some(now);
            }
            if let Some(cl) = c.closing
                && now.saturating_duration_since(cl) >= FADE_OUT
            {
                gone.push(c);
                continue;
            }
            keep.push(c);
        }
        self.cards = keep;
        gone
    }
}
