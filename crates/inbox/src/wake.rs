//! Wakes and appends (I:internal/inbox/wake.go:23-391): planning with
//! ride-along and the MaxWakeTries fallback, confirmations, turns.

use gocompat::quote::quote;
use gocompat::time::GoTime;

use crate::fence::{batch, fence, notes_section};
use crate::messages::clip;
use crate::state::{
    APPEND_TOKEN_PREFIX, Drec, Inner, REMINDER_TOKEN_PREFIX, TurnRow, WAKE_TOKEN_PREFIX, WakeRec,
    WakeRow, trim,
};
use crate::{Delivery, Error, ErrorKind, Inbox, MAX_NOTE, Result, Wake, kinds, mode, path, st};

/// Orders deliveries by message, then delivery id (oldest first).
fn sort_ready(ds: &mut [&Drec]) {
    ds.sort_by(|x, y| {
        x.d.message_id
            .cmp(&y.d.message_id)
            .then_with(|| x.d.id.cmp(&y.d.id))
    });
}

fn ids_of(ds: &[&Drec]) -> Vec<String> {
    ds.iter().map(|d| d.d.id.clone()).collect()
}

impl Inner {
    /// d is the person's wind-down (card #220): it goes out under Pause all.
    pub(crate) fn wind_down(&self, d: &Drec) -> bool {
        self.msgs
            .get(&d.d.message_id)
            .is_some_and(|m| m.reason == crate::REASON_WIND_DOWN && self.is_person(&m.from))
    }

    fn next_wake(&mut self, agent: &str, p: &str) -> Result<Option<Wake>> {
        // Wakes reach only sessions this device hosts (v0.4 §6.3).
        if !self.local(agent) {
            return Ok(None);
        }
        self.drop_stale(agent)?;
        // The person's stops (card #207) go first and alone: past Pause all
        // (the person paused) and past an outstanding wake's gate (the mod
        // cuts the running turn, then submits this).
        let mut stops: Vec<&Drec> = self
            .open_recs()
            .filter(|d| {
                d.d.to == agent
                    && d.d.state == st::QUEUED
                    && d.d.wake_id.is_empty()
                    && d.stop()
                    && !d.untaken()
            })
            .collect();
        if !stops.is_empty() {
            sort_ready(&mut stops);
            let ready = ids_of(&stops);
            let id = format!("wake_{}", self.ids.next(&self.now()));
            let token = format!("{WAKE_TOKEN_PREFIX}{id}]");
            return self
                .issue(agent, p, &id, &token, false, mode::STOP, &ready)
                .map(Some);
        }
        if self.outstanding(agent).is_some() {
            return Ok(None);
        }
        // Under Pause all only the person's wind-down (card #220) wakes.
        let paused = self.pol.paused;
        let (mut wake_items, mut ride): (Vec<&Drec>, Vec<&Drec>) = (vec![], vec![]);
        for d in self.open_recs() {
            if d.d.to != agent || d.d.state != st::QUEUED || !d.d.wake_id.is_empty() {
                continue;
            }
            if paused && !self.wind_down(d) {
                continue;
            }
            if d.next_turn() || (p != path::CHANNEL && d.untaken()) {
                ride.push(d);
            } else {
                wake_items.push(d);
            }
        }
        sort_ready(&mut wake_items);
        sort_ready(&mut ride);
        let (mut reminder, mut md) = (false, mode::WAKE);
        let ready: Vec<String> = if !wake_items.is_empty() {
            let mut v = ids_of(&wake_items);
            v.extend(ids_of(&ride));
            v
        } else if p == path::CHANNEL && !ride.is_empty() {
            md = mode::NEXT_TURN;
            ids_of(&ride)
        } else if paused {
            return Ok(None);
        } else {
            let mut due: Vec<&Drec> = vec![];
            for id in self.reminders.get(agent).cloned().unwrap_or_default() {
                if let Some(d) = self.dels.get(&id)
                    && d.d.to == agent
                    && st::open(&d.d.state)
                    && d.d.state != st::HELD
                    && !d.d.reminded
                {
                    due.push(d);
                }
            }
            if due.is_empty() {
                self.reminders.remove(agent);
                self.touch(&[agent]);
                return Ok(None);
            }
            sort_ready(&mut due);
            reminder = true;
            ids_of(&due)
        };
        let id = format!("wake_{}", self.ids.next(&self.now()));
        let token = if reminder {
            format!("{REMINDER_TOKEN_PREFIX}{id}]")
        } else {
            format!("{WAKE_TOKEN_PREFIX}{id}]")
        };
        self.issue(agent, p, &id, &token, reminder, md, &ready)
            .map(Some)
    }

