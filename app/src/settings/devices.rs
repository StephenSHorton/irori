//! Settings > Devices and connections (devices-panel.tsx, lib/devices.ts;
//! docs/design/v0.4-devices.md §3, §7.1, §9): each device, this one
//! first, with one status dot, when it was last seen, at most one warning
//! line, Rename, its colour and Remove; the sides this device runs; your
//! colour in shared spaces; the home server and public relays, with the
//! advanced relay; then Pair a device (the code, the words the new device
//! shows typed here, this device's key typed there) and Join with a code.
//!
//! Pairing goes through the window's pairing seam (`w.seams.pairing`, the
//! old Tauri devices_* commands); its calls block on the pairing runtime,
//! so they run off the UI thread, and `devices:pairing` re-reads it.

use std::collections::HashMap;

use irori_window::devices::{DeviceView, HomeServerView, PairStatus, RelayView};
use irori_window::events::{EVENT_DEVICES_CHANGED, EVENT_DEVICES_PAIRING, EVENT_ROSTER_CHANGED};
use rune::{Color, El, TextEdit, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Switch, TextInput};

use crate::ui::{Event, Frame, State};

use super::{Act, Confirm, choice, confirm_button, error_line, hint, loading, muted_line, ruled, warn_line};

// ---- lib/devices.ts ------------------------------------------------------------------

/// coord::devname::MAX_LEN.
pub const MAX_DEVICE_NAME: usize = 40;
/// coord::sides::MAX_LEN and MAX_SIDES.
pub const MAX_SIDE: usize = 40;
pub const MAX_SIDES: usize = 8;

/// The colours a device (or a person) can wear (internal/devcolor.Palette),
/// with the window's shade of each (index.css --device-*).
pub const DEVICE_COLORS: &[(&str, (f32, f32, f32))] = &[
    ("teal", (0.76, 0.12, 185.0)),
    ("orange", (0.74, 0.14, 50.0)),
    ("pink", (0.74, 0.14, 350.0)),
    ("lime", (0.8, 0.15, 125.0)),
    ("cyan", (0.78, 0.11, 215.0)),
    ("indigo", (0.7, 0.13, 275.0)),
    ("sand", (0.78, 0.06, 80.0)),
];

/// A device colour's shade; None for "" or one unknown.
pub fn device_color(name: &str) -> Option<Color> {
    DEVICE_COLORS.iter().find(|(n, _)| *n == name).map(|(_, (l, c, h))| rune::color::oklch(*l, *c, *h))
}

/// coord::devname::clean: one line, trimmed, at most MAX_DEVICE_NAME
/// characters.
pub fn clean_device_name(name: &str) -> String {
    let one = name.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(MAX_DEVICE_NAME).collect()
}

pub fn clean_side(side: &str) -> String {
    clean_device_name(side)
}

/// coord::sides::same: case doesn't count.
pub fn same_side(a: &str, b: &str) -> bool {
    clean_side(a).to_lowercase() == clean_side(b).to_lowercase()
}

/// Each side cleaned, empty ones and repeats (any case) dropped, order kept.
pub fn unique_sides<S: AsRef<str>>(sides: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in sides {
        let c = clean_side(s.as_ref());
        if !c.is_empty() && !out.iter().any(|o| same_side(o, &c)) {
            out.push(c);
        }
    }
    out
}

/// coord::sides::clean_list: unique_sides, at most MAX_SIDES.
pub fn clean_sides<S: AsRef<str>>(sides: &[S]) -> Vec<String> {
    let mut v = unique_sides(sides);
    v.truncate(MAX_SIDES);
    v
}

/// What this device's pin means.
pub fn sides_line(pinned: &[String]) -> String {
    if pinned.is_empty() {
        return "This device runs every side. Pick the sides it runs to keep work and personal cards apart; every device still shows every card.".into();
    }
    let list = if pinned.len() == 1 {
        pinned[0].clone()
    } else {
        format!("{} and {}", pinned[..pinned.len() - 1].join(", "), pinned[pinned.len() - 1])
    };
    format!(
        "This device runs {list}, and cards with no side. New cards typed here go on {}. Every device still shows every card.",
        pinned[0]
    )
}

/// The warning under the home server: with public relays on, "can't be
/// reached from outside" says the relays cover it (issue 9).
pub fn home_line(hs: &HomeServerView) -> String {
    let line = super::plain_line(&hs.line);
    if !hs.public_relays {
        return line;
    }
    let relays = "Your home server can't be reached from outside your home, but public relays connect your devices and invited people from any network.";
    match hs.reach.as_str() {
        "cgnat" => relays.into(),
        "forward" => {
            // Keep the router steps, for anyone who wants a direct connection too.
            let steps = line
                .strip_prefix("Away from home your devices can't reach this PC.")
                .map(str::trim_start)
                .unwrap_or(&line);
            if !steps.is_empty() && steps != line {
                format!("{relays} For a direct connection: {steps}")
            } else {
                relays.into()
            }
        }
        _ => line,
    }
}

/// The pairing states (internal/pair).
pub mod pair_state {
    pub const WAITING: &str = "waiting";
    pub const CONFIRM: &str = "confirm";
    pub const GRANTED: &str = "granted";
    pub const WORDS: &str = "words";
    pub const DONE: &str = "done";
    pub const CANCELED: &str = "canceled";
    pub const EXPIRED: &str = "expired";
}

/// A pairing is still in progress.
pub fn is_open(s: &PairStatus) -> bool {
    matches!(s.state.as_str(), pair_state::WAITING | pair_state::CONFIRM | pair_state::GRANTED | pair_state::WORDS)
}

/// A device's state after its name: "this device", "online", "seen 3m ago"…
pub fn state_line(d: &DeviceView, now_ms: i64) -> String {
    let seen = (!d.last_seen.is_zero()).then(|| d.last_seen.unix_milli_value());
    if d.this {
        "this device".into()
    } else if d.revoked {
        "removed".into()
    } else if d.state == "connected" {
        "online".into()
    } else if let (true, Some(t)) = (d.state == "recent", seen) {
        format!("seen {}", super::relative_time(t, now_ms))
    } else if let Some(t) = seen {
        format!("offline · seen {}", super::relative_time(t, now_ms))
    } else {
        "offline · never seen".into()
    }
}

