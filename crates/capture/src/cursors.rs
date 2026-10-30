//! I:internal/capture/cursors.go: `<DataDir>/capture-cursors.json`, the
//! persisted byte offsets (capture.md §0 I3). An entry's presence means its
//! file or reflog has been seen, so backfill and "start at the end" rules
//! run once. Local bookkeeping: never in the log.

use std::collections::BTreeMap;
use std::io::Write;

use gocompat::json::{self, Out, Schema};
use serde::Serialize;

/// The file name inside the data dir.
pub const CURSOR_FILE_NAME: &str = "capture-cursors.json";

/// The persisted offsets. Keys: `files` = transcript paths as listed (native
/// separators, on-disk case); `reflogs` = normRoot of the worktree root.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CursorFile {
    pub files: BTreeMap<String, i64>,
    pub reflogs: BTreeMap<String, i64>,
    #[serde(skip)]
    pub dirty: bool,
}

fn schema() -> Schema {
    Schema::object(vec![
        ("files", Schema::Map(Box::new(Schema::Int(64)))),
        ("reflogs", Schema::Map(Box::new(Schema::Int(64)))),
    ])
}

fn int_map(o: Option<&Out>) -> BTreeMap<String, i64> {
    match o {
        Some(Out::Map(Some(m))) => m
            .iter()
            .map(|(k, v)| (k.clone(), crate::jsonx::out_int(Some(v))))
            .collect(),
        _ => BTreeMap::new(),
    }
}

impl CursorFile {
    /// Reads the file. A missing file is empty; a damaged one is empty too
    /// (re-reading is safe: row ids dedupe). Other read errors are returned
    /// with an empty file.
    pub fn load(data_dir: &str) -> (CursorFile, Option<std::io::Error>) {
        let path = crate::gopath::join(&[data_dir, CURSOR_FILE_NAME]);
        let b = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (CursorFile::default(), None);
            }
            Err(e) => return (CursorFile::default(), Some(e)),
        };
        match json::unmarshal(&b, &schema()) {
            Ok(o) => (
                CursorFile {
                    files: int_map(o.get("files")),
                    reflogs: int_map(o.get("reflogs")),
                    dirty: false,
                },
                None,
            ),
            Err(_) => (CursorFile::default(), None),
        }
    }

    pub fn set_file(&mut self, path: &str, off: i64) {
        if self.files.get(path) != Some(&off) {
            self.files.insert(path.to_string(), off);
            self.dirty = true;
        }
    }

    pub fn set_reflog(&mut self, root: &str, off: i64) {
        if self.reflogs.get(root) != Some(&off) {
            self.reflogs.insert(root.to_string(), off);
            self.dirty = true;
        }
    }

    /// Go's `json.Marshal(cf)`.
    pub fn bytes(&self) -> Vec<u8> {
        json::marshal(self).expect("cursor file marshals")
    }

    /// Writes the file atomically (temp file `capture-cursors.json.*.tmp`,
    /// fsync, rename) when it changed.
    pub fn save(&mut self, data_dir: &str) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let b = self.bytes();
        let mut tmp = tempfile::Builder::new()
            .prefix(&format!("{CURSOR_FILE_NAME}."))
            .suffix(".tmp")
            .tempfile_in(data_dir)?;
        tmp.write_all(&b)?;
        tmp.as_file().sync_all()?;
        tmp.persist(crate::gopath::join(&[data_dir, CURSOR_FILE_NAME]))
            .map_err(|e| e.error)?;
        self.dirty = false;
        Ok(())
    }
}
