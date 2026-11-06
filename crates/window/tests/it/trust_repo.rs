//! Settings › Agents' Trust… (card #456, sa-6): the second path that
//! writes Claude Code trust. It trusts only a repo this device has a
//! checkout of, for its main root, leaves the rest of `.claude.json` alone,
//! and writes nothing for an unknown or already-trusted repo.

use crate::common;

use common::*;
use irori_window::claude_trust;

#[test]
fn trust_repo_records_the_listed_root_only_and_nothing_else() {
    let r = Rig::new();
    let w = &r.w;

    // Two checkouts: api (not trusted, with a subfolder seen first) and
    // web (trusted already).
    let dirs = tempfile::tempdir().unwrap();
    let base = dirs
        .path()
        .canonicalize()
        .map(irori_window::spawn::plain)
        .unwrap();
    let api = base.join("api");
    let web = base.join("web");
    for p in [&api, &web] {
        std::fs::create_dir_all(p.join(".git")).unwrap();
    }
    std::fs::create_dir_all(api.join("server")).unwrap();
    let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
    for (key, root) in [
        ("remote:x/api", s(&api.join("server"))),
        ("remote:x/web", s(&web)),
    ] {
        w.core
            .capture()
            .emit_row(
                "irori",
                "repo.seen",
                &serde_json::json!({
                    "repoKey": key,
                    "root": root,
                    "at": "2026-11-04T10:00:00Z",
                }),
            )
            .unwrap();
    }
    let file = base.join(".claude.json");
    let cfg = serde_json::json!({
        "numStartups": 3,
        "projects": { s(&web): { "hasTrustDialogAccepted": true } },
    });
    std::fs::write(&file, cfg.to_string()).unwrap();
    wait("capture lists both repos", || {
        w.trusted_repos_in(&file).len() == 2
    });
    let before = std::fs::read(&file).unwrap();

    // An unknown repo is refused, and nothing is written.
    assert!(w.trust_repo_in(&file, "remote:x/elsewhere").is_err());
    assert_eq!(std::fs::read(&file).unwrap(), before);

    // An already-trusted repo is left as it is.
    let rows = w.trust_repo_in(&file, "remote:x/web").unwrap();
    assert!(
        rows.iter()
            .any(|r| r.repo_key == "remote:x/web" && r.trusted)
    );
    assert_eq!(std::fs::read(&file).unwrap(), before, "no rewrite");

    // Trust… on api: recorded for its root, not the subfolder seen.
    let rows = w.trust_repo_in(&file, "remote:x/api").unwrap();
    let row = rows.iter().find(|r| r.repo_key == "remote:x/api").unwrap();
    assert_eq!((row.folder.as_str(), row.trusted), (s(&api).as_str(), true));
    assert!(claude_trust::is_trusted(&file, &s(&api)));
    assert!(!claude_trust::is_trusted(&file, &s(&api.join("server"))));
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(v["numStartups"], 3, "nothing else in the file changed");
    assert_eq!(v["projects"][s(&web)]["hasTrustDialogAccepted"], true);
}