// ---- state ------------------------------------------------------------------------------

#[derive(Default)]
pub struct Model {
    pub devices: Option<Vec<DeviceView>>,
    pub err: Option<String>,
    /// The device being renamed, its field, and why a save failed.
    pub renaming: Option<String>,
    pub rename: TextEdit,
    pub rename_err: String,
    pub rename_busy: bool,
    /// The device whose colours are open.
    pub color_open: Option<String>,
    pub color_err: HashMap<String, String>,
    pub side_names: Vec<String>,
    pub side_add: TextEdit,
    pub sides_err: String,
    pub sides_busy: bool,
    pub person_color: Option<String>,
    pub person_err: String,
    pub home: Option<HomeServerView>,
    pub home_failed: bool,
    pub home_err: String,
    pub public_addr: TextEdit,
    pub advanced: bool,
    pub relay: Option<RelayView>,
    pub relay_editing: bool,
    pub relay_url: TextEdit,
    pub relay_err: String,
    pub pair: Option<PairStatus>,
    /// The new device's role: "desktop" or "node".
    pub role: String,
    pub joining: bool,
    pub code: TextEdit,
    pub away: bool,
    pub server: TextEdit,
    pub pair_busy: bool,
    pub pair_err: String,
    pub copied: bool,
    pub words: TextEdit,
    pub words_err: String,
    pub key: TextEdit,
    pub key_err: String,
}

pub fn load(s: &mut State) {
    load_devices(s);
    s.spawn(
        |w| (w.side_names(), w.person_color(), w.relay(), w.seams.pairing.status()),
        |s, (names, color, relay, pair)| {
            let d = &mut s.settings.devices;
            d.side_names = names;
            d.person_color = Some(color);
            d.relay = Some(relay);
            // A pairing that had already ended when the dialog opened isn't
            // shown again.
            d.pair = is_open(&pair).then_some(pair);
        },
    );
    load_home(s);
}

pub fn load_devices(s: &mut State) {
    s.spawn(|w| w.devices(), |s, v| {
        s.data.devices = v.clone();
        s.settings.devices.devices = Some(v);
        s.settings.devices.err = None;
    });
}

fn load_home(s: &mut State) {
    s.spawn(|w| w.home_server(), |s, hs| {
        s.settings.devices.home = Some(hs);
    });
}

/// Every few seconds: liveness changes without an event.
pub fn tick(s: &mut State) {
    load_devices(s);
    load_home(s);
}

fn load_pair(s: &mut State) {
    s.spawn(|w| w.seams.pairing.status(), |s, p| {
        let d = &mut s.settings.devices;
        // An ended pairing shows until OK; one that ended unseen stays away.
        if d.pair.is_some() || is_open(&p) {
            d.pair = Some(p);
        }
    });
}

pub fn on_event(s: &mut State, ev: &Event) {
    if super::open_pane(s) != Some("devices") {
        return;
    }
    if ev.is(EVENT_DEVICES_PAIRING) {
        load_pair(s);
        load_devices(s);
    } else if ev.is(EVENT_DEVICES_CHANGED) || ev.is(EVENT_ROSTER_CHANGED) {
        load_devices(s);
    }
}

// ---- the pane ---------------------------------------------------------------------------

pub fn view(s: &State, f: &mut Frame) -> El {
    let m = &s.settings.devices;
    let mut out = col().key("settings-devices").w_full().gap(12.0);
    match &m.devices {
        None => out = out.child(loading(f, "settings-devices-loading", "Loading your devices")),
        Some(list) => {
            let mut ul = col().key("settings-device-list").w_full().gap(6.0).role("list").label("Your devices");
            for d in list {
                ul = ul.child(device_row(s, f, d));
            }
            out = out.child(ul);
            if let Some(e) = &m.err {
                out = out.child(error_line(f, e));
            }
            if let Some(here) = list.iter().find(|d| d.this) {
                out = out.child(sides(s, f, here));
            }
        }
    }
    out = out.child(person_color(s, f)).child(home_server(s, f));
    let pairing = match &m.pair {
        Some(p) if !p.state.is_empty() => pairing(s, f, p),
        _ => start(s, f),
    };
    out.child(ruled(f, "settings-pairing", pairing)).child(hint(
        f,
        "Devices find each other on your local network, through your home server, and with public relays on, through iroh's free relays, which see which devices connect but never what they say. Others on a network see that sealed messages pass, never their content, and a relay you add under Advanced sees IP addresses. What reaches another device stays there, so a secret an agent wrote that the scrubber missed is copied for good.",
    ))
}

fn dot(f: &Frame, state: &str) -> Color {
    match state {
        "this" | "connected" => crate::ui::theme::status::working(),
        "recent" => crate::ui::theme::status::decide(),
        _ => f.k.alpha(f.k.c.text_muted, 0.4),
    }
}

