//! `public-relays.json`: whether this device meets other devices through
//! n0's public iroh relays (card #328, docs/design/public-relays.md).
//! Absent or unreadable means on; only an explicit `{"on": false}` is off.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "public-relays.json";

#[derive(Debug, Serialize, Deserialize)]
struct Saved {
    on: bool,
}

/// The saved switch; on unless saved off.
pub fn load(dir: &Path) -> bool {
    std::fs::read(dir.join(FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
        .is_none_or(|s| s.on)
}

pub fn save(dir: &Path, on: bool) -> io::Result<()> {
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&Saved { on })?)?;
    std::fs::rename(tmp, dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_is_on_and_off_sticks() {
        let d = tempfile::tempdir().unwrap();
        assert!(load(d.path()));
        save(d.path(), false).unwrap();
        assert!(!load(d.path()));
        save(d.path(), true).unwrap();
        assert!(load(d.path()));
        std::fs::write(d.path().join(FILE), b"junk").unwrap();
        assert!(load(d.path()));
    }
}
