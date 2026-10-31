//! `irori attach --session <id> [--cwd …] [--mod-version …]` (I:attach.go):
//! the Irori Claude Code mod's wake stream. stdout carries only event lines
//! (NDJSON); logs go to stderr. The mod spawns it with pipes and stdin
//! already closed, so stdin is never read. It exits when the mod kills it,
//! when its parent process dies, on Ctrl+C, or when a newer attach for the
//! same session supersedes it.

use std::time::{Duration, SystemTime};

use tokio_util::sync::CancellationToken;

use crate::datadir;

/// The fallback parent check's interval (Go `parentPollEvery`).
const PARENT_POLL_EVERY: Duration = Duration::from_secs(5);

/// The flags, Go `flag` style: `-f v`, `--f v`, `-f=v`, `--f=v`; parsing
/// stops at the first non-flag argument or `--`. Merge note: this moves to
/// `gocompat::flag` when another role needs it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub session: String,
    pub cwd: String,
    pub mod_version: String,
}

const USAGE: &str = "Usage of attach:
  -cwd string
    \tthe session's working directory (for logs)
  -mod-version string
    \tthe Irori mod's version
  -session string
    \tClaude Code session id (required)
";

/// Go `flag.FlagSet.Parse` over the three string flags. An error is what
/// Go returns (and the caller prints as `irori: <err>`); Go also writes it
/// and the usage to stderr, which [`main`] does.
pub fn parse(args: &[String]) -> Result<Flags, String> {
    let mut f = Flags::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.len() < 2 || !a.starts_with('-') {
            break;
        }
        let mut name = &a[1..];
        if let Some(rest) = name.strip_prefix('-') {
            if rest.is_empty() {
                break; // "--" ends the flags
            }
            name = rest;
        }
        if name.is_empty() || name.starts_with('-') || name.starts_with('=') {
            return Err(format!("bad flag syntax: {a}"));
        }
        i += 1;
        let (name, value) = match name.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (name, None),
        };
        if name == "help" || name == "h" {
            return Err("flag: help requested".into());
        }
        let slot = match name {
            "session" => &mut f.session,
            "cwd" => &mut f.cwd,
            "mod-version" => &mut f.mod_version,
            _ => return Err(format!("flag provided but not defined: -{name}")),
        };
        *slot = match value {
            Some(v) => v,
            None if i < args.len() => {
                i += 1;
                args[i - 1].clone()
            }
            None => return Err(format!("flag needs an argument: -{name}")),
        };
    }
    Ok(f)
}

/// Go `log.New(os.Stderr, "", log.LstdFlags)`: `2006/01/02 15:04:05 msg`.
fn log_line(msg: &str) {
    let now = SystemTime::now();
    let t = gocompat::time::GoTime::from_system(now);
    let secs = t.unix_milli_value().div_euclid(1000);
    let (y, mo, d, h, mi, s) = t.with_offset(sys::tz::local_offset(secs)).civil();
    eprintln!("{y:04}/{mo:02}/{d:02} {h:02}:{mi:02}:{s:02} {msg}");
}

/// Runs the role; `Err` is printed by `main` as `irori: <err>`, exit 1.
pub fn main(args: &[String]) -> Result<(), String> {
    let f = match parse(args) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            eprint!("{USAGE}");
            return Err(e);
        }
    };
    let dir = datadir::process_data_dir()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let stop = CancellationToken::new();
    // The blocking parent wait's token, canceled with `stop`.
    let waiting = sys::cancel::Cancel::new();

    // Our parent started before we did; a process at its pid that started
    // later is a reuse.
    let me = sys::proc::start_time(i64::from(std::process::id()));
    let ppid = sys::proc::parent_pid();
    if reparented(ppid) {
        log_line(&format!("attach: parent {ppid} is gone; exiting"));
        stop.cancel();
    } else {
        let (stop, waiting) = (stop.clone(), waiting.clone());
        std::thread::spawn(move || {
            if watch_parent(&waiting, ppid, me) {
                log_line(&format!("attach: parent {ppid} is gone; exiting"));
                stop.cancel();
            }
        });
    }

    rt.block_on(async {
        let s2 = stop.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                s2.cancel();
            }
        });
        let s3 = stop.clone();
        tokio::spawn(async move {
            s3.cancelled().await;
            waiting.cancel();
        });
        attach::runner::run(attach::runner::Options {
            session: f.session,
            cwd: f.cwd,
            mod_version: f.mod_version,
            data_dir: dir,
            stdout: Box::new(std::io::stdout()),
            log: Box::new(log_line),
            cancel: stop.clone(),
            ..attach::runner::Options::default()
        })
        .await
    })
}