fn device_row(s: &State, f: &Frame, d: &DeviceView) -> El {
    let k = f.k;
    let m = &s.settings.devices;
    let name = super::plain_line(&d.name);
    let shown = if name.is_empty() { d.device_id.clone() } else { name.clone() };
    let renaming = m.renaming.as_deref() == Some(d.device_id.as_str());
    let mut top = row()
        .w_full()
        .gap(10.0)
        .center_y()
        .child(el().size(8.0, 8.0).radius(4.0).fill(dot(f, if d.revoked { "offline" } else { &d.state })));
    if renaming {
        return col()
            .key(format!("settings-device-{}", d.device_id))
            .w_full()
            .child(top.child(rename_field(s, f, d)));
    }
    let mut small = String::new();
    if d.role == "node" {
        small.push_str("node · ");
    }
    small.push_str(&state_line(d, s.settings.now_ms));
    if !d.via.is_empty() {
        small.push_str(&format!(" · through {}", super::plain_line(&d.via)));
    }
    if !d.sides.is_empty() {
        small.push_str(&format!(" · runs {}", d.sides.join(", ")));
    }
    let mut name_el = text(shown).font(k.ty.ui).color(k.c.text_base);
    if d.revoked {
        let n = name_el_len(&name, &d.device_id);
        name_el = name_el.strike(0, n, None);
    }
    top = top.child(
        row()
            .grow(1.0)
            .min_w(0.0)
            .gap(6.0)
            .flex_wrap()
            .child(name_el)
            .child(text(small).font(k.ty.small).color(k.c.text_muted).wrap_anywhere().min_w(0.0)),
    );
    if !d.revoked {
        let id = d.device_id.clone();
        let mut actions = row().gap(8.0).center_y().child(
            Button::new("Rename")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut State| {
                    let dm = &mut s.settings.devices;
                    let name = dm.devices.as_ref().and_then(|l| l.iter().find(|x| x.device_id == id)).map(|x| x.name.clone());
                    dm.rename = TextEdit::new(name.unwrap_or_default());
                    dm.renaming = Some(id.clone());
                    dm.rename_err.clear();
                })
                .build(k),
        );
        actions = actions.child(color_pick(s, f, d));
        if !d.this {
            let who = if name.is_empty() { "this device".to_string() } else { name.clone() };
            actions = actions.child(confirm_button(
                s,
                f,
                Confirm {
                    id: format!("remove-{}", d.device_id),
                    question: format!(
                        "Remove {who}? It keeps everything it already has. Removing it stops it from syncing and from writing anything new that your other devices will accept, and gives your remaining devices a new person key."
                    ),
                    label: "Remove device".into(),
                    danger: true,
                    act: Act::RemoveDevice(d.device_id.clone()),
                },
                "Remove",
                false,
            ));
        }
        top = top.child(actions);
    }
    let mut out = col().key(format!("settings-device-{}", d.device_id)).w_full().gap(2.0).child(top);
    if !d.warning.is_empty() {
        let mut w = row().pl(18.0).gap(8.0).center_y().child(warn_line(f, &super::plain_line(&d.warning)).grow(1.0));
        if d.this && d.action == "reset" {
            w = w.child(confirm_button(
                s,
                f,
                Confirm {
                    id: "reset-device".into(),
                    question: "Reset this device? Its event log, device id and keys move aside (nothing is deleted) and only the settings stay. Irori quits; open it again to pair this device as a new one.".into(),
                    label: "Reset and quit".into(),
                    danger: true,
                    act: Act::ResetDevice,
                },
                "Reset",
                false,
            ));
        }
        out = out.child(w);
    }
    out
}

fn name_el_len(name: &str, id: &str) -> usize {
    if name.is_empty() { id.len() } else { name.len() }
}

/// RenameField: the name shows on all the person's devices, never on a
/// shared board; empty goes back to the computer's name.
fn rename_field(s: &State, f: &Frame, d: &DeviceView) -> El {
    let k = f.k;
    let m = &s.settings.devices;
    let current = d.name.clone();
    let id = d.device_id.clone();
    let save = move |s: &mut State| {
        let dm = &mut s.settings.devices;
        if dm.rename_busy {
            return;
        }
        let name = dm.rename.text.chars().take(MAX_DEVICE_NAME).collect::<String>();
        if name.trim() == current.trim() {
            dm.renaming = None;
            return;
        }
        dm.rename_busy = true;
        let id = id.clone();
        s.spawn(move |w| w.rename_device(&id, &name), |s, r| {
            let dm = &mut s.settings.devices;
            dm.rename_busy = false;
            match r {
                Ok(_) => {
                    dm.renaming = None;
                    dm.rename_err.clear();
                    load_devices(s);
                }
                Err(e) => dm.rename_err = e,
            }
        });
    };
    let save2 = save.clone();
    col()
        .grow(1.0)
        .min_w(0.0)
        .gap(4.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    TextInput::new(&m.rename)
                        .label("Device name")
                        .placeholder("The computer's name")
                        .width(Some(220.0))
                        .on_edit(|s: &mut State| &mut s.settings.devices.rename)
                        .on_submit(move |s: &mut State, _: &mut Tx| save(s))
                        .on_key(|s: &mut State, ev: &rune::KeyEvent, _: &mut Tx| {
                            if ev.key == rune::Key::Escape {
                                s.settings.devices.renaming = None;
                                return true;
                            }
                            false
                        })
                        .build(k)
                        .key("settings-rename-field"),
                )
                .child(
                    Button::new("Save")
                        .control_size(ControlSize::Small)
                        .variant(ButtonVariant::Contrast)
                        .disabled(m.rename_busy)
                        .on_press(move |s: &mut State| save2(s))
                        .build(k),
                )
                .child(
                    Button::new("Cancel")
                        .control_size(ControlSize::Small)
                        .variant(ButtonVariant::Ghost)
                        .on_press(|s: &mut State| s.settings.devices.renaming = None)
                        .build(k),
                ),
        )
        .child(if m.rename_err.is_empty() {
            hint(f, "Your devices show this name. Leave it empty for the computer's name.")
        } else {
            error_line(f, &m.rename_err)
        })
}

/// A round colour button (the palettes' swatches).
fn swatch(f: &Frame, key: String, color: Option<Color>, size: f32, on: bool, label: String) -> El {
    let k = f.k;
    let mut e = el()
        .key(key)
        .size(size, size)
        .radius(size / 2.0)
        .role("button")
        .label(label.clone())
        .help(label)
        .focusable(true)
        .checked(on);
    e = match color {
        Some(c) => e.fill(c),
        None => e.border(1.0, k.c.border_base),
    };
    if on { e.border(2.0, k.c.text_base) } else { e }
}

/// ColorPick: a device's colour (card #22), its swatch opening the palette.
fn color_pick(s: &State, f: &Frame, d: &DeviceView) -> El {
    let m = &s.settings.devices;
    let name = {
        let n = super::plain_line(&d.name);
        if n.is_empty() { "this device".to_string() } else { n }
    };
    let mut out = row().gap(4.0).center_y();
    if m.color_open.as_deref() == Some(d.device_id.as_str()) {
        for (c, _) in DEVICE_COLORS {
            let (id, c2) = (d.device_id.clone(), c.to_string());
            out = out.child(
                swatch(f, format!("settings-dc-{}-{c}", d.device_id), device_color(c), 16.0, d.color == *c, format!("{c} for {name}"))
                    .on_press(move |s: &mut State| {
                        let (id, c) = (id.clone(), c2.clone());
                        let id2 = id.clone();
                        s.spawn(move |w| w.set_device_color(&id, &c), move |s, r| match r {
                            Ok(()) => {
                                s.settings.devices.color_open = None;
                                s.settings.devices.color_err.remove(&id2);
                                load_devices(s);
                            }
                            Err(e) => {
                                s.settings.devices.color_err.insert(id2, e);
                            }
                        });
                    }),
            );
        }
    } else {
        let id = d.device_id.clone();
        let label = format!(
            "{name}'s colour: {}. Change it",
            if d.color.is_empty() { "default" } else { &d.color }
        );
        out = out.child(
            swatch(f, format!("settings-dc-{}", d.device_id), device_color(&d.color), 16.0, false, label)
                .border(1.0, f.k.c.border_base)
                .on_press(move |s: &mut State| s.settings.devices.color_open = Some(id.clone())),
        );
    }
    if let Some(e) = m.color_err.get(&d.device_id) {
        out = out.child(error_line(f, e).w(160.0));
    }
    out
}

