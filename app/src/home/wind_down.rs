//! Wind down all (the React app's wind-down-dialog.tsx and
//! home/wind-down-line.tsx, cards #220, #484): every agent of the
//! person's saves its place, hands back its cards, releases its leases and
//! stops. Before a run the dialog asks where and whether to turn Pause all
//! on after; during one it lists each agent live, with Stop for one that
//! doesn't answer. Home's one line stands for a whole run.

use irori_window::wind_down::{WindDownRow, WindDownStatus};
use rune::{El, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, ControlSize};

use super::pure::{wind_down_line, wind_down_state};
use crate::logic::cards::Tone;
use crate::logic::names::name_or;
use crate::logic::plain::{plain_block, plain_line};
use super::icons::POWER;
use crate::ui::{Dialog, Frame, State, icons};
use crate::widgets::bits::badge;

/// Where the dismissed run's id is kept (the React app's localStorage key).
pub const SEEN_PREF: &str = "home.windDown.dismissed";

/// The dialog's own state.
#[derive(Clone, Debug)]
pub struct Dlg {
    pub all: bool,
    pub pause_after: bool,
    /// "start", or the row key being stopped.
    pub busy: String,
    pub error: Option<String>,
    /// The run Wind down answered, until the status read has it.
    pub started: Option<WindDownStatus>,
}

impl Default for Dlg {
    fn default() -> Dlg {
        Dlg {
            all: false,
            pause_after: true,
            busy: String::new(),
            error: None,
            started: None,
        }
    }
}

/// Re-reads the run (`windDown:changed`, and when the dialog opens).
pub fn reload(s: &mut State) {
    s.spawn(|w| w.wind_down_status(), |s, st| s.home.wd = st);
}

/// Home's line: the run's summary, opening the dialog, with a Dismiss
/// that holds until the next run.
pub fn line(s: &State, f: &Frame) -> Option<El> {
    let k = f.k;
    let st = &s.home.wd;
    let t = wind_down_line(st);
    if t.is_empty() || s.home.wd_seen == st.id {
        return None;
    }
    let id = st.id.clone();
    Some(
        row()
            .key("home/wd-line")
            .role("status")
            .w_full()
            .px(4.0)
            .gap(8.0)
            .center_y()
            .child(icon(&POWER, 14.0).color(k.c.text_muted))
            .child(
                text(t)
                    .key("home/wd-line/open")
                    .role("button")
                    .focusable(true)
                    .help("Show each agent's answer")
                    .font(rune::tok::TextStyle::new(12.5, 400.0))
                    .color(k.c.text_muted)
                    .max_lines(1)
                    .shrink()
                    .on_press(|s: &mut State| s.open_dialog(Dialog::WindDown)),
            )
            .child(el().grow(1.0))
            .child(
                el().key("home/wd-line/x")
                    .role("button")
                    .label("Dismiss")
                    .help("Dismiss")
                    .focusable(true)
                    .padding(2.0)
                    .radius(4.0)
                    .on_press(move |s: &mut State| {
                        s.home.wd_seen = id.clone();
                        s.prefs.set(SEEN_PREF, id.clone());
                    })
                    .child(icon(&icons::X, 14.0).color(k.c.text_muted)),
            ),
    )
}

fn start(s: &mut State) {
    if s.home.wd_dlg.busy == "start" {
        return;
    }
    s.home.wd_dlg.busy = "start".into();
    s.home.wd_dlg.error = None;
    let scope = if s.home.wd_dlg.all { "all" } else { "this" };
    let pause = s.home.wd_dlg.pause_after;
    s.spawn(
        move |w| w.wind_down_all(scope, pause).map(|st| (st, w.wind_down_status())),
        |s, r| {
            s.home.wd_dlg.busy.clear();
            match r {
                Ok((st, now)) => {
                    s.home.wd_dlg.started = Some(st);
                    s.home.wd = now;
                }
                Err(e) => s.home.wd_dlg.error = Some(e),
            }
        },
    );
}

fn stop(s: &mut State, key: String) {
    s.home.wd_dlg.busy = key.clone();
    s.home.wd_dlg.error = None;
    s.spawn(
        move |w| w.wind_down_stop(&key),
        |s, r| {
            s.home.wd_dlg.busy.clear();
            if let Err(e) = r {
                s.home.wd_dlg.error = Some(e);
            }
        },
    );
}

fn close(s: &mut State) {
    s.close_dialog();
    s.home.wd_dlg = Dlg::default();
}

fn state_tone(state: &str) -> Tone {
    match state {
        "winding" => Tone::Warning,
        "done" => Tone::Success,
        "no_answer" => Tone::Destructive,
        _ => Tone::Secondary,
    }
}

