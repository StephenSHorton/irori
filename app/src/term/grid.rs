//! One terminal on screen (the React app's term/terminal.tsx over
//! suzuri-view's `index.js`, `frame.js` painting and `pointer.js`): the
//! cell grid as Rune elements, the cursor, the keyboard, the pointer and
//! the wheel. Frames arrive through [`super::Live`]; this paints what they
//! left and turns input into `Window::term_*` calls.
//!
//! Painting: each row is one memoised subtree (rebuilt only when a frame
//! touched it): background runs as rects, ASCII as one mono `rich` text
//! with a span per run, other glyphs one per cell so a fallback font can't
//! push the grid out of line, and box drawing as rects (boxdraw.rs).

use std::cell::Cell as StdCell;

use rune::{Color, Cursor, El, PointerEvent, Span, Tx, WheelEvent, el, memo, rich, text, text_input};

use irori_window::rune_term::{Mouse, MouseKind, SelectKind};

use super::frame::{
    self, A_BOLD, A_DIM, A_HIDDEN, A_ITALIC, A_SELECTED, A_STRIKE, A_UNDERLINE, Cell, DEFAULT_BG,
    Grid, M_ALT_SCREEN, M_ALT_SCROLL, M_APP_CURSOR, M_FOCUS, M_MOUSE_MOTION, MOUSE_MODES,
};
use super::keys::{self, Clip, KeyIn};
use super::{Live, boxdraw};
use crate::ui::{Frame, State};

/// The terminal's own look (terminal.tsx's setStyle): it stays dark in
/// both themes, as the React app's did.
pub const BG: &str = "#0c0c0c";
const SELECTION: (u32, f32) = (0xb8a0c8, 0.38);
const CURSOR: u32 = 0xb8a0c8;
/// The padding around the grid (suzuri-view's default [6, 4]).
pub const PAD_X: f32 = 6.0;
pub const PAD_Y: f32 = 4.0;
const FONT_PX: f32 = 13.0;
const LINE: f32 = 16.0;

fn is_mac() -> bool {
    cfg!(target_os = "macos")
}

/// The mono style every cell is set in.
pub fn style() -> rune::TextStyle {
    let mut st = rune::tok::TextStyle::new(FONT_PX, 400.0).family(rune::tok::Family::SystemMono);
    st.line_height = LINE;
    st
}

thread_local! {
    static CELL_W: StdCell<f32> = const { StdCell::new(0.0) };
}

/// The cell's size in points: the mono advance (measured once) and the
/// line height.
pub fn metrics() -> (f32, f32) {
    let w = CELL_W.with(|c| {
        if c.get() <= 0.0 {
            let (_, max) = rune::text::content_widths("MMMMMMMMMM", &[], style());
            let w = if max > 0.0 { max / 10.0 } else { FONT_PX * 0.6 };
            c.set(w);
        }
        c.get()
    });
    (w, LINE)
}

/// A frame colour (0xRRGGBB) as a Rune colour.
fn rgb(v: u32) -> Color {
    rune::color::hex(&format!("#{:06x}", v & 0xff_ffff)).unwrap_or(Color::WHITE)
}

/// The cell under a point in the box's own points: (col, row, right half,
/// which edge it is past: -1 above, 1 below).
fn cell_at(g: &Grid, x: f32, y: f32) -> (u16, u16, bool, i32) {
    let (cw, ch) = metrics();
    let cols = g.cols.max(1);
    let rows = g.rows.max(1);
    let fx = (x - PAD_X) / cw;
    let fy = (y - PAD_Y) / ch;
    let col = (fx.floor().max(0.0) as u16).min(cols - 1);
    let row = (fy.floor().max(0.0) as u16).min(rows - 1);
    let dy = if fy < 0.0 {
        -1
    } else if fy >= rows as f32 {
        1
    } else {
        0
    };
    let right = fx - fx.floor() >= 0.5 || fx >= cols as f32;
    (col, row, right, dy)
}

// ---- painting ----------------------------------------------------------

