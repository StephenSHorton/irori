//! The chip rows over the grid (card-grid.tsx FilterBar, DeviceBar,
//! BoardBar, MemberBar) and the List/Canvas switch (canvas/switch.tsx
//! ViewSwitch).

use irori_window::boards::SummaryMember;
use rune::{El, el, icon, row, text};

use super::filter::{FILTERS, Filter, RowBoard, presence_line};
use super::{View, VIEW_PREF, set_filter};
use crate::logic::agent_status::now_ms;
use crate::logic::cards::DeviceChip;
use crate::logic::owners::owner_filter;
use crate::logic::plain::plain_line;
use crate::ui::theme::status;
use crate::ui::{Frame, State, icons};
use crate::widgets::faces::device_color;

/// How a pressed chip looks: the tabs' filled one, or the chips' grey one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Default,
    Secondary,
}

/// One chip button (a shadcn Button size sm, ghost until pressed).
pub fn chip(f: &Frame, key: &str, label: &str, on: bool, look: Look, tip: &str, lead: Option<El>, trail: Option<El>, press: impl Fn(&mut State) + 'static) -> El {
    let k = f.k;
    let (fill, ink) = match (on, look) {
        (true, Look::Default) => (k.c.text_base, k.c.bg_base),
        (true, Look::Secondary) => (k.c.layer_03, k.c.text_base),
        _ => (rune::Color::TRANSPARENT, k.c.text_base),
    };
    let mut c = row()
        .key(key.to_string())
        .role("button")
        .label(label.to_string())
        .checked(on)
        .focusable(true)
        .h(28.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .radius(k.radius.md)
        .fill(fill)
        .cursor(rune::Cursor::Pointer)
        .on_press(press);
    if !on {
        c = c.hover(k.c.hover);
    }
    if !tip.is_empty() {
        c = c.help(tip.to_string());
    }
    if let Some(l) = lead {
        c = c.child(l);
    }
    c = c.child(text(label.to_string()).font(k.ty.small_medium).color(ink));
    if let Some(t) = trail {
        c = c.child(t);
    }
    c
}

/// The tabs: All, Needs a decision (its count red), In review while some
/// are, Closed (`only`: the tabs shown).
pub fn filter_bar(f: &Frame, home: bool, on: &Filter, decide: usize, review: usize, only: &[Filter], scope: &str) -> El {
    let k = f.k;
    let mut r = row().key("board-tabs").role("group").label("Filter cards").gap(4.0).center_y().flex_wrap();
    for (id, label) in FILTERS.iter() {
        if !only.contains(id) {
            continue;
        }
        if *id == Filter::Review && review == 0 && *on != Filter::Review {
            continue;
        }
        let pressed = on == id;
        let count = match id {
            Filter::Decide if decide > 0 => Some((decide, if pressed { k.c.bg_base.alpha(0.8) } else { status::failed() })),
            Filter::Review if review > 0 => Some((review, if pressed { k.c.bg_base.alpha(0.8) } else { k.c.text_muted })),
            _ => None,
        };
        let trail = count.map(|(n, c)| text(n.to_string()).font(k.ty.text_xsmall).color(c));
        let (fl, scope) = (id.clone(), scope.to_string());
        r = r.child(chip(f, &format!("board-tab-{}", id.id()), label, pressed, Look::Default, "", None, trail, move |s: &mut State| {
            set_filter(s, home, fl.clone(), &scope)
        }));
    }
    r
}

fn dot(c: rune::Color) -> El {
    el().size(8.0, 8.0).radius(4.0).fill(c).a11y_hidden()
}

/// A chip per device, narrowing to that device's cards (card #22).
pub fn device_bar(f: &Frame, home: bool, on: &Filter, devices: &[DeviceChip], scope: &str) -> Option<El> {
    if devices.is_empty() {
        return None;
    }
    let mut r = row().key("board-devices").role("group").label("Filter cards by device").gap(4.0).center_y().flex_wrap();
    for d in devices {
        let id = Filter::Device(d.id.clone());
        let pressed = *on == id;
        let color = device_color(&d.color).unwrap_or(f.k.c.text_muted);
        let scope = scope.to_string();
        r = r.child(chip(
            f,
            &format!("board-device-{}", d.id),
            &d.name,
            pressed,
            Look::Secondary,
            &format!("Cards its agents are on: {}", d.name),
            Some(dot(color)),
            None,
            move |s: &mut State| set_filter(s, home, if pressed { Filter::All } else { id.clone() }, &scope),
        ));
    }
    Some(r)
}

/// Home's space switcher (card #199): Private first, then one chip per
/// space with a card on Home.
pub fn board_bar(f: &Frame, home: bool, on: &Filter, boards: &[RowBoard], scope: &str) -> Option<El> {
    if boards.is_empty() {
        return None;
    }
    let k = f.k;
    let mut r = row().key("board-spaces").role("group").label("Filter cards by space").gap(4.0).center_y().flex_wrap();
    let pressed = *on == Filter::Private;
    let sc = scope.to_string();
    r = r.child(chip(
        f,
        "board-space-private",
        "Private",
        pressed,
        Look::Secondary,
        "Your private cards: in no shared space",
        None,
        None,
        move |s: &mut State| set_filter(s, home, if pressed { Filter::All } else { Filter::Private }, &sc),
    ));
    for b in boards {
        let id = Filter::Board(b.board_id.clone());
        let pressed = *on == id;
        let name = plain_line(&b.name);
        let tip = if b.tag.is_empty() {
            format!("Cards in the space {name}")
        } else {
            format!("Cards in the space {name} ({}#…)", plain_line(&b.tag))
        };
        let sc = scope.to_string();
        r = r.child(chip(
            f,
            &format!("board-space-{}", b.board_id),
            &name,
            pressed,
            Look::Secondary,
            &tip,
            Some(icon(&icons::SPACES, 14.0).color(k.c.text_base.alpha(0.7))),
            None,
            move |s: &mut State| set_filter(s, home, if pressed { Filter::All } else { id.clone() }, &sc),
        ));
    }
    Some(r)
}

/// A shared board's member chips (card #268): Mine first, then each other
/// member, with their presence dot and line (card #371).
pub fn member_bar(f: &Frame, home: bool, on: &Filter, members: &[SummaryMember], scope: &str) -> Option<El> {
    if members.len() < 2 {
        return None;
    }
    let k = f.k;
    let mut ordered: Vec<&SummaryMember> = members.iter().collect();
    ordered.sort_by_key(|m| !m.me);
    let now = now_ms();
    let mut r = row().key("board-members").role("group").label("Filter cards by member").gap(4.0).center_y().flex_wrap();
    for m in ordered {
        let id = Filter::Owner(m.person_id.clone());
        debug_assert_eq!(id.id(), owner_filter(&m.person_id));
        let pressed = *on == id;
        let name = Some(plain_line(&m.name)).filter(|n| !n.is_empty()).unwrap_or_else(|| "Someone".into());
        let presence = (!m.presence.is_empty()).then(|| presence_line(m, now));
        let mut tip = if m.me {
            "Your cards: made by you or your agents".to_string()
        } else {
            format!("{name}'s cards: made by them or their agents")
        };
        if let Some(p) = &presence {
            tip.push_str(&format!("\n{}: {p}", if m.me { "You" } else { &name }));
        }
        let mut face = el().size(14.0, 14.0).child(icon(&icons::USER, 14.0).color(k.c.text_base.alpha(0.7)));
        if presence.is_some() {
            let c = match m.presence.as_str() {
                "online" => status::working(),
                "recent" => status::decide(),
                _ => status::idle(),
            };
            face = face.child(el().absolute(9.0, 9.0).size(6.0, 6.0).radius(3.0).fill(c).border(1.0, k.c.bg_base).a11y_hidden());
        }
        let sc = scope.to_string();
        r = r.child(chip(
            f,
            &format!("board-member-{}", m.person_id),
            if m.me { "Mine" } else { &name },
            pressed,
            Look::Secondary,
            &tip,
            Some(face),
            None,
            move |s: &mut State| set_filter(s, home, if pressed { Filter::All } else { id.clone() }, &sc),
        ));
    }
    Some(r)
}

/// List or Canvas (card #93), kept in the prefs.
pub fn view_switch(s: &State, f: &Frame) -> El {
    let mut r = row().key("board-views").role("group").label("Board view").gap(4.0).center_y();
    for (v, label, id) in [(View::List, "List", "list"), (View::Canvas, "Canvas", "canvas")] {
        r = r.child(chip(f, &format!("board-view-{id}"), label, s.board.view == v, Look::Secondary, "", None, None, move |s: &mut State| {
            s.board.view = v;
            s.prefs.set(VIEW_PREF, id);
        }));
    }
    r
}
