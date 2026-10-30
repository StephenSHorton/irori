//! The I:internal/api/v03_test.go cases the golden scenario cannot make:
//! success shapes that need a blob or a commit in place (TestV03Blobs,
//! TestV03Commits, TestV03PRsAndStatus' capture/status) and
//! TestV03WhyUnknownRepoIsReadOnly's "nothing was added" check, on the
//! real Core behind the API.

mod common;

use std::path::Path;
use std::time::Duration;

use irori_api::client::Response;

fn get(rig: &common::Rig, path: &str) -> Response {
    rig.rt
        .block_on(rig.client.raw("GET", path, &[], None))
        .unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn json_error(r: &Response) -> String {
    let v: serde_json::Value = serde_json::from_slice(&r.body).expect("a JSON error body");
    v["error"].as_str().unwrap_or("").to_string()
}

#[test]
fn blob_found_and_absent() {
    let rig = common::Rig::new();
    let text = "--- a/a.txt\n+++ b/a.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n";
    let id = blob::Store::open(rig.data.join("blobs"))
        .unwrap()
        .put(text.as_bytes())
        .unwrap();
    let r = get(&rig, &format!("/v1/blobs/{id}"));
    assert_eq!(
        (r.status, String::from_utf8_lossy(&r.body).as_ref()),
        (200, text)
    );
    for (k, want) in [
        ("Content-Type", "text/plain; charset=utf-8"),
        ("X-Content-Type-Options", "nosniff"),
        ("Cache-Control", "no-store"),
    ] {
        assert_eq!(r.header(k), Some(want), "{k}");
    }
    let r = get(&rig, &format!("/v1/blobs/{}", "0".repeat(64)));
    assert_eq!(r.status, 404);
    assert!(!json_error(&r).is_empty());
}

#[test]
fn commit_found_by_sha_and_prefix() {
    let sha = "3f2a9c41d0b7e65a8c1f4e2d9b0a7c6e5d4f3a21";
    let row = capture::CommitRow {
        repo_key: "repo-key-1".into(),
        root: "C:/work/repo".into(),
        op: "commit".into(),
        branch: "main".into(),
        commit: capture::gitlink::Commit {
            sha: sha.into(),
            parents: vec![],
            at: gocompat::time::GoTime::unix(1_792_490_400, 0),
            author: "Test".into(),
            subject: "add a".into(),
            files: Some(vec![capture::gitlink::FileChange {
                path: "a.txt".into(),
                blob: "0".repeat(40),
                status: "A".into(),
            }]),
            files_omitted: 0,
        },
    };
    // Capture folds commit rows on replay, as Go's fixture writes them
    // before capture.New.
    let rig = common::Rig::seeded(|c| {
        c.store()
            .append("irori", capture::KIND_COMMIT_SEEN, &row)
            .unwrap();
    });
    let r = get(&rig, &format!("/v1/commits/{sha}"));
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.header("Content-Type"), Some("application/json"));
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert!(
        v["sha"] == sha && v["repoKey"] == "repo-key-1" && v["subject"] == "add a",
        "{v}"
    );
    // A prefix, any case.
    let r = get(&rig, &format!("/v1/commits/{}", sha[..7].to_uppercase()));
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!((r.status, v["sha"].as_str()), (200, Some(sha)));
    let r = get(&rig, "/v1/commits/0000000");
    assert_eq!(r.status, 404);
    assert!(!json_error(&r).is_empty());
}

#[test]
fn capture_status_shape() {
    let rig = common::Rig::new();
    let r = get(&rig, "/v1/capture/status");
    assert_eq!(r.status, 200);
    assert_eq!(r.header("Content-Type"), Some("application/json"));
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    keys.sort();
    // Go capture.Status, lastError omitted when empty.
    assert_eq!(
        keys,
        [
            "backfilling",
            "backlog",
            "dropped",
            "files",
            "lastScan",
            "repos"
        ],
        "{v}"
    );
    let mut dropped: Vec<&str> = v["dropped"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    dropped.sort();
    assert_eq!(
        dropped,
        ["artifact", "ignored", "outsideRepo", "overCap", "unchanged"]
    );
    // Go writes the Status in its field order.
    let text = String::from_utf8_lossy(&r.body);
    assert!(
        text.starts_with("{\"files\":") && text.contains("\"backlog\":") && text.ends_with("}\n"),
        "{text}"
    );
}

/// GET /v1/why is a read: a git repo capture has never seen is 404, and
/// the query adds no repo (no repo.seen row, no reflog watch).
#[test]
fn why_unknown_repo_is_read_only() {
    let rig = common::Rig::new();
    let repo_rows = |rig: &common::Rig| {
        rig.core
            .store()
            .list(&store::Filter {
                kinds: vec![capture::KIND_REPO_SEEN.into()],
                limit: 1000,
                ..Default::default()
            })
            .unwrap()
            .len()
    };
    let (before, rows_before) = (rig.core.capture().repos(), repo_rows(&rig));

    let other = tempfile::tempdir().unwrap();
    let o = other.path();
    git(o, &["init", "-q"]);
    git(o, &["config", "core.autocrlf", "false"]);
    std::fs::write(o.join("b.txt"), "b\n").unwrap();
    git(o, &["add", "b.txt"]);
    git(o, &["commit", "-q", "-m", "add b"]);
    let q = irori_api::client::query_escape;
    let abs = o.join("b.txt").to_string_lossy().into_owned();
    let cwd = o.to_string_lossy().into_owned();
    for path in [
        format!("/v1/why?line=1&path={}", q(&abs)),
        format!("/v1/why?path=b.txt&cwd={}", q(&cwd)),
    ] {
        let r = get(&rig, &path);
        assert_eq!(
            r.status,
            404,
            "{path}: {}",
            String::from_utf8_lossy(&r.body)
        );
        assert!(!json_error(&r).is_empty());
    }
    // A repo with no commits (no repo key) is not a known repo either.
    let empty = tempfile::tempdir().unwrap();
    git(empty.path(), &["init", "-q"]);
    let r = get(
        &rig,
        &format!(
            "/v1/why?path=c.txt&cwd={}",
            q(&empty.path().to_string_lossy())
        ),
    );
    assert_eq!(r.status, 404);

    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(rig.core.capture().repos(), before, "why added repos");
    assert_eq!(repo_rows(&rig), rows_before, "why appended a repo.seen row");
}
