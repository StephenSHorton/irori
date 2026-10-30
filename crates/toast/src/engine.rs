//! The card engine shared by both platforms: the model under one lock and
//! the window-side state that syncs windows to it (go-toast's `syncCards`,
//! `measure`, the mouse handlers and `click`, which Go writes once per OS).
//!
//! A platform implements [`Surface`] (create, draw, place, fade and destroy
//! one window per card, measure text, find the work area, run a tick timer)
//! and calls [`Engine::sync`], [`Engine::tick`], [`Engine::rehome`] and
//! [`Engine::mouse`] on its window thread. Tests drive the same engine
//! with a recording surface, headless.
//!
//! Rules kept from Go: the window thread never runs a note's callbacks
//! itself (each gets its own thread once the card is out of the model and
//! the lock is free, so a callback that posts, closes or blocks can't wedge
//! the windows); a click only takes the card out of the model and asks for
//! a sync, so the clicked window isn't destroyed inside its own message.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use crate::config::{Config, Kind, Palette};
use crate::layout::{
    CARD_GAP_DIP, CARD_MARGIN_DIP, CARD_WIDTH_DIP, MAX_BODY_LINES, Part, Rect, TEXT_LEFT_DIP,
    TEXT_RIGHT_DIP, button_rects, card_height_dip, close_rect, hit_part, px, stack_origins,
};
use crate::note::{Callback, FULL_ALPHA, Note, Options, Stack, clean};
use crate::sound::{Sound, sound_for};

/// What a window shows: everything a platform needs to draw a card.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub key: u64,
    pub title: String,
    pub body: String,
    pub kind: Kind,
    pub labels: Vec<String>,
    /// Body lines (0, 1 or 2).
    pub lines: usize,
    pub w: i32,
    pub h: i32,
    /// The pixel scale (1 on macOS, where geometry is in points).
    pub scale: f64,
    /// Corner radius in px.
    pub radius: i32,
    pub close: Rect,
    pub buttons: Vec<Rect>,
    pub palette: Palette,
    /// The part under the cursor.
    pub hot: Part,
    /// The cursor is over this card.
    pub hovered: bool,
    /// The body has a click action (a hand cursor).
    pub clickable: bool,
}

impl Look {
    /// The cursor a platform shows over the card.
    pub fn hand_cursor(&self) -> bool {
        matches!(self.hot, Part::Button(_) | Part::Close)
            || (self.hot == Part::Body && self.clickable)
    }
}

/// A platform's windows, driven on its window thread.
pub trait Surface {
    /// The work area (px, top-down) and scale of the screen a new stack
    /// goes on (`active`: where the person is looking) or of the one the
    /// stack at `old` is on (the active one when it is gone).
    fn work_area(&mut self, active: bool, old: Rect) -> (Rect, f64);
    /// How many lines `body` wraps to in `width` px (1 or 2 once clamped).
    fn body_lines(&mut self, body: &str, width: i32, scale: f64) -> usize;
    /// A button label's text width in px; `None` when it can't measure.
    fn label_width(&mut self, label: &str, scale: f64) -> Option<i32>;
    /// Makes the card's window, hidden and fully transparent.
    fn create(&mut self, key: u64, cfg: &Config, w: i32, h: i32) -> bool;
    /// The card's content or hover state changed.
    fn draw(&mut self, look: &Look);
    /// Moves and sizes the window; `raise` puts it back on top of the
    /// topmost band, `placed` says it was placed before.
    #[allow(clippy::too_many_arguments)]
    fn place(&mut self, key: u64, x: i32, y: i32, w: i32, h: i32, raise: bool, placed: bool);
    fn alpha(&mut self, key: u64, a: u8);
    /// Shows it without activating anything.
    fn show(&mut self, key: u64);
    fn destroy(&mut self, key: u64);
    /// Runs or stops the 30 ms tick.
    fn timer(&mut self, on: bool);
}

/// How the next sync places the stack, in increasing strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Reloc {
    None,
    /// Re-place the stack on its screen (it may have changed).
    Rehome,
    /// A new stack: the screen the person is on.
    Active,
}

/// The tick interval while any card fades or counts down.
pub const TICK_MS: u32 = 30;

/// A mouse event on a card, in its top-down pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
    Move,
    Leave,
    Down,
    Up,
    /// A right click (or a Control-click on the Mac) dismisses.
    RightUp,
}

struct Shared {
    stack: Stack,
    reloc: Reloc,
    raise: bool,
    cfg: Config,
    opts: Options,
    /// Card keys on screen, oldest first (tests).
    keys: Vec<u64>,
}

