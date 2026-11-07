//! The Board's canvas (P4; the React app's components/canvas/*: data.ts,
//! node.tsx, reactflow/index.tsx with flow.css, switch.tsx): the board as
//! a map of who is doing what (cards #80, #93, #410, #587). Each live agent
//! is a hub with the cards it holds, the cards nobody is on sit in their
//! own area, done cards are tucked away at the end; blockers hang under
//! their cards, and lines join parents to subtasks, blockers to cards,
//! agents to the cards they hold or watch, and agents that talk.
//!
//! - [`data`]: the graph and its layout (pure, with data.test.ts's tests).
//! - [`node`]: how each node is drawn.
//! - `edges`: the lines as Rune shapes.
//!
//! Pan with a drag on the background, a trackpad scroll or the middle
//! button; zoom with a pinch, a mouse wheel or Cmd/Ctrl + scroll, or the
//! controls; drag a card to place it (kept in coord through
//! `Window::place`, so every device shows it); click a card or a blocker
//! to open the card. The camera lives in Rune `Live` values, so a pan
//! draws without running the view.
//!
//! # API (for the board package)
//!
//! - [`view_switch`]: the List / Canvas switch for the board's header;
//!   [`wants_canvas`] says Canvas is picked (`board.view` in prefs).
//! - [`view`]: the canvas over the cards of the current route's scope (as
//!   the board's list shows them), filling its parent: give it
//!   `.grow(1.0).min_h(0.0)`. [`view_of`] takes the cards.
//! - [`overlay`]: the canvas full screen (the corner button or F), drawn
//!   over the whole window; ui/mod.rs calls it.

pub mod data;
mod edges;
mod icons;
pub mod node;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use irori_window::cards::{CardDetail, CardSummary};
use irori_window::events::{EVENT_BOARD_CHANGED, EVENT_ROSTER_CHANGED};
use rune::{Beat, Color, Drag, El, Key, Live, Phase, PointerButton, Sequence, Shape, canvas, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant};

use crate::logic::cards::open_ask;
use crate::ui::theme::status;
use crate::ui::{Event, Frame, Route, State};

use data::{Edge, EdgeKind, Graph, Kind, Layout, Node, Point, Rect, node_size};

/// The board's picked view, "list" or "canvas" (switch.tsx's
/// irori.board.view, per window).
pub const VIEW_PREF: &str = "board.view";

const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 1.75;
/// How much of the view a fit leaves around the map.
const FIT_PAD: f32 = 0.08;
/// The background's dots, this far apart at zoom 1.
const DOT_GAP: f32 = 22.0;
const MINI_W: f32 = 200.0;
const MINI_H: f32 = 150.0;
/// A press that travels less than this is a click.
const CLICK_SLOP: f32 = 4.0;

/// A card or agent being dragged: where it was, where it is now.
#[derive(Clone, Debug)]
struct NodeDrag {
    id: String,
    from: Point,
    to: Point,
}

/// Where the minimap draws the map: mini = world * scale + off.
#[derive(Clone, Copy, Debug, Default)]
struct Mini {
    scale: f32,
    off: [f32; 2],
}

pub struct Model {
    // The camera: drawn = world * zoom + pan, in the viewport's points.
    pan: Live<[f32; 2]>,
    zoom: Live<f32>,
    /// The dot grid's offset (the pan, modulo its gap).
    dots: Live<[f32; 2]>,
    /// The view's rect on the minimap.
    mini_rect: Live<[f32; 4]>,
    // What the view last drew with, so a handler can keep the live values
    // in step without running it.
    mini: Cell<Mini>,
    dot_gap: Cell<f32>,
    size: Cell<[f32; 2]>,
    /// The map the view was last fitted to (its areas and their sizes).
    fitted: Cell<u64>,
    /// The person panned or zoomed: from then on the view is theirs.
    touched: bool,
    pan_at_press: [f32; 2],
    drag: Option<NodeDrag>,
    /// The last press travelled: it was a drag, not a click.
    dragged: bool,
    /// Spots dropped here, until the cards come back with them.
    moved: HashMap<String, Point>,
    /// The node under the pointer: its lines show, full strength.
    hover: Option<String>,
    /// The page data of each card with open or answered blockers, so each
    /// blocker is its own node, and the change stamp it was read at.
    details: HashMap<String, CardDetail>,
    stamps: HashMap<String, i64>,
    /// Each card's area last time, for hand-offs.
    prev: Option<HashMap<String, String>>,
    /// The cards that just changed hands flash once.
    flash: HashSet<String>,
    flash_gen: u64,
    /// The canvas fills the window (card #410).
    pub full: bool,
    /// The headless scene shows the canvas as the page.
    pub scene: bool,
}

