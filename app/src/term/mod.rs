//! Terminals (P10 Inline terminals): the Rune port of the React app's
//! components/term/terminal-section.tsx, terminal.tsx and lib/term.ts,
//! with suzuri's JS view (ui/view: frame.js, keys.js, kitty.js,
//! pointer.js, boxdraw.js) ported to Rust:
//!
//! - [`section`]: the card page's Terminal card: the card's shells on this
//!   device as tabs, New terminal (with the folder picker), the shown one.
//! - grid.rs: one terminal on screen (painting, keys, pointer, wheel).
//! - frame.rs, keys.rs, boxdraw.rs, folders.rs: the pure pieces.
//!
//! The shells belong to the Core (`core.terms()`, via
//! irori_window::rune_term), so leaving the page leaves them running and
//! coming back shows them again. Frames come from rsz-term's threads into
//! a [`Feed`]; one `AppHandle::update` at a time drains it on the UI
//! thread, paints, and acks the last frame (rsz-term keeps one frame in
//! flight until it is acked).

mod boxdraw;
mod folders;
mod frame;
mod grid;
mod keys;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rune::{El, Icon, TextEdit, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Select};

use irori_window::rune_term::{Link, Scope, TermId, TermInfo, TermMsg};
use irori_window::terms::Folder;

use crate::ui::{Event, Frame, State};

/// The size a shell starts at, before the view measures its box.
const START_COLS: u16 = 100;
const START_ROWS: u16 = 24;
/// The terminal box's height (terminal.tsx: h-[360px] min-h-[200px]);
/// the grip under it resizes it.
const HEIGHT: f32 = 360.0;
const MIN_HEIGHT: f32 = 200.0;

/// Lucide's square-terminal (the tabs' icon).
static SQUARE_TERMINAL: Icon = Icon {
    name: "lucide-square-terminal",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m7 11 2-2-2-2"/><path d="M11 13h4"/><rect width="18" height="18" x="3" y="3" rx="2" ry="2"/></g>"#,
};

pub struct Model {
    /// The card the section shows.
    pub card: Option<String>,
    /// The card's shells on this device; None while loading.
    pub terms: Option<Vec<TermInfo>>,
    pub shown: Option<TermId>,
    /// The folders offered for a new shell; None while loading.
    pub folders: Option<Vec<Folder>>,
    /// The New terminal row's folder (typed, or chosen from several).
    pub choice: TextEdit,
    pub picking: bool,
    pub select_open: bool,
    pub error: Option<String>,
    pub busy: bool,
    /// The shown terminal on screen.
    pub live: Option<Live>,
    pub height: f32,
    /// The terminal's keyboard: a hidden field on its cursor.
    pub sink: TextEdit,
    /// The headless demo: a canned frame, no shell.
    pub demo: bool,
}

impl Default for Model {
    fn default() -> Model {
        Model {
            card: None,
            terms: None,
            shown: None,
            folders: None,
            choice: TextEdit::new(""),
            picking: false,
            select_open: false,
            error: None,
            busy: false,
            live: None,
            height: HEIGHT,
            sink: TextEdit::new(""),
            demo: false,
        }
    }
}

/// What rsz-term's threads hand the window, drained on the UI thread.
#[derive(Default)]
pub struct Feed {
    /// False once the view let go: the callbacks answer it, and the pane
    /// forgets the view.
    alive: AtomicBool,
    /// An update is queued to drain this.
    scheduled: AtomicBool,
    queue: std::sync::Mutex<(Vec<Vec<u8>>, Vec<TermMsg>)>,
}