/// One row's subtree: rebuilt only when its version changes.
fn row_el(key: &str, cells: &[Cell], y: u16) -> El {
    let (cw, ch) = metrics();
    let mut row = el()
        .key(key.to_string())
        .absolute(PAD_X, PAD_Y + y as f32 * ch)
        .w(cells.len() as f32 * cw)
        .h(ch);
    // Backgrounds and the selection wash, one rect per run.
    let mut x = 0;
    while x < cells.len() {
        let c = cells[x];
        let mut j = x + 1;
        while j < cells.len() && cells[j].bg == c.bg && (cells[j].attrs & A_SELECTED) == (c.attrs & A_SELECTED) {
            j += 1;
        }
        let (rx, rw) = (x as f32 * cw, (j - x) as f32 * cw);
        if c.bg != DEFAULT_BG {
            row = row.child(el().absolute(rx, 0.0).w(rw).h(ch).fill(rgb(c.bg)));
        }
        if c.attrs & A_SELECTED != 0 {
            row = row.child(el().absolute(rx, 0.0).w(rw).h(ch).fill(rgb(SELECTION.0).alpha(SELECTION.1)));
        }
        x = j;
    }
    // ASCII as one string with a span per run; the rest per cell.
    let mut s = String::with_capacity(cells.len());
    let mut spans: Vec<Span> = Vec::new();
    let mut overlays: Vec<El> = Vec::new();
    let mut lines: Vec<El> = Vec::new();
    for (i, c) in cells.iter().enumerate() {
        let hidden = c.attrs & A_HIDDEN != 0;
        let ascii = (0x21..0x7f).contains(&c.cp);
        let at = s.len();
        s.push(if ascii && !hidden { c.cp as u8 as char } else { ' ' });
        let fg = fg_of(c);
        if ascii && !hidden {
            let mut sp = Span::new(at, at + 1);
            sp.color = Some(fg);
            if c.attrs & A_BOLD != 0 {
                sp.weight = Some(700.0);
            }
            sp.italic = c.attrs & A_ITALIC != 0;
            match spans.last_mut() {
                Some(last) if last.end as usize == at && last.color == sp.color && last.weight == sp.weight && last.italic == sp.italic => last.end += 1,
                _ => spans.push(sp),
            }
        } else if !hidden && c.cp > 0x7f {
            let x = i as f32 * cw;
            if let Some(rects) = boxdraw::glyph(c.cp, cw, ch) {
                for ([rx, ry, rw, rh], a) in rects {
                    overlays.push(el().absolute(x + rx, ry).w(rw).h(rh).fill(fg.alpha(fg.a * a)));
                }
            } else if let Some(g) = char::from_u32(c.cp) {
                overlays.push(text(g.to_string()).font(glyph_style(c)).color(fg).absolute(x, 0.0).h(ch));
            }
        }
        if !hidden && c.attrs & (A_UNDERLINE | A_STRIKE) != 0 && c.cp != 0 {
            let ly = if c.attrs & A_UNDERLINE != 0 { ch - 2.0 } else { (ch / 2.0).floor() };
            lines.push(el().absolute(i as f32 * cw, ly).w(cw).h(1.0).fill(fg));
        }
    }
    let lead = s.len() - s.trim_start().len();
    let trimmed = s.trim();
    if !trimmed.is_empty() {
        let end = lead + trimmed.len();
        let spans: Vec<Span> = spans
            .into_iter()
            .filter(|sp| (sp.start as usize) < end && sp.end as usize > lead)
            .map(|mut sp| {
                sp.start = sp.start.max(lead as u32) - lead as u32;
                sp.end = sp.end.min(end as u32) - lead as u32;
                sp
            })
            .collect();
        row = row.child(
            rich(trimmed.to_string(), spans)
                .font(style())
                .color(rgb(0xd0d0d8))
                .absolute(lead as f32 * cw, 0.0)
                .w((end - lead) as f32 * cw + cw)
                .h(ch),
        );
    }
    for o in overlays {
        row = row.child(o);
    }
    for l in lines {
        row = row.child(l);
    }
    row
}

fn glyph_style(c: &Cell) -> rune::TextStyle {
    let mut st = style();
    if c.attrs & A_BOLD != 0 {
        st.weight = 700.0;
    }
    st.italic = c.attrs & A_ITALIC != 0;
    st
}

fn fg_of(c: &Cell) -> Color {
    let fg = rgb(c.fg);
    if c.attrs & A_DIM != 0 { fg.alpha(0.6) } else { fg }
}

