//! Sending and the recipient's and the person's actions on a delivery
//! (I:internal/inbox/messages.go).

use gocompat::quote::quote;
use gocompat::time::GoTime;

use crate::policy::placed;
use crate::state::{AliasRow, DeliveryRow, Drec, Inner, norm_alias, pair_key, trim};
use crate::{
    Address, DEFAULT_INBOX_LIST, DIRECT_LIMIT, DIRECT_WINDOW, Delivery, Error, ErrorKind, Inbox,
    MAX_ATTACHMENTS, MAX_DEFER, MAX_NOTE, MAX_PERSON_TEXT, MAX_REF, MAX_TEXT, Mention, Message,
    REASON_MENTIONED, Result, SendOptions, Sender, addr, choice, kinds, mode, path, st, tm, trust,
};

/// Trims, then cuts to n runes (no ellipsis).
pub(crate) fn clip(s: &str, n: usize) -> String {
    let s = trim(s);
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n).collect()
}

/// Cuts s to at most n characters for an agent's wake, ending it with a
/// pointer to `show` for all of it (card #265). Text within n is returned
/// trimmed and whole; with no ref it is cut plain.
pub fn clip_show(s: &str, n: usize, ref_: &str) -> String {
    let s = trim(s);
    if s.chars().count() <= n {
        return s.to_string();
    }
    let ref_ = trim(ref_);
    if ref_.is_empty() {
        return s.chars().take(n).collect();
    }
    let tail = format!("\n… (clipped; show {ref_} for all of it)");
    let keep = n.saturating_sub(tail.chars().count());
    let mut out: String = s.chars().take(keep).collect();
    out.push_str(&tail);
    out
}

pub(crate) fn bad_state(d: &Drec, what: &str) -> Error {
    Error::wrap(
        ErrorKind::BadState,
        format!("can't {what} a {} delivery", d.d.state),
    )
}

pub(crate) fn not_found_delivery(id: &str) -> Error {
    Error::wrap(ErrorKind::NotFound, format!("delivery {}", quote(id)))
}