/// One terminal on screen.
#[derive(Default)]
pub struct Live {
    pub id: TermId,
    /// The attached feed; None while detached (the page went away).
    feed: Option<Arc<Feed>>,
    pub grid: frame::Grid,
    /// The size last sent (cols, rows), and whether the cell's pixels were.
    sent: (u16, u16),
    cell_sent: bool,
    pub title: String,
    /// The shell exited, with its code.
    pub exit: Option<Option<u32>>,
    /// A program's copy (OSC 52), offered, never written for it.
    pub clip: Option<String>,
    /// An attach or write that failed (the line under the box).
    pub error: Option<String>,
    /// A frame the view refused (the banner inside the box).
    pub frame_error: Option<String>,
    wheel: f32,
    /// The mouse button a program was told is down.
    held: Option<u8>,
    selecting: bool,
    has_selection: bool,
    last_cell: Option<(u16, u16)>,
    pub link: Option<Link>,
    focused: bool,
}

impl Live {
    fn new(id: TermId) -> Live {
        Live {
            id,
            ..Live::default()
        }
    }
}

fn theme_hex(s: &str) -> rune::Color {
    crate::ui::theme::hex(s)
}

// ---- the feed ----------------------------------------------------------

/// Attaches the view to terminal `id`: its cached state, then a full frame.
fn attach(s: &mut State, id: TermId) {
    release(s);
    let keep = s.term.live.take().filter(|l| l.id == id);
    let mut live = keep.unwrap_or_else(|| Live::new(id));
    live.cell_sent = false;
    live.sent = (0, 0);
    if s.term.demo {
        s.term.live = Some(live);
        return;
    }
    let Some(h) = s.handle.clone() else {
        s.term.live = Some(live);
        return;
    };
    let feed = Arc::new(Feed {
        alive: AtomicBool::new(true),
        ..Feed::default()
    });
    let schedule = {
        let feed = feed.clone();
        move || {
            if !feed.scheduled.swap(true, Ordering::AcqRel) {
                let feed = feed.clone();
                h.update(move |s: &mut State, _| drain(s, &feed));
            }
            feed.alive.load(Ordering::Acquire)
        }
    };
    let on_frame = {
        let (feed, schedule) = (feed.clone(), schedule.clone());
        move |b: Vec<u8>| {
            feed.queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .0
                .push(b);
            schedule()
        }
    };
    let on_msg = {
        let feed = feed.clone();
        move |m: TermMsg| {
            feed.queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .1
                .push(m);
            schedule()
        }
    };
    live.feed = Some(feed);
    s.term.live = Some(live);
    if let Err(e) = s.win().clone().term_attach(id, on_frame, on_msg)
        && let Some(l) = s.term.live.as_mut()
    {
        l.error = Some(e);
    }
}

/// Lets go of the shown terminal's feed (the shell keeps running).
fn release(s: &mut State) {
    let Some(l) = s.term.live.as_mut() else {
        return;
    };
    let id = l.id;
    let Some(feed) = l.feed.take() else { return };
    feed.alive.store(false, Ordering::Release);
    grid::blur(s, id);
    if let Some(w) = s.w.clone() {
        w.term_detach(id);
    }
}

/// Paints what the feed holds and acks the last frame.
fn drain(s: &mut State, feed: &Arc<Feed>) {
    feed.scheduled.store(false, Ordering::Release);
    let (frames, msgs) = std::mem::take(&mut *feed.queue.lock().unwrap_or_else(|e| e.into_inner()));
    let current = s
        .term
        .live
        .as_ref()
        .and_then(|l| l.feed.as_ref())
        .is_some_and(|f| Arc::ptr_eq(f, feed));
    if !current {
        return;
    }
    // The card page went away: let go until it shows again.
    if s.nav.card.is_none() || s.nav.card != s.term.card {
        release(s);
        return;
    }
    let Some(l) = s.term.live.as_mut() else {
        return;
    };
    let id = l.id;
    let mut info: Option<(String, bool)> = None;
    for m in msgs {
        match m {
            TermMsg::Title { title } => {
                l.title = title.clone();
                info = Some((title, l.exit.is_none()));
            }
            TermMsg::Exit { code } => {
                l.exit = Some(code);
                info = Some((l.title.clone(), false));
            }
            TermMsg::Clipboard { text } => l.clip = Some(text),
            // Bells, prompts, progress and images: not shown yet.
            _ => {}
        }
    }
    let mut seq = None;
    for f in &frames {
        match l.grid.apply(f) {
            Ok(h) => {
                seq = Some(h.seq);
                l.frame_error = None;
            }
            Err(e) => {
                // Refused, and nothing acked: suzuri-view drops the queue.
                l.frame_error = Some(e);
                seq = None;
                break;
            }
        }
    }
    if let Some((title, running)) = info
        && let Some(t) = s
            .term
            .terms
            .as_mut()
            .and_then(|ts| ts.iter_mut().find(|t| t.id == id))
    {
        t.title = title;
        t.running = running;
    }
    if let Some(seq) = seq {
        let _ = s.win().term_ack(id, seq);
    }
}

