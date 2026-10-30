//! Go `blob.Store` (`store.go`): patches and summaries, gzip at
//! `<dir>/<id[:2]>/<id>`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};

use crate::error::{Error, ErrorKind, Result, os_text};
use crate::fsutil::{create_temp, mkdir_all, open_stored, place, regular};

/// The largest blob kept. Put truncates longer data to MAX_SIZE bytes (cut
/// back to a UTF-8 boundary) and appends [`TRUNCATED_MARKER`].
pub const MAX_SIZE: usize = 4 << 20;

/// Ends a truncated blob.
pub const TRUNCATED_MARKER: &str = "\n[irori: truncated]\n";

/// Go `ValidID`: 64 lowercase hex characters.
pub fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn rune_start(b: u8) -> bool {
    b & 0xC0 != 0x80
}

/// Go `truncate`: data longer than MAX_SIZE is cut back over at most three
/// continuation bytes (binary data with longer runs is cut at MAX_SIZE as
/// is) and gets the marker.
pub fn truncate(data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if data.len() <= MAX_SIZE {
        return data.into();
    }
    let mut cut = MAX_SIZE;
    for _ in 0..3 {
        if cut == 0 || rune_start(data[cut]) {
            break;
        }
        cut -= 1;
    }
    if !rune_start(data[cut]) {
        cut = MAX_SIZE;
    }
    let mut out = Vec::with_capacity(cut + TRUNCATED_MARKER.len());
    out.extend_from_slice(&data[..cut]);
    out.extend_from_slice(TRUNCATED_MARKER.as_bytes());
    out.into()
}

pub(crate) fn sha_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// The content-addressed blob store. Safe for concurrent use, including two
/// processes putting the same blob at once.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// Go `Open`: creates `dir` (0700) if needed.
    pub fn open(dir: impl AsRef<Path>) -> Result<Store> {
        let dir = dir.as_ref();
        if dir.as_os_str().is_empty() {
            return Err(Error::other("blob: empty dir"));
        }
        mkdir_all(dir).map_err(|e| {
            Error::other(format!("mkdir {}: {}", dir.to_string_lossy(), os_text(&e)))
        })?;
        Ok(Store { dir: dir.into() })
    }

    /// The directory the blobs live in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(&id[..2]).join(id)
    }

    /// Go `Put`: stores data and returns its id. Putting the same content
    /// again is a no-op returning the same id.
    pub fn put(&self, data: &[u8]) -> Result<String> {
        let data = truncate(data);
        let id = sha_hex(&data);
        if self.has(&id) {
            return Ok(id);
        }
        let sub = self.dir.join(&id[..2]);
        let io = |op: &str, e: std::io::Error| Error::other(format!("{op}: {}", os_text(&e)));
        mkdir_all(&sub).map_err(|e| {
            Error::other(format!("mkdir {}: {}", sub.to_string_lossy(), os_text(&e)))
        })?;
        let (f, tmp) = create_temp(&sub, &format!("{id}.tmp-")).map_err(|e| {
            Error::other(format!(
                "open {}: {}",
                sub.join(format!("{id}.tmp-*")).to_string_lossy(),
                os_text(&e)
            ))
        })?;
        let mut zw = GzEncoder::new(f, Compression::default());
        zw.write_all(&data).map_err(|e| io("write", e))?;
        let f = zw.finish().map_err(|e| io("write", e))?;
        f.sync_all().map_err(|e| io("sync", e))?;
        // Closed before the rename: Windows refuses to rename an open file.
        drop(f);
        place(tmp.path(), &self.path(&id)).map_err(|e| {
            Error::other(format!(
                "rename {} {}: {}",
                tmp.path().to_string_lossy(),
                self.path(&id).to_string_lossy(),
                os_text(&e)
            ))
        })?;
        Ok(id)
    }

    /// Go `Get`: a blob's content; BadId for a malformed id (never touching
    /// the file system with it), NotFound when absent, Corrupt when it does
    /// not decode or its hash differs.
    pub fn get(&self, id: &str) -> Result<Vec<u8>> {
        if !valid_id(id) {
            return Err(Error::bad_id());
        }
        let f = open_stored(&self.path(id))?;
        let corrupt =
            |why: String| Error::new(ErrorKind::Corrupt, format!("blob: {id} is corrupt: {why}"));
        // Nothing Put writes is larger than this; a bigger stream is corrupt.
        let limit = (MAX_SIZE + TRUNCATED_MARKER.len()) as u64;
        let mut buf = Vec::new();
        let n = MultiGzDecoder::new(f)
            .take(limit + 1)
            .read_to_end(&mut buf)
            .map_err(|e| corrupt(e.to_string()))? as u64;
        if n > limit {
            return Err(corrupt(format!("larger than {limit} bytes")));
        }
        if sha_hex(&buf) != id {
            return Err(corrupt("content hash differs".into()));
        }
        Ok(buf)
    }

    /// Go `Has`: whether a blob is stored (a regular file at its path).
    pub fn has(&self, id: &str) -> bool {
        valid_id(id) && regular(&self.path(id))
    }

    /// Go's unexported `place`, for the race tests.
    #[doc(hidden)]
    pub fn place(&self, tmp: &Path, final_path: &Path) -> std::io::Result<()> {
        place(tmp, final_path)
    }
}
