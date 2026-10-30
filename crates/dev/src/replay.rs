//! `irori-dev replay` (architecture.md §6.3 "Folds", §8 M3): a headless,
//! read-only Rust Core on a copy of a data dir, its views dumped for a
//! comparison with the Go Core's on an identical copy.
//!
//!   irori-dev replay --home <copy> --out <dir> [--now <rfc3339>]
//!   irori-dev replay --compare <dir>
//!
//! The first form opens the copy's log as the Core does (Chain-verified,
//! no signer), builds the mesh roster, capture, the inbox and the Board on
//! it, and writes each view, marshaled the Go way, to `<dir>/rust/`, plus
//! `<dir>/meta.json` (this device, its seq, the pinned "now" and the
//! timeline keys) for the Go side. It runs no sweep, tick, catch-up or
//! tail pass, appends nothing, and fails if the log's vector moved.
//! tools/goldens/core_replay_test.go then starts the Go Core on its own
//! copy and writes the same views to `<dir>/go/`; the second form compares
//! the two directories file by file.
//!
//! The views are the services' own values (Go `json.Marshal` of what
//! Coord.Board, Inbox.List and friends return), not the API layer's
//! nil-to-`[]` wrappers, which the Rust API (M4) adds.
//!
//! Wiring here stands in for the `core` crate until M4 builds it:
//! - one mesh roster (coord::roster::Roster, no local presence, every
//!   other device offline) behind the coord, inbox and capture Roster
//!   traits, mapping its AgentCard onto each crate's projection;
//! - the Chain's `is_owner` and cuts for coord and the inbox;
//! - capture's PR owners for the inbox;
//! - the real scrubber and blob store for capture;
//! - capture's decisions (`Capture::decisions`), checked against the
//!   worktree as Go's capture.Decisions does.
//!
//! Output is counts and view names, never row contents.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use coord::decision;
use gocompat::json::marshal;
use gocompat::time::GoTime;
use irori_identity::{Chain, DeviceJson, Identity};
use store::{Filter, OpenOptions, Store};

pub struct Opts {
    home: Option<PathBuf>,
    out: Option<PathBuf>,
    now: Option<String>,
    compare: Option<PathBuf>,
}

impl Opts {
    pub fn parse(args: &[String]) -> Result<Opts, String> {
        let mut o = Opts {
            home: None,
            out: None,
            now: None,
            compare: None,
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let (flag, inline) = match a.split_once('=') {
                Some((f, v)) => (f, Some(v.to_string())),
                None => (a.as_str(), None),
            };
            let mut val = || {
                inline
                    .clone()
                    .or_else(|| it.next().cloned())
                    .ok_or_else(|| format!("{flag} needs a value"))
            };
            match flag.trim_start_matches('-') {
                "home" => o.home = Some(PathBuf::from(val()?)),
                "out" => o.out = Some(PathBuf::from(val()?)),
                "now" => o.now = Some(val()?),
                "compare" => o.compare = Some(PathBuf::from(val()?)),
                _ => return Err(format!("unknown flag {a}")),
            }
        }
        match (&o.home, &o.out, &o.compare) {
            (Some(_), Some(_), None) | (None, None, Some(_)) => Ok(o),
            _ => Err("give --home and --out, or --compare alone".into()),
        }
    }
}