impl Inner {
    /// Validates, routes and records one message; `o.mode` is resolved.
    /// `reply_of` is the message Reply answers.
    pub(crate) fn send_locked(
        &mut self,
        from: &Sender,
        to: &Address,
        text: &str,
        o: &SendOptions,
        reply_of: Option<&Message>,
    ) -> Result<(Message, Delivery)> {
        if trim(&from.key).is_empty() {
            return Err(Error::other("inbox: no sender; join first"));
        }
        let text = trim(text);
        // Files alone make a message too: a Talk DM of a pasted
        // screenshot goes to the agent as one (card #643).
        if text.is_empty() && o.attachments.is_empty() {
            return Err(Error::sentinel(ErrorKind::Empty));
        }
        let n = text.chars().count();
        if n > MAX_TEXT && self.is_owner(&from.key) {
            // The person's own message is stored whole and clipped for the
            // agent's wake (fence) with a show pointer.
            if text.len() > MAX_PERSON_TEXT {
                return Err(Error::wrap(
                    ErrorKind::TooLong,
                    format!("{} bytes; at most {MAX_PERSON_TEXT}", text.len()),
                ));
            }
        } else if n > MAX_TEXT {
            return Err(Error::wrap(
                ErrorKind::TooLong,
                format!("{n} characters; at most {MAX_TEXT}"),
            ));
        }
        let now = self.now();
        let cards = self.live_cards()?;

        let recipient: String;
        let (mut repo, mut reply_to, mut for_owner) = (String::new(), String::new(), String::new());
        let mut a = Address {
            kind: to.kind.clone(),
            value: trim(&to.value).to_string(),
        };
        match to.kind.as_str() {
            addr::AGENT => recipient = self.resolve_agent(&from.key, &a.value, &cards)?,
            addr::REPO => {
                if a.value.is_empty() {
                    return Err(Error::other("inbox: say which repo"));
                }
                repo = a.value.clone();
                recipient = self.desk(&repo, &cards, &now, &[&from.key]);
            }
            addr::REPLY => {
                let Some(orig) = self.msgs.get(&a.value) else {
                    return Err(Error::wrap(
                        ErrorKind::NotFound,
                        format!("message {}", quote(&a.value)),
                    ));
                };
                if orig.from.key == from.key {
                    return Err(Error::sentinel(ErrorKind::SelfAddress));
                }
                if orig.from.is_system() {
                    return Err(Error::wrap(
                        ErrorKind::BadState,
                        "that is Irori's own notice; it needs no reply",
                    ));
                }
                recipient = orig.from.key.clone();
                reply_to = orig.id.clone();
            }
            addr::PR => {
                let r = self.route_pr(&from.key, &a.value, &cards, &now)?;
                a.value = r.key;
                recipient = r.to;
                repo = r.repo;
                for_owner = r.for_owner;
            }
            other => {
                return Err(Error::other(format!(
                    "inbox: unknown address kind {}",
                    quote(other)
                )));
            }
        }

        // Rate limit per unordered pair, both directions, sliding window.
        if !self.is_person(from) && self.counts_for_rate(&from.key, &recipient) {
            let pk = pair_key(&from.key, &recipient);
            let win = tm::d(DIRECT_WINDOW);
            let kept = self.pairs.entry(pk).or_default();
            kept.retain(|t| tm::sub(&now, t) < win);
            if kept.len() >= DIRECT_LIMIT {
                let n = kept.len();
                return Err(Error::wrap(
                    ErrorKind::RateLimited,
                    format!(
                        "{n} messages between you and {} in the last {} minutes; stop the back-and-forth or ask the person",
                        self.describe(&recipient, &cards),
                        DIRECT_WINDOW.as_secs() / 60
                    ),
                ));
            }
        }

        let mut m = Message {
            id: format!("msg_{}", self.ids.next(&now)),
            from: from.clone(),
            to: a,
            text: text.to_string(),
            reason: clip(&o.reason, MAX_NOTE),
            reply_to,
            trust: trust::UNTRUSTED.into(),
            sent_at: now,
            ref_: clip(&o.ref_, MAX_REF),
            notice: o.notice,
            stop: o.mode == mode::STOP,
            attachments: o.attachments.clone(),
            mentions: o.mentions.clone(),
            ..Message::default()
        };
        if !o.parent.is_empty()
            && let Some(p) = self.msgs.get(&o.parent)
        {
            m.parent_id = p.id.clone();
            m.hop = p.hop + 1;
        }
        if self.is_person(from) {
            m.trust = trust::USER.into();
        }
        // Hops: a message sent from inside a woken turn continues its chain.
        if m.parent_id.is_empty()
            && let Some(run) = self.running.get(&from.key)
            && let Some(parent) = self.newest_in(&run.wake_id)
        {
            m.parent_id = parent.id.clone();
            m.hop = parent.hop + 1;
        }
        if m.parent_id.is_empty()
            && let Some(r) = reply_of
        {
            m.parent_id = r.id.clone();
            m.hop = r.hop + 1;
        }
        // Card #488: once either agent made progress since the message
        // this one follows (a step ticked, a card's state set, a commit),
        // the chain is getting somewhere and its hop count starts again.
        if m.hop > 0
            && let Some(p) = self.msgs.get(&m.parent_id)
            && self.chain_progressed(&[&from.key, &recipient, &p.from.key], &p.sent_at)
        {
            m.hop = 0;
        }

        // The sender becomes addressable by the name it sent under.
        let al = norm_alias(&from.name);
        if !al.is_empty() && !self.is_person(from) && !from.is_system() {
            let last = self.aliases.get(&from.key).and_then(|l| l.last());
            if last != Some(&al) {
                let row = AliasRow {
                    key: from.key.clone(),
                    alias: al,
                    at: now,
                };
                self.emit(&from.key, kinds::AGENT_ALIAS, &row)?;
            }
        }
        self.emit(&from.key, kinds::MESSAGE_SENT, &m)?;
        // A recipient on another device gets a queued row with no hold
        // decision: its host decides (v0.4 §6.3).
        let mut h = "";
        if self.local(&recipient) || recipient.is_empty() {
            h = self.hold_id(&m.id, &recipient, &cards); // as folded: it knows its writer
        }
        let r = placed(
            DeliveryRow {
                delivery_id: format!("dlv_{}", self.ids.next(&now)),
                message_id: m.id.clone(),
                to: recipient,
                repo_key: repo,
                mode: o.mode.clone(),
                for_: for_owner,
                ..DeliveryRow::default()
            },
            h,
        );
        let did = r.delivery_id.clone();
        self.put_delivery(r, &from.key)?;
        Ok((self.msgs[&m.id].clone(), self.copy_id(&did)))
    }

