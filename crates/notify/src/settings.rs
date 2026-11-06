//! settings.json (I:internal/notify/settings.go): Irori's app settings in
//! its data folder. Each preference is a top-level key, and keys this
//! crate doesn't know are kept as they are. Go's notify package is the only
//! writer of this file in Go (`settings_service.go` reads through it), so
//! this is the one Rust writer too: every owner of a key goes through
//! [`load`]/[`save`]'s process lock.
//!
//! Byte-compatible with Go (testdata/golden/notify/settings.json): the
//! file is `json.MarshalIndent(map[string]json.RawMessage, "", "  ")` plus
//! a newline, so keys come out sorted and every value, the unknown ones
//! too, is compacted, HTML-escaped and re-indented; `notifications` is the
//! options' compact `json.Marshal`. Writes go to a temp file named
//! `settings.json.*` in the folder, then a rename.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use std::sync::Mutex;

use gocompat::json::{Out, RawJson, Schema};

use crate::Options;
use crate::look::CardLook;

/// Irori's app settings file in its data folder.
pub const SETTINGS_FILE: &str = "settings.json";

/// The notifications object's key.
const SETTINGS_KEY: &str = "notifications";

/// The cards' look's key (card #527).
const LOOK_KEY: &str = "cards";
/// Keep this device up to date (card #578); missing until the person sets it.
const KEEP_KEY: &str = "keepUpToDate";

/// Serializes read-modify-write of the file in this process.
static FILE_MU: Mutex<()> = Mutex::new(());

fn options_schema() -> Schema {
    Schema::object(vec![
        ("enabled", Schema::Bool),
        ("sound", Schema::Bool),
        ("volume", Schema::Float(64)),
        ("corner", Schema::String),
        ("confetti", Schema::Bool),
    ])
}

/// The saved options: the defaults for a missing or unreadable file, and
/// unset fields filled from the defaults.
pub fn load(dir: &Path) -> Options {
    let _g = FILE_MU.lock().unwrap_or_else(|e| e.into_inner());
    let all = match read_all(dir) {
        Ok(all) => all,
        Err(e) => {
            eprintln!("irori: notifications: {e}; using the defaults");
            return Options::default();
        }
    };
    let Some(raw) = all.get(SETTINGS_KEY) else {
        return Options::default();
    };
    // Decoding over the defaults keeps a field the file leaves out.
    let d = Options::default();
    let mut out = Out::Struct(vec![
        ("enabled", Out::Bool(d.enabled)),
        ("sound", Out::Bool(d.sound)),
        ("volume", Out::Float(d.volume)),
        ("corner", Out::String(d.corner.clone())),
        ("confetti", Out::Bool(d.confetti)),
    ]);
    if let Err(e) = gocompat::json::lenient::unmarshal_into(raw, &options_schema(), &mut out) {
        eprintln!("irori: notifications: {SETTINGS_FILE}: {e}; using the defaults");
        return Options::default();
    }
    let b = |name: &str| matches!(out.get(name), Some(Out::Bool(true)));
    Options {
        enabled: b("enabled"),
        sound: b("sound"),
        volume: match out.get("volume") {
            Some(Out::Float(f)) => *f,
            _ => d.volume,
        },
        corner: out.get_str("corner").to_string(),
        confetti: b("confetti"),
    }
    .normalized()
}

/// Writes o under `notifications`, keeping the file's other keys. A broken
/// file is replaced.
pub fn save(dir: &Path, o: &Options) -> Result<(), String> {
    let raw = gocompat::json::marshal(o).map_err(|e| e.to_string())?;
    save_raw(dir, SETTINGS_KEY, raw)
}

/// The saved look of the cards, if the window has sent one (card #527).
pub fn load_look(dir: &Path) -> Option<CardLook> {
    let _g = FILE_MU.lock().unwrap_or_else(|e| e.into_inner());
    let raw = read_all(dir).ok()?.remove(LOOK_KEY)?;
    match serde_json::from_slice::<Option<CardLook>>(&raw) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("irori: notifications: {SETTINGS_FILE}: {LOOK_KEY}: {e}");
            None
        }
    }
}

/// Writes the cards' look under `cards`, keeping the file's other keys.
pub fn save_look(dir: &Path, l: &CardLook) -> Result<(), String> {
    let raw = gocompat::json::marshal(l).map_err(|e| e.to_string())?;
    save_raw(dir, LOOK_KEY, raw)
}

