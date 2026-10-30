//! Ported from I:internal/identity/boardsecret_test.go and
//! I:internal/owner/owner_test.go (its DeviceOf/MayWrite half; that every
//! table kind replicates is the store registry's check).

use irori_identity::*;
use zeroize::Zeroizing;

const BOARD: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn board_topic_derivation() {
    let sec = [7u8; BOARD_SECRET_SIZE];
    let a = derive_board_topic(BOARD, 1, &sec).unwrap();
    assert_eq!(a, derive_board_topic(BOARD, 1, &sec).unwrap());
    assert!(a.topic.starts_with("irori/board/1/") && a.topic.len() == "irori/board/1/".len() + 32);
    for (name, b) in [
        ("epoch", derive_board_topic(BOARD, 2, &sec)),
        (
            "board",
            derive_board_topic("fedcba9876543210fedcba9876543210", 1, &sec),
        ),
        ("secret", derive_board_topic(BOARD, 1, &[8u8; 32])),
    ] {
        let b = b.unwrap();
        assert!(
            b.topic != a.topic && b.key != a.key,
            "another {name} gives the same topic or key"
        );
    }
    assert!(derive_board_topic("nope", 1, &sec).is_err());
    assert!(derive_board_topic(BOARD, 1, &sec[..5]).is_err());
    assert!(
        !format!("{a:?}").contains(&hex::encode(a.key)),
        "Debug shows the key"
    );
}

#[test]
fn board_secrets_rotate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{BOARD}.secrets"));
    let s = BoardSecrets::open(&path, BOARD, Box::new(FileKeystore)).unwrap();
    assert_eq!(s.epoch(), 0);
    s.mint(1, "pc").unwrap();
    let one = s.secret(1).unwrap().0;
    s.mint(1, "pc").unwrap();
    assert_eq!(
        *s.secret(1).unwrap().0,
        *one,
        "minting an epoch twice replaced its secret"
    );
    let ts = s.topics().unwrap();
    assert_eq!(ts.iter().map(|t| t.epoch).collect::<Vec<_>>(), vec![1]);
    s.mint(2, "pc").unwrap();
    let ts = s.topics().unwrap();
    assert_eq!(ts.iter().map(|t| t.epoch).collect::<Vec<_>>(), vec![2, 1]);
    assert_ne!(ts[0].topic, ts[1].topic);
    let mac = [9u8; 32];
    s.add(2, "aaa-mac", &mac).unwrap();
    let (got, maker) = s.secret(2).unwrap();
    assert_eq!((got.as_slice(), maker.as_str()), (&mac[..], "aaa-mac"));
    s.settle(2).unwrap();
    assert_eq!(
        s.topics()
            .unwrap()
            .iter()
            .map(|t| t.epoch)
            .collect::<Vec<_>>(),
        vec![2]
    );
    let r = BoardSecrets::open(&path, BOARD, Box::new(FileKeystore)).unwrap();
    let (got, maker) = r.secret(2).unwrap();
    assert_eq!(
        (got.as_slice(), maker.as_str(), r.epoch()),
        (&mac[..], "aaa-mac", 2)
    );
    assert!(
        r.secret(1).is_none(),
        "a settled epoch's predecessor is still held"
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"keystore\": \"file\"")
    );
    let other = AesKeystore::new("other", Box::new(|| Ok(Zeroizing::new(vec![0u8; 32]))));
    assert!(
        BoardSecrets::open(&path, BOARD, Box::new(other)).is_err(),
        "another keystore opened the secrets"
    );
    // Bad adds.
    assert!(s.add(3, "pc", &[1u8; 5]).is_err());
    assert!(s.add(0, "pc", &[1u8; 32]).is_err());
    assert!(s.add(3, "", &[1u8; 32]).is_err());
}

#[test]
fn board_secret_makers_that_count() {
    let dir = tempfile::tempdir().unwrap();
    let s = BoardSecrets::open(
        &dir.path().join(format!("{BOARD}.secrets")),
        BOARD,
        Box::new(FileKeystore),
    )
    .unwrap();
    s.add(2, "aaa-stolen", &[1u8; 32]).unwrap();
    s.add(2, "pc", &[2u8; 32]).unwrap();
    assert_eq!(
        s.secret(2).unwrap().1,
        "aaa-stolen",
        "with no rule the lower maker wins"
    );
    s.set_makers(Some(Box::new(|d: &str| d != "aaa-stolen")));
    assert_eq!(
        s.secret(2).unwrap().1,
        "pc",
        "a cut device's secret won the tie"
    );
    s.set_makers(Some(Box::new(|_: &str| false)));
    assert!(s.topics().unwrap().is_empty());
}

#[test]
fn device_of_and_may_write() {
    for (key, dev) in [
        ("pc/aaaa-session", "pc"),
        ("mac/grok-1", "mac"),
        ("user:pc", "pc"),
        ("irori", ""),
        ("", ""),
        ("remote:github.com/a/b", ""),
    ] {
        assert_eq!(owner::device_of(key), dev, "{key:?}");
    }
    assert!(owner::may_write("pc/s", "pc") && !owner::may_write("pc/s", "mac"));
    assert!(owner::may_write("user:mac", "mac") && !owner::may_write("user:mac", "pc"));
    assert!(owner::may_write("irori", "anyone"));
    for r in owner::TABLE {
        for k in r.owner_kinds {
            assert_eq!(owner::entity_of(k), Some(r.entity));
        }
    }
    assert_eq!(owner::entity_of("message.sent"), None);
}

/// Go `TestTableKindsAreClassifiedAndReplicate`
/// (I:internal/owner/owner_test.go): every owner and request kind is a
/// replicated kind, and each owner kind maps back to its entity.
#[test]
fn table_kinds_are_classified_and_replicate() {
    for r in owner::TABLE {
        for k in r.owner_kinds.iter().chain(r.request_kinds) {
            assert_eq!(
                store::kind_side(k),
                store::Side::Replicated,
                "{k} ({}) is not a replicated kind",
                r.entity
            );
        }
        for k in r.owner_kinds {
            assert_eq!(owner::entity_of(k), Some(r.entity), "EntityOf({k})");
        }
    }
    assert_eq!(
        owner::entity_of("message.sent"),
        None,
        "message.sent is anyone's to write"
    );
}
