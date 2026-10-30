//! Git object ids for file contents, a port of I:internal/gitsha, so an
//! edit's result can be matched against the blob a commit stored
//! (capture.md §0 I5).

use sha1::{Digest, Sha1};

/// The git blob id (SHA-1, lowercase hex) of `data`: what `git hash-object`
/// prints for a file holding exactly these bytes.
pub fn blob(data: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", data.len()).as_bytes());
    h.update(data);
    hex::encode(h.finalize())
}

/// `data` with every CRLF turned into LF: what core.autocrlf commits on
/// Windows. Borrowed when there is no CRLF.
pub fn lf(data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if !data.windows(2).any(|w| w == b"\r\n") {
        return std::borrow::Cow::Borrowed(data);
    }
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'\r' && data.get(i + 1) == Some(&b'\n') {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    std::borrow::Cow::Owned(out)
}

/// `blob(data)` and, when it differs, `blob(lf(data))`; the second is ""
/// when `data` has no CRLF.
pub fn pair(data: &[u8]) -> (String, String) {
    let raw = blob(data);
    let l = lf(data);
    let lf_id = if l.len() != data.len() {
        blob(&l)
    } else {
        String::new()
    };
    (raw, lf_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// I:internal/gitsha/gitsha_test.go.
    #[test]
    fn known_vectors_and_pair() {
        assert_eq!(blob(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(blob(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
        let (raw, l) = pair(b"hello\r\n");
        assert_eq!(l, blob(b"hello\n"));
        assert_ne!(raw, l);
        assert_eq!(pair(b"hello\n").1, "");
    }
}
