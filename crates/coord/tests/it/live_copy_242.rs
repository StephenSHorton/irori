//! Card #242: sweep a scratch copy of a real board log and count the
//! task.left rows it writes. The board's people come from its own rows
//! (agent.card for cards, the board root's and member.device.added certs
//! for devices), as the identity layer folds them.

use crate::common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use common::*;
use coord::*;
use gocompat::time::GoTime;
use store::{OpenOptions, Store, kinds};

fn lefts_by(path: &std::path::Path, dev: &str) -> i64 {
    let db = rusqlite::Connection::open(path).unwrap();
    db.query_row(
        "SELECT count(*) FROM events WHERE kind = ?1 AND device_id = ?2",
        (kinds::TASK_LEFT, dev),
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
#[ignore = "needs IRORI_BOARD_COPY (a scratch copy of boards/<id>.db) and IRORI_BOARD_DEV"]
fn sweeping_a_live_board_copy_writes_no_repeat_task_left() {
    let path = std::path::PathBuf::from(std::env::var_os("IRORI_BOARD_COPY").unwrap());
    let dev = std::env::var("IRORI_BOARD_DEV").unwrap();
    let (cards, devs) = {
        let db = rusqlite::Connection::open(&path).unwrap();
        let mut cards = HashMap::new();
        let mut devs = HashMap::new();
        let mut q = db
            .prepare("SELECT kind, CAST(payload AS TEXT) FROM events ORDER BY hlc, device_id, seq")
            .unwrap();
        let rows = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap();
        for r in rows {
            let (kind, p) = r.unwrap();
            let v: serde_json::Value = serde_json::from_str(&p).unwrap_or_default();
            let s = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
            match kind.as_str() {
                kinds::AGENT_CARD => {
                    cards.entry(s(&v["cardId"])).or_insert(s(&v["personId"]));
                }
                kinds::BOARD_CREATED => {
                    devs.insert(
                        s(&v["creator"]["rootDeviceId"]),
                        s(&v["creator"]["personId"]),
                    );
                }
                kinds::BOARD_MEMBER_ADDED => {
                    devs.insert(s(&v["rootDeviceId"]), s(&v["personId"]));
                }
                "member.device.added" => {
                    devs.insert(s(&v["cert"]["deviceId"]), s(&v["cert"]["personId"]));
                }
                _ => {}
            }
        }
        (cards, devs)
    };
    let me = devs[&dev].clone();
    let clk = Clk(Arc::new(Mutex::new(GoTime::from_system(SystemTime::now()))));
    let c2 = clk.clone();
    let hc = hlc::Clock::with_now(Box::new(move || c2.now().to_system()));
    let st = Arc::new(
        Store::open_with(
            &path,
            &dev,
            Arc::new(hc),
            OpenOptions {
                board: true,
                ..OpenOptions::default()
            },
        )
        .unwrap(),
    );
    let person_key = format!("person:{me}");
    let o = Options {
        now: Some(clk.func()),
        board: true,
        done_check: true,
        person: person_key.clone(),
        is_owner: Some(Arc::new(move |k: &str| k == person_key)),
        key_person: Some(Arc::new(move |k: &str| {
            if let Some(p) = k.strip_prefix("person:") {
                return p.to_string();
            }
            k.strip_prefix("agent:")
                .and_then(|c| cards.get(c).cloned())
                .unwrap_or_default()
        })),
        device_person: Some(Arc::new(move |d: &str| {
            devs.get(d).cloned().unwrap_or_default()
        })),
        ..Options::default()
    };
    // No agent is live here: every holder is absent.
    let ros = TestRoster::new(&[]);
    let c = Coord::new(st.clone(), Some(ros), o).unwrap();
    let mut per_sweep = Vec::new();
    c.sweep().unwrap();
    clk.add(HOLDER_GRACE + Duration::from_secs(1));
    for _ in 0..6 {
        let before = lefts_by(&path, &dev);
        c.sweep().unwrap();
        per_sweep.push(lefts_by(&path, &dev) - before);
        clk.add(Duration::from_secs(5));
    }
    eprintln!("task.left written per sweep: {per_sweep:?}");
    assert!(
        per_sweep[1..].iter().all(|n| *n == 0),
        "a repeat task.left after the first sweep: {per_sweep:?}"
    );
}
