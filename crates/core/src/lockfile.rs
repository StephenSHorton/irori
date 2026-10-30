//! `irori.json`, the discovery lockfile (I:internal/discovery; surface.md
//! §2.2, §7.13): how agents and the other roles find the running Core. It
//! holds the loopback port and the bearer token; no sockets, no fixed port.
//!
//! Written atomically (a `irori.json.*.tmp` beside it, fsync, rename with a
//! short retry), 0600, as `json.MarshalIndent(lf, "", "  ")` with no
//! trailing newline. The token is a secret: nothing here logs or prints it,
//! and [`Lockfile`]'s `Debug` leaves it out.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gocompat::json::lenient::{self, Out, Schema};
use gocompat::time::GoTime;
use serde::Serialize;

/// The lockfile's name inside the data dir.
pub const FILE_NAME: &str = "irori.json";

/// `irori.json`. Field order is Go's.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct Lockfile {
    pub pid: i64,
    pub port: i64,
    /// 32 random bytes, hex. A secret.
    pub token: String,
    pub version: String,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub headless: bool,
    #[serde(rename = "startedAt")]
    pub started_at: GoTime,
}

impl Default for Lockfile {
    fn default() -> Self {
        Lockfile {
            pid: 0,
            port: 0,
            token: String::new(),
            version: String::new(),
            device_id: String::new(),
            headless: false,
            started_at: GoTime::ZERO,
        }
    }
}

impl std::fmt::Debug for Lockfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lockfile")
            .field("pid", &self.pid)
            .field("port", &self.port)
            .field("version", &self.version)
            .field("device_id", &self.device_id)
            .field("headless", &self.headless)
            .field("started_at", &self.started_at.format_rfc3339_nano())
            .finish_non_exhaustive()
    }
}

/// Why [`read`] found no running Core, or could not tell.
#[derive(Debug)]
pub enum Error {
    /// Go `ErrNotRunning`: no lockfile, or its process is gone.
    NotRunning,
    /// The file is there but does not parse (not `NotRunning`).
    Unreadable(String),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotRunning => f.write_str("irori is not running"),
            Error::Unreadable(s) => f.write_str(s),
            Error::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// A fresh token: 32 bytes from the OS RNG, lowercase hex.
pub fn new_token() -> io::Result<String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(hex::encode(b))
}

/// Writes `dir/irori.json` atomically, creating `dir` (0700).
pub fn write(dir: &Path, lf: &Lockfile) -> io::Result<()> {
    create_dir_all_private(dir)?;
    let out = gocompat::json::marshal_indent(lf, "", "  ")
        .map_err(|e| io::Error::other(e.to_string()))?;
    let (tmp, mut f) = create_temp(dir)?;
    let guard = RemoveOnDrop(tmp.clone());
    let res = (|| {
        set_private(&f)?;
        f.write_all(&out)?;
        f.sync_all()
    })();
    drop(f);
    res?;
    rename_retry(&tmp, &dir.join(FILE_NAME))?;
    std::mem::forget(guard);
    Ok(())
}

/// Reads `dir/irori.json`: [`Error::NotRunning`] when it is missing or its
/// pid no longer runs.
pub fn read(dir: &Path) -> Result<Lockfile, Error> {
    let lf = load(dir)?;
    if !sys::proc::alive(lf.pid) {
        return Err(Error::NotRunning);
    }
    Ok(lf)
}

/// Deletes `dir/irori.json` only while it still names `pid`, so an older
/// instance exiting never removes a newer one's.
pub fn remove(dir: &Path, pid: i64) -> Result<(), Error> {
    let lf = match load(dir) {
        Ok(lf) => lf,
        Err(Error::NotRunning) => return Ok(()),
        Err(e) => return Err(e),
    };
    if lf.pid != pid {
        return Ok(());
    }
    match fs::remove_file(dir.join(FILE_NAME)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// This process's pid as the lockfile writes it.
pub fn own_pid() -> i64 {
    i64::from(std::process::id())
}

/// The file without looking at its pid. The open is kept short: Windows
/// refuses the writer's rename while a reader holds the file.
fn load(dir: &Path) -> Result<Lockfile, Error> {
    let path = dir.join(FILE_NAME);
    let raw = match fs::read(&path) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Error::NotRunning),
        Err(e) => return Err(e.into()),
    };
    let schema = Schema::object(vec![
        ("pid", Schema::Int(64)),
        ("port", Schema::Int(64)),
        ("token", Schema::String),
        ("version", Schema::String),
        ("deviceId", Schema::String),
        ("headless", Schema::Bool),
        ("startedAt", Schema::Time),
    ]);
    let v = lenient::unmarshal(&raw, &schema).map_err(|e| {
        Error::Unreadable(format!("discovery: {} is unreadable: {e}", path.display()))
    })?;
    let int = |n: &str| match v.get(n) {
        Some(Out::Int(i)) => *i,
        _ => 0,
    };
    Ok(Lockfile {
        pid: int("pid"),
        port: int("port"),
        token: v.get_str("token").to_string(),
        version: v.get_str("version").to_string(),
        device_id: v.get_str("deviceId").to_string(),
        headless: matches!(v.get("headless"), Some(Out::Bool(true))),
        started_at: match v.get("startedAt") {
            Some(Out::Time(t)) => *t,
            _ => GoTime::ZERO,
        },
    })
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Go `os.CreateTemp(dir, "irori.json.*.tmp")`.
fn create_temp(dir: &Path) -> io::Result<(PathBuf, fs::File)> {
    for _ in 0..10_000 {
        let mut b = [0u8; 4];
        getrandom::fill(&mut b).map_err(|e| io::Error::other(e.to_string()))?;
        let n = u32::from_le_bytes(b);
        let p = dir.join(format!("{FILE_NAME}.{n}.tmp"));
        match fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(f) => return Ok((p, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("discovery: no free temp name"))
}

/// Retries a refused rename for up to a second: on Windows it fails while a
/// reader has the target open without delete sharing.
fn rename_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut wait = Duration::from_millis(5);
    while wait < Duration::from_secs(1) {
        if fs::rename(from, to).is_ok() {
            return Ok(());
        }
        std::thread::sleep(wait);
        wait *= 2;
    }
    fs::rename(from, to)
}

/// `os.MkdirAll(dir, 0o700)`.
pub(crate) fn create_dir_all_private(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

#[cfg(unix)]
fn set_private(f: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    f.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private(_: &fs::File) -> io::Result<()> {
    // Windows honours only the read-only bit; the data dir is per-user.
    Ok(())
}
