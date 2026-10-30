//! Go update/service_test.go, against a fake GitHub on 127.0.0.1.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use sha2::{Digest, Sha256};
use update::nsis::wait_pid_arg;
use update::*;

const TEST_INSTALLER: &str = r"C:\Temp\irori-update\irori-1.2.3-windows-amd64-installer.exe";
const TEST_ASSET: &str = "irori-1.2.3-windows-amd64-installer.exe";

#[derive(Default)]
struct Launches {
    calls: Mutex<Vec<Vec<String>>>,
    fail: bool,
}

fn recorder(l: &Arc<Launches>) -> Launcher {
    let l = l.clone();
    Arc::new(move |path: &Path, args: &[String]| {
        let mut c = vec![path.to_string_lossy().into_owned()];
        c.extend(args.iter().cloned());
        l.calls.lock().unwrap().push(c);
        if l.fail {
            Err(std::io::Error::other("boom"))
        } else {
            Ok(())
        }
    })
}

fn ready_status() -> Status {
    let mut st = idle();
    st.phase = PHASE_READY.into();
    st.version = "1.2.3".into();
    st.installer_path = PathBuf::from(TEST_INSTALLER);
    st
}

fn idle() -> Status {
    serde_json_status("idle")
}

fn serde_json_status(phase: &str) -> Status {
    Status {
        phase: phase.into(),
        version: String::new(),
        notes: String::new(),
        installer_path: PathBuf::new(),
        error: String::new(),
    }
}

/// Go `readyService`.
fn ready_service(fail: bool) -> (Updater, Arc<Launches>) {
    let l = Arc::new(Launches {
        fail,
        ..Default::default()
    });
    let s = Updater::new(
        "x/y",
        "1.0.0",
        std::env::temp_dir().join("irori-update-unused"),
    )
    .with_launcher(recorder(&l))
    .with_platform(Platform::Nsis);
    s.force_status(ready_status());
    (s, l)
}

fn calls(l: &Launches) -> Vec<Vec<String>> {
    l.calls.lock().unwrap().clone()
}

fn want(args: &[&str]) -> Vec<Vec<String>> {
    let mut c = vec![TEST_INSTALLER.to_string()];
    c.extend(args.iter().map(|s| s.to_string()));
    vec![c]
}

#[test]
fn restart_launches_silent() {
    let (s, l) = ready_service(false);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    s.set_on_change(Arc::new(move |st: &Status| {
        seen2.lock().unwrap().push(st.phase.clone())
    }));
    s.restart_to_update().unwrap();
    assert_eq!(calls(&l), want(&["/S", &wait_pid_arg()]));
    assert_eq!(s.status().phase, PHASE_INSTALLING);
    assert_eq!(*seen.lock().unwrap(), vec![PHASE_INSTALLING.to_string()]);
}

#[test]
fn restart_emits_and_quits() {
    let (s, _) = ready_service(false);
    let events = Arc::new(Mutex::new(Vec::new()));
    let quits = Arc::new(AtomicUsize::new(0));
    let (e2, q2) = (events.clone(), quits.clone());
    s.set_hooks(
        Arc::new(move |name: &str, st: &Status| {
            e2.lock().unwrap().push(format!("{name}:{}", st.phase))
        }),
        Arc::new(move || {
            q2.fetch_add(1, Ordering::SeqCst);
        }),
    );
    s.restart_to_update().unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        vec![format!("{EVENT_UPDATE_STATUS}:{PHASE_INSTALLING}")]
    );
    assert_eq!(quits.load(Ordering::SeqCst), 1);
}

