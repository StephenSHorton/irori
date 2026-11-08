//! Settings' small panes (settings-dialog.tsx NotificationsPane and
//! AppPane, TranscribeSwitch, voice-note.tsx, maintenance-pane.tsx with
//! cards/clear-done.tsx) and lib/settings.ts.

use irori_window::settings::{EVENT_LAUNCH_AT_LOGIN, NotifyOptions};
use rune::{El, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Slider};

use crate::ui::{Event, Frame, State};

use super::{choice, error_line, heading, hint, labeled, loading, muted_line, ruled, toggle};

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
    /// The transcribe switch is read (into the call package's state).
    pub transcribe_read: bool,
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
    s.spawn(
        |w| w.get_notifications(),
        |s, n| {
            s.settings.general.notify = Some(n);
            s.settings.general.notify_err = None;
        },
    );
}

fn set_notify(s: &mut State, patch: impl FnOnce(&mut NotifyOptions)) {
    let Some(mut n) = s.settings.general.notify.clone() else {
        return;
    };
    patch(&mut n);
    // The control moves at once; the reply settles it.
    s.settings.general.notify = Some(n.clone());
    s.settings.general.tested = None;
    s.spawn(
        move |w| w.set_notifications(n),
        |s, r| match r {
            Ok(n) => {
                s.settings.general.notify = Some(n);
                s.settings.general.notify_err = None;
            }
            Err(e) => s.settings.general.notify_err = Some(e),
        },
    );
}

pub fn notifications(s: &State, f: &mut Frame) -> El {
    let m = &s.settings.general;
    let Some(n) = &m.notify else {
        return match &m.notify_err {
            Some(e) => muted_line(f, &format!("Could not load: {e}")),
            None => loading(
                f,
                "settings-notify-loading",
                "Loading notification settings",
            ),
        };
    };
    let off = !n.enabled;
    let k = f.k;
    let reduced = f.cx.is_reduced_motion();
    let volume = m.volume_drag.unwrap_or(n.volume as f32);
    let corners: Vec<(String, String)> = CORNERS
        .iter()
        .map(|(v, l)| (v.to_string(), l.to_string()))
        .collect();
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
        .child(toggle(
            f,
            "settings-notify-sound",
            "Sound",
            None,
            n.sound,
            off,
            |s, v| set_notify(s, |n| n.sound = v),
        ))
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
    let mut test = col()
        .key("settings-notify-test")
        .gap(4.0)
        .items_start()
        .child(
            Button::new("Send a test notification")
                .variant(ButtonVariant::Outline)
                .disabled(off)
                .on_press(|s: &mut State| {
                    s.spawn(
                        |w| w.test_notification(),
                        |s, r| {
                            s.settings.general.tested = Some(match r {
                                Ok(()) => "Sent. Look in the corner of your screen.".into(),
                                Err(e) => e,
                            })
                        },
                    )
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

/// The volume, 0 to 100% in fives (ui/slider): the value follows a drag
/// and is saved once it settles (a released drag, a key step).
fn slider(f: &Frame, v: f32, disabled: bool) -> El {
    Slider::new(v)
        .range(0.0, 1.0)
        .step(0.05)
        .width(160.0)
        .label("Volume")
        .disabled(disabled)
        .on_change(|s: &mut State, v, tx: &mut Tx| {
            let g = &mut s.settings.general;
            // A cancelled drag reports the saved value: nothing to hold.
            let saved = g.notify.as_ref().map(|n| n.volume as f32);
            g.volume_drag = (saved != Some(v)).then_some(v);
            tx.instant();
        })
        .on_commit(|s: &mut State, v, _| {
            s.settings.general.volume_drag = None;
            set_notify(s, move |n| n.volume = f64::from(v));
        })
        .build(f.k)
        .key("settings-volume")
}

// ---- App ----------------------------------------------------------------------

pub fn load_app(s: &mut State) {
    s.spawn(
        |w| w.get_launch_at_login(),
        |s, r| {
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
        },
    );
    // The voice note reads the call package's status, read afresh here.
    crate::call::refresh_voice(s);
    // TranscribeSwitch reads into the call package's own state, which
    // the huddle's notices read too.
    s.spawn(
        |w| w.call_transcribe(),
        |s, t| {
            s.call.view.transcribe_on = t;
            s.settings.general.transcribe_read = true;
        },
    );
}

const LAUNCH_HINT: &str = "Irori starts in the tray when you sign in. The tray offers it too.";
const TRANSCRIBE_LABEL: &str = "Transcribe what I say in calls";
const TRANSCRIBE_HINT: &str = "Huddles write down what each person says, on their own device. Turn this off and your voice isn't written down; you still hear and speak.";

/// The version as shown: "dev" as it is, else "v1.2.3".
pub fn version_label(v: &str) -> String {
    if v == "dev" || v == "sample" {
        v.to_string()
    } else {
        format!("v{}", v.trim_start_matches('v'))
    }
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
                s.spawn(
                    move |w| w.set_launch_at_login(v),
                    move |s, r| {
                        if let Err(e) = r {
                            s.settings.general.launch_err = Some(e);
                            s.spawn(
                                |w| w.get_launch_at_login(),
                                move |s, r| {
                                    s.settings.general.launch = Some(r.unwrap_or(!v));
                                },
                            );
                        } else {
                            s.settings.general.launch_err = None;
                        }
                    },
                );
            },
        )
    });
    if let Some(e) = &m.launch_err {
        out = out.child(error_line(f, e));
    }
    // VoiceNote: where the audio goes, and whether the model is here.
    if let Some(v) = crate::call::voice_note(s, f) {
        out = out.child(v);
    }
    // TranscribeSwitch (#656): on all the person's devices.
    let tr = if !m.transcribe_read {
        loading(
            f,
            "settings-transcribe-loading",
            "Loading huddle transcripts",
        )
    } else {
        toggle(
            f,
            "settings-transcribe",
            TRANSCRIBE_LABEL,
            Some(TRANSCRIBE_HINT),
            crate::call::transcribe_on(s),
            false,
            crate::call::set_transcribe,
        )
    };
    out = out.child(ruled(f, "settings-transcribe-box", tr));
    // Updates: UpdateStatusLine and Keep this device up to date.
    out = out.child(crate::status::update_section(s, f));
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

// ---- Maintenance --------------------------------------------------------------

/// Settings > Maintenance (maintenance-pane.tsx, card #138): the board's
/// own Clear done (every scope), which counts itself when it shows.
pub fn maintenance(s: &State, f: &mut Frame) -> El {
    col()
        .key("settings-maintenance")
        .w_full()
        .gap(8.0)
        .child(heading(f, "Clear done cards"))
        .child(muted_line(
            f,
            "Cleared cards leave the app's lists, here and in your spaces. They stay in Irori's log, so agents still find them with search and show. To clear or delete single cards, use Select on Closed.",
        ))
        .child(crate::board::clear::clear_done(s, f, "settings-clear", "", false))
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
}
