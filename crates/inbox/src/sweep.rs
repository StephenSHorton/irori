//! The sweeper (I:internal/inbox/sweep.go): a pure planner over the open
//! deliveries, then the actions, the card handoffs (outside the lock) and
//! the passed-message card fills.

use std::collections::BTreeSet;

use gocompat::time::GoTime;

use crate::messages::clip;
use crate::policy::{Cards, placed};
use crate::pr::pr_fallback_repo;
use crate::state::{DeliveryRow, Inner, PassRec};
use crate::{ErrorKind, Inbox, MAX_NOTE, Message, NOTE_RECIPIENT_GONE, Result, hold, st, tm};

/// What the planner sees of one open delivery.
#[derive(Debug, Clone, Default)]
pub(crate) struct SweepItem {
    pub id: String,
    pub to: String,
    pub from: String,
    pub repo: String,
    pub gone_repo: String,
    pub state: String,
    pub hold: String,
    pub assigned_at: GoTime,
    pub updated_at: GoTime,
    pub until: GoTime,
    pub in_flight: bool,
    pub issued_at: GoTime,
    pub moved_from: bool,
    pub engaged_once: bool,
    pub reminded: bool,
    pub next_turn: bool,
    pub ref_: String,
    pub notice: bool,
    pub system: bool,
    pub from_person: bool,
}

/// The world the planner plans against. Durations in nanoseconds.
pub(crate) struct SweepEnv<'a> {
    pub now: GoTime,
    pub unengaged: i128,
    pub gone: i128,
    pub quiet: i128,
    pub grace: i128,
    pub paused: bool,
    pub gone_for: &'a dyn Fn(&str) -> i128,
    pub ended_for: &'a dyn Fn(&str) -> i128,
    pub self_dev: String,
    pub reachable: &'a dyn Fn(&str) -> bool,
    pub desk: &'a dyn Fn(&str, &[&str]) -> String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Act {
    Reclaim,
    Requeue,
    Move,
    Assign,
    Remind,
    GoneCard,
    GonePerson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SweepAction {
    pub kind: Act,
    pub delivery_id: String,
    pub to: String,
}

pub(crate) fn plan_sweep(items: &[SweepItem], env: &SweepEnv<'_>) -> Vec<SweepAction> {
    items.iter().filter_map(|it| plan_one(it, env)).collect()
}

pub(crate) fn plan_one(it: &SweepItem, env: &SweepEnv<'_>) -> Option<SweepAction> {
    let act = |k: Act, to: String| {
        Some(SweepAction {
            kind: k,
            delivery_id: it.id.clone(),
            to,
        })
    };
    match it.state.as_str() {
        st::HELD => {
            // Only unowned repo work moves on its own.
            if it.hold == hold::UNOWNED && !it.repo.is_empty() {
                let to = (env.desk)(&it.repo, &[&it.from]);
                if !to.is_empty() {
                    return act(Act::Assign, to);
                }
            }
            return None;
        }
        st::QUEUED | st::DELIVERED | st::ENGAGED | st::DEFERRED => {}
        _ => return None,
    }
    let gr = if it.repo.is_empty() {
        &it.gone_repo
    } else {
        &it.repo
    };
    if !gr.is_empty() && !it.to.is_empty() && (env.gone_for)(&it.to) > env.gone {
        return act(Act::Move, (env.desk)(gr, &[&it.to, &it.from]));
    }
    if gr.is_empty()
        && !it.to.is_empty()
        && !it.notice
        && !it.system
        && (env.ended_for)(&it.to) > env.gone
    {
        if ref_names_card(&it.ref_) {
            return act(Act::GoneCard, String::new());
        }
        if !it.from_person {
            return act(Act::GonePerson, format!("user:{}", env.self_dev));
        }
    }
    if it.in_flight && tm::sub(&env.now, &it.issued_at) > env.grace {
        return act(Act::Reclaim, String::new());
    }
    if it.state == st::DEFERRED {
        if !tm::before(&env.now, &it.until) {
            return act(Act::Requeue, String::new());
        }
        return None;
    }
    if it.next_turn {
        return None;
    }
    if !it.repo.is_empty()
        && !it.engaged_once
        && (it.state == st::QUEUED || it.state == st::DELIVERED)
        && tm::sub(&env.now, &it.assigned_at) > env.unengaged
    {
        if !it.moved_from {
            let to = (env.desk)(&it.repo, &[&it.to, &it.from]);
            if !to.is_empty() {
                return act(Act::Move, to);
            }
        }
        if it.state == st::DELIVERED && !it.reminded && !env.paused && (env.reachable)(&it.to) {
            return act(Act::Remind, it.to.clone());
        }
        return None;
    }
    let quiet = if it.state == st::DELIVERED && !it.engaged_once {
        env.unengaged
    } else {
        env.quiet
    };
    if (it.state == st::ENGAGED || it.state == st::DELIVERED)
        && !it.reminded
        && !env.paused
        && tm::sub(&env.now, &it.updated_at) > quiet
        && (env.reachable)(&it.to)
    {
        return act(Act::Remind, it.to.clone());
    }
    None
}

