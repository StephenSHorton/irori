//! The card grid (card-grid.tsx CardGrid, CardTile, SubtaskDetailRow,
//! PartOf, SpawnText, MasonryGrid, EmptyCards): each card a tile that
//! opens its page; a family is its parent's tile with its subtasks as rows
//! in a tray under it; the tiles go, in order, into whichever column is
//! shortest (card #113). Pinned tiles drag to reorder (card #138's
//! move_pin).

use std::collections::HashSet;

use irori_window::cards::{CardRef, CardSummary};
use rune::{Color, Drag, El, Phase, Tx, col, el, icon, row, text};

use super::filter::family_groups;
use super::live_map;
use crate::logic::agent_status::ms;
use crate::logic::cards::{Tone, Work, busy, holder_line_shows, nobody_on, open_ask, over, percent, with_live, work_label};
use crate::logic::names::display_name;
use crate::logic::plain::{plain_line, relative_time};
use crate::logic::spawn_line::{SpawnState, SpawnTone, spawn_line};
use crate::logic::wait_name::{Wait, wait_tip};
use crate::ui::theme::status;
use crate::ui::{Frame, State, icons};
use crate::widgets::bits::{
    ask_line, badge, holder_line, owner_chip, pr_chip, progress_bar, scope_badge, short_ref, side_badge, spinner, state_pill,
};
use crate::widgets::faces::{FACES_MAX, faces, talk_bubble};

const GAP: f32 = 12.0;
const MIN_COL: f32 = 260.0;

/// A pinned tile being dragged: its id, and the pinned tile it is over.
#[derive(Clone, Debug, Default)]
pub struct Dragging {
    pub id: String,
    pub over: String,
}

fn small() -> rune::tok::TextStyle {
    rune::tok::TextStyle::new(12.0, 400.0)
}

fn tiny() -> rune::tok::TextStyle {
    rune::tok::TextStyle::new(11.0, 400.0)
}

fn ref_of(r: &CardRef) -> String {
    if r.short.is_empty() { "new".into() } else { format!("{}#{}", r.tag, r.short) }
}

/// What a press on a tile does: opens the card, or ticks it in Closed's
/// select mode.
fn on_tile(s: &mut State, id: &str) {
    if s.board.done.picking {
        if !s.board.done.picked.remove(id) {
            s.board.done.picked.insert(id.to_string());
        }
    } else {
        s.open_card(id);
    }
}

/// The cards as a masonry grid of families. `movable`: the pinned grid,
/// whose pinned tiles drag to a new place. `stamp`: Closed shows each
/// card's "updated" time.
pub fn card_grid(s: &State, f: &mut Frame, name: &str, cards: &[&CardSummary], context: &HashSet<String>, movable: bool, stamp: Option<i64>) -> El {
    let key = format!("board-grid-{name}");
    let pins: Vec<&str> = cards.iter().filter(|c| c.pinned).map(|c| c.id.as_str()).collect();
    let mut items = Vec::new();
    for (head, kids) in family_groups(cards) {
        let t = tile(s, f, head, context.contains(&head.id), movable && head.pinned, stamp);
        let node = if kids.is_empty() {
            t
        } else {
            let k = f.k;
            let mut tray = col()
                .key(format!("board-tray-{}", head.id))
                .mx(8.0)
                .radius(8.0)
                .clip_rounded(8.0)
                .fill(k.c.layer_02.alpha(0.5))
                .border(1.0, k.c.border_muted);
            for c in &kids {
                tray = tray.child(subtask_row(s, f, c, context.contains(&c.id)));
            }
            if head.progress.done > 0 {
                tray = tray.child(
                    row()
                        .key(format!("board-tray-{}/done", head.id))
                        .gap(8.0)
                        .px(12.0)
                        .py(6.0)
                        .center_y()
                        .child(text("✓").font(tiny()).color(status::working()))
                        .child(text(format!("{} done", head.progress.done)).font(tiny()).color(k.c.text_muted)),
                );
            }
            col()
                .key(format!("board-family-{}", head.id))
                .role("group")
                .label(format!("{} and its subtasks", crate::logic::card_status::card_ref(head)))
                .w_full()
                .child(t)
                .child(tray)
        };
        items.push((head.id.clone(), node));
    }
    let _ = pins;
    masonry(f, &key, if name == "pinned" { "Pinned cards" } else { "Shared work" }, items)
}