// ---- the section's actions ---------------------------------------------

/// Reads a card's shells and folders (the section's first load).
fn load(s: &mut State, card: &str) {
    release(s);
    s.term.card = Some(card.to_string());
    s.term.terms = None;
    s.term.shown = None;
    s.term.folders = None;
    s.term.choice = TextEdit::new("");
    s.term.picking = false;
    s.term.select_open = false;
    s.term.error = None;
    s.term.busy = false;
    s.term.live = None;
    s.term.demo = false;
    let ts = s.win().term_list(card);
    s.term.shown = ts.last().map(|t| t.id);
    s.term.terms = Some(ts);
    if let Some(id) = s.term.shown {
        attach(s, id);
    }
    let c = card.to_string();
    s.spawn(move |w| w.term_folders(&c), {
        let card = card.to_string();
        move |s, r: Result<Vec<Folder>, String>| {
            if s.term.card.as_deref() != Some(card.as_str()) {
                return;
            }
            match r {
                Ok(fs) => {
                    let kept = folders::remembered(&s.prefs, &card);
                    let pick = folders::pick(kept.as_deref(), &fs)
                        .or_else(|| fs.first().map(|f| f.path.clone()))
                        .unwrap_or_default();
                    s.term.choice = TextEdit::new(pick);
                    s.term.folders = Some(fs);
                }
                Err(e) => s.term.error = Some(e),
            }
        }
    });
}

/// Shows a tab.
fn show(s: &mut State, id: TermId) {
    if s.term.shown == Some(id) && s.term.live.as_ref().is_some_and(|l| l.feed.is_some()) {
        return;
    }
    s.term.shown = Some(id);
    attach(s, id);
}

/// Starts a shell in `cwd` (empty: the home folder).
fn start(s: &mut State, cwd: String) {
    let Some(card) = s.term.card.clone() else {
        return;
    };
    s.term.busy = true;
    s.term.error = None;
    let scope = Scope::Card(card.clone());
    let dir = (!cwd.is_empty()).then(|| cwd.clone());
    s.spawn(
        move |w| w.term_open(scope, START_COLS, START_ROWS, dir),
        move |s, r| {
            s.term.busy = false;
            if s.term.card.as_deref() != Some(card.as_str()) {
                return;
            }
            match r {
                Ok(t) => {
                    if !cwd.is_empty() {
                        folders::remember(&mut s.prefs, &card, &cwd);
                    }
                    let id = t.id;
                    s.term.terms.get_or_insert_with(Vec::new).push(t);
                    s.term.picking = false;
                    s.term.shown = Some(id);
                    attach(s, id);
                    grid::focus(s, id);
                }
                Err(e) => s.term.error = Some(e),
            }
        },
    );
}

/// New terminal: straight away when the folder is known, else the picker.
fn new_term(s: &mut State) {
    let Some(card) = s.term.card.clone() else {
        return;
    };
    let kept = folders::remembered(&s.prefs, &card);
    match s
        .term
        .folders
        .as_deref()
        .and_then(|fs| folders::pick(kept.as_deref(), fs))
    {
        Some(p) => start(s, p),
        None => s.term.picking = true,
    }
}