/// One card's window and its copy of the content.
struct CardWin {
    key: u64,
    rev: u64,
    scale: f64,
    cfg: Config,
    title: String,
    body: String,
    kind: Kind,
    labels: Vec<String>,
    clickable: bool,
    lines: usize,
    w: i32,
    h: i32,
    radius: i32,
    close: Rect,
    buttons: Vec<Rect>,
    x: i32,
    y: i32,
    pw: i32,
    ph: i32,
    placed: bool,
    alpha: u8,
    hot: Part,
    pressed: Part,
    /// The hover state last drawn.
    hovered: bool,
}

struct Ui {
    wins: BTreeMap<u64, CardWin>,
    work: Rect,
    scale: f64,
    /// The card under the cursor, 0 = none.
    hover_key: u64,
    ticking: bool,
}

/// What a post did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Posted {
    /// The card is in the model (the window thread should sync).
    pub shown: bool,
    /// The sound to play, at this volume.
    pub sound: Option<(Sound, f64)>,
}

/// The cards' model and window state.
pub struct Engine {
    shared: Mutex<Shared>,
    ui: Mutex<Ui>,
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs a callback on its own thread; a panic is logged, not spread.
pub fn run_callback(f: Option<Callback>, log_prefix: &str) {
    let Some(f) = f else { return };
    let prefix = log_prefix.to_string();
    let spawned = std::thread::Builder::new()
        .name("toast-callback".into())
        .spawn(move || {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f())).is_err() {
                eprintln!("{prefix}notification callback panicked");
            }
        });
    if let Err(e) = spawned {
        eprintln!("{log_prefix}can't run a notification callback: {e}");
    }
}

impl Engine {
    pub fn new() -> Engine {
        Engine {
            shared: Mutex::new(Shared {
                stack: Stack::default(),
                reloc: Reloc::None,
                raise: false,
                cfg: Config::default().normalized(),
                opts: Options::default(),
                keys: vec![],
            }),
            ui: Mutex::new(Ui {
                wins: BTreeMap::new(),
                work: Rect::default(),
                scale: 1.0,
                hover_key: 0,
                ticking: false,
            }),
        }
    }

    /// Sets the app-level setup. Windows already up keep the one they were
    /// made with.
    pub fn configure(&self, c: Config) {
        lock(&self.shared).cfg = c.normalized();
    }

    /// The setup in effect, defaults filled in.
    pub fn config(&self) -> Config {
        lock(&self.shared).cfg.clone()
    }

    /// Applies the person's settings; turning cards off clears them.
    /// Returns true: the window thread should sync (a corner change moves
    /// the stack).
    pub fn set_options(&self, o: Options) -> bool {
        let o = o.normalized();
        let mut s = lock(&self.shared);
        if !o.enabled {
            s.stack.cards.clear();
        }
        s.opts = o;
        true
    }

    pub fn options(&self) -> Options {
        lock(&self.shared).opts.clone()
    }

    /// Puts n in the stack (or replaces the card with its id). A no-op when
    /// cards are off or the note has nothing to say.
    pub fn post(&self, n: Note, now: Instant) -> Posted {
        let mut s = lock(&self.shared);
        let o = s.opts.clone();
        let none = Posted {
            shown: false,
            sound: None,
        };
        if !o.enabled {
            return none;
        }
        let Some(n) = clean(n) else { return none };
        if s.stack.cards.is_empty() {
            s.reloc = Reloc::Active; // a new stack goes where the person is looking
        } else if s.reloc < Reloc::Rehome {
            s.reloc = Reloc::Rehome; // a screen unplugged under a sticky card moves the stack
        }
        let sound =
            (!n.silent && o.sound).then(|| (sound_for(n.kind, !n.actions.is_empty()), o.volume));
        s.stack.post(n, now);
        s.raise = true;
        Posted { shown: true, sound }
    }

    /// Removes the card with id without running its callbacks; true when one
    /// was up (sync).
    pub fn close(&self, id: &str) -> bool {
        lock(&self.shared).stack.close_id(id)
    }

    /// A display or work-area change: re-place the stack on the next sync.
    pub fn rehome(&self) {
        let mut s = lock(&self.shared);
        if !s.stack.cards.is_empty() && s.reloc < Reloc::Rehome {
            s.reloc = Reloc::Rehome;
            s.raise = true;
        }
    }

    /// Advances expiry (the tick timer), then the caller syncs.
    pub fn tick(&self, now: Instant) {
        let hovered = lock(&self.ui).hover_key != 0;
        lock(&self.shared).stack.tick(now, hovered);
    }

    /// The cards on screen, oldest first.
    pub fn keys(&self) -> Vec<u64> {
        lock(&self.shared).keys.clone()
    }

    /// Whether any card window is up.
    pub fn visible(&self) -> bool {
        !lock(&self.ui).wins.is_empty()
    }

    /// The model's notes, oldest first (tests).
    pub fn notes(&self) -> Vec<Note> {
        lock(&self.shared)
            .stack
            .cards
            .iter()
            .map(|c| c.note.clone())
            .collect()
    }