/// The cursor: a block with its glyph redrawn on top, an underline, a beam
/// or a hollow box (suzuri-view's .cursor classes).
fn cursor_el(g: &Grid) -> Option<El> {
    let h = g.head;
    if h.shape == 0 || h.cx >= g.cols || h.cy >= g.rows {
        return None;
    }
    let (cw, ch) = metrics();
    let (x, y) = (PAD_X + h.cx as f32 * cw, PAD_Y + h.cy as f32 * ch);
    let ink = rgb(CURSOR);
    let mut c = el().key("term-cursor").absolute(x, y);
    c = match h.shape {
        2 => c.w(cw).h(2.0).absolute(x, y + ch - 2.0).fill(ink),
        3 => c.w(2.0).h(ch).fill(ink),
        4 => c.w(cw).h(ch).border(1.0, ink),
        _ => {
            let cell = g.cell(h.cx, h.cy);
            let mut b = c.w(cw).h(ch).fill(ink.alpha(0.85));
            if let Some(gl) = char::from_u32(cell.cp).filter(|g| !g.is_whitespace() && cell.cp != 0) {
                b = b.child(text(gl.to_string()).font(glyph_style(&cell)).color(super::theme_hex(BG)).absolute(0.0, 0.0));
            }
            b
        }
    };
    Some(c)
}

/// The terminal's box: the grid, cursor, badge, error, link hover and the
/// keyboard's hidden field, with the pointer, wheel and resize wired.
pub fn view(s: &State, f: &mut Frame, live: &Live, height: f32) -> El {
    let k = f.k;
    let g = &live.grid;
    let id = live.id;
    let (cw, ch) = metrics();
    let bg = super::theme_hex(BG);
    let mut b = el()
        .key(format!("term-box-{id}"))
        .w_full()
        .h(height)
        .min_h(0.0)
        .fill(bg)
        .radius(6.0)
        .border(1.0, if s.term.sink.focused { k.c.border_strong } else { k.c.border_base })
        .clip_rounded(6.0)
        .cursor(if live.link.is_some() { Cursor::Pointer } else { Cursor::Text })
        .preserve_focus_on_press(true)
        .on_layout_tx(move |s: &mut State, r: [f32; 4], tx: &mut Tx| fit(s, id, r, tx))
        .on_drag(move |s: &mut State, d: &mut rune::Drag, tx: &mut Tx| drag(s, id, d, tx))
        .on_secondary_press(move |s: &mut State, _, tx: &mut Tx| paste(s, id, tx))
        .on_pointer_move_local(move |s: &mut State, p: PointerEvent, tx: &mut Tx| hover(s, id, p, tx))
        .on_wheel(move |s: &mut State, w: WheelEvent, tx: &mut Tx| wheel(s, id, w, tx));
    for y in 0..g.rows {
        let key = format!("term-row-{id}-{y}");
        let ver = (g.ver[y as usize], cw.to_bits(), g.cols);
        b = b.child(memo(key.clone(), ver, || row_el(&key, g.row(y), y)));
    }
    if let Some(l) = &live.link {
        let mod_key = if is_mac() { "⌘" } else { "Ctrl" };
        b = b.child(
            el().key("term-link")
                .absolute(PAD_X + l.x0 as f32 * cw, PAD_Y + l.row as f32 * ch)
                .w((l.x1.saturating_sub(l.x0)) as f32 * cw)
                .h(ch)
                .fill(rgb(0xb8a0c8).alpha(0.12))
                .child(el().absolute(0.0, ch - 1.0).w_full().h(1.0).fill(rgb(0xb8a0c8)))
                .help(format!("{}\n{mod_key}+click to open", l.url)),
        );
    }
    if let Some(c) = cursor_el(g) {
        b = b.child(c);
    }
    // The keyboard: a hidden field on the cursor, as suzuri-view's
    // textarea, so keys, paste and the input method land here.
    let (cx, cy) = (PAD_X + g.head.cx as f32 * cw, PAD_Y + g.head.cy as f32 * ch);
    b = b.child(
        text_input(&s.term.sink, style())
            .key("term-sink")
            .absolute(cx, cy)
            .w(2.0)
            .h(ch)
            .opacity(0.0)
            .on_edit(|s: &mut State| &mut s.term.sink)
            .on_input_key(move |s: &mut State, ev: &rune::KeyEvent, tx: &mut Tx| key(s, id, ev, tx))
            .on_input_paste(move |s: &mut State, c: &rune::ClipboardContent, _: &mut Tx| {
                let t = c.text().to_string();
                if !t.is_empty() {
                    paste_text(s, id, &t);
                }
                true
            })
            .on_input_change(move |s: &mut State, _: &mut Tx| typed(s, id)),
    );
    // A composition under way shows at the cursor (suzuri's preedit).
    if let Some((p, _)) = &s.term.sink.preedit
        && !p.is_empty()
    {
        b = b.child(
            text(p.clone())
                .font(style())
                .color(super::theme_hex("#e8e8ec"))
                .fill(super::theme_hex("#16161c"))
                .absolute(cx, cy)
                .h(ch),
        );
    }
    if g.head.offset > 0 {
        b = b.child(
            el().absolute(0.0, 6.0).w_full().child(
                rune::row().w_full().child(el().grow(1.0)).child(
                    text(format!("↑ {} / {}", g.head.offset, g.head.history))
                        .font(rune::tok::TextStyle::new(10.0, 400.0))
                        .color(super::theme_hex("#e8e0f0"))
                        .fill(rgb(0xb8a0c8).alpha(0.25))
                        .radius(4.0)
                        .px(6.0).py(1.0),
                )
                .child(el().w(10.0)),
            ),
        );
    }
    if let Some(e) = &live.frame_error {
        b = b.child(
            el().absolute(PAD_X, (height - PAD_Y - 26.0).max(0.0)).w_full().pr(2.0 * PAD_X).child(
                text(e.clone())
                    .font(rune::tok::TextStyle::new(12.0, 400.0))
                    .color(Color::WHITE)
                    .fill(rgb(0xa03c3c).alpha(0.85))
                    .radius(4.0)
                    .px(8.0).py(4.0)
                    .w_full()
                    .wrap(),
            ),
        );
    }
    b
}

