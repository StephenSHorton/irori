//! Settings (P8): the Rune port of the React app's components/settings/*
//! (on main, under frontend/src/): every setting in one dialog (card
//! #132), the panes down the left, the chosen one on the right. It opens
//! as `Dialog::Settings(pane)` ("" for the first); each pane reads its own
//! settings when it shows, and while the dialog is open the live parts
//! (devices, limits, running agents, the home server) are read again every
//! few seconds, since they change without an event.

pub mod agents;
pub mod appearance;
pub mod desks;
pub mod devices;
pub mod general;
pub mod number;
pub mod people;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rune::{Color, El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, IconButton, IconButtonVariant, Select, SelectAppearance, Switch,
};

use crate::ui::{Dialog, Event, Frame, State, icons};

/// One pane: its id (`Dialog::Settings(id)`), name, icon and what it holds.
pub struct Pane {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static rune::Icon,
    pub hint: &'static str,
}

pub static PANES: &[Pane] = &[
    Pane {
        id: "notifications",
        label: "Notifications",
        icon: &icons::BELL,
        hint: "What Irori tells you about, and how.",
    },
    Pane {
        id: "appearance",
        label: "Appearance",
        icon: &icons::PALETTE,
        hint: "The theme, your own colours, and light or dark.",
    },
    Pane {
        id: "devices",
        label: "Devices and connections",
        icon: &icons::LAPTOP,
        hint: "Your devices share one inbox and one Board.",
    },
    Pane {
        id: "people",
        label: "People and invites",
        icon: &icons::USERS,
        hint: "Who is in your spaces, and joining someone else's.",
    },
    Pane {
        id: "agents",
        label: "Agents",
        icon: &icons::BOT,
        hint: "Agents Irori starts for your cards, and what they start with.",
    },
    Pane {
        id: "desks",
        label: "Merge desks",
        icon: &icons::MERGE,
        hint: "One per repo: your open PRs from GitHub, for review. It never merges.",
    },
    Pane {
        id: "maintenance",
        label: "Maintenance",
        icon: &icons::TRASH,
        hint: "Clear out done cards.",
    },
    Pane {
        id: "app",
        label: "App",
        icon: &icons::INFO,
        hint: "Starting Irori, voice input, huddle transcripts, updates and the version.",
    },
];

/// The pane a `Dialog::Settings` names, the first for "" or one unknown.
pub fn pane(id: &str) -> &'static Pane {
    PANES.iter().find(|p| p.id == id).unwrap_or(&PANES[0])
}

/// What a confirm step does once confirmed (common/confirm-button.tsx).
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    EndAgent(String),
    TrustRepo(String),
    RemoveDevice(String),
    ResetDevice,
}

/// An open confirm: the alert dialog over Settings.
#[derive(Clone, Debug)]
pub struct Confirm {
    /// Whose button asked; its error shows under it.
    pub id: String,
    pub question: String,
    pub label: String,
    pub danger: bool,
    pub act: Act,
}

#[derive(Default)]
pub struct Model {
    /// The open select's key.
    pub open_select: Option<String>,
    /// The pane last shown: the gear and Cmd+, open Settings there again
    /// (settings-menu.tsx keeps its section while the window lives).
    pub last: String,
    pub confirm: Option<Confirm>,
    /// A confirm's failure, under its button, by its id.
    pub confirm_err: HashMap<String, String>,
    /// Bumped when the dialog opens or closes: the ticker of an older
    /// opening stops.
    pub tick_gen: Arc<AtomicU64>,
    /// The time the live lines are told against (ms), set on each tick.
    pub now_ms: i64,
    pub general: general::Model,
    pub appearance: appearance::Model,
    pub devices: devices::Model,
    pub people: people::Model,
    pub agents: agents::Model,
    pub desks: desks::Model,
}