    fn next_append(&mut self, agent: &str) -> Result<Option<Wake>> {
        if !self.local(agent) || self.pending_append(agent).is_some() {
            return Ok(None);
        }
        self.drop_stale(agent)?;
        // Under Pause all only the person's wind-down (card #220) goes.
        let paused = self.pol.paused;
        let behind = self.queued_behind(agent);
        let mut ready: Vec<&Drec> = vec![];
        for d in self.open_recs() {
            if paused && !self.wind_down(d) {
                continue;
            }
            if d.d.to == agent
                && d.d.state == st::QUEUED
                && d.d.wake_id.is_empty()
                && (behind || d.for_append())
                && (!d.stop() || d.untaken())
            {
                ready.push(d);
                if d.untaken() {
                    eprintln!(
                        "inbox: {}: {} wakes to {agent} went unconfirmed; sending it for the next turn instead",
                        d.d.id, d.wake_tries
                    );
                }
            }
        }
        if ready.is_empty() {
            return Ok(None);
        }
        sort_ready(&mut ready);
        let ready = ids_of(&ready);
        let id = format!("wake_{}", self.ids.next(&self.now()));
        let token = format!("{APPEND_TOKEN_PREFIX}{id}]");
        self.issue(
            agent,
            path::MOD,
            &id,
            &token,
            false,
            mode::NEXT_TURN,
            &ready,
        )
        .map(Some)
    }

    /// Closes as Skipped the queued and held notices to agent (every local
    /// recipient for `""`) that Options.Stale calls stale.
    pub(crate) fn drop_stale(&mut self, agent: &str) -> Result<()> {
        let Some(stale) = self.stale.clone() else {
            return Ok(());
        };
        let mut drop: Vec<&Drec> = vec![];
        for d in self.open_recs() {
            if (!agent.is_empty() && d.d.to != agent)
                || d.d.to.is_empty()
                || !self.local(&d.d.to)
                || !d.d.wake_id.is_empty()
                || (d.d.state != st::QUEUED && d.d.state != st::HELD)
            {
                continue;
            }
            match self.msgs.get(&d.d.message_id) {
                Some(m) if m.notice && !m.ref_.is_empty() => drop.push(d),
                _ => {}
            }
        }
        sort_ready(&mut drop);
        let drop = ids_of(&drop);
        for id in drop {
            let note = stale(&self.copy_id(&id));
            if note.is_empty() {
                continue;
            }
            let mut r = self.dels[&id].row();
            r.state = st::SKIPPED.into();
            r.note = clip(&note, MAX_NOTE);
            r.until = GoTime::ZERO;
            self.put_delivery(r, "irori")?;
        }
        Ok(())
    }

