//! `irori events [--follow] [--kinds a,b] [--json] [--limit N] [--since
//! SEQ]` (I:events.go): the event log of the running Core. Without
//! `--follow` it prints the newest rows (oldest first) and exits. With
//! `--follow` it prints them and then streams new rows as they are written
//! (`GET /v1/events/stream`), with no polling; `--since SEQ` replays this
//! device's rows after SEQ instead. A follow survives a Core restart (it
//! redials and resumes after the last seq it printed). One line per row;
//! `--json` prints the raw NDJSON rows, and `{"resync":true}` where rows
//! may have been dropped.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use gocompat::flag::{ERR_HELP, FlagSet};
use irori_api::client::{Client, Error};
use tokio_util::sync::CancellationToken;

use crate::datadir;

/// Reconnect backoff for `--follow` when the Core goes away.
const FOLLOW_RETRY_MIN: Duration = Duration::from_secs(1);
const FOLLOW_RETRY_MAX: Duration = Duration::from_secs(10);

/// Go `errOutput`.
const ERR_OUTPUT: &str = "events: stdout closed";

fn flags() -> FlagSet {
    let mut fs = FlagSet::new("events");
    fs.bool("follow", false, "keep streaming new rows");
    fs.string(
        "kinds",
        "",
        "comma list of kinds or prefixes ending in \".\" (agent.,device.started)",
    );
    fs.bool("json", false, "print raw NDJSON rows");
    fs.int(
        "limit",
        20,
        "recent rows to print first (0 = none with --follow)",
    );
    fs.int(
        "since",
        -1,
        "with --follow: replay this device's rows after this seq instead of the recent rows",
    );
    fs
}

/// Runs the role; `Err` is printed by `main` as `irori: <err>`, exit 1.
pub fn main(args: &[String]) -> Result<(), String> {
    let mut fs = flags();
    if let Err(e) = fs.parse(args) {
        // Go's ContinueOnError: the error (not for -h) and the usage.
        if e != ERR_HELP {
            eprintln!("{e}");
        }
        eprint!("{}", fs.usage());
        return Err(e);
    }
    if let Some(a) = fs.args().first() {
        return Err(format!(
            "events: unexpected argument {}",
            gocompat::quote::quote(a)
        ));
    }
    let dir = datadir::process_data_dir()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let stop = CancellationToken::new();
    rt.block_on(async {
        {
            let stop = stop.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    stop.cancel();
                }
            });
        }
        let mut p = Printer::new(std::io::stdout(), fs.get_bool("json"), String::new());
        let kinds = split_kinds(&fs.get_string("kinds"));
        let c = Client::dial(&dir).await.map_err(|e| e.to_string())?;
        p.self_id = c.device_id().await.map_err(|e| e.to_string())?;
        if !fs.get_bool("follow") {
            return p
                .recent(&c, &kinds, fs.get_int("limit"))
                .await
                .map_err(|e| e.text());
        }
        follow_events(
            &stop,
            &dir,
            c,
            &kinds,
            fs.get_int("since"),
            fs.get_int("limit"),
            &mut p,
        )
        .await
    })
}

/// Why one stream, or the recent list, stopped.
#[derive(Debug)]
enum Stop {
    /// The Core answered with an error status: the follow ends.
    Status(String),
    /// stdout is gone: the follow ends.
    Output,
    /// Rows were dropped: reopen from the last seq.
    Resync,
    /// Anything else (the Core went away): redial.
    Other(String),
}

impl Stop {
    fn text(&self) -> String {
        match self {
            Stop::Status(s) | Stop::Other(s) => s.clone(),
            Stop::Output => ERR_OUTPUT.into(),
            Stop::Resync => "rows were dropped".into(),
        }
    }
}

impl From<Error> for Stop {
    fn from(e: Error) -> Stop {
        match e {
            Error::Status { .. } => Stop::Status(e.to_string()),
            e => Stop::Other(e.to_string()),
        }
    }
}

