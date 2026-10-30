//! `irori-dev verify` (architecture.md §6.3 "Rows", §8 M2): on a copy of a
//! data dir,
//!
//! 1. every row of `irori.db` (events, refused_events) has a signature the
//!    Chain accepts, and every RowHash (events, local_events,
//!    refused_events) equals Go's, from a `--go-hashes` file written by
//!    `TestDiffLiveRowHashes` (tools/goldens/store_golden_test.go);
//! 2. each honoured revoke's cutHash and each fork's otherHash match the rows;
//! 3. every `boards/*.db` verifies the same way against its BoardVerifier,
//!    pinned from its own `board.pin` rows as the Go Core pins it;
//! 4. with `--lockfile`, the Chain's device list equals `GET /v1/devices`
//!    from the running Go Core that lockfile names;
//! 5. with `--append <manifest>`, one `device.started` row is appended as
//!    this device (signed, through the Chain-verified store) and its
//!    `device\tseq\thash` is written for the Go reopen check.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use irori_identity::{
    BoardVerifier, Chain, DeviceJson, Identity, MemberPin, ROLE_DESKTOP, RowRef, VerifyError,
    decode,
};
use store::{Event, Filter, OpenOptions, Store};

use crate::http;

pub struct Opts {
    home: PathBuf,
    go_hashes: Option<PathBuf>,
    lockfile: Option<PathBuf>,
    append: Option<PathBuf>,
}

impl Opts {
    pub fn parse(args: &[String]) -> Result<Opts, String> {
        let mut home = None;
        let mut o = Opts {
            home: PathBuf::new(),
            go_hashes: None,
            lockfile: None,
            append: None,
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
                "home" => home = Some(PathBuf::from(val()?)),
                "go-hashes" => o.go_hashes = Some(PathBuf::from(val()?)),
                "lockfile" => o.lockfile = Some(PathBuf::from(val()?)),
                "append" => o.append = Some(PathBuf::from(val()?)),
                _ => return Err(format!("unknown flag {a}")),
            }
        }
        o.home = home.ok_or("--home is required")?;
        Ok(o)
    }
}

/// Failures collected over the run; any one fails the command.
#[derive(Default)]
struct Report {
    failures: Vec<String>,
}

impl Report {
    fn fail(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        println!("FAIL {m}");
        self.failures.push(m);
    }
}

/// Refuse a live data dir: open applies cuts and `--append` writes a row,
/// and the Go app holds the live `irori.db` with WAL. A dir holding
/// `irori.json` (a running Core's lockfile; copies are made without it) or
/// Go's default data dir (I:internal/paths/paths.go:12-27) is refused.
pub fn refuse_live(home: &Path) -> Result<(), String> {
    if home.join("irori.json").exists() {
        return Err(format!(
            "{} holds irori.json: a live data dir, or a copy that kept its lockfile. Run on a copy without it",
            home.display()
        ));
    }
    let default = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("irori"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".irori"))
    };
    if let Some(d) = default {
        let same = match (std::fs::canonicalize(home), std::fs::canonicalize(&d)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        if same {
            return Err(format!(
                "{} is Irori's live data dir; run on a copy",
                home.display()
            ));
        }
    }
    Ok(())
}

pub fn run(o: &Opts) -> ExitCode {
    if let Err(e) = refuse_live(&o.home) {
        eprintln!("irori-dev: {e}");
        return ExitCode::from(2);
    }
    let mut r = Report::default();
    if let Err(e) = verify(o, &mut r) {
        r.fail(e);
    }
    if r.failures.is_empty() {
        println!("verify: all passed");
        ExitCode::SUCCESS
    } else {
        println!("verify: {} failure(s)", r.failures.len());
        ExitCode::FAILURE
    }
}