    /// Batches ready under token, records the wake.issued row, returns the
    /// wake.
    #[allow(clippy::too_many_arguments)]
    fn issue(
        &mut self,
        agent: &str,
        p: &str,
        id: &str,
        token: &str,
        reminder: bool,
        md: &str,
        ready: &[String],
    ) -> Result<Wake> {
        let msgs: Vec<Delivery> = ready.iter().map(|id| self.copy_id(id)).collect();
        let (full, overflow, mut prompt) = batch(token, reminder, &msgs);
        if md == mode::STOP {
            prompt = format!("{}\n\n{prompt}", stop_header(&msgs));
        }
        let notes = if md == mode::WAKE && !reminder && !token.starts_with(APPEND_TOKEN_PREFIX) {
            self.wake_notes(agent, &msgs)
        } else {
            crate::WakeNotes::default()
        };
        prompt.push_str(&notes_section(&notes));
        let mut ids: Vec<String> = full.iter().map(|d| d.id.clone()).collect();
        ids.extend(overflow.iter().cloned());
        let row = WakeRow {
            wake_id: id.into(),
            to: agent.into(),
            token: token.into(),
            reminder,
            delivery_ids: ids.clone(),
            path: p.into(),
            mode: if md == mode::NEXT_TURN || md == mode::STOP {
                md.into()
            } else {
                String::new()
            },
            overflow: overflow.clone(),
            at: self.now(),
            notes_task: notes.task.clone(),
            notes: notes.notes.clone(),
        };
        self.emit("irori", kinds::WAKE_ISSUED, &row)?;
        Ok(Wake {
            id: id.into(),
            to: agent.into(),
            token: token.into(),
            reminder,
            mode: md.into(),
            delivery_ids: ids,
            overflow,
            prompt,
            notes_task: notes.task,
            notes: notes.notes,
        })
    }

    /// The accepted notes for a wake to agent (card #412): Options.Notes'
    /// pick for the first card the messages are about, less the notes an
    /// earlier wake already gave this agent (as they read now), within
    /// [`crate::MAX_NOTES_BYTES`].
    fn wake_notes(&self, agent: &str, msgs: &[Delivery]) -> crate::WakeNotes {
        let Some(f) = self.notes.clone() else {
            return crate::WakeNotes::default();
        };
        let mut refs: Vec<String> = Vec::new();
        for d in msgs {
            let r = trim(&d.message.ref_);
            if r.starts_with("task:") && !refs.iter().any(|x| x == r) {
                refs.push(r.to_string());
            }
        }
        let mut out = f(agent, &refs);
        // Seen as given: the same id with the same text. An accepted edit
        // goes out again.
        let seen: std::collections::HashSet<(&str, &str)> = self
            .wakes
            .values()
            .filter(|w| w.w.to == agent)
            .flat_map(|w| w.w.notes.iter().map(|n| (n.id.as_str(), n.text.as_str())))
            .collect();
        out.notes
            .retain(|n| !seen.contains(&(n.id.as_str(), n.text.as_str())));
        // The section rides above batch's MAX_WAKE_BYTES by at most
        // MAX_NOTES_BYTES; a note left out goes in a later wake.
        while notes_section(&out).len() > crate::MAX_NOTES_BYTES {
            out.notes.pop();
        }
        if out.notes.is_empty() {
            out.task.clear();
        }
        out
    }

    /// The wake for token, checked to be agent's.
    fn wake_by_token(&self, agent: &str, token: &str) -> Result<&WakeRec> {
        let w = self
            .tokens
            .get(trim(token))
            .and_then(|id| self.wakes.get(id));
        let Some(w) = w else {
            return Err(Error::wrap(
                ErrorKind::NotFound,
                format!("wake token {}", quote(token)),
            ));
        };
        if w.w.to != agent {
            return Err(Error::wrap(ErrorKind::NotYours, format!("wake {}", w.w.id)));
        }
        Ok(w)
    }

    fn mark_delivered(&mut self, agent: &str, token: &str, appended: bool) -> Result<()> {
        let w = self.wake_by_token(agent, token)?;
        if w.w.reminder {
            return Ok(());
        }
        let appended = appended && !w.is_append();
        let (wid, wpath, ids) = (w.w.id.clone(), w.path.clone(), w.w.delivery_ids.clone());
        for id in ids {
            let Some(d) = self.dels.get(&id) else {
                continue;
            };
            if !d.d.wake_id.is_empty() && d.d.wake_id != wid {
                continue; // one already in another wake is left there
            }
            // A late confirmation after a reclaim still counts.
            if d.d.state != st::QUEUED && (!appended || d.d.state != st::DELIVERED || d.appended) {
                continue;
            }
            let mut r = d.row();
            r.state = st::DELIVERED.into();
            r.wake_id = wid.clone();
            r.via = wpath.clone();
            r.appended = appended;
            if r.via.is_empty() {
                r.via = path::MOD.into();
            }
            self.put_delivery(r, agent)?;
        }
        Ok(())
    }