impl Default for Model {
    fn default() -> Model {
        Model {
            pan: Live::new([0.0, 0.0]),
            zoom: Live::new(1.0),
            dots: Live::new([0.0, 0.0]),
            mini_rect: Live::new([0.0, 0.0, 0.0, 0.0]),
            mini: Cell::new(Mini::default()),
            dot_gap: Cell::new(DOT_GAP),
            size: Cell::new([0.0, 0.0]),
            fitted: Cell::new(0),
            touched: false,
            pan_at_press: [0.0, 0.0],
            drag: None,
            dragged: false,
            moved: HashMap::new(),
            hover: None,
            details: HashMap::new(),
            stamps: HashMap::new(),
            prev: None,
            flash: HashSet::new(),
            flash_gen: 0,
            full: false,
            scene: false,
        }
    }
}

impl Model {
    fn camera(&self) -> ([f32; 2], f32) {
        (self.pan.get(), self.zoom.get())
    }

    /// Moves the camera and what follows it (the dots, the minimap's view
    /// rect). Only live values change, so a handler that calls just this
    /// can say `tx.live_only()`.
    fn set_camera(&self, pan: [f32; 2], zoom: f32) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan.set(pan);
        self.zoom.set(zoom);
        let g = self.dot_gap.get().max(1.0);
        self.dots.set([pan[0].rem_euclid(g) - g, pan[1].rem_euclid(g) - g]);
        let m = self.mini.get();
        let [w, h] = self.size.get();
        self.mini_rect.set([
            -pan[0] / zoom * m.scale + m.off[0],
            -pan[1] / zoom * m.scale + m.off[1],
            w / zoom * m.scale,
            h / zoom * m.scale,
        ]);
    }

    /// Zooms by `factor` keeping the point under (x, y) where it is.
    fn zoom_about(&self, x: f32, y: f32, factor: f32) {
        let (pan, z) = self.camera();
        let nz = (z * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let p = [(x - pan[0]) / z, (y - pan[1]) / z];
        self.set_camera([x - p[0] * nz, y - p[1] * nz], nz);
    }

    /// The whole map in view, a little room around it.
    fn fit(&self, b: Rect) {
        let [w, h] = self.size.get();
        if w <= 0.0 || h <= 0.0 || b.w <= 0.0 || b.h <= 0.0 {
            return;
        }
        let z = (w / (b.w * (1.0 + 2.0 * FIT_PAD)))
            .min(h / (b.h * (1.0 + 2.0 * FIT_PAD)))
            .clamp(MIN_ZOOM, MAX_ZOOM);
        let pan = [w / 2.0 - (b.x + b.w / 2.0) * z, h / 2.0 - (b.y + b.h / 2.0) * z];
        self.set_camera(pan, z);
    }

    /// Centres the view on world point p.
    fn center_on(&self, p: [f32; 2]) {
        let (_, z) = self.camera();
        let [w, h] = self.size.get();
        self.set_camera([w / 2.0 - p[0] * z, h / 2.0 - p[1] * z], z);
    }
}

// ---- the cards ---------------------------------------------------------------

/// The cards of the current route's scope, as the board's list holds them:
/// Home's in the picked space (your own with the shared cards your agents
/// are on, card #199, or the picked shared space's), a
/// space's on its page, every card elsewhere. Open asks and blockers
/// first, then newest change first (the backend's grid order).
pub fn scoped(s: &State) -> Vec<&CardSummary> {
    let mut v: Vec<&CardSummary> = match &s.nav.route {
        // Home's scope in the switcher's space (card #393).
        Route::Home => {
            let (space, scope) = crate::home::home_scope(s);
            s.data
                .cards
                .iter()
                .filter(|c| crate::logic::spaces::card_in_space(c, &space) && crate::board::filter::in_scope(c, &scope))
                .collect()
        }
        Route::Board { space, board } => s
            .data
            .cards
            .iter()
            .filter(|c| c.board == *space)
            .filter(|c| board.as_ref().is_none_or(|b| c.space_board == *b))
            .collect(),
        _ => s.data.cards.iter().collect(),
    };
    v.sort_by_key(|c| (!open_ask(c.ask.as_ref()), c.open_blockers <= 0));
    v
}

/// Canvas is the board's picked view.
pub fn wants_canvas(s: &State) -> bool {
    s.prefs.get::<String>(VIEW_PREF).as_deref() == Some("canvas")
}

fn active(s: &State) -> bool {
    s.canvas.scene || s.canvas.full || wants_canvas(s)
}

/// Picks the board's view.
pub fn set_view(s: &mut State, v: &str) {
    s.prefs.set(VIEW_PREF, v.to_string());
    if v == "canvas" {
        refresh_details(s);
    } else {
        s.canvas.full = false;
    }
}

