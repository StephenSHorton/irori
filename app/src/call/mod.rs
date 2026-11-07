//! Calls (P6: huddles, voice input and screen share), the Rune port of the
//! React app's components/talk/huddle.tsx, share.tsx, transcript-pane.tsx,
//! common/mic-button.tsx and settings/voice-note.tsx, with lib/call.ts,
//! share.ts, voice.ts and mic-shortcut.ts (on main, under frontend/src/).
//!
//! The picture is `irori_window::call::CallView` (`Window::call_state`),
//! re-read on every `call:state`. This package owns the huddle's state for
//! the whole window: the ringing banner and the call bar over every page
//! ([`overlay`]), the notice before the mic opens, the start sheet, the
//! share picker and viewer, an open transcript and dictation. Talk (P5)
//! places the pieces in a conversation; see the `pub fn`s below:
//! - [`huddle_button`] for the conversation header (Huddle, Join, In huddle),
//! - [`huddle_panel`] beside the posts while this device is in its huddle,
//! - [`share_stage`] in the posts' place while a share is on,
//! - [`transcript_pane`] beside the posts once [`open_transcript`] ran,
//! - [`huddle_started`], [`huddle_ended`] and [`huddle_line`] for posts and
//!   list rows, [`ask_join`] for a Join anywhere,
//! - [`in_call_here`] for whether the panel and the stage show,
//! - [`mic_button`] for a composer, [`on_key`] for the dictation shortcut,
//!   [`voice_note`] and [`set_transcribe`] for Settings.

mod dictation;
mod huddle;
mod icons;
mod logic;
mod mic;
mod share;
mod transcript;
mod video;

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use irori_window::call::{
    CallCaptionView, CallLiveView, CallMineView, CallPeerView, CallRingView, CallShareView,
    CallView, EVENT_CALL_STATE, ShareSourceView,
};
use irori_window::talk::TalkTranscript;
use irori_window::voice::{EVENT_VOICE, EVENT_VOICE_DOWNLOAD, VoiceStatus};
use rune::{El, TextEdit};

use crate::ui::{Event, Frame, Route, State};

// The pieces other packages place (Talk, the card page, the window's keys).
pub use huddle::{huddle_button, huddle_ended, huddle_line, huddle_panel, huddle_started};
pub use logic::StartWhere;
pub use mic::{Target as MicTarget, mic_button, mic_button_in, on_key};
// Settings' voice note.
pub use mic::{refresh_voice, voice_note};
pub use share::share_stage;
pub use transcript::transcript_pane;

/// A huddle about to open the mic: starting one, or joining.
#[derive(Clone, Debug, PartialEq)]
pub struct NoticeAsk {
    pub start: bool,
    /// The huddle a join is for: the notice closes once it's no longer
    /// ringing or on.
    pub call_id: String,
    pub conv_id: String,
    pub conv_kind: String,
    pub conv_name: String,
}

/// The start sheet open on one conversation's Huddle.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub conv_id: String,
    pub at: StartWhere,
    /// The mic's name (None until it answers; "" when it can't say).
    pub mic: Option<String>,
}

/// The transcript open beside a conversation.
pub struct Transcript {
    pub call_id: String,
    pub got: Option<Result<TalkTranscript, String>>,
    pub q: TextEdit,
}

/// The share picker's list.
#[derive(Default)]
pub struct Picker {
    pub open: bool,
    /// None while it looks.
    pub list: Option<Vec<ShareSourceView>>,
    pub error: String,
}

