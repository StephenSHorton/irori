//! The huddle's views (huddle.tsx): the panel beside the posts, its tiles
//! and controls, the call bar over every page, the ring banner, the notice
//! before the mic opens, the conversation header's Huddle with its start
//! sheet, the posts a huddle leaves, and the live captions.

use irori_window::call::{CallCaptionView, CallMineView, CallPeerView, CallRingView};
use rune::{Color, El, ScrollSpec, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Dialog, IconButton, IconButtonVariant, Kit, Placement,
    Popover,
};

use super::icons as ic;
use super::logic::{self, Health, StartWhere};
use crate::ui::{Frame, Route, State};

/// The six device hues (index.css's --device-*), for a name's avatar.
fn hue(name: &str) -> Color {
    let (l, c, h) = [
        (0.76, 0.12, 185.0), // teal
        (0.74, 0.14, 50.0),  // orange
        (0.74, 0.14, 350.0), // pink
        (0.70, 0.13, 275.0), // indigo
        (0.80, 0.15, 125.0), // lime
        (0.78, 0.11, 215.0), // cyan
    ][logic::hue_index(name)];
    rune::oklch(l, c, h)
}

fn health_color(k: &Kit, h: Health) -> Color {
    match h {
        Health::Good => k.c.success.icon,
        Health::Fair => k.c.warning.icon,
        Health::Poor => k.c.danger.icon,
        Health::Off => k.c.text_muted.alpha(0.5),
    }
}

fn green(k: &Kit) -> Color {
    k.c.success.icon
}

fn red(k: &Kit) -> Color {
    k.c.danger.icon
}

/// Who a tile shows: a peer, or this device.
pub(super) struct Who {
    pub name: String,
    pub device: String,
    /// How it reaches you, in words; "this device" for this one.
    pub link: String,
    pub health: Option<Health>,
    pub speaking: bool,
    /// The mic is off (muted, or deafened).
    pub muted: bool,
    pub deafened: bool,
    pub sharing: bool,
    /// Whether what they say is written down; None: not known.
    pub transcribed: Option<bool>,
}

fn peer_who(p: &CallPeerView, sharing: bool) -> Who {
    Who {
        name: logic::person_word(&p.name),
        device: logic::name_or(&p.device, ""),
        link: logic::health_word(p),
        health: Some(logic::health_of(p)),
        speaking: p.speaking,
        muted: p.muted,
        deafened: false,
        sharing,
        transcribed: p.transcribing,
    }
}

/// whoIn: everyone in the huddle, you first.
pub(super) fn who_in(m: &CallMineView) -> Vec<Who> {
    let share = m.share.as_ref();
    let mut v = vec![Who {
        name: "You".into(),
        device: m.device.clone(),
        link: "this device".into(),
        health: None,
        speaking: m.speaking,
        muted: m.muted || m.deafened,
        deafened: m.deafened,
        sharing: share.is_some_and(|s| s.mine),
        transcribed: Some(m.transcribing),
    }];
    v.extend(
        m.peers
            .iter()
            .filter(|p| p.phase != "gone")
            .map(|p| peer_who(p, share.is_some_and(|s| logic::shares_from(s, p)))),
    );
    v
}

fn mic_off(m: &CallMineView) -> bool {
    m.muted || m.deafened
}

/// An avatar: the name's initials on its hue, with the link's dot.
pub(super) fn avatar(k: &Kit, key: String, name: &str, health: Option<Health>, small: bool) -> El {
    let (side, font) = if small { (20.0, 9.0) } else { (40.0, 14.0) };
    let letters = crate::profile::avatar::Letters {
        text: logic::initials(name),
        size: font,
        weight: 650.0,
        ink: k.c.bg_base,
    };
    let status = health.map(|h| health_color(k, h));
    crate::profile::avatar::kit_face(name, side, hue(name), letters, None, status)
        .build(k)
        .key(key)
}

/// A call's square icon button: on `fill` with `ink` on the icon (red for
/// the destructive ones, the call's green), or with no fill a ghost one:
/// `ink` the muted or base icon colour, else red.
#[allow(clippy::too_many_arguments)]
pub(super) fn round_button(
    k: &Kit,
    key: String,
    i: &'static rune::Icon,
    large: bool,
    fill: Option<Color>,
    ink: Color,
    label: &str,
    on: impl Fn(&mut State) + 'static,
) -> El {
    let b = IconButton::new(i)
        .control_size(if large {
            ControlSize::ExtraLarge
        } else {
            ControlSize::Regular
        })
        .help(label.to_string());
    // main's small call buttons draw their icon smaller than the kit's 16.
    let b = if large { b } else { b.glyph_size(14.0) };
    let b = match fill {
        Some(c) => b.tint(c, ink),
        None if ink == k.c.icon_muted => b.variant(IconButtonVariant::GhostMuted),
        None if ink == k.c.icon_base => b.variant(IconButtonVariant::Ghost),
        None => b.variant(IconButtonVariant::GhostDanger),
    };
    b.on_press(on).build(k).key(key)
}

