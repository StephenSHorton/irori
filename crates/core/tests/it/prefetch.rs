//! Card #580: a desktop picks up the small files another device offers
//! (no-server.md §4.3 "Leave nothing stranded"). One person on three
//! devices on loopback iroh: A attaches files, B has prefetch on (with a
//! small cap), C has it off. B holds the small file without anyone asking
//! and offers it on; the big one stays where it was; C fetches nothing
//! until asked, and then gets it from B once A is gone.

use crate::common;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use common::mesh::{Edit, person_with};
use common::wait_for;

const WAIT: u64 = 45;
const CAP: u64 = 4096;

#[test]
fn a_desktop_holds_small_offered_files_so_others_get_them_when_the_first_is_away() {
    let mut devs = person_with("p", 3, |i| {
        (i == 1)
            .then(|| Arc::new(|o: &mut irori_core::Options| o.prefetch_files = Some(CAP)) as Edit)
    });
    let files = tempfile::tempdir().unwrap();
    let small = files.path().join("trace.txt");
    std::fs::write(&small, vec![b'x'; 1024]).unwrap();
    let big = files.path().join("dump.bin");
    std::fs::write(&big, vec![b'y'; 8192]).unwrap();
    let (a, b, c) = (&devs[0], &devs[1], &devs[2]);
    let refs = a
        .core
        .attach_files(
            &a.agent,
            &[
                big.to_string_lossy().into_owned(),
                small.to_string_lossy().into_owned(),
            ],
        )
        .unwrap();
    let (rbig, rsmall) = (refs[0].clone(), refs[1].clone());

    wait_for(WAIT, "B picks up the small file unasked", || {
        b.core.files().has(&rsmall.sha256)
    });
    // B offers it on, and C hears that.
    wait_for(WAIT, "C hears B holds it", || {
        mesh::files::holders_in(c.core.store(), &rsmall.sha256)
            .unwrap()
            .contains(&b.core.device_id().to_string())
    });
    // Over B's cap: never fetched. C has prefetch off: fetched nothing.
    std::thread::sleep(Duration::from_secs(3));
    assert!(!b.core.files().has(&rbig.sha256), "B fetched the big file");
    assert!(!c.core.files().has(&rsmall.sha256), "C fetched unasked");
    assert!(!c.core.files().has(&rbig.sha256), "C fetched unasked");

    // A goes away; C asks and gets the small file from B.
    let a = devs.remove(0);
    a.core.close().unwrap();
    let (b, c) = (&devs[0], &devs[1]);
    assert!(!c.core.files().has(&rsmall.sha256), "C fetched unasked");
    let mut got = Err(irori_core::meshview::FileFetchError::TimedOut);
    wait_for(WAIT, "C fetches the small file from B", || {
        got = c.core.fetch_file_blocking(&rsmall);
        got.is_ok()
    });
    assert_eq!(got.unwrap(), b.core.device_id());
    assert!(c.core.files().has(&rsmall.sha256));
    // The big one only A held.
    assert!(c.core.fetch_file_blocking(&rbig).is_err());
    for d in devs {
        d.core.close().unwrap();
    }
}

/// A file on a shared space's card: Bob's desktop picks it up from the
/// board's log unasked and offers it there in turn.
#[test]
fn a_desktop_holds_small_files_offered_on_a_shared_board() {
    let alice = person_with("alice", 1, |_| None);
    let bob = person_with("bob", 1, |_| {
        Some(Arc::new(|o: &mut irori_core::Options| o.prefetch_files = Some(CAP)) as Edit)
    });
    let (a1, b1) = (&alice[0], &bob[0]);
    let board = common::mesh::invited(a1, b1);
    let aco = a1.core.board_coord(&board).unwrap();
    let t = aco
        .create_task(
            &a1.person_actor(),
            coord::NewTask {
                title: "look at the trace".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let files = tempfile::tempdir().unwrap();
    let shared = files.path().join("trace.txt");
    std::fs::write(&shared, b"frame 12 took 40ms").unwrap();
    let out = a1.tool(
        "board_comment",
        serde_json::json!({
            "task_id": "mlg#1",
            "text": "the trace",
            "attach": [shared.to_string_lossy()],
        }),
    );
    assert!(!out.starts_with("Error"), "board_comment: {out}");
    let sha = aco.comments(&t.id).unwrap()[0].attachments[0]
        .sha256
        .clone();
    a1.core.kick_board(&board);
    wait_for(WAIT, "Bob picks up the board's file unasked", || {
        b1.core.files().has(&sha)
    });
    let held = mesh::files::holders_in(&b1.core.board_store(&board).unwrap(), &sha).unwrap();
    assert!(held.contains(&b1.core.device_id().to_string()));
    for d in alice.into_iter().chain(bob) {
        d.core.close().unwrap();
    }
}

/// Offers older than 30 days are left alone (card #580). B's clock runs a
/// month ahead when A offers the first file, so B skips it; then A's
/// clock catches up, and its next offer B picks up.
#[test]
fn a_desktop_leaves_offers_older_than_a_month() {
    const MONTH: i64 = 31 * 24 * 3600;
    let skews: Vec<Arc<AtomicI64>> = (0..2).map(|_| Arc::new(AtomicI64::new(0))).collect();
    let devs = person_with("p", 2, |i| {
        let sk = skews[i].clone();
        Some(Arc::new(move |o: &mut irori_core::Options| {
            let sk = sk.clone();
            o.now = Some(Arc::new(move || {
                let d = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap();
                gocompat::time::GoTime::unix(
                    d.as_secs() as i64 + sk.load(Ordering::SeqCst),
                    i64::from(d.subsec_nanos()),
                )
            }));
            if i == 1 {
                o.prefetch_files = Some(CAP);
            }
        }) as Edit)
    });
    let (a, b) = (&devs[0], &devs[1]);
    let files = tempfile::tempdir().unwrap();
    let attach = |name: &str, body: &[u8]| {
        let p = files.path().join(name);
        std::fs::write(&p, body).unwrap();
        a.core
            .attach_files(&a.agent, &[p.to_string_lossy().into_owned()])
            .unwrap()
            .remove(0)
    };

    skews[1].store(MONTH, Ordering::SeqCst);
    let old = attach("old.txt", b"from last month");
    wait_for(WAIT, "B hears A's old offer", || {
        mesh::files::holders_in(b.core.store(), &old.sha256)
            .unwrap()
            .contains(&a.core.device_id().to_string())
    });
    std::thread::sleep(Duration::from_secs(3));
    assert!(!b.core.files().has(&old.sha256), "B fetched an old offer");

    skews[0].store(MONTH, Ordering::SeqCst);
    let fresh = attach("fresh.txt", b"from today");
    wait_for(WAIT, "B picks up the fresh file", || {
        b.core.files().has(&fresh.sha256)
    });
    assert!(!b.core.files().has(&old.sha256), "B fetched an old offer");
    for d in devs {
        d.core.close().unwrap();
    }
}