// ---- input -------------------------------------------------------------

/// The shown terminal, if it is `id`.
fn live(s: &mut State, id: u32) -> Option<&mut Live> {
    s.term.live.as_mut().filter(|l| l.id == id)
}

/// Sends what the person typed, unless the shell has exited.
pub fn write(s: &mut State, id: u32, data: &str) {
    let Some(l) = live(s, id) else { return };
    if l.exit.is_some() || data.is_empty() {
        return;
    }
    if let Err(e) = s.win().clone().term_write(id, data.as_bytes()) {
        if let Some(l) = live(s, id) {
            l.error = Some(e);
        }
    }
}

fn paste_text(s: &mut State, id: u32, t: &str) {
    if live(s, id).is_none_or(|l| l.exit.is_some()) {
        return;
    }
    if let Err(e) = s.win().clone().term_paste(id, t) {
        if let Some(l) = live(s, id) {
            l.error = Some(e);
        }
    }
}

/// Reads the clipboard off the event thread, then pastes it.
fn paste(s: &mut State, id: u32, tx: &mut Tx) {
    if live(s, id).is_none_or(|l| l.exit.is_some()) {
        return;
    }
    tx.read_clipboard(move |s: &mut State, c: &rune::ClipboardContent, _: &mut Tx| {
        let t = c.text().to_string();
        if !t.is_empty() {
            paste_text(s, id, &t);
        }
    });
}

/// Copies the selection; false when there was none.
fn copy(s: &mut State, id: u32, tx: &mut Tx) -> bool {
    match s.win().term_selection(id) {
        Some(t) if !t.is_empty() => {
            tx.copy(t);
            true
        }
        _ => false,
    }
}

/// A key in the terminal's field: the copy and paste chords, then the
/// encoder. False leaves the key to the field (the input method) and the
/// window (⌘ shortcuts).
fn key(s: &mut State, id: u32, ev: &rune::KeyEvent, tx: &mut Tx) -> bool {
    let mac = is_mac();
    let Some(k) = KeyIn::from_rune(ev, tx.key_base(), mac) else {
        return false;
    };
    let Some(l) = live(s, id) else { return false };
    let (modes, exited, sel) = (l.grid.head.modes, l.exit.is_some(), l.has_selection);
    if let Some(c) = keys::clip_shortcut(&k, mac, sel) {
        match c {
            Clip::Copy => {
                copy(s, id, tx);
            }
            Clip::Paste => paste(s, id, tx),
            Clip::CopyOrInterrupt => {
                if !copy(s, id, tx) {
                    write(s, id, "\x03");
                }
            }
        }
        return true;
    }
    let Some(data) = keys::encode(&k, modes, mac) else {
        return false;
    };
    // An encodable key is the terminal's even after the exit, when
    // nothing is sent (no Tab moving focus away).
    if !exited {
        if let Some(l) = live(s, id)
            && l.has_selection
        {
            l.has_selection = false;
        }
        write(s, id, &data);
    }
    true
}

/// Text the field took that no key sent: an input method's commit or a
/// dead key's character.
fn typed(s: &mut State, id: u32) {
    if s.term.sink.preedit.is_some() || s.term.sink.text.is_empty() {
        return;
    }
    let t = std::mem::take(&mut s.term.sink.text);
    s.term.sink.clear();
    write(s, id, &t);
}

