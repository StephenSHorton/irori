//! One window per data dir (card #674), in place of Tauri's
//! single-instance plugin: the running window listens on a loopback port it
//! writes to `<data>/desk-show.port`; a second launch on the same data dir
//! sends it `show` and exits, and the first one's window comes forward.
//! Std only. Any local process could ask the window to show; that is all
//! the port does.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const FILE: &str = "desk-show.port";
const SHOW: &str = "show";

fn port_file(data: &Path) -> PathBuf {
    data.join(FILE)
}

/// Asks the running window on `data` to show. True when one answered.
pub fn hand_off(data: &Path) -> bool {
    let Some(port) = std::fs::read_to_string(port_file(data))
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
    else {
        return false;
    };
    let addr = (Ipv4Addr::LOCALHOST, port).into();
    let Ok(mut c) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) else {
        return false;
    };
    let _ = c.set_read_timeout(Some(Duration::from_secs(2)));
    if writeln!(c, "{SHOW}").is_err() {
        return false;
    }
    let mut line = String::new();
    BufReader::new(c).read_line(&mut line).is_ok() && line.trim() == "ok"
}

/// Listens for later launches; `show` runs for each. The port file goes
/// when the returned guard drops.
pub fn listen(data: &Path, show: impl Fn() + Send + 'static) -> std::io::Result<Guard> {
    let ln = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let path = port_file(data);
    std::fs::write(&path, ln.local_addr()?.port().to_string())?;
    std::thread::Builder::new()
        .name("irori-desk-instance".into())
        .spawn(move || {
            for c in ln.incoming().flatten() {
                let _ = c.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                let mut r = BufReader::new(&c);
                if r.read_line(&mut line).is_ok() && line.trim() == SHOW {
                    show();
                    let _ = (&c).write_all(b"ok\n");
                }
            }
        })?;
    Ok(Guard(path))
}

/// Removes the port file.
pub struct Guard(PathBuf);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_second_launch_shows_the_first() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!hand_off(dir.path()), "nobody listens yet");
        let shown = Arc::new(AtomicUsize::new(0));
        let s = shown.clone();
        let guard = listen(dir.path(), move || {
            s.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        assert!(hand_off(dir.path()));
        assert!(hand_off(dir.path()));
        assert_eq!(shown.load(Ordering::SeqCst), 2);
        drop(guard);
        assert!(!port_file(dir.path()).exists());
        assert!(!hand_off(dir.path()));
    }
}