/// The open pane's id, if the dialog is open.
pub fn open_pane(s: &State) -> Option<&'static str> {
    match &s.dialog {
        Some(Dialog::Settings(p)) => Some(pane(p).id),
        _ => None,
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Shows a pane and reads what it shows.
pub fn show(s: &mut State, id: &str) {
    s.settings.open_select = None;
    appearance::hidden(s);
    s.dialog = Some(Dialog::Settings(id.to_string()));
    s.settings.last = pane(id).id.to_string();
    load(s);
}

/// Reads the open pane's settings.
pub fn load(s: &mut State) {
    let Some(p) = open_pane(s) else { return };
    s.settings.now_ms = now_ms();
    match p {
        "notifications" => general::load_notify(s),
        "appearance" => appearance::load(s),
        "devices" => devices::load(s),
        "people" => people::load(s),
        "agents" => agents::load(s),
        "desks" => desks::load(s),
        "app" => general::load_app(s),
        _ => {}
    }
}

/// The live parts, read again every few seconds while the dialog is open.
fn tick(s: &mut State) {
    s.settings.now_ms = now_ms();
    match open_pane(s) {
        Some("devices") => devices::tick(s),
        Some("agents") => agents::tick(s),
        _ => {}
    }
}

/// Starts the dialog's ticker (not headless: a snapshot has one frame).
fn start_ticker(s: &mut State) {
    let generation = s.settings.tick_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let (Some(h), false) = (s.handle.clone(), s.headless) else {
        return;
    };
    let gen_now = s.settings.tick_gen.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(4));
            if gen_now.load(Ordering::SeqCst) != generation {
                return;
            }
            let g = gen_now.clone();
            h.update(move |s: &mut State, _| {
                if open_pane(s).is_none() {
                    g.fetch_add(1, Ordering::SeqCst);
                    return;
                }
                tick(s);
            });
        }
    });
}

/// Once a frame (ui::poll): the Agents pane's number boxes save when they
/// lose the focus. True when one did (its box may show the saved value
/// again).
pub fn poll(s: &mut State) -> bool {
    open_pane(s) == Some("agents") && agents::poll(s)
}

#[allow(dead_code)] // The package contract; Settings is a dialog.
pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(
        f,
        "Settings",
        "Settings is a dialog: the gear in the sidebar opens it.",
    )
}

pub fn on_event(s: &mut State, ev: &Event) {
    if ev.is("app:dialog-opened") {
        if let Some(Dialog::Settings(id)) = &mut s.dialog {
            if id.is_empty() {
                id.clone_from(&s.settings.last);
            }
            s.settings.last = pane(id).id.to_string();
        }
        if open_pane(s).is_some() {
            start_ticker(s);
            load(s);
        }
        return;
    }
    if ev.is("app:start") {
        s.settings.now_ms = now_ms();
        // The look the React window had, on a first start; then the
        // notification cards follow it (main.tsx, card #527).
        if !s.data_dir.as_os_str().is_empty() {
            let dir = s.data_dir.clone();
            appearance::carry_over(&mut s.prefs, &dir);
        }
        appearance::send_card_look(s);
        return;
    }
    // The profile page wears the same name and colour; it hears these too.
    general::on_event(s, ev);
    if open_pane(s).is_none() {
        return;
    }
    devices::on_event(s, ev);
    agents::on_event(s, ev);
    desks::on_event(s, ev);
    people::on_event(s, ev);
}

/// The headless scene (`irori --snap x.png --scene settings`): the pane
/// `IRORI_SCENE_PANE` names, else the first.
pub fn scene(s: &mut State) {
    s.close_card();
    let p = std::env::var("IRORI_SCENE_PANE").unwrap_or_default();
    // The Core's spaces open after the first frame; the People and
    // Agents panes list them.
    crate::spaces::wait_for_spaces(s);
    s.open_dialog(Dialog::Settings(pane(&p).id.to_string()));
}

