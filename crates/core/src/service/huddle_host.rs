//! Which computer hosts each person's assistant in a huddle (card #1049,
//! docs/design/call-agents.md §7 "Which device hosts it", §8 `call.assist`).
//!
//! One person's assistant never runs in two places. The host is the first
//! device in `AgentDevices.homes` (what `coord.agent_home("")` reads) that
//! isn't paused and isn't long gone. Each device's view of who is online is
//! its own, so two can pick themselves; `call.assist` rows arbitrate:
//! - the host writes `on` when it decides to host and `off` when it stops;
//! - a device that sees another of its person's devices say `on` leaves
//!   it the role (and writes `off` if it had taken it): when both say `on`,
//!   the lower device id keeps it, which every device reads the same, where
//!   the `homes` order is each device's own view and can put each first;
//! - an `on` whose device has been away for over [`AWAY_AFTER`] counts as
//!   off: the roster says "reconnecting" and the next device takes over;
//! - a host that wakes from sleep re-picks before any look.
//!
//! The host opens the held session (card #1046) when its person is in the
//! call, and closes it when they leave, when the call is over, or when the
//! device stops being the host. The chair's session stays open past its
//! person's leaving (card 10); until that lands, it closes with them.
//!
//! The decision is pure ([`verdict`], [`roster_state`]) and tested here;
//! the glue below gathers its facts from the Core and acts on it from a
//! sweep that runs while a huddle needs one.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use coord::talk::{AssistInfo, is_person_key};

use super::Services;
use super::huddle_session::LookError;
use crate::events::{CoreEvent, TalkChange};

/// `logf` with format arguments.
macro_rules! logf {
    ($($t:tt)*) => { crate::logf(format_args!($($t)*)) };
}

/// A device away this long no longer counts as hosting, or as able to.
pub const AWAY_AFTER: Duration = Duration::from_secs(60);
/// A lower-ranked device hosts if the device above it hasn't said `on`
/// this long after its person was seen in the call: an older Irori that
/// ranks first but never hosts would otherwise leave the person without.
pub const TAKE_OVER_AFTER: Duration = Duration::from_secs(15);
/// How often the sweep looks while a huddle needs one.
pub const SWEEP_EVERY: Duration = Duration::from_secs(2);
/// A sweep this much later than expected means the device slept: the pick
/// runs again before anything else.
pub const WAKE_GAP: Duration = Duration::from_secs(20);
/// After a session fails to open, how long before trying again.
pub const RETRY_AFTER: Duration = Duration::from_secs(30);

/// What a device knows when it decides whether to host.
#[derive(Debug, Clone)]
pub struct Facts<'a> {
    /// This device.
    pub me: &'a str,
    /// The person's computers, best first (`AgentDevices.homes`).
    pub homes: &'a [String],
    /// The devices whose agents are paused (Stop here, Stop everywhere).
    pub paused: &'a HashSet<String>,
    /// How long each device that is away has been away.
    pub away_for: &'a BTreeMap<String, Duration>,
    /// The person's devices whose newest `call.assist` row says `on`.
    pub on: &'a HashSet<String>,
    /// This device could run a held session: a runner is installed, Claude
    /// Code answered, and the last open didn't just fail.
    pub can_host: bool,
    /// How long this device has seen its person in the call.
    pub waited: Duration,
}

/// What this device does about hosting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Host: write `on` and hold a session.
    Host,
    /// Another device hosts (or should): write nothing, hold nothing.
    Stand(String),
    /// A device above hasn't said `on` yet: wait for it.
    Wait,
    /// This device can't host.
    Out,
}

fn gone(f: &Facts, d: &str) -> bool {
    f.away_for.get(d).is_some_and(|t| *t >= AWAY_AFTER)
}

/// Whether `d` could host now: not paused, not away for good.
fn usable(f: &Facts, d: &str) -> bool {
    !f.paused.contains(d) && !gone(f, d)
}

/// The first device that qualifies to host, in `homes` order, or None when
/// no computer is on (a person on a phone with their computer off).
pub fn first_usable(
    homes: &[String],
    paused: &HashSet<String>,
    away_for: &BTreeMap<String, Duration>,
) -> Option<String> {
    homes
        .iter()
        .find(|d| !paused.contains(*d) && !away_for.get(*d).is_some_and(|t| *t >= AWAY_AFTER))
        .cloned()
}