/// The board's view switch (card #93): List (the grid) or Canvas.
pub fn view_switch(s: &State, f: &Frame) -> El {
    let canvas = wants_canvas(s);
    let mut r = row().key("canvas/switch").gap(4.0).center_y().role("group").label("Board view");
    for (id, label) in [("list", "List"), ("canvas", "Canvas")] {
        let on = (id == "canvas") == canvas;
        r = r.child(
            Button::new(label)
                .control_size(ControlSize::Regular)
                .variant(if on { ButtonVariant::Neutral } else { ButtonVariant::Ghost })
                .on_press(move |s: &mut State| set_view(s, id))
                .build(f.k)
                .key(format!("canvas/switch/{id}"))
                .checked(on),
        );
    }
    r
}

/// Reads the page of each card with open or answered blockers that
/// changed since it was last read, off the UI thread.
fn refresh_details(s: &mut State) {
    let want: Vec<(String, i64)> = s
        .data
        .cards
        .iter()
        .filter(|c| c.open_blockers > 0 || c.waiting > 0)
        .map(|c| (c.id.clone(), c.updated_at.unix_milli_value()))
        .collect();
    let keep: HashSet<&str> = want.iter().map(|(id, _)| id.as_str()).collect();
    s.canvas.details.retain(|id, _| keep.contains(id.as_str()));
    s.canvas.stamps.retain(|id, _| keep.contains(id.as_str()));
    let ask: Vec<(String, i64)> = want
        .into_iter()
        .filter(|(id, at)| s.canvas.stamps.get(id) != Some(at))
        .collect();
    if ask.is_empty() {
        return;
    }
    for (id, at) in &ask {
        s.canvas.stamps.insert(id.clone(), *at);
    }
    s.spawn(
        move |w| {
            ask.into_iter()
                .filter_map(|(id, _)| w.card(&id).ok().map(|d| (id, d)))
                .collect::<Vec<_>>()
        },
        |s, got| {
            for (id, d) in got {
                s.canvas.details.insert(id, d);
            }
        },
    );
}

/// Marks the cards that just changed hands, and drops dropped spots the
/// cards now carry.
fn note_changes(s: &mut State) {
    let cards: Vec<&CardSummary> = s.data.cards.iter().collect();
    let g = data::to_graph(&cards, &HashMap::new());
    let now = data::holders(&g);
    if let Some(prev) = &s.canvas.prev {
        let moved = data::hand_offs(prev, &g);
        if !moved.is_empty() {
            s.canvas.flash = moved.into_iter().collect();
            s.canvas.flash_gen += 1;
        }
    }
    s.canvas.prev = Some(now);
    // A card's dropped spot holds until the card comes back with it; an
    // agent's is only for a look, until the board changes.
    let data = &s.data;
    s.canvas.moved.retain(|id, p| {
        data.card(id).is_some_and(|c| {
            c.pos
                .as_ref()
                .is_none_or(|q| (q.x as f32 - p.x).abs() > 1.0 || (q.y as f32 - p.y).abs() > 1.0)
        })
    });
}

pub fn on_event(s: &mut State, ev: &Event) {
    match ev.name.as_str() {
        "app:start" | EVENT_BOARD_CHANGED | EVENT_ROSTER_CHANGED => {
            note_changes(s);
            if active(s) {
                refresh_details(s);
            }
        }
        _ => {}
    }
}

// ---- the view ----------------------------------------------------------------

/// Everything the canvas draws, laid out.
struct Map {
    graph: Graph,
    auto: Layout,
    pos: HashMap<String, Point>,
    groups: HashMap<String, Rect>,
}

fn map_of(s: &State, cards: &[&CardSummary]) -> Map {
    let graph = data::to_graph(cards, &s.canvas.details);
    let auto = data::auto_layout(&graph);
    let layout = data::with_placed(&graph, &auto);
    let mut pos = layout.pos.clone();
    let mut shifted = false;
    for (id, p) in &s.canvas.moved {
        if pos.contains_key(id) {
            pos.insert(id.clone(), *p);
            shifted = true;
        }
    }
    if let Some(d) = &s.canvas.drag
        && pos.contains_key(&d.id)
    {
        pos.insert(d.id.clone(), d.to);
        shifted = true;
    }
    // A card dragged carries its blockers; each area holds its members
    // where they are now.
    let (pos, groups) = if shifted {
        let pos = data::followers(&graph, &pos);
        let groups = data::group_rects(&graph, &pos, &layout, Some(&auto.groups));
        (pos, groups)
    } else {
        (pos, layout.groups.clone())
    };
    Map { graph, auto, pos, groups }
}