/// The dialog: a large modal, the panes down the left and the chosen one
/// on the right, scrolling.
pub fn dialog(s: &State, f: &mut Frame) {
    let Some(Dialog::Settings(id)) = &s.dialog else {
        return;
    };
    let here = pane(id);
    let k = f.k;
    // The panes' menus (selects) go into the layer as they build; the
    // panel goes in before them, so they draw over it and find their
    // triggers to anchor to (built after them, they fell to the corner).
    let at = f.layer.len();
    let mut nav = col()
        .key("settings-nav")
        .w(216.0)
        .h_full()
        .padding(12.0)
        .gap(2.0)
        .border_sides([0.0, 1.0, 0.0, 0.0], k.c.border_base)
        .role("navigation")
        .label("Settings sections")
        .child(
            text("Settings")
                .font(rune::tok::TextStyle::new(16.0, 600.0))
                .color(k.c.text_base)
                .pl(8.0)
                .pt(4.0)
                .pb(12.0),
        );
    for p in PANES {
        let id = p.id;
        let current = p.id == here.id;
        nav = nav.child(
            Button::new(p.label)
                .variant(if current {
                    ButtonVariant::Neutral
                } else {
                    ButtonVariant::Ghost
                })
                .icon(p.icon)
                .on_press(move |s: &mut State| show(s, id))
                .build(k)
                .key(format!("settings-pane-{id}"))
                .w_full()
                .justify_start(),
        );
    }
    let body = match here.id {
        "notifications" => general::notifications(s, f),
        "appearance" => appearance::view(s, f),
        "devices" => devices::view(s, f),
        "people" => people::view(s, f),
        "agents" => agents::view(s, f),
        "desks" => desks::view(s, f),
        "maintenance" => general::maintenance(s, f),
        _ => general::app(s, f),
    };
    let k = f.k;
    let section = col()
        .key(format!("settings-section-{}", here.id))
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .child(
            col()
                .key("settings-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .key("settings-pane")
                        .w_full()
                        .padding(20.0)
                        .pr(48.0)
                        .gap(12.0)
                        .child(
                            col()
                                .gap(2.0)
                                .child(
                                    text(here.label)
                                        .font(rune::tok::TextStyle::new(16.0, 600.0))
                                        .color(k.c.text_base),
                                )
                                .child(
                                    text(here.hint)
                                        .font(k.ty.ui)
                                        .color(k.c.text_muted)
                                        .w_full()
                                        .wrap(),
                                ),
                        )
                        .child(body),
                ),
        );
    let close = IconButton::new(k.glyph.close)
        .variant(IconButtonVariant::Ghost)
        .label("Close")
        .on_press(|s: &mut State| close_dialog(s))
        .build(k)
        .key("settings-close")
        .absolute(0.0, 0.0)
        .pin(None, Some(12.0), Some(12.0), None);
    let panel = row()
        .key("settings-dialog")
        .role("dialog")
        .label("Settings")
        .w_full()
        .max_w(800.0)
        .h_full()
        .max_h(640.0)
        .radius(10.0)
        .fill(k.c.layer_01)
        .child(nav)
        .child(section)
        .child(close)
        .modal(true, |s: &mut State| close_dialog(s))
        .modal_click_outside(true);
    let panel = k.elevate(panel, &k.elevation.overlay);
    f.layer.insert(
        at,
        el().key("settings-scrim")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(k.c.scrim)
            .a11y_hidden(),
    );
    f.layer.insert(
        at + 1,
        el().key("settings-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .padding(32.0)
            .center()
            .child(panel),
    );
    confirm_dialog(s, f);
}

fn close_dialog(s: &mut State) {
    if s.settings.confirm.is_some() {
        s.settings.confirm = None;
        return;
    }
    if s.settings.open_select.take().is_some() {
        return;
    }
    s.settings.tick_gen.fetch_add(1, Ordering::SeqCst);
    s.close_dialog();
}

// ---- confirm ----------------------------------------------------------------

/// A button that asks first (common/confirm-button.tsx): the alert dialog
/// with the question, then the act; its error shows under it.
pub fn confirm_button(s: &State, f: &Frame, c: Confirm, button: &str, disabled: bool) -> El {
    let k = f.k;
    let id = c.id.clone();
    let err = s.settings.confirm_err.get(&id).cloned();
    let mut out = col()
        .key(format!("settings-confirm-{id}"))
        .items_end()
        .gap(4.0)
        .child(
            Button::new(button.to_string())
                .variant(if c.danger {
                    ButtonVariant::Danger
                } else {
                    ButtonVariant::Outline
                })
                .control_size(rune_kit::ControlSize::Small)
                .disabled(disabled)
                .on_press(move |s: &mut State| {
                    s.settings.confirm_err.remove(&c.id);
                    s.settings.confirm = Some(c.clone());
                })
                .build(k),
        );
    if let Some(e) = err {
        out = out.child(
            text(e)
                .font(k.ty.small)
                .color(k.c.danger.fg)
                .max_w(320.0)
                .wrap(),
        );
    }
    out
}

fn confirm_dialog(s: &State, f: &mut Frame) {
    let Some(c) = &s.settings.confirm else { return };
    let k = f.k;
    let label = c.label.clone();
    rune_kit::Dialog::new("settings-ask", c.label.clone())
        .open(true)
        .fit()
        .width(420.0)
        .description(c.question.clone())
        .child(
            row()
                .key("settings-ask-buttons")
                .w_full()
                .padding(16.0)
                .pt(0.0)
                .gap(8.0)
                .justify_end()
                .child(
                    Button::new("Cancel")
                        .variant(ButtonVariant::Ghost)
                        .on_press(|s: &mut State| s.settings.confirm = None)
                        .build(k),
                )
                .child(
                    Button::new(label)
                        .variant(if c.danger {
                            ButtonVariant::Danger
                        } else {
                            ButtonVariant::Contrast
                        })
                        .on_press(|s: &mut State| run_confirm(s))
                        .build(k)
                        .default_button(),
                ),
        )
        .on_close(|s: &mut State| s.settings.confirm = None)
        .build(k, f.layer);
}

fn run_confirm(s: &mut State) {
    let Some(c) = s.settings.confirm.take() else {
        return;
    };
    let id = c.id.clone();
    let fail = move |s: &mut State, e: String| {
        s.settings.confirm_err.insert(id, e);
    };
    match c.act {
        Act::EndAgent(key) => {
            let k2 = key.clone();
            s.spawn(
                move |w| w.agents_end(&key),
                move |s, r| match r {
                    Ok(()) => {
                        s.settings.agents.ending.insert(k2);
                        agents::load_running(s);
                    }
                    Err(e) => fail(s, e),
                },
            )
        }
        Act::TrustRepo(repo) => s.spawn(
            move |w| w.agents_trust_repo(&repo),
            move |s, r| match r {
                Ok(rows) => {
                    s.settings.agents.trusted = Some(rows);
                }
                Err(e) => fail(s, e),
            },
        ),
        Act::RemoveDevice(id) => s.spawn(
            move |w| w.remove_device(&id),
            move |s, r| match r {
                Ok(()) => devices::load_devices(s),
                Err(e) => fail(s, e),
            },
        ),
        Act::ResetDevice => s.spawn(
            |w| w.reset_device(),
            move |s, r| {
                if let Err(e) = r {
                    fail(s, e)
                }
            },
        ),
    }
}

// ---- shared bits --------------------------------------------------------------

/// Untrusted text made safe for one line (lib/irori.ts plainLine): control,
/// zero-width and bidi characters become spaces, runs of space collapse.
pub fn plain_line(s: &str) -> String {
    let hidden = |c: char| {
        let c = c as u32;
        c < 0x20
            || (0x7f..=0x9f).contains(&c)
            || (0x200b..=0x200f).contains(&c)
            || (0x202a..=0x202e).contains(&c)
            || (0x2066..=0x2069).contains(&c)
            || c == 0xfeff
    };
    let swapped: String = s.chars().map(|c| if hidden(c) { ' ' } else { c }).collect();
    swapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "just now", "42s ago", "5m ago", "3h ago", "2d ago" (lib/irori.ts).
pub fn relative_time(then_ms: i64, now_ms: i64) -> String {
    let s = ((now_ms - then_ms) as f64 / 1000.0).round() as i64;
    if s < 5 {
        return "just now".into();
    }
    if s < 60 {
        return format!("{s}s ago");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m ago");
    }
    let h = m / 60;
    if h < 24 {
        format!("{h}h ago")
    } else {
        format!("{}d ago", h / 24)
    }
}

pub fn hex(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::BLACK)
}

/// Tailwind's text-xs: 12 on a 16 line.
pub const XS: rune::tok::TextStyle = rune::tok::TextStyle::new(12.0, 400.0).lh(16.0);

/// A small muted line (text-xs text-muted-foreground).
pub fn hint(f: &Frame, s: &str) -> El {
    text(s.to_string())
        .font(XS)
        .color(f.k.c.text_muted)
        .w_full()
        .wrap()
}

/// A muted paragraph (text-[12.5px] text-muted-foreground).
pub fn muted_line(f: &Frame, s: &str) -> El {
    text(s.to_string())
        .font(rune::tok::TextStyle::new(12.5, 400.0))
        .color(f.k.c.text_muted)
        .w_full()
        .wrap()
}

pub fn error_line(f: &Frame, s: &str) -> El {
    text(s.to_string())
        .font(XS)
        .color(f.k.c.danger.fg)
        .w_full()
        .wrap()
}

pub fn warn_line(f: &Frame, s: &str) -> El {
    text(s.to_string())
        .font(XS)
        .color(f.k.c.warning.fg)
        .w_full()
        .wrap()
}

/// A section's heading (h3 text-sm font-medium).
pub fn heading(f: &Frame, s: &str) -> El {
    text(s.to_string())
        .font(f.k.ty.ui_medium)
        .color(f.k.c.text_base)
}

/// A rule above a group (border-t pt-3).
pub fn ruled(f: &Frame, key: &str, inner: El) -> El {
    col()
        .key(key.to_string())
        .w_full()
        .pt(12.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], f.k.c.border_base)
        .child(inner)
}

