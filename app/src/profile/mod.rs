//! Profile (P8): the Rune port of the React app's components/profile/*
//! and lib/profile.ts (card #393): you, as everyone you share with sees
//! you: your name (#183), colour (#385) and picture, set once and carried
//! into every space; the people you share spaces with; and your spaces,
//! each with its boards and people.

pub mod avatar;

use std::collections::HashMap;

use irori_window::boards::BoardSummary;
use irori_window::events::EVENT_DEVICES_CHANGED;
use rune::{El, Pixels, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant};

use crate::settings::devices::{DEVICE_COLORS, device_color, pick_person_color};
use crate::settings::people::{NameEdit, load_name, name_field};
use crate::ui::{Event, Frame, Route, State, icons};

/// The sidebar's space pick (the React app's space switcher): "personal",
/// a shared space's id, or none for all spaces.
pub const SPACE_PREF: &str = "sidebar.space";
pub const PERSONAL: &str = "personal";

#[derive(Default)]
pub struct Model {
    pub name: NameEdit,
    pub color: String,
    pub avatar: String,
    /// Pictures by avatar data URL, decoded off the UI thread.
    pub pics: HashMap<String, Pixels>,
    pub error: String,
    pub busy: bool,
    pub loaded: bool,
}

/// Friend is another person you share a space with, and the spaces.
#[derive(Clone, Debug, PartialEq)]
pub struct Friend {
    pub person_id: String,
    pub name: String,
    pub color: String,
    pub avatar: String,
    pub spaces: Vec<String>,
}

/// Everyone else in your spaces, once each, by name: their newest name,
/// colour and avatar as the spaces list them.
pub fn friends_of(boards: &[BoardSummary]) -> Vec<Friend> {
    let mut by: Vec<Friend> = Vec::new();
    for b in boards.iter().filter(|b| !b.leaving) {
        for m in b.members.iter().filter(|m| !m.me) {
            let i = match by.iter().position(|f| f.person_id == m.person_id) {
                Some(i) => i,
                None => {
                    by.push(Friend {
                        person_id: m.person_id.clone(),
                        name: m.name.clone(),
                        color: String::new(),
                        avatar: String::new(),
                        spaces: Vec::new(),
                    });
                    by.len() - 1
                }
            };
            let f = &mut by[i];
            if f.color.is_empty() {
                f.color = m.color.clone();
            }
            if f.avatar.is_empty() {
                f.avatar = m.avatar.clone();
            }
            f.spaces.push(if b.name.is_empty() { b.tag.clone() } else { b.name.clone() });
        }
    }
    by.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    by
}

/// "with Blake", "with Blake and Casey", "with Blake +2" (lib/spaces.ts).
pub fn with_line(others: &[String]) -> String {
    match others {
        [] => String::new(),
        [a] => format!("with {a}"),
        [a, b] => format!("with {a} and {b}"),
        [a, rest @ ..] => format!("with {a} +{}", rest.len()),
    }
}

fn load(s: &mut State) {
    load_name(s, |s| &mut s.profile.name);
    s.spawn(|w| (w.person_color(), w.person_avatar()), |s, (c, a)| {
        s.profile.color = c;
        s.profile.avatar = a;
        s.profile.loaded = true;
        decode_pics(s);
    });
}

/// Decodes the pictures on the page not yet decoded (yours and friends').
fn decode_pics(s: &mut State) {
    let mut want: Vec<String> = vec![s.profile.avatar.clone()];
    for b in &s.data.spaces {
        want.extend(b.members.iter().map(|m| m.avatar.clone()));
    }
    want.retain(|a| avatar::is_avatar(a) && !s.profile.pics.contains_key(a));
    want.sort();
    want.dedup();
    if want.is_empty() {
        return;
    }
    s.spawn(
        move |_| want.into_iter().filter_map(|a| avatar::decode(&a).map(|d| (a, d))).collect::<Vec<_>>(),
        |s, got| {
            for (a, (w, h, px)) in got {
                s.profile.pics.insert(a, avatar::pixels(w, h, px));
            }
        },
    );
}

pub fn on_event(s: &mut State, ev: &Event) {
    let on_page = s.nav.route == Route::Profile && s.nav.card.is_none();
    if (ev.is("app:start") || ev.is("app:route-changed")) && on_page {
        load(s);
    } else if !on_page {
        return;
    }
    if ev.is(EVENT_DEVICES_CHANGED) {
        load(s);
    } else if ev.is(irori_window::boards::EVENT_BOARDS_CHANGED) {
        decode_pics(s);
    }
}

/// The headless scene (`irori --snap x.png --scene profile`).
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Profile);
}