/// A message ref names a Board card: a task, or a comment on one.
pub(crate) fn ref_names_card(r: &str) -> bool {
    r.starts_with("task:") || r.starts_with("comment:")
}

/// Agent work whose recipient ended, for the card its message names.
#[derive(Debug, Clone)]
pub(crate) struct Handoff {
    pub delivery_id: String,
    pub message: Message,
    pub to: String,
    pub from_person: bool,
}

impl Inner {
    pub(crate) fn sweep(&mut self) -> Result<Vec<Handoff>> {
        let now = self.now();
        let cards = self.live_cards()?;
        self.drop_stale("")?;

        let gone = |b: &Inner, a: &str| !cards.contains_key(a) || !b.reachable(a, &now);
        let mut seen = BTreeSet::new();
        let tos: Vec<String> = self.open_recs().map(|d| d.d.to.clone()).collect();
        for to in tos {
            if to.is_empty() || self.is_owner(&to) || seen.contains(&to) || !self.local(&to) {
                continue;
            }
            seen.insert(to.clone());
            if gone(self, &to) {
                self.gone_since.entry(to.clone()).or_insert(now);
            } else {
                self.gone_since.remove(&to);
            }
            if !cards.contains_key(&to) {
                self.ended_since.entry(to.clone()).or_insert(now);
            } else {
                self.ended_since.remove(&to);
            }
        }
        self.gone_since.retain(|a, _| seen.contains(a));
        self.ended_since.retain(|a, _| seen.contains(a));
        let open = self.open.clone();
        self.no_card_since.retain(|id, _| open.contains(id));

        let mut items = Vec::with_capacity(self.open.len());
        for id in &self.order {
            if !self.open.contains(id) {
                continue;
            }
            let d = &self.dels[id];
            if !self.owns(d) {
                continue;
            }
            let mut it = SweepItem {
                id: d.d.id.clone(),
                to: d.d.to.clone(),
                repo: d.d.repo_key.clone(),
                state: d.d.state.clone(),
                hold: d.d.hold.clone(),
                assigned_at: d.assigned_at,
                updated_at: d.d.updated_at,
                until: d.d.until,
                moved_from: !d.moved_from.is_empty(),
                engaged_once: d.engaged_once,
                reminded: d.d.reminded,
                next_turn: d.next_turn(),
                ..SweepItem::default()
            };
            let m = self.msgs.get(&d.d.message_id);
            if let Some(m) = m {
                it.from = m.from.key.clone();
                it.ref_ = m.ref_.clone();
                it.notice = m.notice;
                it.system = m.from.is_system();
                it.from_person = self.is_owner(&m.from.key);
            }
            it.gone_repo = pr_fallback_repo(m, &d.d.repo_key);
            if d.d.state == st::QUEUED
                && !d.d.wake_id.is_empty()
                && let Some(w) = self.wakes.get(&d.d.wake_id)
                && !w.w.reminder
            {
                it.in_flight = true;
                it.issued_at = w.issued_at;
            }
            items.push(it);
        }

        let plan = {
            let gone_for = |a: &str| self.gone_since.get(a).map_or(0, |t| tm::sub(&now, t));
            let ended_for = |a: &str| self.ended_for(a, &now);
            let reachable = |a: &str| self.reachable(a, &now);
            let desk = |repo: &str, ex: &[&str]| self.desk(repo, &cards, &now, ex);
            let env = SweepEnv {
                now,
                unengaged: self.lim.unengaged,
                gone: self.lim.gone,
                quiet: self.lim.quiet,
                grace: self.lim.reattach,
                paused: self.pol.paused,
                gone_for: &gone_for,
                ended_for: &ended_for,
                self_dev: self.dev.clone(),
                reachable: &reachable,
                desk: &desk,
            };
            plan_sweep(&items, &env)
        };

        let mut hs = vec![];
        for a in plan {
            let d = &self.dels[&a.delivery_id];
            let mut r = d.row();
            match a.kind {
                Act::Reclaim => {
                    r.wake_id.clear();
                    self.put_delivery(r, "irori")?;
                }
                Act::Requeue => {
                    r.state = st::QUEUED.into();
                    r.until = GoTime::ZERO;
                    r.wake_id.clear();
                    self.put_delivery(r, "irori")?;
                }
                Act::Assign => {
                    r.to = a.to.clone();
                    if self.msgs.contains_key(&d.d.message_id) {
                        let h = self.hold_id(&d.d.message_id, &a.to, &cards);
                        r = placed(r, h);
                    }
                    self.put_delivery(r, "irori")?;
                }
                Act::Move => self.move_to(&a.delivery_id, &a.to, &cards)?,
                Act::GoneCard => {
                    // The card is written outside the lock (hand_off).
                    if let Some(m) = self.msgs.get(&d.d.message_id) {
                        hs.push(Handoff {
                            delivery_id: d.d.id.clone(),
                            message: m.clone(),
                            to: d.d.to.clone(),
                            from_person: self.is_owner(&m.from.key),
                        });
                    }
                }
                Act::GonePerson => self.move_gone(&a.delivery_id, &a.to, &cards)?,
                Act::Remind => {
                    let list = self.reminders.entry(a.to.clone()).or_default();
                    if !list.contains(&a.delivery_id) {
                        list.push(a.delivery_id.clone());
                        self.touch(&[&a.to]);
                    }
                }
            }
        }
        // Some gates open with time alone: reconcile WakeReady for everyone.
        for agent in self.agents_with_open() {
            self.touched.insert(agent);
        }
        Ok(hs)
    }