/// Gives the terminal the keyboard and tells the program (focus reports).
pub fn focus(s: &mut State, id: u32) {
    if !s.term.sink.focused {
        s.term.sink.focused = true;
    }
    let Some(l) = live(s, id) else { return };
    if l.focused {
        return;
    }
    l.focused = true;
    let report = l.grid.head.modes & M_FOCUS != 0 && l.exit.is_none();
    let w = s.win().clone();
    let _ = w.term_focus(id, true);
    if report {
        write(s, id, "\x1b[I");
    }
}

/// The program asked for the mouse (and Shift isn't bypassing it).
fn app_mouse(l: &Live, shift: bool) -> bool {
    l.grid.head.modes & MOUSE_MODES != 0 && !shift
}

fn report(w: &irori_window::Window, id: u32, kind: MouseKind, button: u8, col: u16, row: u16, m: rune::Mods) {
    w.term_mouse(
        id,
        Mouse {
            kind,
            button,
            col,
            row,
            shift: m.shift,
            alt: m.alt,
            ctrl: m.ctrl,
        },
    );
}

/// Press, drag and release on the grid: links, mouse reports and the
/// selection (pointer.js).
fn drag(s: &mut State, id: u32, d: &mut rune::Drag, tx: &mut Tx) {
    let m = tx.mods();
    if d.phase == rune::Phase::Down {
        focus(s, id);
    }
    let w = s.win().clone();
    let Some(l) = live(s, id) else { return };
    let (col, row, right, dy) = cell_at(&l.grid, d.x, d.y);
    match d.phase {
        rune::Phase::Down => {
            if l.exit.is_some() {
                return;
            }
            let link_mod = if is_mac() { m.cmd } else { m.ctrl };
            if link_mod
                && let Some(lk) = &l.link
                && lk.row == row
                && col >= lk.x0
                && col < lk.x1
            {
                let _ = tx.open_url(&lk.url.clone());
                return;
            }
            if app_mouse(l, m.shift) {
                l.held = Some(0);
                l.last_cell = Some((col, row));
                report(&w, id, MouseKind::Press, 0, col, row, m);
                return;
            }
            let extend = m.shift && l.has_selection;
            let res = if extend {
                w.term_select_extend(id, col, row, right)
            } else {
                let kind = match d.clicks {
                    0 | 1 => SelectKind::Cell,
                    2 => SelectKind::Word,
                    _ => SelectKind::Line,
                };
                w.term_select_start(id, col, row, right, kind)
            };
            l.has_selection = extend || d.clicks >= 2;
            l.selecting = res.is_ok();
        }
        rune::Phase::Move => {
            if l.selecting {
                let _ = w.term_select_extend(id, col, row, right);
                l.has_selection = true;
                // Past the top or bottom edge: scroll the history and keep
                // extending (one line per move; suzuri-view repeats on a timer).
                if dy != 0 {
                    let _ = w.term_scroll(id, -dy);
                    let _ = w.term_select_extend(id, col, row, right);
                }
                tx.live_only();
            } else if let Some(b) = l.held {
                if l.last_cell != Some((col, row)) {
                    l.last_cell = Some((col, row));
                    report(&w, id, MouseKind::Motion(Some(b)), 0, col, row, m);
                }
                tx.live_only();
            }
        }
        rune::Phase::Up => {
            l.selecting = false;
            if let Some(b) = l.held.take() {
                report(&w, id, MouseKind::Release, b, col, row, m);
            }
        }
    }
}

/// The pointer moving over the grid: motion reports, and the link under
/// it (underlined, ⌘/Ctrl-click opens it).
fn hover(s: &mut State, id: u32, p: PointerEvent, tx: &mut Tx) {
    let m = tx.mods();
    let w = s.win().clone();
    let Some(l) = live(s, id) else {
        tx.live_only();
        return;
    };
    if l.selecting || l.held.is_some() {
        tx.live_only();
        return;
    }
    let (col, row, _, _) = cell_at(&l.grid, p.x, p.y);
    if l.last_cell == Some((col, row)) {
        tx.live_only();
        return;
    }
    l.last_cell = Some((col, row));
    if l.grid.head.modes & M_MOUSE_MOTION != 0 && !m.shift {
        report(&w, id, MouseKind::Motion(None), 0, col, row, m);
    }
    let link = s.win().term_link_at(id, col, row);
    let Some(l) = live(s, id) else { return };
    if l.link == link {
        tx.live_only();
        return;
    }
    l.link = link;
}