fn mute_button(k: &Kit, key: &str, muted: bool, large: bool) -> El {
    let (fill, ink) = if muted {
        (Some(red(k)), k.c.white)
    } else if large {
        (Some(k.c.layer_02), k.c.icon_base)
    } else {
        (None, k.c.icon_muted)
    };
    round_button(
        k,
        format!("{key}-mute"),
        if muted { &ic::MIC_OFF } else { &ic::MIC },
        large,
        fill,
        ink,
        if muted { "Unmute" } else { "Mute" },
        move |s: &mut State| super::set_muted(s, !muted),
    )
}

fn deafen_button(k: &Kit, key: &str, deafened: bool, large: bool) -> El {
    let (fill, ink) = if deafened {
        (Some(red(k)), k.c.white)
    } else if large {
        (Some(k.c.layer_02), k.c.icon_base)
    } else {
        (None, k.c.icon_muted)
    };
    round_button(
        k,
        format!("{key}-deafen"),
        if deafened {
            &ic::HEADPHONE_OFF
        } else {
            &ic::HEADPHONES
        },
        large,
        fill,
        ink,
        if deafened {
            "Undeafen"
        } else {
            "Deafen: hear nothing, mic off"
        },
        move |s: &mut State| super::set_deafened(s, !deafened),
    )
}

/// ErrorLine: why the huddle isn't working.
pub(super) fn error_line(k: &Kit, key: String, t: &str) -> El {
    row()
        .key(key)
        .w_full()
        .gap(6.0)
        .items_start()
        .role("alert")
        .child(icon(&ic::TRIANGLE_ALERT, 14.0).color(red(k)).mt(2.0))
        .child(
            text(t.to_string())
                .font(k.ty.small)
                .color(red(k))
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}

fn speaking(k: &Kit, key: String) -> El {
    let mut r = row()
        .key(key)
        .h(14.0)
        .gap(2.0)
        .items_end()
        .label("speaking");
    for (i, h) in [6.0, 12.0, 9.0, 14.0].into_iter().enumerate() {
        r = r.child(
            el().key(format!("b{i}"))
                .w(3.0)
                .h(h)
                .radius(1.5)
                .fill(green(k)),
        );
    }
    r
}

fn tile(k: &Kit, i: usize, w: &Who) -> El {
    let line: Vec<&str> = [w.device.as_str(), w.link.as_str()]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect();
    let mut name = row().key("name").w_full().gap(6.0).center_y().child(
        text(w.name.clone())
            .font(k.ty.ui_medium)
            .color(k.c.text_base)
            .truncate()
            .shrink(),
    );
    if w.sharing {
        name = name.child(
            icon(&ic::MONITOR_UP, 14.0)
                .color(k.c.info.icon)
                .label("sharing"),
        );
    }
    let mut mid = col().key("who").grow(1.0).min_w(0.0).child(name).child(
        text(line.join(" · "))
            .font(k.ty.text_xsmall)
            .color(k.c.text_muted)
            .truncate()
            .w_full(),
    );
    if w.transcribed == Some(false) {
        mid = mid.child(
            text("not transcribed")
                .font(rune::tok::TextStyle {
                    italic: true,
                    ..k.ty.text_xsmall
                })
                .color(k.c.text_muted)
                .help(if w.name == "You" {
                    "What you say isn't written down".to_string()
                } else {
                    format!("What {} says isn't written down", w.name)
                }),
        );
    }
    let mut t = row()
        .key(format!("call-tile-{i}"))
        .w_full()
        .gap(10.0)
        .padding(8.0)
        .center_y()
        .radius(8.0)
        .fill(k.c.layer_01.alpha(0.6))
        .border(
            1.0,
            if w.speaking {
                green(k)
            } else {
                k.c.border_muted
            },
        )
        .child(avatar(k, "avatar".into(), &w.name, w.health, false))
        .child(mid);
    if let Some(h) = w.health {
        t = t
            .help(format!("{}: {}", w.name, w.link))
            .label(format!("{}: {}", w.name, w.link));
        let _ = h;
    }
    if w.deafened {
        t = t.child(
            icon(&ic::HEADPHONE_OFF, 14.0)
                .color(red(k))
                .label("deafened"),
        );
    } else if w.muted {
        t = t.child(icon(&ic::MIC_OFF, 14.0).color(red(k)).label("muted"));
    }
    if w.speaking && !w.muted {
        t = t.child(speaking(k, "speaking".into()));
    }
    t
}

/// TranscriptOn: the badge while anyone in the huddle is written down.
fn transcript_on_badge(k: &Kit) -> El {
    row()
        .key("call-transcript-on")
        .gap(4.0)
        .px(6.0)
        .h(20.0)
        .center_y()
        .radius(4.0)
        .fill(red(k).alpha(0.14))
        .help("Everyone in the huddle sees this. Each device writes down only its own person's voice.")
        .child(icon(&ic::CIRCLE_DOT, 12.0).color(red(k)))
        .child(text("Transcript on").font(k.ty.text_xsmall_medium).color(red(k)))
}

fn elapsed(s: &State, since: i64) -> String {
    logic::duration_text(since, s.call.now_ms)
}

/// CallPanel: the huddle beside the posts (300 points wide): who's in it,
/// how each device reaches you, the captions, and Mute, Deafen, Share and
/// Leave. Nothing while this device is in no huddle.
pub fn huddle_panel(s: &State, f: &mut Frame) -> El {
    let Some(m) = s.call.view.mine.as_ref() else {
        return el().key("call-panel").w(0.0);
    };
    let k = f.k;
    let who = who_in(m);
    let n = who.len();
    let mut head = row()
        .key("call-panel-head")
        .w_full()
        .px(12.0)
        .pt(12.0)
        .pb(8.0)
        .gap(8.0)
        .center_y()
        .child(icon(&ic::HEADPHONES, 16.0).color(green(k)))
        .child(
            text("Huddle")
                .font(rune::tok::TextStyle::new(13.0, 600.0))
                .color(k.c.text_base),
        )
        .child(
            text(elapsed(s, m.joined_at_ms))
                .font(super::tnum(k.ty.small))
                .color(k.c.text_muted),
        )
        .child(el().grow(1.0))
        .child(
            text(if n == 1 {
                "just you".to_string()
            } else {
                format!("{n} in it")
            })
            .font(k.ty.small)
            .color(k.c.text_muted)
            .truncate(),
        );
    if logic::transcript_on(m) {
        head = head.child(transcript_on_badge(k));
    }
    let mut panel = col()
        .key("call-panel")
        .w(300.0)
        .h_full()
        .min_h(0.0)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_muted)
        .child(head);
    if let Some(notes) = notes(s, f, m) {
        panel = panel.child(notes);
    }
    let mut tiles = col().key("call-tiles").w_full().gap(6.0).px(12.0).pb(12.0);
    for (i, w) in who.iter().enumerate() {
        tiles = tiles.child(tile(k, i, w));
    }
    if n == 1 {
        tiles = tiles.child(
            text("The others here were told it's on; they can join from any of their devices.")
                .key("call-alone")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .w_full()
                .px(4.0)
                .pt(4.0)
                .wrap(),
        );
    }
    let has_captions = !m.captions.is_empty();
    let tiles = col()
        .key("call-tiles-scroll")
        .w_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(tiles);
    panel = if has_captions {
        panel
            .child(tiles.max_h(260.0).shrink())
            .child(captions(s, f, &m.captions))
    } else {
        panel.child(tiles.grow(1.0))
    };
    panel.child(controls(s, f, m, true))
}

