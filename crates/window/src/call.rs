//! Huddles' window face (#654): the voice calls in a channel or a person
//! DM. `call_state` is the whole picture (the call this device is in, the
//! calls ringing here, the calls on in conversations it can see), and the
//! pump sends it again as `call:state` on every change. People and
//! conversations are named here, never sent as keys or ids.

use std::collections::HashMap;

use irori_core::{CallShare, CallShareNote, CallSnapshot, LiveCall, MyCall, Ringing, ShareSource};
use serde::Serialize;
use specta::Type;

use crate::Window;

/// The call:state event.
pub const EVENT_CALL_STATE: &str = "call:state";

/// Every call this device knows of, as the window shows them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CallView {
    /// The call this device is in.
    pub mine: Option<CallMineView>,
    /// Calls this device is rung for and hasn't joined or dismissed.
    pub ringing: Vec<CallRingView>,
    /// Calls on now, for the Talk list's rows.
    pub live: Vec<CallLiveView>,
    /// "Transcribe what I say in calls" (#656), the person's switch: the
    /// notice before the mic opens reads it.
    pub transcribe_on: bool,
    /// This device can write down what the person says now (#656): this
    /// machine has a speech engine and the speech model is downloaded.
    /// Known before any call, for the notice; worked out each time the view
    /// is built. (Whether the app installed a transcriber isn't part of it:
    /// the app always does.)
    pub can_transcribe: bool,
}

/// The call this device is in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CallMineView {
    pub call_id: String,
    pub conv_id: String,
    /// The channel's name, or the DM's other person.
    pub conv_name: String,
    /// `channel` or `dm`.
    pub conv_kind: String,
    /// When this device joined, Unix ms.
    pub joined_at_ms: i64,
    pub muted: bool,
    /// Deafened: nothing plays here and the mic is off.
    pub deafened: bool,
    /// This device's mic is above the speech gate.
    pub speaking: bool,
    /// This device's name ("" when it has none).
    pub device: String,
    /// The other devices in it.
    pub peers: Vec<CallPeerView>,
    /// Why the call isn't working, in plain words.
    pub error: Option<String>,
    /// What this device says is written down now (#656).
    pub transcribing: bool,
    /// This device can write down what the person says now: it has a
    /// transcriber, a speech engine, and the speech model is downloaded.
    pub can_transcribe: bool,
    /// "Transcribe what I say in calls", the person's switch.
    pub transcribe_on: bool,
    /// The latest lines said in the call, oldest first.
    pub captions: Vec<CallCaptionView>,
    /// The screen share on in the call, yours or another's (#658).
    pub share: Option<CallShareView>,
    /// This device can share its screen (the desktops; not the phone).
    pub can_share: bool,
    /// A line about sharing, in plain words: who took over, or why your
    /// share stopped.
    pub share_note: Option<String>,
}

/// One line said in the call (#656).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CallCaptionView {
    /// Who said it, by name ("you" for the person, on any of their
    /// devices).
    pub name: String,
    /// The person said it.
    pub mine: bool,
    pub text: String,
    /// When the phrase began, Unix ms.
    pub at_ms: i64,
    /// When it ended, Unix ms.
    pub end_ms: i64,
}

/// The screen share on in a call.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CallShareView {
    pub share_id: String,
    /// You are the one sharing (this device).
    pub mine: bool,
    /// Who shares: "you" for the person, else their name.
    pub name: String,
    /// The sharing device's name when it is one of the person's own, else "".
    pub device: String,
    /// `window` or `display`.
    pub kind: String,
    /// The window's title, or the display's name.
    pub title: String,
    /// The app that owns the window ("" for a display).
    pub app: String,
    /// The picture's size, pixels (0 until the capture runs).
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// The sharer's target bitrate now, kbit/s.
    pub kbps: u32,
    /// How this device reaches the sharer: `direct`, `relay`, or "" (it is
    /// this device).
    pub path: String,
}

/// Something that can be shared: a display or a window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ShareSourceView {
    /// Pass to `share_start`.
    pub id: String,
    /// `window` or `display`.
    pub kind: String,
    pub title: String,
    pub app: String,
    pub width: u32,
    pub height: u32,
}

impl From<ShareSource> for ShareSourceView {
    fn from(s: ShareSource) -> Self {
        ShareSourceView {
            id: s.id,
            kind: s.kind,
            title: s.title,
            app: s.app,
            width: s.width,
            height: s.height,
        }
    }
}

