//! Calls on the Core (#654, docs/research/08-voice-calls/irori-fit.md): a
//! huddle's start and end posts in its conversation.

use coord::talk::{CallMark, NewPost, Post};

use super::{Core, Services};

impl Services {
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

impl Core {
    /// [`Services::post_call_mark`] (tests).
    #[doc(hidden)]
    pub fn post_call_mark(&self, conv: &str, text: &str, mark: CallMark) -> Result<Post, String> {
        self.s.post_call_mark(conv, text, mark)
    }
}