/// The panel's notes: the call's error and the share's note line.
fn notes(_s: &State, f: &mut Frame, m: &CallMineView) -> Option<El> {
    let k = f.k;
    if m.error.is_none() && m.share_note.is_none() {
        return None;
    }
    let mut c = col().key("call-notes").w_full().gap(4.0).px(12.0).pb(8.0);
    if let Some(e) = &m.error {
        c = c.child(error_line(k, "call-error".into(), e));
    }
    if let Some(n) = &m.share_note {
        c = c.child(
            row()
                .key("call-share-note")
                .w_full()
                .gap(6.0)
                .items_start()
                .child(icon(&ic::MONITOR_UP, 14.0).color(k.c.text_muted).mt(2.0))
                .child(
                    text(n.clone())
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .wrap(),
                )
                .child(round_button(
                    k,
                    "call-share-note-x".into(),
                    &ic::X,
                    false,
                    None,
                    k.c.icon_muted,
                    "Dismiss",
                    super::share::note_seen,
                )),
        );
    }
    Some(c)
}

/// The panel's controls: Mute, Deafen, Share (a desktop's) and Leave.
fn controls(s: &State, f: &mut Frame, m: &CallMineView, _full: bool) -> El {
    let k = f.k;
    let mut r = row()
        .key("call-controls")
        .w_full()
        .gap(8.0)
        .px(12.0)
        .py(10.0)
        .center()
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_muted)
        .child(mute_button(k, "call-panel", mic_off(m), true))
        .child(deafen_button(k, "call-panel", m.deafened, true));
    if m.can_share {
        r = r.child(super::share::share_button(s, f, m));
    }
    r.child(round_button(
        k,
        "call-panel-leave".into(),
        &ic::PHONE_OFF,
        true,
        Some(red(k)),
        k.c.white,
        "Leave the huddle",
        super::leave,
    ))
}

