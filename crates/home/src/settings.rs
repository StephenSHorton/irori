//! `home-server.json`, the home server's device-local settings
//! (I:internal/home/home.go:41-108, mesh.md §2.5).
//!
//! `outside` and `awake` are tri-state: absent means on, and a value that
//! was absent is never written back as `false` (mesh.md §7 trap 4).

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The file name in the data dir.
pub const FILE: &str = "home-server.json";

/// The TURN relay's sealed HMAC key under Go. Unused under iroh, never read
/// here, and left in place (mesh.md §2.5).
pub const KEY_FILE: &str = "turn.key";

/// The port N is picked from this range on first start: below the
/// ephemeral ranges of Windows and macOS, and not one fixed number, so a
/// scan for Irori's default port finds nothing (home.go:48-54).
pub const PORT_MIN: u16 = 20000;
pub const PORT_MAX: u16 = 29999;

/// `home-server.json` (Go `home.Settings`), fields in Go's order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub on: bool,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub port: i64,
    /// "Reachable from outside home" (port mapping); `None` is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outside: Option<bool>,
    /// An address the person entered (an IP or a host name, optionally with
    /// a port) for when mapping fails.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub public: String,
    /// The router ports last granted, asked for again so a mapping that
    /// moved keeps its number.
    #[serde(rename = "tcpExternal", default, skip_serializing_if = "is_zero_u16")]
    pub tcp_external: u16,
    #[serde(rename = "udpExternal", default, skip_serializing_if = "is_zero_u16")]
    pub udp_external: u16,
    /// "Keep this PC awake while it's the server"; `None` is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awake: Option<bool>,
}

fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}

fn is_zero_u16(n: &u16) -> bool {
    *n == 0
}

impl Settings {
    /// "Reachable from outside home".
    pub fn outside_on(&self) -> bool {
        self.outside != Some(false)
    }

    /// "Keep this PC awake while it's the server".
    pub fn awake_on(&self) -> bool {
        self.awake != Some(false)
    }

    /// Go `json.MarshalIndent(s, "", "  ")`, as the file holds it.
    pub fn to_file_bytes(&self) -> Vec<u8> {
        gocompat::json::marshal_indent(self, "", "  ").expect("settings json")
    }
}

/// Why `home-server.json` couldn't be read.
#[derive(Debug)]
pub enum LoadError {
    Io(io::Error),
    /// Go: "home: home-server.json is unreadable".
    Unreadable,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "{e}"),
            LoadError::Unreadable => f.write_str("home: home-server.json is unreadable"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Go `LoadSettings`: none is the zero value (off).
pub fn load(data_dir: &Path) -> Result<Settings, LoadError> {
    let raw = match std::fs::read(data_dir.join(FILE)) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(LoadError::Io(e)),
    };
    serde_json::from_slice(&raw).map_err(|_| LoadError::Unreadable)
}

/// Go `saveSettings`: written to `home-server.json.tmp`, mode 0600, then
/// renamed over the file.
pub fn save(data_dir: &Path, s: &Settings) -> io::Result<()> {
    let path = data_dir.join(FILE);
    let tmp = data_dir.join(format!("{FILE}.tmp"));
    write_private(&tmp, &s.to_file_bytes())?;
    std::fs::rename(&tmp, &path)
}

fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
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
        f.write_all(data)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)
    }
}
