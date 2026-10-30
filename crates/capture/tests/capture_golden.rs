//! The capture service against what Go logged and showed
//! (testdata/golden/capture/{replay,tail,vectors}.json, exported by
//! tools/goldens/export.sh with capture_harness_test.go and
//! capture_golden_test.go).

mod common;

use std::sync::Arc;

use capture::cursors::CursorFile;
use capture::{Blobs, Capture, EditQuery, NoScrub, Options, TimelineQuery};
use gocompat::json::{RawJson, marshal};
use gocompat::time::GoTime;
use search::ShowOptions;
use serde_json::Value as J;
// Only tail_logs_go_bytes (Windows) uses these.
#[cfg(windows)]
use {
    capture::{MemBlobs, REPLAY_KINDS},
    store::{Filter, MAX_LIST_LIMIT},
    sys::cancel::Cancel,
};

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

fn m<T: serde::Serialize>(v: &T) -> String {
    String::from_utf8(marshal(v).unwrap()).unwrap()
}

/// Go marshals a nil slice as `null`; these views return nil when empty.
fn m_nil<T: serde::Serialize>(v: &[T]) -> String {
    if v.is_empty() { "null".into() } else { m(&v) }
}

/// Every test's rows, replayed into a fresh Rust service, give Go's views.
#[test]
fn replay_matches_go_views() {
    let g = golden("replay.json");
    let rigs = g.as_array().unwrap();
    assert!(rigs.len() > 40, "{} rigs", rigs.len());
    let mut checked = 0;
    for r in rigs {
        let test = s(&r["test"]);
        let Some(rows) = r["rows"].as_array() else {
            continue;
        };
        let (_d, data) = common::tempdir();
        let (_c, claude) = common::tempdir();
        let st = common::open_store(&data);
        for row in rows {
            st.append(
                s(&row["actor"]),
                s(&row["kind"]),
                &RawJson(s(&row["payload"]).as_bytes().to_vec()),
            )
            .unwrap();
        }
        let now = GoTime::parse_rfc3339(s(&r["now"])).unwrap();
        let views = r["views"].as_object().unwrap();
        // The patches the recorded edit shows print, under Go's blob ids.
        let blobs = KeyedBlobs::default();
        for (name, want) in views {
            if let Some(id) = name.strip_prefix("blob ") {
                blobs
                    .0
                    .lock()
                    .unwrap()
                    .insert(id.into(), s(want).as_bytes().to_vec());
            }
        }
        let mut opts = Options::new(&claude, &data, "dev1", Arc::new(NoScrub), Arc::new(blobs));
        opts.now = Arc::new(move || now);
        let c = Capture::new(st, None, opts).unwrap_or_else(|e| panic!("{test}: {e}"));
        for (name, want) in views {
            if name.starts_with("blob ") {
                continue;
            }
            let got = if let Some(k) = name.strip_prefix("timeline+sub ") {
                m(&c.timeline(&TimelineQuery {
                    key: k.into(),
                    subagents: true,
                    limit: 100,
                    ..TimelineQuery::default()
                }))
            } else if let Some(k) = name.strip_prefix("timeline ") {
                m(&c.timeline(&TimelineQuery {
                    key: k.into(),
                    ..TimelineQuery::default()
                }))
            } else if let Some(k) = name.strip_prefix("prowners ") {
                m_nil(&c.pr_owners(k))
            } else if name == "search" || name.starts_with("search ") {
                let text = name.strip_prefix("search ").unwrap_or("");
                m_nil(&c.search(&own_query(text)).unwrap())
            } else if let Some(r) = name.strip_prefix("show ") {
                let got = m(&c.show(r, &ShowOptions::default()).unwrap());
                assert_eq!(
                    shown_norm(&got, r),
                    shown_norm(s(want), r),
                    "{test}: view {name}\n got {got}\nwant {}",
                    s(want)
                );
                checked += 1;
                continue;
            } else if let Some(rk) = name.strip_prefix("overview ") {
                m(&c.overview(
                    "",
                    &capture::OverviewInput {
                        repo_key: rk.into(),
                        ..Default::default()
                    },
                ))
            } else {
                match name.as_str() {
                    "edits" => m(&c.edits(&EditQuery {
                        limit: 500,
                        ..EditQuery::default()
                    })),
                    "commits" => m(&c.commits("", 500)),
                    "repos" => m(&c.repos()),
                    "prs" => m_nil(&c.prs("")),
                    other => panic!("unknown view {other}"),
                }
            };
            if got != s(want) {
                // Go breaks ties between equal times in map order (random);
                // compare with each run of equal-time entries sorted.
                assert_eq!(
                    ties_sorted(&got),
                    ties_sorted(s(want)),
                    "{test}: view {name}\n got {got}\nwant {}",
                    s(want)
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 100, "{checked} views");
}

/// A blob store holding given ids (Go's ids, whatever bytes they hash).
#[derive(Default)]
struct KeyedBlobs(std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>);

impl Blobs for KeyedBlobs {
    fn put(&self, _: &[u8]) -> Result<String, String> {
        Err("read-only".into())
    }
    fn get(&self, id: &str) -> Result<Vec<u8>, String> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| "blob: not found".into())
    }
}

/// The search the exporter records: capture's own kinds (a Go test's fake
/// sources are not in its log), up to the "more" probe.
fn own_query(text: &str) -> search::Query {
    search::Query {
        text: text.into(),
        kinds: vec![
            search::Kind::Turn,
            search::Kind::Edit,
            search::Kind::Commit,
            search::Kind::Pr,
            search::Kind::Decision,
        ],
        limit: search::MAX_LIMIT + 1,
        ..search::Query::default()
    }
}

/// A Shown's JSON for comparison. Its "anchor status:" line is dropped:
/// Go re-checked the anchor in the test's live temp repo, while the replay
/// only has the rows (with the temp paths rewritten), so it reads Unknown.
/// An agent's lines come from its timeline, whose same-`at` entries Go
/// orders by map iteration, so they and its related refs are sorted.
fn shown_norm(text: &str, r: &str) -> J {
    let mut v: J = serde_json::from_str(text).unwrap();
    let t = v["text"].as_str().unwrap_or("").to_string();
    let mut lines: Vec<&str> = t
        .split('\n')
        .filter(|l| !l.starts_with("anchor status:"))
        .collect();
    if r.starts_with("agent:") {
        lines.sort();
        if let Some(a) = v.get_mut("related").and_then(J::as_array_mut) {
            a.sort_by_key(|x| x.to_string());
        }
    }
    v["text"] = J::String(lines.join("\n"));
    v
}

/// A view's JSON with every run of consecutive array elements that share
/// one `at` sorted by their text (timeline entries, commits).
fn ties_sorted(text: &str) -> J {
    fn fix(v: &mut J) {
        let arr = match v {
            J::Array(a) => Some(a),
            J::Object(o) => o.get_mut("entries").and_then(|e| e.as_array_mut()),
            _ => None,
        };
        let Some(a) = arr else { return };
        let mut i = 0;
        while i < a.len() {
            let at = a[i]["at"].clone();
            let mut j = i + 1;
            while j < a.len() && a[j]["at"] == at {
                j += 1;
            }
            a[i..j].sort_by_key(|x| x.to_string());
            i = j;
        }
    }
    let mut v: J = serde_json::from_str(text).unwrap();
    fix(&mut v);
    v
}

#[cfg(windows)]
fn all_rows(st: &store::Store) -> Vec<(String, String, String)> {
    st.list(&Filter {
        kinds: REPLAY_KINDS.iter().map(|k| k.to_string()).collect(),
        limit: MAX_LIST_LIMIT,
        ..Filter::default()
    })
    .unwrap()
    .into_iter()
    .map(|e| (e.kind, e.actor, String::from_utf8(e.payload).unwrap()))
    .collect()
}

/// The synthetic sessions Go tailed (no repo, fixed paths) log the same
/// bytes here, keep the same blobs and write the same cursor file; a
/// restart re-reads nothing new.
#[cfg(windows)]
#[test]
fn tail_logs_go_bytes() {
    let g = golden("tail.json");
    let (_d, data) = common::tempdir();
    let (_c, claude) = common::tempdir();
    for f in g["files"].as_array().unwrap() {
        let path = capture::gopath::join(&[&claude, s(&f["rel"])]);
        let body: String = f["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| s(l).to_string())
            .collect();
        common::write_file(&path, &body);
    }
    let blobs = Arc::new(MemBlobs::default());
    let opts = Options::new(
        &claude,
        &data,
        s(&g["device"]),
        Arc::new(NoScrub),
        blobs.clone(),
    );
    let st = common::open_store(&data);
    let c = Capture::new(st.clone(), None, opts.clone()).unwrap();
    c.tail_pass(&Cancel::new());
    let got = all_rows(&st);
    let want: Vec<(String, String, String)> = g["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                s(&r["kind"]).to_string(),
                s(&r["actor"]).to_string(),
                s(&r["payload"]).to_string(),
            )
        })
        .collect();
    assert_eq!(got.len(), want.len(), "{got:#?}");
    // Files are tailed newest first, and these were written within one
    // mtime tick, so compare each file's rows in order: group by session
    // key and agent id.
    let by_file = |rows: &[(String, String, String)]| {
        let mut m: std::collections::BTreeMap<String, Vec<(String, String, String)>> =
            Default::default();
        for r in rows {
            let p: J = serde_json::from_str(&r.2).unwrap();
            let key = format!(
                "{}|{}",
                s(&p["key"]).to_owned() + s(&p["session"]),
                s(&p["agentId"])
            );
            m.entry(key).or_default().push(r.clone());
        }
        m
    };
    assert_eq!(by_file(&got), by_file(&want));
    for (id, body) in g["blobs"].as_object().unwrap() {
        assert_eq!(blobs.get(id).unwrap(), s(body).as_bytes(), "blob {id}");
    }
    let cur =
        std::fs::read_to_string(capture::gopath::join(&[&data, "capture-cursors.json"])).unwrap();
    let cur = cur.replace(&claude.replace('\\', "\\\\"), "$CLAUDE");
    assert_eq!(cur, s(&g["cursors"]));
    st.close().unwrap();
    let st = common::open_store(&data);
    let c = Capture::new(st.clone(), None, opts).unwrap();
    c.tail_pass(&Cancel::new());
    assert_eq!(
        all_rows(&st).len() as u64,
        g["rowsAfterRestart"].as_u64().unwrap()
    );
}