pub fn run(o: &Opts) -> ExitCode {
    if let Some(dir) = &o.compare {
        return match compare(dir) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(e) => {
                eprintln!("irori-dev: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let (Some(home), Some(out)) = (&o.home, &o.out) else {
        return ExitCode::from(2);
    };
    if let Err(e) = crate::verify::refuse_live(home) {
        eprintln!("irori-dev: {e}");
        return ExitCode::from(2);
    }
    match dump(home, out, o.now.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            println!("FAIL {e}");
            ExitCode::FAILURE
        }
    }
}

/// Timeline pages per key, at most (100 turns each).
const MAX_PAGES: usize = 1000;
/// The list limit the views are read with (Go's MaxCaptureLimit and the
/// inbox list cap).
const LIST: usize = 500;

/// The adapters: one mesh roster, seen through each crate's trait.
struct CoordRoster(Arc<coord::roster::Roster>);
struct InboxRoster(Arc<coord::roster::Roster>);
struct CaptureRoster(Arc<coord::roster::Roster>);

impl coord::roster::Agents for CoordRoster {
    fn agents(&self) -> Vec<coord::roster::AgentCard> {
        self.0.agents()
    }
}

impl inbox::Roster for InboxRoster {
    fn agents(&self) -> Vec<inbox::AgentCard> {
        self.0
            .agents()
            .into_iter()
            .map(|c| inbox::AgentCard {
                key: c.key,
                device_id: c.device_id,
                session_id: c.session_id,
                agent: c.agent,
                name: c.name,
                status: c.status,
                repo_key: c.repo_key,
                permission_mode: c.permission_mode,
            })
            .collect()
    }
}

impl capture::Roster for CaptureRoster {
    fn agents(&self) -> Vec<capture::AgentCard> {
        self.0
            .agents()
            .into_iter()
            .map(|c| capture::AgentCard {
                key: c.key,
                session_id: c.session_id,
                cwd: c.cwd,
                status: c.status,
                name: c.name,
                title: c.title,
                repo_key: c.repo_key,
            })
            .collect()
    }
}

struct Scrub(scrub::Scrubber);

impl capture::Scrubber for Scrub {
    fn string(&self, s: &str) -> String {
        self.0.string(s)
    }
    fn skip_file(&self, path: &str) -> bool {
        scrub::skip_file(path)
    }
}

struct Blobs(blob::Store);

impl capture::Blobs for Blobs {
    fn put(&self, data: &[u8]) -> Result<String, String> {
        self.0.put(data).map_err(|e| e.to_string())
    }
    fn get(&self, id: &str) -> Result<Vec<u8>, String> {
        self.0.get(id).map_err(|e| e.to_string())
    }
}

/// What the Go side reads back: the same device, keys and clock.
#[derive(serde::Serialize, serde::Deserialize)]
struct Meta {
    device: String,
    /// This device's newest seq when the Rust views were taken; the Go
    /// side reports the rows its Core appended after it.
    seq: i64,
    /// The pinned "now" of the Board fold (RFC 3339, local offset).
    now: String,
    #[serde(rename = "timelineKeys")]
    timeline_keys: Vec<String>,
    /// Agent keys the Board is also viewed as (coord.md §3.15): the agents
    /// that hold or made a card, sorted, at most [`AGENT_VIEWERS`].
    #[serde(rename = "boardViewers")]
    board_viewers: Vec<String>,
}

/// How many agent viewers the Board is compared for.
const AGENT_VIEWERS: usize = 5;

fn dump(home: &Path, out: &Path, now: Option<&str>) -> Result<(), String> {
    let raw = std::fs::read(home.join("device.json")).map_err(|e| format!("device.json: {e}"))?;
    let dev = DeviceJson::parse(&raw).ok_or("device.json does not parse")?;
    let ident = Identity::load(home, dev, None).map_err(|e| format!("identity: {e}"))?;
    let me = ident.device_id();
    let cur = ident.current();
    let chain = Arc::new(Chain::new(&cur.id, &cur.root_device_id, &me));
    let st = Arc::new(
        Store::open_with(
            home.join("irori.db"),
            &me,
            Arc::new(store::Clock::new()),
            OpenOptions {
                verifier: Some(chain.clone()),
                ..Default::default()
            },
        )
        .map_err(|e| format!("open irori.db: {e}"))?,
    );
    let before = st.vector().map_err(|e| e.to_string())?;

    let now = match now {
        Some(s) => GoTime::parse_rfc3339(s).ok_or_else(|| format!("--now {s}: not RFC 3339"))?,
        None => capture::gotime::now(),
    };
    let now_s = now.format_rfc3339_nano();

    let revoked = {
        let ch = chain.clone();
        Arc::new(move |d: &str| ch.cut(d).is_some()) as Arc<dyn Fn(&str) -> bool + Send + Sync>
    };
    let is_owner = {
        let ch = chain.clone();
        Arc::new(move |k: &str| ch.is_owner(k)) as Arc<dyn Fn(&str) -> bool + Send + Sync>
    };

    // The mesh roster: no local presence (Go's side runs on an empty
    // Claude dir), every other device offline (no network).
    let roster = coord::roster::Roster::new(coord::roster::Options {
        store: st.clone(),
        self_id: me.clone(),
        local: None,
        liveness: None,
        revoked: Some(revoked.clone()),
    })
    .map_err(|e| format!("roster: {e}"))?;

    let claude = tempfile::tempdir().map_err(|e| e.to_string())?;
    let blobs = blob::Store::open(home.join("blobs")).map_err(|e| format!("blobs: {e}"))?;
    let none: [&str; 0] = [];
    let copts = capture::Options::new(
        &claude.path().to_string_lossy(),
        &home.to_string_lossy(),
        &me,
        Arc::new(Scrub(scrub::Scrubber::new(&none))),
        Arc::new(Blobs(blobs)),
    );
    let cap = capture::Capture::new(
        st.clone(),
        Some(Arc::new(CaptureRoster(roster.clone()))),
        copts,
    )
    .map_err(|e| format!("capture: {e}"))?;

    let ib = {
        let c = cap.clone();
        inbox::Inbox::new(
            st.clone(),
            Arc::new(InboxRoster(roster.clone())),
            inbox::Options {
                device_id: me.clone(),
                now: Some(Arc::new(capture::gotime::now)),
                pr_owners: Some(Arc::new(move |k: &str| c.pr_owners(k))),
                is_owner: Some(is_owner.clone()),
                revoked: Some(revoked.clone()),
                ..Default::default()
            },
        )
        .map_err(|e| format!("inbox: {e}"))?
    };
    let ib = Arc::new(ib);

    let co = coord::Coord::new(
        st.clone(),
        Some(Arc::new(CoordRoster(roster.clone()))),
        coord::Options {
            now: Some(Arc::new(move || now)),
            is_owner: Some(is_owner.clone()),
            revoked: Some(revoked.clone()),
            ..Default::default()
        },
    )
    .map_err(|e| format!("coord: {e}"))?;
    let co = Arc::new(co);

    // Capture's search dispatches the Board and inbox kinds to these, as
    // core wires them (capture.md §2.7).
    cap.add_source(co.clone());
    cap.add_source(ib.clone());

    let dir = out.join("rust");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let w = |name: &str, bytes: Vec<u8>| -> Result<(), String> {
        std::fs::write(dir.join(name), bytes).map_err(|e| format!("write {name}: {e}"))
    };

    // Board.
    let user = format!("user:{me}");
    let board_user = co.board(&coord::BoardFilter {
        viewer: user.clone(),
        done_limit: coord::MAX_DONE_CARD,
        ..Default::default()
    });
    w("board-user.json", m(&board_user)?)?;
    let board_all = co.board(&coord::BoardFilter {
        done_limit: coord::MAX_DONE_CARD,
        ..Default::default()
    });
    w("board-all.json", m(&board_all)?)?;
    let mut viewers = BTreeSet::new();
    for t in co.list_tasks(&coord::TaskFilter::default()) {
        for k in t.holders.iter().chain([&t.created_by]) {
            if k.contains('/') && !k.contains(':') {
                viewers.insert(k.clone());
            }
        }
    }
    let viewers: Vec<String> = viewers.into_iter().take(AGENT_VIEWERS).collect();
    for (i, k) in viewers.iter().enumerate() {
        let b = co.board(&coord::BoardFilter {
            viewer: k.clone(),
            done_limit: coord::MAX_DONE_CARD,
            ..Default::default()
        });
        w(&format!("board-agent-{i:03}.json"), m(&b)?)?;
    }
    w("leases.json", m(&co.leases())?)?;

    // Inbox.
    w("inbox-list.json", m(&ib.list(LIST))?)?;
    w("inbox-held.json", m(&ib.held_history(LIST))?)?;
    w("inbox-open.json", m(&ib.open())?)?;
    w("inbox-policy.json", m(&ib.policy())?)?;

    // Decisions.
    w("decisions.json", decisions(&cap)?)?;

    // Timelines: every agent with a turn or summary row.
    let keys = timeline_keys(&st)?;
    for (i, k) in keys.iter().enumerate() {
        for sub in [false, true] {
            let mut pages = Vec::new();
            let mut q = capture::TimelineQuery {
                key: k.clone(),
                limit: 100,
                subagents: sub,
                ..Default::default()
            };
            for _ in 0..MAX_PAGES {
                let tl = cap.timeline(&q);
                let more = tl.more;
                let last = tl.entries.last().map(|e| e.at);
                pages.push(marshal(&tl).map_err(|e| e.to_string())?);
                match (more, last) {
                    (true, Some(at)) => q.before = at,
                    _ => break,
                }
            }
            let name = format!("timeline{}-{i:03}.json", if sub { "-sub" } else { "" });
            w(&name, join_array(&pages))?;
        }
    }

    let after = st.vector().map_err(|e| e.to_string())?;
    if before != after {
        return Err("the Rust replay changed the log's vector; it must append nothing".into());
    }
    let meta = Meta {
        device: me.clone(),
        seq: after.get(&me).copied().unwrap_or(0),
        now: now_s,
        timeline_keys: keys.clone(),
        board_viewers: viewers,
    };
    std::fs::write(
        out.join("meta.json"),
        serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("write meta.json: {e}"))?;

    let total: i64 = after.values().sum();
    println!(
        "replay: rust views written: {} rows by vector over {} devices, log unchanged; board {} cards (doneTotal {}), {} leases; inbox {} deliveries ({} open, {} held); {} decisions; {} timeline keys",
        total,
        after.len(),
        board_user.needs_you.len()
            + board_user.blocked.len()
            + board_user.working.len()
            + board_user.open.len()
            + board_user.done.len(),
        board_user.done_total,
        co.leases().len(),
        ib.list(LIST).len(),
        ib.open().len(),
        ib.held_history(LIST).len(),
        decision_count(&cap),
        keys.len()
    );
    drop(cap);
    drop(co);
    drop(ib);
    st.close().map_err(|e| e.to_string())?;
    Ok(())
}

/// Marshaled the Go way.
fn m<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, String> {
    marshal(v).map_err(|e| e.to_string())
}

fn join_array(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"[".to_vec();
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.extend_from_slice(it);
    }
    out.push(b']');
    out
}

