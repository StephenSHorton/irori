//! Voice input on the text boxes (common/mic-button.tsx, lib/voice.ts,
//! lib/mic-shortcut.ts, settings/voice-note.tsx): tap to talk (tap again
//! to stop), or hold and let go. The words appear in the box live at the
//! caret, a phrase at a time; the person edits and sends as usual. Typing,
//! or Send clearing the box, stops listening. Speech is transcribed on this
//! device (Whisper); the first press offers the one-time model download.
//! Cmd+D (Ctrl+D off a Mac) toggles it too: [`on_key`].

use std::time::{Duration, Instant};

use rune::{Drag, El, KeyEvent, Phase, TextEdit, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Placement, Popover, Progress};
use serde_json::Value;

use super::dictation::{self, Anchor, MicSeen, Session};
use super::icons as ic;
use crate::ui::{Frame, State};

/// Where a box's words go: a getter for its `TextEdit`.
pub type Target = fn(&mut State) -> &mut TextEdit;

/// A press held this long is push-to-talk: letting go stops.
const HOLD: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase2 {
    Starting,
    Listening,
    Stopping,
}

/// The one dictation running, in one box.
pub struct Dictation {
    pub key: String,
    target: Target,
    pub phase: Phase2,
    session: Option<Session>,
    anchor: Anchor,
    /// Where the words start in the box, and how long they are now.
    start: usize,
    len: usize,
    /// The box's text as dictation last set it; anything else means the
    /// person typed or Send cleared it.
    composed: String,
    pressed_at: Option<Instant>,
}

/// A mic's popover: the download offer, or why it stopped.
#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    Download,
    Error { code: String, text: String },
}

/// A mic drawn this frame, for the shortcut.
#[derive(Clone)]
pub struct Drawn {
    pub key: String,
    pub target: Target,
    pub usable: bool,
}

/// Reads whether to show the mic (again after a download).
pub(super) fn load_status(s: &mut State, force: bool) {
    if s.call.voice.is_some() && !force {
        return;
    }
    s.spawn(|w| w.voice_status(), |s, st| s.call.voice = Some(st));
}

fn in_call(s: &State) -> bool {
    s.call.view.mine.is_some()
}

/// One voice:event: the box acts on its own session only.
pub(super) fn on_voice(s: &mut State, p: &Value) {
    let str_of = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let session = p.get("session").and_then(Value::as_u64).unwrap_or(0) as u32;
    let (kind, words, code) = (str_of("kind"), str_of("text"), str_of("code"));
    let Some(d) = s.call.dict.as_mut() else {
        return;
    };
    let Some(sess) = d.session.as_mut() else {
        return;
    };
    if sess.id != session {
        return;
    }
    sess.step(session, &kind, &words, &code);
    let spoken = sess.spoken();
    let error = sess.error.clone();
    let ended = sess.ended;
    let target = d.target;
    let (start, len, composed) = (d.start, d.len, d.composed.clone());
    // The person typed (or Send cleared the box): stop listening.
    let typed = target(s).text != composed;
    if typed {
        if !ended {
            stop(s);
        }
    } else if matches!(kind.as_str(), "partial" | "final" | "ended") {
        let anchor = s
            .call
            .dict
            .as_ref()
            .map(|d| d.anchor.clone())
            .unwrap_or_default();
        let (full, caret) = dictation::compose(&anchor, &spoken);
        // The box is before + mid + after: replace only our part.
        let mid_end = full.len() - anchor.after.len();
        let mid = full[start.min(mid_end)..mid_end].to_string();
        let edit = target(s);
        edit.set_selection(start, start + len);
        edit.insert(&mid);
        edit.set_caret(caret);
        let now = edit.text.clone();
        if let Some(d) = s.call.dict.as_mut() {
            d.len = mid.len();
            d.composed = now;
        }
    }
    if let Some((code, text)) = error {
        let key = s
            .call
            .dict
            .as_ref()
            .map(|d| d.key.clone())
            .unwrap_or_default();
        s.call.mic_note = Some((key, Note::Error { code, text }));
    }
    if ended {
        s.call.dict = None;
    }
}