/// The rect around every area and node.
fn bounds(m: &Map) -> Rect {
    let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    let mut grow = |r: Rect| {
        b[0] = b[0].min(r.x);
        b[1] = b[1].min(r.y);
        b[2] = b[2].max(r.x + r.w);
        b[3] = b[3].max(r.y + r.h);
    };
    for r in m.groups.values() {
        grow(*r);
    }
    for n in &m.graph.nodes {
        if let (Some(k), Some(p)) = (n.kind(), m.pos.get(n.id())) {
            let s = node_size(k);
            grow(Rect {
                x: p.x,
                y: p.y,
                w: s.w,
                h: s.h,
            });
        }
    }
    if !b[0].is_finite() {
        return Rect::default();
    }
    Rect {
        x: b[0],
        y: b[1],
        w: b[2] - b[0],
        h: b[3] - b[1],
    }
}

/// What the map is made of: its areas and how many cards each holds. The
/// view fits again when it changes, until the person moves it.
fn signature(g: &Graph, size: [f32; 2]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut count: Vec<(&str, usize)> = Vec::new();
    for c in g.cards() {
        match count.iter_mut().find(|x| x.0 == c.group) {
            Some(x) => x.1 += 1,
            None => count.push((&c.group, 1)),
        }
    }
    count.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    count.hash(&mut h);
    // The first fit waits for a size.
    (size[0] > 0.0).hash(&mut h);
    h.finish() | 1
}

/// The canvas over the cards of the current route's scope. While it is
/// full screen the page keeps an empty box and [`overlay`] draws it.
pub fn view(s: &State, f: &mut Frame) -> El {
    let cards = scoped(s);
    view_of(s, f, &cards)
}

/// The canvas over `cards`.
pub fn view_of(s: &State, f: &mut Frame, cards: &[&CardSummary]) -> El {
    if s.canvas.full {
        return el().key("canvas/box").w_full().h_full().min_h(0.0);
    }
    let c = board_canvas(s, f, cards, false);
    framed(f, c)
}

/// The canvas in its panel on the page.
fn framed(f: &Frame, c: El) -> El {
    el().key("canvas/box")
        .w_full()
        .h_full()
        .min_h(280.0)
        .radius(10.0)
        .border(1.0, f.k.c.border_muted)
        .clip_rounded(10.0)
        .child(c)
}

/// The canvas full screen, over the whole window (card #410): Esc or the
/// corner button closes it.
pub fn overlay(s: &State, f: &mut Frame) {
    if !s.canvas.full {
        return;
    }
    let cards = scoped(s);
    let c = board_canvas(s, f, &cards, true);
    f.layer.push(
        el().key("canvas/full")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .fill(f.k.c.bg_base)
            .modal(false, |s: &mut State| s.canvas.full = false)
            .child(c),
    );
}

/// The headless scene's page: the board's header with the view switch over
/// the canvas. `None` unless the scene is on.
pub fn page(s: &State, f: &mut Frame) -> Option<El> {
    // A card opened from the canvas shows its page over it.
    if !s.canvas.scene || s.nav.card.is_some() {
        return None;
    }
    let title = match &s.nav.route {
        Route::Board { space, .. } => s
            .data
            .spaces
            .iter()
            .find(|b| &b.board_id == space)
            .map_or_else(|| "Board".to_string(), |b| b.name.clone()),
        _ => "Home".to_string(),
    };
    let header = crate::ui::page_header(f, &title, vec![view_switch(s, f)]);
    let body = view(s, f);
    Some(
        col()
            .key("canvas/page")
            .w_full()
            .h_full()
            .min_h(0.0)
            .child(header)
            .child(
                col()
                    .key("canvas/page-body")
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .px(20.0)
                    .pb(20.0)
                    .child(body),
            ),
    )
}

