//! Device liveness and status (mesh.md §1.2, §3.1 "Devices warnings";
//! I:internal/mesh/node.go:1028-1160, devices.go). Status and the device
//! rows are kept in memory and never logged. Their JSON is Go's
//! (`/v1/devices`): field order, `omitempty`/`omitzero`, durations as
//! integer nanoseconds and times as RFC3339Nano.

use std::collections::BTreeMap;
use std::time::SystemTime;

use gocompat::time::GoTime;
use identity::{DeviceRevoked, objects::decode};
use serde::Serialize;
use store::{Event, Filter, MAX_LIST_LIMIT};

use crate::node::{
    CONNECTED_WITHIN, Node, PeerInfo, RECENT_WITHIN, RowsHeard, STATE_ERROR, STATE_ON,
    STATE_REMOVED, within,
};

fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn go_time(t: Option<SystemTime>) -> Option<GoTime> {
    t.map(GoTime::from_system)
}

/// One other device as the mesh sees it (Go `PeerState`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PeerState {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub label: String,
    pub role: String,
    /// connected, recent, offline.
    pub state: String,
    #[serde(skip_serializing_if = "is_false")]
    pub revoked: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub epoch: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// Nanoseconds (Go `time.Duration`).
    #[serde(skip_serializing_if = "is_zero")]
    pub rtt: i64,
    #[serde(rename = "lastPing", skip_serializing_if = "Option::is_none")]
    pub last_ping: Option<GoTime>,
    #[serde(rename = "lastSync", skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<GoTime>,
    /// When its newest row here was written, however it came (card #544).
    #[serde(rename = "lastRows", skip_serializing_if = "Option::is_none")]
    pub last_rows: Option<GoTime>,
    /// The device its rows come through while there is no session with it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub via: String,
    /// Nanoseconds.
    #[serde(skip_serializing_if = "is_zero")]
    pub skew: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub refused: i64,
    /// Its rows the folds ignored.
    #[serde(skip_serializing_if = "is_zero")]
    pub ignored: i64,
    /// (A removed device) how many of the remaining devices, this one
    /// included, have seen its removal, of `remaining`.
    #[serde(skip_serializing_if = "is_zero")]
    pub pending: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub remaining: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub held: i64,
    /// Why its stream stopped here.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stopped: String,
    /// Its rows wait for wall time.
    #[serde(skip_serializing_if = "is_false")]
    pub ahead: bool,
    /// Its schema is too old to sync.
    #[serde(skip_serializing_if = "is_false")]
    pub old: bool,
}

/// The net layer's state and every other certified device (Go `Status`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Status {
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    pub epoch: i64,
    pub topics: i64,
    /// Own rows coming back from this device.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub restoring: String,
    /// Go's nil slice is `null`.
    pub peers: Option<Vec<PeerState>>,
}

impl Status {
    /// The peers (empty for none).
    pub fn peer_list(&self) -> &[PeerState] {
        self.peers.as_deref().unwrap_or(&[])
    }
}

/// One of the person's devices as Settings → Devices and `/v1/devices`
/// show it (Go `DeviceRow`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DeviceRow {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub name: String,
    pub role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub os: String,
    #[serde(skip_serializing_if = "is_false")]
    pub this: bool,
    /// connected, recent or offline (this device: "this").
    pub state: String,
    #[serde(skip_serializing_if = "is_false")]
    pub revoked: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub warning: String,
    /// "reset" (this device) or "remove" (another one).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub action: String,
    #[serde(rename = "lastSeen", skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<GoTime>,
    /// The device id its rows come through (`PeerState::via`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub via: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<PeerState>,
}

/// One device the private log syncs with, and when this device was last in
/// step with it (card #580).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fresh {
    pub device_id: String,
    pub at: Option<SystemTime>,
}