/// Starts dictation into `key`'s box.
fn begin(s: &mut State, key: &str, target: Target) {
    let Some(st) = s.call.voice.clone() else {
        load_status(s, false);
        return;
    };
    match st.state.as_str() {
        "unsupported" => return,
        "needs-model" => {
            s.call.mic_note = Some((key.to_string(), Note::Download));
            return;
        }
        _ => {}
    }
    if in_call(s) {
        return;
    }
    let edit = target(s);
    let v = edit.text.clone();
    // A box without focus (the shortcut's main composer) takes the words at
    // its end, so Enter sends them; a focused one at its caret.
    let (a, b) = if edit.focused {
        edit.range()
    } else {
        (v.len(), v.len())
    };
    let anchor = dictation::anchor_at(&v, a, b);
    let start = anchor.before.len();
    s.call.mic_note = None;
    s.call.dict = Some(Dictation {
        key: key.to_string(),
        target,
        phase: Phase2::Starting,
        session: None,
        anchor,
        start,
        len: b.saturating_sub(a),
        composed: v,
        pressed_at: None,
    });
    let key = key.to_string();
    let Some(arc) = s.w.clone() else { return };
    s.spawn(
        move |_| arc.voice_start(),
        move |s, r| match r {
            Ok(id) => {
                let mine = s
                    .call
                    .dict
                    .as_mut()
                    .filter(|d| d.key == key && d.session.is_none());
                match mine {
                    Some(d) => {
                        d.session = Some(Session::new(id));
                        d.phase = Phase2::Listening;
                        // A huddle began while it was starting: it has the mic.
                        if s.call.view.mine.is_some() {
                            stop(s);
                        }
                    }
                    None => s.spawn(move |w| w.voice_stop(id), |_, _| {}),
                }
            }
            Err(e) => {
                s.call.dict = None;
                let code = dictation::code_of(&e).to_string();
                s.call.mic_note = Some((key, Note::Error { code, text: e }));
            }
        },
    );
}

/// Stops the dictation running, if any; its `ended` finishes it (or 4 s
/// on, if that never comes).
pub(super) fn stop(s: &mut State) {
    let Some(d) = s.call.dict.as_mut() else {
        return;
    };
    let Some(id) = d.session.as_ref().map(|x| x.id) else {
        // Still starting: the start's answer stops it.
        s.call.dict = None;
        return;
    };
    d.phase = Phase2::Stopping;
    s.spawn(move |w| w.voice_stop(id), |_, _| {});
    if let Some(h) = s.handle.clone()
        && !s.headless
    {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(4));
            h.update(move |s: &mut State, _| {
                if s.call
                    .dict
                    .as_ref()
                    .and_then(|d| d.session.as_ref())
                    .is_some_and(|x| x.id == id)
                {
                    s.call.dict = None;
                }
            });
        });
    }
}

fn download(s: &mut State, key: String, target: Target) {
    if s.call.download.is_some() {
        return;
    }
    let total = s.call.voice.as_ref().map_or(0, |v| v.model_bytes);
    s.call.download = Some((0, total));
    s.call.download_error.clear();
    let Some(arc) = s.w.clone() else { return };
    s.spawn(
        move |_| tokio_block(async move { arc.voice_download_model().await }),
        move |s, r| {
            s.call.download = None;
            match r {
                Ok(()) => {
                    s.call.mic_note = None;
                    s.spawn(
                        |w| w.voice_status(),
                        move |s, st| {
                            s.call.voice = Some(st);
                            begin(s, &key, target);
                        },
                    );
                }
                Err(e) => s.call.download_error = e,
            }
        },
    );
}

/// Runs a future to its end on a small runtime of its own (the download is
/// async in the Window; this is already off the UI thread).
fn tokio_block<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime for the model download")
        .block_on(f)
}

/// The mic for a composer: `key` names it (prefixed by its package, e.g.
/// "talk-composer"), `target` gets its box. Nothing where this device has
/// no speech engine. Never disabled by its box: see [`mic_button_in`].
pub fn mic_button(s: &State, f: &mut Frame, key: &str, target: Target) -> El {
    mic_button_in(s, f, key, target, false)
}