    /// The newest message among a wake's deliveries.
    pub(crate) fn newest_in(&self, wake_id: &str) -> Option<&Message> {
        let w = self.wakes.get(wake_id)?;
        let mut best: Option<&Message> = None;
        for id in &w.w.delivery_ids {
            let Some(d) = self.dels.get(id) else {
                continue;
            };
            if let Some(m) = self.msgs.get(&d.d.message_id)
                && best.is_none_or(|b| m.id > b.id)
            {
                best = Some(m);
            }
        }
        best
    }

    /// A delivery addressed to agent.
    pub(crate) fn mine(&self, agent: &str, did: &str) -> Result<&Drec> {
        let Some(d) = self.dels.get(did) else {
            return Err(not_found_delivery(did));
        };
        if d.d.to != agent || agent.is_empty() {
            return Err(Error::wrap(ErrorKind::NotYours, did));
        }
        Ok(d)
    }

    /// Finishes r for a delivery going Engaged: a notice closes at once.
    pub(crate) fn engaged(&self, msg_id: &str, mut r: DeliveryRow) -> DeliveryRow {
        r.engaged_once = true;
        if self.msgs.get(msg_id).is_some_and(|m| m.notice) {
            r.state = st::SKIPPED.into();
            r.note = "notice".into();
            r.until = GoTime::ZERO;
        }
        r
    }

    /// Queues held or dismissed d past the policy.
    pub(crate) fn release_locked(&mut self, did: &str, by: &str) -> Result<()> {
        let d = &self.dels[did];
        let mut r = d.row();
        if r.to.is_empty() {
            let cards = self.live_cards()?;
            let d = &self.dels[did];
            let sender = self
                .msgs
                .get(&d.d.message_id)
                .map(|m| m.from.key.clone())
                .unwrap_or_default();
            r.to = self.desk(&d.d.repo_key, &cards, &self.now(), &[&sender]);
            if r.to.is_empty() {
                return Err(Error::wrap(
                    ErrorKind::BadState,
                    "no agent in that repo can take it yet",
                ));
            }
        }
        r.state = st::QUEUED.into();
        r.hold.clear();
        r.wake_id.clear();
        self.put_delivery(r, by)
    }

    fn newest_first(&self, keep: impl Fn(&Drec) -> bool, limit: usize) -> Vec<Delivery> {
        let limit = if limit == 0 {
            DEFAULT_INBOX_LIST
        } else {
            limit
        };
        let mut out = vec![];
        for id in self.order.iter().rev() {
            if out.len() >= limit {
                break;
            }
            let d = &self.dels[id];
            if keep(d) {
                out.push(self.copy_out(d));
            }
        }
        out
    }
}

/// A delivery that hasn't reached its agent yet (card #787): queued or
/// held. Once delivered, the agent has it in hand.
pub(crate) fn not_reached(state: &str) -> bool {
    state == st::QUEUED || state == st::HELD
}

/// A wind-down the person turned off before it reached its agent.
pub(crate) fn wind_down_off(d: &Drec) -> DeliveryRow {
    let mut r = d.row();
    r.state = st::SKIPPED.into();
    r.note = crate::NOTE_WIND_DOWN_OFF.into();
    r.until = GoTime::ZERO;
    r.wake_id.clear();
    r
}

pub(crate) fn dismissed(d: &Drec) -> DeliveryRow {
    let mut r = d.row();
    r.state = st::DISMISSED.into();
    r
}

impl Inbox {
    /// `send_with(…, {reason, mode: wake})`.
    pub fn send(
        &self,
        from: &Sender,
        to: &Address,
        text: &str,
        reason: &str,
    ) -> Result<(Message, Delivery)> {
        self.send_with(
            from,
            to,
            text,
            SendOptions {
                reason: reason.into(),
                mode: mode::WAKE.into(),
                ..SendOptions::default()
            },
        )
    }

