//! Go's `path/filepath`, lexical and byte for byte (capture.md §7.1,
//! identity.md §7.3 trap 4).
//!
//! Both flavours are always compiled, so either can be tested anywhere:
//! [`unix`] (Go on macOS and Linux) and [`windows`] (Go on Windows: `\` and
//! `/` both separate, drive letters and UNC volumes, case-insensitive
//! [`windows::rel`]). The functions at this module's root are the flavour of
//! the OS being built, as Go's `filepath` is.
//!
//! Nothing here touches the file system, except [`windows::abs`] on Windows,
//! which asks `GetFullPathNameW` as Go does. `std::fs::canonicalize` resolves
//! symlinks and adds `\\?\`, and `Path::strip_prefix` is case-sensitive;
//! neither matches Go.

pub mod unix;
pub mod windows;

#[cfg(not(windows))]
pub use unix::{SEPARATOR, abs, clean, is_abs, join, rel, volume_name};
#[cfg(windows)]
pub use windows::{SEPARATOR, abs, clean, is_abs, join, rel, volume_name};

/// Why [`rel`] failed, with Go's text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Rel: can't make {targ} relative to {base}")]
pub struct RelError {
    pub base: String,
    pub targ: String,
}

/// Go's `lazybuf` from `internal/filepathlite`. It matters for more than
/// speed: Windows `postClean` only runs when the buffer was modified.
pub(crate) struct LazyBuf<'a> {
    path: &'a [u8],
    buf: Option<Vec<u8>>,
    w: usize,
    vol_and_path: &'a [u8],
    vol_len: usize,
}

impl<'a> LazyBuf<'a> {
    pub(crate) fn new(path: &'a [u8], vol_and_path: &'a [u8], vol_len: usize) -> Self {
        LazyBuf {
            path,
            buf: None,
            w: 0,
            vol_and_path,
            vol_len,
        }
    }

    fn index(&self, i: usize) -> u8 {
        match &self.buf {
            Some(b) => b[i],
            None => self.path[i],
        }
    }

    fn append(&mut self, c: u8) {
        if self.buf.is_none() {
            if self.w < self.path.len() && self.path[self.w] == c {
                self.w += 1;
                return;
            }
            let mut b = vec![0u8; self.path.len().max(self.w + 1)];
            b[..self.w].copy_from_slice(&self.path[..self.w]);
            self.buf = Some(b);
        }
        let b = self.buf.as_mut().expect("buf");
        if self.w < b.len() {
            b[self.w] = c;
        } else {
            b.push(c);
        }
        self.w += 1;
    }

    pub(crate) fn modified(&self) -> bool {
        self.buf.is_some()
    }

    /// The whole allocated buffer, stale bytes past `w` included: Go's
    /// `postClean` ranges over all of it, so a `:` left behind by an
    /// element that `..` removed still counts (`z/../ab:/../c` cleans to
    /// `.\c` on Windows).
    pub(crate) fn full_buf(&self) -> &[u8] {
        self.buf.as_deref().unwrap_or(&[])
    }

    pub(crate) fn prepend(&mut self, prefix: &[u8]) {
        let b = self.buf.get_or_insert_with(|| self.path[..self.w].to_vec());
        b.truncate(self.w);
        let mut v = prefix.to_vec();
        v.extend_from_slice(b);
        *b = v;
        self.w += prefix.len();
    }

    fn string(&self) -> Vec<u8> {
        match &self.buf {
            None => self.vol_and_path[..self.vol_len + self.w].to_vec(),
            Some(b) => {
                let mut v = self.vol_and_path[..self.vol_len].to_vec();
                v.extend_from_slice(&b[..self.w]);
                v
            }
        }
    }
}