/// Ends a shell and drops its tab.
fn close(s: &mut State, id: TermId) {
    if s.term.live.as_ref().is_some_and(|l| l.id == id) {
        release(s);
        s.term.live = None;
    }
    s.spawn(
        move |w| w.term_close(id),
        move |s, _| {
            let Some(ts) = s.term.terms.as_mut() else {
                return;
            };
            ts.retain(|t| t.id != id);
            if s.term.shown == Some(id) {
                let next = ts.last().map(|t| t.id);
                s.term.shown = next;
                if let Some(n) = next {
                    attach(s, n);
                }
            }
        },
    );
}

/// Restart: end the exited shell, start one in the same folder.
fn restart(s: &mut State, id: TermId) {
    let cwd = s
        .term
        .terms
        .as_ref()
        .and_then(|ts| ts.iter().find(|t| t.id == id))
        .map(|t| t.cwd.clone())
        .unwrap_or_default();
    close(s, id);
    start(s, cwd);
}

/// A terminal opened for a card elsewhere (an agent's Attach, card #456,
/// widgets::spawned_actions; the React app's `irori:term-opened` event):
/// its tab, shown.
pub fn opened(s: &mut State, card_id: &str, info: TermInfo) {
    let card = card_id.trim_start_matches("task:");
    if s.term.card.as_deref() != Some(card) {
        return;
    }
    let id = info.id;
    let ts = s.term.terms.get_or_insert_with(Vec::new);
    if !ts.iter().any(|t| t.id == id) {
        ts.push(info);
    }
    show(s, id);
}

// ---- the package contract ----------------------------------------------

/// The card's Terminal section as a page of its own (the card page embeds
/// [`section`]).
#[allow(dead_code)] // The package contract; the card page embeds `section`.
pub fn view(s: &State, f: &mut Frame) -> El {
    match s.nav.card.clone() {
        Some(id) => col()
            .key("term-page")
            .w_full()
            .padding(20.0)
            .child(section(s, f, &id)),
        None => crate::ui::not_yet(f, "Terminals", "Open a card to see its terminals."),
    }
}

pub fn on_event(s: &mut State, ev: &Event) {
    if ev.is("app:card-opened") {
        let Some(card) = s.nav.card.clone() else {
            return;
        };
        let supported = s.w.as_ref().is_some_and(|w| w.core.terms().supported());
        if supported && s.term.card.as_deref() != Some(card.as_str()) {
            load(s, &card);
        }
    }
}

/// The headless scene: the first card with a shell, else the newest card
/// with a demo terminal painted from a canned frame.
pub fn scene(s: &mut State) {
    let with_shell = s
        .data
        .cards
        .iter()
        .find(|c| !s.win().term_list(&c.id).is_empty())
        .map(|c| c.id.clone());
    if let Some(id) = with_shell {
        s.open_card(&id);
        return;
    }
    let Some(id) = s.data.cards.first().map(|c| c.id.clone()) else {
        return;
    };
    s.open_card(&id);
    release(s);
    s.term.card = Some(id.clone());
    s.term.demo = true;
    let info = TermInfo {
        id: 1,
        scope: Scope::Card(id),
        cwd: "/Users/stephen/projects/irori".into(),
        title: "zsh".into(),
        running: true,
    };
    let mut live = Live::new(1);
    let _ = live.grid.apply(&frame::demo_frame(START_COLS, START_ROWS));
    live.title = "zsh".into();
    s.term.terms = Some(vec![info]);
    s.term.shown = Some(1);
    s.term.live = Some(live);
    s.term.sink.focused = true;
}

// ---- the section -------------------------------------------------------

fn small(_f: &Frame) -> rune::TextStyle {
    rune::tok::TextStyle::new(12.5, 400.0)
}

