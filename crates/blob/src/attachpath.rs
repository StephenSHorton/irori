//! Go `attachpath.go`: the never-leave rules, `Within` and `SafeName`.

use std::fs;

use gocompat::filepath;
use gocompat::text;

use crate::error::{self, Error, ErrorKind, Result, os_text};
use crate::fsutil::go_ext;

/// Where attachment_get puts files by default, inside the data dir:
/// `<dataDir>/received/<session>/<name>`. It is the one part of the data
/// dir an attach may read, so a received file can be passed on.
pub const RECEIVED_DIR: &str = "received";

/// Folders whose files never leave the device, on any path element.
const CREDENTIAL_DIRS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".password-store",
    "keychains",
];

/// Files that never leave, besides scrub's SkipFile names.
const CREDENTIAL_NAMES: &[&str] = &[
    ".git-credentials",
    ".credentials.json",
    ".pgpass",
    "irori.json",
    "device.key",
    "person.json",
];

/// Extensions of key and password stores.
const CREDENTIAL_SUFFIXES: &[&str] = &[".keychain", ".keychain-db", ".kdbx", ".ppk", ".gpg"];

fn never_leaves_err(why: String) -> Error {
    Error::new(
        ErrorKind::NeverLeaves,
        format!("{}: {why}", error::NEVER_LEAVES),
    )
}

/// Go `strings.TrimSpace` (Unicode white space).
fn trim_space(s: &str) -> &str {
    s.trim_matches(text::is_space)
}

/// Go `CheckAttachPath`: resolves `path` to its canonical form, following
/// symlinks, and applies the never-leave rules to the given and the
/// resolved path: anything inside `data_dir` (except [`RECEIVED_DIR`]),
/// .env files, private keys and credential stores are refused with the
/// reason. A relative path is taken against `cwd`; with no cwd it must be
/// absolute. Returns the resolved path of a regular file.
pub fn check_attach_path(path: &str, cwd: &str, data_dir: &str) -> Result<String> {
    let mut path = trim_space(path).to_string();
    if path.is_empty() {
        return Err(Error::other("blob: give a file path"));
    }
    if !filepath::is_abs(&path) {
        if cwd.is_empty() || !filepath::is_abs(cwd) {
            return Err(Error::other(format!(
                "blob: {} is relative and your working directory is unknown: give an absolute path",
                gocompat::quote::quote(&path)
            )));
        }
        path = filepath::join(&[cwd, path.as_str()]);
    }
    // The name as given is checked too, so a link named .env is refused
    // whatever it points at.
    if let Some(why) = never_leaves(&path) {
        return Err(never_leaves_err(why));
    }
    let abs = go_abs(&path)?;
    let real = eval_symlinks(&abs).map_err(|e| Error::other(format!("blob: {e}")))?;
    let real = go_abs(&real)?;
    if let Some(why) = never_leaves(&real) {
        return Err(never_leaves_err(format!("it resolves to {real}, {why}")));
    }
    if !data_dir.is_empty() {
        let data = eval_symlinks(data_dir).unwrap_or_else(|_| data_dir.to_string());
        let data = go_abs(&data)?;
        if within(&real, &data) && !within(&real, &filepath::join(&[data.as_str(), RECEIVED_DIR])) {
            return Err(never_leaves_err(format!(
                "it resolves into Irori's data dir ({real})"
            )));
        }
    }
    let meta = fs::metadata(&real)
        .map_err(|e| Error::other(format!("blob: {} {real}: {}", stat_op(), os_text(&e))))?;
    if !meta.is_file() {
        return Err(Error::new(
            ErrorKind::NotFile,
            format!("{}: {real}", error::NOT_FILE),
        ));
    }
    Ok(real)
}

/// The op Go's `os.Stat` names in its error for a missing file.
pub(crate) fn stat_op() -> &'static str {
    if cfg!(windows) {
        "GetFileAttributesEx"
    } else {
        "stat"
    }
}

/// The op Go's `os.Lstat` names in its error for a missing file.
fn lstat_op() -> &'static str {
    if cfg!(windows) {
        "GetFileAttributesEx"
    } else {
        "lstat"
    }
}

fn go_abs(p: &str) -> Result<String> {
    filepath::abs(p).map_err(|e| Error::other(os_text(&e)))
}

/// Go `filepath.EvalSymlinks` for an existing path: the canonical path with
/// every link resolved, without Windows' `\\?\` prefix. A path that does
/// not exist fails as Go's walk does, on the first missing prefix:
/// "lstat <prefix>: <os text>".
fn eval_symlinks(p: &str) -> std::result::Result<String, String> {
    match fs::canonicalize(p) {
        Ok(c) => Ok(strip_verbatim(&c.to_string_lossy())),
        Err(e) => {
            // Find the first prefix that does not lstat.
            let clean = filepath::clean(p);
            let mut prefix = String::new();
            for (i, part) in clean.split(['/', '\\']).enumerate() {
                if i > 0 {
                    prefix.push(filepath::SEPARATOR);
                }
                prefix.push_str(part);
                if part.is_empty() || (i == 0 && part.ends_with(':')) {
                    continue;
                }
                if let Err(le) = fs::symlink_metadata(&prefix) {
                    return Err(format!("{} {prefix}: {}", lstat_op(), os_text(&le)));
                }
            }
            Err(format!("{} {clean}: {}", lstat_op(), os_text(&e)))
        }
    }
}