/// Sides (card #494): the sides this device runs, picked here and only here.
fn sides(s: &State, f: &mut Frame, here: &DeviceView) -> El {
    let k = f.k;
    let m = &s.settings.devices;
    let pinned = here.sides.clone();
    let mut all_in = pinned.clone();
    all_in.extend(m.side_names.iter().cloned());
    let all = unique_sides(&all_in);
    let full = pinned.len() >= MAX_SIDES;
    let busy = m.sides_busy;
    let mut sec = col()
        .gap(6.0)
        .child(text("Work and personal sides").font(k.ty.ui).color(k.c.text_base));
    if !all.is_empty() {
        let mut group = row().gap(6.0).flex_wrap().role("group").label("Sides this device runs");
        for side in &all {
            let runs = pinned.iter().any(|p| same_side(p, side));
            let (p2, s2) = (pinned.clone(), side.clone());
            group = group.child(
                Button::new(side.clone())
                    .variant(if runs { ButtonVariant::Contrast } else { ButtonVariant::Outline })
                    .control_size(ControlSize::Small)
                    // A device runs at most MAX_SIDES: a ninth would be cut.
                    .disabled(busy || (full && !runs))
                    .on_press(move |s: &mut State| {
                        let next = if runs {
                            p2.iter().filter(|p| !same_side(p, &s2)).cloned().collect()
                        } else {
                            let mut v = p2.clone();
                            v.push(s2.clone());
                            v
                        };
                        save_sides(s, next);
                    })
                    .build(k)
                    .key(format!("settings-side-{side}"))
                    .checked(runs),
            );
        }
        sec = sec.child(group);
    }
    if pinned.len() > 1 {
        let choices: Vec<(String, String)> = pinned.iter().map(|p| (p.clone(), p.clone())).collect();
        let p2 = pinned.clone();
        sec = sec.child(
            col()
                .gap(4.0)
                .child(hint(f, "New cards typed here go on"))
                .child(choice(s, f, "settings-side-first", "New cards typed here go on", &pinned[0], &choices, 224.0, busy, move |s, v| {
                    let mut next = vec![v.to_string()];
                    next.extend(p2.iter().filter(|p| !same_side(p, v)).cloned());
                    save_sides(s, next);
                })),
        );
    }
    let add = move |s: &mut State| {
        let c: String = clean_side(&s.settings.devices.side_add.text).chars().take(MAX_SIDE).collect();
        if c.is_empty() {
            return;
        }
        s.settings.devices.side_add = TextEdit::new("");
        let here = s.settings.devices.devices.as_ref().and_then(|l| l.iter().find(|d| d.this)).map(|d| d.sides.clone()).unwrap_or_default();
        if !here.iter().any(|p| same_side(p, &c)) {
            let mut v = here;
            v.push(c);
            save_sides(s, v);
        }
    };
    let add2 = add;
    sec = sec.child(
        row()
            .gap(8.0)
            .center_y()
            .child(
                TextInput::new(&m.side_add)
                    .label("Name a side this device runs")
                    .placeholder(if all.is_empty() { "Name a side, like Work" } else { "Another side" })
                    .width(Some(224.0))
                    .disabled(busy || full)
                    .on_edit(|s: &mut State| &mut s.settings.devices.side_add)
                    .on_submit(move |s: &mut State, _: &mut Tx| add(s))
                    .build(k)
                    .key("settings-side-add"),
            )
            .child(
                Button::new("Add")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .disabled(busy || full || clean_side(&m.side_add.text).is_empty())
                    .on_press(move |s: &mut State| add2(s))
                    .build(k),
            ),
    );
    sec = sec.child(if m.sides_err.is_empty() { hint(f, &sides_line(&pinned)) } else { error_line(f, &m.sides_err) });
    ruled(f, "settings-sides", sec)
}

fn save_sides(s: &mut State, next: Vec<String>) {
    let next = clean_sides(&next);
    s.settings.devices.sides_busy = true;
    s.spawn(
        move |w| w.set_device_sides(&next).map(|_| (w.devices(), w.side_names())),
        |s, r| {
            let dm = &mut s.settings.devices;
            dm.sides_busy = false;
            match r {
                Ok((devs, names)) => {
                    dm.sides_err.clear();
                    dm.side_names = names;
                    dm.devices = Some(devs.clone());
                    s.data.devices = devs;
                }
                Err(e) => dm.sides_err = e,
            }
        },
    );
}

/// PersonColor: your colour on shared boards (card #385).
fn person_color(s: &State, f: &Frame) -> El {
    let k = f.k;
    let m = &s.settings.devices;
    let color = m.person_color.clone().unwrap_or_default();
    let mut pal = row().gap(4.0).center_y();
    for (c, _) in DEVICE_COLORS {
        let on = color == *c;
        let next = if on { String::new() } else { c.to_string() };
        pal = pal.child(
            swatch(f, format!("settings-pc-{c}"), device_color(c), 16.0, on, format!("{c} in shared spaces"))
                .on_press(move |s: &mut State| pick_person_color(s, next.clone())),
        );
    }
    let line = if !m.person_err.is_empty() {
        error_line(f, &m.person_err)
    } else if !color.is_empty() {
        hint(f, "Friends see you and your agents in it. Click it again for the default.")
    } else {
        hint(f, "Friends see you and your agents in a colour none of their devices wear. Pick one to choose it.")
    };
    ruled(
        f,
        "settings-person-color",
        col()
            .gap(4.0)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .child(text("Your colour in shared spaces").font(k.ty.ui).color(k.c.text_base))
                    .child(el().grow(1.0))
                    .child(pal),
            )
            .child(line),
    )
}

