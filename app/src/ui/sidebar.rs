//! The window's sidebar (the React app's app-sidebar.tsx): the sections
//! with their counts, the pinned spaces, the agents strip, and the
//! profile, settings and pause row at the foot.

use rune::{El, col, el, row, text};
use rune_kit::{IconButton, IconButtonVariant, Sidebar, SidebarRow};

use super::{Dialog, Frame, Route, State, icons};
use crate::logic::cards::Tone;
use crate::logic::sidebar as logic;

/// The sidebar's width (lib/sidebar-width.ts's default).
pub const WIDTH: f32 = 240.0;
/// Folded to icons (lib/sidebar-width.ts foldedWidth).
pub const FOLDED_WIDTH: f32 = 56.0;
/// The prefs key that keeps the fold (localStorage irori.sidebarFolded).
pub const FOLDED_PREF: &str = "sidebar.folded";

/// The sidebar is folded to icons.
pub fn folded(s: &State) -> bool {
    s.prefs.get_or(FOLDED_PREF, false)
}

/// Folds the sidebar to icons, or opens it again (remembered).
pub fn set_folded(s: &mut State, on: bool) {
    if on {
        s.prefs.set(FOLDED_PREF, true);
    } else {
        s.prefs.remove(FOLDED_PREF);
    }
}

/// The top button that folds the sidebar to icons or opens it again.
fn fold_button(k: &rune_kit::Kit, folded: bool) -> El {
    IconButton::new(&icons::PANEL_LEFT)
        .variant(IconButtonVariant::Ghost)
        .label(if folded { "Open the sidebar" } else { "Fold the sidebar" })
        .help(if folded { "Open the sidebar" } else { "Fold the sidebar to icons" })
        .on_press(move |s: &mut State| set_folded(s, !folded))
        .build(k)
        .key("sb-fold")
}

/// How a count chip by a section reads (app-sidebar.tsx NavItem's
/// countTone): the primary "default", or one of the badge tones.
#[derive(Clone, Copy)]
enum CountTone {
    Primary,
    Tone(Tone),
}

/// A section's count chip, with what it counts as its tooltip.
fn count_chip(f: &Frame, key: &str, n: usize, tone: CountTone, tip: &str) -> El {
    let k = f.k;
    match tone {
        CountTone::Tone(t) => crate::widgets::bits::badge(f, key, &n.to_string(), t, false, tip),
        CountTone::Primary => row()
            .key(key.to_string())
            .h(20.0)
            .px(7.0)
            .center_y()
            .radius(5.0)
            .fill(k.c.text_base)
            .help(tip.to_string())
            .child(text(n.to_string()).font(k.ty.text_xsmall_medium).color(k.c.page)),
    }
}

struct Nav<'a> {
    key: &'a str,
    label: &'a str,
    icon: &'static rune::Icon,
    current: bool,
    count: usize,
    tone: CountTone,
    tip: &'a str,
}

fn nav_row<F>(f: &Frame, n: Nav, on: F) -> El
where
    F: Fn(&mut State) + 'static,
{
    let mut r = SidebarRow::new(n.label)
        .icon(n.icon)
        .selected(n.current)
        .height(36.0)
        .on_select(move |s: &mut State, _| on(s));
    if n.count > 0 {
        r = r.trailing(count_chip(f, &format!("sb-count-{}", n.key), n.count, n.tone, n.tip));
    }
    r.build(f.k).key(format!("sb-nav-{}", n.key))
}

/// A section's own row: current when it is the page showing (no card open).
fn here(s: &State, route: &Route) -> bool {
    s.nav.card.is_none() && s.nav.route.section() == route.section()
}

fn go(route: Route) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        s.close_card();
        s.go(route.clone());
    }
}

/// A group's heading (app-sidebar.tsx Group): small, uppercase, muted.
fn group_label(f: &Frame, key: &str, t: &str) -> El {
    text(t.to_uppercase())
        .key(key.to_string())
        .font(rune::tok::TextStyle::new(11.0, 500.0).tracking(0.4))
        .color(f.k.c.text_muted)
        .px(10.0)
        .pt(16.0)
        .pb(4.0)
}

