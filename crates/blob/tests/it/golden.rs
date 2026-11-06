//! Replays of testdata/golden/blob/ (tools/goldens/blob_golden_test.go).

use std::path::{Path, PathBuf};

use base64::Engine;
use blob::{
    FileRef, MAX_SIZE, RECEIVED_DIR, Store, check_attach_path, detect_content_type, never_leaves,
    safe_name, truncate, within,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn gold_dir() -> PathBuf {
    manifest_dir().join("../../testdata/golden/blob")
}

fn golden() -> Value {
    serde_json::from_slice(&std::fs::read(gold_dir().join("blob.json")).expect("golden file"))
        .expect("golden JSON")
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap()
}

fn b64(v: &Value) -> Vec<u8> {
    match v {
        Value::Null => vec![],
        v => base64::engine::general_purpose::STANDARD
            .decode(v.as_str().unwrap())
            .unwrap(),
    }
}

#[test]
fn safe_name_matches_go() {
    for c in golden()["safeName"].as_array().unwrap() {
        let (n, sha) = (str_of(c, "name"), str_of(c, "sha"));
        assert_eq!(safe_name(n, sha), str_of(c, "out"), "SafeName({n:?})");
    }
}

#[test]
fn never_leaves_matches_go() {
    for c in golden()["neverLeaves"].as_array().unwrap() {
        let p = str_of(c, "in");
        assert_eq!(
            never_leaves(p).unwrap_or_default(),
            str_of(c, "out"),
            "neverLeaves({p:?})"
        );
    }
}

#[test]
fn within_matches_go() {
    if !cfg!(windows) {
        return; // Go ran these with Windows path rules
    }
    for c in golden()["within"].as_array().unwrap() {
        let (p, base) = (str_of(c, "p"), str_of(c, "base"));
        assert_eq!(
            within(p, base),
            c["want"].as_bool().unwrap(),
            "Within({p:?}, {base:?})"
        );
    }
}

#[test]
fn describe_matches_go() {
    for c in golden()["describe"].as_array().unwrap() {
        let r: FileRef = serde_json::from_str(str_of(c, "in")).unwrap();
        assert_eq!(r.describe(), str_of(c, "out"), "{r:?}");
        // FileRef marshals as Go's struct does.
        assert_eq!(
            String::from_utf8(gocompat::json::marshal(&r).unwrap()).unwrap(),
            str_of(c, "in")
        );
    }
}

#[test]
fn sniff_matches_go() {
    for c in golden()["sniff"].as_array().unwrap() {
        let head = b64(&c["head"]);
        assert_eq!(detect_content_type(&head), str_of(c, "out"), "{head:?}");
    }
}

#[test]
fn truncation_ids_match_go() {
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    inputs.push(vec![b'a'; MAX_SIZE + 1]);
    let mut v = vec![b'a'; MAX_SIZE - 1];
    v.extend_from_slice("€€€".as_bytes());
    inputs.push(v);
    let mut v = vec![b'a'; MAX_SIZE - 2];
    v.extend_from_slice("😀😀".as_bytes());
    inputs.push(v);
    inputs.push(vec![0x80; MAX_SIZE + 10]);
    let mut v = vec![b'a'; MAX_SIZE - 3];
    v.extend_from_slice(&[0xF0, 0x80, 0x80, 0x80, 0x80]);
    inputs.push(v);
    let g = golden();
    let cuts = g["truncate"].as_array().unwrap();
    assert_eq!(cuts.len(), inputs.len());
    for (d, c) in inputs.iter().zip(cuts) {
        let name = format!("len{}tail{}", d.len(), hex::encode(&d[d.len() - 6..]));
        assert_eq!(name, str_of(c, "in"));
        assert_eq!(
            hex::encode(Sha256::digest(truncate(d))),
            str_of(c, "out"),
            "{name}"
        );
    }
}

/// CheckAttachPath in the same scratch tree Go used, $ROOT for its path.
#[test]
fn check_attach_path_matches_go() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().to_string_lossy().into_owned();
    let data = Path::new(&root).join("irori");
    let work = Path::new(&root).join("work");
    for d in [
        data.clone(),
        data.join(RECEIVED_DIR).join("s1"),
        work.clone(),
    ] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(work.join("report.md"), b"x").unwrap();
    std::fs::write(data.join("irori.db"), b"x").unwrap();
    std::fs::write(data.join(RECEIVED_DIR).join("s1").join("got.txt"), b"x").unwrap();
    let native = |s: &str| {
        let s = s.replace("$ROOT", &root);
        if cfg!(windows) {
            s.replace('/', "\\")
        } else {
            s
        }
    };
    let rel = |s: &str| s.replace(&root, "$ROOT");
    let ds = data.to_string_lossy().into_owned();
    for c in golden()["checkAttachPath"].as_array().unwrap() {
        let (p, cwd) = (str_of(c, "path"), str_of(c, "cwd"));
        let path = if p.trim().is_empty() {
            p.to_string()
        } else {
            native(p)
        };
        let got = check_attach_path(&path, &native(cwd), &ds);
        let (out, err) = match got {
            Ok(o) => (rel(&o), String::new()),
            Err(e) => (String::new(), rel(&e.to_string())),
        };
        if cfg!(windows) {
            assert_eq!(out, str_of(c, "out"), "CheckAttachPath({p:?}, {cwd:?})");
            assert_eq!(err, str_of(c, "err"), "CheckAttachPath({p:?}, {cwd:?})");
        } else {
            // The golden is Go on Windows; elsewhere only success agrees.
            assert_eq!(out.is_empty(), str_of(c, "out").is_empty(), "{p:?}");
        }
    }
}

/// A store Go wrote reads back here, byte for byte.
#[test]
fn reads_a_store_go_wrote() {
    let t = tempfile::tempdir().unwrap();
    // Copy so the test never writes into testdata.
    let src = gold_dir().join("store");
    let dst = t.path().join("store");
    copy_dir(&src, &dst);
    let s = Store::open(&dst).unwrap();
    let g = golden();
    let stored = g["stored"].as_array().unwrap();
    assert!(!stored.is_empty());
    for c in stored {
        let id = str_of(c, "id");
        let data = b64(&c["data"]);
        assert!(s.has(id));
        assert_eq!(s.get(id).unwrap(), data, "{id}");
        // And the Rust Store gives the same id for the same content.
        let fresh = Store::open(t.path().join("fresh")).unwrap();
        assert_eq!(fresh.put(&data).unwrap(), id);
    }
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let to = dst.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), to).unwrap();
        }
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