/// How another device reads from here (card #544): the one rule behind
/// `status`, `liveness` and `last_seen`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceRead {
    /// connected, recent or offline. Recent is not a session: fetching an
    /// attachment still needs a connected holder. Read this, not
    /// `last_ping` or `last_sync` alone.
    pub state: &'static str,
    /// When it was last heard: a session with it, or the newest row of its
    /// that reached this device. `None`: never.
    pub seen: Option<SystemTime>,
    /// The device id its rows come through, while it reads recent with no
    /// session of its own; "" otherwise.
    pub via: String,
}

/// "connected" when a ping arrived within 45 s on a live connection;
/// "recent" when it synced, or wrote a row that reached this device, within
/// 10 minutes; else "offline". A removed device's rows count for nothing.
fn read_of(now: SystemTime, k: &PeerInfo, rows: &RowsHeard, up: bool, revoked: bool) -> DeviceRead {
    let at = if revoked { None } else { rows.at };
    let seen = [k.last_ping, k.last_sync, at].into_iter().flatten().max();
    if up
        && k.last_ping
            .is_some_and(|t| within(now, t, CONNECTED_WITHIN))
    {
        return DeviceRead {
            state: "connected",
            seen,
            via: String::new(),
        };
    }
    let fresh = |t: Option<SystemTime>| t.is_some_and(|t| within(now, t, RECENT_WITHIN));
    let state = if fresh(k.last_sync) || fresh(at) {
        "recent"
    } else {
        "offline"
    };
    // A session newer than the rows says more than they do.
    let relayed = !up && fresh(at) && at > k.last_sync;
    DeviceRead {
        state,
        seen,
        via: if relayed {
            rows.via.clone()
        } else {
            String::new()
        },
    }
}

impl Node {
    /// Go `Status`.
    pub fn status(&self) -> Status {
        let now = self.now();
        let sessions = self.session_list();
        let (state, detail, topics, restoring, restore_to) = self.status_parts();
        let mut st = Status {
            state,
            detail,
            epoch: self.o.ident.current().epoch,
            topics: topics as i64,
            restoring,
            peers: None,
        };
        if st.state == STATE_ON && st.topics == 0 && !st.detail.is_empty() {
            st.state = STATE_ERROR.into();
        }
        let devices = self.o.chain.devices();
        for d in &devices {
            self.seed_rows(&d.cert.device_id);
        }
        let (known, rows, ahead) = {
            let g = self.lock();
            let ahead = g
                .ahead
                .iter()
                .filter(|(_, t)| within(now, **t, 2 * self.iv.vector))
                .map(|(d, _)| (d.clone(), true))
                .collect::<BTreeMap<String, bool>>();
            (g.known.clone(), g.rows.clone(), ahead)
        };
        let stopped = self.stopped_map();
        let ignored = self.o.ignored.as_ref().map(|f| f()).unwrap_or_default();
        if !st.restoring.is_empty()
            && let Ok(v) = self.o.store.vector()
            && restore_to <= v.get(&self.self_id).copied().unwrap_or(0)
        {
            st.restoring.clear();
        }
        let mut peers = Vec::new();
        for d in devices {
            let id = d.cert.device_id.clone();
            if id == self.self_id {
                continue;
            }
            let k = known.get(&id).cloned().unwrap_or_default();
            let heard = rows.get(&id).cloned().unwrap_or_default();
            let mut ps = PeerState {
                device_id: id.clone(),
                label: d.cert.label.clone(),
                role: d.cert.role.clone(),
                revoked: d.revoked,
                epoch: k.epoch,
                last_ping: go_time(k.last_ping),
                last_sync: go_time(k.last_sync),
                last_rows: go_time(heard.at.filter(|_| !d.revoked)),
                skew: k.skew,
                refused: k.refused,
                stopped: stopped.get(&id).cloned().unwrap_or_default(),
                ahead: ahead.contains_key(&id),
                old: k.schema_old,
                state: "offline".into(),
                ignored: ignored.get(&id).copied().unwrap_or(0),
                ..PeerState::default()
            };
            if d.revoked {
                (ps.pending, ps.remaining) = self.removal_seen(&id, &known);
            }
            for s in &sessions {
                if s.conn.id() == id && s.hello_done() {
                    ps.path = s.conn.path();
                    ps.rtt = s.conn.rtt().as_nanos() as i64;
                    ps.held += s.held_count() as i64;
                }
            }
            let p = read_of(now, &k, &heard, !ps.path.is_empty(), d.revoked);
            ps.state = p.state.into();
            ps.via = p.via;
            peers.push(ps);
        }
        if !peers.is_empty() {
            st.peers = Some(peers);
        }
        st
    }

