//! Ports of I:internal/blob/store_test.go.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use blob::{ErrorKind, MAX_SIZE, Store, TRUNCATED_MARKER, valid_id};
use sha2::{Digest, Sha256};

fn sha(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn new_store() -> (Store, PathBuf, tempfile::TempDir) {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("blobs");
    (Store::open(&dir).unwrap(), dir, t)
}

fn gz(b: &[u8]) -> Vec<u8> {
    let mut w = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    w.write_all(b).unwrap();
    w.finish().unwrap()
}

fn assert_no_temps(dir: &Path) {
    for e in walk(dir) {
        let name = e.file_name().unwrap().to_string_lossy().to_string();
        assert!(!name.contains(".tmp-"), "temp file left behind: {e:?}");
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            }
            out.push(p);
        }
    }
    out
}

#[test]
fn open_creates_dir() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("a").join("b").join("blobs");
    Store::open(&dir).unwrap();
    assert!(dir.is_dir());
    Store::open(&dir).unwrap();
    assert!(Store::open("").is_err());
}

#[test]
fn put_get_round_trip() {
    let (s, dir, _t) = new_store();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        (
            "text",
            b"--- a/x.go\n+++ b/x.go\n@@ -1 +1 @@\n-a\n+b\n".to_vec(),
        ),
        ("crlf", b"line one\r\nline two\r\n".to_vec()),
        ("binary", vec![0, 1, 2, 0xff, 0xfe, 0x80]),
        ("unicode", "囲炉裏 summary ✓\n".as_bytes().to_vec()),
        ("exactly max", vec![b'x'; MAX_SIZE]),
    ];
    for (name, data) in cases {
        let id = s.put(&data).unwrap();
        assert_eq!(id, sha(&data), "{name}");
        assert!(s.has(&id), "{name}");
        assert!(dir.join(&id[..2]).join(&id).is_file(), "{name}");
        assert_eq!(s.get(&id).unwrap(), data, "{name}");
    }
}

#[test]
fn stored_gzipped() {
    let (s, dir, _t) = new_store();
    let data = "hello blob\n".repeat(1000).into_bytes();
    let id = s.put(&data).unwrap();
    let raw = fs::read(dir.join(&id[..2]).join(&id)).unwrap();
    let mut out = Vec::new();
    use std::io::Read;
    flate2::read::GzDecoder::new(&raw[..])
        .read_to_end(&mut out)
        .expect("file is gzip");
    assert_eq!(out, data);
}

#[test]
fn put_idempotent() {
    let (s, dir, _t) = new_store();
    let a = s.put(b"same content").unwrap();
    let p = dir.join(&a[..2]).join(&a);
    let before = fs::metadata(&p).unwrap().modified().unwrap();
    let b = s.put(b"same content").unwrap();
    assert_eq!(a, b);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), before);
    assert_no_temps(&dir);
}

#[test]
fn distinct_content_distinct_ids() {
    let (s, _, _t) = new_store();
    assert_ne!(s.put(b"a\n").unwrap(), s.put(b"a\r\n").unwrap());
}

#[test]
fn truncation() {
    let mut straddle = vec![b'a'; MAX_SIZE - 1];
    straddle.extend_from_slice("€€€".as_bytes());
    let mut exact = vec![b'a'; MAX_SIZE - 4];
    exact.extend_from_slice("😀tail".as_bytes());
    let cases: Vec<(&str, Vec<u8>, usize)> = vec![
        ("one over", vec![b'b'; MAX_SIZE + 1], MAX_SIZE),
        ("ascii", vec![b'a'; MAX_SIZE + 100], MAX_SIZE),
        ("rune straddles", straddle, MAX_SIZE - 1),
        ("rune ends at max", exact, MAX_SIZE),
        ("invalid utf8", vec![0x80; MAX_SIZE + 10], MAX_SIZE),
    ];
    for (name, data, cut) in cases {
        let (s, _, _t) = new_store();
        let id = s.put(&data).unwrap();
        let got = s.get(&id).unwrap();
        let mut want = data[..cut].to_vec();
        want.extend_from_slice(TRUNCATED_MARKER.as_bytes());
        assert_eq!(got.len(), want.len(), "{name}");
        assert!(got == want, "{name}");
        assert_eq!(id, sha(&want), "{name}");
        if name != "invalid utf8" {
            assert!(std::str::from_utf8(&got).is_ok(), "{name}");
        }
    }
}

