//! Making and re-checking code anchors (I:internal/decision/anchor.go,
//! coord.md §3.17, §4). Paths follow Go's `path/filepath` on this OS.

use std::sync::atomic::{AtomicI64, Ordering};

use gocompat::filepath;

use super::gitsha;
use super::lines::{hash_lines, split_lines};
use super::{Anchor, AnchorState, CONTEXT_LINES, Error, Status};

/// The largest file `check` scans; bigger files are Changed (tests lower it).
pub(crate) static MAX_CHECK_SIZE: AtomicI64 = AtomicI64::new(8 << 20);
/// The bytes `check` hashes at most while looking for a moved window.
pub(crate) static SCAN_BUDGET: AtomicI64 = AtomicI64::new(64 << 20);

fn from_slash(p: &str) -> String {
    if cfg!(windows) {
        p.replace('/', "\\")
    } else {
        p.to_string()
    }
}

fn to_slash(p: &str) -> String {
    if cfg!(windows) {
        p.replace('\\', "/")
    } else {
        p.to_string()
    }
}

fn join(root: &str, rel: &str) -> String {
    filepath::join(&[root, &from_slash(rel)])
}

/// Reads `<root>/<path>` and builds an anchor. `path` may be absolute (it
/// must then lie under root) or root-relative; it is stored relative with
/// forward slashes. `start == 0` anchors the whole file (its LF git blob
/// id); otherwise the normalised lines `start..=end` and up to
/// CONTEXT_LINES lines around them.
pub fn make_anchor(
    root: &str,
    p: &str,
    start: i64,
    end: i64,
    repo_key: &str,
    head_sha: &str,
) -> Result<Anchor, Error> {
    if start < 0 || end < 0 || (start == 0 && end != 0) || (start > 0 && end < start) {
        return Err(Error::BadLines);
    }
    let rel = rel_path(root, p)?;
    let full = join(root, &rel);
    match std::fs::metadata(&full) {
        Ok(m) if !m.is_dir() => {}
        _ => return Err(Error::NoFile),
    }
    let data = std::fs::read(&full).map_err(|_| Error::NoFile)?;
    let rel = on_disk_case(root, &rel);
    let mut a = Anchor {
        repo_key: repo_key.into(),
        root: filepath::clean(root),
        path: rel,
        sha: head_sha.into(),
        ..Anchor::default()
    };
    if start == 0 {
        a.hash = gitsha::blob(&gitsha::lf(&data));
        return Ok(a);
    }
    let lines = split_lines(&data);
    let n = lines.len() as i64;
    if end > n {
        return Err(Error::Range);
    }
    a.start = start;
    a.end = end;
    a.hash = hash_lines(&lines[(start - 1) as usize..end as usize]);
    let ctx = CONTEXT_LINES as i64;
    for i in (start - ctx).max(1)..start {
        a.context.push(lossy(&lines[(i - 1) as usize]));
    }
    for i in end + 1..=n.min(end + ctx) {
        a.context.push(lossy(&lines[(i - 1) as usize]));
    }
    Ok(a)
}

/// A Go string holding these bytes, as JSON would carry it.
fn lossy(b: &[u8]) -> String {
    gocompat::utf8::to_valid_string(b)
}

/// An absolute path under root, or a root-relative one, as a clean
/// forward-slash repo-relative path.
pub(crate) fn rel_path(root: &str, p: &str) -> Result<String, Error> {
    let p = crate::names::trim_space(&p.replace('\\', "/")).to_string();
    if root.is_empty() || p.is_empty() || has_dot_dot(&p) {
        return Err(Error::BadPath);
    }
    let native = from_slash(&p);
    let mut p = p;
    if filepath::is_abs(&native) {
        let r = under(&filepath::clean(root), &filepath::clean(&native)).ok_or(Error::BadPath)?;
        p = to_slash(&r);
    } else if p.starts_with('/') || !filepath::volume_name(&native).is_empty() {
        return Err(Error::BadPath);
    }
    let p = filepath::unix::clean(&p);
    if p == "." || p.starts_with('/') || has_dot_dot(&p) {
        return Err(Error::BadPath);
    }
    Ok(p)
}