/// The wheel: to the program when it reports the mouse, arrow keys on an
/// alt screen that asked for them, else the history.
fn wheel(s: &mut State, id: u32, ev: WheelEvent, tx: &mut Tx) -> bool {
    let w = s.win().clone();
    let (_, ch) = metrics();
    let Some(l) = live(s, id) else { return false };
    l.wheel += ev.dy / ch;
    let lines = l.wheel.trunc() as i32;
    tx.live_only();
    if lines == 0 {
        return true;
    }
    l.wheel -= lines as f32;
    let modes = l.grid.head.modes;
    let (col, row, _, _) = cell_at(&l.grid, ev.x, ev.y);
    if app_mouse(l, ev.mods.shift) {
        // Content moving down (dy > 0) is the wheel turning up.
        let kind = if lines > 0 { MouseKind::WheelUp } else { MouseKind::WheelDown };
        for _ in 0..lines.unsigned_abs().min(32) {
            report(&w, id, kind, 0, col, row, ev.mods);
        }
        return true;
    }
    if modes & M_ALT_SCREEN != 0 {
        if modes & M_ALT_SCROLL != 0 {
            let lead = if modes & M_APP_CURSOR != 0 { "\x1bO" } else { "\x1b[" };
            let arrow = if lines > 0 { "A" } else { "B" };
            write(s, id, &format!("{lead}{arrow}").repeat(lines.unsigned_abs() as usize));
        }
        return true;
    }
    let _ = w.term_scroll(id, lines);
    true
}

/// The box was laid out: fit the grid to it (resize), and attach again if
/// the page came back after a detach.
fn fit(s: &mut State, id: u32, r: [f32; 4], tx: &mut Tx) {
    let (cw, ch) = metrics();
    let cols = (((r[2] - 2.0 * PAD_X) / cw).floor() as i32).max(2) as u16;
    let rows = (((r[3] - 2.0 * PAD_Y) / ch).floor() as i32).max(1) as u16;
    if s.term.live.as_ref().is_some_and(|l| l.id == id && l.feed.is_none() && !s.term.demo) {
        super::attach(s, id);
    }
    let demo = s.term.demo;
    let Some(l) = live(s, id) else {
        tx.live_only();
        return;
    };
    if l.sent == (cols, rows) && l.cell_sent {
        tx.live_only();
        return;
    }
    if demo {
        l.sent = (cols, rows);
        l.cell_sent = true;
        l.grid = Grid::default();
        let _ = l.grid.apply(&frame::demo_frame(cols, rows));
        return;
    }
    let first_cell = !l.cell_sent;
    l.sent = (cols, rows);
    l.cell_sent = true;
    let w = s.win().clone();
    if first_cell {
        let _ = w.term_cell_px(id, cw.round() as u16, ch.round() as u16);
    }
    let _ = w.term_resize(id, cols, rows);
    tx.live_only();
}

/// Focus lost (the window or another field took it): report it.
pub fn blur(s: &mut State, id: u32) {
    let Some(l) = live(s, id) else { return };
    if !l.focused {
        return;
    }
    l.focused = false;
    let report = l.grid.head.modes & M_FOCUS != 0 && l.exit.is_none();
    let w = s.win().clone();
    let _ = w.term_focus(id, false);
    if report {
        write(s, id, "\x1b[O");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::frame::{Encoder, Header, cells};

    #[test]
    fn cell_at_clamps_and_reports_edges() {
        let mut g = Grid::default();
        let mut e = Encoder::new(Header {
            full: true,
            cols: 10,
            rows: 5,
            version: frame::FORMAT_VERSION,
            ..Header::default()
        });
        e.row(0, 0, &cells("x", 1, DEFAULT_BG, 0));
        g.apply(&e.finish()).unwrap();
        let (cw, ch) = metrics();
        let (c, r, right, dy) = cell_at(&g, PAD_X + 2.6 * cw, PAD_Y + 1.5 * ch);
        assert_eq!((c, r, right, dy), (2, 1, true, 0));
        let (c, r, _, dy) = cell_at(&g, -50.0, -50.0);
        assert_eq!((c, r, dy), (0, 0, -1));
        let (c, r, _, dy) = cell_at(&g, 10_000.0, 10_000.0);
        assert_eq!((c, r, dy), (9, 4, 1));
    }
}
