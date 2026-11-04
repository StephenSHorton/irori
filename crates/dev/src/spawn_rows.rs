//! Card #467: v0.11.0's spawn hot loop (a card with no checkout, #464)
//! wrote ~3.4M `task.spawn` rows. Two checks on a copy of a data dir:
//!
//!   irori-dev spawn-fold --home <copy> --person <id> --root <device> --out <dir>
//!                        [--now <rfc3339>]
//!       opens the copy's log as the Core does (Chain-verified, no signer, so
//!       no secret keys are read), folds the Board, and writes every card's
//!       whole folded state (`Debug`, spawns included) and the Board views to
//!       `<dir>`. It prints the open and fold times. Two dirs (before and
//!       after `compact-spawn`) should be identical: `diff -r a b`.
//!
//!   irori-dev compact-spawn --home <copy> --floor <device>=<seq|top> ...
//!                           [--dry-run] [--no-vacuum]
//!       deletes superseded `task.spawn` rows: a row is superseded when the
//!       same device wrote a row for the same card part (taskId, step,
//!       subtask) by the same writer (`by`) with a higher gen. The fold
//!       keeps only the newest gen of a part (coord board/spawn.rs
//!       fold_spawn), so the folded state does not change. Only rows at or
//!       below the device's floor go, and never a device's newest row:
//!       a peer whose vector for that device is below a deleted seq refuses
//!       every later row as a Gap (store db.rs insert_one) and stalls on it
//!       for good, so the floor must be at or below every peer's vector for
//!       that device. `top` (the device's newest seq) is only for measuring
//!       what a compaction could reach; it is safe only when every peer holds
//!       that device's whole log. A device with no --floor keeps every row.
//!
//! Both refuse a live data dir (verify::refuse_live). Output is counts,
//! times and sizes, never payloads or keys.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use gocompat::json::marshal;
use gocompat::time::GoTime;
use irori_identity::Chain;
use store::{OpenOptions, Store};

use crate::verify::refuse_live;

pub struct FoldOpts {
    home: PathBuf,
    out: PathBuf,
    person: String,
    root: String,
    now: Option<String>,
}

fn take(it: &mut std::slice::Iter<'_, String>, flag: &str) -> Result<String, String> {
    it.next()
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

impl FoldOpts {
    pub fn parse(args: &[String]) -> Result<FoldOpts, String> {
        let (mut home, mut out, mut person, mut root, mut now) = (None, None, None, None, None);
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--home" => home = Some(PathBuf::from(take(&mut it, a)?)),
                "--out" => out = Some(PathBuf::from(take(&mut it, a)?)),
                "--person" => person = Some(take(&mut it, a)?),
                "--root" => root = Some(take(&mut it, a)?),
                "--now" => now = Some(take(&mut it, a)?),
                _ => return Err(format!("spawn-fold: unknown flag {a}")),
            }
        }
        Ok(FoldOpts {
            home: home.ok_or("spawn-fold needs --home")?,
            out: out.ok_or("spawn-fold needs --out")?,
            person: person.ok_or("spawn-fold needs --person (person.json personId)")?,
            root: root.ok_or("spawn-fold needs --root (person.json rootDeviceId)")?,
            now,
        })
    }
}