fn verify(o: &Opts, r: &mut Report) -> Result<(), String> {
    let home = &o.home;
    let dj = home.join("device.json");
    let raw = std::fs::read(&dj).map_err(|e| format!("read {}: {e}", dj.display()))?;
    let dev = DeviceJson::parse(&raw).ok_or("device.json does not parse")?;
    // Unseals device.key and person.json (DPAPI works on a copy for the
    // same Windows user); nothing secret is printed.
    let ident =
        Arc::new(Identity::load(home, dev.clone(), None).map_err(|e| format!("identity: {e}"))?);
    let me = ident.device_id();
    let go = match &o.go_hashes {
        Some(p) => Some(read_go_hashes(p)?),
        None => {
            println!("note: no --go-hashes; RowHashes are computed but not compared with Go");
            None
        }
    };

    // 1-2. The private log, opened as the Core opens it but unsigned.
    let chain = new_chain(&ident);
    let db = home.join("irori.db");
    let st = Store::open_with(
        &db,
        &me,
        Arc::new(store::Clock::new()),
        OpenOptions {
            verifier: Some(chain.clone()),
            ..Default::default()
        },
    )
    .map_err(|e| format!("open {}: {e}", db.display()))?;
    let mut seen = BTreeSet::new();
    let sum = check_log(
        &st,
        "irori.db",
        &|ev| chain.verify_row(ev.into()),
        go.as_ref(),
        &mut seen,
        r,
    )?;
    for c in chain.cut_claims() {
        check_claim(&st, &c, r)?;
    }
    if let Some(go) = &go {
        let missing: Vec<_> = go.keys().filter(|k| !seen.contains(*k)).collect();
        if !missing.is_empty() {
            r.fail(format!(
                "irori.db: {} rows Go hashed that Rust did not read (first {:?})",
                missing.len(),
                missing[0]
            ));
        }
        println!(
            "irori.db: Go hashed {} rows, Rust matched {} of them",
            go.len(),
            sum.matched
        );
    }
    println!(
        "chain: person {}…, {} certified devices, {} bad identity rows, {} cut claims",
        short(&chain.person_id()),
        chain.devices().len(),
        chain.bad(),
        chain.cut_claims().len()
    );
    if chain.bad() > 0 {
        r.fail(format!(
            "chain: {} identity rows did not verify",
            chain.bad()
        ));
    }

    // 3. Board logs.
    check_boards(home, &me, r)?;

    // 4. The Chain's devices against Go's /v1/devices.
    match &o.lockfile {
        Some(lockfile) => check_devices(&chain, &dev, lockfile, r),
        None => println!("note: no --lockfile; the Chain is not compared with /v1/devices"),
    }

    // 5. A Rust append for the Go reopen check.
    if let Some(manifest) = &o.append {
        st.close().map_err(|e| e.to_string())?;
        drop(st);
        append_for_go(home, &ident, manifest, r)?;
    }
    Ok(())
}

fn new_chain(ident: &Identity) -> Arc<Chain> {
    let cur = ident.current();
    Arc::new(Chain::new(&cur.id, &cur.root_device_id, &ident.device_id()))
}

fn short(s: &str) -> &str {
    &s[..s.len().min(8)]
}

type Key = (String, String, i64);

/// `TestDiffLiveRowHashes` output: `table\tdevice\tseq\thash[\tother]`.
fn read_go_hashes(p: &Path) -> Result<BTreeMap<Key, String>, String> {
    let txt = std::fs::read_to_string(p).map_err(|e| format!("read {}: {e}", p.display()))?;
    let mut out = BTreeMap::new();
    for line in txt.lines().filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 4 {
            return Err(format!("{}: bad line", p.display()));
        }
        if f[0] == "forks" {
            continue;
        }
        let seq: i64 = f[2]
            .parse()
            .map_err(|_| format!("{}: bad seq", p.display()))?;
        out.insert((f[0].to_string(), f[1].to_string(), seq), f[3].to_string());
    }
    Ok(out)
}

#[derive(Default)]
struct Sum {
    matched: usize,
}

#[derive(Default)]
struct Counts {
    rows: usize,
    ok: usize,
    unsigned: usize,
    by_err: BTreeMap<String, usize>,
    hash_mismatch: usize,
    hash_missing: usize,
}