/// Go `followEvents`: streams rows until `stop`, redialling when the Core
/// goes away and reopening after a resync, both resuming after
/// `p.last_self`. `since < 0` starts after this device's newest row
/// (looked up first, so a resync before any own row is printed still
/// replays what was dropped) and prints the newest `recent` rows once the
/// first stream is open, so a row written meanwhile is in the list or the
/// stream and printed once. `since >= 0` replays from there and prints no
/// recent rows.
async fn follow_events<W: Write>(
    stop: &CancellationToken,
    dir: &Path,
    c: Client,
    kinds: &[String],
    mut since: i64,
    mut recent: i64,
    p: &mut Printer<W>,
) -> Result<(), String> {
    if since < 0 {
        since = c.last_seq(&p.self_id).await.map_err(|e| e.to_string())?;
    } else {
        recent = 0;
    }
    p.last_self = p.last_self.max(since);
    let mut c = Some(c);
    let mut wait = FOLLOW_RETRY_MIN;
    let mut idle = 0; // resyncs in a row that printed nothing and moved no seq
    loop {
        if let Some(cl) = &c {
            let (from, printed) = (p.last_self, p.printed);
            let res = tokio::select! {
                _ = stop.cancelled() => return Ok(()),
                r = stream_once(cl, kinds, from, p, &mut recent) => r,
            };
            match &res {
                Err(Stop::Status(s)) => return Err(s.clone()),
                Err(Stop::Output) => return Err(ERR_OUTPUT.into()),
                _ => {}
            }
            if p.last_self != from || p.printed != printed {
                idle = 0;
            }
            if let Err(Stop::Resync) = res {
                idle += 1;
                if idle == 1 {
                    wait = FOLLOW_RETRY_MIN;
                    continue; // reopen now, resuming from the last seq
                }
                // Resynced again with nothing to show for it: back off
                // rather than spin on a row that cannot be read.
            } else {
                let why = match res {
                    Err(e) => e.text(),
                    Ok(()) => "Core closed it".into(),
                };
                eprintln!("irori events: stream ended ({why}); reconnecting");
                c = None;
                idle = 0;
            }
        }
        tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            _ = tokio::time::sleep(wait) => {}
        }
        wait = (wait * 2).min(FOLLOW_RETRY_MAX);
        if c.is_some() {
            continue;
        }
        if let Ok(nc) = Client::dial(dir).await {
            c = Some(nc);
            wait = FOLLOW_RETRY_MIN;
        }
    }
}

/// Go `streamOnce`: one stream until it ends, printing the recent rows
/// once it is open. A resync is printed and returned as [`Stop::Resync`],
/// so the caller reopens with since; an own row skipped as too long counts
/// as seen, so the reopened stream moves past it.
async fn stream_once<W: Write>(
    c: &Client,
    kinds: &[String],
    since: i64,
    p: &mut Printer<W>,
    recent: &mut i64,
) -> Result<(), Stop> {
    let mut rx = c.events_stream(kinds, since).await?;
    if *recent > 0 {
        p.recent(c, kinds, *recent).await?;
        *recent = 0;
    }
    while let Some(it) = rx.recv().await {
        if let Some(e) = it.err {
            return Err(Stop::Other(e));
        }
        if it.resync {
            if it.skipped_device == p.self_id && it.skipped_seq > p.last_self {
                p.last_self = it.skipped_seq;
            }
            p.resync().map_err(|_| Stop::Output)?;
            return Err(Stop::Resync);
        }
        if let Some(row) = it.row {
            p.event(&row.into_event()).map_err(|_| Stop::Output)?;
        }
    }
    Ok(())
}

/// Go `splitKinds`.
fn split_kinds(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string)
        .collect()
}

/// Go `eventPrinter`: rows as readable lines or raw NDJSON, remembering
/// the newest seq of this device it printed.
struct Printer<W> {
    w: W,
    json: bool,
    /// This device's id; other devices' rows are tagged.
    self_id: String,
    last_self: i64,
    /// Rows written so far.
    printed: usize,
    /// Recent rows of a follow, skipped when they arrive live.
    seen: HashSet<(String, i64)>,
    /// The UTC offset (seconds) at a Unix time: local time, or UTC in tests.
    offset: fn(i64) -> i32,
}