/// This device's id from the copy's device.json (public fields only).
fn device_id(home: &Path) -> Result<String, String> {
    let raw = std::fs::read(home.join("device.json")).map_err(|e| format!("device.json: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|e| format!("device.json: {e}"))?;
    v["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "device.json has no id".to_string())
}

pub fn fold(o: &FoldOpts) -> ExitCode {
    match fold_run(o) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("irori-dev: spawn-fold: {e}");
            ExitCode::FAILURE
        }
    }
}

fn fold_run(o: &FoldOpts) -> Result<(), String> {
    refuse_live(&o.home)?;
    let me = device_id(&o.home)?;
    let now = match &o.now {
        Some(s) => GoTime::parse_rfc3339(s).ok_or_else(|| format!("--now {s}: not RFC 3339"))?,
        None => return Err("spawn-fold needs --now, so two runs fold at the same instant".into()),
    };
    let chain = Arc::new(Chain::new(&o.person, &o.root, &me));
    let t0 = Instant::now();
    let st = Arc::new(
        Store::open_with(
            o.home.join("irori.db"),
            &me,
            Arc::new(store::Clock::new()),
            OpenOptions {
                verifier: Some(chain.clone()),
                ..Default::default()
            },
        )
        .map_err(|e| format!("open irori.db: {e}"))?,
    );
    let open_ms = t0.elapsed().as_millis();
    let before = st.vector().map_err(|e| e.to_string())?;

    let is_owner = {
        let ch = chain.clone();
        Arc::new(move |k: &str| ch.is_owner(k)) as Arc<dyn Fn(&str) -> bool + Send + Sync>
    };
    let revoked = {
        let ch = chain.clone();
        Arc::new(move |d: &str| ch.cut(d).is_some()) as Arc<dyn Fn(&str) -> bool + Send + Sync>
    };
    let t1 = Instant::now();
    let co = coord::Coord::new(
        st.clone(),
        None,
        coord::Options {
            now: Some(Arc::new(move || now)),
            is_owner: Some(is_owner),
            revoked: Some(revoked),
            ..Default::default()
        },
    )
    .map_err(|e| format!("coord: {e}"))?;
    let fold_ms = t1.elapsed().as_millis();

    std::fs::create_dir_all(&o.out).map_err(|e| format!("{}: {e}", o.out.display()))?;
    let w = |name: &str, bytes: &[u8]| -> Result<(), String> {
        std::fs::write(o.out.join(name), bytes).map_err(|e| format!("write {name}: {e}"))
    };
    let m = |v: &dyn erased::Ser| v.bytes();
    let board_all = co.board(&coord::BoardFilter {
        done_limit: coord::MAX_DONE_CARD,
        ..Default::default()
    });
    w("board-all.json", &m(&board_all)?)?;
    let board_user = co.board(&coord::BoardFilter {
        viewer: format!("user:{me}"),
        done_limit: coord::MAX_DONE_CARD,
        ..Default::default()
    });
    w("board-user.json", &m(&board_user)?)?;
    w("leases.json", &m(&co.leases())?)?;
    let mut tasks = co.list_tasks(&coord::TaskFilter::default());
    tasks.sort_by(|a, b| a.id.cmp(&b.id));
    let mut dump = String::new();
    let mut spawns = 0usize;
    for t in &tasks {
        spawns += t.spawns.len();
        let _ = writeln!(dump, "{t:#?}");
    }
    w("tasks.debug", dump.as_bytes())?;

    let after = st.vector().map_err(|e| e.to_string())?;
    if before != after {
        return Err("the fold changed the log's vector; it must append nothing".into());
    }
    let rows: i64 = after.values().sum();
    println!(
        "spawn-fold: open {open_ms} ms, fold {fold_ms} ms; vector covers {rows} seqs over {} devices; {} cards, {spawns} folded spawn parts",
        after.len(),
        tasks.len()
    );
    Ok(())
}

/// Lets `fold_run` marshal its views through one closure.
mod erased {
    pub trait Ser {
        fn bytes(&self) -> Result<Vec<u8>, String>;
    }
    impl<T: serde::Serialize> Ser for T {
        fn bytes(&self) -> Result<Vec<u8>, String> {
            super::marshal(self).map_err(|e| e.to_string())
        }
    }
}

pub struct SimOpts {
    from: PathBuf,
    to: PathBuf,
    person: String,
    root: String,
    max_rows: i64,
    now: String,
}

impl SimOpts {
    pub fn parse(args: &[String]) -> Result<SimOpts, String> {
        let (mut from, mut to, mut person, mut root, mut now) = (None, None, None, None, None);
        let mut max_rows = i64::MAX;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--from" => from = Some(PathBuf::from(take(&mut it, a)?)),
                "--to" => to = Some(PathBuf::from(take(&mut it, a)?)),
                "--person" => person = Some(take(&mut it, a)?),
                "--root" => root = Some(take(&mut it, a)?),
                "--now" => now = Some(take(&mut it, a)?),
                "--max-rows" => {
                    let v = take(&mut it, a)?;
                    max_rows = v.parse().map_err(|_| format!("--max-rows {v}"))?;
                }
                _ => return Err(format!("reconnect-sim: unknown flag {a}")),
            }
        }
        Ok(SimOpts {
            from: from.ok_or("reconnect-sim needs --from")?,
            to: to.ok_or("reconnect-sim needs --to")?,
            person: person.ok_or("reconnect-sim needs --person")?,
            root: root.ok_or("reconnect-sim needs --root")?,
            now: now.ok_or("reconnect-sim needs --now")?,
            max_rows,
        })
    }
}

pub fn sim(o: &SimOpts) -> ExitCode {
    match sim_run(o) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("irori-dev: reconnect-sim: {e}");
            ExitCode::FAILURE
        }
    }
}