pub fn pick_person_color(s: &mut State, c: String) {
    let c2 = c.clone();
    s.spawn(move |w| w.set_person_color(&c2), move |s, r| match r {
        Ok(()) => {
            s.settings.devices.person_color = Some(c.clone());
            s.settings.devices.person_err.clear();
            s.profile.color = c;
            s.profile.error.clear();
        }
        Err(e) => {
            s.settings.devices.person_err = e.clone();
            s.profile.error = e;
        }
    });
}

fn set_home(s: &mut State, work: impl FnOnce(&irori_window::Window) -> Result<HomeServerView, String> + Send + 'static) {
    s.settings.devices.home_err.clear();
    s.spawn(work, |s, r| match r {
        Ok(hs) => s.settings.devices.home = Some(hs),
        Err(e) => s.settings.devices.home_err = e,
    });
}

/// HomeServer (§7.1, §9): the switch on this device, its reach, and
/// "Meet through public relays" on every device; Advanced holds the relay.
fn home_server(s: &State, f: &Frame) -> El {
    let k = f.k;
    let m = &s.settings.devices;
    let Some(hs) = &m.home else {
        return if m.home_failed { el() } else { loading(f, "settings-home-loading", "Loading the home server") };
    };
    let mut sec = col().key("settings-home").w_full().gap(6.0);
    if hs.can && (hs.on || hs.suggest || !hs.elsewhere) {
        let reach = match hs.reach.as_str() {
            "forward" => "reachable at home; outside needs a port forward",
            "cgnat" => "reachable at home only",
            _ if hs.on && !hs.running => "starts once you have another device",
            _ => "",
        };
        let (outside, public) = (hs.outside, hs.public.clone());
        let on = hs.on;
        let p1 = public.clone();
        let mut label = col().grow(1.0).min_w(0.0).child(text("Home server: run it on this PC").font(k.ty.ui).color(k.c.text_base));
        if !reach.is_empty() {
            label = label.child(hint(f, reach));
        }
        sec = sec.child(
            row().w_full().gap(8.0).center_y().child(label).child(
                Switch::new(on)
                    .label("Home server: run it on this PC")
                    .hide_label()
                    .on_toggle(move |s: &mut State| {
                        let p = p1.clone();
                        set_home(s, move |w| w.set_home_server(!on, outside, &p))
                    })
                    .build(k),
            ),
        );
        if !hs.on {
            let mut t = String::new();
            if hs.suggest {
                t.push_str("This device is on most of the time, so it suits the server: your devices can reach each other through it from anywhere, even with public relays off. It's optional: any device that's on often keeps the others up to date. ");
            }
            t.push_str("Windows asks once to allow Irori on private networks; the server needs it.");
            sec = sec.child(hint(f, &t));
        } else {
            let p2 = public.clone();
            sec = sec.child(small_switch(f, "settings-home-outside", "Reachable from outside home", None, hs.outside, move |s| {
                let p = p2.clone();
                set_home(s, move |w| w.set_home_server(true, !outside, &p))
            }));
            let awake = hs.awake;
            let awake_line = (!hs.awake_line.is_empty()).then(|| {
                let ok = hs.awake && hs.awake_line == "Keeping this PC awake";
                (hs.awake_line.clone(), ok)
            });
            sec = sec.child(small_switch(
                f,
                "settings-home-awake",
                "Keep this PC awake while it's the server",
                awake_line,
                awake,
                move |s| set_home(s, move |w| w.set_home_server_awake(!awake)),
            ));
        }
        let line = home_line(hs);
        if !line.is_empty() {
            sec = sec.child(warn_line(f, &line));
        }
        let ask_public = hs.on && hs.outside && (hs.reach == "forward" || !hs.public.is_empty());
        if ask_public {
            let save = move |s: &mut State| {
                let addr = s.settings.devices.public_addr.text.trim().to_string();
                set_home(s, move |w| w.set_home_server(true, outside, &addr))
            };
            let save2 = save;
            sec = sec.child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        TextInput::new(&m.public_addr)
                            .label("Public address")
                            .placeholder(if hs.public.is_empty() {
                                "Public address (optional): IP or host name".to_string()
                            } else {
                                hs.public.clone()
                            })
                            .on_edit(|s: &mut State| &mut s.settings.devices.public_addr)
                            .on_submit(move |s: &mut State, _: &mut Tx| save(s))
                            .build(k)
                            .grow(1.0),
                    )
                    .child(Button::new("Save").variant(ButtonVariant::Outline).on_press(move |s: &mut State| save2(s)).build(k)),
            );
        }
    }
    let relays = hs.public_relays;
    sec = sec.child(small_switch(
        f,
        "settings-public-relays",
        "Meet through public relays",
        Some((
            if relays {
                "Reach your devices and invited people from any network, no port forwarding. iroh's free relays see which devices connect, never what they say.".into()
            } else {
                "Only your home server connects you from outside home.".into()
            },
            true,
        )),
        relays,
        move |s| set_home(s, move |w| w.set_public_relays(!relays)),
    ));
    if !hs.lan.is_empty() {
        sec = sec.child(warn_line(f, &super::plain_line(&hs.lan)));
    }
    if !hs.remote.is_empty() {
        sec = sec.child(warn_line(f, &super::plain_line(&hs.remote)));
    }
    if !m.home_err.is_empty() {
        sec = sec.child(error_line(f, &m.home_err));
    }
    sec = sec.child(
        Button::new(if m.advanced { "Hide advanced" } else { "Advanced" })
            .variant(ButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .expanded(m.advanced)
            .on_press(|s: &mut State| s.settings.devices.advanced = !s.settings.devices.advanced)
            .build(k)
            .self_start(),
    );
    if m.advanced {
        sec = sec.child(relay(s, f));
    }
    sec
}