/// Whether this device hosts its person's assistant for a huddle its person
/// is in.
///
/// `homes` is this device's own ranking (it puts itself first while it
/// sees the others only as recent), so it picks who *starts* hosting; the
/// rows decide who *keeps* it: a usable device that says `on` holds the
/// role, and if two say `on` the lower device id does. A device that
/// returns above the host doesn't take the role back.
pub fn verdict(f: &Facts) -> Verdict {
    let Some(rank) = f.homes.iter().position(|d| d == f.me) else {
        return Verdict::Out;
    };
    if !f.can_host || f.paused.contains(f.me) {
        return Verdict::Out;
    }
    let holder =
        f.on.iter()
            .filter(|d| d.as_str() != f.me && usable(f, d))
            // A device that isn't one of our homes can't be told from a stale
            // row of an old install: only the person's computers hold it.
            .filter(|d| f.homes.contains(d))
            .min();
    if f.on.contains(f.me) {
        // Hosting: the lower id of those who say `on` keeps it.
        return match holder {
            Some(d) if d.as_str() < f.me => Verdict::Stand(d.clone()),
            _ => Verdict::Host,
        };
    }
    if let Some(d) = holder {
        return Verdict::Stand(d.clone());
    }
    let above = f.homes[..rank].iter().filter(|d| usable(f, d)).count();
    if above == 0 || f.waited >= TAKE_OVER_AFTER {
        Verdict::Host
    } else {
        Verdict::Wait
    }
}

/// How a person's assistant reads in the roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RosterState {
    /// A device hosts it.
    Connected,
    /// Its host has been away: the next device takes over.
    Reconnecting,
    /// No device hosts it.
    NotConnected,
}

impl RosterState {
    pub fn as_str(self) -> &'static str {
        match self {
            RosterState::Connected => "connected",
            RosterState::Reconnecting => "reconnecting",
            RosterState::NotConnected => "not connected",
        }
    }
}

/// A person's assistant's state from its devices' newest rows: `on` rows
/// whose device is here (or away under [`AWAY_AFTER`]) mean connected;
/// `on` rows all gone mean reconnecting; else not connected.
pub fn roster_state(rows: &[AssistInfo], away_for: &BTreeMap<String, Duration>) -> RosterState {
    let on: Vec<&AssistInfo> = rows.iter().filter(|r| r.on).collect();
    if on.is_empty() {
        return RosterState::NotConnected;
    }
    let here = |r: &&AssistInfo| !away_for.get(&r.device).is_some_and(|t| *t >= AWAY_AFTER);
    if on.iter().any(here) {
        RosterState::Connected
    } else {
        RosterState::Reconnecting
    }
}

/// Whether a failed turn's text says Claude Code isn't signed in.
pub fn looks_signed_out(text: &str) -> bool {
    let t = text.to_lowercase();
    [
        "not logged in",
        "/login",
        "invalid api key",
        "authentication",
        "oauth",
        "sign in",
        "signed out",
        "log in",
        "credentials",
    ]
    .iter()
    .any(|k| t.contains(k))
}

/// One person's assistant in a huddle, as the roster shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantState {
    /// The person, in this device's key form.
    pub person: String,
    /// `connected`, `reconnecting` or `not connected`.
    pub state: String,
    /// The device hosting it, when connected.
    pub device: String,
    /// Why it isn't connected, when this device knows.
    pub why: String,
}

/// The hosting state on [`Services`].
#[derive(Default)]
pub(crate) struct Hosting {
    /// A sweep is running (or about to).
    ticking: AtomicBool,
    /// Huddles this device's person is in, with what this device did about
    /// each.
    calls: Mutex<HashMap<String, Hosted>>,
    /// When each device not our own was first seen unreachable.
    unreach: Mutex<HashMap<String, SystemTime>>,
    /// When the last sweep ran, to see a sleep.
    last: Mutex<Option<SystemTime>>,
    /// When this device first saw each huddle over (card #1053): the clock
    /// of the chair's hand-on and of the end it owes.
    pub(super) over: Mutex<HashMap<String, SystemTime>>,
    /// How long a huddle runs before its chair proposes a title, in
    /// milliseconds; 0 is the default (tests shorten it).
    pub(super) title_after_ms: std::sync::atomic::AtomicU64,
}

