//! Go `path/filepath` as built for Windows: `\` and `/` both separate, the
//! output uses `\`, drive letters, UNC shares and `\\?\` / `\\.\` device
//! paths are volumes, and [`rel`] compares with `strings.EqualFold`.

use super::{Flavour, LazyBuf, RelError, bytes_to_string, clean_with, rel_with};
use crate::text::equal_fold;

/// `filepath.Separator`.
pub const SEPARATOR: char = '\\';

pub(crate) struct Windows;

fn is_sep(c: u8) -> bool {
    c == b'\\' || c == b'/'
}

fn to_upper(c: u8) -> u8 {
    c.to_ascii_uppercase()
}

/// `pathHasPrefixFold`.
fn has_prefix_fold(s: &[u8], prefix: &[u8]) -> bool {
    if s.len() < prefix.len() {
        return false;
    }
    for (i, &p) in prefix.iter().enumerate() {
        if is_sep(p) {
            if !is_sep(s[i]) {
                return false;
            }
        } else if to_upper(p) != to_upper(s[i]) {
            return false;
        }
    }
    !(s.len() > prefix.len() && !is_sep(s[prefix.len()]))
}

fn unc_len(p: &[u8], prefix_len: usize) -> usize {
    let mut count = 0;
    for (i, &c) in p.iter().enumerate().skip(prefix_len) {
        if is_sep(c) {
            count += 1;
            if count == 2 {
                return i;
            }
        }
    }
    p.len()
}

fn cut_path(p: &[u8]) -> Option<&[u8]> {
    p.iter().position(|&c| is_sep(c)).map(|i| &p[i + 1..])
}

/// filepathlite `volumeNameLen` for Windows.
fn volume_name_len(p: &[u8]) -> usize {
    if p.len() >= 2 && p[1] == b':' {
        return 2;
    }
    if p.is_empty() || !is_sep(p[0]) {
        return 0;
    }
    if has_prefix_fold(p, br"\\.\UNC") {
        return unc_len(p, br"\\.\UNC\".len());
    }
    if has_prefix_fold(p, br"\\.") || has_prefix_fold(p, br"\\?") || has_prefix_fold(p, br"\??") {
        if p.len() == 3 {
            return 3;
        }
        return match cut_path(&p[4..]) {
            None => p.len(),
            Some(rest) => p.len() - rest.len() - 1,
        };
    }
    if p.len() >= 2 && is_sep(p[1]) {
        return unc_len(p, 2);
    }
    0
}

impl Flavour for Windows {
    const SEP: u8 = b'\\';
    fn is_sep(c: u8) -> bool {
        is_sep(c)
    }
    fn volume_name_len(p: &[u8]) -> usize {
        volume_name_len(p)
    }
    fn post_clean(out: &mut LazyBuf<'_>) {
        if out_vol_len(out) != 0 || !out.modified() {
            return;
        }
        for &c in out.full_buf() {
            if is_sep(c) {
                break;
            }
            if c == b':' {
                out.prepend(b".\\");
                return;
            }
        }
        let w = out.full_buf();
        if w.len() >= 3 && is_sep(w[0]) && w[1] == b'?' && w[2] == b'?' {
            out.prepend(b"\\.");
        }
    }
    fn from_slash(mut p: Vec<u8>) -> Vec<u8> {
        for c in &mut p {
            if *c == b'/' {
                *c = b'\\';
            }
        }
        p
    }
}

fn out_vol_len(out: &LazyBuf<'_>) -> usize {
    out.vol_len
}

/// `filepath.Clean`.
pub fn clean(p: &str) -> String {
    clean_with::<Windows>(p)
}

/// `filepath.VolumeName`, with `/` turned into `\`.
pub fn volume_name(p: &str) -> String {
    let n = volume_name_len(p.as_bytes());
    bytes_to_string(Windows::from_slash(p.as_bytes()[..n].to_vec()))
}

/// `filepath.IsAbs`.
pub fn is_abs(p: &str) -> bool {
    let b = p.as_bytes();
    let l = volume_name_len(b);
    if l == 0 {
        return false;
    }
    if is_sep(b[0]) && is_sep(b[1]) {
        return true;
    }
    b.get(l).is_some_and(|&c| is_sep(c))
}

/// `filepath.Join`.
pub fn join<S: AsRef<str>>(elem: &[S]) -> String {
    let mut b = String::new();
    let mut last: u8 = 0;
    for e in elem {
        let mut e = e.as_ref();
        if b.is_empty() {
            // The first non-empty element goes in unchanged.
        } else if is_sep(last) {
            while e.as_bytes().first().is_some_and(|&c| is_sep(c)) {
                e = &e[1..];
            }
            if b.len() == 1 && e.starts_with("??") && (e.len() == 2 || is_sep(e.as_bytes()[2])) {
                b.push_str(".\\");
            }
        } else if last == b':' {
            // `C:` + `f` = `C:f`, keeping the drive-relative form.
        } else {
            b.push('\\');
            last = b'\\';
        }
        if !e.is_empty() {
            b.push_str(e);
            last = *e.as_bytes().last().expect("non-empty");
        }
    }
    if b.is_empty() {
        String::new()
    } else {
        clean(&b)
    }
}

/// `filepath.Rel`: element comparisons ignore case (`strings.EqualFold`).
pub fn rel(base: &str, targ: &str) -> Result<String, RelError> {
    rel_with::<Windows>(base, targ, clean, volume_name, equal_fold)
}

/// `filepath.Abs`: `GetFullPathNameW` (which resolves `.` and `..` against
/// the current directory, and the per-drive directory for `C:x`), then
/// `Clean`. No symlink is followed.
#[cfg(windows)]
pub fn abs(p: &str) -> std::io::Result<String> {
    use windows_sys::Win32::Storage::FileSystem::GetFullPathNameW;
    let p = if p.is_empty() { "." } else { p };
    if p.contains('\0') {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let wide: Vec<u16> = p.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf: Vec<u16> = vec![0; 260];
    loop {
        let n = unsafe {
            GetFullPathNameW(
                wide.as_ptr(),
                buf.len() as u32,
                buf.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        if n == 0 {
            return Err(std::io::Error::last_os_error());
        }
        if (n as usize) < buf.len() {
            buf.truncate(n as usize);
            break;
        }
        buf = vec![0; n as usize];
    }
    Ok(clean(&String::from_utf16_lossy(&buf)))
}