/// The flavour-specific parts of `Clean`.
pub(crate) trait Flavour {
    const SEP: u8;
    fn is_sep(c: u8) -> bool;
    fn volume_name_len(p: &[u8]) -> usize;
    fn post_clean(_out: &mut LazyBuf<'_>) {}
    fn from_slash(p: Vec<u8>) -> Vec<u8> {
        p
    }
}

/// filepathlite `Clean`, generic over the flavour.
pub(crate) fn clean_with<F: Flavour>(original: &str) -> String {
    let op = original.as_bytes();
    let vol_len = F::volume_name_len(op);
    let path = &op[vol_len..];
    if path.is_empty() {
        if vol_len > 1 && F::is_sep(op[0]) && F::is_sep(op[1]) {
            // UNC
            return bytes_to_string(F::from_slash(op.to_vec()));
        }
        return format!("{original}.");
    }
    let rooted = F::is_sep(path[0]);
    let n = path.len();
    let mut out = LazyBuf::new(path, op, vol_len);
    let (mut r, mut dotdot) = (0usize, 0usize);
    if rooted {
        out.append(F::SEP);
        r = 1;
        dotdot = 1;
    }
    while r < n {
        if F::is_sep(path[r]) || (path[r] == b'.' && (r + 1 == n || F::is_sep(path[r + 1]))) {
            // An empty element or a `.` element.
            r += 1;
        } else if path[r] == b'.'
            && path.get(r + 1) == Some(&b'.')
            && (r + 2 == n || F::is_sep(path[r + 2]))
        {
            r += 2;
            if out.w > dotdot {
                out.w -= 1;
                while out.w > dotdot && !F::is_sep(out.index(out.w)) {
                    out.w -= 1;
                }
            } else if !rooted {
                if out.w > 0 {
                    out.append(F::SEP);
                }
                out.append(b'.');
                out.append(b'.');
                dotdot = out.w;
            }
        } else {
            if (rooted && out.w != 1) || (!rooted && out.w != 0) {
                out.append(F::SEP);
            }
            while r < n && !F::is_sep(path[r]) {
                out.append(path[r]);
                r += 1;
            }
        }
    }
    if out.w == 0 {
        out.append(b'.');
    }
    F::post_clean(&mut out);
    bytes_to_string(F::from_slash(out.string()))
}

/// Paths are sliced only at ASCII bytes, so UTF-8 input stays UTF-8.
pub(crate) fn bytes_to_string(b: Vec<u8>) -> String {
    String::from_utf8(b).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// filepath `Rel`, generic over the flavour. `same` is `sameWord`.
pub(crate) fn rel_with<F: Flavour>(
    base_path: &str,
    targ_path: &str,
    clean: fn(&str) -> String,
    volume_name: fn(&str) -> String,
    same: fn(&str, &str) -> bool,
) -> Result<String, RelError> {
    let base_vol = volume_name(base_path);
    let targ_vol = volume_name(targ_path);
    let base_c = clean(base_path);
    let targ_c = clean(targ_path);
    if same(&targ_c, &base_c) {
        return Ok(".".into());
    }
    let mut base = &base_c[base_vol.len()..];
    let targ = &targ_c[targ_vol.len()..];
    let sep_s = (F::SEP as char).to_string();
    if base == "." {
        base = "";
    } else if base.is_empty() && F::volume_name_len(base_vol.as_bytes()) > 2 {
        base = &sep_s;
    }
    let base_slashed = base.as_bytes().first() == Some(&F::SEP);
    let targ_slashed = targ.as_bytes().first() == Some(&F::SEP);
    let err = || RelError {
        base: base_path.to_string(),
        targ: targ_path.to_string(),
    };
    if base_slashed != targ_slashed || !same(&base_vol, &targ_vol) {
        return Err(err());
    }
    let (bb, tb) = (base.as_bytes(), targ.as_bytes());
    let (bl, tl) = (bb.len(), tb.len());
    let (mut b0, mut bi, mut t0, mut ti) = (0, 0, 0, 0);
    loop {
        while bi < bl && bb[bi] != F::SEP {
            bi += 1;
        }
        while ti < tl && tb[ti] != F::SEP {
            ti += 1;
        }
        if !same(&targ[t0..ti], &base[b0..bi]) {
            break;
        }
        if bi < bl {
            bi += 1;
        }
        if ti < tl {
            ti += 1;
        }
        b0 = bi;
        t0 = ti;
    }
    if &base[b0..bi] == ".." {
        return Err(err());
    }
    if b0 != bl {
        let seps = bb[b0..bl].iter().filter(|&&c| c == F::SEP).count();
        let mut buf = String::from("..");
        for _ in 0..seps {
            buf.push(F::SEP as char);
            buf.push_str("..");
        }
        if t0 != tl {
            buf.push(F::SEP as char);
            buf.push_str(&targ[t0..]);
        }
        return Ok(clean(&buf));
    }
    Ok(targ[t0..].to_string())
}