fn set_avatar(s: &mut State, a: String) {
    let a2 = a.clone();
    s.spawn(move |w| w.set_person_avatar(&a2), move |s, r| {
        s.profile.busy = false;
        match r {
            Ok(()) => {
                s.profile.avatar = a;
                s.profile.error.clear();
                decode_pics(s);
            }
            Err(e) => s.profile.error = e,
        }
    });
}

/// A picked or dropped file as your picture.
fn pick_file(s: &mut State, path: std::path::PathBuf) {
    s.profile.busy = true;
    s.spawn(
        move |_| std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| avatar::shrink_avatar(&b)),
        |s, r| match r {
            Ok(a) => set_avatar(s, a),
            Err(e) => {
                s.profile.busy = false;
                s.profile.error = e;
            }
        },
    );
}

fn choose(s: &mut State, tx: &mut Tx) {
    let opts = rune::FileDialogOptions { title: "Your picture".into(), ..Default::default() }.filter("Pictures", &["png", "jpg", "jpeg"]);
    tx.open_file(opts, |s: &mut State, r, _| match r {
        Ok(Some(p)) => pick_file(s, p),
        Ok(None) => {}
        Err(_) => s.profile.error = "Drop a PNG or JPEG on your face to use it as your picture.".into(),
    });
    let _ = s;
}