/// Lays items into as many 260pt+ columns as fit; each goes, in order, to
/// the shortest column so far, by the heights the last layout measured.
fn masonry(f: &mut Frame, key: &str, label: &str, items: Vec<(String, El)>) -> El {
    let width = f.cx.layout_size(key).map_or(0.0, |s| s[0]);
    let n = (((width + GAP) / (MIN_COL + GAP)).floor() as usize).max(1);
    let mut cols: Vec<Vec<El>> = (0..n).map(|_| Vec::new()).collect();
    let mut tall = vec![0.0f32; n];
    for (id, node) in items {
        let item_key = format!("board-item-{id}");
        let h = f.cx.layout_size(&item_key).map_or(200.0, |s| s[1]);
        let mut c = 0;
        for i in 1..n {
            if tall[i] < tall[c] {
                c = i;
            }
        }
        tall[c] += h + GAP;
        cols[c].push(col().key(item_key).w_full().child(node));
    }
    let mut r = row().key(key.to_string()).role("group").label(label.to_string()).w_full().gap(GAP);
    for (i, items) in cols.into_iter().enumerate() {
        let mut c = col().key(format!("{key}/col{i}")).grow(1.0).min_w(0.0).gap(GAP);
        for it in items {
            c = c.child(it);
        }
        r = r.child(c);
    }
    r
}

/// A subtask's "part of #12" chip.
fn part_of(f: &Frame, key: &str, p: &CardRef) -> El {
    text(format!("part of {}", ref_of(p)))
        .key(key.to_string())
        .font(rune::tok::TextStyle::new(11.0, 400.0).mono())
        .color(f.k.c.text_muted)
        .help(format!("A subtask of {} {}", ref_of(p), plain_line(&p.title)))
}

/// A line led by a dot: a question, a wait, a subtask's blocker.
fn dot_line(key: String, color: Color, words: String) -> El {
    row()
        .key(key)
        .w_full()
        .gap(6.0)
        .child(text("●").font(small()).color(color).a11y_hidden())
        .child(text(words).font(small()).color(color).max_lines(2).grow(1.0).min_w(0.0))
}

/// The pinned tiles' rects, read while a drag is on.
fn pin_rects(s: &State, f: &mut Frame) -> Vec<(String, [f32; 4])> {
    s.data
        .cards
        .iter()
        .filter(|c| c.pinned)
        .filter_map(|c| f.cx.layout(&format!("board-tile-{}", c.id)).map(|r| (c.id.clone(), r)))
        .collect()
}

fn hit(rects: &[(String, [f32; 4])], [x, y]: [f32; 2]) -> String {
    rects
        .iter()
        .find(|(_, r)| x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3])
        .map(|(id, _)| id.clone())
        .unwrap_or_default()
}

/// A pinned tile dropped on another takes its place (before it when it
/// came from later in the list, after it when from earlier).
fn drop_on(s: &mut State, d: &Drag, rects: &[(String, [f32; 4])]) {
    let Some(g) = s.board.order.drag.take() else { return };
    if d.cancelled() {
        return;
    }
    let target = hit(rects, d.window_point());
    if target.is_empty() {
        return;
    }
    // The pins in the grid's order (pinned first by rank, as cards_in lists them).
    let mut pins: Vec<(f64, String)> = s.data.cards.iter().filter(|c| c.pinned).map(|c| (c.pin_rank, c.id.clone())).collect();
    pins.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let pins: Vec<String> = pins.into_iter().map(|(_, id)| id).collect();
    let (Some(from), Some(to)) = (pins.iter().position(|p| *p == g.id), pins.iter().position(|p| *p == target)) else {
        return;
    };
    if from == to {
        return;
    }
    let before = if from < to { pins.get(to + 1).cloned().unwrap_or_default() } else { target };
    let id = g.id.clone();
    s.act("Move the pinned card", move |w| w.move_pin(&id, &before), |_, _| {});
}

