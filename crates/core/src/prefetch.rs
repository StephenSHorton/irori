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
//! file is tried only while a device that offered it is connected, one
//! fetch at a time with a pause between; a fetch that fails waits and
//! tries again, 1 min doubling to 1 h, or at once when another device
//! offers the same file.

use std::collections::{BTreeMap, HashMap, HashSet};
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
/// The least time between two fetches (card #580): each one scans the
/// logs for holders.
const PACE: Duration = Duration::from_millis(500);

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
    /// Every other device that offered it, the origin first.
    holders: Vec<String>,
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

    /// The file `ev` offers, if it is one to pick up.
    fn check(&self, ev: &Event) -> Option<FileRef> {
        if ev.kind != store::kinds::ATTACH_OFFERED {
            return None;
        }
        let o = serde_json::from_slice::<Offer>(&ev.payload).ok()?;
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
            // The person deleted it here from Storage (card #728).
            || self.files.dropped(&o.sha256)
        {
            return None;
        }
        Some(FileRef {
            sha256: o.sha256,
            name: o.name,
            size: o.size,
            mime: o.mime,
            device: ev.device_id.clone(),
        })
    }

    /// Queues `ev` if it offers a file to pick up.
    fn take(&self, q: &mut BTreeMap<String, Pending>, ev: &Event) {
        if let Some(r) = self.check(ev) {
            queue(q, r);
        }
    }

    /// Queues every recent offer in `st`. The listing runs off the async
    /// workers (card #580), and `stop` ends the wait for it.
    async fn seed(
        self: &Arc<Self>,
        q: &mut BTreeMap<String, Pending>,
        st: &Arc<Store>,
        stop: &CancellationToken,
    ) {
        let (me, st) = (self.clone(), st.clone());
        let job = tokio::task::spawn_blocking(move || me.offered(&st));
        let refs = tokio::select! {
            _ = stop.cancelled() => return,
            got = job => got.unwrap_or_default(),
        };
        for r in refs {
            queue(q, r);
        }
    }

    /// Every recent offer in `st` to pick up. Blocking.
    fn offered(&self, st: &Store) -> Vec<FileRef> {
        let since = self.now_ms() - MAX_AGE.as_millis() as i64;
        let mut f = Filter {
            kinds: vec![store::kinds::ATTACH_OFFERED.into()],
            after_hlc: Timestamp::from_unix_ms(since.max(0)),
            limit: MAX_LIST_LIMIT,
            ..Filter::default()
        };
        let mut out = Vec::new();
        loop {
            let evs = match st.list(&f) {
                Ok(evs) => evs,
                Err(e) => {
                    crate::logf(format_args!("prefetch: {e}"));
                    return out;
                }
            };
            out.extend(evs.iter().filter_map(|ev| self.check(ev)));
            if (evs.len() as i64) < f.limit {
                return out;
            }
            f.after = evs.last().map(Event::cursor).unwrap_or_default();
        }
    }

    /// The first due file a holder of which is connected now, here or on
    /// an open board. Reads only what's in memory, so files whose holders
    /// are all away cost nothing until one comes back (card #580).
    fn ready(&self, q: &BTreeMap<String, Pending>, now: Instant) -> Option<String> {
        let mut due = q.iter().filter(|(_, p)| p.due <= now).peekable();
        due.peek()?;
        let on_boards: HashSet<String> = self
            .mesh
            .board_list()
            .iter()
            .flat_map(|b| b.peers().into_iter().map(|(_, d)| d))
            .collect();
        due.find(|(_, p)| {
            p.holders
                .iter()
                .any(|h| on_boards.contains(h) || self.mesh.node.is_connected(h))
        })
        .map(|(sha, _)| sha.clone())
    }

    /// Follows the private log and every open board's log until `stop`,
    /// fetching one file at a time, at most one every [`PACE`].
    pub(crate) async fn run(self, sub: Arc<Subscription>, stop: CancellationToken) {
        let this = Arc::new(self);
        let mut q: BTreeMap<String, Pending> = BTreeMap::new();
        let mut boards: HashMap<String, (Arc<Store>, Subscription)> = HashMap::new();
        this.seed(&mut q, &this.store, &stop).await;
        let mut boards_at = Instant::now();
        let mut rest = Instant::now();
        let mut look = true;
        loop {
            if look {
                look = false;
                boards_at = Instant::now() + BOARDS_EVERY;
                let open = this.mesh.board_list();
                boards.retain(|id, _| open.iter().any(|b| b.board_id() == id));
                for b in open {
                    // A board closed and joined again has a new log.
                    if boards
                        .get(b.board_id())
                        .is_some_and(|(st, _)| Arc::ptr_eq(st, b.log()))
                    {
                        continue;
                    }
                    // Subscribed before the listing, so no row is missed.
                    let bsub = b.log().subscribe();
                    this.seed(&mut q, b.log(), &stop).await;
                    boards.insert(b.board_id().to_string(), (b.log().clone(), bsub));
                }
            }
            let mut resync = Vec::new();
            for (st, bsub) in boards.values() {
                loop {
                    match bsub.try_recv() {
                        TryRecv::Item(ev) if ev.is_resync() => resync.push(st.clone()),
                        TryRecv::Item(ev) => this.take(&mut q, &ev),
                        TryRecv::Empty | TryRecv::Closed => break,
                    }
                }
            }
            for st in resync {
                this.seed(&mut q, &st, &stop).await;
            }
            let now = Instant::now();
            let next = (now >= rest).then(|| this.ready(&q, now)).flatten();
            if let Some(sha) = next {
                let r = q[&sha].r.clone();
                let got = tokio::select! {
                    _ = stop.cancelled() => return,
                    got = tokio::time::timeout(
                        crate::meshview::FETCH_TIMEOUT,
                        crate::meshview::fetch_any(&this.mesh, &r),
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
                rest = Instant::now() + PACE;
            }
            // Due files wait for a holder to connect: looked at each tick.
            let now = Instant::now();
            let wake = q
                .values()
                .map(|p| p.due)
                .filter(|d| *d > now)
                .min()
                .unwrap_or(boards_at)
                .min(boards_at)
                .min(now + TICK)
                .max(rest);
            tokio::select! {
                _ = stop.cancelled() => return,
                ev = sub.recv_async() => match ev {
                    None => return,
                    Some(ev) if ev.is_resync() => this.seed(&mut q, &this.store, &stop).await,
                    Some(ev) => this.take(&mut q, &ev),
                },
                _ = tokio::time::sleep_until(wake) => {}
            }
            if Instant::now() >= boards_at {
                look = true;
            }
        }
    }
}

/// Queues `r`. A pending file another device offers too is tried again
/// at once.
fn queue(q: &mut BTreeMap<String, Pending>, r: FileRef) {
    match q.get_mut(&r.sha256) {
        Some(p) => {
            if !p.holders.contains(&r.device) {
                p.holders.push(r.device);
                p.due = Instant::now();
            }
        }
        None => {
            q.insert(
                r.sha256.clone(),
                Pending {
                    holders: vec![r.device.clone()],
                    r,
                    due: Instant::now(),
                    wait: FIRST_RETRY,
                },
            );
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