/// Whose page it is: you, as your profile has you.
pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let p = &s.profile;
    let me = p.name.name.clone().unwrap_or_else(|| s.data.your_name.clone());
    let my_color = device_color(&p.color);
    let my_pic = p.pics.get(&p.avatar);
    let live = s.data.devices.iter().filter(|d| !d.revoked).count();
    let agents = s.data.agents.len();

    // You.
    let has_pic = avatar::is_avatar(&p.avatar);
    let pic_button = col()
        .key("profile-face-button")
        .role("button")
        .label(if has_pic { "Change your picture" } else { "Add a picture" })
        .help(if has_pic { "Change your picture" } else { "Add a picture" })
        .focusable(true)
        .on_press_tx(|s: &mut State, tx: &mut Tx| choose(s, tx))
        .accepts_extensions(&["png", "jpg", "jpeg"])
        .on_drop_files(|s: &mut State, files: &[std::path::PathBuf], _: &mut Tx| {
            if let Some(p) = files.first() {
                pick_file(s, p.clone());
            }
        })
        .child(avatar::face(f, "profile-face".into(), &me, my_color, my_pic, 64.0));
    let mut buttons = row().gap(4.0).pt(4.0).child(
        Button::new(if p.busy {
            "Shrinking…"
        } else if has_pic {
            "Change picture"
        } else {
            "Add a picture"
        })
        .variant(ButtonVariant::Outline)
        .control_size(ControlSize::Small)
        .disabled(p.busy)
        .on_press_tx(|s: &mut State, tx: &mut Tx| choose(s, tx))
        .build(k),
    );
    if has_pic {
        buttons = buttons.child(
            IconButton::new(&icons::TRASH)
                .variant(IconButtonVariant::Ghost)
                .label("Remove your picture")
                .help("Remove your picture")
                .on_press(|s: &mut State| set_avatar(s, String::new()))
                .build(k),
        );
    }
    let who = row()
        .gap(12.0)
        .center_y()
        .child(pic_button)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(if me.is_empty() { "…".into() } else { me.clone() }).font(rune::tok::TextStyle::new(16.0, 600.0)).color(k.c.text_base).truncate())
                .child(
                    text(format!(
                        "{live} {} · {agents} {}",
                        if live == 1 { "device" } else { "devices" },
                        if agents == 1 { "agent" } else { "agents" }
                    ))
                    .font(k.ty.small)
                    .color(k.c.text_muted),
                )
                .child(buttons),
        );
    let mut pal = row().gap(6.0).center_y();
    for (c, _) in DEVICE_COLORS {
        let on = p.color == *c;
        let next = if on { String::new() } else { c.to_string() };
        let mut sw = el()
            .key(format!("profile-color-{c}"))
            .size(24.0, 24.0)
            .radius(12.0)
            .fill(device_color(c).unwrap_or(k.c.layer_03))
            .role("button")
            .label(format!("{c} as your colour"))
            .help(c.to_string())
            .checked(on)
            .focusable(true)
            .on_press(move |s: &mut State| pick_person_color(s, next.clone()));
        if on {
            sw = sw.border(2.0, k.c.text_base);
        }
        pal = pal.child(sw);
    }
    let note = if !p.error.is_empty() {
        text(p.error.clone()).font(k.ty.small).color(k.c.danger.fg).w_full().wrap()
    } else {
        let mut t = "Others see your name, colour and picture everywhere you share: spaces, channels, DMs. Your agents wear the colour as a stripe.".to_string();
        if p.color.is_empty() {
            t.push_str(" With no pick, each friend's screen picks one none of their devices wear.");
        }
        text(t).font(k.ty.small).color(k.c.text_muted).w_full().wrap()
    };
    let mut name_box = col()
        .gap(4.0)
        .child(caps(f, "Your name"))
        .child(name_field(f, "profile-name", &p.name, None, |s| &mut s.profile.name));
    if let Some(e) = &p.name.error {
        name_box = name_box.child(text(e.clone()).font(k.ty.small).color(k.c.danger.fg));
    }
    let you = panel(f, "profile-you")
        .gap(16.0)
        .padding(16.0)
        .child(who)
        .child(name_box)
        .child(col().gap(6.0).child(caps(f, "Your colour")).child(pal))
        .child(note);

    // Friends.
    let friends = friends_of(&s.data.spaces);
    let mut fr = panel(f, "profile-friends").gap(4.0).padding(12.0);
    if friends.is_empty() {
        fr = fr.child(
            text("Nobody yet. Invite someone to a space and they show here.")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .px(4.0)
                .w_full()
                .wrap(),
        );
    }
    for fd in &friends {
        let spaces: Vec<String> = fd.spaces.iter().map(|x| crate::settings::plain_line(x)).collect();
        fr = fr.child(
            row()
                .key(format!("profile-friend-{}", fd.person_id))
                .w_full()
                .gap(10.0)
                .center_y()
                .px(4.0)
                .py(4.0)
                .child(avatar::face(f, format!("profile-friend-face-{}", fd.person_id), &fd.name, device_color(&fd.color), p.pics.get(&fd.avatar), 28.0))
                .child(text(crate::settings::plain_line(&fd.name)).font(k.ty.ui).color(k.c.text_base).truncate().grow(1.0).min_w(0.0))
                .child(text(format!("in {}", spaces.join(", "))).font(k.ty.small).color(k.c.text_muted).truncate().max_w(200.0)),
        );
    }
    fr = fr.child(
        Button::new("+ Invite a friend")
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .on_press(|s: &mut State| s.go(Route::Spaces))
            .build(k)
            .self_start(),
    );

    let left = col()
        .key("profile-left")
        .w(320.0)
        .gap(24.0)
        .child(column(f, "You", you))
        .child(column(f, "Friends", fr));

    // Spaces.
    let current = s.prefs.get::<String>(SPACE_PREF).unwrap_or_default();
    let me_face = avatar::face(f, "profile-me-small".into(), &me, my_color, my_pic, 20.0);
    let mut spaces = col().key("profile-spaces").w_full().gap(16.0).child(space_card(
        f,
        "profile-space-personal",
        "Personal",
        current == PERSONAL,
        PERSONAL.into(),
        row().gap(6.0).center_y().child(me_face).child(text("just you + your agents").font(k.ty.small).color(k.c.text_muted)),
        &[],
        "Everything not shared lives here: your own cards and threads.",
    ));
    let mut shared: Vec<&BoardSummary> = s.data.spaces.iter().filter(|b| !b.leaving).collect();
    shared.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    for b in shared {
        let mut people = row().gap(2.0).center_y();
        for m in &b.members {
            let (name, color, pic) = if m.me {
                (if me.is_empty() { m.name.clone() } else { me.clone() }, my_color, my_pic)
            } else {
                (m.name.clone(), device_color(&m.color), p.pics.get(&m.avatar))
            };
            people = people.child(avatar::face(f, format!("profile-m-{}-{}", b.board_id, m.person_id), &name, color, pic, 20.0).border(2.0, k.c.bg_base));
        }
        let boards: Vec<(String, String)> = if b.boards.is_empty() {
            vec![(b.board_id.clone(), b.name.clone())]
        } else {
            b.boards.iter().map(|x| (x.board_id.clone(), if x.name.is_empty() { x.tag.clone() } else { x.name.clone() })).collect()
        };
        let others: Vec<String> = b.members.iter().filter(|m| !m.me).map(|m| crate::settings::plain_line(&m.name)).collect();
        let note = if b.members.len() > 1 {
            format!(
                "Shared {}. Who's in, and what reaches your agents from them, is set once here, not per board.",
                with_line(&others)
            )
        } else {
            "Only you so far. Invite someone from Space settings.".into()
        };
        let name = crate::settings::plain_line(if b.name.is_empty() { &b.tag } else { &b.name });
        spaces = spaces.child(space_card(
            f,
            &format!("profile-space-{}", b.board_id),
            &name,
            current == b.board_id,
            b.board_id.clone(),
            people,
            &boards.into_iter().map(|(id, n)| (b.board_id.clone(), id, crate::settings::plain_line(&n))).collect::<Vec<_>>(),
            &note,
        ));
    }
    let right = col()
        .key("profile-right")
        .grow(1.0)
        .min_w(0.0)
        .child(column(f, "Spaces: a group of people that holds many boards and channels", spaces));

    col()
        .key("profile-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(crate::ui::page_header(f, "Profile", vec![]))
        .child(
            col()
                .key("profile-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(
                    row()
                        .key("profile-columns")
                        .w_full()
                        .max_w(1100.0)
                        .gap(24.0)
                        .items_start()
                        .px(20.0)
                        .pt(8.0)
                        .pb(32.0)
                        .child(left)
                        .child(right),
                ),
        )
}