/// [`mic_button`] for a box that may be off: `disabled` (the box is
/// disabled or read-only) greys the mic and keeps the shortcut off it, as
/// React's `disabled` prop did.
pub fn mic_button_in(s: &State, f: &mut Frame, key: &str, target: Target, disabled: bool) -> El {
    let off = disabled;
    let k = f.k;
    let Some(st) = s.call.voice.as_ref().filter(|v| v.state != "unsupported") else {
        return el().key(format!("call-mic-{key}")).w(0.0);
    };
    let d = s.call.dict.as_ref().filter(|d| d.key == key);
    let phase = d.map(|d| d.phase);
    let live = phase == Some(Phase2::Listening);
    let busy = matches!(phase, Some(Phase2::Starting | Phase2::Stopping));
    let dl = s.call.download;
    let call = in_call(s);
    let denied = st.state == "denied" && !live;
    let disabled = off || (call && !live) || busy || dl.is_some();
    // The shortcut's view of it (the latest frame's mics).
    s.call.mics.borrow_mut().0.push(Drawn {
        key: key.to_string(),
        target,
        usable: !off && !call && dl.is_none() && phase.is_none(),
    });
    let label = if live {
        "Stop voice input".to_string()
    } else if call {
        "Voice input is off during a huddle".to_string()
    } else if st.state == "needs-model" {
        "Voice input (downloads the speech model first)".to_string()
    } else {
        "Voice input: tap to talk, or hold and let go".to_string()
    };
    let tip = if denied {
        format!("{label}. {}", st.reason)
    } else {
        label.clone()
    };
    let tip = format!(
        "{tip}  {}",
        if cfg!(target_os = "macos") {
            "⌘D"
        } else {
            "Ctrl+D"
        }
    );
    let (glyph, ink, fill) = if busy || dl.is_some() {
        (&ic::LOADER, k.c.icon_muted, None)
    } else if denied {
        (&ic::MIC_OFF, k.c.text_muted, None)
    } else if live {
        (&ic::MIC, k.c.danger.icon, Some(k.c.danger.icon.alpha(0.15)))
    } else {
        (&ic::MIC, k.c.icon_muted, None)
    };
    let key_s = key.to_string();
    let mut b = el()
        .key(format!("call-mic-{key}"))
        .role("button")
        .label(label)
        .help(tip)
        .checked(live)
        .size(28.0, 28.0)
        .radius(6.0)
        .center()
        .child(icon(glyph, 16.0).color(ink))
        .opacity(if disabled { 0.5 } else { 1.0 });
    if let Some(c) = fill {
        b = b.fill(c);
    }
    if !disabled {
        b = b
            .hover(k.c.hover)
            .on_drag(
                move |s: &mut State, g: &mut Drag, _tx: &mut Tx| match g.phase {
                    Phase::Down => {
                        let mine = s.call.dict.as_ref().is_some_and(|d| d.key == key_s);
                        if s.call.dict.is_none() {
                            begin(s, &key_s, target);
                            if let Some(d) = s.call.dict.as_mut() {
                                d.pressed_at = Some(Instant::now());
                            }
                        } else if mine
                            && s.call
                                .dict
                                .as_ref()
                                .is_some_and(|d| d.phase == Phase2::Listening)
                        {
                            stop(s);
                        }
                    }
                    Phase::Up => {
                        let held = s
                            .call
                            .dict
                            .as_mut()
                            .filter(|d| d.key == key_s)
                            .and_then(|d| d.pressed_at.take())
                            .is_some_and(|at| at.elapsed() >= HOLD);
                        if held {
                            stop(s);
                        }
                    }
                    Phase::Move => {}
                },
            );
    }
    let note = s
        .call
        .mic_note
        .as_ref()
        .filter(|(k2, _)| k2 == key)
        .map(|(_, n)| n.clone());
    let open =
        note.is_some() || (dl.is_some() && s.call.dict.is_none() && s.call.mic_note.is_none());
    let mut p = Popover::new(format!("call-mic-note-{key}"))
        .trigger(b)
        .open(open && (note.is_some() || dl.is_some()))
        .placement(Placement::TopEnd)
        .on_close(|s: &mut State| {
            if s.call.download.is_none() {
                s.call.mic_note = None;
            }
        });
    if open {
        p = p.child(note_body(s, k, key, target, note, st.model_bytes));
    }
    p.build(k, f.layer)
}