    fn mark_engaged(
        &mut self,
        agent: &str,
        token: &str,
        turn_id: &str,
        appended: bool,
    ) -> Result<()> {
        let w = self.wake_by_token(agent, token)?;
        if w.w.reminder {
            return Ok(());
        }
        let (wid, wpath, ids, overflow, is_append) = (
            w.w.id.clone(),
            w.path.clone(),
            w.w.delivery_ids.clone(),
            w.w.overflow.clone(),
            w.is_append(),
        );
        for id in ids {
            let Some(d) = self.dels.get(&id) else {
                continue;
            };
            if (d.d.state != st::QUEUED && d.d.state != st::DELIVERED)
                || (!d.d.wake_id.is_empty() && d.d.wake_id != wid)
            {
                continue;
            }
            let mut r = d.row();
            r.state = st::ENGAGED.into();
            r.wake_id = wid.clone();
            if r.via.is_empty() {
                r.via = wpath.clone();
            }
            // An overflow id was only named: a notice among them stays open.
            if overflow.contains(&id) {
                r.engaged_once = true;
            } else {
                let mid = d.d.message_id.clone();
                r = self.engaged(&mid, r);
            }
            self.put_delivery(r, agent)?;
        }
        if is_append || appended {
            return Ok(());
        }
        if self
            .running
            .get(agent)
            .is_some_and(|cur| cur.wake_id == wid && cur.turn_id == turn_id)
        {
            return Ok(());
        }
        let row = TurnRow {
            agent: agent.into(),
            wake_id: wid,
            turn_id: turn_id.into(),
            at: self.now(),
        };
        self.emit(agent, kinds::TURN_ENGAGED, &row)
    }

    fn turn_ended(&mut self, agent: &str, token: &str, turn_id: &str) -> Result<()> {
        let w = self.wake_by_token(agent, token)?;
        if w.w.reminder || w.is_append() {
            return Ok(());
        }
        let wid = w.w.id.clone();
        if !self
            .running
            .get(agent)
            .is_some_and(|cur| cur.wake_id == wid)
        {
            return Ok(()); // not the running turn
        }
        let row = TurnRow {
            agent: agent.into(),
            wake_id: wid,
            turn_id: turn_id.into(),
            at: self.now(),
        };
        self.emit(agent, kinds::TURN_ENDED, &row)
    }

    /// w with its prompt built the way it first went out.
    pub(crate) fn rebuild(&self, w: &WakeRec) -> Wake {
        let msgs: Vec<Delivery> =
            w.w.delivery_ids
                .iter()
                .filter(|id| !w.w.overflow.contains(id))
                .filter_map(|id| self.dels.get(id).map(|d| self.copy_out(d)))
                .collect();
        let mut out = w.w.clone();
        out.prompt = fence(&w.w.token, w.w.reminder, &msgs, &w.w.overflow);
        out.prompt.push_str(&notes_section(&crate::WakeNotes {
            task: w.w.notes_task.clone(),
            notes: w.w.notes.clone(),
        }));
        out
    }
}

impl Inbox {
    /// The wake to submit to agent now, or `None` (nothing queued, or a
    /// previous wake still gates it). The deliveries are recorded as handed
    /// to it.
    pub fn next_wake(&self, agent: &str, p: &str) -> Result<Option<Wake>> {
        if agent.is_empty() {
            return Err(Error::other("inbox: no agent"));
        }
        self.mutate(|b| b.next_wake(agent, p))
    }