pub(super) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One huddle this device's person is in.
#[derive(Debug, Clone)]
struct Hosted {
    conv: String,
    /// First seen with the person in it.
    since: SystemTime,
    /// This device wrote `on` and not `off` since.
    wrote_on: bool,
    /// Claude Code is signed out here: not tried again this call.
    signed_out: Option<String>,
    /// A session failed to open: not tried again until then.
    retry_at: Option<SystemTime>,
}

impl Services {
    /// Starts the sweep if it isn't running: it runs while this device's
    /// person is in a huddle, or while this device holds one for them.
    pub(crate) fn huddle_host_kick(self: &std::sync::Arc<Self>) {
        if self.hosting.ticking.swap(true, Ordering::SeqCst) {
            return;
        }
        let w = std::sync::Arc::downgrade(self);
        self.pairing.handle.spawn(async move {
            loop {
                let Some(s) = w.upgrade() else { return };
                let s2 = s.clone();
                let keep = tokio::task::spawn_blocking(move || s2.huddle_host_sweep())
                    .await
                    .unwrap_or(false);
                if !keep {
                    s.hosting.ticking.store(false, Ordering::SeqCst);
                    // A frame may have landed after the sweep saw none.
                    if !s.calls_with_person(&s.person_id()).is_empty()
                        && !s.hosting.ticking.swap(true, Ordering::SeqCst)
                    {
                        continue;
                    }
                    return;
                }
                drop(s);
                tokio::time::sleep(SWEEP_EVERY).await;
            }
        });
    }

    /// One look at every huddle this device's person is in, and every one
    /// this device holds for them: host, stand down or close as the facts
    /// say. True while anything remains to watch.
    pub(crate) fn huddle_host_sweep(&self) -> bool {
        let now = SystemTime::now();
        let woke = {
            let mut last = lock(&self.hosting.last);
            let woke = last.is_some_and(|t| now.duration_since(t).unwrap_or_default() > WAKE_GAP);
            *last = Some(now);
            woke
        };
        if woke {
            // Everyone looks away after our own sleep: measure again.
            lock(&self.hosting.unreach).clear();
            logf!("huddle host: woke from sleep: picking again");
        }
        let me = self.person_id();
        let mut wanted: BTreeMap<String, String> = BTreeMap::new();
        for (call, conv, _space) in self.calls_with_person(&me) {
            wanted.insert(call, conv);
        }
        let known: Vec<String> = lock(&self.hosting.calls).keys().cloned().collect();
        for call in known {
            if !wanted.contains_key(&call) {
                self.huddle_host_call(&call, None, now);
            }
        }
        for (call, conv) in &wanted {
            self.huddle_host_call(call, Some(conv), now);
        }
        !wanted.is_empty() || !lock(&self.hosting.calls).is_empty()
    }

    /// Whether this device hosts `call` (it wrote `on`), as the last sweep
    /// left it: cheap, for each line. The
    /// full pick runs before each look ([`Services::huddle_hosting`]).
    pub(crate) fn huddle_in(&self, call: &str) -> bool {
        lock(&self.hosting.calls)
            .get(call)
            .is_some_and(|h| h.wrote_on && h.signed_out.is_none())
    }

    /// Runs the pick for `call` now, and says whether this device hosts it:
    /// what a look asks before it runs (a host that woke from sleep finds
    /// out here that another took over).
    pub(crate) fn huddle_hosting(&self, call: &str) -> bool {
        let conv = lock(&self.hosting.calls).get(call).map(|h| h.conv.clone());
        let Some(conv) = conv else {
            return false;
        };
        let me = self.person_id();
        let in_it = self
            .calls_with_person(&me)
            .iter()
            .any(|(id, _, _)| id == call);
        self.huddle_host_call(call, in_it.then_some(conv.as_str()), SystemTime::now());
        lock(&self.hosting.calls)
            .get(call)
            .is_some_and(|h| h.wrote_on && h.signed_out.is_none())
    }