#[derive(Default)]
pub struct Model {
    /// The whole picture, as call:state last said.
    pub view: CallView,
    /// call:state reads in flight: only the newest lands.
    seq: u64,
    /// Now, Unix ms, moved each second while in a huddle (the timers).
    pub now_ms: i64,
    ticker: Option<Arc<AtomicBool>>,
    /// A Start, Join or Leave is running.
    pub busy: bool,
    /// Why the last huddle action failed (the panel, the start sheet).
    pub error: String,
    /// A Join from the ring banner: its call, and why it failed.
    pub ring_busy: String,
    pub ring_error: (String, String),
    /// The notice before the mic opens (#656).
    pub notice: Option<NoticeAsk>,
    /// This device can write down its person's voice: None while asking
    /// (the notice and the start sheet ask afresh each time they open).
    pub can_transcribe: Option<bool>,
    pub sheet: Option<Sheet>,
    pub picker: Picker,
    /// The share being watched.
    viewer: Option<video::Viewer>,
    /// The share stage fills the window (Full screen).
    pub full: bool,
    pub transcript: Option<Transcript>,
    // ---- voice input ----
    pub voice: Option<VoiceStatus>,
    /// The model download: (done, total) while it runs.
    pub download: Option<(u64, u64)>,
    pub download_error: String,
    dict: Option<mic::Dictation>,
    /// The mic whose note (download offer, error) is open.
    mic_note: Option<(String, mic::Note)>,
    /// The mics drawn this frame and the last, for the shortcut.
    mics: RefCell<(Vec<mic::Drawn>, Vec<mic::Drawn>)>,
    /// The headless scene: the conversation, the panel and the stage draw
    /// over the page, and Leave and Stop sharing act on the fake picture.
    demo: bool,
    /// The scene's composer, for its mic.
    demo_draft: TextEdit,
    /// The scene's huddle is in a real Talk channel (IRORI_CALL_SCENE
    /// "talk"): Talk draws it, not the stand-in conversation.
    demo_in_talk: bool,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Tabular figures, for timers and clocks.
fn tnum(st: rune::tok::TextStyle) -> rune::tok::TextStyle {
    rune::tok::TextStyle { features: "\"tnum\"", ..st }
}

/// The first `n` bytes of a line, in the name's weight and colour.
fn bold_span(n: usize, c: rune::Color) -> rune::Span {
    rune::Span { color: Some(c), ..rune::Span::new(0, n).weight(560.0) }
}

pub(crate) fn log(msg: &str) {
    eprintln!("irori call: {msg}");
}

// ---- events ------------------------------------------------------------------

pub fn on_event(s: &mut State, ev: &Event) {
    match ev.name.as_str() {
        "app:start" => {
            s.call.now_ms = now_ms();
            refresh(s);
            mic::load_status(s, false);
        }
        EVENT_CALL_STATE => refresh(s),
        EVENT_VOICE => mic::on_voice(s, &ev.payload),
        EVENT_VOICE_DOWNLOAD => {
            let g = |k: &str| ev.payload.get(k).and_then(serde_json::Value::as_u64).unwrap_or(0);
            if s.call.download.is_some() {
                s.call.download = Some((g("done"), g("total")));
            }
        }
        _ => {}
    }
}

/// Re-reads the picture (call:state's payload has NaN as null, which the
/// window's types don't read back, so the Window answers instead).
fn refresh(s: &mut State) {
    if s.call.demo {
        return;
    }
    s.call.seq += 1;
    let seq = s.call.seq;
    s.spawn(
        |w| w.call_state(),
        move |s, v| {
            if seq == s.call.seq {
                apply(s, v);
            }
        },
    );
}

/// The picture changed: keep what hangs off it in step.
fn apply(s: &mut State, v: CallView) {
    s.call.view = v;
    let mine = s.call.view.mine.clone();
    // The timers tick only while in a huddle.
    match (&mine, &s.call.ticker) {
        (Some(_), None) => start_ticker(s),
        (None, Some(t)) => {
            t.store(false, Ordering::Relaxed);
            s.call.ticker = None;
        }
        _ => {}
    }
    // A join's huddle that ended, or stopped ringing, closes its notice.
    if let Some(n) = &s.call.notice
        && !n.start
        && !n.call_id.is_empty()
        && !s.call.view.ringing.iter().any(|r| r.call_id == n.call_id)
        && !s.call.view.live.iter().any(|l| l.call_id == n.call_id)
    {
        s.call.notice = None;
    }
    // The sheet closes when a huddle comes on in its conversation.
    if let Some(sh) = &s.call.sheet
        && (logic::in_call_here(&s.call.view, &sh.conv_id)
            || s.call.view.live.iter().any(|l| l.conv_id == sh.conv_id && !l.people.is_empty()))
    {
        s.call.sheet = None;
    }
    // A huddle has the mic: dictation lets go of it.
    if mine.is_some() {
        mic::stop(s);
    }
    watch_share(s, mine.as_ref().and_then(|m| m.share.as_ref()));
    if mine.as_ref().is_none_or(|m| m.share.as_ref().is_none_or(|x| x.mine)) {
        s.call.full = false;
        s.call.picker.open = s.call.picker.open && mine.as_ref().is_some_and(|m| m.can_share);
    }
}

/// Watches another's share while one is on in this huddle (the React app
/// watched only while the stage was on screen; Rune has no unmount, so
/// here it is while the share is on).
fn watch_share(s: &mut State, share: Option<&CallShareView>) {
    let want = share.filter(|x| !x.mine).map(|x| x.share_id.clone());
    let have = s.call.viewer.as_ref().map(|v| v.share_id.clone());
    if want == have || s.headless {
        return;
    }
    let Some(w) = s.w.clone() else { return };
    if let Some(v) = s.call.viewer.take() {
        v.detach(&w);
    }
    if let Some(id) = want {
        match video::Viewer::attach(&w, s.handle.clone(), &id) {
            Ok(v) => s.call.viewer = Some(v),
            Err(e) => log(&format!("share attach: {e}")),
        }
    }
}

fn start_ticker(s: &mut State) {
    s.call.now_ms = now_ms();
    let Some(h) = s.handle.clone() else { return };
    if s.headless {
        return;
    }
    let on = Arc::new(AtomicBool::new(true));
    s.call.ticker = Some(on.clone());
    std::thread::spawn(move || {
        while on.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(1000 - (now_ms() % 1000) as u64));
            if !on.load(Ordering::Relaxed) {
                break;
            }
            h.update(|s: &mut State, _| s.call.now_ms = now_ms());
        }
    });
}