fn all_decisions() -> decision::Filter {
    decision::Filter {
        all: true,
        limit: LIST as i64,
        ..Default::default()
    }
}

fn decision_count(cap: &capture::Capture) -> usize {
    cap.decisions(&all_decisions()).len()
}

/// Go capture.Decisions(Filter{All, Limit 500}): each anchored decision
/// re-checked against its own root when that directory exists, else a
/// live card's root in its repo. This replay has no live local cards and
/// other devices' cwds are not on this disk, so the fallback is "" on both
/// sides.
fn decisions(cap: &capture::Capture) -> Result<Vec<u8>, String> {
    marshal(&cap.decisions(&all_decisions())).map_err(|e| e.to_string())
}

/// Every agent key with a `capture.turn` or `capture.summary` row, sorted.
fn timeline_keys(st: &Store) -> Result<Vec<String>, String> {
    let mut keys = BTreeSet::new();
    let mut after = store::Cursor::default();
    loop {
        let evs = st
            .list(&Filter {
                kinds: vec!["capture.turn".into(), "capture.summary".into()],
                after: after.clone(),
                limit: store::MAX_LIST_LIMIT,
                ..Default::default()
            })
            .map_err(|e| format!("list capture rows: {e}"))?;
        for ev in &evs {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&ev.payload)
                && let Some(k) = v.get("key").and_then(|k| k.as_str())
                && !k.is_empty()
            {
                keys.insert(k.to_string());
            }
            after = ev.cursor();
        }
        if (evs.len() as i64) < store::MAX_LIST_LIMIT {
            break;
        }
    }
    Ok(keys.into_iter().collect())
}