fn board_canvas(s: &State, f: &mut Frame, cards: &[&CardSummary], full: bool) -> El {
    let k = f.k;
    let m = &s.canvas;
    let map = map_of(s, cards);
    let b = bounds(&map);

    // The viewport's size: fit the map while it is untouched, else keep
    // the middle still as the size changes (a window resize, full screen).
    let size = f.cx.layout_size("canvas/viewport").unwrap_or([0.0, 0.0]);
    let old = m.size.get();
    if size != old {
        m.size.set(size);
        if old[0] > 0.0 && old[1] > 0.0 {
            let (pan, z) = m.camera();
            m.set_camera([pan[0] + (size[0] - old[0]) / 2.0, pan[1] + (size[1] - old[1]) / 2.0], z);
        }
    }
    let (_, zoom) = m.camera();
    // The dots: DOT_GAP apart at zoom 1, never closer than 8 points.
    let mut gap = DOT_GAP * zoom;
    while gap < 8.0 {
        gap *= 2.0;
    }
    m.dot_gap.set(gap);
    let narrow = size[0] > 0.0 && size[0] < 640.0;
    let sig = signature(&map.graph, size);
    if !m.touched && sig != m.fitted.get() && size[0] > 0.0 {
        m.fitted.set(sig);
        m.fit(b);
    }
    // The minimap shows the map and the view together (React Flow's
    // MiniMap), as the view stood when this view ran.
    let mini = if b.w > 0.0 && b.h > 0.0 {
        let (pan, z) = m.camera();
        let mut u = b;
        if size[0] > 0.0 {
            let v = Rect {
                x: -pan[0] / z,
                y: -pan[1] / z,
                w: size[0] / z,
                h: size[1] / z,
            };
            let (x0, y0) = (u.x.min(v.x), u.y.min(v.y));
            let (x1, y1) = ((u.x + u.w).max(v.x + v.w), (u.y + u.h).max(v.y + v.h));
            u = Rect {
                x: x0,
                y: y0,
                w: x1 - x0,
                h: y1 - y0,
            };
        }
        let pad = 6.0;
        let scale = ((MINI_W - 2.0 * pad) / u.w).min((MINI_H - 2.0 * pad) / u.h);
        Mini {
            scale,
            off: [
                (MINI_W - u.w * scale) / 2.0 - u.x * scale,
                (MINI_H - u.h * scale) / 2.0 - u.y * scale,
            ],
        }
    } else {
        Mini::default()
    };
    m.mini.set(mini);
    // Keep the live values in step with what this view draws.
    let (pan, z) = m.camera();
    m.set_camera(pan, z);

    let mut world = el()
        .key("canvas/world")
        .pin(Some(0.0), Some(0.0), None, None)
        .size(b.x.max(0.0) + b.w.max(1.0), b.y.max(0.0) + b.h.max(1.0))
        .origin(0.0, 0.0)
        .live_offset(&m.pan)
        .live_scale(&m.zoom);
    // Areas behind everything.
    for n in &map.graph.nodes {
        if let Node::Group(g) = n
            && let Some(r) = map.groups.get(&g.id)
        {
            world = world.child(
                node::group_body(f, g, r.w, r.h)
                    .key(format!("canvas/g/{}", g.id))
                    .pin(Some(r.x), Some(r.y), None, None),
            );
        }
    }
    // Then the lines, lit ones last.
    let mut lit_edges = Vec::new();
    for e in &map.graph.edges {
        let lit = m.hover.as_ref().is_some_and(|h| *h == e.source || *h == e.target);
        if lit {
            lit_edges.push(e);
            continue;
        }
        if let Some(x) = edge_el(s, f, &map, e, b, false) {
            world = world.child(x);
        }
    }
    for e in lit_edges {
        if let Some(x) = edge_el(s, f, &map, e, b, true) {
            world = world.child(x);
        }
    }
    // Then the nodes.
    for n in &map.graph.nodes {
        if let Some(x) = node_el(s, f, n, &map) {
            world = world.child(x);
        }
    }
    // Talk lines' labels above the nodes.
    for e in &map.graph.edges {
        if e.kind == EdgeKind::Talk
            && !e.label.is_empty()
            && let Some(pts) = route_of(&map, e)
        {
            let mid = pts[pts.len() / 2];
            world = world.child(
                el().key(format!("canvas/el/{}", e.id))
                    .pin(Some(mid[0] - 30.0), Some(mid[1] - 9.0), None, None)
                    .px(4.0)
                    .h(18.0)
                    .center_y()
                    .fill(k.c.layer_01)
                    .child(
                        text(e.label.clone())
                            .font(rune::tok::TextStyle::new(11.0, 400.0))
                            .color(status::review())
                            .max_lines(1),
                    ),
            );
        }
    }

    let mut viewport = el()
        .key("canvas/viewport")
        .role("group")
        .label("Board canvas")
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(k.c.bg_base)
        .clip_soft(0.0)
        .focusable(true)
        .focus_ring(0.0, 0.0, Color::default())
        .pointer_buttons([PointerButton::Middle])
        .child(dots(f, size, gap, &m.dots))
        .child(world);
    if map.graph.nodes.is_empty() {
        viewport = viewport.child(
            text("No cards here.")
                .key("canvas/empty")
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .pin(Some(20.0), Some(20.0), None, None),
        );
    }
    viewport = viewport
        .child(controls(f))
        .child(toolbar(s, f, &map, full));
    if !narrow && b.w > 0.0 {
        viewport = viewport.child(minimap(s, f, &map, mini));
    }
    viewport
        // A trackpad scroll pans; a mouse wheel, or a scroll with Cmd or
        // Ctrl held, zooms about the pointer (React Flow's way).
        .on_wheel(|s: &mut State, e: rune::WheelEvent, tx| {
            s.canvas.touched = true;
            if e.mods.cmd || e.mods.ctrl || !e.precise {
                s.canvas.zoom_about(e.x, e.y, (e.dy * 0.002).exp());
            } else {
                let (pan, z) = s.canvas.camera();
                s.canvas.set_camera([pan[0] + e.dx, pan[1] + e.dy], z);
                tx.live_only();
            }
            true
        })
        .on_magnify(|s: &mut State, e: rune::MagnifyEvent, _tx| {
            s.canvas.touched = true;
            s.canvas.zoom_about(e.x, e.y, 1.0 + e.delta);
            true
        })
        // A drag on the background (or with the middle button) pans.
        .on_drag(|s: &mut State, d: &mut Drag, tx| {
            let m = &mut s.canvas;
            if d.phase == Phase::Down {
                m.pan_at_press = m.pan.get();
                m.touched = true;
            }
            let (_, z) = m.camera();
            let pan = if d.cancelled() {
                m.pan_at_press
            } else {
                [m.pan_at_press[0] + d.dx, m.pan_at_press[1] + d.dy]
            };
            m.set_camera(pan, z);
            tx.live_only();
        })
        .on_key(|s: &mut State, e: &rune::KeyEvent, _tx| {
            let plain = !e.mods.cmd && !e.mods.ctrl && !e.mods.alt;
            match &e.key {
                Key::Text(t) if plain && t.eq_ignore_ascii_case("f") => {
                    s.canvas.full = !s.canvas.full;
                    true
                }
                Key::Escape if s.canvas.full => {
                    s.canvas.full = false;
                    true
                }
                _ => false,
            }
        })
}

