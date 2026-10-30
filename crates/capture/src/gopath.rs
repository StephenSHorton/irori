//! The `path/filepath` pieces gocompat doesn't have: `Dir`, `Base`,
//! `FromSlash`, `ToSlash`, `IsPathSeparator`, plus Go `path.Clean` (the
//! slash-only cleaner, which is filepath's Unix flavour on any OS).

pub use gocompat::filepath::{SEPARATOR, abs, clean, is_abs, join, rel, volume_name};

/// Go `os.IsPathSeparator` for the OS being built.
pub fn is_sep(c: u8) -> bool {
    if cfg!(windows) {
        c == b'\\' || c == b'/'
    } else {
        c == b'/'
    }
}

/// Go `filepath.FromSlash`.
pub fn from_slash(p: &str) -> String {
    if cfg!(windows) {
        p.replace('/', "\\")
    } else {
        p.to_string()
    }
}

/// Go `filepath.ToSlash`.
pub fn to_slash(p: &str) -> String {
    if cfg!(windows) {
        p.replace('\\', "/")
    } else {
        p.to_string()
    }
}

/// Go `filepath.Dir`.
pub fn dir(p: &str) -> String {
    let vol = volume_name(p);
    let b = p.as_bytes();
    let mut i = b.len();
    while i > vol.len() && !is_sep(b[i - 1]) {
        i -= 1;
    }
    let d = clean(&p[vol.len()..i]);
    if d == "." && vol.len() > 2 {
        return vol;
    }
    vol + &d
}

/// Go `filepath.Base`.
pub fn base(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let mut s = p;
    while !s.is_empty() && is_sep(s.as_bytes()[s.len() - 1]) {
        s = &s[..s.len() - 1];
    }
    let vol = volume_name(s);
    s = &s[vol.len()..];
    let b = s.as_bytes();
    let mut i = b.len();
    while i > 0 && !is_sep(b[i - 1]) {
        i -= 1;
    }
    s = &s[i..];
    if s.is_empty() {
        return SEPARATOR.to_string();
    }
    s.to_string()
}

/// Go `path.Clean` (forward slashes only, on any OS).
pub fn slash_clean(p: &str) -> String {
    gocompat::filepath::unix::clean(p)
}

/// Go `path.Base`.
pub fn slash_base(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let s = p.trim_end_matches('/');
    if s.is_empty() {
        return "/".into();
    }
    match s.rfind('/') {
        Some(i) => s[i + 1..].to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_and_base() {
        if cfg!(windows) {
            assert_eq!(dir(r"C:\a\b"), r"C:\a");
            assert_eq!(dir(r"C:\a"), r"C:\");
            assert_eq!(dir(r"C:\"), r"C:\");
            assert_eq!(dir(r"\\h\s\x"), r"\\h\s\");
            assert_eq!(base(r"C:\a\b\"), "b");
            assert_eq!(base(r"C:\"), r"\");
        }
        assert_eq!(dir("a"), ".");
        assert_eq!(base(""), ".");
        assert_eq!(slash_base("/usr/bin/git"), "git");
        assert_eq!(slash_base("git"), "git");
    }
}