/// The card page's Terminal card (card-detail.tsx's TerminalCard around
/// terminal-section.tsx): nothing on a device that can't run shells.
pub fn section(s: &State, f: &mut Frame, card_id: &str) -> El {
    let k = f.k;
    let card = card_id.trim_start_matches("task:");
    if !s.w.as_ref().is_some_and(|w| w.core.terms().supported()) && !s.term.demo {
        return el().key("term-none");
    }
    let mut body = col().key("term-section").w_full().gap(8.0);
    if s.term.card.as_deref() == Some(card) {
        body = body.child(tabs(s, f));
        if s.term.picking && s.term.folders.is_some() {
            body = body.child(picker(s, f));
        }
        if let Some(l) = s.term.live.as_ref().filter(|l| Some(l.id) == s.term.shown) {
            body = body.child(terminal(s, f, l));
        }
        if s.term.terms.as_ref().is_some_and(|t| t.is_empty()) && !s.term.picking {
            body = body.child(
                text("No shell for this card on this device.")
                    .font(small(f))
                    .color(k.c.text_muted),
            );
        }
        if let Some(e) = &s.term.error {
            body = body.child(
                text(e.clone())
                    .font(small(f))
                    .color(k.c.danger.fg)
                    .w_full()
                    .wrap(),
            );
        }
    }
    col()
        .key("term-card")
        .w_full()
        .radius(10.0)
        .border(1.0, k.c.border_base)
        .fill(k.c.layer_01)
        .px(20.0)
        .py(14.0)
        .gap(10.0)
        .child(
            text("TERMINAL")
                .font(rune::tok::TextStyle::new(11.0, 500.0))
                .color(k.c.text_muted),
        )
        .child(body)
}

/// The tabs and New terminal.
fn tabs(s: &State, f: &Frame) -> El {
    let k = f.k;
    let mut r = row()
        .key("term-tabs")
        .w_full()
        .gap(4.0)
        .center_y()
        .flex_wrap();
    for (i, t) in s.term.terms.iter().flatten().enumerate() {
        let id = t.id;
        let on = s.term.shown == Some(id);
        let tip = if t.title.is_empty() {
            t.cwd.clone()
        } else {
            format!("{}\n{}", t.title, t.cwd)
        };
        let ink = if on { k.c.text_base } else { k.c.text_muted };
        let name = row()
            .key(format!("term-tab-{id}"))
            .gap(6.0)
            .center_y()
            .px(8.0)
            .py(4.0)
            .max_w(224.0)
            .child(icon(&SQUARE_TERMINAL, 14.0).color(ink))
            .child(
                text(folders::tab_name(t, i))
                    .font(rune::tok::TextStyle::new(12.0, 400.0))
                    .color(ink)
                    .truncate()
                    .shrink(),
            )
            .help(tip)
            .on_press(move |s: &mut State| show(s, id));
        let x = el()
            .key(format!("term-tab-close-{id}"))
            .px(4.0)
            .py(4.0)
            .child(icon(&crate::ui::icons::X, 12.0).color(k.c.text_muted))
            .help("End this shell")
            .label("End this shell")
            .on_press(move |s: &mut State| close(s, id));
        let mut tab = row()
            .key(format!("term-tabbox-{id}"))
            .center_y()
            .radius(6.0)
            .border(
                1.0,
                if on {
                    k.c.border_base
                } else {
                    rune::Color::TRANSPARENT
                },
            )
            .child(name)
            .child(x);
        if on {
            tab = tab.fill(k.c.layer_02);
        }
        r = r.child(tab);
    }
    r.child(
        Button::new("New terminal")
            .variant(ButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .icon(&crate::ui::icons::PLUS)
            .disabled(s.term.busy || s.term.folders.is_none())
            .on_press(new_term)
            .build(k)
            .key("term-new"),
    )
}

/// The folder picker: a select when several are offered, else a path field.
fn picker(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let folders = s.term.folders.as_deref().unwrap_or_default();
    let choice = s.term.choice.text.trim().to_string();
    let field = if folders.len() > 1 {
        let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
        Select::new("term-folder", &paths)
            .current(Some(choice.as_str()))
            .label("Folder")
            .width(420.0)
            .open(s.term.select_open)
            .on_toggle(|s: &mut State| s.term.select_open = !s.term.select_open)
            .on_close(|s: &mut State| s.term.select_open = false)
            .on_choose(|s: &mut State, v: &str, _: &mut Tx| {
                s.term.choice = TextEdit::new(v);
                s.term.select_open = false;
            })
            .build(k, f.layer)
    } else {
        rune_kit::TextInput::new(&s.term.choice)
            .placeholder("No checkout of this card's repo here: a folder path")
            .label("Folder")
            .text_style(
                rune::tok::TextStyle::new(12.5, 400.0).family(rune::tok::Family::SystemMono),
            )
            .width(Some(448.0))
            .on_edit(|s: &mut State| &mut s.term.choice)
            .on_submit(|s: &mut State, _: &mut Tx| {
                let c = s.term.choice.text.trim().to_string();
                if !c.is_empty() {
                    start(s, c);
                }
            })
            .build(k)
            .key("term-folder-field")
    };
    row()
        .key("term-picker")
        .w_full()
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(field)
        .child(
            Button::new("Open")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .disabled(s.term.busy || choice.is_empty())
                .on_press(|s: &mut State| {
                    let c = s.term.choice.text.trim().to_string();
                    start(s, c);
                })
                .build(k)
                .key("term-open"),
        )
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut State| s.term.picking = false)
                .build(k),
        )
}