    /// Device `dev` as this one reads it ([`read_of`]).
    pub fn device_read(&self, dev: &str) -> DeviceRead {
        self.seed_rows(dev);
        let now = self.now();
        let up = self.connected_to(dev);
        let revoked = self.o.chain.cut(dev).is_some();
        let (k, rows) = {
            let g = self.lock();
            (
                g.known.get(dev).cloned().unwrap_or_default(),
                g.rows.get(dev).cloned().unwrap_or_default(),
            )
        };
        read_of(now, &k, &rows, up, revoked)
    }

    /// Go `Liveness`: [`Node::device_read`]'s state.
    pub fn liveness(&self, dev: &str) -> &'static str {
        self.device_read(dev).state
    }

    /// When another of the person's devices was last heard, as Settings →
    /// Devices shows it; `None` never.
    pub fn last_seen(&self, dev: &str) -> Option<SystemTime> {
        self.device_read(dev).seen
    }

    /// How fresh the private log is (card #580, no-server.md §4.3): for
    /// each of the person's other devices, when this one was last in step
    /// with it. Connected and holding all it has said it has: now. Else the
    /// last time that was true while Irori ran, or, after a restart, when
    /// it wrote the newest row of its that this device holds (it was in
    /// step at least that recently). `None`: never, as far as this device
    /// knows. Removed devices are left out.
    pub fn freshness(&self) -> Vec<Fresh> {
        let now = self.now();
        let ours = self.o.store.vector().unwrap_or_default();
        let mut out = Vec::new();
        for p in self.status().peer_list() {
            if p.revoked {
                continue;
            }
            let dev = p.device_id.clone();
            self.seed_rows(&dev);
            let up = self.connected_to(&dev);
            let (k, rows) = {
                let g = self.lock();
                (
                    g.known.get(&dev).cloned().unwrap_or_default(),
                    g.rows.get(&dev).cloned().unwrap_or_default(),
                )
            };
            let live = up && crate::node::covers(&ours, &k.vec);
            let at = if live {
                Some(now)
            } else {
                [k.in_step, rows.at.filter(|_| rows.via.is_empty())]
                    .into_iter()
                    .flatten()
                    .max()
            };
            out.push(Fresh { device_id: dev, at });
        }
        out
    }

    /// Go `Devices`: the person's devices, this one first; `detail` fills
    /// each other device's `detail`.
    pub fn devices(&self, detail: bool) -> Vec<DeviceRow> {
        let st = self.status();
        let me_dev = self.o.ident.self_device();
        let mut me = DeviceRow {
            device_id: me_dev.id.clone(),
            name: me_dev.name.clone(),
            role: identity::objects::ROLE_DESKTOP.into(),
            os: me_dev.os.clone(),
            this: true,
            state: "this".into(),
            ..DeviceRow::default()
        };
        if let Some(cert) = self.o.chain.cert(&me_dev.id)
            && !cert.role.is_empty()
        {
            me.role = cert.role;
        }
        if st.state == STATE_REMOVED {
            me.warning = "Removed from your devices. Reset this device to pair it again.".into();
            me.action = "reset".into();
            let by = self.revoked_by(&me_dev.id);
            if !by.is_empty() {
                me.warning = format!(
                    "Removed from your devices by {by}. Reset this device to pair it again."
                );
            }
        } else if self.is_stopped(&me_dev.id) {
            me.warning =
                "This device signed two different histories (restored from a copy?). Reset it."
                    .into();
            me.action = "reset".into();
        } else if !st.restoring.is_empty() {
            me.warning = "Restoring this device's rows from another device.".into();
        } else if st.state == STATE_ERROR && !st.detail.is_empty() {
            me.warning = st.detail.clone();
        }
        let mut out = vec![me];
        for p in st.peer_list() {
            let mut r = DeviceRow {
                device_id: p.device_id.clone(),
                name: p.label.clone(),
                role: p.role.clone(),
                os: self.device_os(&p.device_id),
                state: p.state.clone(),
                revoked: p.revoked,
                last_seen: p.last_ping,
                via: p.via.clone(),
                ..DeviceRow::default()
            };
            for s in [p.last_sync, p.last_rows].into_iter().flatten() {
                if r.last_seen.is_none_or(|l| s.unix_nano() > l.unix_nano()) {
                    r.last_seen = Some(s);
                }
            }
            if p.revoked && p.pending < p.remaining {
                r.warning = format!(
                    "Removal pending: seen by {} of {} devices.",
                    p.pending, p.remaining
                );
            } else if p.revoked {
                r.warning = "Removed.".into();
            } else if !p.stopped.is_empty() {
                r.warning = format!("It {}. Remove it.", p.stopped);
                r.action = "remove".into();
            } else if p.old {
                r.warning = "It runs an older Irori. Update it.".into();
            } else if p.ahead {
                r.warning = "Its clock is ahead: its rows are waiting.".into();
            } else if p.refused > 0 {
                r.warning = format!("{} of its rows were refused.", p.refused);
            } else if p.ignored > 0 {
                r.warning = format!(
                    "{} of its rows were ignored: written by the wrong device. Remove it if you don't know why.",
                    p.ignored
                );
                r.action = "remove".into();
            }
            if detail {
                r.detail = Some(p.clone());
            }
            out.push(r);
        }
        out
    }

    /// Go `Connected`: the other devices connected now (health's
    /// net.peers).
    pub fn connected_count(&self) -> usize {
        self.status()
            .peer_list()
            .iter()
            .filter(|p| p.state == "connected")
            .count()
    }

    fn revokes(&self, refused_too: bool) -> Vec<Event> {
        let mut evs = self
            .o
            .store
            .list(&Filter {
                kinds: vec![identity::kinds::PERSON_DEVICE_REVOKED.into()],
                limit: MAX_LIST_LIMIT,
                ..Filter::default()
            })
            .unwrap_or_default();
        if refused_too && let Ok(rf) = self.o.store.refused(MAX_LIST_LIMIT) {
            evs.extend(rf);
        }
        evs
    }

    /// Go `removalSeen`: the remaining devices (certified, unrevoked, this
    /// one included) whose last vector holds a revoke of `dev`, of how
    /// many there are.
    fn removal_seen(&self, dev: &str, known: &BTreeMap<String, PeerInfo>) -> (i64, i64) {
        let revokes: Vec<Event> = self
            .revokes(false)
            .into_iter()
            .filter(|ev| {
                decode::<DeviceRevoked>(&ev.payload).is_some_and(|r| r.revoke.device_id == dev)
            })
            .collect();
        let devices = self.o.chain.devices();
        let (mut seen, mut of) = (0, 0);
        if revokes.is_empty() {
            // Only a revoke kept in refused_events: it travels loose,
            // outside any vector, so there is nothing to count.
            of = devices.iter().filter(|d| !d.revoked).count() as i64;
            return (of, of);
        }
        for d in devices {
            if d.revoked {
                continue;
            }
            let id = &d.cert.device_id;
            of += 1;
            if *id == self.self_id {
                seen += 1;
                continue;
            }
            let vec = known.get(id).map(|k| &k.vec);
            if revokes.iter().any(|ev| {
                ev.device_id == *id
                    || vec.and_then(|v| v.get(&ev.device_id)).copied().unwrap_or(0) >= ev.seq
            }) {
                seen += 1;
            }
        }
        (seen, of)
    }

    /// Go `revokedBy`: the label (else id) of the device that wrote the
    /// first held revoke of `dev`, or "".
    fn revoked_by(&self, dev: &str) -> String {
        for ev in self.revokes(true) {
            if ev.kind != identity::kinds::PERSON_DEVICE_REVOKED {
                continue;
            }
            if !decode::<DeviceRevoked>(&ev.payload).is_some_and(|r| r.revoke.device_id == dev) {
                continue;
            }
            return match self.o.chain.cert(&ev.device_id) {
                Some(c) if !c.label.is_empty() => c.label,
                _ => ev.device_id,
            };
        }
        String::new()
    }

    /// Go `deviceOS`: the os its newest device.started row names.
    fn device_os(&self, dev: &str) -> String {
        let evs = self
            .o
            .store
            .list(&Filter {
                kinds: vec![store::kinds::DEVICE_STARTED.into()],
                device_id: dev.into(),
                desc: true,
                limit: 1,
                ..Filter::default()
            })
            .unwrap_or_default();
        let Some(ev) = evs.first() else {
            return String::new();
        };
        let schema = gocompat::json::Schema::object(vec![("os", gocompat::json::Schema::String)]);
        match gocompat::json::unmarshal(&ev.payload, &schema) {
            Ok(o) if o.get_str("os").len() <= 20 => o.get_str("os").to_string(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;
    use std::time::Duration;

    const NOW: u64 = 1_000_000;

    fn ago(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(NOW - s)
    }

    fn now() -> SystemTime {
        ago(0)
    }

    fn rows(s: u64, via: &str) -> RowsHeard {
        RowsHeard {
            at: Some(ago(s)),
            via: via.into(),
            seeded: true,
        }
    }

    fn peer(ping: Option<u64>, sync: Option<u64>) -> PeerInfo {
        PeerInfo {
            last_ping: ping.map(ago),
            last_sync: sync.map(ago),
            ..PeerInfo::default()
        }
    }

    #[test]
    fn rows_through_another_device_read_recent_and_name_it() {
        let r = read_of(now(), &PeerInfo::default(), &rows(5, "phone"), false, false);
        assert_eq!(r.state, "recent");
        assert_eq!(r.seen, Some(ago(5)));
        assert_eq!(r.via, "phone");
    }

    #[test]
    fn old_rows_arriving_now_do_not_read_fresh() {
        let r = read_of(
            now(),
            &PeerInfo::default(),
            &rows(3600, "phone"),
            false,
            false,
        );
        assert_eq!(r.state, "offline");
        assert_eq!(r.seen, Some(ago(3600)), "still heard from, at that time");
        assert_eq!(r.via, "");
    }

    #[test]
    fn a_live_link_wins_over_a_relay() {
        let r = read_of(
            now(),
            &peer(Some(1), Some(1)),
            &rows(0, "phone"),
            true,
            false,
        );
        assert_eq!((r.state, r.via.as_str()), ("connected", ""));
    }

    #[test]
    fn a_session_newer_than_the_rows_names_no_relay() {
        let r = read_of(
            now(),
            &peer(Some(60), Some(60)),
            &rows(90, "phone"),
            false,
            false,
        );
        assert_eq!((r.state, r.via.as_str()), ("recent", ""));
    }

    #[test]
    fn a_link_lost_and_rows_still_coming_names_the_relay() {
        let r = read_of(
            now(),
            &peer(Some(300), Some(300)),
            &rows(5, "phone"),
            false,
            false,
        );
        assert_eq!((r.state, r.via.as_str()), ("recent", "phone"));
    }

    #[test]
    fn rows_of_a_removed_device_count_for_nothing() {
        let r = read_of(now(), &PeerInfo::default(), &rows(5, "phone"), false, true);
        assert_eq!((r.state, r.seen, r.via.as_str()), ("offline", None, ""));
    }

    #[test]
    fn a_device_never_heard_reads_so() {
        let r = read_of(
            now(),
            &PeerInfo::default(),
            &RowsHeard::default(),
            false,
            false,
        );
        assert_eq!((r.state, r.seen), ("offline", None));
    }
}
