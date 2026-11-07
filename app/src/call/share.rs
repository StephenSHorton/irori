//! Screen share in a huddle (share.tsx): the stage that takes the posts'
//! place while a share is on (theirs with its picture, or yours with Stop
//! sharing), Full screen, and the huddle's Share with its picker of this
//! device's displays and windows.

use irori_window::call::{CallMineView, CallShareView};
use rune::{El, ScrollSpec, SurfaceFit, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Placement, Popover};

use super::huddle::round_button;
use super::icons as ic;
use super::logic;
use super::video::{self, Look};
use crate::ui::{Frame, State};

/// Shares `source_id` in the huddle; anyone else's share stops (they're
/// told). call:state shows it.
pub fn start(s: &mut State, source_id: &str) {
    s.call.picker.open = false;
    let id = source_id.to_string();
    s.spawn(
        move |w| w.share_start(&id),
        |s, r| {
            if let Err(e) = r {
                s.call.error = e;
            }
        },
    );
}

pub fn stop(s: &mut State) {
    if s.call.demo {
        if let Some(m) = s.call.view.mine.as_mut() {
            m.share = None;
        }
        return;
    }
    s.spawn(|w| w.share_stop(), |_, ()| {});
}

/// The share's note line was dismissed: clear it at once; call:state
/// confirms it.
pub fn note_seen(s: &mut State) {
    if let Some(m) = s.call.view.mine.as_mut() {
        m.share_note = None;
    }
    // Fast: the Core clears it under its lock.
    if let Some(w) = s.w.clone() {
        w.core.share_note_seen();
    }
}

/// Opens the picker and reads what this device can share (the Mac asks for
/// Screen Recording the first time).
fn open_picker(s: &mut State) {
    s.call.picker.open = true;
    s.call.picker.list = None;
    s.call.picker.error.clear();
    s.spawn(
        |w| w.share_sources(),
        |s, r| match r {
            Ok(l) => s.call.picker.list = Some(logic::sort_sources(l)),
            Err(e) => {
                s.call.picker.error = e;
                s.call.picker.list = Some(Vec::new());
            }
        },
    );
}

/// ShareButton: the picker, or Stop sharing while the share is yours.
pub(super) fn share_button(s: &State, f: &mut Frame, m: &CallMineView) -> El {
    let k = f.k;
    let share = m.share.as_ref();
    if share.is_some_and(|x| x.mine) {
        return round_button(
            k,
            "call-share-stop".into(),
            &ic::MONITOR_OFF,
            true,
            Some(k.c.layer_02),
            k.c.info.icon,
            "Stop sharing",
            stop,
        );
    }
    let open = s.call.picker.open;
    let trigger = round_button(
        k,
        "call-share".into(),
        &ic::MONITOR_UP,
        true,
        open.then_some(k.c.layer_02),
        k.c.icon_base,
        "Share a window or screen",
        move |s: &mut State| {
            if s.call.picker.open {
                s.call.picker.open = false;
            } else {
                open_picker(s);
            }
        },
    );
    let theirs = share.filter(|x| !x.mine).map(|x| logic::person_word(&x.name)).unwrap_or_default();
    let mut p = Popover::new("call-picker")
        .trigger(trigger)
        .open(open)
        .placement(Placement::Top)
        .on_close(|s: &mut State| s.call.picker.open = false);
    if open {
        p = p.child(picker(s, k, &theirs));
    }
    p.build(k, f.layer)
}

