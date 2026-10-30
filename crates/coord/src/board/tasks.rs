//! Task and lease calls, and the device's duties as a holder
//! (I:internal/coord/impl.go, mesh.go; coord.md §3.3, §3.4).

use std::time::Duration;

use gocompat::quote::quote;
use gocompat::time::GoTime;
use store::{Event, kinds};

use super::cards::{block_question, check_steps, new_steps};
use super::comments::check_reason;
use super::error::{Result, bad_state, held, invalid, not_allowed, not_found, subtask};
use super::fold::{
    FOLLOW_KINDS, Fold, REASON_EXPIRED, REASON_HOLDER_GONE, REASON_RELEASED, engaged, is_holder,
    restore_state, stolen_key,
};
use super::links::{MAX_LINKS, parse_link};
use super::resource::{
    check_repo_key, conflicts, normalize_glob, normalize_resource, parse_resource,
};
use super::rows::*;
use super::types::*;
use super::util::{SWEEPER, caller, check_key, cmp_str, cut, deeper, new_id, runes};
use super::{DEFAULT_TTL, MAX_OBJECTIVE, MAX_SCOPE, MAX_TITLE, MAX_TTL};
use crate::gotime;
use crate::names::trim_space;

/// A NewTask checked and trimmed.
#[derive(Debug, Clone, Default)]
pub(crate) struct CleanTask {
    pub by: String,
    pub title: String,
    pub obj: String,
    pub done: String,
    pub repo_key: String,
    pub reason: String,
    pub parent: String,
    pub scope: Vec<String>,
    pub links: Vec<Link>,
    pub shared: bool,
    pub blocked_by: Option<BlockedBy>,
    pub steps: Vec<Step>,
}

/// CreateTask's checks.
pub(crate) fn clean_new(by: &str, nt: &NewTask) -> Result<CleanTask> {
    let by = trim_space(by);
    if by.is_empty() {
        return Err(invalid("no caller"));
    }
    let title = trim_space(&nt.title).to_string();
    if title.is_empty() {
        return Err(invalid("a task needs a title"));
    }
    if runes(&title) > MAX_TITLE {
        return Err(invalid(format!("title longer than {MAX_TITLE} characters")));
    }
    let obj = trim_space(&nt.objective).to_string();
    let done = trim_space(&nt.done_criteria).to_string();
    if runes(&obj) > MAX_OBJECTIVE || runes(&done) > MAX_OBJECTIVE {
        return Err(invalid(format!(
            "objective or done criteria longer than {MAX_OBJECTIVE} characters"
        )));
    }
    let repo_key = trim_space(&nt.repo_key).to_string();
    if !repo_key.is_empty() {
        check_repo_key(&repo_key)?;
    }
    let mut scope = Vec::new();
    for g in &nt.scope {
        if trim_space(g).is_empty() {
            continue;
        }
        scope.push(normalize_glob(g)?);
    }
    if scope.len() > MAX_SCOPE {
        return Err(invalid(format!("more than {MAX_SCOPE} scope globs")));
    }
    let mut links: Vec<Link> = Vec::new();
    for s in &nt.links {
        if trim_space(s).is_empty() {
            continue;
        }
        let l = parse_link(s)?;
        if !links.iter().any(|x| x.reference == l.reference) {
            links.push(l);
        }
    }
    if links.len() > MAX_LINKS {
        return Err(invalid(format!("more than {MAX_LINKS} links")));
    }
    let reason = trim_space(&nt.block_reason).to_string();
    if reason.is_empty() && nt.blocked_by.is_some() {
        return Err(invalid("BlockedBy needs a BlockReason"));
    }
    check_reason(&reason, false)?;
    let mut steps = nt.steps.clone();
    check_steps(&mut steps)?;
    Ok(CleanTask {
        by: by.into(),
        title,
        obj,
        done,
        repo_key,
        reason,
        scope,
        links,
        shared: nt.shared,
        blocked_by: nt.blocked_by.clone(),
        steps: new_steps(&steps),
        parent: String::new(),
    })
}

impl Fold {
    pub(crate) fn create_task(&mut self, by: &str, nt: NewTask) -> Result<Task> {
        let mut ct = clean_new(by, &nt)?;
        if !nt.parent.is_empty() {
            let pid = self.task_ref_locked(&nt.parent)?;
            self.check_parent(&pid, "")?;
            ct.parent = pid;
        }
        let by = ct.by.clone();
        self.create_locked(&by, ct)
    }