/// Compares `<dir>/rust` with `<dir>/go`, file by file.
fn compare(dir: &Path) -> Result<bool, String> {
    let names = |d: &Path| -> Result<BTreeSet<String>, String> {
        let mut s = BTreeSet::new();
        for e in std::fs::read_dir(d).map_err(|e| format!("{}: {e}", d.display()))? {
            let e = e.map_err(|e| e.to_string())?;
            s.insert(e.file_name().to_string_lossy().into_owned());
        }
        Ok(s)
    };
    let (rd, gd) = (dir.join("rust"), dir.join("go"));
    let (rn, gn) = (names(&rd)?, names(&gd)?);
    let mut ok = true;
    // (byte-equal, equal once normalized, differ) per group.
    let mut summary: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for n in rn.union(&gn) {
        let e = summary.entry(group_of(n)).or_default();
        let (Some(r), Some(g)) = (
            std::fs::read(rd.join(n)).ok(),
            std::fs::read(gd.join(n)).ok(),
        ) else {
            println!(
                "DIFF {n}: only on the {} side",
                if rn.contains(n) { "Rust" } else { "Go" }
            );
            e.2 += 1;
            ok = false;
            continue;
        };
        if r == g {
            e.0 += 1;
            continue;
        }
        let rv: Result<serde_json::Value, _> = serde_json::from_slice(&r);
        let gv: Result<serde_json::Value, _> = serde_json::from_slice(&g);
        let (Ok(rv), Ok(gv)) = (rv, gv) else {
            println!("DIFF {n}: not JSON on one side");
            e.2 += 1;
            ok = false;
            continue;
        };
        if rv == gv {
            println!("DIFF {n}: same values, different bytes (key order or escaping)");
            e.2 += 1;
            ok = false;
            continue;
        }
        let (rn_, gn_) = (normalize(n, rv.clone()), normalize(n, gv.clone()));
        if rn_ == gn_ {
            e.1 += 1;
            continue;
        }
        e.2 += 1;
        ok = false;
        let mut diffs = Vec::new();
        diff_paths(&rn_, &gn_, String::new(), &mut diffs);
        println!(
            "DIFF {n}: {} differing paths after normalizing (first: {})",
            diffs.len(),
            diffs.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
        );
    }
    for (g, (eq, norm, ne)) in &summary {
        println!("replay: {g}: {eq} byte-equal, {norm} equal after normalizing, {ne} differ");
    }
    println!("replay: normalized: {NORMALIZED}");
    // The Go views are taken while the Go Core's loops run; a row it
    // appended beyond its start row (a self-cert, an inbox hold) can change
    // them, so such a run proves nothing and fails, naming the kinds.
    match std::fs::read(dir.join("go-appended.json")) {
        Ok(b) => {
            println!(
                "replay: rows the Go Core appended on its copy: {}",
                String::from_utf8_lossy(&b).trim()
            );
            match serde_json::from_slice::<BTreeMap<String, i64>>(&b) {
                Ok(kinds) => {
                    let extra: Vec<&String> = kinds
                        .keys()
                        .filter(|k| k.as_str() != store::kinds::DEVICE_STARTED)
                        .collect();
                    if !extra.is_empty() {
                        println!(
                            "FAIL go-appended.json: the Go Core appended {extra:?} beyond its start row; its views were taken while its loops ran, so those rows can change them and the views can't be called equal"
                        );
                        ok = false;
                    }
                }
                Err(e) => {
                    println!("FAIL go-appended.json: not a kind count: {e}");
                    ok = false;
                }
            }
        }
        Err(e) => {
            println!(
                "FAIL go-appended.json: {e}; cannot tell whether Go's views raced its own appends"
            );
            ok = false;
        }
    }
    println!(
        "replay: {}",
        if ok {
            "all views equal"
        } else {
            "views differ"
        }
    );
    Ok(ok)
}