/// One card's tile (CardTile).
pub fn tile(s: &State, f: &mut Frame, c: &CardSummary, dim: bool, movable: bool, stamp: Option<i64>) -> El {
    let k = f.k;
    let p = format!("board-tile-{}", c.id);
    let pct = percent(c.progress.done, c.progress.total);
    let closed = over(&c.state);
    let parent = c.subtasks > 0;
    let blocked = c.open_blockers > 0;
    let title = Some(plain_line(&c.title)).filter(|t| !t.is_empty()).unwrap_or_else(|| "(untitled)".into());
    let live = live_map(s);
    let lived = with_live(&c.working, &live);
    let working = lived.as_deref().unwrap_or(&c.working);
    let work = Work {
        state: &c.state,
        open_blockers: c.open_blockers,
        working,
    };
    let label = work_label(&work);
    let parked = c.later.as_ref();
    let quiet = parked.is_some() || crate::logic::later::follows(&c.id, c.order.as_ref());
    let picking = s.board.done.picking;
    let selected = if picking { s.board.done.picked.contains(&c.id) } else { s.board.selected == c.id };
    let dragging = s.board.order.drag.as_ref();

    // Line one: ref, pin, part of, state, scope, side, owner, PR, warnings.
    let mut chips = row().key(format!("{p}/chips")).w_full().gap(6.0).center_y().flex_wrap().min_w(0.0);
    if let Some(r) = short_ref(f, &format!("{p}/ref"), c) {
        chips = chips.child(r);
    }
    if c.pinned {
        chips = chips.child(icon(&icons::PIN, 12.0).color(k.c.icon_accent).label("Pinned"));
    }
    if let Some(pr) = &c.parent {
        chips = chips.child(part_of(f, &format!("{p}/part"), pr));
    }
    chips = chips.child(state_pill(f, &format!("{p}/state"), c)).child(scope_badge(f, &format!("{p}/scope"), c));
    if let Some(b) = side_badge(f, &format!("{p}/side"), c) {
        chips = chips.child(b);
    }
    if let Some(b) = owner_chip(s, f, &format!("{p}/owner"), c) {
        chips = chips.child(b);
    }
    if let Some(b) = pr_chip(f, &format!("{p}/pr"), c) {
        chips = chips.child(b);
    }
    if c.working.iter().any(|w| !w.conflict.is_empty()) {
        chips = chips.child(badge(f, &format!("{p}/conflict"), "Lease conflict", Tone::Warning, false, "Two devices leased the same thing while apart"));
    }
    if !closed && !open_ask(c.ask.as_ref()) && nobody_on(c) && c.starting.is_empty() {
        chips = chips.child(badge(f, &format!("{p}/nobody"), "Nobody on it", Tone::Warning, false, "The agent on it went away; nobody is working it"));
    }
    if c.stale {
        chips = chips.child(badge(
            f,
            &format!("{p}/stale"),
            "Stale",
            Tone::Secondary,
            false,
            "Nothing has happened on it for a while, and nobody is busy on it",
        ));
    }
    if busy(&work) {
        chips = chips.child(el().grow(1.0)).child(spinner(f, &format!("{p}/spin")));
    }

    let mut body = col()
        .key(p.clone())
        .role("button")
        .label(title.clone())
        .focusable(true)
        .w_full()
        .min_w(0.0)
        .padding(14.0)
        .gap(10.0)
        .radius(12.0)
        .fill(k.c.layer_01)
        .border(if selected { 1.5 } else { 1.0 }, if selected { k.c.icon_accent } else { k.c.border_muted })
        .hover(k.c.hover)
        .cursor(rune::Cursor::Pointer)
        .child(chips);
    if !c.accent.is_empty()
        && let Some(col) = crate::widgets::faces::device_color(&c.accent)
    {
        body = body.clip_rounded(12.0).child(el().absolute(0.0, 0.0).w(3.0).h_full().fill(col).a11y_hidden());
    }
    if selected {
        body = body.focus_ring(1.0, 0.0, k.c.border_focus);
    }
    if dim || quiet || dragging.is_some_and(|d| d.id == c.id) {
        body = body.opacity(0.6);
    }
    if dragging.is_some_and(|d| d.over == c.id && d.id != c.id) {
        body = body.border(2.0, k.c.icon_accent);
    }
    let id = c.id.clone();
    body = body.on_press(move |s: &mut State| on_tile(s, &id));
    if movable {
        let id = c.id.clone();
        // The pinned tiles' rects ride in the handler (a gesture keeps the
        // handler it started with).
        let rects = pin_rects(s, f);
        body = body.on_drag(move |s: &mut State, d: &mut Drag, _tx: &mut Tx| match d.phase {
            Phase::Down => {}
            Phase::Move => {
                if s.board.order.drag.is_none() && (d.dx.abs() > 4.0 || d.dy.abs() > 4.0) {
                    s.board.order.drag = Some(Dragging {
                        id: id.clone(),
                        over: String::new(),
                    });
                }
                let over = hit(&rects, d.window_point());
                if let Some(g) = &mut s.board.order.drag {
                    g.over = over;
                }
            }
            Phase::Up => drop_on(s, d, &rects),
        });
        body = body.help("Drag to move it among the pinned cards");
    }

    if let Some(tb) = talk_bubble(s, f, &format!("{p}/talk"), &c.talking) {
        body = body.child(tb);
    }
    // A long unbroken word (a test's name) breaks anywhere rather than
    // running out of the tile.
    let long = title.split_whitespace().any(|w| w.chars().count() > 28);
    let t = text(title).font(rune::tok::TextStyle::new(14.0, 500.0)).color(k.c.text_base).w_full();
    body = body.child(if long { t.wrap_anywhere() } else { t.wrap() });
    if blocked && !c.blocker_question.is_empty() {
        let color = if c.ready && c.open_blockers == 1 { status::working() } else { status::failed() };
        body = body.child(dot_line(format!("{p}/q"), color, plain_line(&c.blocker_question)));
    }
    if let Some(l) = &c.left_off
        && !blocked
    {
        let t = plain_line(&l.text);
        body = body.child(
            text(format!("Left off: {t}"))
                .key(format!("{p}/left"))
                .font(small())
                .color(k.c.text_muted)
                .w_full()
                .max_lines(1)
                .help(t),
        );
    }
    if let Some(l) = parked {
        body = body.child(text(format!("Back {}", plain_line(&l.line))).key(format!("{p}/back")).font(small()).color(k.c.text_muted).w_full().wrap());
    }
    if !quiet && !blocked && c.waiting_on > 0 {
        body = body.child(dot_line(format!("{p}/wait"), k.c.text_muted, wait_tip(&Wait::from(c))));
    }
    if !blocked && !c.subtask_blocked.is_empty() {
        body = body.child(dot_line(format!("{p}/sub"), status::decide(), plain_line(&c.subtask_blocked)));
    }
    match &c.ask {
        Some(a) if open_ask(Some(a)) || closed => body = body.child(ask_line(f, &format!("{p}/ask"), a)),
        _ if quiet || (closed && pct.is_none()) => {}
        _ if pct.is_none() => body = body.child(holder_line(s, f, &format!("{p}/holder"), c)),
        _ => {
            if let Some(b) = progress_bar(f, &format!("{p}/bar"), c) {
                body = body.child(b.help(if parent {
                    format!("{} of {} subtasks done", c.progress.done, c.progress.total)
                } else {
                    format!("{}% complete", pct.unwrap_or(0))
                }));
            }
            if !closed && holder_line_shows(&c.holder) {
                body = body.child(holder_line(s, f, &format!("{p}/holder"), c));
            }
        }
    }
    if !quiet {
        body = body.child(footer(s, f, &p, c, pct, parent, closed, working, label, stamp));
    }
    body
}