    /// The append to make to agent now, or `None`.
    pub fn next_append(&self, agent: &str) -> Result<Option<Wake>> {
        if agent.is_empty() {
            return Err(Error::other("inbox: no agent"));
        }
        self.mutate(|b| b.next_append(agent))
    }

    /// The append issued to agent and not yet confirmed, rebuilt.
    pub fn outstanding_append(&self, agent: &str) -> Option<Wake> {
        self.read(|b| b.pending_append(agent).map(|w| b.rebuild(w)))
    }

    /// The wake gating agent and where it stands (queued or delivered).
    pub fn outstanding_wake(&self, agent: &str) -> Option<(Wake, &'static str)> {
        self.read(|b| b.outstanding(agent).map(|(w, s)| (b.rebuild(w), s)))
    }

    /// The transport handed the wake (or append) over. Idempotent.
    pub fn mark_delivered(&self, agent: &str, token: &str) -> Result<()> {
        self.mutate(|b| b.mark_delivered(agent, token, false))
    }

    /// MarkDelivered for a wake the mod put in as an append (card #23).
    pub fn mark_appended_delivered(&self, agent: &str, token: &str) -> Result<()> {
        self.mutate(|b| b.mark_delivered(agent, token, true))
    }

    /// A turn whose text contains token started.
    pub fn mark_engaged(&self, agent: &str, token: &str, turn_id: &str) -> Result<()> {
        self.mutate(|b| b.mark_engaged(agent, token, turn_id, false))
    }

    /// MarkEngaged for a wake the mod put in as an append (card #23).
    pub fn mark_appended_engaged(&self, agent: &str, token: &str, turn_id: &str) -> Result<()> {
        self.mutate(|b| b.mark_engaged(agent, token, turn_id, true))
    }

    /// That turn finished.
    pub fn turn_ended(&self, agent: &str, token: &str, turn_id: &str) -> Result<()> {
        self.mutate(|b| b.turn_ended(agent, token, turn_id))
    }

    /// Since when a wake item has waited on agent without being taken up,
    /// when that is longer than `after` (card #110): the oldest open
    /// delivery to agent that should wake it (not next-turn, not held or
    /// deferred) still queued or delivered and not engaged. `None` while
    /// wakes are paused, for an agent another device hosts, or when nothing
    /// waits that long. Whether the agent is idle is the caller's to check:
    /// a busy agent takes its wakes when its turn ends.
    pub fn unanswered_since(&self, agent: &str, after: std::time::Duration) -> Option<GoTime> {
        if agent.is_empty() {
            return None;
        }
        self.read(|b| {
            if b.pol.paused || !b.local(agent) {
                return None;
            }
            let now = b.now();
            let mut oldest: Option<GoTime> = None;
            for d in b.open_recs() {
                if d.d.to != agent
                    || d.next_turn()
                    || (d.d.state != crate::st::QUEUED && d.d.state != crate::st::DELIVERED)
                {
                    continue;
                }
                let since = d.waiting_since;
                if oldest.is_none_or(|o| crate::tm::before(&since, &o)) {
                    oldest = Some(since);
                }
            }
            oldest.filter(|o| crate::tm::sub(&now, o) > crate::tm::d(after))
        })
    }

    /// The wake whose turn agent is in now, or `""`.
    pub fn running_wake(&self, agent: &str) -> String {
        self.read(|b| {
            b.running
                .get(agent)
                .map(|r| r.wake_id.clone())
                .unwrap_or_default()
        })
    }
}

/// The first line of a stop's prompt (card #207): the agent must not read it
/// as an ordinary message.
pub(crate) fn stop_header(msgs: &[Delivery]) -> String {
    let who = msgs
        .first()
        .map(|d| d.message.from.name.trim())
        .filter(|n| !n.is_empty())
        .unwrap_or("The person");
    format!(
        "STOP from {who}: your running turn was cut. Answer {who} first and do nothing else until they say so."
    )
}
