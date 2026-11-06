//! A board's warning lines in the Boards dialog (v0.5-shared-boards §6.3-
//! 6.5, §8): one line per thing that is wrong, and nothing when all is
//! well.
//!
//! - **Update Irori** (§6.3): a member device's hello named another core
//!   version; the line says whose Irori is the older one.
//! - **A stalled forwarder** (§6.4): a member device still talking to us
//!   whose vector has stopped moving for a stream that we, or another
//!   member device, hold further.
//! - **This device is stalled** (no-server.md §4.3): it has stopped taking
//!   in a writer's rows, and says so itself instead of leaving it to others.
//! - **Can't reach any home server** (§6.5): other people are on the board,
//!   no member device of theirs is connected on it, and this device's own
//!   home relay isn't connected either.
//! - **Removed**: an owner took this person off; their copy stays here
//!   and nothing they write counts.
//! - **Nothing heard** (no-server.md §5): a member device nobody here has
//!   any word from. It doesn't hold up a write, so it could wake too old
//!   to read one.
//!
//! [`board_warnings`] is pure; [`Services::board_status`] gathers its
//! input from the board, its mesh and the endpoint.

use std::collections::BTreeMap;
use std::time::Duration;

use irori_identity::membership::{MEMBER_ACTIVE, MEMBER_REMOVED};
use mesh::boardmesh::{PeerHeard, SelfStall};

use super::Services;
use super::boards::BoardLog;

/// A member device still counts as talking when its last frame is this
/// recent.
pub const HEARD_LIVE: Duration = Duration::from_secs(90);
/// How long a stream may lag on a talking device before it is stalled.
pub const STALL_AFTER: Duration = Duration::from_secs(120);
/// How long after opening a board before "Can't reach" can show: the
/// first dials take a moment.
pub const REACH_GRACE: Duration = Duration::from_secs(60);

/// What [`board_warnings`] reads.
#[derive(Debug, Clone, Default)]
pub struct BoardStatusInput {
    pub tag: String,
    /// This device's core version.
    pub version: String,
    /// The person at this device.
    pub me: String,
    /// This person's state on the board (active, removed, left).
    pub my_state: String,
    /// Other people active on the board.
    pub others: bool,
    /// A board connection to another person's device is up.
    pub connected: bool,
    /// This device's home relay is connected.
    pub home: bool,
    /// Public relays are on (card #328): the home relay is an n0 relay, so
    /// the warning can't blame home servers.
    pub public: bool,
    /// The board has been open here at least [`REACH_GRACE`].
    pub settled: bool,
    /// What each member device last said ([`mesh::BoardMesh::heard`]).
    pub heard: Vec<PeerHeard>,
    /// The member devices with no word on what they read.
    pub unheard: Vec<String>,
    /// This device's board vector.
    pub vector: BTreeMap<String, i64>,
    /// The writers this device stopped taking ([`mesh::BoardMesh::stalled`]).
    pub stalled: Vec<SelfStall>,
    pub now_ms: i64,
    /// A person's name ("" when unknown).
    pub names: BTreeMap<String, String>,
    /// The person a device is certified to.
    pub device_person: BTreeMap<String, String>,
}

impl BoardStatusInput {
    fn name(&self, person: &str) -> String {
        self.names
            .get(person)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| "someone".into())
    }

    /// Whose rows a device's stream carries, as a line says it.
    fn whose(&self, dev: &str) -> String {
        match self.device_person.get(dev) {
            Some(p) if p == &self.me => "your".to_string(),
            Some(p) => format!("{}'s", self.name(p)),
            None => "a member's".to_string(),
        }
    }

    fn device(&self, dev: &str) -> String {
        let short: String = dev.chars().take(6).collect();
        match self.device_person.get(dev) {
            Some(p) if p == &self.me => format!("your device {short}"),
            Some(p) => format!("{}'s device {short}", self.name(p)),
            None => format!("device {short}"),
        }
    }
}

/// `a` against `b` as `x.y.z[-pre]`: below zero when `a` is older. A
/// prerelease sorts before its release; unparsable parts count as 0.
pub fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(v: &str) -> ([i64; 3], Option<&str>) {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        let mut out = [0; 3];
        for (i, p) in core.split('.').take(3).enumerate() {
            out[i] = p.parse().unwrap_or(0);
        }
        (out, pre)
    }
    let ((na, pa), (nb, pb)) = (parts(a), parts(b));
    na.cmp(&nb).then_with(|| match (pa, pb) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(x), Some(y)) => x.cmp(y),
    })
}