#[test]
fn truncated_puts_match() {
    let (s, _, _t) = new_store();
    let mut a = vec![b'z'; MAX_SIZE];
    let mut b = a.clone();
    a.extend_from_slice(b"one");
    b.extend_from_slice(b"two");
    assert_eq!(s.put(&a).unwrap(), s.put(&b).unwrap());
}

#[test]
fn get_bad_id() {
    let (s, dir, _t) = new_store();
    let valid = s.put(b"x").unwrap();
    fs::write(dir.join("evil"), b"x").unwrap();
    let cases = [
        ("empty", String::new()),
        ("short", valid[..63].to_string()),
        ("long", format!("{valid}0")),
        ("uppercase", valid.to_uppercase()),
        ("non-hex", "g".repeat(64)),
        ("traversal slash", format!("../{}", &valid[3..])),
        ("traversal backslash", format!(r"..\{}", &valid[3..])),
        ("absolute windows", format!(r"C:\{}", &valid[3..])),
        ("dots", ".".repeat(64)),
        ("space", format!(" {}", &valid[1..])),
        ("nul", format!("\0{}", &valid[1..])),
        ("sha1 length", valid[..40].to_string()),
    ];
    for (name, id) in cases {
        let e = s.get(&id).unwrap_err();
        assert!(e.is(ErrorKind::BadId), "{name}: {e}");
        assert_eq!(
            e.to_string(),
            "blob: id must be 64 lowercase hex characters"
        );
        assert!(!s.has(&id), "{name}");
    }
}

#[test]
fn get_not_found() {
    let (s, _, _t) = new_store();
    let id = sha(b"never stored");
    let e = s.get(&id).unwrap_err();
    assert!(e.is(ErrorKind::NotFound));
    assert_eq!(e.to_string(), "blob: not found");
    assert!(!s.has(&id));
}

#[test]
fn has_ignores_directory() {
    let (s, dir, _t) = new_store();
    let id = sha(b"dir squatter");
    fs::create_dir_all(dir.join(&id[..2]).join(&id)).unwrap();
    assert!(!s.has(&id));
}

#[test]
fn corrupt_blob() {
    let good = gz(b"original content");
    let mut bad_crc = good[..good.len() - 8].to_vec();
    bad_crc.extend_from_slice(&[0; 8]);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty file", vec![]),
        ("not gzip", b"plain text, not gzip".to_vec()),
        ("truncated gzip", good[..good.len() / 2].to_vec()),
        ("bad crc", bad_crc),
        ("wrong content", gz(b"different content")),
    ];
    for (name, raw) in cases {
        let (s, dir, _t) = new_store();
        let id = sha(b"original content");
        fs::create_dir_all(dir.join(&id[..2])).unwrap();
        fs::write(dir.join(&id[..2]).join(&id), raw).unwrap();
        let e = s.get(&id).expect_err(name);
        assert!(
            !e.is(ErrorKind::NotFound) && !e.is(ErrorKind::BadId),
            "{name}: {e}"
        );
    }
}

#[test]
fn concurrent_put_same_content() {
    let (s, dir, _t) = new_store();
    let s = Arc::new(s);
    let data = Arc::new("shared patch\r\n".repeat(5000).into_bytes());
    let want = sha(&data);
    let start = Arc::new(Barrier::new(64));
    let hs: Vec<_> = (0..64)
        .map(|_| {
            let (s, data, start) = (s.clone(), data.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                s.put(&data)
            })
        })
        .collect();
    for h in hs {
        assert_eq!(h.join().unwrap().unwrap(), want);
    }
    assert_eq!(s.get(&want).unwrap(), *data);
    assert_no_temps(&dir);
}