/// A muted switch row with an optional line under (ok: muted, else amber).
fn small_switch(f: &Frame, key: &str, label: &str, under: Option<(String, bool)>, on: bool, toggle: impl Fn(&mut State) + 'static) -> El {
    let k = f.k;
    let mut words = col().grow(1.0).min_w(0.0).child(text(label.to_string()).font(k.ty.ui).color(k.c.text_muted).w_full().wrap());
    if let Some((t, ok)) = under {
        words = words.child(if ok { hint(f, &t) } else { warn_line(f, &t) });
    }
    row()
        .key(key.to_string())
        .w_full()
        .gap(8.0)
        .center_y()
        .child(words)
        .child(Switch::new(on).label(label.to_string()).hide_label().on_toggle(toggle).build(k))
}

/// Relay: the advanced TURN relay you run or rent elsewhere (§4.4, §9).
fn relay(s: &State, f: &Frame) -> El {
    let k = f.k;
    let m = &s.settings.devices;
    let direct = m.relay.as_ref().is_none_or(|r| r.state == "none");
    let mut out = col().key("settings-relay").w_full().gap(4.0);
    let save = |s: &mut State| {
        let url = s.settings.devices.relay_url.text.trim().to_string();
        s.settings.devices.relay_err.clear();
        s.spawn(move |w| w.set_relay(&url), |s, r| {
            let dm = &mut s.settings.devices;
            match r {
                Ok(v) => {
                    dm.relay = Some(v);
                    dm.relay_editing = false;
                    dm.relay_url = TextEdit::new("");
                }
                Err(e) => dm.relay_err = e,
            }
        });
    };
    if m.relay_editing {
        out = out.child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    TextInput::new(&m.relay_url)
                        .label("TURN credential URL")
                        .placeholder("https://… TURN credential URL (empty: none)")
                        .on_edit(|s: &mut State| &mut s.settings.devices.relay_url)
                        .on_submit(move |s: &mut State, _: &mut Tx| save(s))
                        .build(k)
                        .grow(1.0),
                )
                .child(Button::new("Save").variant(ButtonVariant::Contrast).on_press(move |s: &mut State| save(s)).build(k))
                .child(
                    Button::new("Back")
                        .variant(ButtonVariant::Ghost)
                        .on_press(|s: &mut State| s.settings.devices.relay_editing = false)
                        .build(k),
                ),
        );
    } else {
        let url = m.relay.as_ref().map(|r| super::plain_line(&r.credential_url)).unwrap_or_default();
        out = out.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text("Other relay:").font(k.ty.ui).color(k.c.text_muted))
                .child(text(if direct { "none".into() } else { url }).font(k.ty.ui).color(k.c.text_base).truncate().grow(1.0).min_w(0.0))
                .child(
                    Button::new("Change")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(|s: &mut State| s.settings.devices.relay_editing = true)
                        .build(k),
                ),
        );
    }
    if let Some(r) = m.relay.as_ref().filter(|r| r.state == "error") {
        out = out.child(warn_line(f, &format!("The relay is failing: {}", super::plain_line(&r.detail))));
    }
    if !m.relay_err.is_empty() {
        out = out.child(error_line(f, &m.relay_err));
    }
    out
}

// ---- pairing ------------------------------------------------------------------------------

/// Runs a pairing call that answers a status; the status shows.
fn pair_run(s: &mut State, work: impl FnOnce(&irori_window::Window) -> Result<PairStatus, String> + Send + 'static) {
    let dm = &mut s.settings.devices;
    dm.pair_busy = true;
    dm.pair_err.clear();
    s.spawn(work, |s, r| {
        let dm = &mut s.settings.devices;
        dm.pair_busy = false;
        match r {
            Ok(p) => {
                dm.copied = false;
                dm.words = TextEdit::new("");
                dm.key = TextEdit::new("");
                dm.words_err.clear();
                dm.key_err.clear();
                dm.pair = Some(p);
            }
            Err(e) => dm.pair_err = e,
        }
    });
}

fn join(s: &mut State) {
    let dm = &s.settings.devices;
    let code = dm.code.text.trim().to_string();
    if code.is_empty() {
        return;
    }
    let server = if dm.away { dm.server.text.trim().to_string() } else { String::new() };
    pair_run(s, move |w| w.seams.pairing.join(&code, &server));
}

/// Start: Pair a device, and Join with a code.
fn start(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let m = &s.settings.devices;
    let mut out = col().key("settings-pair-start").w_full().gap(8.0);
    if m.joining {
        let mut form = col().gap(8.0).child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    TextInput::new(&m.code)
                        .label("Pairing code")
                        .placeholder("XXXX-XXXX-XXXX-XXXX")
                        .text_style(k.ty.ui.mono())
                        .on_edit(|s: &mut State| &mut s.settings.devices.code)
                        .on_submit(|s: &mut State, _: &mut Tx| join(s))
                        .build(k)
                        .key("settings-pair-code")
                        .grow(1.0),
                )
                .child(
                    Button::new("Join")
                        .variant(ButtonVariant::Contrast)
                        .disabled(m.pair_busy || m.code.text.trim().is_empty())
                        .on_press(|s: &mut State| join(s))
                        .build(k),
                )
                .child(
                    Button::new("Back")
                        .variant(ButtonVariant::Ghost)
                        .on_press(|s: &mut State| s.settings.devices.joining = false)
                        .build(k),
                ),
        );
        form = form.child(if m.away {
            TextInput::new(&m.server)
                .label("Home server address")
                .placeholder("Home server address, like 203.0.113.7:24817")
                .text_style(k.ty.ui.mono())
                .on_edit(|s: &mut State| &mut s.settings.devices.server)
                .on_submit(|s: &mut State, _: &mut Tx| join(s))
                .build(k)
        } else {
            Button::new("Joining from outside your home?")
                .variant(ButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut State| s.settings.devices.away = true)
                .build(k)
                .self_start()
        });
        out = out.child(form);
    } else {
        let roles: Vec<(String, String)> = vec![("desktop".into(), "Desktop".into()), ("node".into(), "Often-on PC".into())];
        let role = if m.role.is_empty() { "desktop".to_string() } else { m.role.clone() };
        let r2 = role.clone();
        out = out.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .flex_wrap()
                .child(
                    Button::new("Pair a device")
                        .variant(ButtonVariant::Contrast)
                        .disabled(m.pair_busy)
                        .on_press(move |s: &mut State| {
                            let role = r2.clone();
                            pair_run(s, move |w| w.seams.pairing.add(&role))
                        })
                        .build(k),
                )
                .child(choice(s, f, "settings-pair-role", "The new device's role", &role, &roles, 150.0, false, |s, v| {
                    s.settings.devices.role = v.to_string()
                }))
                .child(el().grow(1.0))
                .child(
                    Button::new("Join with a code")
                        .variant(ButtonVariant::Outline)
                        .on_press(|s: &mut State| s.settings.devices.joining = true)
                        .build(k),
                ),
        );
    }
    if !m.pair_err.is_empty() {
        out = out.child(error_line(f, &m.pair_err));
    }
    out
}

