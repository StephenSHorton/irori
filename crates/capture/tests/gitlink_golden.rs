//! gitlink, repo and gitsha against vectors Go produced
//! (testdata/golden/capture/gitlink.json and repo.json, exported by
//! tools/goldens/export.sh from gitlink_golden_test.go and
//! repo_golden_test.go).

use capture::gitlink::{
    self, Commit, CommitCmd, EditRef, FileChange, is_commit_command, parse_raw_z, parse_reflog,
    split_commands, under_root,
};
use capture::repo;
use gocompat::time::GoTime;
use serde_json::Value as J;

fn golden(name: &str) -> J {
    let p = format!(
        "{}/../../testdata/golden/capture/{name}",
        manifest_dir().display()
    );
    serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap()
}

fn s(v: &J) -> &str {
    v.as_str().unwrap_or("")
}

fn t(v: &J) -> GoTime {
    GoTime::parse_rfc3339(s(v)).unwrap_or_else(|| panic!("time {v}"))
}

fn gj<T: serde::Serialize>(v: &T) -> J {
    serde_json::from_slice(&gocompat::json::marshal(v).unwrap()).unwrap()
}

#[test]
fn is_commit_command_matches_go() {
    let g = golden("gitlink.json");
    let cases = g["isCommitCommand"].as_array().unwrap();
    assert!(cases.len() > 100, "{}", cases.len());
    for c in cases {
        assert_eq!(
            is_commit_command(s(&c["Cmd"])),
            c["Commit"].as_bool().unwrap(),
            "{:?}",
            s(&c["Cmd"])
        );
    }
}

#[test]
fn split_commands_matches_go() {
    let g = golden("gitlink.json");
    for c in g["splitCommands"].as_array().unwrap() {
        let got = serde_json::to_value(split_commands(s(&c["in"]))).unwrap();
        let want = if c["out"].is_null() {
            J::Array(vec![])
        } else {
            c["out"].clone()
        };
        assert_eq!(got, want, "{:?}", s(&c["in"]));
    }
}

#[test]
fn parse_reflog_matches_go() {
    let g = golden("gitlink.json");
    for c in g["parseReflog"].as_array().unwrap() {
        let (es, n) = parse_reflog(s(&c["data"]).as_bytes());
        assert_eq!(n as u64, c["consumed"].as_u64().unwrap(), "{c}");
        assert_eq!(gj(&es), c["entries"], "{c}");
    }
}

#[test]
fn parse_raw_z_matches_go() {
    let g = golden("gitlink.json");
    for c in g["parseRawZ"].as_array().unwrap() {
        match parse_raw_z(s(&c["in"])) {
            Ok(fs) => {
                assert_eq!(s(&c["err"]), "", "{c}");
                assert_eq!(gj(&fs), c["files"], "{c}");
            }
            Err(e) => assert_eq!(format!("{e}"), s(&c["err"]), "{c}"),
        }
    }
}

#[test]
fn under_root_matches_go() {
    let g = golden("gitlink.json");
    for c in g["underRoot"].as_array().unwrap() {
        assert_eq!(
            under_root(s(&c["dir"]), s(&c["root"])),
            c["under"].as_bool().unwrap(),
            "{c}"
        );
    }
}

fn commit_of(v: &J) -> Commit {
    Commit {
        sha: s(&v["sha"]).into(),
        parents: Vec::new(),
        at: t(&v["at"]),
        author: s(&v["author"]).into(),
        subject: s(&v["subject"]).into(),
        files: Some(
            v["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| FileChange {
                    path: s(&f["path"]).into(),
                    blob: s(&f["blob"]).into(),
                    status: s(&f["status"]).into(),
                })
                .collect(),
        ),
        files_omitted: 0,
    }
}

#[test]
fn classify_and_committer_match_go() {
    let g = golden("gitlink.json");
    let cases = g["classify"].as_array().unwrap();
    assert_eq!(cases.len(), 300);
    for c in cases {
        let commit = commit_of(&c["commit"]);
        let cands: Vec<EditRef> = c["cands"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|e| EditRef {
                        id: s(&e["ID"]).into(),
                        root: s(&e["Root"]).into(),
                        path: s(&e["Path"]).into(),
                        after_blob: s(&e["AfterBlob"]).into(),
                        after_blob_lf: s(&e["AfterBlobLF"]).into(),
                        heuristic: e["Heuristic"].as_bool().unwrap(),
                        at: t(&e["At"]),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let cmds: Vec<CommitCmd> = c["cmds"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|m| CommitCmd {
                        key: s(&m["Key"]).into(),
                        turn_id: s(&m["TurnID"]).into(),
                        cwd: s(&m["Cwd"]).into(),
                        at: t(&m["At"]),
                        start: t(&m["Start"]),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let root = s(&c["root"]);
        let links = gitlink::classify(&commit, root, &cands);
        assert_eq!(gj(&links), c["links"], "{c}");
        let who = gitlink::committer(&commit, root, &cmds);
        match (&who, &c["committer"]) {
            (None, J::Null) => {}
            (Some(w), want) => {
                assert_eq!(w.key, s(&want["Key"]), "{c}");
                assert_eq!(w.turn_id, s(&want["TurnID"]), "{c}");
            }
            (None, want) => panic!("no committer, Go had {want}: {c}"),
        }
    }
}

#[test]
fn normalize_remote_matches_go() {
    let g = golden("repo.json");
    let cases = g["normalizeRemote"].as_array().unwrap();
    assert!(cases.len() > 100);
    for c in cases {
        assert_eq!(
            repo::normalize_remote(s(&c["In"])),
            s(&c["Out"]),
            "{:?}",
            s(&c["In"])
        );
    }
    for c in g["smallestSHA"].as_array().unwrap() {
        assert_eq!(repo::smallest_sha(s(&c["In"])), s(&c["Out"]));
    }
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