#[test]
fn restart_requires_ready() {
    let (s, l) = ready_service(false);
    let mut st = ready_status();
    st.phase = PHASE_DOWNLOADING.into();
    s.force_status(st);
    let err = s.restart_to_update().unwrap_err();
    assert_eq!(err, r#"no update ready to install (phase "downloading")"#);
    assert!(calls(&l).is_empty());
}

#[test]
fn restart_launch_failure_reverts_to_ready() {
    let (s, _) = ready_service(true);
    let err = s.restart_to_update().unwrap_err();
    assert_eq!(err, "launch installer: boom");
    assert_eq!(s.status().phase, PHASE_READY);
}

#[test]
fn install_on_exit_no_relaunch() {
    let (s, l) = ready_service(false);
    s.install_on_exit();
    assert_eq!(calls(&l), want(&["/S", "/NORELAUNCH", &wait_pid_arg()]));
    assert_eq!(s.status().phase, PHASE_INSTALLING);
}

#[test]
fn install_on_exit_only_when_ready() {
    for phase in [
        PHASE_IDLE,
        PHASE_CHECKING,
        PHASE_DOWNLOADING,
        PHASE_INSTALLING,
        PHASE_ERROR,
    ] {
        let (s, l) = ready_service(false);
        let mut st = ready_status();
        st.phase = phase.into();
        s.force_status(st);
        s.install_on_exit();
        assert!(calls(&l).is_empty(), "phase {phase}");
    }
}

#[test]
fn neither_launches_twice() {
    let (s, l) = ready_service(false);
    s.restart_to_update().unwrap();
    s.install_on_exit();
    assert!(s.restart_to_update().is_err());
    assert_eq!(calls(&l), want(&["/S", &wait_pid_arg()]));

    let (s, l) = ready_service(false);
    s.install_on_exit();
    s.install_on_exit();
    assert_eq!(calls(&l).len(), 1);
}

#[tokio::test]
async fn check_and_download_returns_ready_without_rerun() {
    let (s, l) = ready_service(false);
    let st = s.check_and_download().await.unwrap();
    assert_eq!(st.phase, PHASE_READY);
    assert_eq!(st.version, "1.2.3");
    assert!(calls(&l).is_empty());
}

#[test]
fn status_json_is_gos() {
    let st = ready_status();
    assert_eq!(
        serde_json::to_string(&st).unwrap(),
        r#"{"phase":"ready","version":"1.2.3","notes":"","error":""}"#
    );
}

/// Go `fakeGitHub`: a latest release with one asset and SHA256SUMS. `sums`
/// overrides the SHA256SUMS body (`None` = the right hash, `Some("-")` =
/// no SHA256SUMS asset).
struct FakeGitHub {
    tag: String,
    sums: Option<String>,
    payload: Vec<u8>,
    asset: String,
    hits: AtomicUsize,
    base: Mutex<String>,
}

impl FakeGitHub {
    fn new(tag: &str, payload: &[u8]) -> FakeGitHub {
        FakeGitHub {
            tag: tag.into(),
            sums: None,
            payload: payload.to_vec(),
            asset: TEST_ASSET.into(),
            hits: AtomicUsize::new(0),
            base: Mutex::new(String::new()),
        }
    }

    async fn serve(self) -> (Arc<FakeGitHub>, String) {
        let g = Arc::new(self);
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        *g.base.lock().unwrap() = base.clone();
        let app = Router::new()
            .route("/repos/x/y/releases/latest", get(latest))
            .route("/dl/{name}", get(dl))
            .with_state(g.clone());
        tokio::spawn(async move {
            axum::serve(l, app).await.unwrap();
        });
        (g, base)
    }
}

async fn latest(State(g): State<Arc<FakeGitHub>>) -> impl IntoResponse {
    g.hits.fetch_add(1, Ordering::SeqCst);
    let base = g.base.lock().unwrap().clone();
    let mut assets = vec![
        serde_json::json!({"name": g.asset, "browser_download_url": format!("{base}/dl/{}", g.asset)}),
    ];
    if g.sums.as_deref() != Some("-") {
        assets.push(serde_json::json!({"name": "SHA256SUMS", "browser_download_url": format!("{base}/dl/SHA256SUMS")}));
    }
    serde_json::json!({"tag_name": g.tag, "body": "notes", "assets": assets}).to_string()
}

async fn dl(
    State(g): State<Arc<FakeGitHub>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::response::Response {
    if name == "SHA256SUMS" {
        let body = g.sums.clone().unwrap_or_else(|| {
            format!(
                "{}  irori-1.2.3-windows-amd64.zip\n{}  {}\n",
                "0".repeat(64),
                hex::encode(Sha256::digest(&g.payload)),
                g.asset
            )
        });
        return body.into_response();
    }
    if name == g.asset {
        return g.payload.clone().into_response();
    }
    axum::http::StatusCode::NOT_FOUND.into_response()
}

fn test_service(base: &str, current: &str, dir: &Path) -> Updater {
    Updater::new("x/y", current, dir)
        .with_api_base(base)
        .with_platform(Platform::Nsis)
}

#[tokio::test]
async fn find_update_picks_installer_and_checksum() {
    let (g, base) = FakeGitHub::new("v1.2.3", b"installer bytes").serve().await;
    let dir = tempfile::tempdir().unwrap();
    let s = test_service(&base, "1.0.0", dir.path());
    let info = s.check_for_update().await.unwrap().unwrap();
    assert_eq!(info.version, "1.2.3");
    assert_eq!(info.asset_name, TEST_ASSET);
    assert_eq!(info.notes, "notes");
    assert_eq!(info.sha256, hex::encode(Sha256::digest(&g.payload)));
}

#[tokio::test]
async fn find_update_current_or_dev() {
    let (g, base) = FakeGitHub::new("v1.2.3", b"x").serve().await;
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        test_service(&base, "1.2.3", dir.path())
            .check_for_update()
            .await
            .unwrap(),
        None
    );
    let before = g.hits.load(Ordering::SeqCst);
    for dev in ["dev", ""] {
        assert_eq!(
            test_service(&base, dev, dir.path())
                .check_for_update()
                .await
                .unwrap(),
            None
        );
    }
    assert_eq!(
        g.hits.load(Ordering::SeqCst),
        before,
        "a dev build must not query GitHub"
    );
}

#[tokio::test]
async fn find_update_no_releases() {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(l, Router::new()).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        test_service(&base, "1.0.0", dir.path())
            .check_for_update()
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn find_update_requires_checksum() {
    for (sums, err) in [
        ("-", "release v1.2.3 has no SHA256SUMS asset"),
        (
            "deadbeef  some-other-file.exe\n",
            "SHA256SUMS has no line for irori-1.2.3-windows-amd64-installer.exe",
        ),
    ] {
        let mut g = FakeGitHub::new("v1.2.3", b"x");
        g.sums = Some(sums.into());
        let (_g, base) = g.serve().await;
        let dir = tempfile::tempdir().unwrap();
        let got = test_service(&base, "1.0.0", dir.path())
            .check_for_update()
            .await
            .unwrap_err();
        assert_eq!(got, err);
    }
}

#[tokio::test]
async fn find_update_wrong_platform_asset() {
    let mut g = FakeGitHub::new("v1.2.3", b"x");
    g.asset = "irori-1.2.3-macos-arm64.app.zip".into();
    let (_g, base) = g.serve().await;
    let dir = tempfile::tempdir().unwrap();
    let got = test_service(&base, "1.0.0", dir.path())
        .check_for_update()
        .await
        .unwrap_err();
    assert_eq!(got, "release v1.2.3 has no windows installer asset");
}

#[tokio::test]
async fn download_verified() {
    let (_g, base) = FakeGitHub::new("v1.2.3", b"installer bytes").serve().await;
    let dir = tempfile::tempdir().unwrap();
    let s = test_service(&base, "1.0.0", dir.path());
    let stale = dir.path().join("irori-1.1.0-windows-amd64-installer.exe");
    let log = dir.path().join("update.log");
    std::fs::write(&stale, b"old").unwrap();
    std::fs::write(&log, b"keep").unwrap();
    // Go keeps a bare `.log` too (`filepath.Ext(".log") == ".log"`).
    let bare_log = dir.path().join(".log");
    std::fs::write(&bare_log, b"keep").unwrap();
    std::fs::create_dir(dir.path().join("unpacked")).unwrap();
    let mut info = s.check_for_update().await.unwrap().unwrap();
    let path = s.download_verified(&info).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"installer bytes");
    assert_eq!(path.parent().unwrap(), dir.path());
    assert!(!stale.exists(), "stale installer kept");
    assert!(log.exists() && bare_log.exists() && dir.path().join("unpacked").exists());

    info.sha256 = "0".repeat(64);
    let err = s.download_verified(&info).await.unwrap_err();
    assert!(err.starts_with("update checksum mismatch: got "), "{err}");
    assert!(!path.exists(), "mismatched installer left on disk");
}

#[tokio::test]
async fn download_keeps_only_base_name() {
    let (_g, base) = FakeGitHub::new("v1.2.3", b"abc").serve().await;
    let dir = tempfile::tempdir().unwrap();
    let s = test_service(&base, "1.0.0", dir.path());
    let mut info = s.check_for_update().await.unwrap().unwrap();
    info.asset_name = format!("../../{TEST_ASSET}");
    let p = s.download_verified(&info).await.unwrap();
    assert_eq!(p, dir.path().join(TEST_ASSET));
}

/// Go TestCheckAndDownloadPlatform with this OS's own platform.
#[tokio::test]
async fn check_and_download_platform() {
    let (g, base) = FakeGitHub::new("v1.2.3", b"installer bytes").serve().await;
    let dir = tempfile::tempdir().unwrap();
    let s = Updater::new("x/y", "1.0.0", dir.path()).with_api_base(&base);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    s.set_on_change(Arc::new(move |st: &Status| {
        seen2.lock().unwrap().push(st.phase.clone())
    }));
    let r = s.check_and_download().await;
    let st = s.status();
    if cfg!(target_os = "macos") {
        assert!(
            r.is_err() && st.phase == PHASE_ERROR && st.error.contains("Irori.app"),
            "{st:?}"
        );
        assert_eq!(g.hits.load(Ordering::SeqCst), 0);
        return;
    }
    if !SUPPORTED {
        assert!(
            r.is_err() && st.phase == PHASE_ERROR && st.error.contains("not supported"),
            "{st:?}"
        );
        assert_eq!(g.hits.load(Ordering::SeqCst), 0);
        return;
    }
    let st = r.unwrap();
    assert_eq!(st.phase, PHASE_READY);
    assert_eq!(st.version, "1.2.3");
    assert_eq!(st.installer_path, dir.path().join(TEST_ASSET));
    assert_eq!(
        *seen.lock().unwrap(),
        vec![PHASE_CHECKING, PHASE_DOWNLOADING, PHASE_READY]
    );
}

/// D11: a Go v0.5.0 updater (same rules) offers the first Rust release, and
/// this updater, once at 0.6.0, does not offer it again.
#[tokio::test]
async fn go_050_takes_rust_060() {
    let mut g = FakeGitHub::new("v0.6.0", b"rust installer");
    g.asset = "irori-0.6.0-windows-amd64-installer.exe".into();
    let (_g, base) = g.serve().await;
    let dir = tempfile::tempdir().unwrap();
    let info = test_service(&base, "0.5.0", dir.path())
        .check_for_update()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.version, "0.6.0");
    assert_eq!(
        test_service(&base, "0.6.0", dir.path())
            .check_for_update()
            .await
            .unwrap(),
        None
    );
}

/// The diagnostics go to the logger the app sets (irori_core::log, so
/// irori.log), not stderr, which the GUI exe does not have.
#[tokio::test]
async fn failures_reach_the_logger() {
    let mut g = FakeGitHub::new("v1.2.3", b"x");
    g.sums = Some("-".into());
    let (_g, base) = g.serve().await;
    let dir = tempfile::tempdir().unwrap();
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let l2 = lines.clone();
    let s = test_service(&base, "1.0.0", dir.path())
        .with_logger(Arc::new(move |l: &str| l2.lock().unwrap().push(l.into())));
    s.auto_update().await;
    let lines = lines.lock().unwrap().clone();
    assert!(
        lines
            .iter()
            .any(|l| l == "irori: update: release v1.2.3 has no SHA256SUMS asset"),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("irori: auto-update failed: ")),
        "{lines:?}"
    );
}