/// Captions: the transcript as it's written, newest at the bottom; it
/// follows the newest line unless scrolled up.
fn captions(_s: &State, f: &mut Frame, lines: &[CallCaptionView]) -> El {
    let k = f.k;
    let mut list = col()
        .key("call-captions-list")
        .w_full()
        .gap(8.0)
        .px(12.0)
        .py(8.0);
    for (i, l) in lines.iter().enumerate() {
        list = list.child(
            col()
                .key(format!("call-cap-{i}-{}", l.at_ms))
                .w_full()
                .child(
                    row()
                        .key("who")
                        .gap(6.0)
                        .center_y()
                        .child(
                            text(logic::person_word(&l.name))
                                .font(k.ty.small_medium)
                                .color(k.c.text_base),
                        )
                        .child(
                            text(logic::clock_text(l.at_ms))
                                .font(super::tnum(k.ty.text_xsmall))
                                .color(k.c.text_muted),
                        ),
                )
                .child(
                    text(l.text.clone())
                        .font(k.ty.small)
                        .color(k.c.text_weak)
                        .w_full()
                        .wrap(),
                ),
        );
    }
    col()
        .key("call-captions")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_muted)
        .child(
            row()
                .key("call-captions-head")
                .gap(6.0)
                .px(12.0)
                .pt(10.0)
                .center_y()
                .child(icon(&ic::FILE_TEXT, 12.0).color(k.c.text_muted))
                .child(
                    text("TRANSCRIPT, AS IT'S WRITTEN")
                        .font(rune::tok::TextStyle {
                            tracking: 0.4,
                            ..k.ty.text_xsmall_medium
                        })
                        .color(k.c.text_muted),
                ),
        )
        .child(
            col()
                .key("call-captions-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: true })
                .child(list),
        )
}

/// CallBar: the huddle you're in, on every page, at the foot of the
/// sidebar above your name. Its name opens the conversation.
pub(super) fn call_bar(s: &State, f: &mut Frame) {
    let Some(m) = s.call.view.mine.as_ref() else {
        return;
    };
    let k = f.k;
    let where_ = logic::conv_label(&m.conv_kind, &m.conv_name);
    let who = who_in(m);
    let conv = m.conv_id.clone();
    let mut top = row()
        .key("call-bar-top")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(icon(&ic::HEADPHONES, 16.0).color(green(k)))
        .child(
            text(format!("Huddle · {where_}"))
                .key("call-bar-open")
                .role("button")
                .help(format!("Open {where_}"))
                .font(k.ty.small_medium)
                .color(k.c.text_base)
                .truncate()
                .grow(1.0)
                .min_w(0.0)
                .on_press(move |s: &mut State| {
                    s.close_card();
                    s.go(Route::Talk(Some(conv.clone())));
                }),
        );
    if logic::transcript_on(m) {
        top = top.child(
            icon(&ic::CIRCLE_DOT, 12.0)
                .color(red(k))
                .label("Transcript on")
                .help("Transcript on: each device writes down only its own person's voice."),
        );
    }
    top = top.child(
        text(elapsed(s, m.joined_at_ms))
            .font(super::tnum(k.ty.text_xsmall))
            .color(k.c.text_muted),
    );
    let mut faces = row().key("call-bar-faces").center_y();
    for (i, w) in who.iter().take(4).enumerate() {
        let tip = if w.health.is_some() {
            format!("{}: {}", w.name, w.link)
        } else {
            w.name.clone()
        };
        faces = faces.child(
            el().key(format!("f{i}"))
                .ml(if i == 0 { 0.0 } else { -6.0 })
                .radius(10.0)
                .border(1.5, k.c.bg_base)
                .help(tip)
                .child(avatar(k, "a".into(), &w.name, w.health, true)),
        );
    }
    let bottom = row()
        .key("call-bar-bottom")
        .w_full()
        .gap(4.0)
        .center_y()
        .child(faces)
        .child(
            text(if who.len() == 1 {
                "just you".to_string()
            } else {
                format!("{} in it", who.len())
            })
            .font(k.ty.text_xsmall)
            .color(k.c.text_muted)
            .ml(4.0)
            .truncate()
            .grow(1.0)
            .min_w(0.0),
        )
        .child(mute_button(k, "call-bar", mic_off(m), false))
        .child(deafen_button(k, "call-bar", m.deafened, false))
        .child(round_button(
            k,
            "call-bar-leave".into(),
            &ic::PHONE_OFF,
            false,
            None,
            red(k),
            "Leave the huddle",
            super::leave,
        ));
    let mut bar = col()
        .key("call-bar")
        .w(crate::ui::sidebar::WIDTH - 16.0)
        .gap(6.0)
        .padding(8.0)
        .radius(8.0)
        .fill(k.c.bg_deep.mix(green(k), 0.1))
        .border(1.0, green(k).alpha(0.4))
        .child(top);
    if let Some(e) = &m.error {
        bar = bar.child(error_line(k, "call-bar-error".into(), e));
    }
    bar = bar.child(bottom);
    // Above the sidebar's foot (its profile row is 40 points).
    f.layer.push(
        el().key("call-bar-at")
            .pin(Some(8.0), None, None, Some(48.0))
            .child(bar),
    );
}