    /// Makes the windows match the model: creates, re-measures, places,
    /// fades and destroys them, and runs the tick timer while any card is
    /// fading or counting down (a stack of only sticky cards needs none).
    pub fn sync(&self, surf: &mut dyn Surface, now: Instant) {
        struct View {
            key: u64,
            rev: u64,
            title: String,
            body: String,
            kind: Kind,
            labels: Vec<String>,
            clickable: bool,
            alpha: u8,
        }
        let mut timed = false;
        let (views, reloc, up, corner, cfg) = {
            let mut s = lock(&self.shared);
            let views: Vec<View> = s
                .stack
                .cards
                .iter()
                .map(|c| {
                    let alpha = c.alpha(now);
                    if c.until.is_some() || c.closing.is_some() || alpha < FULL_ALPHA {
                        timed = true;
                    }
                    View {
                        key: c.key,
                        rev: c.rev,
                        title: c.note.title.clone(),
                        body: c.note.body.clone(),
                        kind: c.note.kind,
                        labels: c.note.actions.iter().map(|a| a.label.clone()).collect(),
                        clickable: c.note.on_click.is_some(),
                        alpha,
                    }
                })
                .collect();
            let out = (
                views,
                s.reloc,
                s.raise,
                s.opts.corner.clone(),
                s.cfg.clone(),
            );
            s.reloc = Reloc::None;
            s.raise = false;
            out
        };

        let mut guard = lock(&self.ui);
        let ui = &mut *guard;
        if reloc != Reloc::None && !views.is_empty() {
            let (w, sc) = surf.work_area(reloc == Reloc::Active, ui.work);
            ui.work = w;
            ui.scale = sc;
        }
        let sc = ui.scale;

        let gone: Vec<u64> = ui
            .wins
            .keys()
            .copied()
            .filter(|k| !views.iter().any(|v| v.key == *k))
            .collect();
        for k in gone {
            ui.wins.remove(&k);
            if ui.hover_key == k {
                ui.hover_key = 0; // no leave comes for a destroyed window
            }
            surf.destroy(k);
        }

        let hover_key = ui.hover_key;
        let mut order: Vec<(u64, u8)> = vec![];
        for v in views {
            let fresh = !ui.wins.contains_key(&v.key);
            let mut cw = ui.wins.remove(&v.key).unwrap_or_else(|| CardWin {
                key: v.key,
                rev: 0,
                scale: 0.0,
                cfg: cfg.clone(),
                title: String::new(),
                body: String::new(),
                kind: Kind::Info,
                labels: vec![],
                clickable: false,
                lines: 0,
                w: 0,
                h: 0,
                radius: 0,
                close: Rect::default(),
                buttons: vec![],
                x: 0,
                y: 0,
                pw: 0,
                ph: 0,
                placed: false,
                alpha: 0,
                hot: Part::None,
                pressed: Part::None,
                hovered: false,
            });
            if fresh || cw.rev != v.rev || cw.scale != sc {
                cw.rev = v.rev;
                cw.scale = sc;
                cw.title = v.title;
                cw.body = v.body;
                cw.kind = v.kind;
                cw.labels = v.labels;
                cw.clickable = v.clickable;
                measure(&mut cw, surf);
                if fresh && !surf.create(cw.key, &cw.cfg, cw.w, cw.h) {
                    continue;
                }
                cw.hovered = hover_key == cw.key;
                surf.draw(&look(&cw));
            }
            order.push((cw.key, v.alpha));
            ui.wins.insert(cw.key, cw);
        }

        let heights: Vec<i32> = order.iter().map(|(k, _)| ui.wins[k].h).collect();
        let origins = stack_origins(
            ui.work,
            &corner,
            px(CARD_WIDTH_DIP, sc),
            &heights,
            px(CARD_GAP_DIP, sc),
            px(CARD_MARGIN_DIP, sc),
        );
        for (i, (k, alpha)) in order.iter().enumerate() {
            let p = origins[i];
            let cw = ui.wins.get_mut(k).expect("placed above");
            let moved = !cw.placed || p.x != cw.x || p.y != cw.y || cw.w != cw.pw || cw.h != cw.ph;
            if moved || up {
                surf.place(cw.key, p.x, p.y, cw.w, cw.h, up, cw.placed);
                cw.x = p.x;
                cw.y = p.y;
                cw.pw = cw.w;
                cw.ph = cw.h;
            }
            if *alpha != cw.alpha || !cw.placed {
                cw.alpha = *alpha;
                surf.alpha(cw.key, cw.alpha);
            }
            if !cw.placed {
                cw.placed = true;
                surf.show(cw.key);
            }
        }

        let keys: Vec<u64> = order.iter().map(|(k, _)| *k).collect();
        if timed && !ui.ticking {
            surf.timer(true);
            ui.ticking = true;
        } else if !timed && ui.ticking {
            surf.timer(false);
            ui.ticking = false;
        }
        drop(guard);
        lock(&self.shared).keys = keys;
    }