// ---- actions -----------------------------------------------------------------

fn mine_mut(s: &mut State) -> Option<&mut CallMineView> {
    s.call.view.mine.as_mut()
}

/// Mute (or unmute) shows at once; call:state confirms it. Unmuting while
/// deafened undeafens too.
pub fn set_muted(s: &mut State, on: bool) {
    let mut undeafen = false;
    if let Some(m) = mine_mut(s) {
        undeafen = !on && m.deafened;
        m.muted = on;
        if undeafen {
            m.deafened = false;
        }
    }
    s.act(
        "Mute",
        move |w| {
            if undeafen {
                w.call_deafen(false);
            }
            w.call_mute(on);
            Ok(())
        },
        |_, ()| {},
    );
}

/// Deafen turns the speaker and the mic off (or back) at once; undeafen
/// keeps the mute it had.
pub fn set_deafened(s: &mut State, on: bool) {
    if let Some(m) = mine_mut(s) {
        m.deafened = on;
        if on {
            m.speaking = false;
        }
    }
    s.act(
        "Deafen",
        move |w| {
            w.call_deafen(on);
            Ok(())
        },
        |_, ()| {},
    );
}

pub fn leave(s: &mut State) {
    if s.call.demo {
        s.call.view.mine = None;
        s.call.view.live.clear();
        return;
    }
    s.call.busy = true;
    s.call.error.clear();
    s.spawn(
        |w| w.call_leave(),
        |s, r| {
            s.call.busy = false;
            if let Err(e) = r {
                s.call.error = e;
            }
        },
    );
}

pub fn dismiss_ring(s: &mut State, call_id: &str) {
    s.call.view.ringing.retain(|r| r.call_id != call_id);
    let id = call_id.to_string();
    s.spawn(move |w| w.call_dismiss(&id), |_, ()| {});
}

/// Asks before a Join opens the mic (the notice); its Join joins.
pub fn ask_join(s: &mut State, call_id: &str, conv_id: &str, conv_kind: &str, conv_name: &str) {
    s.call.notice = Some(NoticeAsk {
        start: false,
        call_id: call_id.to_string(),
        conv_id: conv_id.to_string(),
        conv_kind: conv_kind.to_string(),
        conv_name: conv_name.to_string(),
    });
    check_transcribe(s);
}

/// Whether this device can write down its person's voice, asked afresh.
fn check_transcribe(s: &mut State) {
    s.call.can_transcribe = None;
    s.spawn(
        |w| {
            let v = w.call_state();
            v.can_transcribe
        },
        |s, ok| s.call.can_transcribe = Some(ok),
    );
}

/// Joins huddle `call_id` (the notice's Join). A ring's join opens its
/// conversation once in.
fn join(s: &mut State, call_id: String, conv_id: String, from_ring: bool) {
    mic::stop(s);
    if from_ring {
        s.call.ring_busy = call_id.clone();
        s.call.ring_error = (String::new(), String::new());
    } else {
        s.call.busy = true;
        s.call.error.clear();
    }
    let id = call_id.clone();
    s.spawn(
        move |w| w.call_join(&id),
        move |s, r| {
            if s.call.ring_busy == call_id {
                s.call.ring_busy.clear();
            }
            if !from_ring {
                s.call.busy = false;
            }
            match r {
                Ok(()) => {
                    if from_ring {
                        s.close_card();
                        s.go(Route::Talk(Some(conv_id)));
                    }
                    refresh(s);
                }
                Err(e) if from_ring => s.call.ring_error = (call_id, e),
                Err(e) => s.call.error = e,
            }
        },
    );
}