/// Another device in the call.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CallPeerView {
    /// Whose it is ("you" for another of the person's own devices).
    pub name: String,
    /// The device's name when it is one of the person's own, else "".
    pub device: String,
    /// `connecting`, `live` or `gone`.
    pub phase: String,
    /// `direct`, `relay` or "" (not connected).
    pub path: String,
    pub rtt_ms: f64,
    /// Datagrams lost on the way here, percent.
    pub loss_pct: f64,
    pub speaking: bool,
    pub muted: bool,
    /// Whether what they say is written down (#656); null until their
    /// device says (an older Irori never does).
    pub transcribing: Option<bool>,
}

/// A call ringing here.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CallRingView {
    pub call_id: String,
    pub conv_id: String,
    pub conv_name: String,
    /// `channel` or `dm`.
    pub conv_kind: String,
    /// Who started it.
    pub by: String,
    /// The device that rang, by name, when it is one of the person's own
    /// (a Personal channel's huddle, #663, or one started on another of
    /// their devices); "" otherwise.
    pub by_device: String,
    /// When it rang, Unix ms.
    pub at_ms: i64,
}

/// A call on now in a conversation this device can see.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CallLiveView {
    pub call_id: String,
    pub conv_id: String,
    /// Who is in it, each person once, by name.
    pub people: Vec<String>,
}

impl Window {
    /// The calls on now (`call:state`'s payload).
    pub fn call_state(&self) -> CallView {
        self.call_view(self.core.call_snapshot())
    }

    /// Snapshot `s` as the window shows it (tests feed their own).
    #[doc(hidden)]
    pub fn call_view(&self, s: CallSnapshot) -> CallView {
        let names = Names::new(self, &s);
        let transcribe_on = self.core.call_transcribe();
        let can_transcribe = crate::voice::can_transcribe_here(self.core.data_dir());
        CallView {
            mine: s.mine.map(|m| names.mine(m, transcribe_on, can_transcribe)),
            ringing: s.ringing.into_iter().map(|r| names.ring(r)).collect(),
            live: s.live.into_iter().map(|l| names.live(l)).collect(),
            transcribe_on,
            can_transcribe,
        }
    }

    /// Starts a huddle in channel or person DM `conv`; returns its id.
    /// Blocking: the mic and speaker open first.
    pub fn call_start(&self, conv: &str) -> Result<String, String> {
        let conv = crate::trim(conv);
        if conv.is_empty() {
            return Err("pick a conversation".into());
        }
        self.core.call_start(conv)
    }

    /// Joins call `call_id`. Blocking, as [`Window::call_start`].
    pub fn call_join(&self, call_id: &str) -> Result<(), String> {
        self.core.call_join(crate::trim(call_id))
    }

    /// Leaves the call this device is in.
    pub fn call_leave(&self) -> Result<(), String> {
        self.core.call_leave()
    }

    pub fn call_mute(&self, on: bool) {
        self.core.call_mute(on)
    }

    pub fn call_deafen(&self, on: bool) {
        self.core.call_deafen(on)
    }

    /// The mic a huddle would open, by name, for the start sheet ("" when
    /// it can't say).
    pub fn call_mic(&self) -> String {
        self.core.call_mic_name()
    }

    /// Stops ringing for `call_id` here.
    pub fn call_dismiss(&self, call_id: &str) {
        self.core.call_dismiss(crate::trim(call_id))
    }

    /// "Transcribe what I say in calls" (#656): on unless the person
    /// turned it off.
    pub fn call_transcribe(&self) -> bool {
        self.core.call_transcribe()
    }

    /// Turns "Transcribe what I say in calls" on or off on all the
    /// person's devices; a call this device is in follows at once.
    pub fn call_transcribe_set(&self, on: bool) -> Result<(), String> {
        self.core.set_call_transcribe(on)
    }

    /// "Name new threads" (card #782): on unless the person turned it
    /// off.
    pub fn assistant_names(&self) -> bool {
        self.core.names_on()
    }

    /// Turns "Name new threads" on or off on all the person's devices.
    pub fn assistant_names_set(&self, on: bool) -> Result<(), String> {
        self.core.set_names_on(on)
    }