/// `p` relative to root when it lies strictly inside it, compared
/// case-insensitively on Windows.
fn under(root: &str, p: &str) -> Option<String> {
    let sep = filepath::SEPARATOR;
    let mut prefix = root.to_string();
    if !prefix.ends_with(sep) {
        prefix.push(sep);
    }
    if p.len() <= prefix.len() || !p.is_char_boundary(prefix.len()) {
        // Go slices bytes; a cut inside a rune can never fold-equal the
        // prefix, which ends in a separator.
        return None;
    }
    let head = &p[..prefix.len()];
    let same = if cfg!(windows) {
        gocompat::text::equal_fold(head, &prefix)
    } else {
        head == prefix
    };
    same.then(|| p[prefix.len()..].to_string())
}

fn has_dot_dot(p: &str) -> bool {
    p.split('/').any(|s| s == "..")
}

/// Re-reads the anchored file under root (`""` is Unknown).
pub fn check(a: &Anchor, root: &str) -> Status {
    if root.is_empty() || a.hash.is_empty() || a.start < 0 || (a.start > 0 && a.end < a.start) {
        return Status::of(AnchorState::Unknown);
    }
    match std::fs::metadata(root) {
        Ok(m) if m.is_dir() => {}
        _ => return Status::of(AnchorState::Unknown),
    }
    let rel = match rel_path(root, &a.path) {
        Ok(r) if r == a.path => r,
        _ => return Status::of(AnchorState::Unknown),
    };
    let full = join(root, &rel);
    let meta = match std::fs::metadata(&full) {
        Ok(m) if m.is_dir() => return Status::of(AnchorState::Gone),
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Status::of(AnchorState::Gone);
        }
        Err(_) => return Status::of(AnchorState::Unknown),
    };
    if meta.len() as i64 > MAX_CHECK_SIZE.load(Ordering::Relaxed) {
        return Status::of(AnchorState::Changed);
    }
    let data = match std::fs::read(&full) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Status::of(AnchorState::Gone);
        }
        Err(_) => return Status::of(AnchorState::Unknown),
    };
    if a.start == 0 {
        if gitsha::blob(&gitsha::lf(&data)) == a.hash {
            return Status::of(AnchorState::Ok);
        }
        return Status::of(AnchorState::Changed);
    }
    let lines = split_lines(&data);
    let n = lines.len() as i64;
    if a.end - a.start >= n {
        return Status::of(AnchorState::Changed);
    }
    let size = a.end - a.start + 1;
    let last = n - size + 1;
    let window = |s: i64| &lines[(s - 1) as usize..(s - 1 + size) as usize];
    if a.start <= last && hash_lines(window(a.start)) == a.hash {
        return Status {
            state: AnchorState::Ok,
            start: a.start,
            end: a.end,
        };
    }
    let mut off = vec![0i64; lines.len() + 1];
    for (i, l) in lines.iter().enumerate() {
        off[i + 1] = off[i] + l.len() as i64 + 1;
    }
    let (mut lo, mut hi) = (a.start - 1, a.start.wrapping_add(1));
    if a.start > last {
        lo = last;
        hi = last + 1;
    }
    let mut budget = SCAN_BUDGET.load(Ordering::Relaxed);
    while lo >= 1 || hi <= last {
        let s;
        if lo >= 1 && (hi > last || a.start - lo <= hi - a.start) {
            s = lo;
            lo -= 1;
        } else {
            s = hi;
            hi += 1;
        }
        budget -= off[(s - 1 + size) as usize] - off[(s - 1) as usize];
        if budget < 0 {
            break;
        }
        if hash_lines(window(s)) == a.hash {
            return Status {
                state: AnchorState::Moved,
                start: s,
                end: s + size - 1,
            };
        }
    }
    Status::of(AnchorState::Changed)
}

/// Rewrites each segment of `rel` to its on-disk spelling when exactly one
/// directory entry matches it case-insensitively and none exactly.
fn on_disk_case(root: &str, rel: &str) -> String {
    let mut segs: Vec<String> = rel.split('/').map(str::to_string).collect();
    let mut dir = root.to_string();
    for seg in segs.iter_mut() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return rel.to_string();
        };
        let mut found = false;
        let mut folded = String::new();
        let mut nfold = 0;
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        // os.ReadDir sorts by name.
        names.sort();
        for name in names {
            if name == *seg {
                found = true;
                break;
            }
            if gocompat::text::equal_fold(&name, seg) {
                folded = name;
                nfold += 1;
            }
        }
        if !found && nfold == 1 {
            *seg = folded;
        }
        dir = filepath::join(&[dir.as_str(), seg.as_str()]);
    }
    segs.join("/")
}