/// The background's dots, in the viewport's points, moved by the pan
/// modulo their gap.
fn dots(f: &Frame, size: [f32; 2], gap: f32, off: &Live<[f32; 2]>) -> El {
    let (w, h) = (size[0] + 2.0 * gap, size[1] + 2.0 * gap);
    let mut sh = Shape::new();
    if size[0] > 0.0 {
        let c = f.k.c.border_base;
        let r = 0.6;
        let (cols, rows) = ((w / gap).ceil() as usize + 1, (h / gap).ceil() as usize + 1);
        for j in 0..rows {
            for i in 0..cols {
                let (x, y) = (i as f32 * gap, j as f32 * gap);
                sh = sh.fill(vec![[x - r, y - r], [x + r, y - r], [x + r, y + r], [x - r, y + r]], c);
            }
        }
    }
    canvas(sh)
        .key("canvas/dots")
        .pin(Some(0.0), Some(0.0), None, None)
        .size(w.max(1.0), h.max(1.0))
        .live_offset(off)
        .cache()
}

fn rect_of(map: &Map, id: &str) -> Option<(Point, data::Size)> {
    let n = map.graph.node(id)?;
    Some((*map.pos.get(id)?, node_size(n.kind()?)))
}

fn route_of(map: &Map, e: &Edge) -> Option<Vec<[f32; 2]>> {
    Some(edges::route(e.kind, rect_of(map, &e.source)?, rect_of(map, &e.target)?))
}

/// One line. A holder line inside its agent's hub (the box says it) and a
/// watcher line wait for a hover, so the map isn't a web; with a node
/// hovered, its lines show full strength and the rest fade a little.
fn edge_el(s: &State, f: &Frame, map: &Map, e: &Edge, b: Rect, lit: bool) -> Option<El> {
    let hidden = e.kind == EdgeKind::Watcher || e.in_hub;
    if hidden && !lit {
        return None;
    }
    let pts = route_of(map, e)?;
    let lk = edges::look(e.kind);
    let color = match e.kind {
        EdgeKind::Subtask | EdgeKind::Watcher => f.k.c.text_muted,
        EdgeKind::Blocker => status::failed(),
        EdgeKind::Holder => f.k.c.icon_accent,
        EdgeKind::Talk => status::review(),
    };
    let opacity = match (&s.canvas.hover, lit) {
        (_, true) => 1.0,
        (Some(_), false) => lk.opacity * 0.5,
        (None, false) => lk.opacity,
    };
    // Lines can reach outside the map's rect a little (a step's elbow).
    let pad = 40.0;
    let origin = [b.x - pad, b.y - pad];
    Some(
        canvas(edges::shape(&pts, lk, color, opacity, lk.width, origin))
            .key(format!("canvas/e/{}", e.id))
            .pin(Some(origin[0]), Some(origin[1]), None, None)
            .size(b.w + 2.0 * pad, b.h + 2.0 * pad),
    )
}

const FLASH: &[Beat] = &[Beat::new(0.0), Beat::new(1.5).opacity(0.0)];