/// Verify and hash every row of one log.
fn check_log(
    st: &Store,
    name: &str,
    verify: &dyn Fn(&Event) -> Result<(), VerifyError>,
    go: Option<&BTreeMap<Key, String>>,
    seen: &mut BTreeSet<Key>,
    r: &mut Report,
) -> Result<Sum, String> {
    let mut sum = Sum::default();
    let mut hash = |table: &str, ev: &Event, c: &mut Counts, r: &mut Report| {
        let k = (table.to_string(), ev.device_id.clone(), ev.seq);
        let h = store::row_hash(ev);
        if let Some(go) = go {
            match go.get(&k) {
                Some(g) if *g == h => sum.matched += 1,
                Some(_) => {
                    c.hash_mismatch += 1;
                    if c.hash_mismatch <= 5 {
                        r.fail(format!(
                            "{name} {table} {}/{}: RowHash differs from Go",
                            ev.device_id, ev.seq
                        ));
                    }
                }
                None => c.hash_missing += 1,
            }
        }
        seen.insert(k);
    };
    let sig = |table: &str, ev: &Event, c: &mut Counts, r: &mut Report| {
        if ev.sig.is_empty() {
            c.unsigned += 1;
            if c.unsigned <= 5 {
                r.fail(format!(
                    "{name} {table} {}/{}: unsigned",
                    ev.device_id, ev.seq
                ));
            }
            return;
        }
        match verify(ev) {
            Ok(()) => c.ok += 1,
            Err(e) => {
                let kind = format!("{:?}", e.kind());
                let n = c.by_err.entry(kind.clone()).or_default();
                *n += 1;
                if *n <= 5 {
                    r.fail(format!(
                        "{name} {table} {}/{} ({}): {kind}",
                        ev.device_id, ev.seq, ev.kind
                    ));
                }
            }
        }
    };

    let mut ev_c = Counts::default();
    for ev in page_all(st, false)? {
        ev_c.rows += 1;
        sig("events", &ev, &mut ev_c, r);
        hash("events", &ev, &mut ev_c, r);
    }
    let mut loc_c = Counts::default();
    for ev in page_all(st, true)? {
        loc_c.rows += 1;
        hash("local_events", &ev, &mut loc_c, r);
    }
    let mut ref_c = Counts::default();
    for ev in &refused_all(st)? {
        ref_c.rows += 1;
        // Rows moved out by a cut were verified when they came in, and an
        // honoured revoke is verified before it is kept, so each holds a
        // signature its writer's cert accepts (`verify_row` checks the
        // cert and signature, not the cut). A writer whose own cert came
        // from a row past its certifier's cut has no cert now: counted as
        // NoCert, not failed.
        if !ev.sig.is_empty()
            && let Err(e) = verify(ev)
            && matches!(e, VerifyError::NoCert(_))
        {
            *ref_c.by_err.entry("NoCert (noted)".into()).or_default() += 1;
            hash("refused_events", ev, &mut ref_c, r);
            continue;
        }
        sig("refused_events", ev, &mut ref_c, r);
        hash("refused_events", ev, &mut ref_c, r);
    }
    let forks = st.forks().map_err(|e| e.to_string())?;
    let mut fork_bad = 0;
    for f in &forks {
        if f.reason == store::FORK_TWO_ROWS && store::row_hash(&f.other) != f.other_hash {
            fork_bad += 1;
            r.fail(format!(
                "{name} fork {}/{}: otherHash is not the other row's RowHash",
                f.other.device_id, f.other.seq
            ));
        }
    }
    for (table, c) in [
        ("events", &ev_c),
        ("local_events", &loc_c),
        ("refused_events", &ref_c),
    ] {
        let errs: Vec<String> = c.by_err.iter().map(|(k, n)| format!("{k}={n}")).collect();
        println!(
            "{name} {table}: {} rows, {} signatures ok, {} unsigned, errors [{}], RowHash vs Go: {} differ, {} not in Go's list",
            c.rows,
            c.ok,
            c.unsigned,
            errs.join(" "),
            c.hash_mismatch,
            c.hash_missing
        );
        if c.hash_missing > 0 {
            r.fail(format!(
                "{name} {table}: {} rows not in Go's hash list",
                c.hash_missing
            ));
        }
    }
    println!(
        "{name} forks: {} rows, {} with a wrong otherHash",
        forks.len(),
        fork_bad
    );
    Ok(sum)
}

