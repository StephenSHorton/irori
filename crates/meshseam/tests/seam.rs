//! MeshSecret against Go (testdata/golden/meshseam, tools/goldens/meshseam_golden_test.go)
//! and the stream framing rules (mesh.md §2.2-2.3).

use irori_meshseam::*;

#[derive(serde::Deserialize)]
struct Gold {
    app: String,
    secrets: Vec<Row>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    seed: String,
    person_id: String,
    epoch: i64,
    info: String,
    key: String,
    topic: String,
}

#[test]
fn mesh_secret_matches_go() {
    let path = manifest_dir().join("../../testdata/golden/meshseam/secret.json");
    let g: Gold = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(g.app, MESH_APP);
    assert!(g.secrets.len() > 50);
    for r in &g.secrets {
        let seed = hex::decode(&r.seed).unwrap();
        assert_eq!(hex::encode(mesh_info(&r.person_id, r.epoch)), r.info);
        let s = mesh_secret(&seed, &r.person_id, r.epoch);
        assert_eq!(hex::encode(s.key), r.key, "{} {}", r.person_id, r.epoch);
        assert_eq!(s.topic, r.topic);
        assert_eq!(s.epoch, r.epoch);
    }
}

#[test]
fn mesh_secret_moves_with_the_epoch() {
    let seed = [7u8; 32];
    let a = mesh_secret(&seed, "p", 1);
    let b = mesh_secret(&seed, "p", 2);
    assert_ne!(a.key, b.key);
    assert_ne!(a.topic, b.topic);
    assert!(!format!("{a:?}").contains(&hex::encode(a.key)));
}

#[tokio::test(flavor = "current_thread")]
async fn frames_round_trip_and_keep_their_limits() {
    let mut buf = Vec::new();
    write_frame(&mut buf, b"hello", 8).await.unwrap();
    write_frame(&mut buf, b"", 8).await.unwrap();
    assert!(matches!(
        write_frame(&mut buf, b"123456789", 8).await,
        Err(FrameError::TooLong(9, 8))
    ));
    let mut r = &buf[..];
    assert_eq!(
        read_frame(&mut r, 8, true).await.unwrap().unwrap(),
        b"hello"
    );
    assert_eq!(read_frame(&mut r, 8, true).await.unwrap().unwrap(), b"");
    assert!(read_frame(&mut r, 8, true).await.unwrap().is_none());

    let mut r = &[0u8, 0, 0, 0][..];
    assert!(matches!(
        read_frame(&mut r, 8, false).await,
        Err(FrameError::Empty)
    ));
    let mut r = &[0u8, 0, 0, 9, 1][..];
    assert!(matches!(
        read_frame(&mut r, 8, true).await,
        Err(FrameError::TooLong(9, 8))
    ));
    let mut r = &[0u8, 0, 0, 3, 1][..];
    assert!(matches!(
        read_frame(&mut r, 8, true).await,
        Err(FrameError::Truncated)
    ));
    let mut r = &[0u8, 0][..];
    assert!(matches!(
        read_frame(&mut r, 8, true).await,
        Err(FrameError::Truncated)
    ));
}

#[test]
fn offerer_is_the_lower_id_unless_set() {
    let mut c = RoomConfig::new("b");
    assert!(c.offers_to("c"));
    assert!(!c.offers_to("a"));
    c.offerer = Some(std::sync::Arc::new(|_| true));
    assert!(c.offers_to("a"));
}

#[test]
fn close_errors_say_whether_to_retry() {
    assert!(ConnError::Transport("timed out".into()).retry());
    assert!(!ConnError::Local(reason::LEAVING.into()).retry());
    assert!(!ConnError::Remote(reason::REPLACED.into()).retry());
    assert!(ConnError::Remote(reason::REPLACED.into()).replaced());
    assert!(ConnError::Remote(reason::REFUSED.into()).refused());
    assert_eq!(colon_hex(&[0xab, 0x01]), "AB:01");
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