    /// Routes and records a message and its delivery. Policy, rate and hop
    /// limits apply; a held delivery is still a successful send.
    pub fn send_with(
        &self,
        from: &Sender,
        to: &Address,
        text: &str,
        mut o: SendOptions,
    ) -> Result<(Message, Delivery)> {
        self.mutate(|b| {
            match o.mode.as_str() {
                "" => {
                    o.mode = if b.is_person(from) || from.is_system() {
                        mode::NEXT_TURN.into()
                    } else {
                        mode::WAKE.into()
                    };
                }
                mode::WAKE | mode::NEXT_TURN => {}
                mode::STOP => {
                    if !b.is_person(from) {
                        return Err(Error::other(
                            "inbox: only the person may stop an agent (mode stop)",
                        ));
                    }
                }
                other => {
                    return Err(Error::other(format!(
                        "inbox: unknown delivery mode {} (wake, next_turn or stop)",
                        quote(other)
                    )));
                }
            }
            if o.reason == crate::REASON_WIND_DOWN && !b.is_person(from) {
                return Err(Error::other(
                    "inbox: only the person may wind an agent down (reason wind_down)",
                ));
            }
            if o.attachments.len() > MAX_ATTACHMENTS {
                return Err(Error::other(format!(
                    "inbox: at most {MAX_ATTACHMENTS} attachments on a message"
                )));
            }
            b.send_locked(from, to, text, &o, None)
        })
    }

    /// Answers a delivery addressed to from: marks it Answered and sends a
    /// reply back to its sender.
    pub fn reply(&self, from: &Sender, did: &str, text: &str) -> Result<(Message, Delivery)> {
        self.reply_with(from, did, text, &[])
    }

    /// [`Inbox::reply`] with the agents the reply mentions on its row.
    pub fn reply_with(
        &self,
        from: &Sender,
        did: &str,
        text: &str,
        mentions: &[Mention],
    ) -> Result<(Message, Delivery)> {
        self.reply_files(from, did, text, mentions, &[])
    }

    /// [`Inbox::reply_with`] with files on the reply (card #195: the
    /// person's Reply in a message takes pasted, dropped or picked files).
    pub fn reply_files(
        &self,
        from: &Sender,
        did: &str,
        text: &str,
        mentions: &[Mention],
        files: &[crate::FileRef],
    ) -> Result<(Message, Delivery)> {
        self.reply_files_mode(from, did, text, mentions, files, mode::WAKE)
    }

    /// [`Inbox::reply_files`] with the reply's mode: `wake`, or the person's
    /// `stop` (card #207), which cuts the sender's running turn.
    pub fn reply_files_mode(
        &self,
        from: &Sender,
        did: &str,
        text: &str,
        mentions: &[Mention],
        files: &[crate::FileRef],
        md: &str,
    ) -> Result<(Message, Delivery)> {
        if md != mode::WAKE && md != mode::STOP {
            return Err(Error::other(format!(
                "inbox: a reply wakes or stops, not {}",
                quote(md)
            )));
        }
        self.mutate(|b| {
            let d = b.mine(&from.key, did)?;
            match d.d.state.as_str() {
                st::QUEUED | st::DELIVERED | st::ENGAGED | st::DEFERRED => {}
                _ => return Err(bad_state(d, "reply to")),
            }
            let Some(orig) = b.msgs.get(&d.d.message_id).cloned() else {
                return Err(Error::wrap(
                    ErrorKind::NotFound,
                    format!("message {}", quote(&d.d.message_id)),
                ));
            };
            if md == mode::STOP && !b.is_person(from) {
                return Err(Error::other(
                    "inbox: only the person may stop an agent (mode stop)",
                ));
            }
            // A reply to the person's wind-down (card #307) is for the Wind
            // down dialog only: it answers the delivery and keeps its text
            // there, and sends nothing back, so the person gets no message,
            // notification or unread count per agent. It returns the
            // wind-down itself and the answered delivery.
            if crate::fence::is_wind_down(&orig) {
                let mut r = b.dels[did].row();
                r.state = st::ANSWERED.into();
                r.until = GoTime::ZERO;
                r.note = clip(text, MAX_NOTE);
                b.put_delivery(r, &from.key)?;
                let d = b.copy_out(&b.dels[did]);
                return Ok((orig, d));
            }
            let o = SendOptions {
                mode: md.into(),
                ref_: orig.ref_.clone(),
                mentions: mentions.to_vec(),
                attachments: files.to_vec(),
                ..SendOptions::default()
            };
            let (m, nd) = b.send_locked(
                from,
                &Address::new(addr::REPLY, &orig.id),
                text,
                &o,
                Some(&orig),
            )?;
            let mut r = b.dels[did].row();
            r.state = st::ANSWERED.into();
            r.until = GoTime::ZERO;
            b.put_delivery(r, &from.key)?;
            Ok((m, nd))
        })
    }