/// Keep this device up to date (card #578), once the person has set it.
/// `None` means they never have: the device follows its home server switch.
pub fn load_keep_up_to_date(dir: &Path) -> Option<bool> {
    let _g = FILE_MU.lock().unwrap_or_else(|e| e.into_inner());
    let raw = read_all(dir).ok()?.remove(KEEP_KEY)?;
    match serde_json::from_slice::<Option<bool>>(&raw) {
        Ok(on) => on,
        Err(e) => {
            eprintln!("irori: notifications: {SETTINGS_FILE}: {KEEP_KEY}: {e}");
            None
        }
    }
}

/// Writes the person's choice under `keepUpToDate`, keeping the other keys.
pub fn save_keep_up_to_date(dir: &Path, on: bool) -> Result<(), String> {
    let raw = gocompat::json::marshal(&on).map_err(|e| e.to_string())?;
    save_raw(dir, KEEP_KEY, raw)
}

fn save_raw(dir: &Path, key: &str, raw: Vec<u8>) -> Result<(), String> {
    let _g = FILE_MU.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = read_all(dir).unwrap_or_default();
    all.insert(key.into(), raw);
    let map: BTreeMap<String, RawJson> = all.into_iter().map(|(k, v)| (k, RawJson(v))).collect();
    let mut out = gocompat::json::marshal_indent(&map, "", "  ").map_err(|e| e.to_string())?;
    out.push(b'\n');
    make_dir(dir).map_err(|e| e.to_string())?;
    let fail = |e: std::io::Error| format!("save {SETTINGS_FILE}: {e}");
    let mut tmp = tempfile::Builder::new()
        .prefix(&format!("{SETTINGS_FILE}."))
        .rand_bytes(9)
        .tempfile_in(dir)
        .map_err(|e| e.to_string())?;
    tmp.write_all(&out).map_err(fail)?;
    tmp.persist(dir.join(SETTINGS_FILE))
        .map_err(|e| fail(e.error))?;
    Ok(())
}

#[cfg(unix)]
fn make_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn make_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// The file's top-level object as raw values (Go
/// `map[string]json.RawMessage`): a missing file is empty, a file that
/// isn't a JSON object is an error. A top-level `null` reads as empty (Go
/// would leave a nil map, and its save would then panic).
fn read_all(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let b = match std::fs::read(dir.join(SETTINGS_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.to_string()),
    };
    let v = gocompat::json::lenient::unmarshal(&b, &Schema::Map(Box::new(Schema::Raw)))
        .map_err(|e| format!("{SETTINGS_FILE}: {e}"))?;
    let mut out = BTreeMap::new();
    if let Out::Map(Some(m)) = v {
        for (k, v) in m {
            if let Out::Raw(Some(raw)) = v {
                out.insert(k, raw);
            } else {
                out.insert(k, b"null".to_vec());
            }
        }
    }
    Ok(out)
}

/// The message cards already shown (#364): one delivery id per line in
/// `notify-seen` in the data folder, written whole through a temp file and
/// a rename. Nothing there yet reads as empty.
pub const SEEN_FILE: &str = "notify-seen";

pub struct SeenFile(pub std::path::PathBuf);

impl SeenFile {
    /// The file in data folder dir.
    pub fn new(dir: &Path) -> SeenFile {
        SeenFile(dir.to_path_buf())
    }
}

impl inbox::watch::SeenStore for SeenFile {
    fn load(&self) -> std::collections::HashSet<String> {
        std::fs::read_to_string(self.0.join(SEEN_FILE))
            .map(|s| {
                s.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn save(&self, ids: &std::collections::HashSet<String>) {
        let mut v: Vec<&String> = ids.iter().collect();
        v.sort();
        let mut out = String::new();
        for id in v {
            out.push_str(id);
            out.push('\n');
        }
        let r = (|| -> std::io::Result<()> {
            make_dir(&self.0)?;
            let mut tmp = tempfile::Builder::new()
                .prefix(&format!("{SEEN_FILE}."))
                .rand_bytes(9)
                .tempfile_in(&self.0)?;
            tmp.write_all(out.as_bytes())?;
            tmp.persist(self.0.join(SEEN_FILE)).map_err(|e| e.error)?;
            Ok(())
        })();
        if let Err(e) = r {
            eprintln!("irori: notifications: save {SEEN_FILE}: {e}");
        }
    }
}
