//! The files that explain a crash after the fact (I:internal/diag;
//! surface.md §2.3, §7.16): the app log (`irori.log`, rotated by size)
//! and `crash.log`, where an unrecovered panic is written with its
//! backtrace. The desktop app is a GUI-subsystem exe and `serve
//! --headless` may run from Task Scheduler, so without these a crash
//! leaves nothing behind.
//!
//! Differences from Go, which writes crashes through the runtime's
//! `SetCrashOutput`: Rust has no runtime fatal-error hook, so a panic hook
//! writes `panic: <msg>` (with the thread name and location) and a forced
//! backtrace of the panicking thread, then runs the previous hook (stderr).
//! An abort that is not a panic (a stack overflow) is not caught. The
//! start line names the Rust toolchain where Go names its own, and has no
//! `BadToolchain` warning (that was a Go 1.26.0-1.26.1 bug).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The file in the data dir crashes go to.
pub const CRASH_NAME: &str = "crash.log";
/// The app log in the data dir.
pub const LOG_NAME: &str = "irori.log";

/// crash.log moves to crash.log.1 past this at startup.
const CRASH_MAX: u64 = 4 << 20;
/// irori.log rotates past this.
const LOG_MAX: u64 = 4 << 20;
/// irori.log.1 .. irori.log.3.
const LOG_KEEP: usize = 3;

fn open_append(path: &Path) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)
}

fn mkdir(dir: &Path) -> io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

/// Go `InstallCrashOutput`: any unrecovered panic is also written to
/// `<dir>/crash.log` with a backtrace. Every role calls it, so the file is
/// opened for append and shared. The returned handle lets the caller write
/// a start line.
pub fn install_crash_output(dir: &Path) -> io::Result<Arc<Mutex<File>>> {
    mkdir(dir)?;
    let path = dir.join(CRASH_NAME);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > CRASH_MAX) {
        // Fails if another role has it open; it grows a little more.
        let _ = std::fs::rename(&path, dir.join(format!("{CRASH_NAME}.1")));
    }
    let f = Arc::new(Mutex::new(open_append(&path)?));
    let out = f.clone();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "Box<dyn Any>".into());
        let at = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>").to_string();
        let bt = std::backtrace::Backtrace::force_capture();
        let text = format!(
            "{} panic: {msg}\n\nthread {name:?} at {at} (pid {})\n{bt}\n",
            stamp_rfc3339(),
            std::process::id()
        );
        if let Ok(mut f) = out.lock() {
            let _ = f.write_all(text.as_bytes());
            let _ = f.flush();
        }
        prev(info);
    }));
    Ok(f)
}

/// The toolchain this build used (core's build.rs), Go's
/// `runtime.Version()` slot.
pub fn toolchain() -> &'static str {
    env!("IRORI_RUSTC_VERSION")
}

fn go_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    }
}

fn go_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        a => a,
    }
}

/// Now in local time, RFC 3339 (Go `time.Now().Format(time.RFC3339)`).
fn stamp_rfc3339() -> String {
    let t = gocompat::time::GoTime::from_system(std::time::SystemTime::now());
    let secs = t.unix_milli_value().div_euclid(1000);
    t.with_offset(sys::tz::local_offset(secs)).format_rfc3339()
}

/// Go `log.LstdFlags`' prefix for now: `2006/01/02 15:04:05 `.
pub fn log_stamp() -> String {
    let t = gocompat::time::GoTime::from_system(std::time::SystemTime::now());
    let secs = t.unix_milli_value().div_euclid(1000);
    let (y, mo, d, h, mi, s) = t.with_offset(sys::tz::local_offset(secs)).civil();
    format!("{y:04}/{mo:02}/{d:02} {h:02}:{mi:02}:{s:02} ")
}

/// Go `StartLine`: which build is running, for the top of a log or before
/// a crash.
pub fn start_line(role: &str, version: &str) -> String {
    format!(
        "{} irori {version} start: role={role} pid={} {} {}/{}\n",
        stamp_rfc3339(),
        std::process::id(),
        toolchain(),
        go_os(),
        go_arch()
    )
}

/// An append-only file that rotates by size: name, name.1 .. name.keep
/// (Go `diag.Log`).
pub struct Log {
    inner: Mutex<LogInner>,
}

struct LogInner {
    path: PathBuf,
    max: u64,
    keep: usize,
    f: Option<File>,
    size: u64,
}

/// Go `OpenLog`: `<dir>/irori.log`, for the long-running roles (the app
/// and `serve --headless`; one of them runs per device).
pub fn open_log(dir: &Path) -> io::Result<Log> {
    Log::open(&dir.join(LOG_NAME), LOG_MAX, LOG_KEEP)
}

