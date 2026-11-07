//! The window's sidebar (the React app's app-sidebar.tsx): the sections
//! with their counts, the pinned spaces, the agents strip, and the
//! profile, settings and pause row at the foot.

use rune::{El, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Sidebar, SidebarRow, SidebarSection};

use super::{Dialog, Frame, Route, State, icons};

/// The sidebar's width (lib/sidebar-width.ts's default).
pub const WIDTH: f32 = 240.0;

fn count(n: impl Into<i64>) -> Option<String> {
    let n = n.into();
    (n > 0).then(|| n.to_string())
}

fn nav_row(
    s: &State,
    k: &rune_kit::Kit,
    label: &str,
    ic: &'static rune::Icon,
    route: Route,
    badge: Option<String>,
) -> El {
    let on = s.nav.card.is_none() && s.nav.route.section() == route.section();
    let mut r = SidebarRow::new(label)
        .icon(ic)
        .selected(on)
        .on_select(move |s: &mut State, _| {
            s.close_card();
            s.go(route.clone());
        });
    if let Some(b) = badge {
        r = r.badge(b);
    }
    r.build(k)
}

pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let d = &s.data;
    let main = SidebarSection::new("")
        .key("sb-main")
        .row(nav_row(s, k, "Home", &icons::HOME, Route::Home, count(d.decide as i64)))
        .row(nav_row(s, k, "Talk", &icons::TALK, Route::Talk(None), count(crate::talk::waiting_count(s) as i64)))
        .row(nav_row(s, k, "Merge desks", &icons::MERGE, Route::Desk(None), None))
        .row(nav_row(s, k, "Notes", &icons::NOTES, Route::Notes, None))
        .row(nav_row(s, k, "Spaces", &icons::SPACES, Route::Spaces, None))
        .row(nav_row(
            s,
            k,
            "Notifications",
            &icons::BELL,
            Route::Notifications,
            count(d.unread_notifications),
        ));
    let mut sb = Sidebar::new()
        .label("Irori")
        .header(header(s, f))
        .section(main);
    let pinned: Vec<_> = d.spaces.iter().filter(|b| b.pinned).collect();
    if !pinned.is_empty() {
        let mut sec = SidebarSection::new("Pinned").key("sb-pinned");
        for b in pinned {
            let space = b.board_id.clone();
            let on = s.nav.card.is_none()
                && matches!(&s.nav.route, Route::Board { space: sp, .. } if *sp == space);
            sec = sec.row(
                SidebarRow::new(b.name.clone())
                    .icon(&icons::SPACES)
                    .selected(on)
                    .on_select(move |s: &mut State, _| {
                        s.close_card();
                        s.go(Route::Board {
                            space: space.clone(),
                            board: None,
                        });
                    })
                    .build(k),
            );
        }
        sb = sb.section(sec);
    }
    if d.inbox.held > 0 {
        sb = sb.section(
            SidebarSection::new("").key("sb-held").row(
                SidebarRow::new("Held")
                    .icon(&icons::INBOX)
                    .badge(d.inbox.held.to_string())
                    .on_select(|s: &mut State, _| s.open_dialog(Dialog::Held { from: None }))
                    .build(k),
            ),
        );
    }
    let sb = sb
        .child(crate::widgets::agent_strip(s, f))
        .footer(footer(s, f))
        .build(k);
    col()
        .key("sidebar")
        .w(WIDTH)
        .h_full()
        .fill(k.c.bg_deep)
        .border(1.0, k.c.border_muted)
        .child(sb.grow(1.0).min_h(0.0))
}

/// The title row, then the space switcher (card #393) under it.
fn header(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let title = row()
        .key("sb-header")
        .w_full()
        .h(36.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .child(
            text("Irori")
                .font(rune::tok::TextStyle::new(14.0, 650.0))
                .color(k.c.text_base),
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
        );
    col()
        .key("sb-head")
        .w_full()
        .gap(6.0)
        .pb(4.0)
        .child(title)
        .child(crate::spaces::switcher(s, f))
}

fn footer(s: &State, f: &Frame) -> El {
    let k = f.k;
    let name = if s.data.your_name.is_empty() {
        "You".to_string()
    } else {
        s.data.your_name.clone()
    };
    let on_profile = s.nav.card.is_none() && s.nav.route == Route::Profile;
    let paused = s.data.pause.on;
    row()
        .key("sb-footer")
        .w_full()
        .h(40.0)
        .px(4.0)
        .gap(4.0)
        .center_y()
        .child(
            row()
                .key("sb-profile")
                .role("button")
                .label("Profile")
                .focusable(true)
                .grow(1.0)
                .min_w(0.0)
                .h(32.0)
                .px(6.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .fill(if on_profile { k.c.layer_02 } else { k.c.bg_deep })
                .on_press(|s: &mut State| {
                    s.close_card();
                    s.go(Route::Profile);
                })
                .child(icon(&icons::USER, 16.0).color(k.c.icon_muted))
                .child(
                    text(name)
                        .font(k.ty.ui)
                        .color(k.c.text_base)
                        .max_lines(1)
                        .shrink(),
                ),
        )
        .child(
            IconButton::new(if paused { &icons::PLAY } else { &icons::PAUSE })
                .variant(IconButtonVariant::Ghost)
                .label(if paused { "Resume agents" } else { "Pause agents" })
                .help(if paused {
                    "Agents are paused: resume them"
                } else {
                    "Pause every agent's wakes"
                })
                .on_press(move |s: &mut State| {
                    s.act(
                        "Pause agents",
                        move |w| w.agents_pause_set(!paused),
                        |s, v| s.data.pause = v,
                    )
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