    /// "Weigh new cards" (card #965): on unless the person turned it off.
    pub fn assistant_weighs(&self) -> bool {
        self.core.weighs_on()
    }

    /// Turns "Weigh new cards" on or off on all the person's devices.
    pub fn assistant_weighs_set(&self, on: bool) -> Result<(), String> {
        self.core.set_weighs_on(on)
    }

    /// What this device can share: displays, then windows.
    pub fn share_sources(&self) -> Result<Vec<ShareSourceView>, String> {
        Ok(self
            .core
            .share_sources()?
            .into_iter()
            .map(ShareSourceView::from)
            .collect())
    }

    /// Shares source `id` in the huddle this device is in. Blocking: the
    /// capture and encoder start first.
    pub fn share_start(&self, id: &str) -> Result<(), String> {
        let id = crate::trim(id);
        if id.is_empty() {
            return Err("pick a window or screen".into());
        }
        self.core.share_start(id)
    }

    pub fn share_stop(&self) {
        self.core.share_stop()
    }

    /// Leaves any call and closes the devices: the shell calls it as the
    /// app exits, before [`Window::voice_shutdown`] frees the speech model
    /// the call's transcriber may hold.
    pub fn call_shutdown(&self) {
        self.core.call_shutdown()
    }
}

/// The names one snapshot needs, looked up once.
struct Names<'a> {
    w: &'a Window,
    me: String,
    people: HashMap<String, String>,
    devices: std::collections::BTreeMap<String, String>,
    /// The snapshot's conversations (mine, ringing): kind and name.
    convs: HashMap<String, (String, String)>,
}

