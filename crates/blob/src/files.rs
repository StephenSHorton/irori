//! Go `blob.Files` (`files.go`): the attachment store.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::attachpath::safe_name;
use crate::error::{self, Error, ErrorKind, Result, os_text, path_err};
use crate::fsutil::{create_temp, go_base, mkdir_all, open_stored, place, private_new, regular};
use crate::mime::mime_of;
use crate::store::valid_id;

/// The default cap on one attachment between the person's own devices
/// (decision D6): 256 MiB.
pub const MAX_FILE_SIZE: u64 = 256 << 20;

/// The list of attachments the person let go here (card #728), one id a
/// line, in the store's folder.
const DROPPED: &str = "dropped";

/// Serializes changes to the dropped list: the Core and the mesh node each
/// open their own [`Files`] on the same folder, and fetches finish on many
/// threads.
static DROPPED_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// An attachment as a message or a comment carries it. `device` is where
/// it was attached; `name` comes from that device and is only a hint until
/// [`safe_name`] has reduced it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileRef {
    #[serde(rename = "sha256")]
    pub sha256: String,
    pub name: String,
    pub size: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mime: String,
    pub device: String,
}

impl FileRef {
    /// Go `Describe`: one line for a model or a person: the safe name, the
    /// size, the type, the hash and the attaching device.
    pub fn describe(&self) -> String {
        let mut b = format!(
            "{} ({}",
            safe_name(&self.name, &self.sha256),
            human_size(self.size)
        );
        // Mime and Device come from another device's row: shown only when
        // plain.
        if !self.mime.is_empty() && self.mime.len() <= 100 && plain(&self.mime, "/+.-;= ") {
            b.push_str(", ");
            b.push_str(&self.mime);
        }
        b.push_str(") sha256=");
        b.push_str(&self.sha256);
        if !self.device.is_empty() && plain(&self.device, "-_") {
            // plain() admits ASCII only, so byte 8 is a char boundary.
            let dev = if self.device.len() > 8 {
                &self.device[..8]
            } else {
                &self.device
            };
            b.push_str(" from device ");
            b.push_str(dev);
        }
        b
    }
}

fn human_size(n: i64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MiB", n as f64 / (1u64 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.1} KiB", n as f64 / 1024.0)
    } else {
        format!("{n} bytes")
    }
}

/// ASCII letters, digits and `extra` only.
fn plain(s: &str, extra: &str) -> bool {
    s.chars()
        .all(|r| r.is_ascii_alphanumeric() || extra.contains(r))
}

/// The attachment store: `<dir>/<id[:2]>/<id>`, stored as is, written once
/// and never changed. Partial fetches wait in `<dir>/partial/<id>` until
/// their length and hash check out.
#[derive(Debug, Clone)]
pub struct Files {
    dir: PathBuf,
    max: u64,
}

impl Files {
    /// Go `OpenFiles`: creates `dir` and `dir/partial` (0700) if needed,
    /// with the default cap.
    pub fn open(dir: impl AsRef<Path>) -> Result<Files> {
        Files::open_with_max(dir, MAX_FILE_SIZE)
    }

    /// [`Files::open`] with another size cap (Go tests lower the
    /// `MaxFileSize` var).
    pub fn open_with_max(dir: impl AsRef<Path>, max: u64) -> Result<Files> {
        let dir = dir.as_ref();
        if dir.as_os_str().is_empty() {
            return Err(Error::other("blob: empty dir"));
        }
        let partial = dir.join("partial");
        mkdir_all(&partial).map_err(|e| {
            Error::other(format!(
                "mkdir {}: {}",
                partial.to_string_lossy(),
                os_text(&e)
            ))
        })?;
        Ok(Files {
            dir: dir.into(),
            max,
        })
    }

    /// The size cap this store keeps.
    pub fn max(&self) -> u64 {
        self.max
    }

    /// A stored attachment's path (for tests and serving).
    pub fn path(&self, id: &str) -> PathBuf {
        self.dir.join(&id[..2.min(id.len())]).join(id)
    }

    fn partial_path(&self, id: &str) -> PathBuf {
        self.dir.join("partial").join(id)
    }

    /// Go `Has`.
    pub fn has(&self, id: &str) -> bool {
        valid_id(id) && regular(&self.path(id))
    }