    /// Gives each agent `orig` mentions, other than its recipient `to`
    /// (named `to_name`) and the sender, its own copy of the message
    /// (board.md "Direct messages"): reason `mentioned`, woken, the same
    /// ref, `parentId` the original. A gone mention gets nothing. Each
    /// result pairs the mention with its delivery or its error; copies to
    /// `to` or the sender are left out.
    pub fn send_mention_copies(
        &self,
        from: &Sender,
        orig: &Message,
        to: &str,
        to_name: &str,
    ) -> Vec<(Mention, Result<Delivery>)> {
        let who = if from.is_user() {
            "The person".to_string()
        } else if from.name.is_empty() {
            from.key.clone()
        } else {
            from.name.clone()
        };
        let to_name = if to_name.is_empty() { to } else { to_name };
        let mut out = Vec::new();
        for m in &orig.mentions {
            if m.key.is_empty() || m.key == to || m.key == from.key {
                continue;
            }
            if m.gone {
                out.push((
                    m.clone(),
                    Err(Error::wrap(
                        ErrorKind::BadState,
                        format!("{} is gone", m.name),
                    )),
                ));
                continue;
            }
            let text = format!(
                "{who} mentioned you in a message to {to_name}:
{}",
                orig.text
            );
            let r = self
                .send_with(
                    from,
                    &Address::new(addr::AGENT, &m.key),
                    &text,
                    SendOptions {
                        reason: REASON_MENTIONED.into(),
                        mode: mode::WAKE.into(),
                        ref_: orig.ref_.clone(),
                        parent: orig.id.clone(),
                        ..SendOptions::default()
                    },
                )
                .map(|(_, d)| d);
            out.push((m.clone(), r));
        }
        out
    }

    /// A channel-path recipient has the delivery (Delivered then Engaged).
    pub fn ack(&self, agent: &str, did: &str) -> Result<()> {
        self.mutate(|b| {
            let d = b.mine(agent, did)?;
            match d.d.state.as_str() {
                st::ENGAGED | st::ANSWERED => return Ok(()),
                st::QUEUED => {
                    let mut r = d.row();
                    r.state = st::DELIVERED.into();
                    r.via = path::CHANNEL.into();
                    b.put_delivery(r, agent)?;
                }
                st::DELIVERED => {}
                _ => return Err(bad_state(d, "ack")),
            }
            let d = &b.dels[did];
            let mut r = d.row();
            r.state = st::ENGAGED.into();
            if r.via.is_empty() {
                r.via = path::CHANNEL.into();
            }
            let r = b.engaged(&d.d.message_id.clone(), r);
            b.put_delivery(r, agent)
        })
    }

    /// agent read a queued delivery itself: Queued becomes Delivered.
    pub fn mark_read(&self, agent: &str, did: &str) -> Result<()> {
        self.mutate(|b| {
            let d = b.mine(agent, did)?;
            if d.d.state != st::QUEUED {
                return Ok(());
            }
            let mut r = d.row();
            r.state = st::DELIVERED.into();
            r.via = path::CHANNEL.into();
            b.put_delivery(r, agent)
        })
    }

