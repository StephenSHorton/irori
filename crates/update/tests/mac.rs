//! Go update/mac_darwin_test.go: the macOS `.app` swap against a fake
//! GitHub on 127.0.0.1 and a fake, ad hoc signed Irori.app in a temp dir
//! (surface.md §6). The updater's `with_running_app` and `with_pid` seams
//! stand in for Go's `macPlatform(s, app, pid)`. macOS only: it runs
//! `codesign`, `ditto`, `plutil` and the real swap helper.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use sha2::{Digest, Sha256};
use update::macapp::{BUNDLE_ID, SWAP_SCRIPT, side_paths};
use update::*;

/// Go `fakeApp`: a minimal Irori.app at `version` under `dir`, signed ad
/// hoc with Irori's bundle id; the executable is a copy of /usr/bin/true.
fn fake_app(dir: &Path, version: &str) -> PathBuf {
    let app = dir.join("Irori.app");
    let macos = app.join("Contents").join("MacOS");
    std::fs::create_dir_all(&macos).unwrap();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key><string>irori</string>
	<key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
	<key>CFBundleName</key><string>Irori</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>{version}</string>
	<key>CFBundleVersion</key><string>{version}</string>
</dict>
</plist>
"#
    );
    std::fs::write(app.join("Contents").join("Info.plist"), plist).unwrap();
    let exe = macos.join("irori");
    std::fs::copy("/usr/bin/true", &exe).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    run(Command::new("codesign")
        .args([
            "--force",
            "--deep",
            "--sign",
            "-",
            "--identifier",
            BUNDLE_ID,
        ])
        .arg(&app));
    app
}