/// What `normalize` changes, and why each is meant to differ.
const NORMALIZED: &str = "inbox-open: sorted by id (Go ranges over a map); timelines: entries flattened across pages, and within each run of equal `at` only the commit entries (Go ranges over its commit map) and the open-turn entries (Go sorts them unstably) are sorted among their own slots; turns, summaries and PRs keep their order, page sizes and `more` kept";

/// Which entries of a timeline Go may order differently on equal `at`:
/// commits and open turns. `None` for an entry whose order Go fixes.
fn tie_class(e: &serde_json::Value) -> Option<&'static str> {
    match e.get("kind").and_then(serde_json::Value::as_str) {
        Some("commit") => Some("commit"),
        Some("turn")
            if e.pointer("/turn/open")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false) =>
        {
            Some("open-turn")
        }
        _ => None,
    }
}

/// The view with the orders Go leaves to map iteration made canonical.
fn normalize(name: &str, v: serde_json::Value) -> serde_json::Value {
    use serde_json::{Value, json};
    let canon = |v: &Value| serde_json::to_string(v).unwrap_or_default();
    if name == "inbox-open.json" {
        if let Value::Array(mut a) = v {
            a.sort_by_key(|d| {
                d.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            });
            return Value::Array(a);
        }
        return v;
    }
    if name.starts_with("timeline") {
        let Value::Array(pages) = v else { return v };
        let mut shape = Vec::new();
        let mut entries: Vec<Value> = Vec::new();
        for p in pages {
            let es = p
                .get("entries")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            shape.push(json!({"key": p.get("key"), "more": p.get("more"), "n": es.len()}));
            entries.extend(es);
        }
        let mut out = Vec::with_capacity(entries.len());
        let mut i = 0;
        while i < entries.len() {
            let at = entries[i].get("at").cloned();
            let mut j = i + 1;
            while j < entries.len() && entries[j].get("at").cloned() == at {
                j += 1;
            }
            // Go appends turns, summaries, PRs, then commits from a map, and
            // sorts them stably by `at`; open turns come first, sorted
            // unstably. So only commits among commits, and open turns among
            // open turns, may differ in order: each class is sorted in the
            // slots it holds, and every other entry stays where it is.
            let mut run: Vec<Value> = entries[i..j].to_vec();
            for class in ["commit", "open-turn"] {
                let slots: Vec<usize> = (0..run.len())
                    .filter(|&k| tie_class(&run[k]) == Some(class))
                    .collect();
                let mut vals: Vec<Value> = slots.iter().map(|&k| run[k].clone()).collect();
                vals.sort_by_key(|e| canon(e));
                for (&k, v) in slots.iter().zip(vals) {
                    run[k] = v;
                }
            }
            out.extend(run);
            i = j;
        }
        return json!({"pages": shape, "entries": out});
    }
    v
}