    /// How long agent has been out of presence (0 while around).
    pub(crate) fn ended_for(&self, agent: &str, now: &GoTime) -> i128 {
        self.ended_since.get(agent).map_or(0, |t| tm::sub(now, t))
    }

    /// h's delivery if it is still the open, owned work of the same
    /// recipient, still out of presence past the grace.
    fn still_gone(&self, h: &Handoff) -> Option<String> {
        if !self.open.contains(&h.delivery_id) {
            return None;
        }
        let d = &self.dels[&h.delivery_id];
        if d.d.state == st::HELD
            || !self.owns(d)
            || d.d.to != h.to
            || self.ended_for(&d.d.to, &self.now()) <= self.lim.gone
        {
            return None;
        }
        if self.roster_cards().contains_key(&d.d.to) {
            return None;
        }
        Some(d.d.id.clone())
    }

    fn close_gone(&mut self, h: &Handoff, task_id: &str) -> Result<()> {
        let Some(id) = self.still_gone(h) else {
            return Ok(());
        };
        self.no_card_since.remove(&id);
        let mut r = self.dels[&id].row();
        r.state = st::SKIPPED.into();
        r.note = clip(
            &format!("{NOTE_RECIPIENT_GONE}; on task:{task_id}"),
            MAX_NOTE,
        );
        r.until = GoTime::ZERO;
        self.put_delivery(r, "irori")
    }

    fn no_card(&mut self, h: &Handoff) -> Result<()> {
        let Some(id) = self.still_gone(h) else {
            self.no_card_since.remove(&h.delivery_id);
            return Ok(());
        };
        if h.from_person {
            return Ok(());
        }
        if self.on_gone.is_some() {
            let Some(since) = self.no_card_since.get(&id).copied() else {
                let now = self.now();
                self.no_card_since.insert(id, now);
                return Ok(());
            };
            if tm::sub(&self.now(), &since) <= self.lim.no_card {
                return Ok(());
            }
        }
        self.no_card_since.remove(&id);
        let cards = self.live_cards()?;
        let to = format!("user:{}", self.dev);
        self.move_gone(&id, &to, &cards)
    }

    /// Queues d's message for its card once this device let d through.
    pub(crate) fn passed(&mut self, did: &str) {
        if self.on_passed.is_none() {
            return;
        }
        let Some(d) = self.dels.get(did) else {
            return;
        };
        if d.d.to.is_empty() || !self.owns(d) {
            return;
        }
        let Some(m) = self.msgs.get(&d.d.message_id) else {
            return;
        };
        if m.notice || !ref_names_card(&m.ref_) {
            return;
        }
        if !self.passing.contains_key(did) {
            let rec = PassRec {
                msg: d.d.message_id.clone(),
                to: d.d.to.clone(),
                since: self.now(),
            };
            self.passing.insert(did.to_string(), rec);
        }
    }