impl<W: Write> Printer<W> {
    fn new(w: W, json: bool, self_id: String) -> Printer<W> {
        Printer {
            w,
            json,
            self_id,
            last_self: 0,
            printed: 0,
            seen: HashSet::new(),
            offset: sys::tz::local_offset,
        }
    }

    fn event(&mut self, ev: &store::Event) -> std::io::Result<()> {
        if self.seen.remove(&(ev.device_id.clone(), ev.seq)) {
            return Ok(());
        }
        if ev.device_id == self.self_id && ev.seq > self.last_self {
            self.last_self = ev.seq;
        }
        self.printed += 1;
        if self.json {
            let mut raw =
                gocompat::json::marshal(ev).map_err(|e| std::io::Error::other(e.to_string()))?;
            raw.push(b'\n');
            self.w.write_all(&raw)?;
        } else {
            let line = format_event(ev, &self.self_id, self.offset) + "\n";
            self.w.write_all(line.as_bytes())?;
        }
        self.w.flush()
    }

    /// The newest `limit` rows, oldest first, remembered so an open stream
    /// does not print them again.
    async fn recent(&mut self, c: &Client, kinds: &[String], limit: i64) -> Result<(), Stop> {
        if limit <= 0 {
            return Ok(());
        }
        let mut evs = c.events(kinds, limit).await?;
        evs.reverse(); // the route answers newest first
        for ev in evs {
            self.event(&ev).map_err(|_| Stop::Output)?;
            self.seen.insert((ev.device_id, ev.seq));
        }
        Ok(())
    }

    fn resync(&mut self) -> std::io::Result<()> {
        let line: &[u8] = if self.json {
            b"{\"resync\":true}\n"
        } else {
            b"-- some rows were dropped; resyncing --\n"
        };
        self.w.write_all(line)?;
        self.w.flush()
    }
}

/// A row's summary leads with these keys, in this order; the rest of the
/// payload's scalar fields follow sorted by name.
const SUMMARY_KEYS: [&str; 9] = [
    "key", "name", "status", "reason", "kind", "repo", "path", "sha", "title",
];

/// Caps a row's summary, in runes.
const MAX_SUMMARY: usize = 160;

/// Go `formatEvent`: local time, kind, device (when not self) and the
/// payload's scalar fields as k=v.
fn format_event(ev: &store::Event, self_id: &str, offset: fn(i64) -> i32) -> String {
    let t = gocompat::time::GoTime::unix_milli(ev.hlc.wall_ms());
    let (_, mo, d, h, mi, s) = t
        .with_offset(offset(ev.hlc.wall_ms().div_euclid(1000)))
        .civil();
    let mut b = format!("{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} {:<16}", ev.kind);
    if ev.device_id != self_id {
        b.push_str(&format!(" [{}]", ev.device_id));
    }
    let sum = summarize(&ev.payload);
    if !sum.is_empty() {
        b.push(' ');
        b.push_str(&sum);
    }
    if !ev.actor.is_empty() && ev.actor != "irori" {
        b.push_str(" by=");
        b.push_str(&ev.actor);
    }
    b.trim_end_matches(' ').to_string()
}