/// SharePicker: what this device can share, its displays first.
fn picker(s: &State, k: &Kit, theirs: &str) -> El {
    let pk = &s.call.picker;
    let mut c = col()
        .key("call-picker-body")
        .w(320.0)
        .gap(8.0)
        .child(
            row()
                .key("title")
                .gap(8.0)
                .center_y()
                .child(icon(&ic::MONITOR_UP, 16.0).color(k.c.text_base))
                .child(text("Share a window or screen").font(rune::tok::TextStyle::new(14.0, 600.0)).color(k.c.text_base)),
        )
        .child(
            text(if theirs.is_empty() {
                "Everyone in the huddle sees only what you pick, nothing else on your screen.".to_string()
            } else {
                format!("{theirs} is sharing now. Sharing yours replaces theirs; they're told.")
            })
            .font(k.ty.small)
            .color(k.c.text_muted)
            .w_full()
            .wrap(),
        );
    if !pk.error.is_empty() {
        c = c.child(text(pk.error.clone()).role("alert").font(k.ty.small).color(k.c.danger.icon).w_full().wrap());
    }
    match &pk.list {
        None => c.child(text("Looking for windows…").font(k.ty.small).color(k.c.text_muted).py(8.0)),
        Some(l) if l.is_empty() && pk.error.is_empty() => {
            c.child(text("Nothing here to share.").font(k.ty.small).color(k.c.text_muted).py(8.0))
        }
        Some(l) => {
            let mut rows = col().key("call-picker-rows").w_full().gap(2.0);
            for src in l {
                let id = src.id.clone();
                rows = rows.child(
                    row()
                        .key(format!("call-src-{}", src.id))
                        .role("button")
                        .label(src.title.clone())
                        .focusable(true)
                        .w_full()
                        .gap(10.0)
                        .px(8.0)
                        .py(6.0)
                        .radius(6.0)
                        .center_y()
                        .hover(k.c.hover)
                        .press(k.c.pressed)
                        .on_press(move |s: &mut State| start(s, &id))
                        .child(
                            icon(if src.kind == "display" { &ic::MONITOR } else { &ic::APP_WINDOW }, 16.0)
                                .color(k.c.text_muted),
                        )
                        .child(
                            col()
                                .key("t")
                                .grow(1.0)
                                .min_w(0.0)
                                .child(text(src.title.clone()).font(k.ty.small).color(k.c.text_base).truncate().w_full())
                                .child(
                                    text(logic::source_line(src))
                                        .font(k.ty.text_xsmall)
                                        .color(k.c.text_muted)
                                        .truncate()
                                        .w_full(),
                                ),
                        ),
                );
            }
            c.child(
                col()
                    .key("call-picker-scroll")
                    .w_full()
                    .max_h(288.0)
                    .min_h(0.0)
                    .scroll(ScrollSpec { stick_bottom: false })
                    .child(rows),
            )
        }
    }
}

/// ShareStage: the share on in the huddle this device is in, for the posts'
/// place; None while no share is on.
pub fn share_stage(s: &State, f: &mut Frame) -> Option<El> {
    let share = s.call.view.mine.as_ref()?.share.as_ref()?;
    if s.call.full && !share.mine {
        // It fills the window (full_screen draws it).
        return Some(el().key("call-stage").grow(1.0));
    }
    Some(if share.mine { my_share(f.k, share) } else { their_share(s, f.k, share, false) })
}

/// The stage over everything while Full screen is on; Escape leaves it.
pub(super) fn full_screen(s: &State, f: &mut Frame) {
    if !s.call.full {
        return;
    }
    let Some(share) = s.call.view.mine.as_ref().and_then(|m| m.share.as_ref()).filter(|x| !x.mine) else {
        return;
    };
    let stage = their_share(s, f.k, share, true);
    f.layer.push(
        el().key("call-full")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .fill(f.k.c.bg_base)
            .modal(false, |s: &mut State| s.call.full = false)
            .child(stage.w_full().h_full()),
    );
}