#[test]
fn vectors_match_go() {
    let g = golden("vectors.json");
    for v in g["ids"].as_array().unwrap() {
        let p: Vec<&str> = v["parts"].as_array().unwrap().iter().map(s).collect();
        let got = match s(&v["kind"]) {
            "turn" => capture::turn_row_id(p[0], p[1], p[2]),
            "edit" => capture::edit_row_id(p[0], p[1], p[2]),
            "summary" => capture::summary_row_id(p[0], p[1], p[2], p[3]),
            k => panic!("{k}"),
        };
        assert_eq!(got, s(&v["id"]), "{v}");
    }
    let home = s(&g["home"]);
    for v in g["commandDir"].as_array().unwrap() {
        if !cfg!(windows) {
            break; // Go ran these with Windows path rules
        }
        assert_eq!(
            capture::command_dir(s(&v["Cmd"]), s(&v["Cwd"]), home),
            s(&v["Dir"]),
            "{v}"
        );
    }
    if cfg!(windows) {
        for v in g["paths"].as_array().unwrap() {
            assert_eq!(capture::norm_root(s(&v["In"])), s(&v["Norm"]), "{v}");
            assert_eq!(capture::edit_path_key(s(&v["In"])), s(&v["Key"]), "{v}");
        }
        for v in g["relTo"].as_array().unwrap() {
            let got = capture::rel_to(s(&v["Root"]), s(&v["Path"]));
            let want = v["OK"].as_bool().unwrap().then(|| s(&v["Rel"]).to_string());
            assert_eq!(got, want, "{v}");
        }
    }
    for v in g["clipRunes"].as_array().unwrap() {
        let n = v["N"].as_u64().unwrap() as usize;
        assert_eq!(capture::clip_runes(s(&v["S"]), n), s(&v["Clip"]));
    }
    for (p, want) in g["isArtifact"].as_object().unwrap() {
        if !cfg!(windows) && p.contains('\\') {
            continue; // Go ran these with Windows path rules
        }
        assert_eq!(capture::is_artifact(p), want.as_bool().unwrap(), "{p}");
    }
    // The cursor file: Go's bytes, and they read back.
    let want = s(&g["cursorFile"]);
    let o: J = serde_json::from_str(want).unwrap();
    let mut cf = CursorFile::default();
    for (k, v) in o["files"].as_object().unwrap() {
        cf.set_file(k, v.as_i64().unwrap());
    }
    for (k, v) in o["reflogs"].as_object().unwrap() {
        cf.set_reflog(k, v.as_i64().unwrap());
    }
    assert_eq!(String::from_utf8(cf.bytes()).unwrap(), want);
    let (_d, dir) = common::tempdir();
    cf.save(&dir).unwrap();
    let (back, err) = CursorFile::load(&dir);
    assert!(err.is_none());
    assert_eq!(back.files, cf.files);
    assert_eq!(back.reflogs, cf.reflogs);
    assert_eq!(
        std::fs::read_to_string(capture::gopath::join(&[&dir, "capture-cursors.json"])).unwrap(),
        want
    );
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