/// RingBanner: "Spencer started a huddle in #netcode · Join / Dismiss",
/// for the newest ring of a huddle this device isn't in.
/// The newest huddle ringing here that this device isn't in, as a banner
/// over the page (React's Ringing, under the update banner): in the
/// page's flow, so it pushes the page down rather than covering its
/// header. An empty row when nothing rings.
pub(super) fn ring(s: &State, f: &mut Frame) -> El {
    let v = &s.call.view;
    let ring = v
        .ringing
        .iter()
        .filter(|r| v.mine.as_ref().is_none_or(|m| m.call_id != r.call_id))
        .max_by_key(|r| r.at_ms);
    let at = row().key("call-ring-at").w_full().justify_center();
    let Some(r) = ring else { return at };
    let banner = ring_banner(s, f.k, r);
    at.px(16.0).pt(8.0).pb(4.0).child(banner)
}

fn ring_banner(s: &State, k: &Kit, r: &CallRingView) -> El {
    let where_ = if r.conv_kind == "channel" {
        format!("in {}", logic::conv_label(&r.conv_kind, &r.conv_name))
    } else {
        "with you".into()
    };
    let on = if r.by_device.is_empty() {
        String::new()
    } else {
        format!(" on {}", r.by_device)
    };
    let busy = s.call.ring_busy == r.call_id;
    let (id, conv, kind, name) = (
        r.call_id.clone(),
        r.conv_id.clone(),
        r.conv_kind.clone(),
        r.conv_name.clone(),
    );
    let id2 = r.call_id.clone();
    let mut b = col()
        .key("call-ring")
        .role("status")
        .w_full()
        .max_w(640.0)
        .gap(6.0)
        .px(12.0)
        .py(8.0)
        .radius(8.0)
        .fill(k.c.bg_base.mix(green(k), 0.12))
        .border(1.0, green(k).alpha(0.4))
        .box_shadow(rune::Shadow {
            y: 4.0,
            blur: 16.0,
            color: Color::BLACK.alpha(0.18),
            ..Default::default()
        })
        .child(
            row()
                .key("line")
                .w_full()
                .gap(8.0)
                .center_y()
                .child(icon(&ic::HEADPHONES, 16.0).color(green(k)))
                .child(
                    rune::rich(
                        format!(
                            "{} started a huddle {where_}{on}",
                            logic::person_word(&r.by)
                        ),
                        vec![super::bold_span(
                            logic::person_word(&r.by).len(),
                            k.c.text_base,
                        )],
                    )
                    .font(k.ty.ui)
                    .color(k.c.text_weak)
                    .grow(1.0)
                    .min_w(0.0)
                    .wrap(),
                )
                .child(
                    Button::new("Join")
                        .variant(ButtonVariant::Submit)
                        .control_size(ControlSize::Small)
                        .disabled(busy)
                        .on_press(move |s: &mut State| super::ask_join(s, &id, &conv, &kind, &name))
                        .build(k),
                )
                .child(round_button(
                    k,
                    "call-ring-x".into(),
                    &ic::X,
                    false,
                    None,
                    k.c.icon_muted,
                    "Dismiss",
                    move |s: &mut State| super::dismiss_ring(s, &id2),
                )),
        );
    if s.call.ring_error.0 == r.call_id && !s.call.ring_error.1.is_empty() {
        b = b.child(error_line(
            k,
            "call-ring-error".into(),
            &s.call.ring_error.1,
        ));
    }
    b
}