#[test]
fn concurrent_put_and_get() {
    let (s, _, _t) = new_store();
    let s = Arc::new(s);
    let data: &[u8] = b"read while writing";
    let id = sha(data);
    let hs: Vec<_> = (0..64)
        .map(|i| {
            let (s, id) = (s.clone(), id.clone());
            std::thread::spawn(move || {
                if i % 2 == 0 {
                    s.put(data).map(|_| ())
                } else {
                    match s.get(&id) {
                        Err(e) if e.is(ErrorKind::NotFound) => Ok(()),
                        Ok(got) => {
                            assert_eq!(got, data, "short read");
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap().unwrap();
    }
}

#[test]
fn concurrent_stores() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("blobs");
    let a = Arc::new(Store::open(&dir).unwrap());
    let b = Arc::new(Store::open(&dir).unwrap());
    let data: &[u8] = b"two processes";
    let hs: Vec<_> = (0..32)
        .map(|i| {
            let st = if i % 2 == 1 { b.clone() } else { a.clone() };
            std::thread::spawn(move || st.put(data))
        })
        .collect();
    for h in hs {
        h.join().unwrap().unwrap();
    }
    assert_eq!(b.get(&sha(data)).unwrap(), data);
    assert_no_temps(&dir);
}

/// On Windows an open reader can make a rename onto the final file fail;
/// an existing final file with the same id is success.
#[test]
fn put_while_final_held_open() {
    let (s, dir, _t) = new_store();
    let data: &[u8] = b"held open";
    let id = s.put(data).unwrap();
    let fin = dir.join(&id[..2]).join(&id);
    let _f = fs::File::open(&fin).unwrap();
    let tmp = dir.join(&id[..2]).join(format!("{id}.tmp-test"));
    fs::write(&tmp, fs::read(&fin).unwrap()).unwrap();
    s.place(&tmp, &fin)
        .expect("place onto an open existing blob");
    let _ = fs::remove_file(&tmp);
    assert_eq!(s.get(&id).unwrap(), data);
}

#[test]
fn put_fails_when_dir_is_file() {
    let (s, dir, _t) = new_store();
    let data: &[u8] = b"blocked";
    let id = sha(data);
    fs::write(dir.join(&id[..2]), b"x").unwrap();
    assert!(s.put(data).is_err());
}

#[test]
fn valid_id_table() {
    for (id, want) in [
        ("0".repeat(64), true),
        ("f".repeat(64), true),
        (
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            true,
        ),
        ("F".repeat(64), false),
        (format!("{}A", "0".repeat(63)), false),
        (format!("{}/", "0".repeat(63)), false),
        ("0".repeat(65), false),
        (String::new(), false),
    ] {
        assert_eq!(valid_id(&id), want, "{id:?}");
    }
}

/// Has and Get agree: a directory squatting on the blob path is absent.
#[test]
fn get_directory_is_not_found() {
    let (s, dir, _t) = new_store();
    let id = sha(b"dir squatter");
    fs::create_dir_all(dir.join(&id[..2]).join(&id)).unwrap();
    assert!(s.get(&id).unwrap_err().is(ErrorKind::NotFound));
}

/// A placed blob is never replaced: replacing a live file on Windows makes
/// concurrent readers fail with a sharing violation.
#[test]
fn place_never_replaces() {
    let (s, dir, _t) = new_store();
    let sub = dir.join("ab");
    fs::create_dir_all(&sub).unwrap();
    let fin = sub.join("final");
    let tmp = sub.join("final.tmp-test");
    fs::write(&fin, b"first").unwrap();
    fs::write(&tmp, b"second").unwrap();
    s.place(&tmp, &fin).unwrap();
    assert_eq!(fs::read(&fin).unwrap(), b"first");
}

/// Many rounds of separate Stores (processes) putting and reading a fresh
/// blob at once; every read sees the blob or NotFound, never an error.
#[test]
fn concurrent_put_and_get_stress() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("blobs");
    for r in 0..200 {
        let data = Arc::new(
            format!("round {r} patch line\r\n")
                .repeat(2500)
                .into_bytes(),
        );
        let id = sha(&data);
        let hs: Vec<_> = (0..16)
            .map(|i| {
                let st = Store::open(&dir).unwrap();
                let (data, id) = (data.clone(), id.clone());
                std::thread::spawn(move || -> Result<(), String> {
                    if i % 2 == 0 {
                        return st.put(&data).map(|_| ()).map_err(|e| e.to_string());
                    }
                    for _ in 0..50 {
                        match st.get(&id) {
                            Err(e) if e.is(ErrorKind::NotFound) => {}
                            Ok(got) if got == *data => {}
                            Ok(_) => return Err("wrong content".into()),
                            Err(e) => return Err(e.to_string()),
                        }
                    }
                    if st.has(&id) {
                        match st.get(&id) {
                            Ok(got) if got == *data => {}
                            Ok(_) => return Err("wrong content".into()),
                            Err(e) => return Err(e.to_string()),
                        }
                    }
                    Ok(())
                })
            })
            .collect();
        for h in hs {
            h.join()
                .unwrap()
                .unwrap_or_else(|e| panic!("round {r}: {e}"));
        }
    }
    assert_no_temps(&dir);
}