/// Starts a huddle in `conv_id` (the start sheet's Start huddle).
pub fn start(s: &mut State, conv_id: &str) {
    mic::stop(s);
    s.call.busy = true;
    s.call.error.clear();
    let id = conv_id.to_string();
    s.spawn(
        move |w| w.call_start(&id),
        |s, r| {
            s.call.busy = false;
            match r {
                Ok(_) => {
                    s.call.sheet = None;
                    refresh(s);
                }
                Err(e) => s.call.error = e,
            }
        },
    );
}

/// Opens (or closes) the start sheet on a conversation's Huddle.
pub fn open_sheet(s: &mut State, conv_id: &str, at: StartWhere) {
    s.call.error.clear();
    s.call.sheet = Some(Sheet {
        conv_id: conv_id.to_string(),
        at,
        mic: None,
    });
    check_transcribe(s);
    let id = conv_id.to_string();
    s.spawn(
        |w| w.call_mic(),
        move |s, name| {
            if let Some(sh) = s.call.sheet.as_mut().filter(|sh| sh.conv_id == id) {
                sh.mic = Some(name);
            }
        },
    );
}

#[allow(dead_code)] // for Settings (P8), whose general pane calls the Window itself for now
/// "Transcribe what I say in calls" on or off on all the person's
/// devices; a huddle in progress follows at once. Shows at once,
/// call:state confirms it.
pub fn set_transcribe(s: &mut State, on: bool) {
    s.call.view.transcribe_on = on;
    if let Some(m) = mine_mut(s) {
        m.transcribe_on = on;
    }
    s.act("Transcribe", move |w| w.call_transcribe_set(on), |_, ()| {});
}

#[allow(dead_code)] // for Settings (P8), whose general pane reads it itself for now
/// The person's "Transcribe what I say in calls" switch, as last read.
pub fn transcribe_on(s: &State) -> bool {
    s.call.view.transcribe_on
}

/// Opens the transcript of huddle `call_id` beside its conversation.
pub fn open_transcript(s: &mut State, call_id: &str) {
    s.call.transcript = Some(Transcript {
        call_id: call_id.to_string(),
        got: None,
        q: TextEdit::new(""),
    });
    let id = call_id.to_string();
    s.spawn(
        move |w| w.talk_call_transcript(&id),
        {
            let id = call_id.to_string();
            move |s, r| {
                if let Some(t) = s.call.transcript.as_mut().filter(|t| t.call_id == id) {
                    t.got = Some(r);
                }
            }
        },
    );
}

pub fn close_transcript(s: &mut State) {
    s.call.transcript = None;
}

/// Whether this device is in `conv_id`'s huddle (Talk shows the panel).
pub fn in_call_here(s: &State, conv_id: &str) -> bool {
    logic::in_call_here(&s.call.view, conv_id)
}

// ---- the package contract -----------------------------------------------------

#[allow(dead_code)] // the package contract; no route shows it
/// The package's page: the huddle this device is in, or a line saying
/// none is (no route shows it; Talk embeds [`huddle_panel`]).
pub fn view(s: &State, f: &mut Frame) -> El {
    if s.call.view.mine.is_some() {
        return huddle_panel(s, f);
    }
    crate::ui::not_yet(f, "Huddle", "You're not in a huddle. Start one from a channel or a DM in Talk.")
}

/// Over every page: the ringing banner, the call bar, the notice before
/// the mic opens, and the share stage when it fills the window.
pub fn overlay(s: &State, f: &mut Frame) {
    {
        let mut m = s.call.mics.borrow_mut();
        let drawn = std::mem::take(&mut m.0);
        m.1 = drawn;
    }
    if s.call.demo && !s.call.demo_in_talk {
        huddle::demo(s, f);
    }
    share::full_screen(s, f);
    huddle::call_bar(s, f);
    huddle::notice(s, f);
}

/// The ring banner's row, between the update banner and the page (the
/// root view puts it there): a huddle someone started where you can join
/// it. Empty when nothing rings.
pub fn ring_row(s: &State, f: &mut Frame) -> El {
    huddle::ring(s, f)
}

