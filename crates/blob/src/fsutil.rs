//! File system helpers shared by [`crate::Store`] and [`crate::Files`].

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{Error, path_err};

/// Go `os.MkdirAll(dir, 0o700)`.
pub(crate) fn mkdir_all(dir: &Path) -> io::Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir)
}

/// Options for a new 0600 file.
pub(crate) fn private_new() -> OpenOptions {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut o = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
}

/// Go `os.CreateTemp(dir, prefix+"*")`: a new file with a random decimal
/// suffix, created exclusively.
pub(crate) fn create_temp(dir: &Path, prefix: &str) -> io::Result<(File, TempPath)> {
    for _ in 0..10_000 {
        let mut b = [0u8; 4];
        getrandom::fill(&mut b).map_err(|e| io::Error::other(format!("random: {e}")))?;
        let name = format!("{prefix}{}", u32::from_le_bytes(b));
        let p = dir.join(name);
        match private_new().write(true).create_new(true).open(&p) {
            Ok(f) => return Ok((f, TempPath(Some(p)))),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "pattern contains path separator",
    ))
}

/// A temp file removed on drop (Go's `defer os.Remove(tmpName)`): a no-op
/// once it was renamed, and it drops a losing temp.
pub(crate) struct TempPath(Option<PathBuf>);

impl TempPath {
    pub(crate) fn path(&self) -> &Path {
        self.0.as_deref().expect("temp path")
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = fs::remove_file(p);
        }
    }
}

/// Go `regular(path)`: os.Stat says a regular file.
pub(crate) fn regular(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// Go `place`: rename `tmp` onto `final_path`. Content-addressed, so a
/// final file that already exists (another goroutine or process won the
/// race, and on Windows may still hold it open) holds the same content and
/// counts as success. It is checked before every rename so a live blob is
/// never replaced. Other failures are retried for about a second.
pub fn place(tmp: &Path, final_path: &Path) -> io::Result<()> {
    let mut wait = Duration::from_millis(5);
    loop {
        if regular(final_path) {
            return Ok(());
        }
        match fs::rename(tmp, final_path) {
            Ok(()) => return Ok(()),
            Err(_) if regular(final_path) => return Ok(()),
            Err(e) => {
                if wait >= Duration::from_secs(1) {
                    return Err(e);
                }
            }
        }
        std::thread::sleep(wait);
        wait *= 2;
    }
}

/// Go `transientOpen`: an open that failed only because another process is
/// placing or replacing the file (Windows sharing violation or access
/// denied); never off Windows.
fn transient_open(e: &io::Error) -> bool {
    cfg!(windows) && matches!(e.raw_os_error(), Some(5) | Some(32))
}

/// Go `openRetry`: open for reading, retrying briefly (1 ms doubling to
/// 500 ms) while another writer's rename holds the file.
pub(crate) fn open_retry(path: &Path) -> io::Result<File> {
    let mut wait = Duration::from_millis(1);
    loop {
        match File::open(path) {
            Err(e) if transient_open(&e) && wait < Duration::from_millis(500) => {
                std::thread::sleep(wait);
                wait *= 2;
            }
            r => return r,
        }
    }
}

/// Open a stored file for reading: missing or not a regular file is
/// NotFound (Go opens a directory and then sees it is not regular; Windows
/// refuses to open one, so the check comes first here).
pub(crate) fn open_stored(path: &Path) -> crate::Result<File> {
    match fs::metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Error::not_found()),
        Ok(m) if !m.is_file() => return Err(Error::not_found()),
        _ => {}
    }
    let f = match open_retry(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Error::not_found()),
        Err(e) => return Err(path_err("open", &path.to_string_lossy(), &e)),
    };
    match f.metadata() {
        Ok(m) if m.is_file() => Ok(f),
        Ok(_) => Err(Error::not_found()),
        Err(e) => Err(path_err("stat", &path.to_string_lossy(), &e)),
    }
}

fn is_sep(b: u8) -> bool {
    b == b'/' || (cfg!(windows) && b == b'\\')
}

/// Go `filepath.Base`.
pub(crate) fn go_base(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let mut p = path;
    while !p.is_empty() && is_sep(p.as_bytes()[p.len() - 1]) {
        p = &p[..p.len() - 1];
    }
    let vol = gocompat::filepath::volume_name(p).len();
    p = &p[vol..];
    if let Some(i) = p.bytes().rposition(is_sep) {
        p = &p[i + 1..];
    }
    if p.is_empty() {
        return gocompat::filepath::SEPARATOR.to_string();
    }
    p.to_string()
}

/// Go `filepath.Ext`.
pub(crate) fn go_ext(path: &str) -> &str {
    let b = path.as_bytes();
    let mut i = b.len();
    while i > 0 && !is_sep(b[i - 1]) {
        i -= 1;
        if b[i] == b'.' {
            return &path[i..];
        }
    }
    ""
}