    /// A mouse event on card key at (x, y) in its pixel coordinates. True
    /// when a click took the card out of the model: the caller queues a
    /// sync (never runs it inside the window's own message).
    pub fn mouse(&self, surf: &mut dyn Surface, key: u64, ev: Mouse, x: i32, y: i32) -> bool {
        let mut guard = lock(&self.ui);
        let ui = &mut *guard;
        let hover_key = ui.hover_key;
        let Some(cw) = ui.wins.get_mut(&key) else {
            return false;
        };
        let mut new_hover = hover_key;
        let mut click = None;
        match ev {
            Mouse::Move => {
                new_hover = key;
                cw.hot = hit_part(x, y, cw.close, &cw.buttons);
            }
            Mouse::Leave => {
                if hover_key == key {
                    new_hover = 0;
                }
                cw.pressed = Part::None;
                cw.hot = Part::None;
            }
            Mouse::Down => cw.pressed = hit_part(x, y, cw.close, &cw.buttons),
            Mouse::Up => {
                // Pressed and released on the same part, like a button; a
                // release off the card (a press dragged away) is no click.
                let p = if x >= 0 && y >= 0 && x < cw.w && y < cw.h {
                    hit_part(x, y, cw.close, &cw.buttons)
                } else {
                    Part::None
                };
                if p != Part::None && p == cw.pressed {
                    click = Some(p);
                }
                cw.pressed = Part::None;
            }
            Mouse::RightUp => click = Some(Part::Close),
        }
        let hovered = new_hover == key;
        let l = look(cw);
        let redraw = cw.hovered != hovered || ev == Mouse::Move || ev == Mouse::Leave;
        cw.hovered = hovered;
        ui.hover_key = new_hover;
        drop(guard);
        if redraw {
            surf.draw(&Look { hovered, ..l });
        }
        match click {
            Some(p) => self.click(key, p),
            None => false,
        }
    }

    /// The card leaves the model first, then its callback gets its own
    /// thread. True when a card was taken (sync).
    pub fn click(&self, key: u64, p: Part) -> bool {
        let (c, prefix) = {
            let mut s = lock(&self.shared);
            let prefix = s.cfg.log_prefix();
            (s.stack.take(key), prefix)
        };
        let Some(c) = c else { return false };
        let n = c.note;
        let f = match p {
            Part::Button(i) if i < n.actions.len() => n.actions[i].run.clone(),
            Part::Body if n.on_click.is_some() => n.on_click.clone(),
            _ => n.on_dismiss.clone(),
        };
        run_callback(f, &prefix);
        true
    }

    /// The look of card key now, for a platform's paint (tests too).
    pub fn look(&self, key: u64) -> Option<Look> {
        let ui = lock(&self.ui);
        ui.wins.get(&key).map(look)
    }
}

fn look(cw: &CardWin) -> Look {
    Look {
        key: cw.key,
        title: cw.title.clone(),
        body: cw.body.clone(),
        kind: cw.kind,
        labels: cw.labels.clone(),
        lines: cw.lines,
        w: cw.w,
        h: cw.h,
        scale: cw.scale,
        radius: cw.radius,
        close: cw.close,
        buttons: cw.buttons.clone(),
        palette: cw.cfg.palette,
        hot: cw.hot,
        hovered: cw.hovered,
        clickable: cw.clickable,
    }
}

/// Sizes cw for its content at its scale: body lines (measured with the
/// real font, so it tracks DPI), height, the x and the buttons. Past two
/// lines the body is ellipsized: a card is a glance, not a paragraph.
fn measure(cw: &mut CardWin, surf: &mut dyn Surface) {
    let sc = cw.scale;
    cw.w = px(CARD_WIDTH_DIP, sc);
    cw.radius = px(cw.cfg.corner_radius, sc);
    cw.lines = 0;
    if !cw.body.is_empty() {
        let width = cw.w - px(TEXT_LEFT_DIP, sc) - px(TEXT_RIGHT_DIP, sc);
        cw.lines = surf
            .body_lines(&cw.body, width, sc)
            .clamp(1, MAX_BODY_LINES);
    }
    let widths: Option<Vec<i32>> = cw.labels.iter().map(|l| surf.label_width(l, sc)).collect();
    cw.h = px(card_height_dip(cw.lines, !cw.labels.is_empty()), sc);
    cw.close = close_rect(cw.w, sc);
    // A failed measure still leaves the buttons clickable.
    let widths = widths.unwrap_or_else(|| vec![0; cw.labels.len()]);
    cw.buttons = button_rects(cw.w, cw.h, &widths, sc);
}