/// The headless scene: a fake huddle in a channel with two others, one
/// of them speaking, captions written down, and the panel open.
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Talk(None));
    let now = now_ms();
    let peer = |name: &str, path: &str, rtt: f64, loss: f64, speaking: bool, muted: bool, tr: Option<bool>| CallPeerView {
        name: name.into(),
        device: String::new(),
        phase: "live".into(),
        path: path.into(),
        rtt_ms: rtt,
        loss_pct: loss,
        speaking,
        muted,
        transcribing: tr,
    };
    let said = [
        ("you", "Okay, quick one: are we tagging 0.10.0 today or waiting on #8?"),
        ("Spencer", "#8 merged an hour ago. The only thing left is the PC transfer, it still needs the Firewall click."),
        ("you", "Then let's tag at two, right after the click."),
    ];
    let captions = said
        .iter()
        .enumerate()
        .map(|(i, (n, t))| {
            let at = now - (3 - i as i64) * 9_000;
            CallCaptionView {
                name: (*n).into(),
                mine: *n == "you",
                text: (*t).into(),
                at_ms: at,
                end_ms: at + 4_000,
            }
        })
        .collect();
    let mine = CallMineView {
        call_id: "scene-call".into(),
        conv_id: "scene-ch".into(),
        conv_name: "suzuri".into(),
        conv_kind: "channel".into(),
        joined_at_ms: now - 252_000,
        device: "MacBook Pro".into(),
        peers: vec![
            peer("Spencer", "direct", 26.0, 0.0, true, false, Some(true)),
            peer("Aiko", "relay", 120.0, 1.4, false, true, Some(false)),
        ],
        transcribing: true,
        can_transcribe: true,
        transcribe_on: true,
        captions,
        share: Some(CallShareView {
            share_id: "scene-share".into(),
            name: "Spencer".into(),
            kind: "window".into(),
            title: "Windows Terminal".into(),
            app: "Windows Terminal".into(),
            width: 1280,
            height: 720,
            fps: 10,
            kbps: 1800,
            path: "direct".into(),
            ..Default::default()
        }),
        can_share: true,
        ..Default::default()
    };
    // IRORI_CALL_SCENE picks a variant: "posts" (no share: the posts, a
    // composer with its mic), "mine" (your own share), "alone" (just you,
    // an error, no captions), "talk" (no share, in the newest real
    // channel, drawn by Talk itself: the panel beside its posts).
    let variant = std::env::var("IRORI_CALL_SCENE").unwrap_or_default();
    let mut mine = mine;
    let mut in_talk = None;
    match variant.as_str() {
        "talk" => {
            mine.share = None;
            let convs = s.win().talk_conversations().unwrap_or_default();
            in_talk = convs
                .iter()
                .filter(|c| c.kind == "channel" && !c.read_only)
                .max_by_key(|c| crate::talk::logic::at_ms(&c.last_at))
                .map(|c| (c.conv_id.clone(), c.name.clone()));
            if let Some((id, name)) = &in_talk {
                mine.conv_id = id.clone();
                mine.conv_name = name.clone();
            }
        }
        "posts" => mine.share = None,
        "mine" => {
            if let Some(x) = mine.share.as_mut() {
                x.mine = true;
                x.name = "you".into();
                x.title = "irori · #658 share stage".into();
                x.app = "Visual Studio Code".into();
                x.path = String::new();
            }
            mine.share_note = Some("Aiko's share stopped, and they were told.".into());
        }
        "alone" => {
            mine.peers.clear();
            mine.captions.clear();
            mine.share = None;
            mine.transcribing = false;
            mine.error = Some("The microphone stopped: it was unplugged.".into());
        }
        _ => {}
    }
    s.call.demo = true;
    s.call.now_ms = now;
    s.call.demo_draft = TextEdit::new("Ship it once the two PC check passes");
    s.call.view = CallView {
        mine: Some(mine),
        ringing: vec![CallRingView {
            call_id: "scene-ring".into(),
            conv_id: "scene-ch-2".into(),
            conv_name: "netcode".into(),
            conv_kind: "channel".into(),
            by: "Kenji".into(),
            by_device: String::new(),
            at_ms: now - 20_000,
        }],
        live: vec![CallLiveView {
            call_id: "scene-call".into(),
            conv_id: in_talk.as_ref().map_or("scene-ch".into(), |(id, _)| id.clone()),
            people: vec!["you".into(), "Spencer".into(), "Aiko".into()],
        }],
        transcribe_on: true,
        can_transcribe: true,
    };
    if let Some((id, _)) = in_talk {
        s.call.demo_in_talk = true;
        crate::talk::on_event(s, &crate::ui::Event {
            name: "app:start".into(),
            payload: serde_json::Value::Null,
        });
        crate::talk::open(s, &id);
    }
}