/// A small uppercase label (text-[11px] tracking-wide uppercase).
fn caps(f: &Frame, s: &str) -> El {
    text(s.to_uppercase()).font(rune::tok::TextStyle::new(11.0, 500.0)).color(f.k.c.text_muted)
}

/// A glass panel (the React app's glass-panel): a raised card.
fn panel(f: &Frame, key: &str) -> El {
    col().key(key.to_string()).w_full().radius(12.0).fill(f.k.c.layer_01).border(1.0, f.k.c.border_base)
}

fn column(f: &Frame, title: &str, body: El) -> El {
    col()
        .w_full()
        .gap(12.0)
        .child(text(title.to_string()).font(rune::tok::TextStyle::new(15.0, 600.0)).color(f.k.c.text_base).w_full().wrap())
        .child(body)
}

#[allow(clippy::too_many_arguments)]
fn space_card(
    f: &Frame,
    key: &str,
    name: &str,
    current: bool,
    pick: String,
    people: El,
    boards: &[(String, String, String)],
    note: &str,
) -> El {
    let k = f.k;
    let mut card = panel(f, key).gap(12.0).padding(16.0).child(
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .child(text(name.to_string()).font(rune::tok::TextStyle::new(15.0, 600.0)).color(k.c.text_base).truncate().grow(1.0).min_w(0.0))
            .child(people)
            .child(
                Button::new(if current { "Showing" } else { "Show" })
                    .variant(if current { ButtonVariant::Neutral } else { ButtonVariant::Outline })
                    .control_size(ControlSize::Small)
                    .help(if current { "The sidebar shows this space" } else { "Show this space in the sidebar" })
                    .on_press(move |s: &mut State| s.prefs.set(SPACE_PREF, pick.clone()))
                    .build(k),
            ),
    );
    if current {
        card = card.border(1.0, k.alpha(k.c.bg_accent, 0.7));
    }
    if !boards.is_empty() {
        let mut list = col().gap(4.0).child(caps(f, "Boards"));
        for (space, id, n) in boards {
            let (space, id2) = (space.clone(), id.clone());
            list = list.child(
                row()
                    .key(format!("{key}-board-{id}"))
                    .gap(8.0)
                    .center_y()
                    .role("button")
                    .label(n.clone())
                    .focusable(true)
                    .on_press(move |s: &mut State| {
                        let board = (id2 != space).then(|| id2.clone());
                        s.go(Route::Board { space: space.clone(), board })
                    })
                    .child(icon(&icons::GRID, 14.0).color(k.c.text_muted))
                    .child(text(n.clone()).font(k.ty.ui).color(k.c.text_base)),
            );
        }
        card = card.child(list);
    }
    card.child(
        text(note.to_string())
            .font(k.ty.small)
            .color(k.c.text_muted)
            .w_full()
            .wrap()
            .pt(10.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_base),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::boards::SummaryMember;

    fn member(id: &str, name: &str, me: bool, color: &str) -> SummaryMember {
        SummaryMember { person_id: id.into(), name: name.into(), me, color: color.into(), ..Default::default() }
    }

    #[test]
    fn friends_are_everyone_else_once_by_name() {
        let a = BoardSummary {
            board_id: "a".into(),
            name: "Garden".into(),
            members: vec![member("me", "Me", true, ""), member("p2", "Casey", false, ""), member("p1", "blake", false, "teal")],
            ..Default::default()
        };
        let b = BoardSummary {
            board_id: "b".into(),
            tag: "site".into(),
            members: vec![member("p2", "Casey", false, "pink")],
            ..Default::default()
        };
        let gone = BoardSummary { leaving: true, members: vec![member("p3", "Dana", false, "")], ..Default::default() };
        let f = friends_of(&[a, b, gone]);
        assert_eq!(f.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["blake", "Casey"]);
        assert_eq!(f[1].spaces, vec!["Garden", "site"]);
        assert_eq!(f[1].color, "pink");
        assert_eq!(f[0].color, "teal");
    }

    #[test]
    fn with_lines() {
        assert_eq!(with_line(&[]), "");
        assert_eq!(with_line(&["Blake".into()]), "with Blake");
        assert_eq!(with_line(&["Blake".into(), "Casey".into()]), "with Blake and Casey");
        assert_eq!(with_line(&["Blake".into(), "Casey".into(), "Dana".into()]), "with Blake +2");
    }
}