/// The board's warning lines, most pressing first.
pub fn board_warnings(i: &BoardStatusInput) -> Vec<String> {
    let mut out = Vec::new();
    if i.my_state == MEMBER_REMOVED {
        out.push(
            "You're no longer in this space: an owner removed you. What's here stays, and nothing you write counts."
                .into(),
        );
        return out;
    }
    if i.others && !i.connected && !i.home && i.settled {
        let tag = if i.tag.is_empty() {
            "this space"
        } else {
            &i.tag
        };
        out.push(if i.public {
            format!("Can't reach anyone on {tag}: no relay answers. Check this device's internet.")
        } else {
            format!("Can't reach any home server for {tag}")
        });
    }

    // Versions: the newest each other person's devices said.
    let mut versions: BTreeMap<String, String> = BTreeMap::new();
    for h in &i.heard {
        if h.person_id.is_empty() || h.person_id == i.me || h.core_version.is_empty() {
            continue;
        }
        let v = versions.entry(h.person_id.clone()).or_default();
        if v.is_empty() || cmp_version(&h.core_version, v).is_gt() {
            *v = h.core_version.clone();
        }
    }
    for (p, v) in &versions {
        match cmp_version(v, &i.version) {
            std::cmp::Ordering::Greater => out.push(format!(
                "Update Irori: {} has {v}, this device {}",
                i.name(p),
                i.version
            )),
            std::cmp::Ordering::Less => out.push(format!(
                "{}'s Irori is out of date ({v}; this device has {}): they should update Irori",
                i.name(p),
                i.version
            )),
            std::cmp::Ordering::Equal => {}
        }
    }

    // Devices nobody has word from.
    for d in &i.unheard {
        out.push(format!(
            "Nothing heard yet from {}: if its Irori is old, it may stall until it updates",
            i.device(d)
        ));
    }

    // This device, stalled. One line per person, after the same wait as
    // the line others show about it, so a passing refusal doesn't flash.
    let stall = STALL_AFTER.as_millis() as i64;
    let mut said: Vec<String> = Vec::new();
    for s in i.stalled.iter().filter(|s| i.now_ms - s.since_ms > stall) {
        let whose = i.whose(&s.device_id);
        // Any of this person's writers an update heals says so.
        let unread = i
            .stalled
            .iter()
            .any(|o| o.unread && i.whose(&o.device_id) == whose);
        if said.contains(&whose) {
            continue;
        }
        out.push(if unread {
            format!("This device is stalled: it isn't taking in {whose} rows until you update Irori here")
        } else {
            format!("This device is stalled: it isn't taking in {whose} rows")
        });
        said.push(whose);
    }

    // Stalled forwarders.
    let live = HEARD_LIVE.as_millis() as i64;
    for h in &i.heard {
        if i.now_ms - h.at_ms > live {
            continue;
        }
        let mut lag: Vec<&str> = Vec::new();
        let mut streams: Vec<&String> = i.vector.keys().collect();
        for o in &i.heard {
            streams.extend(o.vector.keys());
        }
        streams.sort();
        streams.dedup();
        for d in streams {
            if *d == h.device_id {
                continue;
            }
            let theirs = h.vector.get(d).copied().unwrap_or(0);
            let best = i
                .heard
                .iter()
                .filter(|o| o.device_id != h.device_id)
                .filter_map(|o| o.vector.get(d).copied())
                .chain(i.vector.get(d).copied())
                .max()
                .unwrap_or(0);
            let since = h.advanced_ms.get(d).copied().unwrap_or(h.at_ms);
            if theirs < best && i.now_ms - since > stall {
                lag.push(d);
            }
        }
        if let Some(d) = lag.first() {
            out.push(format!(
                "{} is stalled: it isn't taking in {} rows",
                capitalize(&i.device(&h.device_id)),
                i.whose(d)
            ));
        }
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

impl Services {
    /// Board `b`'s warning lines now.
    pub(crate) fn board_status(&self, b: &BoardLog) -> Vec<String> {
        let me = self.person_id();
        let m = b.verifier.membership();
        let mut names = BTreeMap::new();
        let mut device_person = BTreeMap::new();
        for p in &m.members {
            let name = b.verifier.profile(&p.person_id).unwrap_or(p.name.clone());
            names.insert(p.person_id.clone(), name);
            for d in b.verifier.devices(&p.person_id) {
                device_person.insert(d.cert.device_id, p.person_id.clone());
            }
        }
        let net = &self.mesh.net;
        let connected = net.board_conns(&b.id).iter().any(|c| {
            device_person
                .get(&c.peer)
                .is_some_and(|p| p != &me && m.active(p))
        });
        let mesh = self.mesh.board(&b.id);
        let heard = mesh.as_ref().map(|x| x.heard()).unwrap_or_default();
        let stalled = mesh.as_ref().map(|x| x.stalled()).unwrap_or_default();
        let unheard = self
            .space_readers(b)
            .map(|r| r.unheard.into_iter().map(|(_, d)| d).collect())
            .unwrap_or_default();
        board_warnings(&BoardStatusInput {
            tag: m.tag.clone(),
            version: self.version.clone(),
            my_state: m
                .person(&me)
                .map(|p| p.state.clone())
                .unwrap_or_else(|| MEMBER_ACTIVE.into()),
            others: m
                .members
                .iter()
                .any(|p| p.person_id != me && m.active(&p.person_id)),
            connected,
            home: net.home_relays().iter().any(|(_, up)| *up),
            public: net.public_relays(),
            settled: b.opened.elapsed() >= REACH_GRACE,
            heard,
            unheard,
            vector: b.store.vector().unwrap_or_default(),
            stalled,
            now_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
            names,
            device_person,
            me,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10_000_000;

    fn base() -> BoardStatusInput {
        BoardStatusInput {
            tag: "mlg".into(),
            version: "0.6.0".into(),
            me: "pAlice".into(),
            my_state: MEMBER_ACTIVE.into(),
            others: true,
            connected: true,
            home: true,
            public: false,
            settled: true,
            now_ms: NOW,
            names: [
                ("pAlice", "Alice"),
                ("pBlake", "Blake"),
                ("pCasey", "Casey"),
            ]
            .into_iter()
            .map(|(a, b)| (a.into(), b.into()))
            .collect(),
            device_person: [("a1", "pAlice"), ("b1", "pBlake"), ("c1", "pCasey")]
                .into_iter()
                .map(|(a, b)| (a.into(), b.into()))
                .collect(),
            ..Default::default()
        }
    }

    fn heard(dev: &str, person: &str, version: &str, vector: &[(&str, i64)]) -> PeerHeard {
        PeerHeard {
            device_id: dev.into(),
            person_id: person.into(),
            core_version: version.into(),
            vector: vector.iter().map(|(d, s)| (d.to_string(), *s)).collect(),
            at_ms: NOW,
            advanced_ms: vector.iter().map(|(d, _)| (d.to_string(), NOW)).collect(),
            spaces: 0,
            spawns: 0,
            tolerant: 0,
        }
    }

    #[test]
    fn all_well_says_nothing() {
        let mut i = base();
        i.heard = vec![heard("b1", "pBlake", "0.6.0", &[("a1", 3), ("b1", 5)])];
        i.vector = [("a1".into(), 3), ("b1".into(), 5)].into();
        assert!(board_warnings(&i).is_empty(), "{:?}", board_warnings(&i));
    }

    #[test]
    fn versions() {
        let mut i = base();
        i.heard = vec![heard("b1", "pBlake", "0.7.0", &[])];
        assert_eq!(
            board_warnings(&i),
            ["Update Irori: Blake has 0.7.0, this device 0.6.0"]
        );
        i.heard = vec![heard("b1", "pBlake", "0.5.2", &[])];
        assert_eq!(
            board_warnings(&i),
            [
                "Blake's Irori is out of date (0.5.2; this device has 0.6.0): they should update Irori"
            ]
        );
        // A prerelease is older than its release; one person, one line.
        i.heard = vec![
            heard("b1", "pBlake", "0.6.0-rc1", &[]),
            heard("b2", "pBlake", "0.6.0-rc2", &[]),
        ];
        assert_eq!(board_warnings(&i).len(), 1);
        assert!(board_warnings(&i)[0].contains("0.6.0-rc2"));
        assert!(cmp_version("0.10.0", "0.9.9").is_gt());
    }

    /// Card #579: the stalled device says so itself, after the wait the
    /// others use, one line a person, and says when updating heals it.
    #[test]
    fn this_device_says_it_is_stalled() {
        let stall = |d: &str, ago: i64, unread| SelfStall {
            device_id: d.into(),
            seq: 7,
            since_ms: NOW - ago,
            unread,
        };
        let mut i = base();
        i.device_person.insert("c1".into(), "pCasey".into());
        i.device_person.insert("c2".into(), "pCasey".into());
        i.names.insert("pCasey".into(), "Casey".into());
        i.stalled = vec![stall("c1", 10_000, true)];
        assert!(board_warnings(&i).is_empty(), "too soon to say");
        i.stalled = vec![stall("c1", 180_000, true), stall("c2", 180_000, false)];
        assert_eq!(
            board_warnings(&i),
            ["This device is stalled: it isn't taking in Casey's rows until you update Irori here"]
        );
        i.stalled = vec![stall("c2", 180_000, false)];
        assert_eq!(
            board_warnings(&i),
            ["This device is stalled: it isn't taking in Casey's rows"]
        );
        i.stalled = vec![stall("x9", 180_000, false)];
        assert_eq!(
            board_warnings(&i),
            ["This device is stalled: it isn't taking in a member's rows"]
        );
    }

    #[test]
    fn stalled_forwarder() {
        let mut i = base();
        let mut b = heard("b1", "pBlake", "0.6.0", &[("c1", 4), ("b1", 9)]);
        // Casey's stream stopped moving on Blake's device 3 minutes ago,
        // while Casey's own device is at 10.
        b.advanced_ms.insert("c1".into(), NOW - 180_000);
        let c = heard("c1", "pCasey", "0.6.0", &[("c1", 10), ("b1", 9)]);
        i.heard = vec![b.clone(), c.clone()];
        i.vector = [("b1".into(), 9), ("c1".into(), 10)].into();
        assert_eq!(
            board_warnings(&i),
            ["Blake's device b1 is stalled: it isn't taking in Casey's rows"]
        );
        // Lagging only briefly isn't stalled.
        b.advanced_ms.insert("c1".into(), NOW - 10_000);
        i.heard = vec![b.clone(), c.clone()];
        assert!(board_warnings(&i).is_empty());
        // A device no longer talking isn't flagged.
        b.advanced_ms.insert("c1".into(), NOW - 180_000);
        b.at_ms = NOW - 300_000;
        i.heard = vec![b, c];
        assert!(board_warnings(&i).is_empty());
    }

    /// Card #577: a device with no word gets a line naming it, whoever's
    /// it is.
    #[test]
    fn a_device_nobody_has_heard_gets_a_line() {
        let mut i = base();
        i.unheard = vec!["b1".into(), "a1".into()];
        assert_eq!(
            board_warnings(&i),
            [
                "Nothing heard yet from Blake's device b1: if its Irori is old, it may stall until it updates",
                "Nothing heard yet from your device a1: if its Irori is old, it may stall until it updates",
            ]
        );
    }

    #[test]
    fn cant_reach_any_home_server() {
        let mut i = base();
        i.connected = false;
        i.home = false;
        assert_eq!(board_warnings(&i), ["Can't reach any home server for mlg"]);
        i.public = true;
        assert_eq!(
            board_warnings(&i),
            ["Can't reach anyone on mlg: no relay answers. Check this device's internet."]
        );
        // Any one of: a member connected, the home relay up, alone on it,
        // or just opened.
        for f in [
            |i: &mut BoardStatusInput| i.connected = true,
            |i: &mut BoardStatusInput| i.home = true,
            |i: &mut BoardStatusInput| i.others = false,
            |i: &mut BoardStatusInput| i.settled = false,
        ] {
            let mut j = i.clone();
            f(&mut j);
            assert!(board_warnings(&j).is_empty());
        }
    }

    #[test]
    fn removed() {
        let mut i = base();
        i.my_state = MEMBER_REMOVED.into();
        i.heard = vec![heard("b1", "pBlake", "0.7.0", &[])];
        let w = board_warnings(&i);
        assert!(
            w.len() == 1 && w[0].starts_with("You're no longer in this space"),
            "{w:?}"
        );
    }
}
