//! Sync as far as possible before this device sleeps or closes (card #580,
//! docs/design/no-server.md §4.3 "Leave nothing stranded"), so the last
//! thing the person did has left the machine. Irori has no server: a row
//! has left once another device's vector says it holds it. The private log
//! pushes to the person's own devices and their ack vectors say so; a
//! board's member devices pull, and send their vector after a pull.

use std::time::{Duration, Instant};

use irori_identity::membership::MEMBER_LEFT;

use super::freshness::PERSONAL;
use super::{Core, Services};
use crate::logf;

/// How often [`Core::sync_out`] looks again.
const POLL: Duration = Duration::from_millis(50);

/// How long one of the person's own devices up on the private mesh counts
/// as reachable for a board with no board connection to it yet (card
/// #580): about the time a board dial to it takes. A device that never
/// brings the board up (not open there, an older build) costs this, not
/// the whole deadline.
const DIAL_GRACE: Duration = Duration::from_secs(2);

/// One log's part of a [`SyncOutReport`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncOutLog {
    /// [`PERSONAL`] or a shared space's id.
    pub log: String,
    /// This device's newest row there when the sync began.
    pub seq: i64,
    /// The device that holds it; `None`: no device said so in time.
    pub taken_by: Option<String>,
}

/// What [`Core::sync_out`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncOutReport {
    /// Each log with a row of this device's that another device could hold.
    pub logs: Vec<SyncOutLog>,
    /// How long it took.
    pub spent: Duration,
}

impl SyncOutReport {
    /// Every row this device wrote has left it.
    pub fn all_taken(&self) -> bool {
        self.logs.iter().all(|l| l.taken_by.is_some())
    }

    /// The log `log`'s part, if it had anything to send.
    pub fn log(&self, log: &str) -> Option<&SyncOutLog> {
        self.logs.iter().find(|l| l.log == log)
    }
}

impl Services {
    /// The logs to send: this device's newest row in the private log (with
    /// another device of the person's to hold it) and in each open shared
    /// board. Not a board the person left: its `left` row can sit above
    /// this device's cut, where no vector ever counts it.
    fn sync_targets(&self) -> Vec<SyncOutLog> {
        let me = self.device.id.clone();
        let mut out = Vec::new();
        let node = &self.mesh.node;
        let seq = self
            .store
            .vector()
            .ok()
            .and_then(|v| v.get(&me).copied())
            .unwrap_or(0);
        if seq > 0 && node.has_peer() {
            out.push(SyncOutLog {
                log: PERSONAL.into(),
                seq,
                taken_by: None,
            });
        }
        let person = self.person_id();
        for m in self.mesh.board_meshes() {
            if m.member_devices().is_empty()
                || m.verifier()
                    .membership()
                    .person(&person)
                    .is_some_and(|p| p.state == MEMBER_LEFT)
            {
                continue;
            }
            let seq = m
                .log()
                .vector()
                .ok()
                .and_then(|v| v.get(&me).copied())
                .unwrap_or(0);
            if seq > 0 {
                out.push(SyncOutLog {
                    log: m.board_id().to_string(),
                    seq,
                    taken_by: None,
                });
            }
        }
        out
    }

    /// Marks each target another device now holds; true while a target
    /// not yet held has a device connected that could take it. `elapsed`
    /// is the time since the sync began.
    fn sync_check(&self, logs: &mut [SyncOutLog], elapsed: Duration) -> bool {
        let mut reachable = false;
        for l in logs.iter_mut().filter(|l| l.taken_by.is_none()) {
            if l.log == PERSONAL {
                let node = &self.mesh.node;
                l.taken_by = node.taken_by(l.seq);
                // Not while this device's own stream is stopped (a fork):
                // its rows are withheld, so no device will ever hold them.
                reachable |= l.taken_by.is_none()
                    && node.any_connected()
                    && node.stopped_reason(&self.device.id).is_none();
            } else if let Some(m) = self.mesh.board(&l.log) {
                l.taken_by = m.taken_by(l.seq);
                // A board connection, or, for [`DIAL_GRACE`], one of the
                // person's own member devices up on the private mesh, so a
                // dial will reach it.
                reachable |= l.taken_by.is_none()
                    && (!m.peers().is_empty()
                        || (elapsed < DIAL_GRACE
                            && m.member_devices()
                                .iter()
                                .any(|d| self.mesh.node.reaches(&d.device_id))));
            }
        }
        reachable
    }
}

impl Core {
    /// Sends this device's newest rows and waits, up to `deadline`, until
    /// another device says it holds them (card #580): before a quit, a
    /// sleep or a phone going to the background. It returns at once when
    /// every row has left already, or no device that could take one is
    /// connected, so an offline or lone device costs nothing. It polls
    /// with a plain sleep and never blocks on the runtime, so it can run on
    /// the main thread (macOS quits inside `terminate:`).
    pub fn sync_out(&self, deadline: Duration) -> SyncOutReport {
        let start = Instant::now();
        let s = &self.s;
        let mut logs = s.sync_targets();
        let mut reachable = s.sync_check(&mut logs, start.elapsed());
        if reachable {
            self.kick_sync();
        }
        while reachable && start.elapsed() < deadline {
            std::thread::sleep(POLL.min(deadline.saturating_sub(start.elapsed())));
            reachable = s.sync_check(&mut logs, start.elapsed());
        }
        let r = SyncOutReport {
            logs,
            spent: start.elapsed(),
        };
        if !r.logs.is_empty() {
            let parts: Vec<String> = r
                .logs
                .iter()
                .map(|l| {
                    let name = if l.log == PERSONAL {
                        PERSONAL
                    } else {
                        &l.log[..l.log.len().min(8)]
                    };
                    match &l.taken_by {
                        Some(d) => format!("{name} held by {}", &d[..d.len().min(8)]),
                        None => format!("{name} not held yet"),
                    }
                })
                .collect();
            logf(format_args!(
                "sync out: {} in {} ms",
                parts.join(", "),
                r.spent.as_millis()
            ));
        }
        r
    }

    /// Every connection sends now, and devices not connected are dialled
    /// (card #580): after a wake from sleep, or a phone coming back, so
    /// freshness turns current without waiting for the next tick.
    pub fn kick_sync(&self) {
        self.s.mesh.node.nudge();
        self.s.mesh.kick_boards();
    }
}