    /// The pick for one huddle. `conv` is None when the person isn't in it
    /// (any more): this device then stops what it holds for them.
    fn huddle_host_call(&self, call: &str, conv: Option<&str>, now: SystemTime) {
        let mut h = match (lock(&self.hosting.calls).get(call).cloned(), conv) {
            (Some(h), _) => h,
            (None, Some(c)) => Hosted {
                conv: c.to_string(),
                since: now,
                wrote_on: false,
                signed_out: None,
                retry_at: None,
            },
            (None, None) => return,
        };
        let want = conv.is_some() && !self.call_over(call);
        let mine_on = self.assist_on_here(call);
        // A host whose person left, or whose huddle ended, stays while it
        // owes the end or may have to cover for the chair (card #1053).
        let linger = !want && h.wrote_on && self.huddle_lingers(call, now);
        let v = if want || linger {
            self.huddle_verdict(call, &h, now)
        } else {
            Verdict::Out
        };
        match v {
            Verdict::Host => {
                if !h.wrote_on {
                    match self.write_assist(&h.conv, call, true) {
                        Ok(()) => {
                            h.wrote_on = true;
                            logf!("huddle {call}: this device hosts the assistant");
                            // The old host's placeholders still saying
                            // "looking" never get an answer: close them.
                            self.huddle_close_stale_placeholders(call);
                        }
                        Err(e) => {
                            logf!("huddle {call}: assist on: {e}");
                            lock(&self.hosting.calls).insert(call.to_string(), h);
                            return;
                        }
                    }
                }
                if !self.huddle_is_open(call) {
                    let seed = self.huddle_seed(call, &h.conv);
                    if let Err(e) = self.huddle_open(&seed) {
                        logf!("huddle {call}: the assistant didn't connect: {e}");
                        if looks_signed_out(&e) {
                            h.signed_out = Some(e);
                        } else {
                            h.retry_at = Some(now + RETRY_AFTER);
                        }
                    } else {
                        h.retry_at = None;
                    }
                }
                let conv = h.conv.clone();
                lock(&self.hosting.calls).insert(call.to_string(), h);
                // The chair from the rows, the title, and the end (#1053).
                self.huddle_chair_row(call, &conv);
                self.huddle_title_tick(call, now);
                self.huddle_end_tick(call, now);
                self.hub
                    .publish(CoreEvent::Talk(TalkChange::of([conv.as_str()], [call])));
            }
            Verdict::Wait => {
                lock(&self.hosting.calls).insert(call.to_string(), h);
            }
            Verdict::Stand(_) | Verdict::Out => {
                let had = h.wrote_on || mine_on || self.huddle_is_open(call);
                if h.wrote_on || mine_on {
                    match self.write_assist(&h.conv, call, false) {
                        Ok(()) => h.wrote_on = false,
                        Err(e) => logf!("huddle {call}: assist off: {e}"),
                    }
                }
                if self.huddle_is_open(call) || had {
                    self.huddle_close(call);
                }
                if had {
                    logf!("huddle {call}: this device no longer hosts the assistant");
                    self.hub
                        .publish(CoreEvent::Talk(TalkChange::of([h.conv.as_str()], [call])));
                }
                if want || h.wrote_on {
                    // Still in the call (or the off row didn't write):
                    // keep what we know (a signed-out Claude Code, a retry
                    // time) for the next sweep.
                    lock(&self.hosting.calls).insert(call.to_string(), h);
                } else {
                    // The person left, or the call is over: nothing to
                    // keep, and a rejoin starts clean.
                    lock(&self.hosting.calls).remove(call);
                    self.huddle_feed_forget(call);
                }
            }
        }
    }

    /// Whether this device's newest row for `call` says `on`.
    fn assist_on_here(&self, call: &str) -> bool {
        self.talk
            .call_assists(call)
            .iter()
            .any(|a| a.on && a.device == self.device.id && is_person_key(&a.by))
    }

    fn huddle_verdict(&self, call: &str, h: &Hosted, now: SystemTime) -> Verdict {
        let agents = self
            .spawn
            .agents
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let paused: HashSet<String> = agents
            .homes
            .iter()
            .filter(|d| self.agents_paused_on(d))
            .cloned()
            .collect();
        let rows: Vec<AssistInfo> = self
            .call_assists_now(call, "")
            .into_iter()
            .filter(|a| is_person_key(&a.by))
            .collect();
        let on: HashSet<String> = rows
            .iter()
            .filter(|a| a.on)
            .map(|a| a.device.clone())
            .collect();
        let away_for = self.own_away_for(&agents);
        let can_host = self.huddle_runner_installed()
            && h.signed_out.is_none()
            && h.retry_at.is_none_or(|t| now >= t);
        verdict(&Facts {
            me: &self.device.id,
            homes: &agents.homes,
            paused: &paused,
            away_for: &away_for,
            on: &on,
            can_host,
            waited: now.duration_since(h.since).unwrap_or_default(),
        })
    }