fn agent_row(s: &State, f: &Frame, r: &WindDownRow) -> El {
    let k = f.k;
    let id = if r.key.is_empty() { r.name.clone() } else { r.key.clone() };
    let key = format!("home/wd/row-{id}");
    let mut head = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(name_or(&r.name, "an agent"))
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .max_lines(1)
                .shrink(),
        );
    if !r.device.is_empty() {
        head = head.child(text(plain_line(&r.device)).font(k.ty.text_xsmall).color(k.c.text_muted));
    }
    head = head
        .child(el().grow(1.0))
        .child(badge(f, &format!("{key}/state"), wind_down_state(&r.state), state_tone(&r.state), false, ""));
    if r.state == "waiting" || r.state == "winding" {
        let rk = r.key.clone();
        head = head.child(
            Button::new("Stop")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .disabled(s.home.wd_dlg.busy == r.key)
                .help("Cut its running turn and ask it to wind down now")
                .on_press(move |s: &mut State| stop(s, rk.clone()))
                .build(k),
        );
    }
    let mut c = col()
        .key(key)
        .role("listitem")
        .w_full()
        .gap(4.0)
        .px(12.0)
        .py(8.0)
        .radius(8.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(head);
    if !r.reply.is_empty() {
        c = c.child(
            text(plain_block(&r.reply))
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted)
                .w_full()
                .wrap()
                .selectable(true),
        );
    }
    if !r.error.is_empty() {
        c = c.child(text(r.error.clone()).font(k.ty.text_xsmall).color(k.c.danger.fg).w_full().wrap());
    }
    c
}

/// The dialog (mounted in `f.layer` while `s.dialog` is WindDown).
pub fn dialog(s: &State, f: &mut Frame) {
    let k = f.k;
    let d = &s.home.wd_dlg;
    let st = if s.home.wd.id.is_empty() {
        d.started.as_ref().unwrap_or(&s.home.wd)
    } else {
        &s.home.wd
    };
    let here = s.data.agents.iter().filter(|a| a.here).count();
    let all = s.data.agents.len();
    let n = if d.all { all } else { here };
    let running = !st.id.is_empty() && !st.finished;
    let done = st.rows.iter().filter(|r| r.state == "done").count();

    let mut body = col().key("home/wd/body").w_full().gap(12.0);
    if st.id.is_empty() || st.finished {
        let pick = |label: String, on: bool, all: bool| {
            Button::new(label)
                .variant(if on { ButtonVariant::Contrast } else { ButtonVariant::Ghost })
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut State| s.home.wd_dlg.all = all)
                .build(k)
        };
        let mut pause = row()
            .key("home/wd/pause")
            .w_full()
            .gap(6.0)
            .center_y()
            .child(
                Checkbox::new(d.pause_after)
                    .label("Pause all after, so nothing wakes them until you resume")
                    .on_toggle(|s: &mut State| s.home.wd_dlg.pause_after = !s.home.wd_dlg.pause_after)
                    .build(k),
            );
        if d.all {
            pause = pause.child(text("(this device's wakes)").font(k.ty.text_xsmall).color(k.c.text_muted));
        }
        body = body.child(
            col()
                .key("home/wd/where")
                .w_full()
                .gap(8.0)
                .child(
                    row()
                        .key("home/wd/scope")
                        .role("group")
                        .label("Which agents")
                        .gap(4.0)
                        .child(pick(format!("This device ({here})"), !d.all, false))
                        .child(pick(format!("All my devices ({all})"), d.all, true)),
                )
                .child(pause),
        );
    }
    if !st.id.is_empty() {
        let mut sum = format!("{done} of {} wound down", st.rows.len());
        if st.finished && st.paused {
            sum.push_str(" · Pause all is on");
        }
        if st.finished && !st.pause_after {
            sum.push_str(" · Pause all left as it was");
        }
        let mut list = col()
            .key("home/wd/list")
            .role("list")
            .label("Agents")
            .w_full()
            .gap(4.0);
        for r in &st.rows {
            list = list.child(agent_row(s, f, r));
        }
        body = body
            .child(text(sum).key("home/wd/sum").font(k.ty.ui).color(k.c.text_muted))
            .child(
                col()
                    .key("home/wd/scroll")
                    .w_full()
                    .max_h(420.0)
                    .min_h(0.0)
                    .scroll(rune::ScrollSpec { stick_bottom: false })
                    .child(list),
            );
    }
    if let Some(e) = &d.error {
        body = body.child(
            text(e.clone())
                .key("home/wd/error")
                .role("alert")
                .font(k.ty.ui)
                .color(k.c.danger.fg)
                .w_full()
                .wrap(),
        );
    }
    let mut foot = row()
        .key("home/wd/foot")
        .w_full()
        .gap(8.0)
        .child(el().grow(1.0))
        .child(
            Button::new(if running { "Hide" } else { "Close" })
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(close)
                .build(k),
        );
    if !running {
        let label = if !st.id.is_empty() {
            "Wind down again".to_string()
        } else {
            format!("Wind down {n} agent{}", if n == 1 { "" } else { "s" })
        };
        foot = foot.child(
            Button::new(label)
                .variant(ButtonVariant::Contrast)
                .control_size(ControlSize::Small)
                .disabled(d.busy == "start" || n == 0)
                .on_press(start)
                .build(k)
                .default_button(),
        );
    }
    rune_kit::Dialog::new("home-wd", "Wind down all agents")
        .open(true)
        .width(640.0)
        .description(
            "Each agent finishes the step in hand, pushes its work, posts a hand-off on each card it holds and leaves it, releases its leases and answers you. Nothing is cut: a busy agent stops at its next safe point.",
        )
        .on_close(close)
        .child(body)
        .child(foot)
        .build(k, f.layer);
}