/// One peer catching up from another as the mesh session does it
/// (mesh session.rs `next` and the ack): the sender lists
/// `missing(peer's vector, 500)`, the receiver `insert_batch`es it in one
/// transaction and folds the stored rows through its Coord's subscription
/// (resyncs included, as a 500-row burst overflows the queue), then acks
/// with its new vector. No network, no signatures skipped. It stops when
/// the sender has nothing more, when `--max-rows` rows moved, or when a
/// batch moves the receiver's vector by nothing three times in a row (a
/// Gap: the session would resend that batch forever).
fn sim_run(o: &SimOpts) -> Result<(), String> {
    refuse_live(&o.from)?;
    refuse_live(&o.to)?;
    let now = GoTime::parse_rfc3339(&o.now).ok_or("--now: not RFC 3339")?;
    let open = |home: &Path| -> Result<(Arc<Store>, Arc<Chain>), String> {
        let me = device_id(home)?;
        let chain = Arc::new(Chain::new(&o.person, &o.root, &me));
        let st = Store::open_with(
            home.join("irori.db"),
            &me,
            Arc::new(store::Clock::new()),
            OpenOptions {
                verifier: Some(chain.clone()),
                ..Default::default()
            },
        )
        .map_err(|e| format!("open {}: {e}", home.display()))?;
        Ok((Arc::new(st), chain))
    };
    let (from, _) = open(&o.from)?;
    let (to, chain) = open(&o.to)?;
    let is_owner = {
        let ch = chain.clone();
        Arc::new(move |k: &str| ch.is_owner(k)) as Arc<dyn Fn(&str) -> bool + Send + Sync>
    };
    let t_fold = Instant::now();
    let co = coord::Coord::new(
        to.clone(),
        None,
        coord::Options {
            now: Some(Arc::new(move || now)),
            is_owner: Some(is_owner),
            ..Default::default()
        },
    )
    .map_err(|e| format!("coord: {e}"))?;
    println!(
        "reconnect-sim: receiver's start fold {} ms",
        t_fold.elapsed().as_millis()
    );
    let sub = to.subscribe();
    let v0 = to.vector().map_err(|e| e.to_string())?;
    let top = from.vector().map_err(|e| e.to_string())?;
    let behind: i64 = top
        .iter()
        .map(|(d, s)| (s - v0.get(d).copied().unwrap_or(0)).max(0))
        .sum();
    println!("reconnect-sim: receiver is {behind} seqs behind the sender");

    let t0 = Instant::now();
    let (mut moved, mut batches, mut still, mut resyncs) = (0i64, 0i64, 0, 0);
    let (mut t_missing, mut t_insert, mut t_fold, mut t_ack) = (0u128, 0u128, 0u128, 0u128);
    let mut v = v0.clone();
    let mut last_report = Instant::now();
    loop {
        let t = Instant::now();
        let evs = from.missing(&v, 500).map_err(|e| e.to_string())?;
        t_missing += t.elapsed().as_micros();
        if evs.is_empty() {
            println!("reconnect-sim: caught up");
            break;
        }
        let t = Instant::now();
        let (n, res) = to.insert_batch(&evs);
        t_insert += t.elapsed().as_micros();
        let t = Instant::now();
        while let gocompat::sub::TryRecv::Item(ev) = sub.try_recv() {
            if ev.is_resync() {
                resyncs += 1;
            }
            co.handle(&ev).map_err(|e| format!("coord: {e}"))?;
        }
        t_fold += t.elapsed().as_micros();
        let t = Instant::now();
        let nv = to.vector().map_err(|e| e.to_string())?;
        t_ack += t.elapsed().as_micros();
        batches += 1;
        let step: i64 = nv
            .iter()
            .map(|(d, s)| s - v.get(d).copied().unwrap_or(0))
            .sum();
        moved += step;
        if step == 0 {
            still += 1;
            if still >= 3 {
                let first = &evs[0];
                println!(
                    "reconnect-sim: STALLED: the batch starts at {}:{} but the receiver holds that device to {}; insert said {:?} ({n} applied). The session would resend it forever.",
                    first.device_id,
                    first.seq,
                    nv.get(&first.device_id).copied().unwrap_or(0),
                    res.err().map(|e| e.to_string())
                );
                break;
            }
        } else {
            still = 0;
        }
        v = nv;
        if moved >= o.max_rows {
            println!("reconnect-sim: stopped at --max-rows");
            break;
        }
        if last_report.elapsed().as_secs() >= 30 {
            last_report = Instant::now();
            println!(
                "reconnect-sim: ... {moved} rows in {:.0} s",
                t0.elapsed().as_secs_f64()
            );
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    let rate = moved as f64 / secs.max(1e-9);
    let left: i64 = top
        .iter()
        .map(|(d, s)| (s - v.get(d).copied().unwrap_or(0)).max(0))
        .sum();
    let ms = |us: u128| us / 1000;
    println!(
        "reconnect-sim: moved {moved} rows in {batches} batches, {secs:.1} s ({rate:.0} rows/s); per phase: sender missing() {} ms, receiver insert {} ms, fold {} ms ({resyncs} resyncs), ack vector() {} ms; {left} seqs still to go{}",
        ms(t_missing),
        ms(t_insert),
        ms(t_fold),
        ms(t_ack),
        if left > 0 && rate > 0.0 {
            format!(" (~{:.0} min more at this rate)", left as f64 / rate / 60.0)
        } else {
            String::new()
        }
    );
    Ok(())
}

pub struct CompactOpts {
    home: PathBuf,
    /// Device id to its floor; `None` is `top`.
    floors: BTreeMap<String, Option<i64>>,
    dry_run: bool,
    vacuum: bool,
}

impl CompactOpts {
    pub fn parse(args: &[String]) -> Result<CompactOpts, String> {
        let mut o = CompactOpts {
            home: PathBuf::new(),
            floors: BTreeMap::new(),
            dry_run: false,
            vacuum: true,
        };
        let mut home = None;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--home" => home = Some(PathBuf::from(take(&mut it, a)?)),
                "--floor" => {
                    let v = take(&mut it, a)?;
                    let (dev, seq) = v
                        .split_once('=')
                        .ok_or_else(|| format!("--floor {v}: want <device>=<seq|top>"))?;
                    let seq = match seq {
                        "top" => None,
                        s => Some(
                            s.parse::<i64>()
                                .map_err(|_| format!("--floor {v}: {s} is not a seq"))?,
                        ),
                    };
                    o.floors.insert(dev.to_string(), seq);
                }
                "--dry-run" => o.dry_run = true,
                "--no-vacuum" => o.vacuum = false,
                _ => return Err(format!("compact-spawn: unknown flag {a}")),
            }
        }
        o.home = home.ok_or("compact-spawn needs --home")?;
        if o.floors.is_empty() {
            return Err(
                "compact-spawn needs at least one --floor <device>=<seq|top>: \
                 the seq every peer already holds for that device"
                    .into(),
            );
        }
        Ok(o)
    }
}