fn run(cmd: &mut Command) {
    let o = cmd.output().unwrap();
    assert!(
        o.status.success(),
        "{cmd:?}: {}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

/// Go `zipApp`: as package.sh zips it (ditto --keepParent).
fn zip_app(app: &Path, zip: &Path) -> Vec<u8> {
    run(Command::new("ditto")
        .args(["-c", "-k", "--keepParent"])
        .arg(app)
        .arg(zip));
    std::fs::read(zip).unwrap()
}

fn bundle_version(app: &Path) -> String {
    let o = Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(app.join("Contents").join("Info.plist"))
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Go `sleeper`: a stand-in for the GUI process the swap helper waits on.
struct Sleeper(Child);

impl Sleeper {
    fn start() -> Sleeper {
        Sleeper(Command::new("/bin/sleep").arg("60").spawn().unwrap())
    }
    fn kill(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait(); // a zombie still answers kill -0
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        self.kill();
    }
}

/// A latest release with the `.app.zip` and its SHA256SUMS line.
struct FakeGitHub {
    tag: String,
    asset: String,
    payload: Vec<u8>,
    base: Mutex<String>,
}

async fn serve(g: FakeGitHub) -> String {
    let g = Arc::new(g);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    *g.base.lock().unwrap() = base.clone();
    let app = Router::new()
        .route("/repos/x/y/releases/latest", get(latest))
        .route("/dl/{name}", get(dl))
        .with_state(g);
    tokio::spawn(async move {
        axum::serve(l, app).await.unwrap();
    });
    base
}

async fn latest(State(g): State<Arc<FakeGitHub>>) -> impl IntoResponse {
    let base = g.base.lock().unwrap().clone();
    serde_json::json!({
        "tag_name": g.tag,
        "body": "notes",
        "assets": [
            {"name": g.asset, "browser_download_url": format!("{base}/dl/{}", g.asset)},
            {"name": "SHA256SUMS", "browser_download_url": format!("{base}/dl/SHA256SUMS")},
        ],
    })
    .to_string()
}

async fn dl(
    State(g): State<Arc<FakeGitHub>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::response::Response {
    if name == "SHA256SUMS" {
        return format!("{}  {}\n", hex::encode(Sha256::digest(&g.payload)), g.asset)
            .into_response();
    }
    if name == g.asset {
        return g.payload.clone().into_response();
    }
    axum::http::StatusCode::NOT_FOUND.into_response()
}

struct Release {
    s: Updater,
    live: PathBuf,
    dir: PathBuf,
    _root: tempfile::TempDir,
}

/// Go `macRelease`: Irori 1.0.0 installed under root/Applications and a fake
/// release 1.2.3 whose `.app.zip` holds a bundle at `version`, altered by
/// `tweak` after signing. `pid` is the GUI process the helper waits for.
async fn mac_release(version: &str, tweak: Option<fn(&Path)>, pid: Option<u32>) -> Release {
    let root = tempfile::tempdir().unwrap();
    let live = fake_app(&root.path().join("Applications"), "1.0.0");
    let new_app = fake_app(&root.path().join("build"), version);
    if let Some(t) = tweak {
        t(&new_app);
    }
    let asset = format!("irori-1.2.3-macos-{}.app.zip", release::goarch());
    let payload = zip_app(&new_app, &root.path().join(&asset));
    let base = serve(FakeGitHub {
        tag: "v1.2.3".into(),
        asset,
        payload,
        base: Mutex::default(),
    })
    .await;
    let dir = root.path().join("updates");
    let l = live.clone();
    let mut s = Updater::new("x/y", "1.0.0", &dir)
        .with_api_base(&base)
        .with_platform(Platform::Mac)
        .with_running_app(Arc::new(move || Ok(l.clone())));
    if let Some(p) = pid {
        s = s.with_pid(p);
    }
    Release {
        s,
        live,
        dir,
        _root: root,
    }
}

/// Go TestMacUpdateSwapsBundle: find the `.app.zip`, verify it, unpack and
/// check it, stage it beside the live bundle, and on quit swap it in once
/// the GUI process is gone.
#[tokio::test(flavor = "multi_thread")]
async fn mac_update_swaps_bundle() {
    let mut gui = Sleeper::start();
    let r = mac_release("1.2.3", None, Some(gui.0.id())).await;
    let st = r.s.check_and_download().await.unwrap();
    let (next, old) = side_paths(&r.live);
    assert_eq!(st.phase, PHASE_READY, "{st:?}");
    assert_eq!(st.installer_path, next);
    assert_eq!(bundle_version(&next), "1.2.3");

    r.s.install_on_exit(); // quitting: swap without relaunch
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        bundle_version(&r.live),
        "1.0.0",
        "swapped while the GUI still runs"
    );
    gui.kill();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if bundle_version(&r.live) == "1.2.3" && !next.exists() && !old.exists() {
            break;
        }
        if Instant::now() > deadline {
            let log = std::fs::read_to_string(r.dir.join("update.log")).unwrap_or_default();
            panic!(
                "bundle not swapped (version {}, next {}, old {}); helper log:\n{log}",
                bundle_version(&r.live),
                next.exists(),
                old.exists()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    run(Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&r.live));
}

/// Go TestMacUpdateRefusesBadBundle: a bad signature, version or bundle id
/// never reaches ready and leaves nothing staged.
#[tokio::test(flavor = "multi_thread")]
async fn mac_update_refuses_bad_bundle() {
    fn tampered(app: &Path) {
        let res = app.join("Contents").join("Resources");
        let _ = std::fs::create_dir_all(&res);
        let _ = std::fs::write(res.join("extra.txt"), b"added after signing");
    }
    fn other_id(app: &Path) {
        let plist = app.join("Contents").join("Info.plist");
        let _ = Command::new("plutil")
            .args([
                "-replace",
                "CFBundleIdentifier",
                "-string",
                "com.example.other",
            ])
            .arg(&plist)
            .status();
        let _ = Command::new("codesign")
            .args(["--force", "--deep", "--sign", "-"])
            .arg(app)
            .status();
    }
    type Case = (&'static str, &'static str, Option<fn(&Path)>, &'static str);
    let cases: [Case; 3] = [
        ("tampered", "1.2.3", Some(tampered), "signature"),
        ("wrong version", "1.2.2", None, "version"),
        ("wrong bundle id", "1.2.3", Some(other_id), "bundle id"),
    ];
    for (name, version, tweak, want) in cases {
        let r = mac_release(version, tweak, None).await;
        let err = r.s.check_and_download().await.unwrap_err();
        assert_eq!(r.s.status().phase, PHASE_ERROR, "{name}");
        assert!(
            err.contains(want),
            "{name}: {err}, want an error about {want}"
        );
        let (next, _) = side_paths(&r.live);
        assert!(!next.exists(), "{name}: {} left staged", next.display());
    }
}

/// Go TestMacInstallArgs: restart passes the helper this process's pid,
/// both bundles and relaunch=1.
#[tokio::test(flavor = "multi_thread")]
async fn mac_install_args() {
    let r = mac_release("1.2.3", None, None).await;
    r.s.check_and_download().await.unwrap();
    let calls: Arc<Mutex<Vec<Vec<String>>>> = Arc::default();
    let c2 = calls.clone();
    let launch: Launcher = Arc::new(move |p: &Path, args: &[String]| {
        let mut c = vec![p.to_string_lossy().into_owned()];
        c.extend(args.iter().cloned());
        c2.lock().unwrap().push(c);
        Ok(())
    });
    // The launcher is fixed at build time, so rebuild around the same dir
    // and live bundle with the ready status carried over.
    let st = r.s.status();
    let l = r.live.clone();
    let s = Updater::new("x/y", "1.0.0", &r.dir)
        .with_platform(Platform::Mac)
        .with_running_app(Arc::new(move || Ok(l.clone())))
        .with_launcher(launch);
    s.force_status(st);
    s.restart_to_update().unwrap();
    let (next, _) = side_paths(&r.live);
    let want: Vec<String> = vec![
        "/bin/sh".into(),
        "-c".into(),
        SWAP_SCRIPT.into(),
        "irori-update".into(),
        std::process::id().to_string(),
        r.live.to_string_lossy().into_owned(),
        next.to_string_lossy().into_owned(),
        "1".into(),
        r.dir.join("update.log").to_string_lossy().into_owned(),
    ];
    assert_eq!(*calls.lock().unwrap(), vec![want]);
}

/// Go TestMacCleanup: startup removes a staged bundle never installed and
/// an old one a swap left, and nothing else.
#[tokio::test(flavor = "multi_thread")]
async fn mac_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let live = dir.path().join("Irori.app");
    let (next, old) = side_paths(&live);
    let other = dir.path().join("Other.app");
    for p in [&live, &next, &old, &other] {
        std::fs::create_dir_all(p.join("Contents")).unwrap();
    }
    let l = live.clone();
    // A dev version, so auto_update cleans up and never checks.
    let s = Updater::new("x/y", "dev", dir.path().join("updates"))
        .with_platform(Platform::Mac)
        .with_running_app(Arc::new(move || Ok(l.clone())));
    s.auto_update().await;
    assert!(!next.exists(), "{} kept", next.display());
    assert!(!old.exists(), "{} kept", old.display());
    assert!(
        live.exists() && other.exists(),
        "live or other bundle removed"
    );
}