impl<'a> Names<'a> {
    fn new(w: &'a Window, s: &CallSnapshot) -> Names<'a> {
        let me = w.core.chain().person_id();
        let mut ids: Vec<&str> = Vec::new();
        if let Some(m) = &s.mine {
            ids.extend(m.peers.iter().map(|p| p.person_id.as_str()));
            // A speaker who has left is still in the captions.
            ids.extend(m.captions.iter().map(|c| c.person_id.as_str()));
            ids.extend(m.share.iter().map(|x| x.person_id.as_str()));
            if let Some(CallShareNote::Replaced { person_id, .. }) = &m.share_note {
                ids.push(person_id);
            }
        }
        ids.extend(s.ringing.iter().map(|r| r.by_person.as_str()));
        ids.extend(
            s.live
                .iter()
                .flat_map(|l| l.devices.iter().map(|(_, p)| p.as_str())),
        );
        let mut keys: Vec<String> = Vec::new();
        for id in ids {
            let k = person_key(id);
            if !id.is_empty() && id != me && !keys.contains(&k) {
                keys.push(k);
            }
        }
        // The conversations' names: a DM's other person is named in the
        // same single lookup as the call's people.
        let talk = w.core.talk();
        let mut convs: HashMap<String, (String, String)> = HashMap::new();
        let mut dms: Vec<(String, String)> = Vec::new();
        let conv_ids = s
            .mine
            .iter()
            .map(|m| m.conv_id.as_str())
            .chain(s.ringing.iter().map(|r| r.conv_id.as_str()));
        for conv in conv_ids {
            if convs.contains_key(conv) || dms.iter().any(|(c, _)| c == conv) {
                continue;
            }
            if coord::talk::is_channel(conv) {
                let name = talk.channel(conv).map(|c| c.name).unwrap_or_default();
                convs.insert(conv.to_string(), ("channel".into(), name));
            } else if let Some(k) = talk.pair_other(conv) {
                if !keys.contains(&k) {
                    keys.push(k.clone());
                }
                dms.push((conv.to_string(), k));
            } else {
                convs.insert(
                    conv.to_string(),
                    ("dm".into(), tools::talk::UNKNOWN_PERSON.into()),
                );
            }
        }
        let people: HashMap<String, String> =
            keys.iter().cloned().zip(w.core.talk_names(&keys)).collect();
        for (conv, k) in dms {
            let name = people
                .get(&k)
                .cloned()
                .unwrap_or_else(|| tools::talk::UNKNOWN_PERSON.into());
            convs.insert(conv, ("dm".into(), name));
        }
        Names {
            w,
            me,
            people,
            devices: w.core.device_labels(),
            convs,
        }
    }

    /// Person `id` by name: "you" for the person, "another person" when
    /// nothing names them.
    fn person(&self, id: &str) -> String {
        if id == self.me {
            return "you".into();
        }
        self.people
            .get(&person_key(id))
            .cloned()
            .unwrap_or_else(|| tools::talk::UNKNOWN_PERSON.into())
    }

    /// Device `id`'s name when it is one of the person's own, else "".
    fn device(&self, person: &str, id: &str) -> String {
        if person != self.me {
            return String::new();
        }
        self.devices.get(id).cloned().unwrap_or_default()
    }

    /// Conversation `conv`'s kind and name: a channel's name, a DM's other
    /// person.
    fn conv(&self, conv: &str) -> (String, String) {
        self.convs.get(conv).cloned().unwrap_or_default()
    }

    /// `here`: [`crate::voice::can_transcribe_here`].
    fn mine(&self, m: MyCall, transcribe_on: bool, here: bool) -> CallMineView {
        let (conv_kind, conv_name) = self.conv(&m.conv_id);
        CallMineView {
            device: self
                .devices
                .get(self.w.core.device_id())
                .cloned()
                .unwrap_or_default(),
            peers: m
                .peers
                .into_iter()
                .map(|p| CallPeerView {
                    name: self.person(&p.person_id),
                    device: self.device(&p.person_id, &p.device_id),
                    phase: p.phase,
                    path: p.path,
                    rtt_ms: p.rtt_ms,
                    loss_pct: p.loss_pct,
                    speaking: p.speaking,
                    muted: p.muted,
                    transcribing: p.transcribing,
                })
                .collect(),
            captions: m
                .captions
                .into_iter()
                .map(|c| {
                    // Said on this device, or another of the person's.
                    let mine = c.mine || c.person_id == self.me;
                    CallCaptionView {
                        name: if mine {
                            self.person(&self.me)
                        } else {
                            self.person(&c.person_id)
                        },
                        mine,
                        text: c.text,
                        at_ms: c.start_ms,
                        end_ms: c.end_ms,
                    }
                })
                .collect(),
            transcribing: m.transcribing,
            can_transcribe: m.can_transcribe && here,
            transcribe_on,
            call_id: m.call_id,
            conv_id: m.conv_id,
            conv_name,
            conv_kind,
            share: m.share.map(|x| self.share(x)),
            can_share: m.can_share,
            share_note: m.share_note.map(|n| match n {
                CallShareNote::Replaced { person_id, .. } => {
                    let who = self.person(&person_id);
                    if who == "you" {
                        "Your other device started sharing, so this one stopped".into()
                    } else {
                        format!("{who} started sharing, so yours stopped")
                    }
                }
                CallShareNote::Failed(why) => why,
            }),
            joined_at_ms: m.joined_at_ms,
            muted: m.muted,
            deafened: m.deafened,
            speaking: m.speaking,
            error: m.error.filter(|e| !e.trim().is_empty()),
        }
    }

    fn share(&self, x: CallShare) -> CallShareView {
        let i = x.now.info;
        CallShareView {
            name: if x.now.mine {
                "you".into()
            } else {
                self.person(&x.person_id)
            },
            device: if x.now.mine {
                String::new()
            } else {
                self.device(&x.person_id, &x.now.device_id)
            },
            mine: x.now.mine,
            share_id: i.share_id,
            kind: i.kind,
            title: i.title,
            app: i.app,
            width: i.width,
            height: i.height,
            fps: i.fps,
            kbps: i.kbps,
            path: x.path,
        }
    }

    fn ring(&self, r: Ringing) -> CallRingView {
        let (conv_kind, conv_name) = self.conv(&r.conv_id);
        CallRingView {
            by: self.person(&r.by_person),
            by_device: self.device(&r.by_person, &r.by_device),
            call_id: r.call_id,
            conv_id: r.conv_id,
            conv_name,
            conv_kind,
            at_ms: r.at_ms,
        }
    }

    fn live(&self, l: LiveCall) -> CallLiveView {
        let mut people: Vec<String> = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for (_, p) in &l.devices {
            if !seen.contains(&p.as_str()) {
                seen.push(p);
                people.push(self.person(p));
            }
        }
        CallLiveView {
            call_id: l.call_id,
            conv_id: l.conv_id,
            people,
        }
    }
}

fn person_key(id: &str) -> String {
    format!("{}{id}", store::kinds::PERSON_ACTOR_PREFIX)
}