impl Log {
    pub fn open(path: &Path, max: u64, keep: usize) -> io::Result<Log> {
        if let Some(dir) = path.parent() {
            mkdir(dir)?;
        }
        let mut l = LogInner {
            path: path.to_path_buf(),
            max,
            keep,
            f: None,
            size: 0,
        };
        l.reopen()?;
        if l.size >= l.max {
            l.rotate();
        }
        Ok(Log {
            inner: Mutex::new(l),
        })
    }

    /// Appends `p`, rotating first when it would take the file past max.
    pub fn write(&self, p: &[u8]) -> io::Result<usize> {
        let mut l = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if l.f.is_none() {
            l.reopen()?;
        }
        if l.size > 0 && l.size + p.len() as u64 > l.max {
            l.rotate();
            if l.f.is_none() {
                return Err(io::Error::other("file already closed"));
            }
        }
        let f = l.f.as_mut().expect("opened above");
        f.write_all(p)?;
        l.size += p.len() as u64;
        Ok(p.len())
    }

    /// Closes the current file.
    pub fn close(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).f = None;
    }
}

impl LogInner {
    fn reopen(&mut self) -> io::Result<()> {
        let f = open_append(&self.path)?;
        self.size = f.metadata().map(|m| m.len()).unwrap_or(0);
        self.f = Some(f);
        Ok(())
    }

    /// name.N-1 → name.N, name → name.1, then a new name. A failed rename
    /// (another process holds the file) keeps writing to the current one
    /// rather than lose lines.
    fn rotate(&mut self) {
        self.f = None;
        let p = self.path.to_string_lossy().into_owned();
        for i in (2..=self.keep).rev() {
            let _ = std::fs::rename(format!("{p}.{}", i - 1), format!("{p}.{i}"));
        }
        let _ = std::fs::remove_file(format!("{p}.{}", self.keep + 1));
        let _ = std::fs::rename(&self.path, format!("{p}.1"));
        if self.reopen().is_err() {
            self.f = None;
            self.size = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // I:internal/diag/diag_test.go TestStartLine.
    #[test]
    fn start_line_names_the_build() {
        let s = start_line("gui", "0.4.2");
        for want in [
            "irori 0.4.2 start",
            "role=gui",
            toolchain(),
            &format!("{}/{}", go_os(), go_arch()),
        ] {
            assert!(s.contains(want), "{s:?} lacks {want:?}");
        }
        assert!(s.ends_with('\n'));
        assert!(toolchain().starts_with("rust1."), "{}", toolchain());
    }

    // TestLogRotates.
    #[test]
    fn log_rotates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("irori.log");
        let l = Log::open(&path, 100, 2).unwrap();
        let line = format!("{}\n", "x".repeat(39)); // 40 bytes: two fit, a third rotates
        for _ in 0..10 {
            l.write(line.as_bytes()).unwrap();
        }
        l.close();
        let p = path.to_string_lossy().into_owned();
        for name in [p.clone(), format!("{p}.1"), format!("{p}.2")] {
            let n = std::fs::metadata(&name).unwrap().len();
            assert!((1..=100).contains(&n), "{name} is {n} bytes");
        }
        assert!(!Path::new(&format!("{p}.3")).exists());
    }

    // TestLogRotatesBigFileAtOpen.
    #[test]
    fn log_rotates_big_file_at_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("irori.log");
        std::fs::write(&path, vec![0u8; 200]).unwrap();
        let l = Log::open(&path, 100, 3).unwrap();
        l.write(b"new\n").unwrap();
        l.close();
        assert_eq!(std::fs::read(&path).unwrap(), b"new\n");
        let old = dir.path().join("irori.log.1");
        assert_eq!(std::fs::metadata(old).unwrap().len(), 200);
    }

    const CRASH_CHILD_ENV: &str = "IRORI_DIAG_CRASH_CHILD";

    // TestCrashOutputLandsInFile: a panic nobody catches lands in
    // crash.log, not only on a stderr a GUI app does not have.
    #[test]
    fn crash_output_lands_in_file() {
        if let Ok(dir) = std::env::var(CRASH_CHILD_ENV) {
            let f = install_crash_output(Path::new(&dir)).unwrap();
            f.lock()
                .unwrap()
                .write_all(start_line("test", "dev").as_bytes())
                .unwrap();
            panic!("diag test crash");
        }
        let dir = tempfile::tempdir().unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "diag::tests::crash_output_lands_in_file",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CRASH_CHILD_ENV, dir.path())
            .output()
            .unwrap();
        assert!(!out.status.success(), "child did not fail");
        let s = std::fs::read_to_string(dir.path().join(CRASH_NAME)).unwrap();
        for want in [
            "role=test",
            "panic: diag test crash",
            "crash_output_lands_in_file",
        ] {
            assert!(s.contains(want), "crash.log lacks {want:?}:\n{s}");
        }
    }
}