/// A switch with its label and hint (settings-dialog.tsx Toggle).
pub fn toggle(
    f: &Frame,
    key: &str,
    label: &str,
    hint_text: Option<&str>,
    checked: bool,
    disabled: bool,
    on: impl Fn(&mut State, bool) + 'static,
) -> El {
    let k = f.k;
    let mut words = col().grow(1.0).min_w(0.0).child(
        text(label.to_string())
            .font(k.ty.ui)
            .color(k.c.text_base)
            .w_full()
            .wrap(),
    );
    if let Some(h) = hint_text {
        words = words.child(hint(f, h));
    }
    let r = row()
        .key(key.to_string())
        .w_full()
        .gap(12.0)
        .items_start()
        .child(words)
        .child(
            Switch::new(checked)
                .label(label.to_string())
                .hide_label()
                .disabled(disabled)
                .on_toggle(move |s: &mut State| on(s, !checked))
                .build(k)
                .mt(2.0),
        );
    if disabled { r.opacity(0.5) } else { r }
}

/// A label on the left, a control on the right (settings-dialog.tsx Row).
pub fn labeled(f: &Frame, key: &str, label: &str, disabled: bool, control: El) -> El {
    let k = f.k;
    row()
        .key(key.to_string())
        .w_full()
        .gap(12.0)
        .center_y()
        .child(
            text(label.to_string())
                .font(k.ty.ui)
                .color(k.c.text_base)
                .opacity(if disabled { 0.5 } else { 1.0 }),
        )
        .child(el().grow(1.0))
        .child(control)
}

