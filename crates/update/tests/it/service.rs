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
    /// Replaces `sums` while set (card #270: a release's SHA256SUMS fixed
    /// after a bad one).
    sums_now: Mutex<Option<Option<String>>>,
    payload: Vec<u8>,
    asset: String,
    hits: AtomicUsize,
    /// Downloads of the asset itself (card #270: a phone's check never
    /// downloads).
    asset_hits: AtomicUsize,
    base: Mutex<String>,
}

impl FakeGitHub {
    fn new(tag: &str, payload: &[u8]) -> FakeGitHub {
        FakeGitHub {
            tag: tag.into(),
            sums: None,
            sums_now: Mutex::new(None),
            payload: payload.to_vec(),
            asset: TEST_ASSET.into(),
            hits: AtomicUsize::new(0),
            asset_hits: AtomicUsize::new(0),
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
        let sums = g
            .sums_now
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| g.sums.clone());
        let body = sums.unwrap_or_else(|| {
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
        g.asset_hits.fetch_add(1, Ordering::SeqCst);
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

// ---- Android (card #270) ----------------------------------------------------

fn apk_name() -> String {
    format!("irori-1.2.3-android-{}.apk", release::goarch())
}

#[derive(Default)]
struct Opens {
    paths: Mutex<Vec<PathBuf>>,
    outcome: Mutex<Option<ApkOutcome>>,
}

fn opener(o: &Arc<Opens>) -> ApkInstaller {
    let o = o.clone();
    Arc::new(move |p: &Path| {
        o.paths.lock().unwrap().push(p.to_path_buf());
        Ok(o.outcome.lock().unwrap().unwrap_or(ApkOutcome::Opened))
    })
}

async fn android(
    payload: &[u8],
    sums: Option<String>,
) -> (Arc<FakeGitHub>, Updater, Arc<Opens>, tempfile::TempDir) {
    let mut g = FakeGitHub::new("v1.2.3", payload);
    g.asset = apk_name();
    g.sums = sums;
    let (g, base) = g.serve().await;
    let dir = tempfile::tempdir().unwrap();
    let opens = Arc::new(Opens::default());
    let s = Updater::new("x/y", "1.0.0", dir.path())
        .with_api_base(&base)
        .with_platform(Platform::Android)
        .with_apk_installer(opener(&opens));
    (g, s, opens, dir)
}

#[test]
fn android_apk_asset_names() {
    let a = release::goarch();
    assert!(release::is_android_apk_asset(
        &format!("irori-0.7.0-android-{a}.apk"),
        a
    ));
    assert!(release::is_android_apk_asset(
        &format!("Irori-0.7.0-Android-{a}.APK"),
        a
    ));
    for n in [
        format!("irori-0.7.0-android-{a}.apk.idsig"),
        format!("irori-0.7.0-android-{a}.aab"),
        "irori-0.7.0-android-x86.apk".to_string(),
        "irori-0.7.0-macos-arm64.app.zip".to_string(),
        format!("other-0.7.0-android-{a}.apk"),
    ] {
        assert!(!release::is_android_apk_asset(&n, a), "{n}");
    }
    let sums = format!(
        "{}  irori-0.7.0-windows-amd64-installer.exe\n{}  *irori-0.7.0-android-{a}.apk\n",
        "a".repeat(64),
        "B".repeat(64)
    );
    assert_eq!(
        release::checksum_for(&sums, &format!("irori-0.7.0-android-{a}.apk")),
        Some("b".repeat(64))
    );
}

#[tokio::test]
async fn android_check_offers_without_downloading() {
    let (g, s, opens, dir) = android(b"apk bytes", None).await;
    assert!(s.supported());
    let st = s.check_and_download().await.unwrap();
    assert_eq!(
        (st.phase.as_str(), st.version.as_str()),
        (PHASE_AVAILABLE, "1.2.3")
    );
    assert_eq!(
        g.asset_hits.load(Ordering::SeqCst),
        0,
        "a check never downloads the APK"
    );
    assert!(
        std::fs::read_dir(dir.path())
            .map(|d| d.count() == 0)
            .unwrap_or(true)
    );
    assert!(opens.paths.lock().unwrap().is_empty());
}

#[tokio::test]
async fn android_install_downloads_verifies_and_opens() {
    let (g, s, opens, dir) = android(b"apk bytes", None).await;
    s.auto_update().await;
    assert_eq!(s.status().phase, PHASE_AVAILABLE);
    let st = s.install_apk().await.unwrap();
    assert_eq!(st.phase, PHASE_READY);
    assert!(st.error.is_empty(), "{st:?}");
    let apk = dir.path().join(apk_name());
    assert_eq!(std::fs::read(&apk).unwrap(), b"apk bytes");
    assert_eq!(*opens.paths.lock().unwrap(), vec![apk.clone()]);
    // Tapping Install again (the screen was cancelled) reopens it without
    // downloading again.
    s.install_apk().await.unwrap();
    assert_eq!(g.asset_hits.load(Ordering::SeqCst), 1);
    assert_eq!(opens.paths.lock().unwrap().len(), 2);
    // Next start: the old APK is cleaned up.
    let next = Updater::new("x/y", "1.2.3", dir.path()).with_platform(Platform::Android);
    next.auto_update().await;
    assert!(
        !apk.exists(),
        "the installed APK is dropped at the next start"
    );
}

#[tokio::test]
async fn android_bad_hash_is_refused() {
    let wrong = format!("{}  {}\n", "0".repeat(64), apk_name());
    let (_g, s, opens, dir) = android(b"tampered", Some(wrong)).await;
    s.check_and_download().await.unwrap();
    let e = s.install_apk().await.unwrap_err();
    assert!(e.contains("checksum mismatch"), "{e}");
    let st = s.status();
    assert_eq!(st.phase, PHASE_ERROR);
    assert!(st.error.contains("checksum mismatch"));
    assert!(
        !dir.path().join(apk_name()).exists(),
        "the bad APK is deleted"
    );
    assert!(
        opens.paths.lock().unwrap().is_empty(),
        "nothing is installed"
    );
}

#[tokio::test]
async fn android_try_again_reads_the_fixed_release() {
    let wrong = format!("{}  {}\n", "0".repeat(64), apk_name());
    let (g, s, opens, dir) = android(b"apk bytes", Some(wrong)).await;
    s.check_and_download().await.unwrap();
    assert!(s.install_apk().await.is_err());
    // The release's SHA256SUMS is fixed: Try again picks it up.
    *g.sums_now.lock().unwrap() = Some(None);
    let st = s.install_apk().await.unwrap();
    assert_eq!(st.phase, PHASE_READY, "{st:?}");
    assert_eq!(
        *opens.paths.lock().unwrap(),
        vec![dir.path().join(apk_name())]
    );
}

#[tokio::test]
async fn android_asks_for_the_install_permission() {
    let (_g, s, opens, _dir) = android(b"apk bytes", None).await;
    *opens.outcome.lock().unwrap() = Some(ApkOutcome::NeedsPermission);
    s.check_and_download().await.unwrap();
    let st = s.install_apk().await.unwrap();
    assert_eq!(st.phase, PHASE_READY);
    assert_eq!(st.error, NEEDS_INSTALL_PERMISSION);
    // Allowed now: Install again opens the screen and clears the line.
    *opens.outcome.lock().unwrap() = Some(ApkOutcome::Opened);
    let st = s.install_apk().await.unwrap();
    assert!(st.error.is_empty(), "{st:?}");
}

#[tokio::test]
async fn android_never_installs_on_exit_or_restart() {
    let (_g, s, opens, _dir) = android(b"apk bytes", None).await;
    s.check_and_download().await.unwrap();
    s.install_apk().await.unwrap();
    assert!(s.restart_to_update().is_err());
    s.install_on_exit();
    assert_eq!(s.status().phase, PHASE_READY);
    assert_eq!(opens.paths.lock().unwrap().len(), 1);
}
