//! Card #1262: why a join sits on "Looking for the other device…" (the
//! hint after a while) and the pairing lines both sides write to the log.

use crate::common;

use std::time::Duration;

use common::{Dev, eventually};
use irori_pair::*;
use mesh::FakeNet;

const Z: Duration = Duration::ZERO;

/// No line names the code (in any spelling) or its room secret.
fn assert_no_secret(d: &Dev, code: &str) {
    let canon = normalize_code(code).unwrap();
    let s = room_secret(&canon).unwrap();
    let key = hex::encode(s.key);
    let bare = canon.replace('-', "");
    for l in d.log() {
        for bad in [canon.as_str(), bare.as_str(), &key, &s.topic] {
            assert!(
                !l.to_uppercase().contains(&bad.to_uppercase()),
                "{}: the log names the code or its secret: {l:?}",
                d.name
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_join_that_finds_nothing_says_why() {
    let net = FakeNet::new();
    let b = Dev::new(&net, "mac", Z);
    b.pair.set_hint_after(Duration::from_millis(400));
    // A code nobody added: nothing answers.
    let code = new_code();
    let st = b.pair.join(&code).await.unwrap();
    assert_eq!(st.state, STATE_WAITING);
    assert!(st.hint.is_empty(), "a hint at once: {st:?}");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        b.pair.status().hint.is_empty(),
        "the hint came before its time"
    );
    eventually("b says why", || !b.pair.status().hint.is_empty()).await;
    let st = b.pair.status();
    assert_eq!(st.state, STATE_WAITING, "the hint closed the window");
    assert_eq!(st.hint, search_hint("", cfg!(target_os = "macos")));
    assert!(b.logged("pair: join window opened"), "{:?}", b.log());
    assert!(
        b.logged("pair: join: nothing reached after"),
        "{:?}",
        b.log()
    );

    b.pair.cancel();
    let st = b.pair.status();
    assert_eq!(st.state, STATE_CANCELED);
    assert!(st.hint.is_empty(), "the hint outlived the window: {st:?}");
    assert!(
        b.logged("pair: join window closed: canceled"),
        "{:?}",
        b.log()
    );
    assert_no_secret(&b, &code);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_hint_names_the_address_typed() {
    let net = FakeNet::new();
    let b = Dev::new(&net, "mac", Z);
    b.pair.set_hint_after(Duration::from_millis(200));
    let code = new_code();
    b.pair.join_via(&code, "10.0.0.9:25817").await.unwrap();
    assert!(b.logged("via 10.0.0.9:25817"), "{:?}", b.log());
    eventually("b says why", || !b.pair.status().hint.is_empty()).await;
    let hint = b.pair.status().hint;
    assert_eq!(
        hint,
        search_hint("10.0.0.9:25817", cfg!(target_os = "macos"))
    );
    assert!(hint.contains("10.0.0.9:25817"), "{hint}");
    b.pair.cancel();
    assert_no_secret(&b, &code);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_window_expiring_is_logged() {
    let net = FakeNet::new();
    let b = Dev::new(&net, "mac", Duration::from_millis(300));
    b.pair.join(&new_code()).await.unwrap();
    eventually("b expires", || b.state() == STATE_EXPIRED).await;
    assert!(
        b.logged("pair: join window closed: expired"),
        "{:?}",
        b.log()
    );
}

#[test]
fn search_hint_texts() {
    let home = search_hint("", false);
    assert!(
        home.starts_with("Nothing found on this network yet."),
        "{home}"
    );
    assert!(home.contains("enter the address shown on the other device"));
    assert!(!home.contains("Local Network"), "{home}");
    let mac = search_hint("", true);
    assert!(mac.starts_with(&home), "{mac}");
    assert!(
        mac.ends_with("Privacy & Security → Local Network → Irori."),
        "{mac}"
    );
    let via = search_hint("pc.example:25817", false);
    assert!(via.contains("pc.example:25817"), "{via}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pairing_logs_both_sides_and_never_hints() {
    let net = FakeNet::new();
    let a = Dev::new(&net, "pc", Z);
    let b = Dev::new(&net, "mac", Z);
    let hint_after = Duration::from_millis(500);
    b.pair.set_hint_after(hint_after);
    let st = a.pair.add(irori_identity::ROLE_DESKTOP).await.unwrap();
    let code = st.code.clone();
    b.pair.join(&code).await.unwrap();
    eventually("b shows words", || b.state() == STATE_WORDS).await;
    eventually("a asks for the words", || a.state() == STATE_CONFIRM).await;
    // Past the hint's time: a device that found the other one has no hint.
    tokio::time::sleep(hint_after + Duration::from_millis(200)).await;
    assert!(
        b.pair.status().hint.is_empty(),
        "a hint once the devices met: {:?}",
        b.pair.status()
    );
    assert!(!b.logged("nothing reached"), "{:?}", b.log());

    let (aid, bid) = (a.id(), b.id());
    for (d, lines) in [
        (
            &a,
            vec![
                "pair: add window opened (role desktop".to_string(),
                format!("pair: add: hello from {} (mac", &bid[..8]),
                format!("pair: add: connected to {}", &bid[..8]),
                "pair: add: both proved the code; words shown".to_string(),
            ],
        ),
        (
            &b,
            vec![
                "pair: join window opened".to_string(),
                format!("pair: join: connected to {}", &aid[..8]),
                "pair: join: both proved the code; words shown".to_string(),
            ],
        ),
    ] {
        for l in lines {
            assert!(d.logged(&l), "{}: no {l:?} in {:?}", d.name, d.log());
        }
    }

    // A wrong typing is logged as a count, never the words.
    let words = b.pair.status().words;
    let _ = a
        .pair
        .confirm("aardvark adroitness aardvark adroitness aardvark")
        .await;
    assert!(
        a.logged("pair: add: the words typed don't match (1 of 2)"),
        "{:?}",
        a.log()
    );
    a.pair.cancel();
    eventually("b hears the cancel", || b.state() == STATE_FAILED).await;
    assert!(
        a.logged("pair: add window closed: canceled"),
        "{:?}",
        a.log()
    );
    eventually("b logs the close", || {
        b.logged("pair: join window closed: failed: The other device canceled.")
    })
    .await;
    for d in [&a, &b] {
        assert_no_secret(d, &code);
        for l in d.log() {
            for w in &words {
                assert!(
                    !l.split(|c: char| !c.is_alphanumeric()).any(|t| t == w),
                    "{}: the log names a word: {l:?}",
                    d.name
                );
            }
        }
    }
}