/// A select of (value, label) choices (common/choice-select.tsx), its
/// open state in `s.settings.open_select`.
#[allow(clippy::too_many_arguments)]
pub fn choice(
    s: &State,
    f: &mut Frame,
    key: &str,
    label: &str,
    value: &str,
    choices: &[(String, String)],
    width: f32,
    disabled: bool,
    on: impl Fn(&mut State, &str) + Clone + 'static,
) -> El {
    let labels: Vec<&str> = choices.iter().map(|(_, l)| l.as_str()).collect();
    let current = choices
        .iter()
        .find(|(v, _)| v == value)
        .map(|(_, l)| l.as_str());
    let pairs = choices.to_vec();
    let k2 = key.to_string();
    let k3 = key.to_string();
    Select::new(key.to_string(), &labels)
        .appearance(SelectAppearance::Field)
        .width(width)
        .label(label.to_string())
        .current(current)
        .placeholder(value.to_string())
        .disabled(disabled)
        .open(s.settings.open_select.as_deref() == Some(key))
        .on_toggle(move |s: &mut State| {
            let o = &mut s.settings.open_select;
            *o = if o.as_deref() == Some(k2.as_str()) {
                None
            } else {
                Some(k2.clone())
            };
        })
        .on_close(move |s: &mut State| {
            if s.settings.open_select.as_deref() == Some(k3.as_str()) {
                s.settings.open_select = None;
            }
        })
        .on_choose(move |s: &mut State, picked: &str, _: &mut Tx| {
            s.settings.open_select = None;
            if let Some((v, _)) = pairs.iter().find(|(_, l)| l == picked) {
                on(s, v);
            }
        })
        .build(f.k, f.layer)
}

