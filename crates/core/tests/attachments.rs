//! Port of I:internal/core/attachments_test.go
//! (TestAttachmentsTravelWithMessagesAndComments) on one Core: the
//! never-leave refusals, a file on a message fetched by the agent it went
//! to, the default folder and a second fetch, the refusals of attachment_get,
//! and a file on a Board comment. The cross-device half (another device
//! fetches the file over the mesh) is in pair.rs, where two Cores pair.
//! TestAttachmentDstFollowsALinkAboveAMissingFolder is in
//! `irori_core::attachments`' unit tests.

mod common;

use std::path::Path;

use common::Rig;
use inbox::Sender;
use irori_core::attachments::NO_ATTACHMENT;

fn sha_of(out: &str) -> Option<String> {
    let i = out.find("sha256=")? + "sha256=".len();
    let h = out.get(i..i + 64)?;
    h.bytes()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        .then(|| h.to_string())
}

fn link_file(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(target, link)
    }
}

#[test]
fn attachments_travel_with_messages_and_comments() {
    let r = Rig::new();
    let caller = |key: &str, name: &str| tools::Caller {
        sender: Sender::new(key, name, "claude"),
        path: inbox::path::MOD.into(),
    };
    let (a, b) = (caller(&r.key_a, "alpha"), caller(&r.key_b, "beta"));
    let tool = |c: &tools::Caller, name: &str, args: serde_json::Value| -> String {
        r.core
            .tools()
            .call(c, name, &serde_json::to_vec(&args).unwrap())
            .unwrap()
    };
    let files = tempfile::tempdir().unwrap();
    let data: Vec<u8> = (0..(1u32 << 20) + 17)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    let src = files.path().join("plan.pdf");
    std::fs::write(&src, &data).unwrap();

    // The data dir never leaves, even through a link.
    let out = tool(
        &a,
        "send",
        serde_json::json!({"to": r.key_b, "text": "keys", "attach": [r.data.join("irori.db")]}),
    );
    assert!(
        out.starts_with("Error: ") && out.contains("data dir"),
        "attaching the log = {out}"
    );
    let link = files.path().join("notes.txt");
    if link_file(&r.data.join("device.json"), &link).is_ok() {
        let out = tool(
            &a,
            "send",
            serde_json::json!({"to": r.key_b, "text": "x", "attach": [link]}),
        );
        assert!(
            out.starts_with("Error: "),
            "a link into the data dir = {out}"
        );
    }

    let out = tool(
        &a,
        "send",
        serde_json::json!({"to": r.key_b, "text": "the plan, attached", "attach": [src]}),
    );
    let sha = sha_of(&out).unwrap_or_else(|| panic!("send = {out}"));
    assert!(out.starts_with("Sent message"), "send = {out}");
    assert!(r.core.files().has(&sha), "not in the attachment store");

    // B's wake names the file; B fetches it (it is here already).
    let w = r
        .core
        .inbox()
        .next_wake(&r.key_b, inbox::path::MOD)
        .unwrap()
        .expect("b has the message");
    assert!(
        w.prompt.contains(&format!(
            "<irori-attachment sha256=\"{sha}\" name=\"plan.pdf\""
        )),
        "the wake doesn't name the file:\n{}",
        w.prompt
    );
    let got = tool(&b, "attachment_get", serde_json::json!({"sha256": sha}));
    assert!(
        got.starts_with("Saved the attachment to "),
        "attachment_get = {got}"
    );
    assert!(!got.contains("fetched"), "it was here: {got}");
    let path = got
        .trim_start_matches("Saved the attachment to ")
        .split(". Its contents")
        .next()
        .unwrap()
        .to_string();
    assert_eq!(std::fs::read(&path).unwrap(), data, "b's copy at {path}");
    let received = r.data.join(blob::RECEIVED_DIR);
    assert!(
        blob::within(&path, &received.to_string_lossy())
            && Path::new(&path).file_name().unwrap() == "plan.pdf",
        "saved at {path}"
    );
    // Asked again it lands on the same file.
    let again = tool(
        &b,
        "attachment_get",
        serde_json::json!({"sha256": sha.to_uppercase()}),
    );
    assert!(again.contains(&path), "second get = {again}");

    // It never writes into the data dir, and an agent that isn't on the
    // message can't fetch it by its hash.
    let out = tool(
        &b,
        "attachment_get",
        serde_json::json!({"sha256": sha, "to": r.data.join("irori.json")}),
    );
    assert!(
        out.starts_with("Error: ") && out.contains("data dir"),
        "to inside the data dir = {out}"
    );
    let stranger = format!("{}/someone-else", r.core.device_id());
    assert_eq!(
        r.core.get_attachment(&stranger, &sha, "").unwrap_err(),
        NO_ATTACHMENT
    );
    // The sender reads its own message; an existing other file at `to` is
    // never overwritten.
    let taken = files.path().join("taken.pdf");
    std::fs::write(&taken, b"mine").unwrap();
    let e = r
        .core
        .get_attachment(&r.key_a, &sha, &taken.to_string_lossy())
        .unwrap_err();
    assert!(e.ends_with("already exists; give another to"), "{e}");
    assert_eq!(std::fs::read(&taken).unwrap(), b"mine");
    // A relative `to` is taken against the reader's working directory.
    let out = tool(
        &a,
        "attachment_get",
        serde_json::json!({"sha256": sha, "to": "copy.pdf"}),
    );
    let cwd = r
        .core
        .local_agents()
        .into_iter()
        .find(|c| c.key == r.key_a)
        .unwrap()
        .cwd;
    let rel = Path::new(&cwd).join("copy.pdf");
    assert!(
        out.starts_with("Saved the attachment to ") && rel.exists(),
        "relative to = {out}"
    );
    // A bad hash and too many files.
    assert!(
        tool(&b, "attachment_get", serde_json::json!({"sha256": "abc"}))
            .starts_with("Error: give sha256")
    );
    let many: Vec<String> = (0..=inbox::MAX_ATTACHMENTS)
        .map(|_| src.to_string_lossy().into_owned())
        .collect();
    let e = r.core.attach_files(&r.key_a, &many).unwrap_err();
    assert_eq!(
        e.to_string(),
        format!("at most {} attachments", inbox::MAX_ATTACHMENTS)
    );

    // A file on a comment: A comments on a card, and the person fetches it
    // into a folder.
    let out = tool(
        &a,
        "board_create",
        serde_json::json!({"title": "ship the pdf"}),
    );
    let id = r.core.coord().list_tasks(&coord::TaskFilter::default())[0]
        .id
        .clone();
    assert!(!out.starts_with("Error: "), "create = {out}");
    let note = files.path().join("review.txt");
    std::fs::write(&note, b"looks good\n").unwrap();
    let out = tool(
        &a,
        "board_comment",
        serde_json::json!({"task_id": id, "text": "review attached", "attach": [note]}),
    );
    let csha = sha_of(&out).unwrap_or_else(|| panic!("board_comment = {out}"));
    assert!(
        out.starts_with("Commented comment:"),
        "board_comment = {out}"
    );
    let dst = files.path().join("in");
    std::fs::create_dir_all(&dst).unwrap();
    let (p, from) = r
        .core
        .get_attachment(&r.core.user_sender().key, &csha, &dst.to_string_lossy())
        .unwrap();
    assert_eq!(from, "");
    assert_eq!(std::fs::read(&p).unwrap(), b"looks good\n");
    assert_eq!(Path::new(&p).file_name().unwrap(), "review.txt");
}
