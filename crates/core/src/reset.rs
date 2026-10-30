//! Reset (I:internal/core/reset.go, surface.md §2.1): this device becomes a
//! new one. Its log, `device.json` and `device.key` move into
//! `reset-<UTC time>-<random>/` in the data dir, so the next start has a new
//! device id and key and its own first-run person. Nothing is deleted, and
//! the settings (`settings.json`, `relay.json`, `irori.json`) stay.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gocompat::time::GoTime;

use crate::lockfile;

/// Marks a Reset asked for while Irori runs; the next start does it before
/// it opens anything.
pub const RESET_FILE: &str = "reset-pending";

/// The data dir entries a Reset moves aside.
pub const RESET_MOVES: [&str; 12] = [
    "irori.db",
    "irori.db-wal",
    "irori.db-shm",
    "irori.db.v0.3",
    "device.json",
    irori_identity::person::DEVICE_KEY_FILE,
    irori_identity::person::PERSON_FILE,
    "mesh-peers.json",
    "pair-rows.json",
    "capture-cursors.json",
    "files",
    "blobs",
];

/// Writes the marker (the GUI's Reset button; the next start finishes it):
/// RFC 3339 UTC and a newline, 0600.
pub fn request_reset(data_dir: &Path) -> io::Result<()> {
    let now = GoTime::from_system(std::time::SystemTime::now()).utc();
    let body = format!("{}\n", now.format_rfc3339());
    write_private(&data_dir.join(RESET_FILE), body.as_bytes())
}

/// Moves this device's log and identity aside now (`irori pair reset`). It
/// refuses while a Core runs on `data_dir`, this process's own included,
/// and returns the folder the files went to.
pub fn reset(data_dir: &Path) -> Result<PathBuf, String> {
    if let Ok(lf) = lockfile::read(data_dir) {
        return Err(format!(
            "{} (pid {}): quit it first, or use Reset in Settings → Devices",
            crate::ALREADY_RUNNING,
            lf.pid
        ));
    }
    reset_now(data_dir).map_err(|e| e.to_string())
}

pub(crate) fn reset_now(data_dir: &Path) -> io::Result<PathBuf> {
    let stamp = GoTime::from_system(std::time::SystemTime::now()).utc();
    let (y, mo, d, h, mi, s) = stamp.civil();
    let prefix = format!("reset-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}-");
    let dst = mkdir_temp(data_dir, &prefix)?;
    for name in RESET_MOVES {
        let src = data_dir.join(name);
        match fs::symlink_metadata(&src) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            _ => {}
        }
        fs::rename(&src, dst.join(name))
            .map_err(|e| io::Error::new(e.kind(), format!("reset: move {name}: {e}")))?;
    }
    match fs::remove_file(data_dir.join(RESET_FILE)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    Ok(dst)
}

/// Runs a Reset that [`request_reset`] marked; `Ok(None)` when none was.
pub(crate) fn reset_if_asked(data_dir: &Path) -> io::Result<Option<PathBuf>> {
    if fs::metadata(data_dir.join(RESET_FILE)).is_err() {
        return Ok(None);
    }
    reset_now(data_dir).map(Some)
}

/// Go `os.MkdirTemp(dir, prefix)`: prefix plus a random decimal.
fn mkdir_temp(dir: &Path, prefix: &str) -> io::Result<PathBuf> {
    for _ in 0..10_000 {
        let mut b = [0u8; 4];
        getrandom::fill(&mut b).map_err(|e| io::Error::other(e.to_string()))?;
        let p = dir.join(format!("{prefix}{}", u32::from_le_bytes(b)));
        match fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("reset: no free folder name"))
}

pub(crate) fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    use std::io::Write as _;
    o.open(path)?.write_all(data)
}
