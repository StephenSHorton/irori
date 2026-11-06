//! Small files picked up as they're offered (card #580, no-server.md §4.3
//! "Leave nothing stranded"): a desktop fetches every small file another
//! device offers, in the private log and in each shared board's log, so a
//! sleeping laptop isn't the only holder. A fetch offers the file again
//! from here (`Node::fetch_file`, `BoardMesh::fetch_file`), so this device
//! then serves it to the others.
//!
//! The trigger is the `attach.offered` row: its payload names the file and
//! its device is a holder. Offers older than [`MAX_AGE`] are left alone,
//! so a first start on an old log doesn't pull every past attachment. A
//! fetch that finds no holder online waits and tries again, 1 min doubling
//! to 1 h, or at once when another device offers the same file.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use blob::FileRef;
use gocompat::sub::TryRecv;
use mesh::FetchError;
use serde::Deserialize;
use store::{Event, Filter, MAX_LIST_LIMIT, Store, Subscription, Timestamp};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::NowFn;

/// The largest file a desktop picks up unasked (card #580).
pub const SMALL_FILE: u64 = 8 << 20;

/// Offers older than this are not picked up.
pub const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);

const FIRST_RETRY: Duration = Duration::from_secs(60);
const LAST_RETRY: Duration = Duration::from_secs(3600);
/// How often the board logs' rows are read and new boards looked for.
const TICK: Duration = Duration::from_secs(2);
const BOARDS_EVERY: Duration = Duration::from_secs(10);

/// An attach.offered row's payload.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Offer {
    sha256: String,
    size: i64,
    name: String,
    mime: String,
}

/// Whether an offer of `size` bytes, written by `device` at `wall_ms`, is
/// one to pick up at `now_ms` (card #580).
pub(crate) fn wanted(
    device: &str,
    me: &str,
    size: i64,
    wall_ms: i64,
    now_ms: i64,
    cap: u64,
) -> bool {
    device != me
        && size >= 0
        && size as u64 <= cap
        && now_ms.saturating_sub(wall_ms) <= MAX_AGE.as_millis() as i64
}

struct Pending {
    r: FileRef,
    due: Instant,
    wait: Duration,
}

/// What the prefetcher reads and fetches with.
pub(crate) struct Prefetch {
    pub mesh: Arc<crate::meshnode::Mesh>,
    pub store: Arc<Store>,
    pub files: blob::Files,
    pub now: NowFn,
    pub me: String,
    pub cap: u64,
}

impl Prefetch {
    fn cap(&self) -> u64 {
        self.cap.min(self.files.max())
    }

    fn now_ms(&self) -> i64 {
        (self.now)().unix_milli_value()
    }

    /// Queues `ev` if it offers a file to pick up. A pending file a new
    /// device offers is tried again at once.
    fn take(&self, q: &mut BTreeMap<String, Pending>, ev: &Event) {
        if ev.kind != store::kinds::ATTACH_OFFERED {
            return;
        }
        let Ok(o) = serde_json::from_slice::<Offer>(&ev.payload) else {
            return;
        };
        if !blob::valid_id(&o.sha256)
            || !wanted(
                &ev.device_id,
                &self.me,
                o.size,
                ev.hlc.wall_ms(),
                self.now_ms(),
                self.cap(),
            )
            || self.files.has(&o.sha256)
        {
            return;
        }
        let now = Instant::now();
        match q.get_mut(&o.sha256) {
            Some(p) => {
                if p.r.device != ev.device_id {
                    p.due = now;
                }
            }
            None => {
                q.insert(
                    o.sha256.clone(),
                    Pending {
                        r: FileRef {
                            sha256: o.sha256,
                            name: o.name,
                            size: o.size,
                            mime: o.mime,
                            device: ev.device_id.clone(),
                        },
                        due: now,
                        wait: FIRST_RETRY,
                    },
                );
            }
        }
    }

    /// Queues every recent offer in `st`.
    fn seed(&self, q: &mut BTreeMap<String, Pending>, st: &Store) {
        let since = self.now_ms() - MAX_AGE.as_millis() as i64;
        let mut f = Filter {
            kinds: vec![store::kinds::ATTACH_OFFERED.into()],
            after_hlc: Timestamp::from_unix_ms(since.max(0)),
            limit: MAX_LIST_LIMIT,
            ..Filter::default()
        };
        loop {
            let evs = match st.list(&f) {
                Ok(evs) => evs,
                Err(e) => {
                    crate::logf(format_args!("prefetch: {e}"));
                    return;
                }
            };
            for ev in &evs {
                self.take(q, ev);
            }
            if (evs.len() as i64) < f.limit {
                return;
            }
            f.after = evs.last().map(Event::cursor).unwrap_or_default();
        }
    }