/// A pinned space or card (app-sidebar.tsx PinRow): its mark in a mono
/// chip, then its name.
fn pin_row<F>(f: &Frame, key: String, mark: &str, label: &str, tip: &str, current: bool, on: F) -> El
where
    F: Fn(&mut State) + 'static,
{
    let k = f.k;
    let r = row()
        .key(key)
        .role("button")
        .label(tip.to_string())
        .help(tip.to_string())
        .focusable(true)
        .w_full()
        .min_w(0.0)
        .h(32.0)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(k.radius.md)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut State| on(s))
        .child(
            row()
                .h(20.0)
                .min_w(24.0)
                .px(4.0)
                .center()
                .radius(4.0)
                .fill(k.c.layer_02)
                .child(
                    text(mark.to_string())
                        .font(rune::tok::TextStyle::new(10.5, 400.0).mono())
                        .color(k.c.text_muted)
                        .max_lines(1),
                ),
        )
        .child(
            text(label.to_string())
                .font(if current { rune::tok::TextStyle { weight: 500.0, ..k.ty.ui } } else { k.ty.ui })
                .color(if current { k.c.text_base } else { k.c.text_muted })
                .max_lines(1)
                .grow(1.0)
                .shrink(),
        );
    let r = k.interactive(r);
    if current { r.fill(k.c.pressed) } else { r }
}

pub fn view(s: &State, f: &mut Frame) -> El {
    if folded(s) {
        return folded_view(s, f);
    }
    let space = crate::spaces::current(s);
    let d = &s.data;
    let desks = s.desk.desks.iter().filter(|x| crate::desk::queue::desk_shown(x)).count();
    let need_you = logic::boards_need_you(&d.cards, &space);
    let fr: &Frame = f;
    let mut rows = vec![
        nav_row(fr, Nav { key: "home", label: "Home", icon: &icons::HOME, current: here(s, &Route::Home), count: 0, tone: CountTone::Primary, tip: "" }, go(Route::Home)),
        nav_row(
            fr,
            Nav {
                key: "talk",
                label: "Talk",
                icon: &icons::TALK,
                current: here(s, &Route::Talk(None)),
                count: crate::talk::waiting_count(s),
                tone: CountTone::Primary,
                tip: "Threads an agent answered and DMs you haven't read",
            },
            go(Route::Talk(None)),
        ),
    ];
    // Merge desks shows while a desk has an open PR of ours (card #625).
    if desks > 0 {
        rows.push(nav_row(
            fr,
            Nav { key: "desk", label: "Merge desks", icon: &icons::MERGE, current: here(s, &Route::Desk(None)), count: 0, tone: CountTone::Primary, tip: "" },
            go(Route::Desk(None)),
        ));
    }
    rows.push(nav_row(
        fr,
        Nav { key: "notes", label: "Notes", icon: &icons::NOTES, current: here(s, &Route::Notes), count: 0, tone: CountTone::Primary, tip: "" },
        go(Route::Notes),
    ));
    rows.push(nav_row(
        fr,
        Nav {
            key: "spaces",
            label: "Spaces",
            icon: &icons::SPACES,
            current: here(s, &Route::Spaces),
            count: need_you,
            tone: CountTone::Tone(Tone::Destructive),
            tip: "Cards in your spaces waiting on you",
        },
        go(Route::Spaces),
    ));
    rows.push(nav_row(
        fr,
        Nav {
            key: "notifications",
            label: "Notifications",
            icon: &icons::BELL,
            current: here(s, &Route::Notifications),
            count: d.unread_notifications as usize,
            tone: CountTone::Tone(Tone::Info),
            tip: "Notifications you haven't read",
        },
        go(Route::Notifications),
    ));
    rows.push(nav_row(
        fr,
        Nav {
            key: "held",
            label: "Held",
            icon: &icons::INBOX,
            current: false,
            count: d.inbox.held as usize,
            tone: CountTone::Tone(Tone::Warning),
            tip: "Messages Irori is holding back",
        },
        |s: &mut State| s.open_dialog(Dialog::Held { from: None }),
    ));

    let pinned = logic::pinned_spaces(&d.spaces, &space);
    if !pinned.is_empty() {
        rows.push(group_label(fr, "sb-pinned-spaces", "Pinned spaces"));
        for b in pinned {
            let id = b.board_id.clone();
            let current = s.nav.card.is_none() && matches!(&s.nav.route, Route::Board { space: sp, .. } if *sp == id);
            let p = logic::space_pin(b);
            rows.push(pin_row(fr, format!("sb-pin-space-{id}"), &p.mark, &p.label, &p.tip, current, move |s: &mut State| {
                s.close_card();
                s.go(Route::Board { space: id.clone(), board: None });
            }));
        }
    }
    let cards = logic::pinned_cards(&d.cards);
    if !cards.is_empty() {
        rows.push(group_label(fr, "sb-pinned-cards", "Pinned cards"));
        for c in cards {
            let id = c.id.clone();
            let current = s.nav.card.as_deref() == Some(c.id.as_str());
            let p = logic::card_pin(c);
            rows.push(pin_row(fr, format!("sb-pin-card-{id}"), &p.mark, &p.label, &p.tip, current, move |s: &mut State| s.open_card(&id)));
        }
    }

    let k = f.k;
    let mut sb = Sidebar::new().label("Sections").row_gap(2.0).header_padding(0.0, 0.0).header(header(s, f));
    for r in rows {
        sb = sb.child(r);
    }
    let sb = sb.footer(crate::widgets::agent_strip(s, f)).footer(footer(s, f)).build(k);
    col()
        .key("sidebar")
        .w(WIDTH)
        .h_full()
        .fill(k.c.bg_deep)
        .border(1.0, k.c.border_muted)
        .child(sb.grow(1.0).min_h(0.0))
}