    /// Postpones a delivery addressed to agent, clamped to MaxDefer.
    pub fn defer(&self, agent: &str, did: &str, minutes: i64, note: &str) -> Result<()> {
        if minutes <= 0 {
            return Err(Error::other("inbox: defer by a positive number of minutes"));
        }
        self.mutate(|b| {
            let d = b.mine(agent, did)?;
            match d.d.state.as_str() {
                st::QUEUED | st::DELIVERED | st::ENGAGED | st::DEFERRED => {}
                _ => return Err(bad_state(d, "defer")),
            }
            let mins = minutes.min((MAX_DEFER.as_secs() / 60) as i64);
            let mut r = d.row();
            r.state = st::DEFERRED.into();
            r.until = tm::add(&b.now(), i128::from(mins) * 60_000_000_000);
            r.note = clip(note, MAX_NOTE);
            r.wake_id.clear();
            r.engaged_once = true;
            r.reminded = false; // the new time earns its own reminder
            b.put_delivery(r, agent)
        })
    }

    /// Closes a delivery addressed to agent without a reply.
    pub fn skip(&self, agent: &str, did: &str, note: &str) -> Result<()> {
        self.mutate(|b| {
            let d = b.mine(agent, did)?;
            match d.d.state.as_str() {
                st::SKIPPED => return Ok(()),
                st::QUEUED | st::DELIVERED | st::ENGAGED | st::DEFERRED => {}
                _ => return Err(bad_state(d, "skip")),
            }
            let mut r = d.row();
            r.state = st::SKIPPED.into();
            r.note = clip(note, MAX_NOTE);
            r.until = GoTime::ZERO;
            b.put_delivery(r, agent)
        })
    }

    /// The person's Deliver on a held (or dismissed) delivery.
    pub fn release(&self, did: &str, by: &str) -> Result<()> {
        let r = self.mutate(|b| {
            let Some(d) = b.dels.get(did) else {
                return Err(not_found_delivery(did));
            };
            if d.d.state != st::HELD && d.d.state != st::DISMISSED {
                return Err(bad_state(d, "release"));
            }
            if !b.owns(d) {
                return b.request(did, choice::RELEASE, "", by);
            }
            b.release_locked(did, by)
        });
        self.fill_cards();
        r
    }

    /// The person's Dismiss on a held delivery.
    pub fn dismiss(&self, did: &str, by: &str) -> Result<()> {
        self.mutate(|b| {
            let Some(d) = b.dels.get(did) else {
                return Err(not_found_delivery(did));
            };
            if d.d.state == st::DISMISSED {
                return Ok(());
            }
            if d.d.state != st::HELD {
                return Err(bad_state(d, "dismiss"));
            }
            if !b.owns(d) {
                return b.request(did, choice::DISMISS, "", by);
            }
            let r = dismissed(d);
            b.put_delivery(r, by)
        })
    }

    /// Wind down turned off (card #787): closes the person's wind-down
    /// `did` if it hasn't reached its agent yet (queued or held), as
    /// skipped with [`crate::NOTE_WIND_DOWN_OFF`]. Another device's goes
    /// to its owner as a request. True when it closed or was asked to; a
    /// delivered one is left alone (its agent is winding down already).
    pub fn cancel_wind_down(&self, did: &str, by: &str) -> Result<bool> {
        self.mutate(|b| {
            let Some(d) = b.dels.get(did) else {
                return Err(not_found_delivery(did));
            };
            if !not_reached(&d.d.state) || !b.wind_down(d) {
                return Ok(false);
            }
            if !b.owns(d) {
                b.request(did, choice::CANCEL, "", by)?;
                return Ok(true);
            }
            let r = wind_down_off(d);
            b.put_delivery(r, by)?;
            Ok(true)
        })
    }

    /// The person's wind-downs still open, every device's (card #787).
    pub fn open_wind_downs(&self) -> Vec<Delivery> {
        self.read(|b| b.newest_first(|d| st::open(&d.d.state) && b.wind_down(d), usize::MAX))
    }