fn group_of(name: &str) -> &'static str {
    if name.starts_with("board") || name.starts_with("leases") {
        "board"
    } else if name.starts_with("inbox") {
        "inbox"
    } else if name.starts_with("decisions") {
        "decisions"
    } else if name.starts_with("timeline") {
        "timeline"
    } else {
        "other"
    }
}

/// JSON paths (no values) where `a` and `b` differ.
fn diff_paths(a: &serde_json::Value, b: &serde_json::Value, at: String, out: &mut Vec<String>) {
    use serde_json::Value;
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => diff_paths(p, q, format!("{at}.{k}"), out),
                    (p, _) => out.push(format!(
                        "{at}.{k} only {}",
                        if p.is_some() { "Rust" } else { "Go" }
                    )),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                out.push(format!("{at} length Rust {} Go {}", x.len(), y.len()));
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                diff_paths(p, q, format!("{at}[{i}]"), out);
            }
        }
        _ if a != b => out.push(if at.is_empty() { "$".into() } else { at }),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tl(entries: serde_json::Value) -> serde_json::Value {
        normalize(
            "timeline-000.json",
            json!([{"key": "k", "more": false, "entries": entries}]),
        )
    }

    // Only commits (Go's map) and open turns (Go's unstable sort) may
    // trade places on equal `at`; turns, summaries and PRs may not.
    #[test]
    fn timeline_ties_only_free_commits_and_open_turns() {
        let at = "2026-10-29T00:00:00Z";
        let turn = |id: &str| json!({"kind": "turn", "at": at, "turn": {"id": id}});
        let open = |id: &str| json!({"kind": "turn", "at": at, "turn": {"id": id, "open": true}});
        let commit = |sha: &str| json!({"kind": "commit", "at": at, "commit": {"sha": sha}});
        let sum = json!({"kind": "summary", "at": at, "summary": {"id": "s"}});
        assert_eq!(
            tl(json!([turn("a"), sum, commit("1"), commit("2")])),
            tl(json!([turn("a"), sum, commit("2"), commit("1")]))
        );
        assert_eq!(
            tl(json!([open("x"), open("y"), turn("a")])),
            tl(json!([open("y"), open("x"), turn("a")]))
        );
        assert_ne!(
            tl(json!([turn("a"), turn("b")])),
            tl(json!([turn("b"), turn("a")]))
        );
        assert_ne!(tl(json!([turn("a"), sum])), tl(json!([sum, turn("a")])));
        assert_ne!(
            tl(json!([turn("a"), commit("1")])),
            tl(json!([commit("1"), turn("a")]))
        );
    }
}