/// The tile's last line: how far, who's on it (or why nobody is), who
/// watches, and in Closed when it last changed.
#[allow(clippy::too_many_arguments)]
fn footer(
    s: &State,
    f: &mut Frame,
    p: &str,
    c: &CardSummary,
    pct: Option<i64>,
    parent: bool,
    closed: bool,
    working: &[irori_window::cards::AgentRef],
    label: &str,
    stamp: Option<i64>,
) -> El {
    let k = f.k;
    let muted = k.c.text_muted;
    let t = |s: String| text(s).font(tiny()).color(muted);
    let mut r = row().key(format!("{p}/foot")).w_full().gap(6.0).center_y().flex_wrap().min_w(0.0);
    if let Some(pc) = pct {
        r = r
            .child(t(if parent {
                format!("{}/{} subtasks", c.progress.done, c.progress.total)
            } else {
                format!("{pc}%")
            }))
            .child(t("·".into()));
    }
    if !working.is_empty() {
        r = r.child(faces(s, f, &format!("{p}/faces"), working, false, Some(FACES_MAX)));
        if !label.is_empty() {
            r = r.child(t(label.into()));
        }
    } else if closed {
    } else if let Some(l) = spawn_line(&SpawnState::from(c)) {
        r = r.child(text(l.text).font(tiny()).color(if l.tone == SpawnTone::Warn { status::decide() } else { muted }).max_lines(2).shrink());
    } else if open_ask(c.ask.as_ref()) {
        r = r.child(t("you asked the room".into()));
    } else if let Some(g) = &c.returned {
        r = r.child(t(format!("nobody on it: {} went away", display_name(g))));
    } else {
        r = r.child(t("nobody on it yet".into()));
    }
    if !closed && !c.watching.is_empty() {
        r = r
            .child(t("·".into()))
            .child(faces(s, f, &format!("{p}/watch"), &c.watching, true, Some(FACES_MAX)))
            .child(t("watching".into()));
    }
    if closed && let Some(now) = stamp {
        let at = ms(&c.updated_at).unwrap_or(0);
        r = r.child(el().grow(1.0)).child(t(format!("updated {}", relative_time(at, now))).help(format!("Last changed {}", crate::logic::spawn_line::clock(&c.updated_at))));
    }
    r
}