/// NoticeText: the notice's words, with the badge's dot when what you say
/// is written down.
fn notice_text(k: &Kit, key: &str, t: &str) -> El {
    let on = t.starts_with("This huddle is transcribed");
    row()
        .key(key.to_string())
        .w_full()
        .gap(8.0)
        .items_start()
        .child(if on {
            icon(&ic::CIRCLE_DOT, 16.0).color(red(k)).mt(1.0)
        } else {
            icon(&ic::MIC, 16.0).color(k.c.text_muted).mt(1.0)
        })
        .child(
            text(t.to_string())
                .font(k.ty.small)
                .color(k.c.text_muted)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}

/// HuddleNotice: asks before the mic opens (#656). Start or Join runs it;
/// Cancel or Escape just closes it.
pub(super) fn notice(s: &State, f: &mut Frame) {
    let Some(a) = &s.call.notice else { return };
    let k = f.k;
    let t = match s.call.can_transcribe {
        None => "Checking…".to_string(),
        Some(can) => {
            logic::transcript_notice(&a.conv_kind, &a.conv_name, s.call.view.transcribe_on, can)
        }
    };
    let title = format!(
        "{} {} {}",
        if a.start {
            "Start a huddle"
        } else {
            "Join the huddle"
        },
        if a.conv_kind == "channel" {
            "in"
        } else {
            "with"
        },
        logic::conv_label(&a.conv_kind, &a.conv_name)
    );
    let ask = a.clone();
    let checking = s.call.can_transcribe.is_none();
    let confirm = Button::new(if a.start { "Start" } else { "Join" })
        .variant(ButtonVariant::Submit)
        .icon(&ic::HEADPHONES)
        .disabled(checking)
        .on_press(move |s: &mut State| {
            s.call.notice = None;
            let a = ask.clone();
            if a.start {
                super::start(s, &a.conv_id);
            } else {
                // A Join from the ring opens its conversation once in.
                let from_ring = s.call.view.ringing.iter().any(|r| r.call_id == a.call_id);
                super::join(s, a.call_id, a.conv_id, from_ring);
            }
        })
        .build(k)
        .default_button();
    Dialog::new("call-notice", title)
        .open(true)
        .fit()
        .width(420.0)
        .on_close(|s: &mut State| s.call.notice = None)
        .child(
            col()
                .key("call-notice-body")
                .w_full()
                .gap(16.0)
                .px(16.0)
                .pb(16.0)
                .child(notice_text(k, "call-notice-text", &t))
                .child(
                    row()
                        .key("call-notice-foot")
                        .w_full()
                        .gap(8.0)
                        .justify_end()
                        .child(
                            Button::new("Cancel")
                                .variant(ButtonVariant::Ghost)
                                .on_press(|s: &mut State| s.call.notice = None)
                                .build(k),
                        )
                        .child(confirm),
                ),
        )
        .build(k, f.layer);
}

/// HuddleButton: a conversation header's way in. Huddle opens the start
/// sheet, Join when one is on here, or the running time while you're in
/// it. `conv` is the conversation's id; `kind` its kind (`channel`, `dm`)
/// and `name` its name as Talk has them; `space` whether a channel is in a
/// space (a Personal channel isn't); `person` the DM's other person.
pub fn huddle_button(
    s: &State,
    f: &mut Frame,
    conv: &str,
    kind: &str,
    name: &str,
    space: bool,
    person: &str,
) -> El {
    let k = f.k;
    let v = &s.call.view;
    if let Some(m) = v.mine.as_ref().filter(|m| m.conv_id == conv) {
        // A secondary button in the call's green, showing the time.
        return row()
            .key("call-huddle-in")
            .h(24.0)
            .px(8.0)
            .gap(6.0)
            .center_y()
            .radius(6.0)
            .fill(k.c.layer_02)
            .help("You're in this conversation's huddle")
            .child(icon(&ic::HEADPHONES, 14.0).color(green(k)))
            .child(
                text(format!("In huddle · {}", elapsed(s, m.joined_at_ms)))
                    .font(super::tnum(k.ty.small_medium))
                    .color(green(k)),
            );
    }
    if let Some(l) = v
        .live
        .iter()
        .find(|l| l.conv_id == conv && !l.people.is_empty())
    {
        let (id, c, kd, nm) = (
            l.call_id.clone(),
            conv.to_string(),
            kind.to_string(),
            logic::plain_line(name),
        );
        return Button::new(format!("Join · {}", l.people.len()))
            .variant(ButtonVariant::Outline)
            .control_size(ControlSize::Small)
            .icon(&ic::HEADPHONES)
            .disabled(s.call.busy && s.call.at == conv)
            .help("A huddle is on here: join it")
            .on_press(move |s: &mut State| super::ask_join(s, &id, &c, &kd, &nm))
            .build(k)
            .key("call-huddle-join");
    }
    let at = StartWhere {
        label: if kind == "channel" {
            format!("#{}", logic::plain_line(name))
        } else {
            logic::name_or(person, logic::UNKNOWN_PERSON)
        },
        kind: if kind != "channel" {
            "dm".into()
        } else if space {
            "channel".into()
        } else {
            "personal".into()
        },
    };
    let open = s.call.sheet.as_ref().is_some_and(|sh| sh.conv_id == conv);
    let c = conv.to_string();
    let trigger = Button::new("Huddle")
        .variant(if open {
            ButtonVariant::Neutral
        } else {
            ButtonVariant::Ghost
        })
        .control_size(ControlSize::Small)
        .icon(&ic::HEADPHONES)
        .help("Start a voice huddle here")
        .on_press(move |s: &mut State| {
            if s.call.sheet.as_ref().is_some_and(|sh| sh.conv_id == c) {
                s.call.sheet = None;
            } else {
                super::open_sheet(s, &c, at.clone());
            }
        })
        .build(k);
    let mut p = Popover::new("call-sheet")
        .trigger(trigger)
        .open(open)
        .placement(Placement::BottomEnd)
        .padding(16.0)
        .on_close(|s: &mut State| s.call.sheet = None);
    if open && let Some(sh) = &s.call.sheet {
        p = p.child(start_sheet(s, k, sh));
    }
    p.build(k, f.layer)
}

/// StartSheet: who gets told, the mic it will use, and the transcript
/// notice, before anything starts.
fn start_sheet(s: &State, k: &Kit, sh: &super::Sheet) -> El {
    let at = &sh.at;
    let notice = logic::start_notice(at, s.call.view.transcribe_on, s.call.can_transcribe);
    let busy = s.call.busy && s.call.at == sh.conv_id;
    let conv = sh.conv_id.clone();
    let mic = match &sh.mic {
        None => "…".to_string(),
        Some(n) if n.is_empty() => "the default microphone".into(),
        Some(n) => n.clone(),
    };
    let mut c = col()
        .key("call-sheet-body")
        // The popover is at most 320 wide, its 16 inset inside that.
        .w(288.0)
        .gap(12.0)
        .child(
            col()
                .key("head")
                .w_full()
                .gap(4.0)
                .child(
                    row()
                        .key("title")
                        .gap(8.0)
                        .center_y()
                        .child(icon(&ic::HEADPHONES, 16.0).color(k.c.text_base))
                        .child(
                            text(if at.kind == "dm" {
                                format!("Start a huddle with {}", at.label)
                            } else {
                                format!("Start a huddle in {}", at.label)
                            })
                            .font(rune::tok::TextStyle::new(14.0, 600.0))
                            .color(k.c.text_base)
                            .truncate()
                            .min_w(0.0),
                        ),
                )
                .child(
                    text(logic::told_text(at))
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .w_full()
                        .wrap(),
                ),
        )
        .child(
            col()
                .key("mic")
                .w_full()
                .gap(6.0)
                .padding(10.0)
                .radius(8.0)
                .border(1.0, k.c.border_muted)
                .fill(k.c.layer_01.alpha(0.6))
                .child(
                    row()
                        .key("line")
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(icon(&ic::MIC, 14.0).color(k.c.text_muted))
                        .child(text("Microphone").font(k.ty.small).color(k.c.text_muted))
                        .child(el().grow(1.0))
                        .child(
                            text(mic)
                                .font(k.ty.small)
                                .color(k.c.text_base)
                                .truncate()
                                .min_w(0.0),
                        ),
                )
                .child(
                    text("Echo cancelling and noise suppression are on.")
                        .font(k.ty.text_xsmall)
                        .color(k.c.text_muted)
                        .pl(22.0)
                        .w_full()
                        .wrap(),
                ),
        )
        .child(
            el().key("notice")
                .w_full()
                .px(10.0)
                .py(8.0)
                .radius(8.0)
                .border(1.0, k.c.border_muted)
                .child(notice_text(k, "text", &notice)),
        );
    if !s.call.error.is_empty() && s.call.at == sh.conv_id {
        c = c.child(error_line(k, "call-sheet-error".into(), &s.call.error));
    }
    c.child(
        row()
            .key("foot")
            .w_full()
            .gap(8.0)
            .justify_end()
            .child(
                Button::new("Cancel")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut State| s.call.sheet = None)
                    .build(k),
            )
            .child(
                Button::new(if busy { "Starting…" } else { "Start huddle" })
                    .variant(ButtonVariant::Submit)
                    .control_size(ControlSize::Small)
                    .icon(&ic::HEADPHONES)
                    .disabled(busy || s.call.can_transcribe.is_none())
                    .on_press(move |s: &mut State| super::start(s, &conv))
                    .build(k),
            ),
    )
}

