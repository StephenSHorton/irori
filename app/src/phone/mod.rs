//! The Android prototype's phone pieces (card #680): Join with a code, as
//! main's Tauri phone app has it in Settings → Devices, shown first while
//! this device has no other device; and the phone's frame for the window
//! (no sidebar, room for the status bar, the comment box near the top).
//!
//! The flow is main's: the code (and, from away, the home server's
//! host:port) from the computer's Add device; this phone then shows words
//! to type on the computer, and takes the key the computer shows beside
//! its code. The Pairing seam blocks, so every call goes through
//! [`State::act`], off the UI thread.

use rune::{El, TextEdit, Tx, col, el, row, text, text_input};
use rune_kit::{Button, ButtonVariant};

use irori_window::events::EVENT_DEVICES_PAIRING;

use crate::ui::{Event, Frame, State};

/// Room for Android's status bar: Rune doesn't feed the safe area on
/// Android yet, so the phone's frame pads it by hand.
pub const STATUS_BAR: f32 = 32.0;

#[derive(Default)]
pub struct Model {
    pub code: TextEdit,
    pub server: TextEdit,
    pub key: TextEdit,
    /// The joining side's state: "" (not started), words, done, failed...
    pub state: String,
    pub words: Vec<String>,
    /// The other device's name, once both proved the code.
    pub peer: String,
    pub error: String,
}

/// True while this phone has no other device: it shows Join with a code.
pub fn needs_pairing(s: &State) -> bool {
    s.phone
        && s.phone_pair.state != "done"
        && !s.data.devices.iter().any(|d| !d.this && !d.revoked)
}

pub fn on_event(s: &mut State, ev: &Event) {
    if !ev.is(EVENT_DEVICES_PAIRING) {
        return;
    }
    let p = &ev.payload;
    let get = |k: &str| p.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    if get("mode") != "join" {
        return;
    }
    s.phone_pair.state = get("state");
    s.phone_pair.peer = get("peer");
    s.phone_pair.words = p
        .get("words")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|w| w.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if s.phone_pair.state == "done" {
        let w = s.w.clone();
        s.data.refresh_all(w.as_deref());
    }
}

fn join(s: &mut State) {
    let code = s.phone_pair.code.text.trim().to_string();
    let server = s.phone_pair.server.text.trim().to_string();
    if code.is_empty() {
        return;
    }
    s.phone_pair.error.clear();
    s.act(
        "Join",
        move |w| w.seams.pairing.join(&code, &server),
        |s, st| {
            s.phone_pair.state = st.state;
            s.phone_pair.words = st.words;
            s.phone_pair.peer = st.peer;
        },
    );
}

fn confirm_key(s: &mut State) {
    let key = s.phone_pair.key.text.trim().to_string();
    if key.is_empty() {
        return;
    }
    s.act(
        "Confirm the key",
        move |w| w.seams.pairing.confirm_key(&key),
        |s, ()| s.phone_pair.key.clear(),
    );
}

fn cancel(s: &mut State) {
    s.phone_pair = Model::default();
    s.spawn(|w| w.seams.pairing.cancel(), |_, ()| {});
}

fn field(
    f: &Frame,
    v: &TextEdit,
    key: &str,
    hint: &str,
    edit: fn(&mut State) -> &mut TextEdit,
    submit: fn(&mut State),
) -> El {
    let k = f.k;
    text_input(v, k.ty.ui)
        .key(key.to_string())
        .single_line()
        .placeholder(hint.to_string(), k.c.text_faint)
        .w_full()
        .padding(10.0)
        .radius(6.0)
        .border(1.0, k.c.border_base)
        .fill(k.c.layer_01)
        .on_edit(edit)
        .on_submit(move |s: &mut State, _: &mut Tx| submit(s))
}

fn para(f: &Frame, s: &str) -> El {
    text(s.to_string())
        .font(f.k.ty.ui)
        .color(f.k.c.text_muted)
        .w_full()
        .wrap()
}

/// Join with a code: the page this phone shows until it has a computer.
pub fn pair_view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let p = &s.phone_pair;
    let mut page = col()
        .key("pair-page")
        .w_full()
        .gap(14.0)
        .padding(20.0)
        .child(
            text("Join your computer")
                .font(rune::tok::TextStyle::new(20.0, 600.0))
                .color(k.c.text_base),
        );
    match p.state.as_str() {
        "words" | "granted" | "key" => {
            let peer = if p.peer.is_empty() {
                "your computer".to_string()
            } else {
                p.peer.clone()
            };
            page = page
                .child(para(f, &format!("Type these words on {peer}:")))
                .child(
                    text(p.words.join(" "))
                        .key("pair-words")
                        .font(rune::tok::TextStyle::new(18.0, 600.0).mono())
                        .color(k.c.text_base)
                        .w_full()
                        .wrap()
                        .selectable(true),
                )
                .child(para(
                    f,
                    "Then type the key your computer shows beside its code:",
                ))
                .child(field(
                    f,
                    &p.key,
                    "pair-key",
                    "The computer's key",
                    |s| &mut s.phone_pair.key,
                    confirm_key,
                ))
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .child(
                            Button::new("Cancel")
                                .variant(ButtonVariant::Ghost)
                                .on_press(cancel)
                                .build(k),
                        )
                        .child(el().grow(1.0))
                        .child(
                            Button::new("Confirm")
                                .variant(ButtonVariant::Submit)
                                .on_press(confirm_key)
                                .build(k),
                        ),
                );
        }
        "" | "failed" | "expired" | "canceled" | "code_used" => {
            page = page
                .child(para(
                    f,
                    "On your computer, open Settings → Devices → Add a device, then type the code it shows.",
                ))
                .child(field(
                    f,
                    &p.code,
                    "pair-code",
                    "Code, like ABCD-EFGH-JKLM-NPQR",
                    |s| &mut s.phone_pair.code,
                    join,
                ))
                .child(para(
                    f,
                    "Away from home, also its home server (host:port):",
                ))
                .child(field(
                    f,
                    &p.server,
                    "pair-server",
                    "Home server, optional",
                    |s| &mut s.phone_pair.server,
                    join,
                ))
                .child(
                    row().w_full().child(el().grow(1.0)).child(
                        Button::new("Join")
                            .variant(ButtonVariant::Submit)
                            .on_press(join)
                            .build(k),
                    ),
                );
            if !p.state.is_empty() {
                page = page.child(para(f, &format!("The last try: {}.", p.state)));
            }
        }
        other => {
            page = page.child(para(f, &format!("Joining… ({other})"))).child(
                Button::new("Cancel")
                    .variant(ButtonVariant::Ghost)
                    .on_press(cancel)
                    .build(k),
            );
        }
    }
    page
}

/// Headless scenes in the phone's frame (`irori --snap p.png --size
/// 412x915 --scene phone`): the Board, or Join with a code on a data dir
/// with no other device.
pub fn scene(s: &mut State) {
    s.phone = true;
    s.close_card();
    s.go(crate::ui::Route::Home);
}

/// The phone's card page: the first card, as `--scene card` picks it.
pub fn card_scene(s: &mut State) {
    s.phone = true;
    crate::card::scene(s);
}