/// Every row of `events` (or `local_events`), in `(hlc, device, seq)` order.
fn page_all(st: &Store, local: bool) -> Result<Vec<Event>, String> {
    let mut out: Vec<Event> = Vec::new();
    let mut f = Filter {
        limit: store::MAX_LIST_LIMIT,
        local,
        ..Default::default()
    };
    loop {
        let page = st.list(&f).map_err(|e| e.to_string())?;
        let Some(last) = page.last() else { break };
        f.after = last.cursor();
        out.extend(page);
    }
    Ok(out)
}

/// Every row of `refused_events`, in `(hlc, device, seq)` order.
fn refused_all(st: &Store) -> Result<Vec<Event>, String> {
    let mut out: Vec<Event> = Vec::new();
    let mut after = store::Cursor::default();
    loop {
        let page = st
            .refused_after(&after, store::MAX_LIST_LIMIT)
            .map_err(|e| e.to_string())?;
        let Some(last) = page.last() else { break };
        after = last.cursor();
        out.extend(page);
    }
    Ok(out)
}

/// An honoured revoke names the row at its cut by hash; the row held
/// there must have that hash, or a cut-hash fork must be recorded.
fn check_claim(st: &Store, c: &store::CutClaim, r: &mut Report) -> Result<(), String> {
    let held = st
        .list(&Filter {
            device_id: c.device_id.clone(),
            after_seq: c.seq - 1,
            limit: 1,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    let Some(ev) = held.into_iter().find(|e| e.seq == c.seq) else {
        println!("cut claim {}/{}: row not held here", c.device_id, c.seq);
        return Ok(());
    };
    if store::row_hash(&ev) != c.hash {
        let forked = st.forks().map_err(|e| e.to_string())?.iter().any(|f| {
            f.reason == store::FORK_CUT_HASH
                && f.other.device_id == c.device_id
                && f.other.seq == c.seq
        });
        if !forked {
            r.fail(format!(
                "cut claim {}/{}: held row has another hash and no fork is recorded",
                c.device_id, c.seq
            ));
        }
    }
    Ok(())
}

fn check_boards(home: &Path, me: &str, r: &mut Report) -> Result<(), String> {
    let dir = home.join("boards");
    let mut dbs: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "db"))
            .collect(),
        Err(_) => Vec::new(),
    };
    dbs.sort();
    println!("boards: {} board logs", dbs.len());
    for db in dbs {
        let id = db
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let name = format!("boards/{id}.db");
        // Pins first, as the Go Core's pinBoard reads them: local board.pin rows.
        let pins: Vec<MemberPin> = {
            let st = Store::open_with(
                &db,
                me,
                Arc::new(store::Clock::new()),
                OpenOptions {
                    board: true,
                    ..Default::default()
                },
            )
            .map_err(|e| format!("open {name}: {e}"))?;
            let evs = st
                .list(&Filter {
                    kinds: vec![store::kinds::BOARD_PIN.to_string()],
                    local: true,
                    limit: store::MAX_LIST_LIMIT,
                    ..Default::default()
                })
                .map_err(|e| e.to_string())?;
            evs.iter()
                .filter_map(|e| decode::<MemberPin>(&e.payload))
                .collect()
        };
        let v = Arc::new(BoardVerifier::new_for(&id, &[]));
        let mut pinned = 0;
        for p in &pins {
            if v.pin(p).is_ok() {
                pinned += 1;
            }
        }
        if pinned == 0 {
            r.fail(format!("{name}: pins nobody; nothing in it verifies"));
            continue;
        }
        let st = Store::open_with(
            &db,
            me,
            Arc::new(store::Clock::new()),
            OpenOptions {
                verifier: Some(v.clone()),
                board: true,
                ..Default::default()
            },
        )
        .map_err(|e| format!("open {name}: {e}"))?;
        let mut seen = BTreeSet::new();
        check_log(
            &st,
            &name,
            &|ev| v.verify_row(ev.into()),
            None,
            &mut seen,
            r,
        )?;
        for c in v.cut_claims() {
            check_claim(&st, &c, r)?;
        }
        println!("{name}: {pinned} pins, {} bad identity rows", v.bad());
    }
    Ok(())
}