/// HuddleLine: a Talk list row's huddle that's on, "Huddle · You,
/// Spencer". None while no huddle is on there.
pub fn huddle_line(s: &State, f: &mut Frame, conv: &str) -> Option<El> {
    let names = logic::huddle_names(&s.call.view, conv);
    if names.is_empty() {
        return None;
    }
    let k = f.k;
    let joined = names.join(", ");
    Some(
        row()
            .key(format!("call-line-{conv}"))
            .min_w(0.0)
            .gap(4.0)
            .center_y()
            .child(icon(&ic::HEADPHONES, 12.0).color(green(k)))
            .child(
                text(format!("Huddle · {joined}"))
                    .font(k.ty.text_xsmall)
                    .color(green(k))
                    .truncate(),
            ),
    )
}

/// The conversation a huddle is in: its id, kind and plain name.
#[derive(Clone, Copy)]
pub struct HuddleIn<'a> {
    pub conv: &'a str,
    pub kind: &'a str,
    pub name: &'a str,
}

/// HuddleStarted: the post a huddle's start leaves; Join while it's on
/// and this device isn't in it. `key` is the post's key.
pub fn huddle_started(
    s: &State,
    f: &mut Frame,
    key: &str,
    by: &str,
    call_id: &str,
    at: HuddleIn,
) -> El {
    let HuddleIn { conv, kind, name } = at;
    let k = f.k;
    let on = s.call.view.live.iter().any(|l| l.call_id == call_id)
        && s.call
            .view
            .mine
            .as_ref()
            .is_none_or(|m| m.call_id != call_id);
    let mut r = row()
        .key(key.to_string())
        .self_stretch()
        .mx(8.0)
        .my(6.0)
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .center_y()
        .radius(8.0)
        .fill(k.c.bg_base.mix(green(k), 0.1))
        .border(1.0, green(k).alpha(0.4))
        .child(icon(&ic::HEADPHONES, 16.0).color(green(k)))
        .child(
            rune::rich(
                format!("{by} started a huddle"),
                vec![super::bold_span(by.len(), k.c.text_base)],
            )
            .font(k.ty.ui)
            .color(k.c.text_weak)
            .grow(1.0)
            .min_w(0.0),
        );
    if on {
        let (id, c, kd, nm) = (
            call_id.to_string(),
            conv.to_string(),
            kind.to_string(),
            name.to_string(),
        );
        r = r.child(
            Button::new("Join")
                .variant(ButtonVariant::Neutral)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut State| super::ask_join(s, &id, &c, &kd, &nm))
                .build(k),
        );
    }
    r
}