    /// How long each of this person's devices that is away has been away.
    pub(super) fn own_away_for(&self, agents: &coord::AgentDevices) -> BTreeMap<String, Duration> {
        let now = (self.now)().unix_nano();
        agents
            .away
            .iter()
            .map(|d| {
                let since = agents.away_since.get(d).copied().unwrap_or(0);
                // No time recorded: away from before we looked.
                let secs = if since > 0 {
                    (now - since).max(0)
                } else {
                    i64::MAX
                };
                (d.clone(), Duration::from_nanos(secs as u64))
            })
            .collect()
    }

    /// Writes this device's `call.assist` row for `call`.
    fn write_assist(&self, conv: &str, call: &str, on: bool) -> Result<(), String> {
        self.write_assist_as(conv, call, on, false)
    }

    /// [`Services::write_assist`] with the chair flag; the chair switch rides
    /// every row (card #1053).
    pub(super) fn write_assist_as(
        &self,
        conv: &str,
        call: &str,
        on: bool,
        chair: bool,
    ) -> Result<(), String> {
        self.talk
            .write_assist_as(
                conv,
                call,
                &self.device.id,
                on,
                chair,
                !self.huddle_chair_on(),
                &self.user_sender().key,
            )
            .map_err(|e| e.to_string())?;
        self.hub
            .publish(CoreEvent::Talk(TalkChange::of([conv], [call])));
        Ok(())
    }

    /// A failed turn on `call`'s session: if Claude Code isn't signed in,
    /// this device stops hosting (the sweep writes `off`) and the roster
    /// says "not connected" (card #1049). Called from the held session's
    /// open and look.
    pub(crate) fn huddle_host_note(&self, call: &str, e: &LookError) {
        let LookError::Failed(text) = e else {
            return;
        };
        if !looks_signed_out(text) {
            return;
        }
        let mut calls = lock(&self.hosting.calls);
        if let Some(h) = calls.get_mut(call) {
            logf!("huddle {call}: Claude Code is signed out here: not hosting");
            h.signed_out = Some(text.clone());
        }
    }

    /// Whether a runner is installed here (a desktop with Claude Code).
    fn huddle_runner_installed(&self) -> bool {
        self.huddle_runner().is_ok()
    }

