//! The real `irori` binary's desktop role on a scratch `IRORI_HOME`, with
//! the shell probe (app/src/shell/probe.rs) on, so no window
//! reaches the screen and no tray icon appears: a `--startup` launch hosts
//! the Core, a second launch hands off to it and exits, and the probe's
//! quit ends the first cleanly. Plus the role dispatch's texts.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn irori(home: &Path, claude: &Path, log: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_irori"));
    c.env("IRORI_HOME", home)
        .env("CLAUDE_CONFIG_DIR", claude)
        .env("IRORI_DEV_SHELL_LOG", log)
        .stdin(Stdio::null());
    c
}

fn lines(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn wait_for(what: &str, limit: Duration, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < limit, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_exit(child: &mut Child, limit: Duration) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            return st;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            panic!("process did not exit within {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A scratch dir removed with retries: WebView2's child processes can
/// hold the profile under `home/webview` for a moment after the app exits,
/// and a plain TempDir drop would then leave the dir behind.
struct Scratch(tempfile::TempDir);

impl Scratch {
    fn path(&self) -> &Path {
        self.0.path()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let p = self.0.path().to_path_buf();
        let start = Instant::now();
        while p.exists() && start.elapsed() < Duration::from_secs(20) {
            if std::fs::remove_dir_all(&p).is_err() {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}

#[test]
fn second_launch_hands_off_and_quit_closes_the_core() {
    let tmp = Scratch(tempfile::tempdir().unwrap());
    let home = tmp.path().join("home");
    let claude = tmp.path().join("claude");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&claude).unwrap();
    let log: PathBuf = tmp.path().join("shell.log");

    let first = irori(&home, &claude, &log)
        .arg("--startup")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn irori --startup");
    let mut first = Kill(first);
    wait_for(
        "the first launch to be ready",
        Duration::from_secs(120),
        || lines(&log).iter().any(|l| l == "ready"),
    );
    // The window's Core holds the lockfile, as a desktop (not headless)
    // Core. Only the flag is read; the token is never printed.
    let lf = irori_core::lockfile::read(&home).expect("lockfile of the running Core");
    assert!(!lf.headless, "the desktop app's lockfile says headless");
    assert_eq!(i64::from(first.0.id()), lf.pid);
    // A login launch shows nothing.
    assert!(!lines(&log).iter().any(|l| l == "show"));

    // A second launch (the desktop icon) hands off and exits.
    let mut second = irori(&home, &claude, &log)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn second irori");
    let st = wait_exit(&mut second, Duration::from_secs(60));
    assert!(st.success(), "second launch: {st}");
    wait_for(
        "the first launch to show its window",
        Duration::from_secs(10),
        || {
            let l = lines(&log);
            l.iter().any(|x| x == "second-instance") && l.iter().any(|x| x == "show")
        },
    );
    // The first one still runs and still owns the Core.
    assert!(first.0.try_wait().unwrap().is_none());
    assert_eq!(irori_core::lockfile::read(&home).unwrap().pid, lf.pid);

    // Quit (the tray's, through the probe) closes the Core and removes
    // the lockfile.
    std::fs::write(tmp.path().join("shell.log.quit"), "").unwrap();
    let st = wait_exit(&mut first.0, Duration::from_secs(60));
    assert!(st.success(), "first launch: {st}");
    assert!(lines(&log).iter().any(|l| l == "exiting"));
    assert!(
        irori_core::lockfile::read(&home).is_err(),
        "lockfile left behind"
    );
}

/// Ends a process this test didn't start (the copy a Reset starts), if a
/// failed assertion leaves it running.
struct KillPid(Option<i64>);
impl Drop for KillPid {
    fn drop(&mut self) {
        let Some(pid) = self.0 else { return };
        if !sys::proc::alive(pid) {
            return;
        }
        #[cfg(unix)]
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
        #[cfg(windows)]
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .status();
    }
}

fn device_id(home: &Path) -> String {
    let raw = std::fs::read(home.join("device.json")).expect("device.json");
    irori_identity::DeviceJson::parse(&raw)
        .expect("device.json parses")
        .id
}

/// Card #1095: Settings → Devices → Reset (the real command, through the
/// page) quits Irori and opens it again on its own, and the copy that
/// comes back is a new device. It keeps the environment: it comes back on
/// the same scratch home with the probe still on, so no window shows.
#[test]
fn reset_quits_and_opens_again_as_a_new_device() {
    let tmp = Scratch(tempfile::tempdir().unwrap());
    let home = tmp.path().join("home");
    let claude = tmp.path().join("claude");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&claude).unwrap();
    let log: PathBuf = tmp.path().join("shell.log");
    let readies = |log: &Path| lines(log).iter().filter(|l| *l == "ready").count();

    // A login launch, as Irori usually runs.
    let first = irori(&home, &claude, &log)
        .arg("--startup")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn irori --startup");
    let mut first = Kill(first);
    wait_for(
        "the first launch to be ready",
        Duration::from_secs(120),
        || readies(&log) == 1,
    );
    let old_pid = i64::from(first.0.id());
    let old_id = device_id(&home);
    // The page mounts once the Core is up; then press Reset through it.
    wait_for("the page to load", Duration::from_secs(60), || {
        lines(&log).iter().any(|l| l.starts_with("page-load"))
    });
    let mut copy = KillPid(None);
    std::fs::write(
        tmp.path().join("shell.log.eval"),
        "window.__TAURI_INTERNALS__.invoke('devices_reset_device')",
    )
    .unwrap();

    // The old one quits by itself.
    let st = wait_exit(&mut first.0, Duration::from_secs(60));
    assert!(st.success(), "the Reset's quit: {st}");
    // And a new one comes up on the same home, as a new device. Its pid
    // is noted as soon as it holds the home, so a failure below ends it.
    let mut lf = None;
    wait_for(
        "Irori to open again after the Reset",
        Duration::from_secs(120),
        || {
            lf = irori_core::lockfile::read(&home)
                .ok()
                .filter(|l| l.pid != old_pid);
            lf.is_some()
        },
    );
    let lf = lf.unwrap();
    copy.0 = Some(lf.pid);
    wait_for(
        "the reopened Irori to be ready",
        Duration::from_secs(120),
        || readies(&log) == 2,
    );
    assert!(!lf.headless);
    assert!(sys::proc::alive(lf.pid), "the reopened Irori is not running");
    assert!(
        !home.join("reset-pending").exists(),
        "the Reset was not finished"
    );
    let moved = std::fs::read_dir(&home)
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| e.file_name().to_string_lossy().starts_with("reset-"));
    assert!(moved, "no reset-<time> folder in the home");
    let new_id = device_id(&home);
    assert_ne!(new_id, old_id, "the reopened Irori kept the old device id");
    // Its quit closes it cleanly.
    std::fs::write(tmp.path().join("shell.log.quit"), "").unwrap();
    wait_for("the reopened Irori to quit", Duration::from_secs(60), || {
        !sys::proc::alive(lf.pid)
    });
    copy.0 = None;
    assert!(
        irori_core::lockfile::read(&home).is_err(),
        "lockfile left behind"
    );
}

/// One AppKit quit of a probe run (card #535): `-[NSApplication terminate:]`
/// exits inside the event loop, so the shell's shutdown must not wait for
/// `run_return`. `model` is the speech model to link into the home and what
/// `<log>.voice` says; `how` is what `<log>.terminate` says.
#[cfg(target_os = "macos")]
fn appkit_quit(model: Option<(&Path, &str)>, how: &str) {
    let tmp = Scratch(tempfile::tempdir().unwrap());
    let home = tmp.path().join("home");
    let claude = tmp.path().join("claude");
    std::fs::create_dir_all(home.join("voice")).unwrap();
    std::fs::create_dir_all(&claude).unwrap();
    let log: PathBuf = tmp.path().join("shell.log");
    if let Some((m, _)) = model {
        std::os::unix::fs::symlink(m, home.join("voice").join(m.file_name().unwrap())).unwrap();
    }
    let child = irori(&home, &claude, &log)
        .arg("--startup")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn irori --startup");
    let mut child = Kill(child);
    wait_for("the app to be ready", Duration::from_secs(120), || {
        lines(&log).iter().any(|l| l == "ready")
    });
    if let Some((_, voice)) = model {
        std::fs::write(tmp.path().join("shell.log.voice"), voice).unwrap();
        wait_for("the model to load", Duration::from_secs(60), || {
            lines(&log).iter().any(|l| l.starts_with("voice"))
        });
        assert!(lines(&log).iter().any(|l| l == "voice-loaded"));
    }
    std::fs::write(tmp.path().join("shell.log.terminate"), how).unwrap();
    let st = wait_exit(&mut child.0, Duration::from_secs(60));
    assert!(st.success(), "{how:?} quit: {st}");
    assert!(lines(&log).iter().any(|l| l == "closed"));
    assert!(
        irori_core::lockfile::read(&home).is_err(),
        "lockfile left behind"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn appkit_quit_closes_the_core() {
    appkit_quit(None, "");
    appkit_quit(None, "menu");
}

/// The abort itself needs the speech model (148 MB, not in the repo): set
/// IRORI_TEST_VOICE_MODEL to a ggml-base.en.bin to run this.
#[cfg(target_os = "macos")]
#[test]
fn appkit_quit_frees_the_speech_model() {
    let Some(model) = std::env::var_os("IRORI_TEST_VOICE_MODEL") else {
        return;
    };
    let model = Path::new(&model);
    for voice in ["kept", "session"] {
        appkit_quit(Some((model, voice)), "");
        appkit_quit(Some((model, voice)), "menu");
    }
}

#[test]
fn role_dispatch_texts() {
    let tmp = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_irori"))
            .args(args)
            .env("IRORI_HOME", tmp.path())
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let out = run(&["version"]);
    assert!(out.status.success());
    let want = format!("irori {}\n", option_env!("IRORI_VERSION").unwrap_or("dev"));
    assert_eq!(String::from_utf8_lossy(&out.stdout), want);

    let out = run(&["bogus"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim_end(),
        "irori: unknown command \"bogus\" (gui, serve, mcp, attach, events, pair, node, version)"
    );

    let out = run(&["serve"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim_end(),
        "irori: serve needs --headless (the desktop app is plain `irori`)"
    );
    // These roles create nothing in the data dir but Go's diagnostics
    // (setupDiagnostics: crash.log for every role but version, and the
    // start line and irori.log for serve).
    for e in std::fs::read_dir(tmp.path()).unwrap() {
        let name = e.unwrap().file_name().into_string().unwrap();
        assert!(
            name == "crash.log" || name.starts_with("irori.log"),
            "{name}"
        );
    }
}