/// A card, blocker or agent at its spot: a click on a card or blocker
/// opens the card, a drag moves a card (its spot is kept) or an agent
/// (for a look), and the pointer over it lights its lines.
fn node_el(s: &State, f: &mut Frame, n: &Node, map: &Map) -> Option<El> {
    let kind = n.kind()?;
    let p = *map.pos.get(n.id())?;
    let id = n.id().to_string();
    let key = format!("canvas/n/{id}");
    let mut e = el().key(key.clone()).pin(Some(p.x), Some(p.y), None, None);
    match n {
        Node::Card(c) => {
            e = e.child(node::card_body(s, f, &format!("{key}/body"), c));
            if let Some(mk) = node::card_markers(f, &key, c) {
                e = e.child(mk);
            }
            if c.done {
                e = e.opacity(0.45);
            }
            if s.canvas.flash.contains(&c.id) {
                e = e.child(flash_ring(f, &key, s.canvas.flash_gen));
            }
            e = e.role("button").label(c.label.clone());
        }
        Node::Blocker(b) => {
            e = e.child(node::blocker_body(f, b)).role("button").label(b.label.clone());
        }
        Node::Agent(a) => {
            e = e.child(node::agent_body(s, f, &key, a));
        }
        Node::Group(_) => return None,
    }
    let hover_id = id.clone();
    e = e.on_hover_change(move |s: &mut State, on, _tx| {
        if on {
            s.canvas.hover = Some(hover_id.clone());
        } else if s.canvas.hover.as_deref() == Some(hover_id.as_str()) {
            s.canvas.hover = None;
        }
    });
    if let Some(open) = node::open_target(n) {
        e = e.on_press(move |s: &mut State| {
            if std::mem::take(&mut s.canvas.dragged) {
                return;
            }
            s.open_card(&open);
        });
    }
    // A blocker follows its card.
    if kind != Kind::Blocker {
        let is_card = kind == Kind::Card;
        e = e.on_drag(move |s: &mut State, d: &mut Drag, _tx| node_drag(s, &id, p, is_card, d));
    }
    Some(e)
}

fn node_drag(s: &mut State, id: &str, at: Point, card: bool, d: &Drag) {
    let m = &mut s.canvas;
    let z = m.zoom.get();
    match d.phase {
        Phase::Down => {
            m.drag = Some(NodeDrag {
                id: id.to_string(),
                from: at,
                to: at,
            });
            m.dragged = false;
        }
        Phase::Move => {
            if d.dx.abs() > CLICK_SLOP || d.dy.abs() > CLICK_SLOP {
                m.dragged = true;
            }
            if let Some(g) = &mut m.drag
                && m.dragged
            {
                g.to = Point {
                    x: g.from.x + d.dx / z,
                    y: g.from.y + d.dy / z,
                };
            }
        }
        Phase::Up => {
            let Some(g) = m.drag.take() else { return };
            if d.cancelled() || !m.dragged {
                return;
            }
            let to = Point {
                x: g.from.x + d.dx / z,
                y: g.from.y + d.dy / z,
            };
            m.moved.insert(g.id.clone(), to);
            if card {
                let (x, y) = (to.x.round() as i64, to.y.round() as i64);
                let id = g.id;
                s.act("Couldn't keep the card's spot", move |w| w.place(&id, x, y), |_, _| {});
            }
        }
    }
}

/// A card that just changed hands flashes once (card #587); Reduce Motion
/// lands it at once.
fn flash_ring(f: &Frame, key: &str, generation: u64) -> El {
    let c = f.k.c.icon_accent;
    el().key(format!("{key}/flash/{generation}"))
        .pin(Some(-3.0), Some(-3.0), Some(-3.0), Some(-3.0))
        .border(3.0, c)
        .shadow(0.0, 24.0, c.alpha(0.6))
        .a11y_hidden()
        .sequence(Sequence::new(FLASH))
}

/// Zoom in, zoom out and fit, bottom left (React Flow's controls).
fn controls(f: &Frame) -> El {
    let k = f.k;
    let b = |icon: &'static rune::Icon, label: &str| IconButton::new(icon).variant(IconButtonVariant::Ghost).label(label).help(label);
    col()
        .key("canvas/controls")
        .pin(Some(12.0), None, None, Some(12.0))
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .radius(6.0)
        .child(
            b(&icons::ZOOM_IN, "Zoom in")
                .on_press(|s: &mut State| {
                    s.canvas.touched = true;
                    let [w, h] = s.canvas.size.get();
                    s.canvas.zoom_about(w / 2.0, h / 2.0, 1.2);
                })
                .build(k)
                .key("canvas/zoom-in"),
        )
        .child(
            b(&icons::ZOOM_OUT, "Zoom out")
                .on_press(|s: &mut State| {
                    s.canvas.touched = true;
                    let [w, h] = s.canvas.size.get();
                    s.canvas.zoom_about(w / 2.0, h / 2.0, 1.0 / 1.2);
                })
                .build(k)
                .key("canvas/zoom-out"),
        )
        .child(
            b(&icons::FIT, "Fit view")
                .on_press(|s: &mut State| {
                    // The next view fits the map again.
                    s.canvas.touched = false;
                    s.canvas.fitted.set(0);
                })
                .build(k)
                .key("canvas/fit"),
        )
}