/// HuddleEnded: the post a huddle leaves when it ends: how long, who was
/// in it, and its Transcript when `transcript` (it has one).
pub fn huddle_ended(
    f: &mut Frame,
    key: &str,
    duration_ms: i64,
    people: &[String],
    call_id: &str,
    transcript: bool,
) -> El {
    let k = f.k;
    let mut c = col()
        .key(key.to_string())
        .self_stretch()
        .mx(8.0)
        .my(6.0)
        .px(12.0)
        .py(8.0)
        .radius(8.0)
        .fill(k.c.layer_01.alpha(0.6))
        .border(1.0, k.c.border_muted)
        .child(
            row()
                .key("head")
                .w_full()
                .gap(8.0)
                .center_y()
                .child(icon(&ic::HEADPHONES, 16.0).color(k.c.text_muted))
                .child(
                    text("Huddle ended")
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base),
                )
                .child(
                    text(logic::length_text(duration_ms))
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .truncate(),
                ),
        );
    if !people.is_empty() {
        c = c.child(
            text(format!(
                "{} {} in it.",
                logic::names_text(people),
                if people.len() == 1 { "was" } else { "were" }
            ))
            .font(k.ty.small)
            .color(k.c.text_muted)
            .pl(24.0)
            .w_full()
            .wrap(),
        );
    }
    if transcript {
        let id = call_id.to_string();
        c = c.child(
            row().key("tr").pl(24.0).mt(8.0).child(
                Button::new("Transcript")
                    .variant(ButtonVariant::Neutral)
                    .control_size(ControlSize::Small)
                    .icon(&ic::FILE_TEXT)
                    .on_press(move |s: &mut State| super::open_transcript(s, &id))
                    .build(k),
            ),
        );
    }
    c
}

/// The scene's stand-in for Talk's conversation: its header with the
/// Huddle button, the share stage where the posts go (or a few posts and a
/// composer with its mic), an open transcript, and the panel beside it.
pub(super) fn demo(s: &State, f: &mut Frame) {
    let k = f.k;
    let header = row()
        .key("call-demo-head")
        .w_full()
        .h(48.0)
        .px(16.0)
        .gap(8.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_muted)
        .child(
            text("#suzuri")
                .font(rune::tok::TextStyle::new(15.0, 600.0))
                .color(k.c.text_base),
        )
        .child(el().grow(1.0))
        .child(huddle_button(
            s, f, "scene-ch", "channel", "suzuri", true, "",
        ));
    let body = match super::share::share_stage(s, f) {
        Some(stage) => stage,
        None => {
            let you = vec!["you".to_string(), "Spencer".into(), "Aiko".into()];
            let mut posts = col()
                .key("call-demo-posts")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .py(8.0)
                .child(huddle_ended(
                    f,
                    "call-demo-ended",
                    18 * 60_000 + 5_000,
                    &you,
                    "scene-old",
                    true,
                ))
                .child(huddle_started(
                    s,
                    f,
                    "call-demo-started",
                    "Spencer",
                    "scene-call",
                    HuddleIn {
                        conv: "scene-ch",
                        kind: "channel",
                        name: "suzuri",
                    },
                ));
            if let Some(line) = huddle_line(s, f, "scene-ch") {
                posts = posts.child(row().key("call-demo-line").px(16.0).py(4.0).child(line));
            }
            let composer = row()
                .key("call-demo-composer")
                .w_full()
                .gap(6.0)
                .padding(12.0)
                .center_y()
                .child(
                    rune_kit::TextInput::new(&s.call.demo_draft)
                        .placeholder("Message #suzuri")
                        .on_edit(|s: &mut State| &mut s.call.demo_draft)
                        .build(k)
                        .grow(1.0)
                        .key("call-demo-input"),
                )
                .child(super::mic_button(s, f, "call-demo", |s: &mut State| {
                    &mut s.call.demo_draft
                }));
            col()
                .key("call-demo-conv-body")
                .grow(1.0)
                .min_h(0.0)
                .child(posts)
                .child(composer)
        }
    };
    let conv = col()
        .key("call-demo-conv")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .child(header)
        .child(body.grow(1.0).min_h(0.0).w_full());
    let mut page = row()
        .key("call-demo")
        .pin(
            Some(crate::ui::sidebar::WIDTH),
            Some(0.0),
            Some(0.0),
            Some(0.0),
        )
        .fill(k.c.bg_base)
        .child(conv);
    if let Some(t) = super::transcript::transcript_pane(s, f) {
        page = page.child(t);
    }
    let panel = huddle_panel(s, f);
    f.layer.push(page.child(panel));
}
