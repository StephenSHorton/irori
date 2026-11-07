//! Settings' small panes (settings-dialog.tsx NotificationsPane and
//! AppPane, TranscribeSwitch, voice-note.tsx, maintenance-pane.tsx with
//! cards/clear-done.tsx) and lib/settings.ts.

use irori_window::settings::{EVENT_LAUNCH_AT_LOGIN, NotifyOptions};
use irori_window::voice::VoiceStatus;
use rune::{Drag, El, Phase, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use crate::ui::{Event, Frame, State};

use super::{Act, Confirm, choice, confirm_button, error_line, heading, hint, labeled, loading, muted_line, ruled, toggle};

#[derive(Default)]
pub struct Model {
    pub notify: Option<NotifyOptions>,
    pub notify_err: Option<String>,
    /// A note after the test button: sent, or why it couldn't be.
    pub tested: Option<String>,
    /// The volume while its slider is dragged (saved on release).
    pub volume_drag: Option<f32>,
    /// None until read; a failed read leaves it None and says why.
    pub launch: Option<bool>,
    pub launch_read: bool,
    pub launch_err: Option<String>,
    pub voice: Option<VoiceStatus>,
    pub transcribe: Option<bool>,
    pub transcribe_err: Option<String>,
    /// Clear done: the age picked, the count it takes, the last clear.
    /// None for the default, Older than a week.
    pub clear_age: Option<usize>,
    pub clear_count: Option<i64>,
    pub clear_err: Option<String>,
    pub cleared: Option<i64>,
}

impl Model {
    pub fn age(&self) -> usize {
        self.clear_age.unwrap_or(1).min(CLEAR_AGES.len() - 1)
    }
}

pub const CORNERS: &[(&str, &str)] = &[
    ("top-right", "Top right"),
    ("bottom-right", "Bottom right"),
    ("top-left", "Top left"),
    ("bottom-left", "Bottom left"),
];

pub fn on_event(s: &mut State, ev: &Event) {
    if ev.is(EVENT_LAUNCH_AT_LOGIN)
        && let Some(on) = ev.parse::<bool>()
    {
        s.settings.general.launch = Some(on);
        s.settings.general.launch_read = true;
    }
}

// ---- Notifications ------------------------------------------------------------

pub fn load_notify(s: &mut State) {
    s.spawn(|w| w.get_notifications(), |s, n| {
        s.settings.general.notify = Some(n);
        s.settings.general.notify_err = None;
    });
}

fn set_notify(s: &mut State, patch: impl FnOnce(&mut NotifyOptions)) {
    let Some(mut n) = s.settings.general.notify.clone() else { return };
    patch(&mut n);
    // The control moves at once; the reply settles it.
    s.settings.general.notify = Some(n.clone());
    s.settings.general.tested = None;
    s.spawn(move |w| w.set_notifications(n), |s, r| match r {
        Ok(n) => {
            s.settings.general.notify = Some(n);
            s.settings.general.notify_err = None;
        }
        Err(e) => s.settings.general.notify_err = Some(e),
    });
}

pub fn notifications(s: &State, f: &mut Frame) -> El {
    let m = &s.settings.general;
    let Some(n) = &m.notify else {
        return match &m.notify_err {
            Some(e) => muted_line(f, &format!("Could not load: {e}")),
            None => loading(f, "settings-notify-loading", "Loading notification settings"),
        };
    };
    let off = !n.enabled;
    let k = f.k;
    let reduced = f.cx.is_reduced_motion();
    let volume = m.volume_drag.unwrap_or(n.volume as f32);
    let corners: Vec<(String, String)> = CORNERS.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect();
    let mut out = col()
        .key("settings-notifications")
        .w_full()
        .gap(12.0)
        .child(toggle(
            f,
            "settings-notify-on",
            "Show notifications",
            Some("Held messages, cards blocked on you, and agents asking you to choose."),
            n.enabled,
            false,
            |s, v| set_notify(s, |n| n.enabled = v),
        ))
        .child(toggle(f, "settings-notify-sound", "Sound", None, n.sound, off, |s, v| {
            set_notify(s, |n| n.sound = v)
        }))
        .child(toggle(
            f,
            "settings-notify-confetti",
            "Confetti on good news",
            Some(if reduced {
                "Off while Reduce Motion is on."
            } else {
                "A burst across the screen when a card is done."
            }),
            n.confetti,
            off,
            |s, v| set_notify(s, |n| n.confetti = v),
        ))
        .child(labeled(
            f,
            "settings-notify-volume",
            "Volume",
            off || !n.sound,
            slider(f, volume, off || !n.sound),
        ));
    let corner = choice(
        s,
        f,
        "settings-corner",
        "Corner",
        &n.corner,
        &corners,
        140.0,
        off,
        |s, v| {
            let v = v.to_string();
            set_notify(s, move |n| n.corner = v)
        },
    );
    out = out.child(labeled(f, "settings-notify-corner", "Corner", off, corner));
    let mut test = col().key("settings-notify-test").gap(4.0).items_start().child(
        Button::new("Send a test notification")
            .variant(ButtonVariant::Outline)
            .disabled(off)
            .on_press(|s: &mut State| {
                s.spawn(|w| w.test_notification(), |s, r| {
                    s.settings.general.tested = Some(match r {
                        Ok(()) => "Sent. Look in the corner of your screen.".into(),
                        Err(e) => e,
                    })
                })
            })
            .build(k),
    );
    if let Some(t) = &m.tested {
        test = test.child(hint(f, t));
    }
    out = out.child(test);
    if let Some(e) = &m.notify_err {
        out = out.child(error_line(f, e));
    }
    out
}

/// The volume, 0 to 100 in fives (ui/slider has no kit widget: Rune's
/// examples/settings slider, by hand).
fn slider(f: &Frame, v: f32, disabled: bool) -> El {
    let k = f.k;
    let (w, h, knob, bar) = (160.0, 20.0, 16.0, 4.0);
    let travel = w - knob;
    let to_value = move |x: f32| (((x - knob / 2.0) / travel).clamp(0.0, 1.0) * 20.0).round() / 20.0;
    let mut e = el()
        .key("settings-volume")
        .role("slider")
        .label("Volume")
        .range_value(f64::from(v), 0.0, 1.0)
        .range_step(0.05)
        .a11y_value(format!("{}%", (v * 100.0).round()))
        .size(w, h)
        .child(
            el().size(w, bar)
                .absolute(0.0, (h - bar) / 2.0)
                .radius(bar / 2.0)
                .fill(k.c.layer_04)
                .a11y_hidden(),
        )
        .child(
            el().size(knob / 2.0 + v * travel, bar)
                .absolute(0.0, (h - bar) / 2.0)
                .radius(bar / 2.0)
                .fill(k.c.bg_accent)
                .a11y_hidden(),
        )
        .child(
            el().size(knob, knob)
                .absolute(v * travel, (h - knob) / 2.0)
                .radius(knob / 2.0)
                .fill(k.c.knob)
                .border(1.0, k.c.border_base)
                .a11y_hidden(),
        );
    if disabled {
        return e.opacity(0.5);
    }
    e = e
        .focusable(true)
        .on_step(|s: &mut State, steps, _| {
            let Some(n) = &s.settings.general.notify else { return };
            let v = (n.volume + f64::from(steps) * 0.05).clamp(0.0, 1.0);
            set_notify(s, move |n| n.volume = v);
        })
        .on_drag(move |s: &mut State, d: &mut Drag, tx| match d.phase {
            Phase::Down | Phase::Move => {
                s.settings.general.volume_drag = Some(to_value(d.x));
                tx.instant();
            }
            Phase::Up => {
                let v = s.settings.general.volume_drag.take();
                if let (Some(v), false) = (v, d.cancelled()) {
                    set_notify(s, move |n| n.volume = f64::from(v));
                }
            }
        });
    e
}

// ---- App ----------------------------------------------------------------------

pub fn load_app(s: &mut State) {
    s.spawn(|w| w.get_launch_at_login(), |s, r| {
        let g = &mut s.settings.general;
        g.launch_read = true;
        match r {
            Ok(on) => {
                g.launch = Some(on);
                g.launch_err = None;
            }
            Err(e) => {
                g.launch = None;
                g.launch_err = Some(e);
            }
        }
    });
    s.spawn(|w| (w.voice_status(), w.call_transcribe()), |s, (v, t)| {
        s.settings.general.voice = Some(v);
        s.settings.general.transcribe = Some(t);
    });
}

const LAUNCH_HINT: &str = "Irori starts in the tray when you sign in. The tray offers it too.";
const TRANSCRIBE_LABEL: &str = "Transcribe what I say in calls";
const TRANSCRIBE_HINT: &str = "Huddles write down what each person says, on their own device. Turn this off and your voice isn't written down; you still hear and speak.";

pub fn megabytes(n: u64) -> String {
    format!("{} MB", (n as f64 / 1_000_000.0).round())
}

/// The version as shown: "dev" as it is, else "v1.2.3".
pub fn version_label(v: &str) -> String {
    if v == "dev" || v == "sample" { v.to_string() } else { format!("v{}", v.trim_start_matches('v')) }
}

pub fn app(s: &State, f: &mut Frame) -> El {
    let m = &s.settings.general;
    let k = f.k;
    let mut out = col().key("settings-app").w_full().gap(12.0);
    // Until it's read, a spinner rather than a switch shown off; a failed
    // read leaves the switch off and disabled.
    out = out.child(if !m.launch_read {
        loading(f, "settings-launch-loading", "Loading Launch at login")
    } else {
        toggle(
            f,
            "settings-launch",
            "Launch at login",
            Some(LAUNCH_HINT),
            m.launch.unwrap_or(false),
            m.launch.is_none(),
            |s, v| {
                s.settings.general.launch = Some(v);
                s.spawn(move |w| w.set_launch_at_login(v), move |s, r| {
                    if let Err(e) = r {
                        s.settings.general.launch_err = Some(e);
                        s.spawn(|w| w.get_launch_at_login(), move |s, r| {
                            s.settings.general.launch = Some(r.unwrap_or(!v));
                        });
                    } else {
                        s.settings.general.launch_err = None;
                    }
                });
            },
        )
    });
    if let Some(e) = &m.launch_err {
        out = out.child(error_line(f, e));
    }
    // VoiceNote: where the audio goes, and whether the model is here.
    if let Some(v) = m.voice.as_ref().filter(|v| v.state != "unsupported") {
        let model = if v.state == "needs-model" {
            format!(
                "The speech model ({}) downloads the first time you press the mic.",
                megabytes(v.model_bytes)
            )
        } else {
            format!("The speech model ({}) is downloaded.", megabytes(v.model_bytes))
        };
        let mut voice = col()
            .gap(4.0)
            .child(heading(f, "Voice input"))
            .child(hint(
                f,
                &format!(
                    "The mic in the text boxes turns speech into text you can edit before sending. Speech is transcribed on this device with Whisper: no audio leaves it, and nothing is sent until you press Send. {model}"
                ),
            ));
        if v.state == "denied" {
            voice = voice.child(error_line(f, &v.reason));
        }
        out = out.child(ruled(f, "settings-voice", voice));
    }
    // TranscribeSwitch (#656): on all the person's devices.
    let mut tr = col().gap(4.0);
    tr = tr.child(match m.transcribe {
        None => loading(f, "settings-transcribe-loading", "Loading huddle transcripts"),
        Some(on) => toggle(f, "settings-transcribe", TRANSCRIBE_LABEL, Some(TRANSCRIBE_HINT), on, false, |s, v| {
            s.settings.general.transcribe = Some(v);
            s.settings.general.transcribe_err = None;
            s.spawn(move |w| w.call_transcribe_set(v).map(|_| w.call_transcribe()), |s, r| match r {
                Ok(on) => s.settings.general.transcribe = Some(on),
                Err(e) => {
                    s.settings.general.transcribe_err = Some(e);
                    s.spawn(|w| w.call_transcribe(), |s, on| s.settings.general.transcribe = Some(on));
                }
            });
        }),
    });
    if let Some(e) = &m.transcribe_err {
        tr = tr.child(error_line(f, e));
    }
    out = out.child(ruled(f, "settings-transcribe-box", tr));
    // Updates: the status line and Keep this device up to date are the
    // status package's (P11, updater/*).
    let updates = update_section(s, f);
    out = out.child(ruled(f, "settings-updates", col().gap(4.0).child(heading(f, "Updates")).child(updates)));
    let version = &s.data.version;
    if !version.is_empty() {
        out = out.child(ruled(
            f,
            "settings-version",
            row()
                .w_full()
                .center_y()
                .child(text("Version").font(k.ty.ui).color(k.c.text_base))
                .child(el().grow(1.0))
                .child(
                    text(version_label(version))
                        .font(k.ty.small.mono())
                        .color(k.c.text_muted)
                        .selectable(true),
                ),
        ));
    }
    out
}

/// The update section's slot. TODO(P11): `crate::status::update_section(s, f)`
/// once the status package exposes it (UpdateStatusLine and Keep this
/// device up to date).
fn update_section(s: &State, f: &mut Frame) -> El {
    let _ = s;
    hint(f, "Update status and Keep this device up to date come with the status package (P11).")
}

// ---- Maintenance --------------------------------------------------------------

const DAY: i64 = 24 * 60 * 60;

/// The ages a clear can take (card #138): (label, seconds, phrase).
pub const CLEAR_AGES: &[(&str, i64, &str)] = &[
    ("Older than a day", DAY, "closed more than a day ago"),
    ("Older than a week", 7 * DAY, "closed more than a week ago"),
    ("Older than a month", 30 * DAY, "closed more than a month ago"),
    ("All", 0, ""),
];

fn plural(n: i64) -> String {
    format!("{n} done {}", if n == 1 { "card" } else { "cards" })
}

/// What a clear by this age takes: "3 done cards closed more than a week
/// ago", "all 3 done cards".
pub fn clear_what(age: usize, count: i64) -> String {
    let (_, _, phrase) = CLEAR_AGES[age.min(CLEAR_AGES.len() - 1)];
    if phrase.is_empty() { format!("all {}", plural(count)) } else { format!("{} {phrase}", plural(count)) }
}

pub fn load_clear(s: &mut State) {
    let secs = CLEAR_AGES[s.settings.general.age()].1;
    s.spawn(move |w| w.clear_done_count(secs, ""), |s, n| {
        s.settings.general.clear_count = Some(n);
        s.settings.general.clear_err = None;
    });
}

pub fn maintenance(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let m = &s.settings.general;
    let mut ages = row().key("settings-clear-ages").gap(4.0).flex_wrap().role("group").label("Clear done cards by age");
    for (i, (label, _, _)) in CLEAR_AGES.iter().enumerate() {
        ages = ages.child(
            Button::new(*label)
                .variant(if m.age() == i { ButtonVariant::Neutral } else { ButtonVariant::Ghost })
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut State| {
                    s.settings.general.clear_age = Some(i);
                    s.settings.general.cleared = None;
                    s.settings.general.clear_count = None;
                    load_clear(s);
                })
                .build(k)
                .key(format!("settings-clear-age-{i}")),
        );
    }
    let count = m.clear_count;
    let what = clear_what(m.age(), count.unwrap_or(0));
    let line = match count {
        None => "Counting…".to_string(),
        Some(0) => "Nothing to clear.".to_string(),
        Some(_) => format!("Clears {what}."),
    };
    let n = count.unwrap_or(0);
    let secs = CLEAR_AGES[m.age()].1;
    let mut act = row()
        .key("settings-clear-act")
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(text(line).font(k.ty.ui).color(k.c.text_muted))
        .child(confirm_button(
            s,
            f,
            Confirm {
                id: "clear-done".into(),
                question: format!(
                    "Clear {what}? They leave the app's lists, in your spaces too. Agents still find them with search and show. Nothing is deleted."
                ),
                label: if n > 0 { format!("Clear {n}") } else { "Clear".into() },
                danger: false,
                act: Act::ClearDone(secs),
            },
            &if n > 0 { format!("Clear {n}…") } else { "Clear".into() },
            n == 0,
        ));
    if let Some(c) = m.cleared {
        act = act.child(hint(f, &format!("Cleared {}.", plural(c))).w(200.0));
    }
    let mut sec = col()
        .key("settings-maintenance")
        .w_full()
        .gap(8.0)
        .child(heading(f, "Clear done cards"))
        .child(muted_line(
            f,
            "Cleared cards leave the app's lists, here and in your spaces. They stay in Irori's log, so agents still find them with search and show. To clear or delete single cards, use Select on Closed.",
        ))
        .child(ages)
        .child(act);
    if let Some(e) = &m.clear_err {
        sec = sec.child(error_line(f, e));
    }
    sec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_read_as_shipped() {
        assert_eq!(version_label("dev"), "dev");
        assert_eq!(version_label("0.12.3"), "v0.12.3");
        assert_eq!(version_label("v0.12.3"), "v0.12.3");
    }

    #[test]
    fn a_clear_says_what_it_takes() {
        assert_eq!(clear_what(1, 3), "3 done cards closed more than a week ago");
        assert_eq!(clear_what(0, 1), "1 done card closed more than a day ago");
        assert_eq!(clear_what(3, 2), "all 2 done cards");
    }

    #[test]
    fn model_sizes_in_megabytes() {
        assert_eq!(megabytes(147_951_465), "148 MB");
    }
}