pub fn compact(o: &CompactOpts) -> ExitCode {
    match compact_run(o) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("irori-dev: compact-spawn: {e}");
            ExitCode::FAILURE
        }
    }
}

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1e6)
}

/// The superseded rows, as a temp table `drop_rows(device_id, seq)`.
///
/// A row is superseded when its device wrote a higher gen for the same
/// part by the same writer. Same device and same `by` give the same answer
/// from the fold's writer check (spawn_writer_ok), so the newer row is
/// folded whenever the older one would have been, and fold_spawn keeps the
/// higher gen whichever comes first. Every row at a part's newest gen
/// stays (its requested and its failed or canceled), and so does every
/// device's newest row.
const SUPERSEDED: &str = "
CREATE TEMP TABLE spawn_rows AS
  SELECT device_id, seq,
         json_extract(CAST(payload AS TEXT), '$.taskId') AS task,
         coalesce(json_extract(CAST(payload AS TEXT), '$.step'), '') AS step,
         coalesce(json_extract(CAST(payload AS TEXT), '$.subtask'), '') AS subtask,
         coalesce(json_extract(CAST(payload AS TEXT), '$.by'), '') AS by_,
         json_extract(CAST(payload AS TEXT), '$.gen') AS gen
  FROM events WHERE kind = 'task.spawn';
CREATE INDEX temp.spawn_rows_part ON spawn_rows (device_id, task, step, subtask, by_);
CREATE TEMP TABLE newest AS
  SELECT device_id, task, step, subtask, by_, max(gen) AS gen
  FROM spawn_rows WHERE task IS NOT NULL AND typeof(gen) = 'integer'
  GROUP BY device_id, task, step, subtask, by_;
CREATE UNIQUE INDEX temp.newest_part ON newest (device_id, task, step, subtask, by_);
CREATE TEMP TABLE tops AS
  SELECT device_id, max(seq) AS seq FROM events GROUP BY device_id;
CREATE TEMP TABLE drop_rows AS
  SELECT s.device_id, s.seq FROM spawn_rows s
  JOIN newest n USING (device_id, task, step, subtask, by_)
  JOIN tops t ON t.device_id = s.device_id
  JOIN floors f ON f.device_id = s.device_id
  WHERE typeof(s.gen) = 'integer' AND s.gen < n.gen
    AND s.seq < t.seq AND s.seq <= f.seq;