/// Go `summarize`: a payload's scalar fields; nested objects and arrays
/// are left out. A payload that is not an object is previewed as is.
fn summarize(payload: &[u8]) -> String {
    use serde_json::Value;
    let m = match serde_json::from_slice::<Value>(payload) {
        Ok(Value::Object(m)) => m,
        Ok(Value::Null) => return String::new(),
        _ => {
            if payload.is_empty() || payload == b"null" {
                return String::new();
            }
            return irori_api::preview(&String::from_utf8_lossy(payload));
        }
    };
    let mut rest: Vec<&String> = m
        .keys()
        .filter(|k| !SUMMARY_KEYS.contains(&k.as_str()))
        .collect();
    rest.sort();
    let order = SUMMARY_KEYS
        .iter()
        .map(|k| k.to_string())
        .chain(rest.into_iter().cloned());
    let mut parts = Vec::new();
    for k in order {
        let Some(v) = m.get(&k) else { continue };
        let s = match v {
            Value::String(v) => {
                if v.is_empty() {
                    continue;
                }
                let s = irori_api::preview(v);
                if s.contains([' ', '=', '"']) {
                    gocompat::quote::quote(&s)
                } else {
                    s
                }
            }
            // strconv.FormatFloat(v, 'f', -1, 64): every JSON number is a
            // float64 in Go's map[string]any.
            Value::Number(n) => format!("{}", n.as_f64().unwrap_or(0.0)),
            Value::Bool(b) => b.to_string(),
            _ => continue, // null, objects, arrays
        };
        parts.push(format!("{k}={s}"));
    }
    let out = parts.join(" ");
    if out.chars().count() > MAX_SUMMARY {
        let cut: String = out.chars().take(MAX_SUMMARY - 1).collect();
        return cut + "…";
    }
    out
}

#[cfg(test)]
mod tests {
    //! I:events_test.go and I:follow_test.go.

    use super::*;
    use std::sync::{Arc, Mutex};

    fn utc(_: i64) -> i32 {
        0
    }

    /// The HLC for a wall time in ms (logical 0).
    fn at(ms: i64) -> store::Timestamp {
        store::Timestamp((ms as u64) << 16)
    }

    // 2026-10-27T09:41:05Z.
    const WHEN: i64 = 1_793_094_065_000;

    fn ev(dev: &str, seq: i64, actor: &str, kind: &str, payload: &str) -> store::Event {
        store::Event {
            device_id: dev.into(),
            seq,
            hlc: at(WHEN),
            actor: actor.into(),
            kind: kind.into(),
            payload: payload.as_bytes().to_vec(),
            sig: vec![],
        }
    }

    #[test]
    fn format_event_table() {
        let long = "x".repeat(300);
        for (name, e, want) in [
            (
                "status with priority keys first",
                ev(
                    "dev1",
                    3,
                    "irori",
                    "agent.status",
                    r#"{"status":"busy","name":"fox","key":"dev1/s1","zeta":1}"#,
                ),
                "10-27 09:41:05 agent.status     key=dev1/s1 name=fox status=busy zeta=1".to_string(),
            ),
            (
                "other device, actor, nested fields left out",
                ev(
                    "dev2",
                    1,
                    "user",
                    "agent.gone",
                    r#"{"key":"dev2/s9","reason":"pid gone","card":{"a":1},"tags":["x"],"ok":true}"#,
                ),
                r#"10-27 09:41:05 agent.gone       [dev2] key=dev2/s9 reason="pid gone" ok=true by=user"#.to_string(),
            ),
            (
                "empty payload",
                ev("dev1", 1, "irori", "device.started", "{}"),
                "10-27 09:41:05 device.started".to_string(),
            ),
            (
                "long kind and non-object payload",
                ev("dev1", 1, "", "capture.commit.seen.long", "\"hi\\nthere\""),
                r#"10-27 09:41:05 capture.commit.seen.long "hi\nthere""#.to_string(),
            ),
            (
                "long value is capped",
                ev("dev1", 1, "", "x", &format!("{{\"name\":\"{long}\"}}")),
                format!("10-27 09:41:05 x                name={}", "x".repeat(120)),
            ),
        ] {
            assert_eq!(format_event(&e, "dev1", utc), want, "{name}");
        }
    }

    #[test]
    fn summarize_cap() {
        let mut m = serde_json::Map::new();
        for k in ["a", "b", "c", "d", "e", "f", "g"] {
            m.insert(k.into(), k.repeat(40).into());
        }
        let s = summarize(&serde_json::to_vec(&m).unwrap());
        assert_eq!(s.chars().count(), MAX_SUMMARY, "{s}");
        assert!(s.ends_with('…'), "{s}");
    }