fn their_share(s: &State, k: &Kit, share: &CallShareView, full: bool) -> El {
    let name = logic::person_word(&share.name);
    let who = if share.device.is_empty() { name.clone() } else { format!("{name} on {}", share.device) };
    let head = row()
        .key("call-stage-head")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(icon(&ic::MONITOR_UP, 16.0).color(k.c.info.icon))
        .child(
            rune::rich(
                format!("{who} {}", logic::what_text(share)),
                vec![super::bold_span(who.len(), k.c.text_base)],
            )
            .font(k.ty.ui)
            .color(k.c.text_weak)
            .truncate()
            .grow(1.0)
            .min_w(0.0),
        )
        .child(
            text(logic::quality_text(share))
                .font(super::tnum(k.ty.text_xsmall))
                .color(k.c.text_muted)
                .help(format!("{} × {}, about {} kbps", share.width, share.height, share.kbps)),
        )
        .child(round_button(
            k,
            "call-stage-full".into(),
            if full { &ic::MINIMIZE } else { &ic::MAXIMIZE },
            false,
            None,
            k.c.icon_muted,
            if full { "Leave full screen (Esc)" } else { "Full screen" },
            |s: &mut State| s.call.full = !s.call.full,
        ));
    let mut c = col()
        .key(if full { "call-stage-full-body" } else { "call-stage" })
        .label("Shared screen")
        .grow(1.0)
        .min_h(0.0)
        .gap(8.0)
        .padding(12.0)
        .fill(k.c.bg_base)
        .child(head)
        .child(picture(s, k));
    if !full {
        c = c.child(
            text(format!(
                "Your view only: {name} chose {}, and nothing else on their screen is sent. Sharing your own replaces theirs after they're told.",
                if share.kind == "display" { "one screen" } else { "one window" }
            ))
            .font(k.ty.text_xsmall)
            .color(k.c.text_muted)
            .w_full()
            .wrap(),
        );
    }
    c
}

/// SharePicture: the decoded picture, fitted inside the stage.
fn picture(s: &State, k: &Kit) -> El {
    let mut box_ = el()
        .key("call-picture")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .radius(8.0)
        .clip_rounded(8.0)
        .fill(crate::ui::theme::hex("#0c0c0c"))
        .border(1.0, k.c.border_muted)
        .center();
    let look = if !video::can_decode() {
        Look::Unsupported
    } else {
        s.call.viewer.as_ref().map_or(Look::Waiting, |v| v.look())
    };
    match (look, s.call.viewer.as_ref()) {
        (Look::Playing, Some(v)) => {
            box_ = box_.child(el().key("call-pixels").w_full().h_full().pixels_fit(&v.pixels, SurfaceFit::Fit));
        }
        (Look::Unsupported, _) => {
            box_ = box_.child(
                text("This device can't show shared screens (no video decoder).")
                    .font(k.ty.small)
                    .color(crate::ui::theme::hex("#cccccc"))
                    .px(16.0),
            );
        }
        _ => {
            box_ = box_.child(text("Waiting for the picture…").font(k.ty.small).color(crate::ui::theme::hex("#9d9d9d")));
        }
    }
    box_
}

/// MyShare: your own share, no picture of your screen, just what's sent
/// and Stop sharing.
fn my_share(k: &Kit, share: &CallShareView) -> El {
    let from = if share.device.is_empty() { String::new() } else { format!("From {}. ", share.device) };
    col()
        .key("call-stage")
        .label("Your shared screen")
        .grow(1.0)
        .min_h(0.0)
        .gap(12.0)
        .padding(24.0)
        .center()
        .child(
            el().key("call-stage-icon")
                .size(48.0, 48.0)
                .radius(24.0)
                .fill(k.c.info.icon.alpha(0.15))
                .center()
                .child(icon(&ic::MONITOR_UP, 24.0).color(k.c.info.icon)),
        )
        .child(
            col()
                .key("call-stage-words")
                .max_w(448.0)
                .gap(4.0)
                .items_center()
                .child(
                    text(format!("You're {}", logic::what_text(share)))
                        .font(rune::tok::TextStyle::new(14.0, 600.0))
                        .color(k.c.text_base)
                        .wrap(),
                )
                .child(
                    text(format!(
                        "{from}Everyone in the huddle sees {} and nothing else on your screen. {}.",
                        if share.kind == "display" { "this screen" } else { "this window" },
                        logic::quality_text(share)
                    ))
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .wrap(),
                ),
        )
        .child(
            Button::new("Stop sharing")
                .variant(ButtonVariant::Danger)
                .control_size(ControlSize::Small)
                .icon(&ic::MONITOR_OFF)
                .on_press(stop)
                .build(k),
        )
}
