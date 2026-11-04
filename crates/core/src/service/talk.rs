//! Talk on the Core (card #389; docs/design/talk-channels.md): the
//! window's calls over [`coord::talk`] and the tools' shared views
//! ([`tools::talk`]), so the person and agents see one thing.

use std::sync::Arc;

use coord::talk::{Channel, Member, Post, Talk};
use inbox::Sender;
use tools::talk::{Conversation, Posted, Target};

use super::{Core, ToolsView};

impl Core {
    /// Talk's fold.
    pub fn talk(&self) -> &Arc<Talk> {
        &self.s.talk
    }

    fn talk_view(&self) -> ToolsView {
        ToolsView(self.s.clone())
    }

    /// `reader`'s conversations, newest first ([`tools::talk::conversations`]).
    pub fn talk_conversations(&self, reader: &str) -> Result<Vec<Conversation>, String> {
        tools::talk::conversations(&self.talk_view(), reader)
    }

    /// Up to `limit` posts of `conv` before post `before` (`None`: the
    /// newest), oldest first, as `reader` may read them.
    pub fn talk_posts(
        &self,
        reader: &str,
        conv: &str,
        before: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Post>, String> {
        let all = tools::talk::posts(&self.talk_view(), reader, conv)?;
        Ok(coord::talk::page(&all, before, limit))
    }

    /// Posts to channel or DM `conv` as `from` ([`tools::talk::post`]); a
    /// thread's posts are its comments, so a thread id is refused here.
    pub fn talk_post(
        &self,
        from: &Sender,
        conv: &str,
        text: &str,
        mentions: &[String],
        agents_now: bool,
        thread_from: Option<String>,
    ) -> Result<Posted, String> {
        let conv = conv.trim();
        let target = if coord::talk::is_channel(conv) {
            Target::Channel(conv.to_string())
        } else if coord::talk::is_dm(conv) {
            Target::Dm(conv.to_string(), None)
        } else {
            return Err(format!(
                "{conv} isn't a channel or a DM: a thread's posts are its comments"
            ));
        };
        tools::talk::post(
            &self.talk_view(),
            from,
            &target,
            text,
            mentions,
            agents_now,
            thread_from,
        )
    }

    /// The DM between `a` and `b` (both joined), for a first post to it.
    pub fn talk_open_dm(&self, a: &Member, b: &Member, by: &str) -> Result<String, String> {
        self.s.talk.open_dm(a, b, by).map_err(|e| e.to_string())
    }

    /// Makes a Personal channel; `by` joins it.
    pub fn talk_channel_create(
        &self,
        by: &str,
        name: &str,
        topic: &str,
    ) -> Result<Channel, String> {
        self.talk_channel_create_in(by, name, topic, "")
    }

    /// Makes a channel in space `space` (empty: Personal); `by` joins it.
    /// A space's channel goes in its log and reaches its members (card
    /// #392), refused until every device in the space reads space rows.
    pub fn talk_channel_create_in(
        &self,
        by: &str,
        name: &str,
        topic: &str,
        space: &str,
    ) -> Result<Channel, String> {
        let space = space.trim();
        if !space.is_empty() && self.s.board(space).is_err() {
            return Err(format!("no space {space} on this device"));
        }
        self.s
            .talk
            .create_channel_in(by, name, topic, space)
            .map_err(|e| e.to_string())
    }

    /// Adds `m` to channel or DM `conv`; false when it was already in.
    pub fn talk_join(&self, conv: &str, m: &Member, by: &str) -> Result<bool, String> {
        self.s.talk.join(conv, m, by).map_err(|e| e.to_string())
    }

    /// Takes `m` out of channel or DM `conv`; false when it wasn't in.
    pub fn talk_leave(&self, conv: &str, m: &Member, by: &str) -> Result<bool, String> {
        self.s.talk.leave(conv, m, by).map_err(|e| e.to_string())
    }

    /// Links post `post_id` to the card made from it (card #390); false
    /// when it already was.
    pub fn talk_link_card(&self, post_id: &str, task_id: &str, by: &str) -> Result<bool, String> {
        self.s
            .talk
            .link_card(post_id, task_id, by)
            .map_err(|e| e.to_string())
    }

    /// `by` has read `conv` up to post `up_to`.
    pub fn talk_mark_read(&self, conv: &str, up_to: &str, by: &str) -> Result<(), String> {
        self.s
            .talk
            .mark_read(conv, up_to, by)
            .map_err(|e| e.to_string())
    }
}