    /// The recipient's own view: open deliveries (or all recent), never
    /// held or dismissed ones.
    pub fn for_agent(&self, agent: &str, all: bool, limit: usize) -> Vec<Delivery> {
        self.read(|b| {
            b.newest_first(
                |d| {
                    if d.d.to != agent || d.d.state == st::HELD || d.d.state == st::DISMISSED {
                        return false;
                    }
                    all || st::open(&d.d.state)
                },
                limit,
            )
        })
    }

    /// Recent deliveries across every recipient, newest first.
    pub fn list(&self, limit: usize) -> Vec<Delivery> {
        self.read(|b| b.newest_first(|_| true, limit))
    }

    /// Deliveries a hold once kept that are no longer held, the most
    /// recently decided first.
    pub fn held_history(&self, limit: usize) -> Vec<Delivery> {
        let limit = if limit == 0 {
            DEFAULT_INBOX_LIST
        } else {
            limit
        };
        let mut out: Vec<Delivery> = self.read(|b| {
            b.order
                .iter()
                .map(|id| &b.dels[id])
                .filter(|d| !d.d.held_for.is_empty() && d.d.state != st::HELD)
                .map(|d| b.copy_out(d))
                .collect()
        });
        let decided = |d: &Delivery| {
            if d.decided_at.is_zero() {
                d.updated_at
            } else {
                d.decided_at
            }
        };
        out.sort_by(|x, y| tm::cmp(&decided(y), &decided(x)));
        out.truncate(limit);
        out
    }

    /// Every delivery of message id, oldest first.
    pub fn of_message(&self, id: &str) -> Vec<Delivery> {
        self.read(|b| {
            b.order
                .iter()
                .map(|did| &b.dels[did])
                .filter(|d| d.d.message_id == id)
                .map(|d| b.copy_out(d))
                .collect()
        })
    }

    /// The newest `limit` deliveries of notices about a comment (ref
    /// `comment:<id>`, reason `reasons`), each with whether its recipient
    /// read it: took it up in a turn, answered or skipped it itself
    /// (card #159). A dismissed, moved or rejected one is left out; one
    /// Irori closed unread (gone, stale) stays, unread.
    pub fn comment_notices(&self, reasons: &[&str], limit: usize) -> Vec<(Delivery, bool)> {
        self.ref_notices("comment:", reasons, limit)
    }

    /// As [`Self::comment_notices`], for notices whose ref starts with
    /// `prefix` (card #476: a blocker answer's notice is about its card,
    /// `task:<id>`).
    pub fn ref_notices(
        &self,
        prefix: &str,
        reasons: &[&str],
        limit: usize,
    ) -> Vec<(Delivery, bool)> {
        self.read(|b| {
            let mut out = Vec::new();
            for id in b.order.iter().rev() {
                if out.len() >= limit {
                    break;
                }
                let d = &b.dels[id];
                if matches!(d.d.state.as_str(), st::DISMISSED | st::MOVED | st::REJECTED) {
                    continue;
                }
                let Some(m) = b.msgs.get(&d.d.message_id) else {
                    continue;
                };
                if !m.notice || !m.ref_.starts_with(prefix) || !reasons.contains(&m.reason.as_str())
                {
                    continue;
                }
                let read = d.engaged_once
                    || d.self_closed
                    || d.d.state == st::ENGAGED
                    || d.d.state == st::ANSWERED;
                out.push((b.copy_out(d), read));
            }
            out
        })
    }

    /// Every open delivery, held ones included (in id order).
    pub fn open(&self) -> Vec<Delivery> {
        self.read(|b| b.open_recs().map(|d| b.copy_out(d)).collect())
    }

    /// One delivery.
    pub fn get(&self, did: &str) -> Option<Delivery> {
        self.read(|b| b.dels.get(did).map(|d| b.copy_out(d)))
    }

    /// Whether s is the person at a GUI (an owner's user key).
    pub fn is_person(&self, s: &Sender) -> bool {
        self.read(|b| b.is_person(s))
    }

    /// One message.
    pub fn message(&self, id: &str) -> Option<Message> {
        self.read(|b| b.msgs.get(id).cloned())
    }

    /// The names agent is addressable by besides its card name, newest last.
    pub fn aliases(&self, agent: &str) -> Vec<String> {
        self.read(|b| b.aliases.get(agent).cloned().unwrap_or_default())
    }
}