/// A subtask in its parent's tray (card #113): its ref, title and state;
/// who's on it and how far; a blocker's question under that.
fn subtask_row(s: &State, f: &mut Frame, c: &CardSummary, dim: bool) -> El {
    let k = f.k;
    let p = format!("board-sub-{}", c.id);
    let blocked = c.open_blockers > 0;
    let pct = percent(c.progress.done, c.progress.total);
    let who = &c.working;
    let tone = if blocked {
        status::failed()
    } else if !who.is_empty() {
        k.c.icon_accent
    } else {
        status::decide()
    };
    let picking = s.board.done.picking;
    let selected = if picking { s.board.done.picked.contains(&c.id) } else { s.board.selected == c.id };
    let mut line2 = row().key(format!("{p}/who")).w_full().gap(12.0).pl(16.0).center_y();
    if !who.is_empty() {
        line2 = line2.child(
            row()
                .gap(6.0)
                .center_y()
                .min_w(0.0)
                .child(faces(s, f, &format!("{p}/faces"), who, false, None))
                .child(text(display_name(&who[0])).font(tiny()).color(k.c.text_muted).max_lines(1).shrink()),
        );
    } else if let Some(l) = spawn_line(&SpawnState::from(c)) {
        line2 = line2.child(text(l.text).font(tiny()).color(if l.tone == SpawnTone::Warn { status::decide() } else { k.c.text_muted }));
    } else {
        line2 = line2.child(text("nobody on it").font(tiny()).color(status::decide()));
    }
    if let Some(pc) = pct {
        line2 = line2.child(
            row()
                .gap(6.0)
                .center_y()
                .child(
                    el().w(56.0)
                        .h(4.0)
                        .radius(2.0)
                        .clip_rounded(2.0)
                        .fill(k.c.border_muted)
                        .child(el().w_frac(pc as f32 / 100.0).h_full().fill(if blocked { status::failed() } else { k.c.icon_accent })),
                )
                .child(text(format!("{pc}%")).font(tiny()).color(k.c.text_muted)),
        );
    }
    let mut r = col()
        .key(p.clone())
        .role("button")
        .label(plain_line(&c.title))
        .focusable(true)
        .w_full()
        .gap(4.0)
        .px(12.0)
        .py(8.0)
        .fill(if selected { k.c.layer_03 } else { k.c.bg_base.alpha(0.4) })
        .hover(k.c.hover)
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_muted.alpha(0.6))
        .cursor(rune::Cursor::Pointer)
        .opacity(if dim { 0.6 } else { 1.0 })
        .on_press({
            let id = c.id.clone();
            move |s: &mut State| on_tile(s, &id)
        });
    let mut line1 = row().key(format!("{p}/l1")).w_full().gap(8.0).center_y().child(text("●").font(rune::tok::TextStyle::new(10.0, 400.0)).color(tone).a11y_hidden());
    if let Some(sr) = short_ref(f, &format!("{p}/ref"), c) {
        line1 = line1.child(sr);
    }
    line1 = line1
        .child(text(plain_line(&c.title)).font(small()).color(k.c.text_base).max_lines(1).grow(1.0).min_w(80.0))
        .child(row().max_w(150.0).min_w(0.0).shrink().child(state_pill(f, &format!("{p}/state"), c)));
    r = r.child(line1).child(line2);
    if blocked && !c.blocker_question.is_empty() {
        r = r.child(text(plain_line(&c.blocker_question)).font(tiny()).color(status::failed()).max_lines(2).pl(16.0).w_full());
    } else if !blocked && c.waiting_on > 0 {
        r = r.child(text(wait_tip(&Wait::from(c))).font(tiny()).color(k.c.text_muted).max_lines(2).pl(16.0).w_full());
    }
    r
}