/// One terminal: its box, the resize grip, and the lines under it (a
/// program's copy offered, the exit and Restart, an error).
fn terminal(s: &State, f: &mut Frame, l: &Live) -> El {
    let k = f.k;
    let id = l.id;
    let cwd = s.term.terms.iter().flatten().find(|t| t.id == id).map(|t| t.cwd.clone()).unwrap_or_default();
    let mut c = col()
        .key(format!("term-view-{id}"))
        .w_full()
        .gap(8.0)
        .label(format!("Terminal in {cwd}"))
        .child(grid::view(s, f, l, s.term.height))
        .child(
            el().key("term-grip")
                .w_full()
                .h(6.0)
                .mt(-8.0)
                .cursor(rune::Cursor::ResizeRow)
                .help("Drag to resize")
                .on_drag(|s: &mut State, d: &mut rune::Drag, _: &mut Tx| {
                    if d.phase == rune::Phase::Down {
                        d.data = s.term.height;
                    }
                    s.term.height = (d.data + d.dy).clamp(MIN_HEIGHT, 1600.0);
                }),
        );
    let line = |key: &str| row().key(key.to_string()).w_full().gap(8.0).center_y();
    if let Some(clip) = &l.clip {
        let copy = clip.clone();
        c = c.child(
            line("term-clip")
                .child(
                    row()
                        .shrink()
                        .min_w(0.0)
                        .child(
                            text("The program wants to copy: ")
                                .font(small(f))
                                .color(k.c.text_muted),
                        )
                        .child(
                            text(clip.clone())
                                .font(small(f).family(rune::tok::Family::SystemMono))
                                .color(k.c.text_muted)
                                .truncate()
                                .shrink(),
                        ),
                )
                .child(
                    Button::new("Copy")
                        .variant(ButtonVariant::Outline)
                        .control_size(ControlSize::Mini)
                        .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                            tx.copy(copy.clone());
                            if let Some(l) = s.term.live.as_mut() {
                                l.clip = None;
                            }
                        })
                        .build(k),
                )
                .child(
                    Button::new("Dismiss")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Mini)
                        .on_press(|s: &mut State| {
                            if let Some(l) = s.term.live.as_mut() {
                                l.clip = None;
                            }
                        })
                        .build(k),
                ),
        );
    }
    if let Some(code) = l.exit {
        let what = match code {
            Some(n) => format!("The shell ended (exit {n})."),
            None => "The shell ended.".to_string(),
        };
        c = c.child(
            line("term-exit")
                .child(text(what).font(small(f)).color(k.c.text_muted))
                .child(
                    Button::new("Restart")
                        .variant(ButtonVariant::Outline)
                        .control_size(ControlSize::Mini)
                        .on_press(move |s: &mut State| restart(s, id))
                        .build(k),
                ),
        );
    }
    if let Some(e) = &l.error {
        c = c.child(
            text(e.clone())
                .font(small(f))
                .color(k.c.danger.fg)
                .w_full()
                .wrap(),
        );
    }
    c
}
