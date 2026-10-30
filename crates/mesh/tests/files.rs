//! I:internal/mesh/files_test.go on FakeNet.

mod common;

use std::io::{Read, Write};
use std::time::Duration;

use blob::{ErrorKind, FileRef};
use common::{Dev, converged, eventually, trio};
use mesh::{FakeNet, FetchError};
use serde_json::json;

/// Writes `size` random bytes to a file, stores it in the device's
/// attachment store and offers it, as attaching does.
fn attach_file(d: &Dev, name: &str, size: usize) -> (FileRef, Vec<u8>, tempfile::TempDir) {
    let mut data = vec![0u8; size];
    getrandom::fill(&mut data).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join(name);
    std::fs::write(&p, &data).unwrap();
    let mut r = d.files.put_file(p.to_str().unwrap()).unwrap();
    r.device = d.id();
    d.node().offer(&r).unwrap();
    (r, data, tmp)
}

fn stored(d: &Dev, r: &FileRef) -> Vec<u8> {
    let mut f = d
        .files
        .open_file(&r.sha256)
        .unwrap_or_else(|e| panic!("{} doesn't hold {}: {e}", d.name, r.name));
    let mut b = Vec::new();
    f.read_to_end(&mut b).unwrap();
    b
}

/// "An attachment fetched from the origin, then from a second holder with
/// the origin gone, while live rows keep flowing on the sync stream."
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attachment_is_fetched_from_the_origin_then_from_a_second_holder() {
    let net = FakeNet::new();
    let (mut pc, mut mac, nd) = trio(&net).await;
    let (r, data, _t) = attach_file(&pc, "design.pdf", (3 << 20) + 123);
    eventually("the offer replicates", || converged(&[&pc, &mac, &nd])).await;

    let from = mac.node().fetch_file(&r).await.expect("mac fetch");
    assert_eq!(from, pc.id());
    assert!(stored(&mac, &r) == data, "mac's copy differs");
    // A second fetch is served from the store.
    assert_eq!(mac.node().fetch_file(&r).await.unwrap(), "");
    // The mac now offers it too.
    eventually("the mac's offer reaches the node", || {
        nd.node().holders(&r.sha256).unwrap().len() == 2
    })
    .await;

    // The origin goes away; the node fetches from the mac while the mac
    // keeps writing rows.
    pc.stop().await;
    net.isolate(&pc.id());
    let (n2, r2) = (nd.node().clone(), r.clone());
    let fetch = tokio::spawn(async move { n2.fetch_file(&r2).await });
    let live: Vec<_> = (0..40)
        .map(|i| mac.append(store::kinds::CAPTURE_TURN, json!({"i": i})))
        .collect();
    let got = tokio::time::timeout(Duration::from_secs(20), fetch)
        .await
        .expect("node fetch hung")
        .unwrap()
        .expect("node fetch");
    assert_eq!(got, mac.id(), "fetched from {got}");
    eventually("the mac's live rows reach the node", || {
        nd.has(live.last().unwrap())
    })
    .await;
    assert!(stored(&nd, &r) == data, "node's copy differs");

    // With every holder away the fetch says where the file is.
    let (other, _, _t2) = attach_file(&mac, "notes.txt", 1000);
    eventually("the second offer reaches the node", || {
        nd.node().holders(&other.sha256).unwrap().len() == 1
    })
    .await;
    mac.stop().await;
    match nd.node().fetch_file(&other).await {
        Err(FetchError::Offline(h)) if h == vec![mac.id()] => {}
        r => panic!("all holders away = {r:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fetch_resumes_and_only_offered_files_are_served() {
    let net = FakeNet::new();
    let (pc, mac, _nd) = trio(&net).await;
    let (r, data, _t) = attach_file(&pc, "log.txt", 200 << 10);
    eventually("the offer replicates", || {
        mac.node().holders(&r.sha256).unwrap().len() == 1
    })
    .await;

    // Half of it is already here from a cut stream: the fetch asks for the
    // rest only.
    let (mut part, _) = mac.files.partial(&r.sha256).unwrap();
    part.write_all(&data[..100 << 10]).unwrap();
    drop(part);
    mac.node().fetch_file(&r).await.unwrap();
    assert!(stored(&mac, &r) == data, "resumed copy differs");

    // A file the pc holds but no row references is never served.
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("private.txt");
    std::fs::write(&p, b"not offered").unwrap();
    let mut hidden = pc.files.put_file(p.to_str().unwrap()).unwrap();
    hidden.device = pc.id();
    let res = mac.node().fetch_file(&hidden).await;
    assert!(
        res.is_err() && !mac.files.has(&hidden.sha256),
        "an unreferenced file was fetched: {res:?}"
    );

    // A ref that claims more than the cap is refused before anything is
    // asked for; one whose length is wrong is never kept.
    let mut big = r.clone();
    big.size = blob::MAX_FILE_SIZE as i64 + 1;
    let res = mac.node().fetch_file(&big).await;
    assert!(
        res.as_ref().is_err_and(|e| e.is_blob(ErrorKind::TooLarge)),
        "over the cap = {res:?}"
    );
    let (other, _, _t2) = attach_file(&pc, "other.bin", 5000);
    eventually("the offer replicates", || {
        mac.node().holders(&other.sha256).unwrap().len() == 1
    })
    .await;
    let mut short = other.clone();
    short.size = 4000;
    let res = mac.node().fetch_file(&short).await;
    assert!(
        res.is_err() && !mac.files.has(&other.sha256),
        "a wrong length was kept: {res:?}"
    );
}

/// A fetch waiting on another fetch of the same file gives up at its own
/// deadline, and the turn comes back once the first is done.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fetch_waiting_its_turn_honours_its_deadline() {
    let net = FakeNet::new();
    let (pc, mac, nd) = trio(&net).await;
    let (r, data, _t) = attach_file(&pc, "shot.png", 4096);
    eventually("the offer replicates", || converged(&[&pc, &mac, &nd])).await;

    let turn = mac.node().lock_fetch(&r.sha256).await;
    let start = std::time::Instant::now();
    let res = tokio::time::timeout(Duration::from_millis(50), mac.node().fetch_file(&r)).await;
    assert!(res.is_err(), "fetch behind a held turn = {res:?}");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "waited {:?} past a 50ms deadline",
        start.elapsed()
    );
    drop(turn);
    mac.node()
        .fetch_file(&r)
        .await
        .expect("fetch after the turn came back");
    assert!(stored(&mac, &r) == data);
}