    /// Writes a checked task's rows.
    pub(crate) fn create_locked(&mut self, by: &str, ct: CleanTask) -> Result<Task> {
        let mut bb = None;
        if !ct.reason.is_empty() {
            bb = self.check_blocker(ct.blocked_by.as_ref(), "")?;
        }
        let at = self.stamp();
        let id = new_id(&at);
        let num = self.next_short();
        self.write(
            by,
            kinds::TASK_CREATED,
            &TaskCreatedRow {
                task_id: id.clone(),
                num,
                title: ct.title.clone(),
                objective: ct.obj.clone(),
                done_criteria: ct.done.clone(),
                repo_key: ct.repo_key.clone(),
                scope: ct.scope.clone(),
                shared: ct.shared,
                links: ct.links.clone(),
                parent: ct.parent.clone(),
                by: by.into(),
                at,
                ask: None,
            },
        )?;
        if !ct.steps.is_empty() {
            self.write(
                by,
                kinds::TASK_STEPS,
                &TaskStepsRow {
                    task_id: id.clone(),
                    steps: ct.steps.clone(),
                    by: by.into(),
                    at,
                },
            )?;
        }
        if !ct.reason.is_empty() {
            let (q, cx) = block_question(&ct.reason);
            let bid = self.next_blocker_id(&id);
            self.write(
                by,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: id.clone(),
                    state: BLOCKED.into(),
                    reason: ct.reason.clone(),
                    blocked_by: bb.clone(),
                    blocker: Some(BlockerRow {
                        id: bid,
                        question: q,
                        context: cx,
                        reason_only: true,
                        on: bb,
                        ..BlockerRow::default()
                    }),
                    by: by.into(),
                    at,
                    ..TaskStateRow::default()
                },
            )?;
        }
        Ok(self.tasks[&id].clone())
    }

    pub(crate) fn list_tasks(&self, f: &TaskFilter) -> Vec<Task> {
        let mut out: Vec<Task> = self
            .tasks
            .values()
            .filter(|t| {
                !(!f.repo_key.is_empty() && t.repo_key != f.repo_key
                    || !f.holder.is_empty() && !is_holder(t, &f.holder)
                    || f.open && is_terminal(effective_state(t)))
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            gotime::cmp(&a.created_at, &b.created_at)
                .then_with(|| self.order.get(&a.id).cmp(&self.order.get(&b.id)))
                .then_with(|| a.id.cmp(&b.id))
        });
        out
    }

    pub(crate) fn claim_task(&mut self, by: &str, id: &str) -> Result<Task> {
        let by = caller(by)?;
        let t = self.open_task(id)?.clone();
        if t.open_ask().is_some() {
            return Err(super::asks::ask_refusal(&t, "one agent takes it"));
        }
        if engaged(&t, &by) && t.state != SUBMITTED {
            return Ok(t);
        }
        let now = self.stamp();
        if !t.shared {
            for h in &t.holders {
                if *h == by {
                    continue;
                }
                self.observe(&now);
                if !self.gone(h, &now) {
                    return Err(held(format!("task {id} is held by {h}")));
                }
            }
        }
        self.write(
            &by,
            kinds::TASK_CLAIMED,
            &TaskClaimedRow {
                task_id: id.into(),
                by: by.clone(),
                at: now,
            },
        )?;
        Ok(self.tasks[id].clone())
    }

    /// AssignTask (wide false) and Assign (wide: also anyone while the
    /// task has no holder). The bool reports whether a row was written.
    pub(crate) fn assign(
        &mut self,
        by: &str,
        id: &str,
        to: &str,
        wide: bool,
    ) -> Result<(Task, bool)> {
        let (by, to) = (trim_space(by).to_string(), trim_space(to).to_string());
        if by.is_empty() || to.is_empty() {
            return Err(invalid("assign needs a caller and an assignee"));
        }
        check_key(&to)?;
        let t = self.open_task(id)?.clone();
        if t.open_ask().is_some() {
            return Err(super::asks::ask_refusal(&t, "nobody is assigned one"));
        }
        if !self.may_manage(&t, &by) && !(wide && t.holders.is_empty()) {
            return Err(not_allowed(format!("assign task {id}")));
        }
        if is_holder(&t, &to) {
            return Ok((t, false));
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_ASSIGNED,
            &TaskAssignedRow {
                task_id: id.into(),
                to,
                by: by.clone(),
                at,
            },
        )?;
        Ok((self.tasks[id].clone(), true))
    }

    pub(crate) fn set_task_state(
        &mut self,
        by: &str,
        id: &str,
        s: &str,
        note: &str,
    ) -> Result<Task> {
        let by = caller(by)?;
        if !valid_state(s) {
            return Err(bad_state(format!(
                "{} (submitted|working|input_required|completed|failed|canceled)",
                quote(s)
            )));
        }
        if s == BLOCKED {
            return Err(bad_state("use Block, which records what is in the way"));
        }
        let mut note = trim_space(note).to_string();
        if runes(&note) > MAX_OBJECTIVE {
            return Err(invalid(format!(
                "note longer than {MAX_OBJECTIVE} characters"
            )));
        }
        let t = self.open_task(id)?.clone();
        if t.open_ask().is_some() {
            if !self.is_owner(&by) || !is_terminal(s) {
                return Err(super::asks::ask_refusal(
                    &t,
                    "its state follows its outcome",
                ));
            }
            note = cmp_str(&note, "closed by the person");
            let at = self.stamp();
            self.write(
                &by,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: id.into(),
                    state: CANCELED.into(),
                    note: note.clone(),
                    by: by.clone(),
                    at,
                    outcome: Some(Outcome {
                        kind: super::asks::OUTCOME_NONE.into(),
                        note: cut(&note, super::asks::MAX_OUTCOME),
                        ..Outcome::default()
                    }),
                    ..TaskStateRow::default()
                },
            )?;
            return Ok(self.tasks[id].clone());
        }
        if !self.may_manage(&t, &by) {
            return Err(not_allowed(format!("change task {id}")));
        }
        let cascade = self.parent_state_refusal(&t, s)?;
        if t.state == s && t.note == note {
            return Ok(t);
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::TASK_STATUS,
            &TaskStateRow {
                task_id: id.into(),
                state: s.into(),
                note,
                by: by.clone(),
                at,
                ..TaskStateRow::default()
            },
        )?;
        if cascade {
            let parent_ref = t.short_ref();
            for k in self.tasks[id].subtasks.clone() {
                if self.tasks.get(&k).is_some_and(|kt| !is_terminal(&kt.state)) {
                    let at = self.stamp();
                    self.write(
                        &by,
                        kinds::TASK_STATUS,
                        &TaskStateRow {
                            task_id: k,
                            state: CANCELED.into(),
                            note: format!("its parent {parent_ref} was canceled"),
                            by: by.clone(),
                            at,
                            ..TaskStateRow::default()
                        },
                    )?;
                }
            }
        }
        Ok(self.tasks[id].clone())
    }

    pub(crate) fn reopen(&mut self, by: &str, id: &str, note: &str) -> Result<(Task, Notice)> {
        let by = caller(by)?;
        let t = self.any_task(id)?.clone();
        if !self.may_manage(&t, &by) {
            return Err(not_allowed(format!("reopen task {id}")));
        }
        let was = t.state.clone();
        self.reopen_locked(&by, id, note)?;
        let t = self.tasks[id].clone();
        let mut n = Notice {
            kind: NOTICE_REOPENED.into(),
            task_id: t.id.clone(),
            reference: format!("task:{}", t.id),
            title: t.title.clone(),
            by: by.clone(),
            to: self.recipients(&t, &by),
            ..Notice::default()
        };
        let who = if self.is_owner(&by) {
            "The person"
        } else {
            &by
        };
        n.text = format!(
            "{who} reopened {} ({}), which was {was}. It is {} again",
            quote(&t.title),
            t.refs(),
            t.state
        );
        if !t.holders.is_empty() {
            n.text.push_str(", with you on it: pick it up, or board_status it done again if there is nothing left to do.");
        } else {
            n.text.push_str(", with nobody on it.");
        }
        n.text.push_str(&deeper(&t.id));
        Ok((t, n))
    }

    /// Writes t's reopening state row; a finished parent comes back first.
    pub(crate) fn reopen_locked(&mut self, by: &str, id: &str, note: &str) -> Result<()> {
        let t = self.tasks[id].clone();
        self.reopen_refusal(&t)?;
        if let Some(p) = self.tasks.get(&t.parent).cloned()
            && is_terminal(&p.state)
        {
            let at = self.stamp();
            self.write(
                by,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: p.id.clone(),
                    state: restore_state(&p).into(),
                    note: format!("its subtask {} was reopened", t.short_ref()),
                    by: by.into(),
                    at,
                    ..TaskStateRow::default()
                },
            )?;
        }
        let at = self.stamp();
        let t = self.tasks[id].clone();
        self.write(
            by,
            kinds::TASK_STATUS,
            &TaskStateRow {
                task_id: id.into(),
                state: restore_state(&t).into(),
                note: note.into(),
                by: by.into(),
                at,
                ..TaskStateRow::default()
            },
        )
    }

    /// Why t can't be reopened.
    pub(crate) fn reopen_refusal(&self, t: &Task) -> Result<()> {
        if !is_terminal(&t.state) {
            return Err(bad_state(format!(
                "{} is {}, not finished",
                t.short_ref(),
                t.state
            )));
        }
        if t.subtasks.is_empty() {
            return Ok(());
        }
        if t.subtasks
            .iter()
            .any(|k| self.tasks.get(k).is_some_and(|kt| !is_terminal(&kt.state)))
        {
            return Ok(());
        }
        Err(subtask(format!(
            "{}'s subtasks have all ended, and a parent's state follows them; reopen a subtask instead",
            t.short_ref()
        )))
    }

    // ---- leases ----

    pub(crate) fn acquire(
        &mut self,
        holder: &str,
        resource: &str,
        ttl: Duration,
        steal: bool,
    ) -> Result<Lease> {
        let holder = trim_space(holder).to_string();
        if holder.is_empty() {
            return Err(invalid("no holder"));
        }
        let res = parse_resource(resource)?;
        let ttl = if ttl.is_zero() { DEFAULT_TTL } else { ttl }.min(MAX_TTL);
        let now = self.stamp();
        self.prune(&now);
        let mut victims: Vec<(String, String, GoTime, GoTime)> = self
            .held
            .values()
            .filter(|l| l.lease.holder != holder && self.live(l, &now) && conflicts(&l.res, &res))
            .map(|l| {
                (
                    l.lease.resource.clone(),
                    l.lease.holder.clone(),
                    l.lease.until,
                    l.lease.acquired_at,
                )
            })
            .collect();
        victims.sort_by(|a, b| a.0.cmp(&b.0));
        if steal
            && let Some(l) = self.held.values().find(|l| {
                victims.iter().any(|v| v.0 == l.lease.resource) && self.others_lease(&holder, l)
            })
        {
            return Err(self.not_yours(&l.lease));
        }
        if !victims.is_empty() && !steal {
            let v = &victims[0];
            let e = held(format!(
                "{} is leased by {} until {} (pass steal to take it)",
                v.0,
                v.1,
                v.2.format_rfc3339()
            ));
            let vs: Vec<(String, String, GoTime)> = victims
                .iter()
                .map(|v| (v.0.clone(), v.1.clone(), v.2))
                .collect();
            if let Err(err) = self.refuse(&holder, &res, &vs, now) {
                return Err(super::error::Error::raw(
                    e.kind(),
                    format!("{e} (logging the refusal failed: {err})"),
                ));
            }
            return Err(e);
        }
        let mut from: Vec<String> = Vec::new();
        let mut stolen = Vec::new();
        for v in &victims {
            stolen.push(StolenLease {
                resource: v.0.clone(),
                holder: v.1.clone(),
            });
            if !from.contains(&v.1) {
                from.push(v.1.clone());
            }
        }
        from.sort();
        let mut l = Lease {
            resource: res.name.clone(),
            holder: holder.clone(),
            until: gotime::add(&now, ttl),
            acquired_at: now,
            stolen_from: from.join(","),
            conflict: String::new(),
        };
        if let Some(old) = self.held.get(&res.name)
            && old.lease.holder == holder
            && self.live(old, &now)
        {
            l.acquired_at = old.lease.acquired_at;
            if l.stolen_from.is_empty() {
                l.stolen_from = old.lease.stolen_from.clone();
            }
        }
        self.write(
            &holder,
            kinds::LEASE_ACQUIRED,
            &LeaseAcquiredRow { lease: l, stolen },
        )?;
        self.held
            .get(&res.name)
            .map(|r| r.lease.clone())
            .ok_or_else(|| not_found(format!("no lease on {}", res.name)))
    }

    pub(crate) fn release(&mut self, by: &str, resource: &str) -> Result<()> {
        let by = caller(by)?;
        let name = normalize_resource(resource)?;
        let now = self.stamp();
        let Some(l) = self.held.get(&name).filter(|l| self.live(l, &now)).cloned() else {
            return Err(not_found(format!("no lease on {name}")));
        };
        if l.lease.holder != by && self.others_lease(&by, &l) {
            return Err(self.not_yours(&l.lease));
        }
        if l.lease.holder != by && !self.is_owner(&by) {
            return Err(held(format!("{name} is leased by {}", l.lease.holder)));
        }
        // On a board the holder's person releases from any of their
        // devices (§2.4); privately another device's lease is stolen.
        if !self.on_board && !self.local_key(&l.lease.holder) {
            return self.steal(&by, &l.lease);
        }
        self.write(
            &by,
            kinds::LEASE_RELEASED,
            &LeaseReleasedRow {
                resource: name,
                holder: l.lease.holder,
                reason: REASON_RELEASED.into(),
                by: by.clone(),
                at: now,
            },
        )
    }

    pub(crate) fn leases(&mut self) -> Vec<Lease> {
        let now = self.stamp();
        self.prune(&now);
        let mut out: Vec<Lease> = self
            .held
            .values()
            .filter(|l| self.live(l, &now))
            .map(|l| l.lease.clone())
            .collect();
        out.extend(self.conflicts.values().map(|l| l.lease.clone()));
        out.sort_by(|a, b| a.resource.cmp(&b.resource));
        out
    }

    /// Releases the leases of holders gone for the grace period.
    pub(crate) fn release_gone(&mut self, now: &GoTime) -> Result<()> {
        self.prune(now);
        self.observe(now);
        let gone: Vec<Lease> = self
            .held
            .values()
            .filter(|l| {
                gotime::after(&l.lease.until, now)
                    && self.lease_local(l)
                    && self.gone(&l.lease.holder, now)
            })
            .map(|l| l.lease.clone())
            .collect();
        for l in gone {
            self.write(
                SWEEPER,
                kinds::LEASE_RELEASED,
                &LeaseReleasedRow {
                    resource: l.resource,
                    holder: l.holder,
                    reason: REASON_HOLDER_GONE.into(),
                    by: SWEEPER.into(),
                    at: *now,
                },
            )?;
        }
        Ok(())
    }

    /// Writes lease.stolen for l.
    pub(crate) fn steal(&mut self, by: &str, l: &Lease) -> Result<()> {
        let at = self.stamp();
        self.write(
            by,
            kinds::LEASE_STOLEN,
            &LeaseStolenRow {
                resource: l.resource.clone(),
                holder: l.holder.clone(),
                acquired_at: l.acquired_at,
                by: by.into(),
                at,
            },
        )
    }

    pub(crate) fn steal_lease(&mut self, by: &str, resource: &str) -> Result<()> {
        if !self.is_owner(by) {
            return Err(invalid("only the person steals a lease"));
        }
        let name = normalize_resource(resource)?;
        let now = self.stamp();
        let Some(l) = self.held.get(&name).filter(|l| self.live(l, &now)).cloned() else {
            return Err(not_found(format!("no lease on {name}")));
        };
        if self.others_lease(by, &l) {
            return Err(self.not_yours(&l.lease));
        }
        self.steal(by, &l.lease)
    }

    /// This device's part as a holder: refuse its own lease that lost a
    /// conflict, record its own leases' expiry, then the ask duties.
    pub(crate) fn holder_duties(&mut self) {
        let now = self.stamp();
        let lost: Vec<(String, Lease, bool)> = self
            .conflicts
            .iter()
            .map(|(n, l)| (n.clone(), l.lease.clone(), self.lease_local(l)))
            .collect();
        for (name, l, local) in lost {
            let key = stolen_key(&name, &l.holder, &l.acquired_at);
            if !local || self.refused.contains(&key) {
                continue;
            }
            let held_by = self
                .held
                .get(&name)
                .map(|w| w.lease.holder.clone())
                .unwrap_or_default();
            let r = Refusal {
                resource: name.clone(),
                by: l.holder.clone(),
                held_by,
                held: name.clone(),
                until: l.until,
                at: now,
                conflict: true,
            };
            if self.write(&l.holder, kinds::LEASE_REFUSED, &r).is_ok() {
                self.refused.insert(key);
            }
        }
        self.prune(&now);
        let lapsed = std::mem::take(&mut self.lapsed);
        for l in lapsed {
            if self
                .held
                .get(&l.resource)
                .is_some_and(|c| c.lease.holder == l.holder)
            {
                continue;
            }
            let _ = self.write(
                SWEEPER,
                kinds::LEASE_RELEASED,
                &LeaseReleasedRow {
                    resource: l.resource,
                    holder: l.holder,
                    reason: REASON_EXPIRED.into(),
                    by: SWEEPER.into(),
                    at: now,
                },
            );
        }
        self.lapsed.clear();
        self.ask_duties();
    }

    /// Folds one row another device wrote (Follow's body); a repeat is
    /// skipped.
    pub(crate) fn apply_remote(&mut self, ev: &Event) {
        if ev.seq <= self.last.get(&ev.device_id).copied().unwrap_or(0) {
            return;
        }
        self.last.insert(ev.device_id.clone(), ev.seq);
        self.apply(ev);
        self.holder_duties();
    }

    /// Rebuilds the fold when the log now holds fewer of some device's
    /// rows than were folded (a revoke's cut moved them away).
    pub(crate) fn refold_if_cut(&mut self, vec: &store::Vector) -> Result<()> {
        let cut = self
            .last
            .iter()
            .any(|(dev, seq)| vec.get(dev).copied().unwrap_or(0) < *seq);
        if !cut {
            return Ok(());
        }
        let shorts: Vec<(String, String)> = self
            .tasks
            .iter()
            .map(|(id, t)| (id.clone(), t.short.clone()))
            .collect();
        self.tasks.clear();
        self.order.clear();
        self.next_order = 0;
        self.held.clear();
        self.comments.clear();
        self.by_task.clear();
        self.refusals.clear();
        self.blocker_seq.clear();
        self.step_seq.clear();
        self.activity.clear();
        self.claims.clear();
        self.claimed.clear();
        self.short_groups.clear();
        self.short_idx.clear();
        self.last.clear();
        self.conflicts.clear();
        self.stolen.clear();
        self.lapsed.clear();
        self.ignored = 0;
        self.ignored_by.clear();
        self.asks = super::asks::AskBook::default();
        self.reset_stale();
        self.links.clear();
        self.pins.clear();
        self.replay_store()?;
        for (id, was) in shorts {
            if was.is_empty() {
                continue;
            }
            if let Some(t) = self.tasks.get(&id)
                && t.short != was
            {
                let (to, ti) = (t.short.clone(), id.clone());
                self.short_moved_to(&ti, &was, &to);
            }
        }
        self.note(Change {
            kind: RESYNC.into(),
            ..Change::default()
        });
        Ok(())
    }

    /// After a store resync signal: refold if cut, then page each other
    /// device's rows past what was folded.
    pub(crate) fn catch_up(&mut self) -> Result<()> {
        let Some(st) = self.st.clone() else {
            return Ok(());
        };
        let vec = st.vector()?;
        self.refold_if_cut(&vec)?;
        for (dev, top) in &vec {
            if *dev == self.dev {
                continue;
            }
            loop {
                let after = self.last.get(dev).copied().unwrap_or(0);
                if after >= *top {
                    break;
                }
                let evs = st.list(&store::Filter {
                    device_id: dev.clone(),
                    after_seq: after,
                    kinds: FOLLOW_KINDS.iter().map(|k| k.to_string()).collect(),
                    limit: store::MAX_LIST_LIMIT,
                    ..store::Filter::default()
                })?;
                if evs.is_empty() {
                    break;
                }
                for ev in &evs {
                    self.apply_remote(ev);
                }
                if (evs.len() as i64) < store::MAX_LIST_LIMIT {
                    break;
                }
            }
        }
        self.holder_duties();
        Ok(())
    }
}