/// A tile with no live state: ref and state, title, question, progress,
/// who holds it (what `board::card` draws).
#[allow(dead_code)] // Home's and Spaces' API
pub fn simple_tile(c: &CardSummary, f: &Frame) -> El {
    let k = f.k;
    let p = format!("board-card-{}", c.id);
    let id = c.id.clone();
    let mut chips = row().key(format!("{p}/chips")).w_full().gap(6.0).center_y().flex_wrap();
    if let Some(r) = short_ref(f, &format!("{p}/ref"), c) {
        chips = chips.child(r);
    }
    chips = chips.child(state_pill(f, &format!("{p}/state"), c));
    let mut body = col()
        .key(p.clone())
        .role("button")
        .label(c.title.clone())
        .focusable(true)
        .w(300.0)
        .padding(14.0)
        .gap(10.0)
        .radius(12.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .hover(k.c.hover)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut State| s.open_card(&id))
        .child(chips)
        .child(text(plain_line(&c.title)).font(rune::tok::TextStyle::new(14.0, 500.0)).color(k.c.text_base).w_full().max_lines(3));
    if c.open_blockers > 0 && !c.blocker_question.is_empty() {
        body = body.child(dot_line(format!("{p}/q"), status::failed(), plain_line(&c.blocker_question)));
    }
    if let Some(b) = progress_bar(f, &format!("{p}/bar"), c) {
        body = body.child(b);
    }
    let who = &c.holder.who;
    let held = if who.name.is_empty() {
        "nobody on it yet".to_string()
    } else if c.holder.others > 0 {
        format!("{} +{}", display_name(who), c.holder.others)
    } else {
        display_name(who)
    };
    body.child(text(held).font(tiny()).color(k.c.text_muted).max_lines(1))
}

/// Where cards come from, when there are none (EmptyCards).
#[allow(dead_code)] // Home's and Spaces' API
pub fn empty_cards(f: &Frame) -> El {
    let k = f.k;
    col()
        .key("board-empty-cards")
        .w_full()
        .padding(40.0)
        .gap(12.0)
        .center()
        .radius(12.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(icon(&icons::SPACES, 40.0).color(k.c.text_muted))
        .child(text("No shared work yet").font(k.ty.ui_medium).color(k.c.text_base))
        .child(
            text(
                "Agents put a card here when their work should be shared: what they're on, how far along it is, who else is watching, and what they need you to decide. Plans and backlogs stay in the project's own tracker. Ask the room above to give the agents something to start now.",
            )
            .font(k.ty.ui)
            .color(k.c.text_muted)
            .max_w(448.0)
            .wrap()
            .align(rune::Align::Center),
        )
}