CREATE UNIQUE INDEX temp.drop_rows_key ON drop_rows (device_id, seq);
";

fn compact_run(o: &CompactOpts) -> Result<(), String> {
    refuse_live(&o.home)?;
    let path = o.home.join("irori.db");
    if !path.exists() {
        return Err(format!("{} has no irori.db", o.home.display()));
    }
    let size0 = file_size(&path) + file_size(&o.home.join("irori.db-wal"));
    let db = rusqlite::Connection::open(&path).map_err(|e| format!("open: {e}"))?;
    let e = |e: rusqlite::Error| e.to_string();
    let vector = |db: &rusqlite::Connection| -> Result<BTreeMap<String, i64>, String> {
        let mut q = db
            .prepare("SELECT device_id, max(seq) FROM events GROUP BY device_id")
            .map_err(e)?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(e)?;
        rows.collect::<Result<_, _>>().map_err(e)
    };
    let before = vector(&db)?;
    for dev in o.floors.keys() {
        if !before.contains_key(dev) {
            return Err(format!("--floor {dev}: no rows of that device in this log"));
        }
    }
    let rows0: i64 = db
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .map_err(e)?;

    let t0 = Instant::now();
    db.execute_batch("BEGIN IMMEDIATE; CREATE TEMP TABLE floors (device_id TEXT PRIMARY KEY, seq INTEGER NOT NULL);")
        .map_err(e)?;
    for (dev, floor) in &o.floors {
        let seq = floor.unwrap_or(before[dev]);
        db.execute("INSERT INTO floors VALUES (?1, ?2)", (dev, seq))
            .map_err(e)?;
    }
    db.execute_batch(SUPERSEDED).map_err(e)?;
    let mut per_dev = BTreeMap::new();
    {
        let mut q = db
            .prepare(
                "SELECT device_id, count(*), min(seq), max(seq) FROM drop_rows GROUP BY device_id",
            )
            .map_err(e)?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ),
                ))
            })
            .map_err(e)?;
        for r in rows {
            let (d, v) = r.map_err(e)?;
            per_dev.insert(d, v);
        }
    }
    let kept_spawn: i64 = db
        .query_row(
            "SELECT (SELECT count(*) FROM spawn_rows) - (SELECT count(*) FROM drop_rows)",
            [],
            |r| r.get(0),
        )
        .map_err(e)?;
    for (dev, floor) in &o.floors {
        let (n, lo, hi) = per_dev.get(dev).copied().unwrap_or((0, 0, 0));
        let f = match floor {
            Some(s) => s.to_string(),
            None => format!("top ({})", before[dev]),
        };
        println!(
            "compact-spawn: {dev}: floor {f}: {n} superseded task.spawn rows (seq {lo}..{hi})"
        );
    }
    let total: i64 = per_dev.values().map(|v| v.0).sum();
    if o.dry_run {
        db.execute_batch("ROLLBACK").map_err(e)?;
        println!(
            "compact-spawn: dry run: would delete {total} of {rows0} rows; {kept_spawn} task.spawn rows would stay"
        );
        return Ok(());
    }
    let deleted = db
        .execute(
            "DELETE FROM events WHERE EXISTS (SELECT 1 FROM drop_rows d
               WHERE d.device_id = events.device_id AND d.seq = events.seq)
               AND kind = 'task.spawn'",
            [],
        )
        .map_err(e)?;
    if deleted as i64 != total {
        db.execute_batch("ROLLBACK").map_err(e)?;
        return Err(format!("deleted {deleted}, expected {total}; rolled back"));
    }
    if vector(&db)? != before {
        db.execute_batch("ROLLBACK").map_err(e)?;
        return Err("the vector moved; rolled back".into());
    }
    db.execute_batch("COMMIT").map_err(e)?;
    let delete_ms = t0.elapsed().as_millis();
    let mut vacuum_ms = 0;
    if o.vacuum {
        let t1 = Instant::now();
        db.execute_batch(
            "PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);",
        )
        .map_err(e)?;
        vacuum_ms = t1.elapsed().as_millis();
    }
    drop(db);
    let size1 = file_size(&path) + file_size(&o.home.join("irori.db-wal"));
    println!(
        "compact-spawn: deleted {deleted} of {rows0} rows in {delete_ms} ms; {kept_spawn} task.spawn rows stay; vector unchanged; vacuum {}; {} -> {}",
        if o.vacuum {
            format!("{vacuum_ms} ms")
        } else {
            "skipped".into()
        },
        mb(size0),
        mb(size1)
    );
    Ok(())
}