    /// Each person's assistant in huddle `call`, as the roster shows it:
    /// everyone in the call, and everyone with an assistant row. Empty for a
    /// huddle that is over.
    pub(crate) fn call_assistants(&self, call: &str) -> Vec<AssistantState> {
        if self.call_over(call) {
            return Vec::new();
        }
        let me_key = self.user_sender().key;
        let norm = |k: &str| {
            if is_person_key(k) {
                me_key.clone()
            } else {
                k.to_string()
            }
        };
        let (live_people, space) = self.call_people_now(call);
        let mut people: Vec<String> = Vec::new();
        let mut add = |k: String| {
            if !people.contains(&k) {
                people.push(k);
            }
        };
        let me = self.person_id();
        for p in &live_people {
            add(if *p == me {
                me_key.clone()
            } else {
                format!("{}{p}", store::kinds::PERSON_ACTOR_PREFIX)
            });
        }
        let rows = self.call_assists_now(call, &space);
        for r in &rows {
            add(norm(&r.by));
        }
        let agents = self
            .spawn
            .agents
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut away = self.own_away_for(&agents);
        self.others_away_for(&space, &rows, &mut away);
        let mine_why = {
            let paused: HashSet<String> = agents
                .homes
                .iter()
                .filter(|d| self.agents_paused_on(d))
                .cloned()
                .collect();
            let signed_out = lock(&self.hosting.calls)
                .get(call)
                .is_some_and(|h| h.signed_out.is_some());
            if signed_out {
                "Claude Code isn't signed in on this computer"
            } else if first_usable(&agents.homes, &paused, &away).is_none() {
                "needs a computer that's on"
            } else {
                ""
            }
        };
        people
            .into_iter()
            .map(|p| {
                let mine = p == me_key;
                let theirs: Vec<AssistInfo> =
                    rows.iter().filter(|r| norm(&r.by) == p).cloned().collect();
                let state = roster_state(&theirs, &away);
                let device = if state == RosterState::Connected {
                    theirs
                        .iter()
                        .find(|r| r.on && !away.get(&r.device).is_some_and(|t| *t >= AWAY_AFTER))
                        .map(|r| r.device.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let why = if state == RosterState::NotConnected && mine {
                    mine_why.to_string()
                } else {
                    String::new()
                };
                AssistantState {
                    person: p,
                    state: state.as_str().into(),
                    device,
                    why,
                }
            })
            .collect()
    }

    /// `call`'s `call.assist` rows, a removed device's read as off (card
    /// #1240): this person's by its chain, another's by `space`'s board.
    pub(super) fn call_assists_now(&self, call: &str, space: &str) -> Vec<AssistInfo> {
        let board = if space.is_empty() {
            None
        } else {
            self.mesh.board(space)
        };
        off_if_removed(self.talk.call_assists(call), |r| {
            if is_person_key(&r.by) {
                self.chain.cut(&r.device).is_some()
            } else {
                board
                    .as_ref()
                    .is_some_and(|m| m.verifier().cut(&r.device).is_some())
            }
        })
    }

    /// Adds to `away` how long devices that aren't this person's have been
    /// unreachable in `space`, for the `on` rows of other people (the
    /// first sighting starts the clock).
    pub(super) fn others_away_for(
        &self,
        space: &str,
        rows: &[AssistInfo],
        away: &mut BTreeMap<String, Duration>,
    ) {
        let Some(live) = self.space_devices_live(space) else {
            return;
        };
        let now = SystemTime::now();
        let mut un = lock(&self.hosting.unreach);
        for r in rows.iter().filter(|r| !is_person_key(&r.by)) {
            if r.device == self.device.id || live.contains(&r.device) {
                un.remove(&r.device);
                continue;
            }
            let t = *un.entry(r.device.clone()).or_insert(now);
            away.insert(r.device.clone(), now.duration_since(t).unwrap_or_default());
        }
    }
}

/// `rows` with each row from a `removed` device off, so no one reads as
/// connected through it (card #1240). The rows stay: who has a row, and
/// when each first said `on`, don't change.
fn off_if_removed(
    mut rows: Vec<AssistInfo>,
    removed: impl Fn(&AssistInfo) -> bool,
) -> Vec<AssistInfo> {
    for r in &mut rows {
        if r.on && removed(r) {
            r.on = false;
            r.chair = false;
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_removed_devices_assist_row_reads_off() {
        let row = |device: &str| AssistInfo {
            device: device.into(),
            by: "user:pc".into(),
            on: true,
            chair: true,
            declines_chair: false,
            first_on: 5,
        };
        let rows = off_if_removed(vec![row("pc"), row("old")], |r| r.device == "old");
        assert!(rows[0].on && rows[0].chair);
        assert!(!rows[1].on && !rows[1].chair);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].first_on, 5);
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|d| d.to_string()).collect()
    }

    struct T {
        homes: Vec<String>,
        paused: HashSet<String>,
        away: BTreeMap<String, Duration>,
        on: HashSet<String>,
        can: bool,
        waited: u64,
    }

    fn t(homes: &[&str]) -> T {
        T {
            homes: s(homes),
            paused: HashSet::new(),
            away: BTreeMap::new(),
            on: HashSet::new(),
            can: true,
            waited: 0,
        }
    }

    impl T {
        fn paused(mut self, d: &str) -> T {
            self.paused.insert(d.into());
            self
        }
        fn away(mut self, d: &str, secs: u64) -> T {
            self.away.insert(d.into(), Duration::from_secs(secs));
            self
        }
        fn on(mut self, d: &str) -> T {
            self.on.insert(d.into());
            self
        }
        fn waited(mut self, secs: u64) -> T {
            self.waited = secs;
            self
        }
        fn v(&self, me: &str) -> Verdict {
            verdict(&Facts {
                me,
                homes: &self.homes,
                paused: &self.paused,
                away_for: &self.away,
                on: &self.on,
                can_host: self.can,
                waited: Duration::from_secs(self.waited),
            })
        }
    }

    #[test]
    fn the_first_home_hosts_and_the_second_waits_for_it() {
        let x = t(&["mac", "pc"]);
        assert_eq!(x.v("mac"), Verdict::Host);
        assert_eq!(x.v("pc"), Verdict::Wait);
    }

    #[test]
    fn a_device_that_sees_another_say_on_leaves_it_the_role() {
        let x = t(&["mac", "pc"]).on("mac");
        assert_eq!(x.v("pc"), Verdict::Stand("mac".into()));
        assert_eq!(x.v("mac"), Verdict::Host);
        // Even one it ranks above: it doesn't take the role back.
        let x = t(&["mac", "pc"]).on("pc");
        assert_eq!(x.v("mac"), Verdict::Stand("pc".into()));
        assert_eq!(x.v("pc"), Verdict::Host);
    }

    #[test]
    fn both_picking_themselves_end_with_the_lower_id() {
        // Each ranks itself first and wrote `on`: in both views the lower
        // id keeps it, whatever the order of `homes`.
        let one = t(&["pc", "mac"]).on("mac").on("pc");
        let two = t(&["mac", "pc"]).on("mac").on("pc");
        for x in [&one, &two] {
            assert_eq!(x.v("mac"), Verdict::Host);
            assert_eq!(x.v("pc"), Verdict::Stand("mac".into()));
        }
    }

    #[test]
    fn a_paused_device_hosts_nothing_and_does_not_hold_the_next() {
        let x = t(&["mac", "pc"]).paused("mac");
        assert_eq!(x.v("mac"), Verdict::Out);
        assert_eq!(x.v("pc"), Verdict::Host);
        // Even if its `on` is still in the rows.
        let x = t(&["mac", "pc"]).paused("mac").on("mac");
        assert_eq!(x.v("pc"), Verdict::Host);
    }

    #[test]
    fn a_host_away_for_a_minute_hands_over_but_a_blip_does_not() {
        let blip = t(&["mac", "pc"]).on("mac").away("mac", 59);
        assert_eq!(blip.v("pc"), Verdict::Stand("mac".into()));
        let gone = t(&["mac", "pc"]).on("mac").away("mac", 60);
        assert_eq!(gone.v("pc"), Verdict::Host);
    }

    #[test]
    fn a_first_home_that_never_hosts_is_taken_over_after_a_wait() {
        let x = t(&["old", "pc"]).waited(14);
        assert_eq!(x.v("pc"), Verdict::Wait);
        let x = t(&["old", "pc"]).waited(15);
        assert_eq!(x.v("pc"), Verdict::Host);
    }

    #[test]
    fn a_row_of_a_device_that_is_no_home_holds_nothing() {
        let x = t(&["pc"]).on("old-install");
        assert_eq!(x.v("pc"), Verdict::Host);
    }

    #[test]
    fn a_device_that_cannot_host_is_out_and_the_next_one_can() {
        let mut x = t(&["mac", "pc"]);
        x.can = false;
        assert_eq!(x.v("mac"), Verdict::Out);
        // A phone is in no home.
        assert_eq!(t(&["mac"]).v("phone"), Verdict::Out);
    }

    fn row(device: &str, on: bool) -> AssistInfo {
        AssistInfo {
            device: device.into(),
            by: "user:x".into(),
            on,
            chair: false,
            declines_chair: false,
            first_on: 0,
        }
    }

    #[test]
    fn the_roster_reads_connected_reconnecting_or_not_connected() {
        let none = BTreeMap::new();
        assert_eq!(roster_state(&[], &none), RosterState::NotConnected);
        assert_eq!(
            roster_state(&[row("mac", false)], &none),
            RosterState::NotConnected
        );
        assert_eq!(
            roster_state(&[row("mac", true)], &none),
            RosterState::Connected
        );
        let mut away = BTreeMap::new();
        away.insert("mac".to_string(), Duration::from_secs(61));
        assert_eq!(
            roster_state(&[row("mac", true)], &away),
            RosterState::Reconnecting
        );
        // The next device took over: connected again.
        assert_eq!(
            roster_state(&[row("mac", true), row("pc", true)], &away),
            RosterState::Connected
        );
    }

    #[test]
    fn no_computer_on_means_no_host() {
        let homes = s(&["mac"]);
        let mut away = BTreeMap::new();
        assert_eq!(
            first_usable(&homes, &HashSet::new(), &away).as_deref(),
            Some("mac")
        );
        away.insert("mac".to_string(), Duration::from_secs(90));
        assert_eq!(first_usable(&homes, &HashSet::new(), &away), None);
        assert_eq!(first_usable(&[], &HashSet::new(), &BTreeMap::new()), None);
    }

    #[test]
    fn a_signed_out_claude_is_told_from_other_failures() {
        assert!(looks_signed_out("Not logged in \u{b7} Please run /login"));
        assert!(looks_signed_out("Invalid API key"));
        assert!(!looks_signed_out("the look timed out after 60 s"));
    }
}
