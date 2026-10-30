//! Go `path/filepath` as built for macOS and Linux (`/` only, no volumes,
//! case-sensitive). `clean` is also Go's `path.Clean`.

use super::{Flavour, RelError, clean_with, rel_with};

/// `filepath.Separator`.
pub const SEPARATOR: char = '/';

pub(crate) struct Unix;

impl Flavour for Unix {
    const SEP: u8 = b'/';
    fn is_sep(c: u8) -> bool {
        c == b'/'
    }
    fn volume_name_len(_: &[u8]) -> usize {
        0
    }
}

/// `filepath.Clean`: lexical; `..` is resolved without looking at the disk.
pub fn clean(p: &str) -> String {
    clean_with::<Unix>(p)
}

/// `filepath.IsAbs`.
pub fn is_abs(p: &str) -> bool {
    p.starts_with('/')
}

/// `filepath.VolumeName`: always empty here.
pub fn volume_name(_: &str) -> String {
    String::new()
}

/// `filepath.Join`: the non-empty elements from the first non-empty one,
/// joined by `/`, then cleaned; `""` when all are empty.
pub fn join<S: AsRef<str>>(elem: &[S]) -> String {
    match elem.iter().position(|e| !e.as_ref().is_empty()) {
        Some(i) => clean(
            &elem[i..]
                .iter()
                .map(|e| e.as_ref())
                .collect::<Vec<_>>()
                .join("/"),
        ),
        None => String::new(),
    }
}

/// `filepath.Rel`.
pub fn rel(base: &str, targ: &str) -> Result<String, RelError> {
    rel_with::<Unix>(base, targ, clean, volume_name, |a, b| a == b)
}

/// `filepath.Abs` with an explicit working directory: `Clean(p)` when
/// absolute, else `Join(cwd, p)`. Lexical, so no symlink is resolved.
pub fn abs_from(p: &str, cwd: &str) -> String {
    if is_abs(p) { clean(p) } else { join(&[cwd, p]) }
}

/// `filepath.Abs` against the process's working directory, as Go's
/// `os.Getwd` names it ([`getwd`]).
pub fn abs(p: &str) -> std::io::Result<String> {
    if is_abs(p) {
        return Ok(clean(p));
    }
    Ok(abs_from(p, &getwd()?))
}

/// Go `os.Getwd` on Unix: `$PWD` when it is absolute and names the same
/// directory as `.` (so a cwd reached through a symlink keeps that
/// spelling), else the system's answer, which resolves symlinks.
pub fn getwd() -> std::io::Result<String> {
    let pwd = std::env::var("PWD").ok();
    getwd_with(pwd.as_deref(), &same_dir_as_dot, &|| {
        std::env::current_dir().map(|c| c.to_string_lossy().into_owned())
    })
}

/// [`getwd`] with its inputs given: `pwd` is `$PWD`, `same_as_dot` is
/// Go's `SameFile(Stat("."), Stat(dir))`, `sys` is `syscall.Getwd`.
pub fn getwd_with(
    pwd: Option<&str>,
    same_as_dot: &dyn Fn(&str) -> bool,
    sys: &dyn Fn() -> std::io::Result<String>,
) -> std::io::Result<String> {
    if let Some(d) = pwd
        && d.starts_with('/')
        && same_as_dot(d)
    {
        return Ok(d.to_string());
    }
    sys()
}

#[cfg(unix)]
fn same_dir_as_dot(dir: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata("."), std::fs::metadata(dir)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_dir_as_dot(dir: &str) -> bool {
    match (std::fs::canonicalize("."), std::fs::canonicalize(dir)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