    #[test]
    fn event_printer() {
        let evs = [
            ev("dev1", 4, "", "agent.seen", r#"{"key":"dev1/s1"}"#),
            ev("dev2", 90, "", "agent.seen", "{}"),
            ev("dev1", 2, "", "agent.seen", "{}"),
        ];
        for json in [false, true] {
            let mut p = Printer::new(Vec::new(), json, "dev1".into());
            p.offset = utc;
            for e in &evs {
                p.event(e).unwrap();
            }
            p.resync().unwrap();
            assert_eq!(p.last_self, 4, "this device's newest");
            let out = String::from_utf8(p.w).unwrap();
            let lines: Vec<&str> = out.trim_end_matches('\n').split('\n').collect();
            assert_eq!(lines.len(), 4, "{out}");
            if json {
                assert_eq!(lines[3], r#"{"resync":true}"#);
                let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
                assert!(v["seq"] == 4 && v["deviceId"] == "dev1", "{out}");
                assert_eq!(
                    lines[0],
                    format!(
                        r#"{{"deviceId":"dev1","seq":4,"hlc":{},"actor":"","kind":"agent.seen","payload":{{"key":"dev1/s1"}}}}"#,
                        at(WHEN).0
                    )
                );
            } else {
                assert!(
                    lines[0].contains("key=dev1/s1")
                        && lines[1].contains("[dev2]")
                        && lines[3].starts_with("--"),
                    "{out}"
                );
            }
        }
    }

    #[test]
    fn split_kinds_like_go() {
        assert_eq!(
            split_kinds("agent., device.started,,"),
            vec!["agent.".to_string(), "device.started".to_string()]
        );
        assert!(split_kinds("").is_empty());
    }

    // ---- follow (I:follow_test.go) --------------------------------------

    const FOLLOW_TOKEN: &str = "follow-test-token";

    type AfterList = Box<dyn Fn(&store::Filter) + Send + Sync>;
    type WrapSub = Box<dyn Fn(store::Subscription) -> store::Subscription + Send + Sync>;

    /// Go `followBackend`: a real event log as device "dev1". `after_list`
    /// runs after `GET /v1/events` listed its rows; `wrap_sub` may rewrite
    /// a subscription.
    struct FollowBackend {
        st: store::Store,
        after_list: Mutex<Option<AfterList>>,
        wrap_sub: Mutex<Option<WrapSub>>,
    }

    impl irori_api::Backend for FollowBackend {
        fn version(&self) -> String {
            "0.0.0-test".into()
        }
        fn device(&self) -> irori_identity::DeviceJson {
            irori_identity::DeviceJson {
                id: "dev1".into(),
                name: "box".into(),
                os: "testos".into(),
                ..Default::default()
            }
        }
        fn agents(&self) -> Vec<presence::AgentCard> {
            vec![]
        }
        fn events(&self, f: &store::Filter) -> Result<Vec<store::Event>, String> {
            let evs = self.st.list(f).map_err(|e| e.to_string())?;
            if let Some(a) = &*self.after_list.lock().unwrap() {
                a(f);
            }
            Ok(evs)
        }
        fn subscribe_events(&self) -> store::Subscription {
            let sub = self.st.subscribe();
            match &*self.wrap_sub.lock().unwrap() {
                Some(w) => w(sub),
                None => sub,
            }
        }
        fn inbox(&self) -> Option<Arc<inbox::Inbox>> {
            None
        }
        fn coord(&self) -> Option<Arc<coord::Coord>> {
            None
        }
        fn capture(&self) -> Option<capture::Capture> {
            None
        }
        fn join_member(&self, _: &str, _: &str) -> Result<irori_api::Member, String> {
            Err("no members".into())
        }
        fn member_by_secret(&self, _: &str) -> Option<irori_api::Member> {
            None
        }
        fn member_stream(&self, _: &str, _: bool) {}
    }

    struct Follow {
        b: Arc<FollowBackend>,
        c: Client,
        stop: CancellationToken,
        _dir: tempfile::TempDir,
    }

    impl Drop for Follow {
        fn drop(&mut self) {
            self.stop.cancel();
        }
    }

    /// Go `startFollow`: a store for the backend, served on loopback.
    async fn start_follow() -> Follow {
        let dir = tempfile::tempdir().unwrap();
        let st = store::Store::open(
            dir.path().join("events.db"),
            "dev1",
            Arc::new(store::Clock::new()),
        )
        .unwrap();
        let b = Arc::new(FollowBackend {
            st,
            after_list: Mutex::new(None),
            wrap_sub: Mutex::new(None),
        });
        let ln = irori_api::listen().await.unwrap();
        let port = ln.local_addr().unwrap().port();
        let stop = CancellationToken::new();
        let srv = irori_api::Server::new(b.clone(), FOLLOW_TOKEN);
        let s = stop.clone();
        tokio::spawn(async move { srv.serve(ln, s).await });
        Follow {
            b,
            c: Client::at(port, FOLLOW_TOKEN),
            stop,
            _dir: dir,
        }
    }

    /// A buffer safe to read while the follow writes it.
    #[derive(Clone, Default)]
    struct SyncBuf(Arc<Mutex<Vec<u8>>>);

    impl Write for SyncBuf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl SyncBuf {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    /// How long the follow tests wait for what must come: generous, since a
    /// loaded machine (a parallel build) stalls the loopback stream for
    /// seconds; a passing run waits only as long as it takes.
    const WAIT: Duration = Duration::from_secs(20);

    /// Waits until `out` contains `want`, failing after [`WAIT`].
    async fn wait_for(out: &SyncBuf, want: &str) {
        let deadline = tokio::time::Instant::now() + WAIT;
        while !out.text().contains(want) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "output never contained {want:?}:\n{}",
                out.text()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Go `runFollow`: the follow in the background until the test ends;
    /// it must return Ok on cancel.
    struct Running {
        stop: CancellationToken,
        task: Option<tokio::task::JoinHandle<Result<(), String>>>,
    }

    impl Running {
        async fn finish(mut self) {
            self.stop.cancel();
            let t = self.task.take().unwrap();
            let r = tokio::time::timeout(WAIT, t)
                .await
                .expect("follow_events did not return on cancel")
                .unwrap();
            assert_eq!(r, Ok(()), "follow_events");
        }
    }

    fn run_follow(f: &Follow, kinds: &[&str], since: i64, limit: i64, out: &SyncBuf) -> Running {
        let stop = CancellationToken::new();
        let (s, c, out) = (stop.clone(), f.c.clone(), out.clone());
        let kinds: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
        let task = tokio::spawn(async move {
            let mut p = Printer::new(out, true, "dev1".into());
            follow_events(&s, Path::new(""), c, &kinds, since, limit, &mut p).await
        });
        Running {
            stop,
            task: Some(task),
        }
    }

    fn append_name(st: &store::Store, kind: &str, name: &str) {
        st.append("irori", kind, &serde_json::json!({ "name": name }))
            .unwrap();
    }

    /// A resync before any own row was printed resumes from this device's
    /// top seq, so a dropped own row is replayed rather than lost.
    #[tokio::test(flavor = "multi_thread")]
    async fn follow_resync_before_own_row() {
        let f = start_follow().await;
        let first = Arc::new(Mutex::new(true));
        let opened = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let op = opened.clone();
        *f.b.wrap_sub.lock().unwrap() = Some(Box::new(move |inner: store::Subscription| {
            if !std::mem::replace(&mut *first.lock().unwrap(), false) {
                return inner;
            }
            // The stream's subscription exists: rows from here on reach it.
            op.store(true, std::sync::atomic::Ordering::SeqCst);
            // The first subscription drops its first row and signals a
            // resync, as the store does on overflow.
            let q = Arc::new(gocompat::sub::Queue::new(256));
            let out = store::Subscription::from_queue(q.clone());
            std::thread::spawn(move || {
                if inner.recv().is_none() {
                    q.close();
                    return;
                }
                q.push(store::Event::default());
                while inner.recv().is_some() {
                    if q.is_closed() {
                        return;
                    }
                }
            });
            out
        }));
        append_name(&f.b.st, "agent.status", "BEFORE");
        let out = SyncBuf::default();
        let run = run_follow(&f, &["agent.status"], irori_api::client::NO_REPLAY, 0, &out);
        // Let the stream open. A fixed sleep lost the race under load: a row
        // written before the subscription is never its first.
        let deadline = tokio::time::Instant::now() + WAIT;
        while !opened.load(std::sync::atomic::Ordering::SeqCst) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the stream never opened"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        append_name(&f.b.st, "agent.status", "DROPPED");
        wait_for(&out, r#"{"resync":true}"#).await;
        append_name(&f.b.st, "agent.status", "AFTER");
        wait_for(&out, "AFTER").await;
        let s = out.text();
        assert!(
            s.contains("DROPPED") && !s.contains("BEFORE"),
            "want DROPPED replayed and BEFORE not:\n{s}"
        );
        run.finish().await;
    }

    /// Rows written while the follow starts (after the top-seq lookup or
    /// the recent-rows list, before the stream opens), own or replicated,
    /// are printed, and exactly once.
    #[tokio::test(flavor = "multi_thread")]
    async fn follow_startup_gap() {
        #[derive(Clone, Copy)]
        enum Write_ {
            Own,
            Remote,
        }
        for (name, limit, on_top, write, old_row) in [
            (
                "own row after list, empty log",
                5,
                false,
                Write_::Own,
                false,
            ),
            (
                "own row after list, only other kinds",
                5,
                false,
                Write_::Own,
                true,
            ),
            (
                "own row after top, no recent rows",
                0,
                true,
                Write_::Own,
                false,
            ),
            (
                "own row after top, listed and replayed",
                5,
                true,
                Write_::Own,
                true,
            ),
            ("remote row after list", 5, false, Write_::Remote, true),
        ] {
            let f = start_follow().await;
            if old_row {
                append_name(&f.b.st, "device.started", "OLD");
            }
            let once = Mutex::new(false);
            let st = f.b.clone();
            *f.b.after_list.lock().unwrap() = Some(Box::new(move |fl: &store::Filter| {
                if fl.device_id.is_empty() == on_top {
                    return;
                }
                if std::mem::replace(&mut *once.lock().unwrap(), true) {
                    return;
                }
                let st = &st.st;
                match write {
                    Write_::Own => append_name(st, "agent.status", "GAP"),
                    Write_::Remote => st
                        .insert(&store::Event {
                            device_id: "dev2".into(),
                            seq: 1,
                            hlc: store::Clock::new().now(),
                            actor: "irori".into(),
                            kind: "agent.status".into(),
                            payload: br#"{"name":"GAP"}"#.to_vec(),
                            sig: vec![],
                        })
                        .unwrap(),
                }
            }));
            let out = SyncBuf::default();
            let run = run_follow(
                &f,
                &["agent.status"],
                irori_api::client::NO_REPLAY,
                limit,
                &out,
            );
            wait_for(&out, "GAP").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            append_name(&f.b.st, "agent.status", "AFTER");
            wait_for(&out, "AFTER").await;
            assert_eq!(
                out.text().matches("GAP").count(),
                1,
                "{name}: GAP printed:\n{}",
                out.text()
            );
            run.finish().await;
            // The hook holds a handle on the backend: drop it before the
            // next case.
            *f.b.after_list.lock().unwrap() = None;
        }
    }

    /// An own row too big for one stream line is reported once and
    /// skipped; the follow moves past it instead of replaying it on every
    /// reconnect.
    #[tokio::test(flavor = "multi_thread")]
    async fn follow_oversized_own_row() {
        let f = start_follow().await;
        append_name(&f.b.st, "agent.status", &"x".repeat(9 << 20));
        let out = SyncBuf::default();
        let run = run_follow(&f, &[], 0, 0, &out);
        wait_for(&out, r#"{"resync":true}"#).await;
        append_name(&f.b.st, "agent.status", "AFTER");
        wait_for(&out, "AFTER").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            out.text().matches(r#"{"resync":true}"#).count(),
            1,
            "resyncs:\n{:.400}",
            out.text()
        );
        run.finish().await;
    }
}