    /// Follows the private log and every open board's log until `stop`,
    /// fetching one file at a time.
    pub(crate) async fn run(self, sub: Arc<Subscription>, stop: CancellationToken) {
        let mut q: BTreeMap<String, Pending> = BTreeMap::new();
        let mut boards: HashMap<String, (Arc<Store>, Subscription)> = HashMap::new();
        self.seed(&mut q, &self.store);
        let mut boards_at = Instant::now();
        let mut look = true;
        loop {
            if look {
                look = false;
                boards_at = Instant::now() + BOARDS_EVERY;
                let open = self.mesh.board_list();
                boards.retain(|id, _| open.iter().any(|b| b.board_id() == id));
                for b in open {
                    if boards.contains_key(b.board_id()) {
                        continue;
                    }
                    // Subscribed before the listing, so no row is missed.
                    let bsub = b.log().subscribe();
                    self.seed(&mut q, b.log());
                    boards.insert(b.board_id().to_string(), (b.log().clone(), bsub));
                }
            }
            for (st, bsub) in boards.values() {
                loop {
                    match bsub.try_recv() {
                        TryRecv::Item(ev) if ev.is_resync() => self.seed(&mut q, st),
                        TryRecv::Item(ev) => self.take(&mut q, &ev),
                        TryRecv::Empty | TryRecv::Closed => break,
                    }
                }
            }
            let now = Instant::now();
            let next = q
                .iter()
                .filter(|(_, p)| p.due <= now)
                .map(|(sha, _)| sha.clone())
                .next();
            if let Some(sha) = next {
                let r = q[&sha].r.clone();
                let got = tokio::select! {
                    _ = stop.cancelled() => return,
                    got = tokio::time::timeout(
                        crate::meshview::FETCH_TIMEOUT,
                        crate::meshview::fetch_any(&self.mesh, &r),
                    ) => got,
                };
                match got {
                    Ok(Ok(_)) => {
                        q.remove(&sha);
                    }
                    Ok(Err(FetchError::NoFiles)) => q.clear(),
                    Ok(Err(FetchError::Blob(k, e))) if k != blob::ErrorKind::Mismatch => {
                        crate::logf(format_args!("prefetch {sha}: {e}"));
                        q.remove(&sha);
                    }
                    // Offline, a deadline, a holder that failed: later.
                    _ => {
                        if let Some(p) = q.get_mut(&sha) {
                            p.due = Instant::now() + p.wait;
                            p.wait = (p.wait * 2).min(LAST_RETRY);
                        }
                    }
                }
                continue;
            }
            let wake = q
                .values()
                .map(|p| p.due)
                .min()
                .unwrap_or(boards_at)
                .min(boards_at)
                .min(Instant::now() + TICK);
            tokio::select! {
                _ = stop.cancelled() => return,
                ev = sub.recv_async() => match ev {
                    None => return,
                    Some(ev) if ev.is_resync() => self.seed(&mut q, &self.store),
                    Some(ev) => self.take(&mut q, &ev),
                },
                _ = tokio::time::sleep_until(wake) => {}
            }
            if Instant::now() >= boards_at {
                look = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 3600 * 1000;

    #[test]
    fn small_recent_files_from_other_devices_only() {
        let now = 100 * DAY;
        assert!(wanted("a", "b", 1024, now - DAY, now, SMALL_FILE));
        assert!(wanted("a", "b", SMALL_FILE as i64, now, now, SMALL_FILE));
        // This device's own offer.
        assert!(!wanted("b", "b", 1024, now, now, SMALL_FILE));
        // Over the cap.
        assert!(!wanted(
            "a",
            "b",
            SMALL_FILE as i64 + 1,
            now,
            now,
            SMALL_FILE
        ));
        assert!(!wanted("a", "b", -1, now, now, SMALL_FILE));
        // Older than 30 days.
        assert!(wanted("a", "b", 1024, now - 30 * DAY, now, SMALL_FILE));
        assert!(!wanted("a", "b", 1024, now - 31 * DAY, now, SMALL_FILE));
    }
}