/// The title row ("Irori 囲炉裏" and the palette and New card buttons),
/// then the space switcher (card #393) under it.
fn header(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let title = row()
        .key("sb-header")
        .w_full()
        .h(48.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .child(
            row()
                .key("sb-title")
                .gap(8.0)
                .center_y()
                .child(text("Irori").font(rune::tok::TextStyle::new(18.0, 600.0)).color(k.c.text_base))
                .child(text("囲炉裏").font(rune::tok::TextStyle::new(14.0, 400.0)).color(k.c.text_muted)),
        )
        .child(el().grow(1.0))
        .child(
            IconButton::new(&icons::SEARCH)
                .variant(IconButtonVariant::Ghost)
                .label("Search and commands")
                .help("Search and commands (⌘K)")
                .on_press(|s: &mut State| s.open_dialog(Dialog::Palette))
                .build(k),
        )
        .child(
            IconButton::new(&icons::PLUS)
                .variant(IconButtonVariant::Ghost)
                .label("New card")
                .help("New card")
                .on_press(|s: &mut State| s.open_dialog(Dialog::NewCard { board: None }))
                .build(k),
        )
        .child(fold_button(k, false));
    col()
        .key("sb-head")
        .w_full()
        .gap(4.0)
        .pb(8.0)
        .child(title)
        .child(crate::spaces::switcher(s, f))
}

/// You (your face, name and this device: it opens your profile), Pause
/// agents (the React app's header switch: the Rune window has no header
/// bar) and Settings.
fn footer(s: &State, f: &Frame) -> El {
    let k = f.k;
    let name = if s.data.your_name.is_empty() { "You".to_string() } else { s.data.your_name.clone() };
    let this = s.data.devices.iter().find(|d| d.this);
    let device = this.map(|d| d.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "this device".into());
    let device_id = this.map(|d| d.device_id.clone()).unwrap_or_default();
    let on_profile = s.nav.card.is_none() && s.nav.route == Route::Profile;
    let paused = s.data.pause.on;
    let profile = row()
        .key("sb-profile")
        .role("button")
        .label("Profile")
        .help("Your profile: name, colour and picture, and your spaces")
        .focusable(true)
        .grow(1.0)
        .min_w(0.0)
        .px(6.0)
        .py(4.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .cursor(rune::Cursor::Pointer)
        .on_press(|s: &mut State| {
            s.close_card();
            s.go(Route::Profile);
        })
        .child(crate::widgets::faces::my_face(s, f, "sb-face", crate::widgets::faces::PersonSize::Md))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(text(name).font(k.ty.text_xsmall_medium).color(k.c.text_base).max_lines(1))
                .child(text(device).font(k.ty.text_xsmall).color(k.c.text_muted).max_lines(1).help(device_id)),
        );
    let profile = k.interactive(profile);
    let profile = if on_profile { profile.fill(k.c.layer_02) } else { profile };
    row()
        .key("sb-footer")
        .w_full()
        .pt(8.0)
        .gap(4.0)
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_muted)
        .child(profile)
        .child(
            IconButton::new(if paused { &icons::PLAY } else { &icons::PAUSE })
                .variant(IconButtonVariant::Ghost)
                .label(if paused { "Resume agents" } else { "Pause agents" })
                .help(if paused { "Agents are paused: resume them" } else { "Pause every agent's wakes" })
                .on_press(move |s: &mut State| {
                    s.act("Pause agents", move |w| w.agents_pause_set(!paused), |s, v| s.data.pause = v)
                })
                .build(k),
        )
        .child(
            IconButton::new(&icons::SETTINGS)
                .variant(IconButtonVariant::Ghost)
                .label("Settings")
                .help("Settings")
                .on_press(|s: &mut State| s.open_dialog(Dialog::Settings(String::new())))
                .build(k),
        )
}