/// `\\?\C:\x` → `C:\x`, `\\?\UNC\srv\share` → `\\srv\share`.
fn strip_verbatim(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

/// Go `neverLeaves`: the rule a path hits, or None.
pub fn never_leaves(p: &str) -> Option<String> {
    if scrub::skip_file(p) {
        return Some("it is a secrets file (.env, a private key or credentials)".into());
    }
    let lower = text::to_lower_str(p);
    let elems: Vec<&str> = lower.split(['/', '\\']).filter(|e| !e.is_empty()).collect();
    let last = elems.len().saturating_sub(1);
    for (i, raw) in elems.iter().enumerate() {
        let e = raw.trim_end_matches(['.', ' ']);
        for d in CREDENTIAL_DIRS {
            if e == *d && i < last {
                return Some(format!("it is inside a credential store ({d})"));
            }
        }
        if i == last {
            for n in CREDENTIAL_NAMES {
                if e == *n {
                    return Some(format!("it is a credential or key file ({n})"));
                }
            }
            for s in CREDENTIAL_SUFFIXES {
                if e.ends_with(s) {
                    return Some(format!("it is a key or password store ({s})"));
                }
            }
        }
        // Windows' credential manager and DPAPI master keys.
        if e == "microsoft"
            && i + 1 < elems.len()
            && (elems[i + 1] == "credentials" || elems[i + 1] == "protect")
        {
            return Some("it is inside the Windows credential store".into());
        }
    }
    None
}

/// Go `Within`: whether `p` is `base` or inside it (case-insensitively on
/// Windows and macOS, whose default file systems are).
pub fn within(p: &str, base: &str) -> bool {
    let (p, base) = if cfg!(any(windows, target_os = "macos")) {
        (text::to_lower_str(p), text::to_lower_str(base))
    } else {
        (p.to_string(), base.to_string())
    };
    let Ok(rel) = filepath::rel(&base, &p) else {
        return false;
    };
    rel == "." || rel != ".." && !rel.starts_with(&format!("..{}", filepath::SEPARATOR))
}

/// The longest sanitized name, in runes, keeping its extension.
const MAX_NAME: usize = 120;

/// Go `SafeName`: reduces an attachment name from a row, which another
/// device wrote, to a single safe file name: no separators, no "..", no
/// drive letter or stream suffix, no control, invisible format or reserved
/// characters, no trailing dots or spaces, and not a reserved Windows
/// device name. An empty result falls back to "attachment-" and the hash's
/// first 12 hex.
pub fn safe_name(name: &str, sha: &str) -> String {
    let mut name = name;
    if let Some(i) = name.rfind(['/', '\\']) {
        name = &name[i + 1..];
    }
    let nb = name.as_bytes();
    if nb.len() >= 2 && nb[1] == b':' && nb[0].is_ascii_alphabetic() {
        name = &name[2..];
    }
    if let Some(i) = name.find(':') {
        name = &name[..i]; // an alternate data stream
    }
    let mut b = String::with_capacity(name.len());
    for r in name.chars() {
        if (r as u32) < 0x20
            || r == '\u{7f}'
            || text::is_control(r)
            || r == '\u{FFFD}'
            || text::is_cf(r)
            || r#"<>:"/\|?*"#.contains(r)
        {
            b.push('_');
        } else {
            b.push(r);
        }
    }
    let trimmed = trim_space(b.trim_end_matches(['.', ' '])).trim_start_matches(['.', ' ']);
    if trimmed.is_empty() {
        let mut end = sha.len().min(12);
        while !sha.is_char_boundary(end) {
            end -= 1;
        }
        return format!("attachment-{}", &sha[..end]);
    }
    let mut name = trimmed.to_string();
    if reserved_name(&name) {
        name = format!("_{name}");
    }
    let runes: Vec<char> = name.chars().collect();
    if runes.len() > MAX_NAME {
        let mut ext: Vec<char> = go_ext(&name).chars().collect();
        if ext.len() > 16 {
            ext.clear();
        }
        let mut out: String = runes[..MAX_NAME - ext.len()].iter().collect();
        out.extend(ext);
        name = out;
    }
    name
}

/// Go `reservedName`: CON, PRN, AUX, NUL, CONIN$, CONOUT$, COM0-9 and
/// LPT0-9 (and their superscript-digit forms), with or without an
/// extension, in any case.
fn reserved_name(name: &str) -> bool {
    let upper = text::to_upper_str(name);
    let base = match upper.find('.') {
        Some(i) => &upper[..i],
        None => upper.as_str(),
    };
    let base = base.trim_end_matches(' ');
    if matches!(base, "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }
    let r: Vec<char> = base.chars().collect();
    if r.len() == 4 {
        let head: String = r[..3].iter().collect();
        if head == "COM" || head == "LPT" {
            let d = r[3];
            return d.is_ascii_digit() || matches!(d, '¹' | '²' | '³');
        }
    }
    false
}