fn note_body(s: &State, k: &Kit, key: &str, target: Target, note: Option<Note>, bytes: u64) -> El {
    // The popover is at most 320 wide, its 12 inset inside that.
    let c = col().key("call-mic-note").w(296.0).gap(8.0);
    if let Some((done, total)) = s.call.download {
        let pct = if total > 0 {
            (done as f32 / total as f32).min(1.0)
        } else {
            0.0
        };
        return c
            .role("status")
            .child(
                text(format!(
                    "Downloading the speech model… {} of {}",
                    dictation::megabytes(done),
                    dictation::megabytes(total)
                ))
                .font(k.ty.small)
                .color(k.c.text_base)
                .w_full()
                .wrap(),
            )
            .child(Progress::new(Some(pct)).build(k));
    }
    match note {
        Some(Note::Download) => {
            let key2 = key.to_string();
            let mut c = c.child(
                text(format!(
                    "Voice input turns speech into text on this device; no audio leaves it. It needs the speech model, a one-time {} download.",
                    dictation::megabytes(bytes)
                ))
                .font(k.ty.small)
                .color(k.c.text_base)
                .w_full()
                .wrap(),
            );
            if !s.call.download_error.is_empty() {
                c = c.child(
                    text(s.call.download_error.clone())
                        .role("alert")
                        .font(k.ty.small)
                        .color(k.c.danger.icon)
                        .w_full()
                        .wrap(),
                );
            }
            c.child(
                row()
                    .key("foot")
                    .w_full()
                    .gap(8.0)
                    .justify_end()
                    .child(
                        Button::new("Not now")
                            .variant(ButtonVariant::Ghost)
                            .control_size(ControlSize::Small)
                            .on_press(|s: &mut State| s.call.mic_note = None)
                            .build(k),
                    )
                    .child(
                        Button::new(format!("Download {}", dictation::megabytes(bytes)))
                            .variant(ButtonVariant::Submit)
                            .control_size(ControlSize::Small)
                            .on_press(move |s: &mut State| download(s, key2.clone(), target))
                            .build(k),
                    ),
            )
        }
        Some(Note::Error { code, text: t }) => {
            let mut foot = row().key("foot").w_full().gap(8.0).justify_end();
            if dictation::settings_fix(&code) {
                let code2 = code.clone();
                foot = foot.child(
                    Button::new("Open Settings")
                        .variant(ButtonVariant::Outline)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut State| {
                            let c = code2.clone();
                            s.spawn(move |w| w.voice_open_settings(&c), |_, _| {});
                        })
                        .build(k),
                );
            }
            foot = foot.child(
                Button::new("OK")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut State| s.call.mic_note = None)
                    .build(k),
            );
            c.child(
                text(dictation::error_line(&code, &t))
                    .role("alert")
                    .font(k.ty.small)
                    .color(k.c.danger.icon)
                    .w_full()
                    .wrap(),
            )
            .child(foot)
        }
        None => c,
    }
}

/// The dictation shortcut (card #634): Cmd+D on a Mac, Ctrl+D elsewhere,
/// toggles the one mic that answers (the one listening; else the focused
/// box's; else the newest drawn). Call it from the window's key handler;
/// true when it took the key.
pub fn on_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    let mac = cfg!(target_os = "macos");
    let m = ev.mods;
    let d_key = matches!(&ev.key, rune::Key::Command(c) if c.eq_ignore_ascii_case(&'d'));
    let on_key = d_key
        && dictation::dictation_key(
            "d",
            m.logo || (mac && m.cmd && !m.ctrl),
            m.ctrl,
            m.shift,
            m.alt,
            mac,
        );
    if !on_key {
        return false;
    }
    let drawn = s.call.mics.borrow().1.clone();
    let live_key = s.call.dict.as_ref().map(|d| d.key.clone());
    let seen: Vec<MicSeen> = drawn
        .iter()
        .map(|d| MicSeen {
            key: d.key.clone(),
            live: live_key.as_deref() == Some(d.key.as_str()),
            focused: (d.target)(s).focused,
            usable: d.usable,
        })
        .collect();
    let Some(pick) = dictation::pick_mic(&seen).cloned() else {
        return false;
    };
    let target = drawn.iter().find(|d| d.key == pick.key).map(|d| d.target);
    match (&s.call.dict, target) {
        (Some(d), _) if d.key == pick.key => {
            if d.phase == Phase2::Listening {
                stop(s);
            }
        }
        (None, Some(t)) => begin(s, &pick.key, t),
        _ => {}
    }
    true
}

/// VoiceNote: Settings' word on voice input: where the audio goes and
/// whether the speech model is downloaded. Nothing where there's no
/// engine.
pub fn voice_note(s: &State, f: &mut Frame) -> Option<El> {
    let k = f.k;
    let st = s.call.voice.as_ref().filter(|v| v.state != "unsupported")?;
    let mb = dictation::megabytes(st.model_bytes);
    let model = if st.state == "needs-model" {
        format!("The speech model ({mb}) downloads the first time you press the mic.")
    } else {
        format!("The speech model ({mb}) is downloaded.")
    };
    let mut c = col()
        .key("call-voice-note")
        .w_full()
        .gap(4.0)
        .pt(12.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_muted)
        .child(text("Voice input").font(k.ty.ui_medium).color(k.c.text_base))
        .child(
            text(format!(
                "The mic in the text boxes turns speech into text you can edit before sending. Speech is transcribed on this device with Whisper: no audio leaves it, and nothing is sent until you press Send. {model}"
            ))
            .font(k.ty.text_xsmall)
            .color(k.c.text_muted)
            .w_full()
            .wrap(),
        );
    if st.state == "denied" {
        c = c.child(
            text(st.reason.clone())
                .font(k.ty.text_xsmall)
                .color(k.c.danger.icon)
                .w_full()
                .wrap(),
        );
    }
    Some(c)
}

/// Settings opened: read the voice status afresh.
pub fn refresh_voice(s: &mut State) {
    load_status(s, true);
}