/// One section as an icon (folded): its count as a dot in its tone.
fn folded_item<F>(f: &Frame, key: &str, label: &str, ic: &'static rune::Icon, current: bool, count: usize, dot: rune::Color, on: F) -> El
where
    F: Fn(&mut State) + 'static,
{
    let k = f.k;
    let name = if count > 0 { format!("{label} ({count})") } else { label.to_string() };
    let mut b = row()
        .key(format!("sb-fold-{key}"))
        .role("button")
        .label(name)
        .help(label.to_string())
        .focusable(true)
        .w_full()
        .h(36.0)
        .center()
        .radius(k.radius.md)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut State| on(s))
        .child(rune::icon(ic, 16.0).color(if current { k.c.icon_base } else { k.c.icon_muted }));
    if count > 0 {
        b = b.child(el().absolute(30.0, 6.0).size(6.0, 6.0).radius(3.0).fill(dot));
    }
    let b = k.interactive(b);
    if current { b.fill(k.c.pressed) } else { b }
}

/// The sidebar folded to icons (app-sidebar.tsx with `folded`): the open
/// button, the switcher's mark, each section with a dot for its count,
/// the pins' marks, then your face and Settings. No Agents section.
fn folded_view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let space = crate::spaces::current(s);
    let d = &s.data;
    let desks = s.desk.desks.iter().filter(|x| crate::desk::queue::desk_shown(x)).count();
    let need_you = logic::boards_need_you(&d.cards, &space);
    let mut list = col().key("sb-folded-list").w_full().gap(2.0).px(8.0).pb(12.0).grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false });
    let fr: &Frame = f;
    let primary = k.c.text_base;
    list = list
        .child(folded_item(fr, "home", "Home", &icons::HOME, here(s, &Route::Home), 0, primary, go(Route::Home)))
        .child(folded_item(fr, "talk", "Talk", &icons::TALK, here(s, &Route::Talk(None)), crate::talk::waiting_count(s), primary, go(Route::Talk(None))));
    if desks > 0 {
        list = list.child(folded_item(fr, "desk", "Merge desks", &icons::MERGE, here(s, &Route::Desk(None)), 0, primary, go(Route::Desk(None))));
    }
    list = list
        .child(folded_item(fr, "notes", "Notes", &icons::NOTES, here(s, &Route::Notes), 0, primary, go(Route::Notes)))
        .child(folded_item(fr, "spaces", "Spaces", &icons::SPACES, here(s, &Route::Spaces), need_you, k.c.danger.fg, go(Route::Spaces)))
        .child(folded_item(
            fr,
            "notifications",
            "Notifications",
            &icons::BELL,
            here(s, &Route::Notifications),
            d.unread_notifications as usize,
            k.c.info.fg,
            go(Route::Notifications),
        ))
        .child(folded_item(fr, "held", "Held", &icons::INBOX, false, d.inbox.held as usize, k.c.warning.fg, |s: &mut State| {
            s.open_dialog(Dialog::Held { from: None })
        }));
    let mark = |key: String, m: &str, tip: &str, current: bool| {
        let r = row()
            .key(key)
            .role("button")
            .label(tip.to_string())
            .help(tip.to_string())
            .focusable(true)
            .w_full()
            .h(32.0)
            .center()
            .radius(k.radius.md)
            .cursor(rune::Cursor::Pointer)
            .child(
                row().h(20.0).min_w(24.0).max_w(40.0).px(4.0).center().radius(4.0).fill(k.c.layer_02).clip_rounded(4.0).child(
                    text(m.to_string()).font(rune::tok::TextStyle::new(10.5, 400.0).mono()).color(k.c.text_muted).truncate(),
                ),
            );
        let r = k.interactive(r);
        if current { r.fill(k.c.pressed) } else { r }
    };
    let pinned = logic::pinned_spaces(&d.spaces, &space);
    let cards = logic::pinned_cards(&d.cards);
    for (i, group) in [pinned.len(), cards.len()].into_iter().enumerate() {
        if group > 0 {
            list = list.child(el().key(format!("sb-fold-rule-{i}")).w_full().h(1.0).mt(8.0).mb(6.0).fill(k.c.border_muted));
        }
        if i == 0 {
            for b in &pinned {
                let id = b.board_id.clone();
                let current = s.nav.card.is_none() && matches!(&s.nav.route, Route::Board { space: sp, .. } if *sp == id);
                let p = logic::space_pin(b);
                list = list.child(mark(format!("sb-pin-space-{id}"), &p.mark, &p.tip, current).on_press(move |s: &mut State| {
                    s.close_card();
                    s.go(Route::Board { space: id.clone(), board: None });
                }));
            }
        } else {
            for c in &cards {
                let id = c.id.clone();
                let current = s.nav.card.as_deref() == Some(c.id.as_str());
                let p = logic::card_pin(c);
                list = list.child(mark(format!("sb-pin-card-{id}"), &p.mark, &p.tip, current).on_press(move |s: &mut State| s.open_card(&id)));
            }
        }
    }
    let on_profile = s.nav.card.is_none() && s.nav.route == Route::Profile;
    let face = row()
        .key("sb-profile")
        .role("button")
        .label("Your profile")
        .help("Your profile: name, colour and picture, and your spaces")
        .focusable(true)
        .padding(4.0)
        .radius(6.0)
        .cursor(rune::Cursor::Pointer)
        .on_press(|s: &mut State| {
            s.close_card();
            s.go(Route::Profile);
        })
        .child(crate::widgets::faces::my_face(s, f, "sb-face", crate::widgets::faces::PersonSize::Md));
    let face = k.interactive(face);
    let face = if on_profile { face.fill(k.c.layer_02) } else { face };
    let footer = col()
        .key("sb-footer")
        .w_full()
        .py(12.0)
        .gap(4.0)
        .center()
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_muted)
        .child(face)
        .child(
            IconButton::new(&icons::SETTINGS)
                .variant(IconButtonVariant::Ghost)
                .label("Settings")
                .help("Settings")
                .on_press(|s: &mut State| s.open_dialog(Dialog::Settings(String::new())))
                .build(k),
        );
    col()
        .key("sidebar")
        .w(FOLDED_WIDTH)
        .h_full()
        .fill(k.c.bg_deep)
        .border(1.0, k.c.border_muted)
        .child(row().key("sb-header").w_full().h(48.0).center().child(fold_button(k, true)))
        .child(el().w_full().px(8.0).pb(8.0).child(crate::spaces::switcher_folded(s, f, true)))
        .child(list)
        .child(footer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::prefs::Prefs;

    #[test]
    fn the_fold_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = State { prefs: Prefs::load(dir.path()), ..State::default() };
        assert!(!folded(&s), "open by default");
        set_folded(&mut s, true);
        let t = State { prefs: Prefs::load(dir.path()), ..State::default() };
        assert!(folded(&t), "a restart finds it folded");
        set_folded(&mut s, false);
        assert!(!folded(&s));
    }
}
