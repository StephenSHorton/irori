//! `pair-rows.json` (I:internal/pair/saved.go): the person rows a pairing
//! handed this device (B), kept until they are stored. They can't be
//! inserted at once (a device's rows are gapless, and A's cert rows sit
//! among A's other rows), so B learns them (`Chain::learn`), which lasts
//! only for the process. Without the file, a B that stopped before its first
//! sync would hold a person with no certs and never start the net layer.
//! The rows are signed and public; no secret is in the file.

use std::path::Path;

use irori_identity::Chain;
use store::{Event, Store};

use crate::wire::WireRow;

/// The file's name in the data dir (Go `RowsFile`).
pub const ROWS_FILE: &str = "pair-rows.json";

pub(crate) fn save_rows(dir: Option<&Path>, rows: &[WireRow]) -> std::io::Result<()> {
    let Some(dir) = dir else { return Ok(()) };
    let raw = serde_json::to_vec(rows).map_err(std::io::Error::other)?;
    let path = dir.join(ROWS_FILE);
    let tmp = dir.join(format!("{ROWS_FILE}.tmp"));
    write_private(&tmp, &raw)?;
    std::fs::rename(&tmp, &path)
}

fn write_private(path: &Path, b: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(b)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, b)
    }
}

/// Go `LearnSaved`: folds the saved pairing rows the store doesn't hold yet
/// into `ch`. The Core calls it at start, once the store is open. When every
/// saved row is held, or the file is unreadable, the file is removed.
pub fn learn_saved(dir: &Path, st: &Store, ch: &Chain) -> Result<(), String> {
    let path = dir.join(ROWS_FILE);
    let raw = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    let Ok(rows) = serde_json::from_slice::<Vec<WireRow>>(&raw) else {
        // Unreadable: the sync will bring the rows.
        return std::fs::remove_file(&path).map_err(|e| e.to_string());
    };
    let vec = st.vector().map_err(|e| e.to_string())?;
    let left: Vec<Event> = rows
        .iter()
        .filter(|r| r.s > vec.get(&r.d).copied().unwrap_or(0))
        .map(WireRow::event)
        .collect();
    if left.is_empty() {
        return std::fs::remove_file(&path).map_err(|e| e.to_string());
    }
    learn_all(ch, &left);
    Ok(())
}

/// Go `learnAll`: folds rows that aren't stored yet into `ch`, in as many
/// passes as it takes for each cert's writer to be certified first.
pub(crate) fn learn_all(ch: &Chain, evs: &[Event]) {
    let mut left: Vec<&Event> = evs.iter().collect();
    while !left.is_empty() {
        let next: Vec<&Event> = left
            .iter()
            .copied()
            .filter(|ev| ch.learn(crate::wire::row_ref(ev)).is_err())
            .collect();
        if next.len() == left.len() {
            return;
        }
        left = next;
    }
}