    /// Hands agent work whose recipient ended to `to` (the person).
    pub(crate) fn move_gone(&mut self, did: &str, to: &str, cards: &Cards) -> Result<()> {
        let old_to = self.dels[did].d.to.clone();
        let for_text = clip(
            &format!("{NOTE_RECIPIENT_GONE}: {}", self.describe(&old_to, cards)),
            MAX_NOTE,
        );
        self.move_with(did, to, cards, for_text, NOTE_RECIPIENT_GONE)
    }

    /// Hands d's message to a new delivery for to (`""` = unowned), then
    /// closes d as Moved. The new row goes first.
    pub(crate) fn move_to(&mut self, did: &str, to: &str, cards: &Cards) -> Result<()> {
        let d = &self.dels[did];
        let mut for_text = d.d.for_.clone();
        if d.d.repo_key.is_empty()
            && !pr_fallback_repo(self.msgs.get(&d.d.message_id), &d.d.repo_key).is_empty()
        {
            for_text = clip(&self.describe(&d.d.to, cards), MAX_NOTE); // its owner has gone
        }
        self.move_with(did, to, cards, for_text, "")
    }

    fn move_with(
        &mut self,
        did: &str,
        to: &str,
        cards: &Cards,
        for_text: String,
        note: &str,
    ) -> Result<()> {
        let d = &self.dels[did];
        let m = self.msgs.get(&d.d.message_id);
        let nr = DeliveryRow {
            delivery_id: format!("dlv_{}", self.ids.next(&self.now())),
            message_id: d.d.message_id.clone(),
            to: to.into(),
            repo_key: pr_fallback_repo(m, &d.d.repo_key),
            mode: d.d.mode.clone(),
            moved_from: d.d.id.clone(),
            for_: for_text,
            ..DeliveryRow::default()
        };
        let h = if !to.is_empty() && !self.local(to) {
            "" // its host decides with its own policy (v0.4 §6.3)
        } else if let Some(m) = m {
            self.hold(m, to, cards)
        } else {
            hold::UNOWNED
        };
        self.put_delivery(placed(nr, h), "irori")?;
        let mut r = self.dels[did].row();
        r.state = st::MOVED.into();
        r.moved_to = to.into();
        r.wake_id.clear();
        if !note.is_empty() {
            r.note = note.into();
        }
        self.put_delivery(r, "irori")
    }
}

impl Inbox {
    /// Runs the sweeper once: reassign, gone-moves, reminders, due
    /// deferrals, front-desk succession, ended agents' work to its card or
    /// the person, and passed messages' text onto their cards.
    pub fn sweep(&self) -> Result<()> {
        let hs = self.mutate(|b| b.sweep())?;
        let r = self.hand_off(&hs);
        self.fill_cards();
        r
    }

    /// Puts the handoffs on their cards without the lock, then closes each.
    fn hand_off(&self, hs: &[Handoff]) -> Result<()> {
        let on_gone = self.read(|b| b.on_gone.clone());
        for h in hs {
            if let Some(f) = &on_gone {
                match f(&h.message, &h.to) {
                    Ok(tid) => {
                        self.mutate(|b| b.close_gone(h, &tid))?;
                        continue;
                    }
                    Err(e) if !e.is(ErrorKind::NoCard) => {
                        eprintln!("inbox: recipient gone: {} to its card: {e}", h.delivery_id);
                        continue;
                    }
                    Err(_) => {}
                }
            }
            self.mutate(|b| b.no_card(h))?;
        }
        Ok(())
    }

    /// Puts the text of messages let through on their cards, without the
    /// lock; a failure is tried again on later calls for NoCardGrace.
    pub(crate) fn fill_cards(&self) {
        let Some(on_passed) = self.read(|b| b.on_passed.clone()) else {
            return;
        };
        let jobs: Vec<(String, PassRec, Message)> = self.mutate(|b| {
            let passing = std::mem::take(&mut b.passing);
            passing
                .into_iter()
                .filter_map(|(id, p)| b.msgs.get(&p.msg).cloned().map(|m| (id, p, m)))
                .collect()
        });
        let mut retry = vec![];
        for (id, p, m) in jobs {
            if let Err(e) = on_passed(&m, &p.to) {
                let (now, grace) = self.read(|b| (b.now(), b.lim.no_card));
                if tm::sub(&now, &p.since) <= grace {
                    retry.push((id, p));
                } else {
                    eprintln!("inbox: {id} was let through, but its text isn't on its card: {e}");
                }
            }
        }
        if retry.is_empty() {
            return;
        }
        self.mutate(|b| {
            for (id, p) in retry {
                b.passing.entry(id).or_insert(p);
            }
        });
    }
}

#[cfg(test)]
mod tests;