fn check_devices(chain: &Chain, dev: &DeviceJson, lockfile: &Path, r: &mut Report) {
    let lock = match http::Lock::read(lockfile) {
        Ok(l) => l,
        Err(e) => return r.fail(format!("devices: {e}")),
    };
    let (status, body) = match http::get(&lock, "/v1/devices") {
        Ok(x) => x,
        Err(e) => return r.fail(format!("devices: GET /v1/devices: {e}")),
    };
    if status == 503 {
        return r.fail("devices: the Go Core has no devices layer (503)");
    }
    if status != 200 {
        return r.fail(format!("devices: GET /v1/devices answered {status}"));
    }
    let Ok(serde_json::Value::Array(rows)) = serde_json::from_slice::<serde_json::Value>(&body)
    else {
        return r.fail("devices: /v1/devices is not a JSON array");
    };
    let s = |v: &serde_json::Value, k: &str| {
        v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    let b = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    let got: Vec<(String, String, String, bool, bool)> = rows
        .iter()
        .map(|v| {
            (
                s(v, "deviceId"),
                s(v, "name"),
                s(v, "role"),
                b(v, "revoked"),
                b(v, "this"),
            )
        })
        .collect();
    // Go's Devices: this device first (name from device.json, role from its
    // cert or desktop), then every other Chain device in id order.
    let mut want = Vec::new();
    let role = chain
        .cert(&dev.id)
        .map(|c| c.role)
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| ROLE_DESKTOP.to_string());
    want.push((dev.id.clone(), dev.name.clone(), role, false, true));
    for d in chain.devices() {
        if d.cert.device_id == dev.id {
            continue;
        }
        want.push((
            d.cert.device_id.clone(),
            d.cert.label.clone(),
            d.cert.role.clone(),
            d.revoked,
            false,
        ));
    }
    println!(
        "devices: Chain {} (this + {} others), Go /v1/devices {}",
        want.len(),
        want.len() - 1,
        got.len()
    );
    for (i, w) in want.iter().enumerate() {
        let state = if w.3 { "revoked" } else { "certified" };
        match got.get(i) {
            Some(g) if g == w => println!("  ok   {} {} ({}, {state})", short(&w.0), w.1, w.2),
            Some(g) => r.fail(format!(
                "devices[{i}]: Chain {} {:?} role {} revoked {} this {}, Go {} {:?} role {} revoked {} this {}",
                short(&w.0), w.1, w.2, w.3, w.4, short(&g.0), g.1, g.2, g.3, g.4
            )),
            None => r.fail(format!("devices[{i}]: Chain has {}, Go has no row", short(&w.0))),
        }
    }
    for g in got.iter().skip(want.len()) {
        r.fail(format!(
            "devices: Go lists {} that the Chain does not",
            short(&g.0)
        ));
    }
}

/// Append one `device.started` row as this device, signed by the device
/// key and checked by a fresh Chain, as the Go Core does at start.
fn append_for_go(
    home: &Path,
    ident: &Arc<Identity>,
    manifest: &Path,
    r: &mut Report,
) -> Result<(), String> {
    let chain = new_chain(ident);
    let me = ident.device_id();
    let st = Store::open_with(
        home.join("irori.db"),
        &me,
        Arc::new(store::Clock::new()),
        OpenOptions {
            signer: Some(ident.clone()),
            verifier: Some(chain.clone()),
            ..Default::default()
        },
    )
    .map_err(|e| format!("open for append: {e}"))?;
    let dev = ident.self_device();
    let payload: BTreeMap<&str, serde_json::Value> = [
        ("headless", serde_json::Value::Bool(true)),
        ("name", dev.name.clone().into()),
        ("os", dev.os.clone().into()),
        ("version", "irori-rust-dev".into()),
    ]
    .into();
    let ev = st
        .append("irori", store::kinds::DEVICE_STARTED, &payload)
        .map_err(|e| format!("append: {e}"))?;
    if let Err(e) = chain.verify_row(RowRef::from(&ev)) {
        r.fail(format!("append: the Chain refuses Rust's own row: {e}"));
    }
    let h = store::row_hash(&ev);
    st.close().map_err(|e| e.to_string())?;
    std::fs::write(manifest, format!("{}\t{}\t{}\n", ev.device_id, ev.seq, h))
        .map_err(|e| format!("write {}: {e}", manifest.display()))?;
    println!(
        "append: device.started at {}/{} (RowHash {}…)",
        short(&ev.device_id),
        ev.seq,
        &h[..12]
    );
    Ok(())
}
