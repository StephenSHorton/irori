//! The ledger against vectors Go 1.26.5 produced (testdata/golden/ledger,
//! exported by tools/goldens/export.sh from ledger_golden_test.go).

use std::collections::HashMap;

use gocompat::json::marshal;
use gocompat::time::GoTime;
use ledger::{
    BashResult, Ledger, RefOptions, decode_entry, extract_refs, key, parse_target, repo_from_key,
    repo_key,
};
use serde_json::Value as J;

fn golden(name: &str) -> J {
    let p = format!(
        "{}/../../testdata/golden/ledger/{name}",
        manifest_dir().display()
    );
    serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap()
}

fn s(v: &J) -> &str {
    v.as_str().unwrap_or("")
}

fn unhex(v: &J) -> Vec<u8> {
    hex::decode(s(v)).unwrap()
}

#[test]
fn extract_refs_matches_go() {
    let g = golden("refs.json");
    let mut opts = HashMap::new();
    for o in g["options"].as_array().unwrap() {
        let aliases = o["aliases"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), s(v).to_string()))
                    .collect()
            })
            .unwrap_or_default();
        opts.insert(
            s(&o["name"]).to_string(),
            RefOptions {
                default_repo: s(&o["defaultRepo"]).into(),
                owner: s(&o["owner"]).into(),
                aliases,
            },
        );
    }
    let cases = g["extract"].as_array().unwrap();
    assert!(cases.len() > 300);
    for c in cases {
        let text = s(&c["text"]);
        let got: Vec<J> = extract_refs(text, &opts[s(&c["opts"])])
            .iter()
            .map(|r| serde_json::json!({"repo": r.repo, "number": r.number}))
            .collect();
        assert_eq!(
            J::Array(got),
            c["refs"],
            "ExtractRefs({text:?}, {})",
            s(&c["opts"])
        );
    }
}

#[test]
fn parse_target_matches_go() {
    let g = golden("refs.json");
    for c in g["target"].as_array().unwrap() {
        let input = s(&c["in"]);
        let got = parse_target(input);
        assert_eq!(got.is_some(), c["ok"].as_bool().unwrap(), "{input:?}");
        let r = got.unwrap_or_default();
        assert_eq!(r.repo, s(&c["ref"]["repo"]), "{input:?}");
        assert_eq!(r.number, c["ref"]["number"].as_i64().unwrap(), "{input:?}");
    }
}

#[test]
fn from_bash_matches_go() {
    let g = golden("bash.json");
    let cases = g.as_array().unwrap();
    assert!(cases.len() > 150);
    for c in cases {
        let i = &c["in"];
        let b = BashResult {
            command: s(&i["command"]).into(),
            stdout: s(&i["stdout"]).into(),
            git_operation: unhex(&i["gitOperationHex"]),
            default_repo: s(&i["defaultRepo"]).into(),
        };
        let got: Vec<J> = ledger::from_bash(&b)
            .iter()
            .map(|r| {
                serde_json::json!({
                    "repo": r.r#ref.repo, "number": r.r#ref.number, "url": r.url,
                    "role": r.role.as_str(), "source": r.source,
                })
            })
            .collect();
        assert_eq!(J::Array(got), c["out"], "FromBash({i})");
    }
}

#[test]
fn keys_match_go() {
    let g = golden("keys.json");
    for c in g["keys"].as_array().unwrap() {
        let repo = s(&c["repo"]);
        let n = c["n"].as_i64().unwrap();
        assert_eq!(key(repo, n), s(&c["key"]), "Key({repo:?}, {n})");
        let rk = repo_key(repo);
        assert_eq!(rk, s(&c["repoKey"]), "RepoKey({repo:?})");
        assert_eq!(repo_from_key(&rk), s(&c["back"]), "RepoFromKey({rk:?})");
    }
    for c in g["fromKey"].as_array().unwrap() {
        let k = s(&c["in"]);
        assert_eq!(repo_from_key(k), s(&c["repo"]), "RepoFromKey({k:?})");
    }
}

#[test]
fn decode_entry_matches_go() {
    let g = golden("keys.json");
    for c in g["decode"].as_array().unwrap() {
        let payload = unhex(&c["hex"]);
        let shown = String::from_utf8_lossy(&payload);
        match decode_entry(&payload) {
            Ok(e) => {
                assert!(c["ok"].as_bool().unwrap(), "{shown}: Go refused it");
                let m = String::from_utf8(marshal(&e).unwrap()).unwrap();
                assert_eq!(m, s(&c["marshal"]), "{shown}");
            }
            Err(err) => assert!(!c["ok"].as_bool().unwrap(), "{shown}: {err}"),
        }
    }
}

fn marshal_all(es: &[ledger::Entry]) -> J {
    J::Array(
        es.iter()
            .map(|e| J::String(String::from_utf8(marshal(e).unwrap()).unwrap()))
            .collect(),
    )
}

#[test]
fn script_replays_like_go() {
    let g = golden("script.json");
    let l = Ledger::new();
    let (mut adds, mut views) = (0, 0);
    for (i, step) in g.as_array().unwrap().iter().enumerate() {
        let op = s(&step["op"]);
        if op == "add" {
            let raw = s(&step["entry"]);
            let e = decode_entry(raw.as_bytes()).unwrap();
            // The decoded entry marshals back to Go's bytes.
            assert_eq!(
                String::from_utf8(marshal(&e).unwrap()).unwrap(),
                raw,
                "step {i}"
            );
            let want = step["changed"].as_bool().unwrap_or(false);
            assert_eq!(l.add(e), want, "step {i}: Add({raw})");
            adds += 1;
            continue;
        }
        let now = GoTime::parse_rfc3339(s(&step["now"])).unwrap();
        let arg = s(&step["arg"]);
        let got = match op {
            "owners" => l.owners(arg, &now),
            "forRepo" => l.for_repo(arg, &now),
            "forSession" => l.for_session(arg, &now),
            _ => panic!("step {i}: op {op}"),
        };
        let want = step.get("out").cloned().unwrap_or(J::Array(vec![]));
        assert_eq!(marshal_all(&got), want, "step {i}: {op}({arg:?}, {now})");
        views += 1;
    }
    assert!(adds > 100 && views > 50, "adds {adds}, views {views}");
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