/// Tidy up (when any card has a kept spot) and full screen, top right.
fn toolbar(s: &State, f: &Frame, map: &Map, full: bool) -> El {
    let k = f.k;
    let mut r = row().key("canvas/toolbar").pin(None, Some(12.0), Some(12.0), None).gap(8.0).center_y();
    let placed = map.graph.cards().any(|n| n.card.pos.is_some()) || !s.canvas.moved.is_empty();
    if placed {
        // Every card back where Irori lays it out.
        let back: Vec<(String, i64, i64)> = map
            .graph
            .cards()
            .filter(|n| n.card.pos.is_some() || s.canvas.moved.contains_key(&n.id))
            .filter_map(|n| map.auto.pos.get(&n.id).map(|p| (n.id.clone(), p.x.round() as i64, p.y.round() as i64)))
            .collect();
        r = r.child(
            Button::new("Tidy up")
                .control_size(ControlSize::Regular)
                .help("Put every card back where Irori lays it out")
                .on_press(move |s: &mut State| {
                    let back = back.clone();
                    s.canvas.moved.clear();
                    s.act(
                        "Couldn't tidy the canvas",
                        move |w| {
                            for (id, x, y) in &back {
                                w.place(id, *x, *y)?;
                            }
                            Ok(())
                        },
                        |_, _| {},
                    );
                })
                .build(k)
                .key("canvas/tidy"),
        );
    }
    let (icon, label, tip) = if full {
        (&icons::MINIMIZE_2, "Exit full screen", "Exit full screen (Esc)")
    } else {
        (&icons::MAXIMIZE_2, "Full screen", "Full screen (F)")
    };
    r.child(
        IconButton::new(icon)
            .label(label)
            .help(tip)
            .on_press(|s: &mut State| s.canvas.full = !s.canvas.full)
            .build(k)
            .key("canvas/full-toggle")
            .checked(full),
    )
}

/// The whole map small, bottom right, with the view's rect on it; a press
/// or drag on it moves the view there.
fn minimap(s: &State, f: &Frame, map: &Map, mini: Mini) -> El {
    let k = f.k;
    let mut sh = Shape::new();
    for n in &map.graph.nodes {
        let (Some(kind), Some(p)) = (n.kind(), map.pos.get(n.id())) else { continue };
        let c = match n {
            Node::Card(c) => node::tone_color(f, c.tone),
            Node::Blocker(_) => status::failed(),
            _ => k.c.icon_accent,
        };
        let sz = node_size(kind);
        let (x0, y0) = (p.x * mini.scale + mini.off[0], p.y * mini.scale + mini.off[1]);
        let (x1, y1) = (x0 + sz.w * mini.scale, y0 + sz.h * mini.scale);
        sh = sh.fill(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], c.alpha(0.85));
    }
    let to_world = move |x: f32, y: f32| [(x - mini.off[0]) / mini.scale.max(1e-6), (y - mini.off[1]) / mini.scale.max(1e-6)];
    el().key("canvas/minimap")
        .role("group")
        .label("Mini map")
        .pin(None, None, Some(12.0), Some(12.0))
        .size(MINI_W, MINI_H)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .radius(6.0)
        .clip_rounded(6.0)
        .child(canvas(sh).key("canvas/minimap/map").size(MINI_W, MINI_H))
        .child(
            el().key("canvas/minimap/view")
                .pin(Some(0.0), Some(0.0), None, None)
                .size(1.0, 1.0)
                .live_rect(&s.canvas.mini_rect)
                .fill(k.c.icon_accent.alpha(0.08))
                .border(1.0, k.c.icon_accent.alpha(0.6)),
        )
        .on_drag(move |s: &mut State, d: &mut Drag, tx| {
            if d.cancelled() {
                return;
            }
            s.canvas.touched = true;
            s.canvas.center_on(to_world(d.x, d.y));
            tx.live_only();
        })
}

/// The headless scene: the canvas as the page, over Home's cards.
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Home);
    s.canvas.scene = true;
    s.prefs.set(VIEW_PREF, "canvas".to_string());
    refresh_details(s);
}
