//! Ports of I:internal/blob/files_test.go and motw_windows_test.go.

use std::fs;
use std::io::Write;
use std::path::Path;

use blob::{
    ErrorKind, Files, RECEIVED_DIR, check_attach_path, mark_downloaded, safe_name, same_content,
};
use sha2::{Digest, Sha256};

fn sha(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn new_files(t: &tempfile::TempDir) -> Files {
    Files::open(t.path().join("files")).unwrap()
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn files_put_resume_and_finish() {
    let t = tempfile::tempdir().unwrap();
    let f = new_files(&t);
    let src = t.path().join("notes.txt");
    let data = "irori attachment\n".repeat(4000).into_bytes();
    fs::write(&src, &data).unwrap();
    let r = f.put_file(&s(&src)).unwrap();
    assert_eq!(r.sha256, sha(&data));
    assert_eq!(r.size, data.len() as i64);
    assert_eq!(r.name, "notes.txt");
    assert!(r.mime.starts_with("text/plain"), "{r:?}");
    // Stored as is, uncompressed.
    assert_eq!(fs::read(f.path(&r.sha256)).unwrap(), data);
    assert_eq!(f.put_file(&s(&src)).unwrap().sha256, r.sha256);

    // A second store fetches it in two pieces, resuming by offset.
    let t2 = tempfile::tempdir().unwrap();
    let g = new_files(&t2);
    let (mut p, off) = g.partial(&r.sha256).unwrap();
    assert_eq!(off, 0);
    p.write_all(&data[..1000]).unwrap();
    drop(p);
    let e = g.finish(&r.sha256, r.size as u64).unwrap_err();
    assert!(e.is(ErrorKind::Mismatch), "{e}");
    assert_eq!(
        e.to_string(),
        format!(
            "blob: the bytes don't match the attachment's length and hash: 1000 of {} bytes",
            data.len()
        )
    );
    let (mut p, off) = g.partial(&r.sha256).unwrap();
    assert_eq!(off, 1000);
    p.write_all(&data[1000..]).unwrap();
    drop(p);
    g.finish(&r.sha256, r.size as u64).unwrap();
    assert!(g.has(&r.sha256));

    // A wrong body is dropped, not kept to resume.
    let t3 = tempfile::tempdir().unwrap();
    let h = new_files(&t3);
    let (mut p, _) = h.partial(&r.sha256).unwrap();
    p.write_all(&vec![b'x'; data.len()]).unwrap();
    drop(p);
    assert!(
        h.finish(&r.sha256, r.size as u64)
            .unwrap_err()
            .is(ErrorKind::Mismatch)
    );
    let (p, off) = h.partial(&r.sha256).unwrap();
    drop(p);
    assert_eq!(off, 0, "wrong partial kept");

    // copy_out never overwrites.
    let dst = t.path().join("out").join("notes.txt");
    g.copy_out(&r.sha256, &dst).unwrap();
    assert!(same_content(&dst, &r.sha256));
    assert!(
        g.copy_out(&r.sha256, &dst).is_err(),
        "copy_out overwrote a file"
    );
    assert_eq!(g.size(&r.sha256).unwrap(), data.len() as u64);
    assert!(g.open_file(&r.sha256).is_ok());
    g.drop_partial(&r.sha256);
}

#[test]
fn files_cap() {
    let t = tempfile::tempdir().unwrap();
    let f = Files::open_with_max(t.path().join("files"), 1 << 10).unwrap();
    let src = t.path().join("big.bin");
    fs::write(&src, vec![0u8; (1 << 10) + 1]).unwrap();
    let e = f.put_file(&s(&src)).unwrap_err();
    assert!(e.is(ErrorKind::TooLarge), "{e}");
    assert_eq!(
        e.to_string(),
        "blob: the file is larger than the attachment cap (1025 bytes, at most 1024)"
    );
    fs::write(&src, vec![0u8; 1 << 10]).unwrap();
    f.put_file(&s(&src)).expect("at the cap");
}

fn same_file(a: &str, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

#[test]
fn check_attach_path_rules() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let data = root.join("irori");
    let work = root.join("work");
    for d in [
        data.clone(),
        data.join(RECEIVED_DIR).join("s1"),
        work.clone(),
        root.join(".ssh"),
    ] {
        fs::create_dir_all(d).unwrap();
    }
    let write = |p: &Path| {
        fs::write(p, b"x").unwrap();
        p.to_path_buf()
    };
    let ok = write(&work.join("report.md"));
    write(&data.join("irori.db"));
    let recv = write(&data.join(RECEIVED_DIR).join("s1").join("got.txt"));
    write(&work.join(".env.local"));
    write(&work.join("server.pem"));
    write(&root.join(".ssh").join("config"));
    write(&work.join(".git-credentials"));
    let ds = s(&data);

    let got = check_attach_path(&s(&ok), "", &ds).unwrap();
    assert!(same_file(&got, &ok), "plain file = {got}");
    let got = check_attach_path("report.md", &s(&work), &ds).unwrap();
    assert!(same_file(&got, &ok), "relative to cwd = {got}");
    let e = check_attach_path("report.md", "", &ds).unwrap_err();
    assert_eq!(
        e.to_string(),
        "blob: \"report.md\" is relative and your working directory is unknown: give an absolute path"
    );
    check_attach_path(&s(&recv), "", &ds).expect("a received file can be passed on");
    assert!(
        check_attach_path(&s(&work), "", &ds)
            .unwrap_err()
            .is(ErrorKind::NotFile)
    );
    for p in [
        data.join("irori.db"),
        data.join("..").join("irori").join("irori.db"),
        work.join(".env.local"),
        work.join("server.pem"),
        root.join(".ssh").join("config"),
        work.join(".git-credentials"),
    ] {
        let e = check_attach_path(&s(&p), "", &ds).unwrap_err();
        assert!(e.is(ErrorKind::NeverLeaves), "{p:?} = {e}");
    }
    if cfg!(windows) {
        let upper = s(&data.join("irori.db")).to_uppercase();
        let e = check_attach_path(&upper, "", &ds).unwrap_err();
        assert!(
            e.is(ErrorKind::NeverLeaves),
            "data dir in another case = {e}"
        );
    }

    // A symlink into the data dir is refused by where it resolves to.
    let link = work.join("innocent.txt");
    if symlink_file(&data.join("irori.db"), &link).is_err() {
        eprintln!("no symlinks here; the link cases are skipped");
        return;
    }
    let e = check_attach_path(&s(&link), "", &ds).unwrap_err();
    assert!(
        e.is(ErrorKind::NeverLeaves),
        "symlink into the data dir = {e}"
    );
    let dir_link = work.join("stuff");
    if symlink_dir(&data, &dir_link).is_ok() {
        let e = check_attach_path(&s(&dir_link.join("irori.db")), "", &ds).unwrap_err();
        assert!(
            e.is(ErrorKind::NeverLeaves),
            "through a linked folder = {e}"
        );
    }
    let key_link = work.join("notes2.txt");
    if symlink_file(&work.join("server.pem"), &key_link).is_ok() {
        let e = check_attach_path(&s(&key_link), "", &ds).unwrap_err();
        assert!(e.is(ErrorKind::NeverLeaves), "link to a key = {e}");
    }
}

#[cfg(windows)]
fn symlink_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(src, dst)
}
#[cfg(windows)]
fn symlink_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(src, dst)
}
#[cfg(unix)]
fn symlink_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}
#[cfg(unix)]
fn symlink_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[test]
fn safe_name_table() {
    let id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    for (input, want) in [
        ("report.pdf", "report.pdf"),
        ("../../etc/passwd", "passwd"),
        (r"..\..\Windows\win.ini", "win.ini"),
        ("a/b/c.txt", "c.txt"),
        ("C:evil.txt", "evil.txt"),
        (r"C:\Users\x\evil.txt", "evil.txt"),
        ("file.txt:Zone.Identifier", "file.txt"),
        ("..", "attachment-0123456789ab"),
        ("", "attachment-0123456789ab"),
        ("...   ", "attachment-0123456789ab"),
        ("/", "attachment-0123456789ab"),
        ("CON", "_CON"),
        ("con.txt", "_con.txt"),
        ("Nul.tar.gz", "_Nul.tar.gz"),
        ("COM1", "_COM1"),
        ("lpt9.log", "_lpt9.log"),
        ("COM10", "COM10"),
        ("console.txt", "console.txt"),
        ("trail. . ", "trail"),
        ("a\x00b\nc.txt", "a_b_c.txt"),
        (r#"what?<>|*".txt"#, "what______.txt"),
        (".hidden", "hidden"),
        // A right-to-left override would show invoiceexe.jpg in Explorer.
        ("invoice\u{202e}gpj.exe", "invoice_gpj.exe"),
        ("a\u{2066}b\u{2069}\u{200b}\u{feff}.png", "a_b___.png"),
    ] {
        assert_eq!(safe_name(input, id), want, "SafeName({input:?})");
    }
    let long = format!("{}.txt", "a".repeat(300));
    let got = safe_name(&long, id);
    assert_eq!(got.chars().count(), 120);
    assert!(got.ends_with(".txt"));
}

#[cfg(windows)]
#[test]
fn mark_downloaded_writes_zone_identifier() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("shot.png");
    fs::write(&p, b"x").unwrap();
    mark_downloaded(&p).unwrap();
    let mut ads = p.as_os_str().to_owned();
    ads.push(":Zone.Identifier");
    let b = fs::read(ads).unwrap();
    assert!(String::from_utf8_lossy(&b).contains("ZoneId=3"));
    assert_eq!(fs::read(&p).unwrap(), b"x");
}

#[cfg(not(windows))]
#[test]
fn mark_downloaded_runs() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("shot.png");
    fs::write(&p, b"x").unwrap();
    mark_downloaded(&p).unwrap();
}