fn cancel_button(f: &Frame) -> El {
    Button::new("Cancel")
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .on_press(|s: &mut State| s.spawn(|w| w.seams.pairing.cancel(), |_, _| {}))
        .build(f.k)
}

/// A code, words or key to read off (bg-muted, mono, selectable).
fn big(f: &Frame, key: &str, s: &str, size: f32) -> El {
    let k = f.k;
    row()
        .key(key.to_string())
        .w_full()
        .px(12.0)
        .py(8.0)
        .radius(8.0)
        .fill(k.c.layer_02)
        .center()
        .child(
            text(s.to_string())
                .font(rune::tok::TextStyle::new(size, 500.0).mono())
                .color(k.c.text_base)
                .selectable(true)
                .wrap(),
        )
}

fn ended(f: &Frame, line: &str, failed: bool) -> El {
    let k = f.k;
    row()
        .key("settings-pair-ended")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(super::plain_line(line))
                .font(k.ty.ui)
                .color(if failed { k.c.danger.fg } else { k.c.text_muted })
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
        .child(
            Button::new("OK")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut State| s.settings.devices.pair = None)
                .build(k),
        )
}

/// The open (or just ended) pairing, either side.
fn pairing(s: &State, f: &Frame, p: &PairStatus) -> El {
    use pair_state::*;
    let k = f.k;
    let m = &s.settings.devices;
    if p.state == DONE {
        return ended(f, "Paired. The devices are syncing now.", false);
    }
    if !is_open(p) {
        let line = if !p.error.is_empty() {
            p.error.clone()
        } else if p.state == EXPIRED {
            "Nobody used the code in time. Make a new one.".into()
        } else {
            "Pairing canceled.".into()
        };
        return ended(f, &line, p.state != CANCELED);
    }
    if p.mode == "add" && p.state == WAITING {
        // ShowCode: step 1 on the device already in your mesh.
        let left = p.expires.as_ref().filter(|e| !e.is_zero()).map(|e| {
            let ms = e.unix_milli_value() - s.settings.now_ms;
            (ms.max(0) + 59_999) / 60_000
        });
        let mut out = col()
            .key("settings-pair-code-shown")
            .w_full()
            .gap(8.0)
            .child(
                text("On the new device, open Settings → Devices → Join with a code, and type this code:")
                    .font(k.ty.ui)
                    .color(k.c.text_base)
                    .w_full()
                    .wrap(),
            )
            .child(big(f, "settings-pair-the-code", &p.code, 18.0))
            .child(hint(
                f,
                &format!(
                    "Treat it like a one-use password: whoever has it can reach this device first.{}",
                    left.map(|l| format!(" It works for {l} more min.")).unwrap_or_default()
                ),
            ));
        if !p.fingerprint.is_empty() {
            out = out.child(hint(f, &format!("The new device will ask for this device's key: {}", p.fingerprint)));
        }
        if !p.server_addr.is_empty() {
            out = out.child(hint(f, &format!("Joining from outside your home? Server address {}", p.server_addr)));
        }
        if m.copied {
            out = out.child(warn_line(f, "Copied. Any agent on this machine can read the clipboard."));
        }
        let code = p.code.clone();
        return out.child(
            row()
                .w_full()
                .center_y()
                .child(
                    Button::new("Copy")
                        .variant(ButtonVariant::Outline)
                        .control_size(ControlSize::Small)
                        .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                            tx.copy(code.clone());
                            s.settings.devices.copied = true;
                        })
                        .build(k),
                )
                .child(el().grow(1.0))
                .child(cancel_button(f)),
        );
    }
    if p.mode == "add" && p.state == CONFIRM {
        // TypeWords: who wants to join, and the words it shows. Pairing
        // goes on only when the typed words match.
        let j = p.joiner.clone().unwrap_or_default();
        let who = super::plain_line(&j.name);
        let mut line = format!("{} ", if who.is_empty() { "A device".to_string() } else { who });
        if !j.os.is_empty() {
            line.push_str(&format!("({}) ", super::plain_line(&j.os)));
        }
        line.push_str("wants to join.");
        if !j.fingerprint.is_empty() {
            line.push_str(&format!("  key {}", super::plain_line(&j.fingerprint)));
        }
        let submit = |s: &mut State| {
            let words = s.settings.devices.words.text.trim().to_string();
            if words.is_empty() {
                return;
            }
            s.settings.devices.pair_busy = true;
            s.settings.devices.words_err.clear();
            s.spawn(move |w| w.seams.pairing.confirm_words(&words), |s, r| {
                let dm = &mut s.settings.devices;
                dm.pair_busy = false;
                if let Err(e) = r {
                    dm.words_err = e;
                    dm.words = TextEdit::new("");
                }
                load_pair(s);
            });
        };
        let mut out = col()
            .key("settings-pair-words")
            .w_full()
            .gap(8.0)
            .child(text(line).font(k.ty.ui).color(k.c.text_base).w_full().wrap())
            .child(
                TextInput::new(&m.words)
                    .label("The words the new device shows")
                    .placeholder("Type the words it shows, then Enter")
                    .disabled(m.pair_busy)
                    .invalid(!m.words_err.is_empty())
                    .on_edit(|s: &mut State| &mut s.settings.devices.words)
                    .on_submit(move |s: &mut State, _: &mut Tx| submit(s))
                    .build(k)
                    .key("settings-pair-words-field"),
            );
        if !m.words_err.is_empty() {
            out = out.child(error_line(f, &m.words_err));
        }
        return out.child(row().w_full().child(el().grow(1.0)).child(cancel_button(f)));
    }
    if p.mode == "add" && p.state == GRANTED {
        let who = p.joiner.as_ref().map(|j| super::plain_line(&j.name)).filter(|n| !n.is_empty()).unwrap_or_else(|| "the new device".into());
        return col()
            .key("settings-pair-granted")
            .w_full()
            .gap(8.0)
            .child(text(format!("The words match. Now type this device's key on {who}:")).font(k.ty.ui).color(k.c.text_base).w_full().wrap())
            .child(big(f, "settings-pair-key", &p.fingerprint, 16.0))
            .child(row().w_full().center_y().child(hint(f, &format!("Waiting for {who}…")).grow(1.0)).child(cancel_button(f)));
    }
    if p.state == WORDS {
        // ReadWords: the joining device. The pairing is taken only from the
        // device whose connection proved the key typed here.
        let who = {
            let n = super::plain_line(&p.peer);
            if n.is_empty() { "the other device".to_string() } else { n }
        };
        let mut out = col()
            .key("settings-pair-read")
            .w_full()
            .gap(8.0)
            .child(text(format!("Type these words on {who}:")).font(k.ty.ui).color(k.c.text_base))
            .child(big(f, "settings-pair-our-words", &p.words.join(" "), 16.0));
        if p.key_ok {
            return out.child(row().w_full().center_y().child(hint(f, &format!("Waiting for {who}…")).grow(1.0)).child(cancel_button(f)));
        }
        let submit = |s: &mut State| {
            let key = s.settings.devices.key.text.trim().to_string();
            if key.is_empty() {
                return;
            }
            s.settings.devices.pair_busy = true;
            s.settings.devices.key_err.clear();
            s.spawn(move |w| w.seams.pairing.confirm_key(&key), |s, r| {
                let dm = &mut s.settings.devices;
                dm.pair_busy = false;
                if let Err(e) = r {
                    dm.key_err = e;
                    dm.key = TextEdit::new("");
                }
                load_pair(s);
            });
        };
        out = out.child(
            TextInput::new(&m.key)
                .label("The other device's key")
                .placeholder(format!("Then type the key {who} shows, then Enter"))
                .text_style(k.ty.ui.mono())
                .disabled(m.pair_busy)
                .invalid(!m.key_err.is_empty())
                .on_edit(|s: &mut State| &mut s.settings.devices.key)
                .on_submit(move |s: &mut State, _: &mut Tx| submit(s))
                .build(k)
                .key("settings-pair-key-field"),
        );
        if !m.key_err.is_empty() {
            out = out.child(error_line(f, &m.key_err));
        }
        return out.child(row().w_full().child(el().grow(1.0)).child(cancel_button(f)));
    }
    row()
        .key("settings-pair-looking")
        .w_full()
        .center_y()
        .child(muted_line(f, "Looking for the other device…").grow(1.0))
        .child(cancel_button(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CGNAT: &str = "Your internet provider shares your public address (CGNAT), so devices away from home can't reach this PC, and a port forward won't help.";
    const FORWARD: &str = "Away from home your devices can't reach this PC. Turn on UPnP on your router, or forward TCP and UDP port 25817 to 192.168.1.20 and enter your public address below.";

    fn home(reach: &str, line: &str, relays: bool) -> HomeServerView {
        HomeServerView {
            can: true,
            on: true,
            running: true,
            outside: true,
            reach: reach.into(),
            line: line.into(),
            public_relays: relays,
            ..Default::default()
        }
    }

    #[test]
    fn cgnat_says_public_relays_cover_it_when_on() {
        let l = home_line(&home("cgnat", CGNAT, true));
        assert!(l.contains("public relays connect your devices"));
        assert!(!l.contains("can't reach this PC"));
    }

    #[test]
    fn port_forward_keeps_the_router_steps_after_the_relays_line() {
        let l = home_line(&home("forward", FORWARD, true));
        assert!(l.contains("public relays connect your devices"));
        assert!(l.contains("For a direct connection: Turn on UPnP"));
        assert!(!l.contains("can't reach this PC"));
    }

    #[test]
    fn lines_unchanged_with_public_relays_off_and_other_lines_always() {
        assert_eq!(home_line(&home("cgnat", CGNAT, false)), CGNAT);
        let moved = "Port 25817 was taken, so the server moved to port 25818.";
        assert_eq!(home_line(&home("home", moved, true)), moved);
    }

    #[test]
    fn clean_sides_trims_drops_blanks_and_repeats_keeps_order() {
        assert_eq!(clean_sides(&["  Work ", "work", "", " ", "Home  lab", "Personal"]), vec!["Work", "Home lab", "Personal"]);
        let many: Vec<String> = (0..MAX_SIDES + 3).map(|i| format!("s{i}")).collect();
        assert_eq!(clean_sides(&many).len(), MAX_SIDES);
    }

    #[test]
    fn same_side_ignores_case_and_spacing() {
        assert!(same_side("Work", " work "));
        assert!(!same_side("Work", "Personal"));
    }

    #[test]
    fn sides_lines() {
        assert!(sides_line(&[]).starts_with("This device runs every side."));
        assert_eq!(
            sides_line(&["Work".into()]),
            "This device runs Work, and cards with no side. New cards typed here go on Work. Every device still shows every card."
        );
        assert!(sides_line(&["Personal".into(), "Garden".into(), "Work".into()]).contains("runs Personal, Garden and Work"));
    }

    #[test]
    fn device_names_are_one_line_and_capped() {
        assert_eq!(clean_device_name("  My\n  Mac  "), "My Mac");
        assert_eq!(clean_device_name(&"x".repeat(50)).chars().count(), MAX_DEVICE_NAME);
    }

    #[test]
    fn device_states() {
        let d = DeviceView { this: true, ..Default::default() };
        assert_eq!(state_line(&d, 0), "this device");
        let d = DeviceView { state: "offline".into(), ..Default::default() };
        assert_eq!(state_line(&d, 0), "offline · never seen");
        let d = DeviceView { state: "connected".into(), ..Default::default() };
        assert_eq!(state_line(&d, 0), "online");
    }

    #[test]
    fn open_pairings() {
        let p = |st: &str| PairStatus { state: st.into(), ..Default::default() };
        assert!(is_open(&p("words")));
        assert!(!is_open(&p("done")));
        assert!(!is_open(&p("code_used")));
    }
}