/// Waits until the process that spawned us is gone (`true`), or `cancel`
/// ends (`false`). Windows waits on the parent's handle and macOS on a
/// kqueue NOTE_EXIT; where that can't be done it checks every
/// [`PARENT_POLL_EVERY`]. `started_by` (our own
/// start) rules out a reused pid either way. Go `watchParent`.
pub fn watch_parent(
    cancel: &sys::cancel::Cancel,
    ppid: i64,
    started_by: Option<SystemTime>,
) -> bool {
    let r = sys::proc::wait_exit(cancel, ppid, started_by);
    if cancel.is_canceled() {
        return false;
    }
    if r == Err(sys::proc::WaitError::CantWait) {
        loop {
            if cancel.sleep(PARENT_POLL_EVERY) {
                return false;
            }
            if reparented(ppid) || !sys::proc::alive(ppid) {
                break;
            }
            if let (Some(st), Some(by)) = (sys::proc::start_time(ppid), started_by)
                && st > by
            {
                break;
            }
        }
    }
    true
}

/// Whether the process that spawned us (`ppid`, read at start) is no
/// longer our parent. Windows never reparents, so there it is always false.
fn reparented(ppid: i64) -> bool {
    if cfg!(windows) {
        return false;
    }
    ppid <= 1 || sys::proc::parent_pid() != ppid
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, SystemTime};

    use super::{Flags, parse, watch_parent};

    /// The stand-in parent `watch_parent_*` spawn: it sleeps for
    /// IRORI_ATTACH_SLEEP_MS and exits. It does nothing in a normal run.
    #[test]
    fn helper_sleep() {
        let Some(ms) = std::env::var("IRORI_ATTACH_SLEEP_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        else {
            return;
        };
        std::thread::sleep(Duration::from_millis(ms));
        std::process::exit(0);
    }

    fn sleeper(ms: u64) -> std::process::Child {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "attach::tests::helper_sleep", "--nocapture"])
            .env("IRORI_ATTACH_SLEEP_MS", ms.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Go TestWatchParent: the parent's exit is noticed at once, not on
    /// the 5 s poll, and a cancel while it lives ends the watch unorphaned.
    #[test]
    fn watch_parent_cases() {
        if !cfg!(any(windows, target_os = "macos")) {
            return; // elsewhere it polls every 5 s
        }
        // The parent exits.
        let mut kid = sleeper(300);
        let c = sys::cancel::Cancel::new();
        let t = Instant::now();
        let orphaned = watch_parent(&c, i64::from(kid.id()), Some(SystemTime::now()));
        assert!(orphaned, "returned without the parent gone");
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "took {:?}",
            t.elapsed()
        );
        let _ = kid.wait();

        // The watch is canceled first.
        let mut kid = sleeper(3_600_000);
        let c = sys::cancel::Cancel::new();
        let c2 = c.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            c2.cancel();
        });
        let t = Instant::now();
        let orphaned = watch_parent(&c, i64::from(kid.id()), Some(SystemTime::now()));
        assert!(!orphaned, "canceled as orphaned with the parent alive");
        assert!(t.elapsed() < Duration::from_secs(3));
        let _ = kid.kill();
        let _ = kid.wait();

        // A process at the pid that started after us is a reused pid: the
        // parent meant is gone.
        let before = SystemTime::now() - Duration::from_secs(60);
        let mut kid = sleeper(3_600_000);
        let t = Instant::now();
        let orphaned = watch_parent(
            &sys::cancel::Cancel::new(),
            i64::from(kid.id()),
            Some(before),
        );
        assert!(orphaned, "a reused pid kept the watch");
        assert!(t.elapsed() < Duration::from_secs(3));
        let _ = kid.kill();
        let _ = kid.wait();
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_parse_like_go() {
        let want = Flags {
            session: "s1".into(),
            cwd: "C:/work".into(),
            mod_version: "0.4.1".into(),
        };
        // The mod's own argv (register.tsx).
        assert_eq!(
            parse(&args(&[
                "--session",
                "s1",
                "--cwd",
                "C:/work",
                "--mod-version",
                "0.4.1"
            ])),
            Ok(Flags { ..want })
        );
        assert_eq!(
            parse(&args(&[
                "-session=s1",
                "-cwd",
                "C:/work",
                "--mod-version=0.4.1",
                "extra",
                "-x"
            ]))
            .unwrap(),
            Flags {
                session: "s1".into(),
                cwd: "C:/work".into(),
                mod_version: "0.4.1".into(),
            }
        );
        assert_eq!(parse(&args(&["--", "-x"])), Ok(Flags::default()));
        assert_eq!(
            parse(&args(&["-x"])),
            Err("flag provided but not defined: -x".into())
        );
        assert_eq!(
            parse(&args(&["--session"])),
            Err("flag needs an argument: -session".into())
        );
        assert_eq!(parse(&args(&["---s"])), Err("bad flag syntax: ---s".into()));
        assert_eq!(parse(&args(&["-h"])), Err("flag: help requested".into()));
    }
}