/// "Could not load: …" or a placeholder line while a read is on its way.
pub fn loading(f: &Frame, key: &str, what: &str) -> El {
    row()
        .key(key.to_string())
        .gap(8.0)
        .center_y()
        .child(rune_kit::Spinner::new().build(f.k))
        .child(muted_line(f, what))
}

/// A bordered list with a rule between rows (divide-y rounded-md border).
pub fn boxed(f: &Frame, key: &str, rows: Vec<El>) -> El {
    let k = f.k;
    let n = rows.len();
    let mut out = col()
        .key(key.to_string())
        .w_full()
        .radius(6.0)
        .border(1.0, k.c.border_base);
    for (i, r) in rows.into_iter().enumerate() {
        let r = r.w_full().px(12.0).py(8.0);
        out = out.child(if i + 1 < n {
            r.border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_base)
        } else {
            r
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_opens_where_it_was_left() {
        let mut s = State::default();
        s.open_dialog(Dialog::Settings(String::new()));
        assert_eq!(
            open_pane(&s),
            Some("notifications"),
            "first time: the first pane"
        );
        show(&mut s, "desks");
        s.close_dialog();
        s.open_dialog(Dialog::Settings(String::new()));
        assert_eq!(open_pane(&s), Some("desks"));
        s.close_dialog();
        // A jump to a named pane goes there, and is remembered.
        s.open_dialog(Dialog::Settings("app".into()));
        s.close_dialog();
        s.open_dialog(Dialog::Settings(String::new()));
        assert_eq!(open_pane(&s), Some("app"));
    }

    #[test]
    fn plain_line_hides_controls_and_collapses_space() {
        assert_eq!(plain_line("  a\u{202e}b\n\tc  "), "a b c");
    }

    #[test]
    fn relative_times() {
        assert_eq!(relative_time(0, 2_000), "just now");
        assert_eq!(relative_time(0, 42_000), "42s ago");
        assert_eq!(relative_time(0, 5 * 60_000), "5m ago");
        assert_eq!(relative_time(0, 3 * 3_600_000), "3h ago");
        assert_eq!(relative_time(0, 50 * 3_600_000), "2d ago");
    }

    #[test]
    fn an_unknown_pane_shows_the_first() {
        assert_eq!(pane("").id, "notifications");
        assert_eq!(pane("nope").id, "notifications");
        assert_eq!(pane("devices").label, "Devices and connections");
    }
}