    /// Go `Size`: a stored attachment's size.
    pub fn size(&self, id: &str) -> Result<u64> {
        if !valid_id(id) {
            return Err(Error::bad_id());
        }
        let p = self.path(id);
        match fs::metadata(&p) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(Error::not_found()),
            Ok(m) if !m.is_file() => Err(Error::not_found()),
            Err(e) => Err(path_err(
                crate::attachpath::stat_op(),
                &p.to_string_lossy(),
                &e,
            )),
            Ok(m) => Ok(m.len()),
        }
    }

    /// Go `Open`: a stored attachment opened for reading.
    pub fn open_file(&self, id: &str) -> Result<File> {
        if !valid_id(id) {
            return Err(Error::bad_id());
        }
        open_stored(&self.path(id))
    }

    /// Go `PutFile`: copies the regular file at `path` into the store and
    /// returns its ref (device unset). Larger than the cap is TooLarge,
    /// before and while copying.
    pub fn put_file(&self, path: &str) -> Result<FileRef> {
        let mut src = File::open(path).map_err(|e| path_err("open", path, &e))?;
        let meta = src.metadata().map_err(|e| path_err("stat", path, &e))?;
        if !meta.is_file() {
            return Err(Error::new(ErrorKind::NotFile, error::NOT_FILE));
        }
        if meta.len() > self.max {
            return Err(Error::new(
                ErrorKind::TooLarge,
                format!(
                    "{} ({} bytes, at most {})",
                    error::TOO_LARGE,
                    meta.len(),
                    self.max
                ),
            ));
        }
        let partial = self.dir.join("partial");
        let (mut tmp, guard) = create_temp(&partial, "put-")
            .map_err(|e| path_err("open", &partial.join("put-*").to_string_lossy(), &e))?;
        let mut h = Sha256::new();
        let mut head: Vec<u8> = Vec::with_capacity(512);
        let mut n: u64 = 0;
        let mut buf = vec![0u8; 64 << 10];
        let mut limited = (&mut src).take(self.max + 1);
        let io = |e: io::Error| Error::other(os_text(&e));
        loop {
            let k = match limited.read(&mut buf) {
                Ok(0) => break,
                Ok(k) => k,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(io(e)),
            };
            tmp.write_all(&buf[..k]).map_err(io)?;
            h.update(&buf[..k]);
            if head.len() < 512 {
                let room = 512 - head.len();
                head.extend_from_slice(&buf[..k.min(room)]);
            }
            n += k as u64;
        }
        if n > self.max {
            return Err(Error::new(
                ErrorKind::TooLarge,
                format!("{} (at most {} bytes)", error::TOO_LARGE, self.max),
            ));
        }
        tmp.sync_all().map_err(io)?;
        drop(tmp);
        let id = hex::encode(h.finalize());
        self.place_file(guard.path(), &id)?;
        Ok(FileRef {
            sha256: id,
            name: go_base(path),
            size: n as i64,
            mime: mime_of(path, &head),
            device: String::new(),
        })
    }

    fn place_file(&self, tmp: &Path, id: &str) -> Result<()> {
        let sub = self.dir.join(&id[..2]);
        mkdir_all(&sub).map_err(|e| {
            Error::other(format!("mkdir {}: {}", sub.to_string_lossy(), os_text(&e)))
        })?;
        let fin = self.path(id);
        place(tmp, &fin).map_err(|e| {
            Error::other(format!(
                "rename {} {}: {}",
                tmp.to_string_lossy(),
                fin.to_string_lossy(),
                os_text(&e)
            ))
        })?;
        // Here again: the person fetched or attached it (card #728).
        let _ = self.mark_dropped(id, false);
        Ok(())
    }

    /// Go `Partial`: the partial file of a fetch of `id`, opened for
    /// appending, and how many bytes it already holds, so a fetch resumes
    /// by offset.
    pub fn partial(&self, id: &str) -> Result<(File, u64)> {
        if !valid_id(id) {
            return Err(Error::bad_id());
        }
        let p = self.partial_path(id);
        let mut o: OpenOptions = private_new();
        let mut f = o
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&p)
            .map_err(|e| path_err("open", &p.to_string_lossy(), &e))?;
        let off = f
            .seek(SeekFrom::End(0))
            .map_err(|e| path_err("seek", &p.to_string_lossy(), &e))?;
        Ok((f, off))
    }

    /// Go `Finish`: checks a fetch's partial file against the ref's length
    /// and hash and moves it into the store. A short file is kept to resume
    /// (Mismatch "n of size bytes"); a wrong or overlong one is dropped.
    pub fn finish(&self, id: &str, size: u64) -> Result<()> {
        let p = self.partial_path(id);
        let meta = fs::metadata(&p)
            .map_err(|e| path_err(crate::attachpath::stat_op(), &p.to_string_lossy(), &e))?;
        if meta.len() < size {
            return Err(Error::new(
                ErrorKind::Mismatch,
                format!("{}: {} of {} bytes", error::MISMATCH, meta.len(), size),
            ));
        }
        let mut f = File::open(&p).map_err(|e| path_err("open", &p.to_string_lossy(), &e))?;
        let sum = hash_reader(&mut f).map_err(|e| Error::other(os_text(&e)))?;
        drop(f);
        if meta.len() != size || sum != id {
            let _ = fs::remove_file(&p);
            return Err(Error::new(ErrorKind::Mismatch, error::MISMATCH));
        }
        self.place_file(&p, id)
    }

    /// Every stored attachment and its size, by id (the Storage page, card
    /// #728). Partial fetches and stray names are left out.
    pub fn list(&self) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        let Ok(top) = fs::read_dir(&self.dir) else {
            return out;
        };
        for sub in top.flatten() {
            let name = sub.file_name().to_string_lossy().into_owned();
            if name.len() != 2 || !sub.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let Ok(rd) = fs::read_dir(sub.path()) else {
                continue;
            };
            for e in rd.flatten() {
                let id = e.file_name().to_string_lossy().into_owned();
                if !valid_id(&id) || !id.starts_with(&name) {
                    continue;
                }
                if let Ok(m) = e.metadata()
                    && m.is_file()
                {
                    out.push((id, m.len()));
                }
            }
        }
        out.sort();
        out
    }

    /// Deletes a stored attachment here (the person, from the Storage page,
    /// card #728) and returns its size. It is marked as let go, so the
    /// small-file pick-up (card #580) doesn't bring it back unasked; a
    /// fetch the person asks for, or attaching the same file again, clears
    /// the mark. Not there is NotFound.
    pub fn remove(&self, id: &str) -> Result<u64> {
        let n = self.size(id)?;
        self.mark_dropped(id, true)?;
        let p = self.path(id);
        if let Err(e) = fs::remove_file(&p) {
            let _ = self.mark_dropped(id, false);
            return Err(path_err("remove", &p.to_string_lossy(), &e));
        }
        Ok(n)
    }

    /// Whether the person let `id` go here ([`Files::remove`]) and hasn't
    /// fetched or attached it since.
    pub fn dropped(&self, id: &str) -> bool {
        self.dropped_ids().iter().any(|d| d == id)
    }

    fn dropped_ids(&self) -> Vec<String> {
        fs::read_to_string(self.dir.join(DROPPED))
            .map(|s| {
                s.lines()
                    .map(str::trim)
                    .filter(|l| valid_id(l))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn mark_dropped(&self, id: &str, on: bool) -> Result<()> {
        let _held = DROPPED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut ids = self.dropped_ids();
        let had = ids.iter().any(|d| d == id);
        if had == on {
            return Ok(());
        }
        if on {
            ids.push(id.to_string());
        } else {
            ids.retain(|d| d != id);
        }
        let p = self.dir.join(DROPPED);
        let tmp = self.dir.join("partial").join(format!("{DROPPED}.tmp"));
        let mut text = ids.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        fs::write(&tmp, text).map_err(|e| path_err("write", &tmp.to_string_lossy(), &e))?;
        fs::rename(&tmp, &p).map_err(|e| path_err("rename", &p.to_string_lossy(), &e))
    }

    /// Go `DropPartial`: removes a fetch's partial file.
    pub fn drop_partial(&self, id: &str) {
        if valid_id(id) {
            let _ = fs::remove_file(self.partial_path(id));
        }
    }

    /// Go `CopyOut`: writes a stored attachment to `dst`, which must not
    /// exist yet (a row never overwrites a file). The copy is checked
    /// against `id` and removed if it fails.
    pub fn copy_out(&self, id: &str, dst: &Path) -> Result<()> {
        let mut src = self.open_file(id)?;
        if let Some(parent) = dst.parent()
            && !parent.as_os_str().is_empty()
        {
            mkdir_all(parent).map_err(|e| {
                Error::other(format!(
                    "mkdir {}: {}",
                    parent.to_string_lossy(),
                    os_text(&e)
                ))
            })?;
        }
        let mut out = private_new()
            .write(true)
            .create_new(true)
            .open(dst)
            .map_err(|e| path_err("open", &dst.to_string_lossy(), &e))?;
        let mut h = Sha256::new();
        let r = copy_hashing(&mut src, &mut out, &mut h);
        drop(out);
        let r = r.and_then(|()| {
            if hex::encode(h.finalize()) != id {
                Err(Error::new(ErrorKind::Mismatch, error::MISMATCH))
            } else {
                Ok(())
            }
        });
        if r.is_err() {
            let _ = fs::remove_file(dst);
        }
        r
    }
}

fn copy_hashing(src: &mut File, out: &mut File, h: &mut Sha256) -> Result<()> {
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let k = match src.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(k) => k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Error::other(os_text(&e))),
        };
        out.write_all(&buf[..k])
            .map_err(|e| Error::other(os_text(&e)))?;
        h.update(&buf[..k]);
    }
}

/// Go `SameContent`: whether the file at `path` already holds `id`'s bytes.
pub fn same_content(path: &Path, id: &str) -> bool {
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    hash_reader(&mut f).is_ok_and(|sum| sum == id)
}

/// The SHA-256 hex of everything `r` yields.
fn hash_reader(r: &mut impl Read) -> io::Result<String> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        match r.read(&mut buf) {
            Ok(0) => return Ok(hex::encode(h.finalize())),
            Ok(k) => h.update(&buf[..k]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}
