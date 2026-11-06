//! Calls on the Core (#654, docs/research/08-voice-calls/irori-fit.md): a
//! huddle's start and end posts in its conversation.

use coord::talk::{CallMark, NewPost, Post};

use super::{Core, Services};

impl Services {
    pub(crate) fn call_snapshot(&self) -> CallSnapshot {
        CallSnapshot::default()
    }

    pub(crate) fn call_start(&self, _conv: &str) -> Result<String, String> {
        Err("calls aren't wired yet".into())
    }

    pub(crate) fn call_join(&self, _call_id: &str) -> Result<(), String> {
        Err("calls aren't wired yet".into())
    }

    pub(crate) fn call_leave(&self) -> Result<(), String> {
        Ok(())
    }

    pub(crate) fn call_mute(&self, _on: bool) {}

    pub(crate) fn call_dismiss(&self, _call_id: &str) {}

    /// Writes huddle mark `mark` in channel or person DM `conv` as the
    /// person, with `text` as what an older reader shows. It goes straight
    /// to Talk, not through `tools::talk::post`: nobody is mentioned and no
    /// agent in the channel is told at its next turn. A thread and a DM
    /// with an agent have no huddle. A person DM must have its space
    /// already: a mark never founds one.
    pub(crate) fn post_call_mark(
        &self,
        conv: &str,
        text: &str,
        mark: CallMark,
    ) -> Result<Post, String> {
        let conv = conv.trim();
        if coord::talk::is_dm(conv) {
            let Some(other) = self.talk.pair_other(conv) else {
                return Err("a DM with an agent has no huddle".into());
            };
            let id = other
                .strip_prefix(store::kinds::PERSON_ACTOR_PREFIX)
                .unwrap_or_default();
            if id.is_empty() || self.dm_writer(id).is_none() {
                return Err("this DM hasn't started yet: write in it first".into());
            }
        } else if !coord::talk::is_channel(conv) {
            return Err(format!(
                "{conv} isn't a channel or a DM: a thread has no huddle"
            ));
        }
        let p = self
            .talk
            .post(NewPost {
                conv_id: conv.to_string(),
                text: text.to_string(),
                by: self.user_sender().key,
                call: Some(mark),
                ..NewPost::default()
            })
            .map_err(|e| e.to_string())?;
        self.hub.publish(crate::events::CoreEvent::Talk);
        Ok(p)
    }
}

/// What the window shows about calls (card #654).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallSnapshot {
    /// The call this device is in.
    pub mine: Option<MyCall>,
    /// Calls this device is rung for and hasn't joined or dismissed.
    pub ringing: Vec<Ringing>,
    /// Calls on now in conversations this device can see, from the frames
    /// it heard (the list rows' "huddle · 3").
    pub live: Vec<LiveCall>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MyCall {
    pub call_id: String,
    pub conv_id: String,
    /// The space its rows and connections belong to (a channel's space, or
    /// a person DM's hidden space).
    pub space_id: String,
    /// When this device joined, Unix ms.
    pub joined_at_ms: i64,
    pub muted: bool,
    /// This device's mic is above the speech gate.
    pub speaking: bool,
    /// The other devices in the call.
    pub peers: Vec<CallPeer>,
    /// Why the call isn't working, in plain words (the mic, the network).
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallPeer {
    pub device_id: String,
    pub person_id: String,
    /// "connecting", "live" or "gone".
    pub phase: String,
    /// "direct", "relay" or "" (not connected).
    pub path: String,
    pub rtt_ms: f64,
    /// Datagrams lost on the way here, percent.
    pub loss_pct: f64,
    pub speaking: bool,
    pub muted: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ringing {
    pub call_id: String,
    pub conv_id: String,
    pub space_id: String,
    pub by_device: String,
    pub by_person: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveCall {
    pub call_id: String,
    pub conv_id: String,
    pub space_id: String,
    /// (device id, person id) of who is in it.
    pub devices: Vec<(String, String)>,
}

impl Core {
    /// The calls on now, as the window shows them.
    pub fn call_snapshot(&self) -> CallSnapshot {
        self.s.call_snapshot()
    }

    /// Starts a huddle in channel or person DM `conv`: opens the mic and
    /// speaker, writes the start post and rings the conversation's devices
    /// that are online. Blocks while the devices open. Returns the call id.
    pub fn call_start(&self, conv: &str) -> Result<String, String> {
        self.s.call_start(conv)
    }

    /// Joins a call this device was rung for or sees live.
    pub fn call_join(&self, call_id: &str) -> Result<(), String> {
        self.s.call_join(call_id)
    }

    /// Leaves the call this device is in; the last one out writes the end
    /// post.
    pub fn call_leave(&self) -> Result<(), String> {
        self.s.call_leave()
    }

    pub fn call_mute(&self, on: bool) {
        self.s.call_mute(on)
    }

    /// Stops ringing for `call_id` here.
    pub fn call_dismiss(&self, call_id: &str) {
        self.s.call_dismiss(call_id)
    }

    /// On quit: leave any call and close the devices.
    pub fn call_shutdown(&self) {
        let _ = self.s.call_leave();
    }

    /// [`Services::post_call_mark`] (tests).
    #[doc(hidden)]
    pub fn post_call_mark(&self, conv: &str, text: &str, mark: CallMark) -> Result<Post, String> {
        self.s.post_call_mark(conv, text, mark)
    }
}
