//! The API golden replay (architecture.md §6.2, §8 M4): the Rust Core and
//! its api::Server run testdata/golden/api/scenario.json, the scenario
//! tools/goldens/api_golden_test.go ran on the Go Core into go.json, and
//! every answer must equal Go's: status, the recorded headers, and the
//! body bytes, once each run's random values are named (as the Go side
//! names them) and ids, times and HLCs are masked.
//!
//! Steps that need a service this build does not have yet (`needs`:
//! `tools` until the tools crate is wired in, `mesh` until M5) are
//! skipped and counted.

mod common;

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use irori_api::client::{Lines, Opened};
use serde_json::Value;

const HEADERS: [&str; 5] = [
    "Content-Type",
    "Cache-Control",
    "X-Content-Type-Options",
    "Allow",
    "WWW-Authenticate",
];

/// Services this build has; a step needing another is skipped.
const HAVE: [&str; 1] = ["tools"];

fn golden_dir() -> std::path::PathBuf {
    manifest_dir().join("../../testdata/golden/api")
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Out {
    id: String,
    status: i64,
    headers: BTreeMap<String, String>,
    body: Option<String>,
    lines: Option<Vec<String>>,
    result: Option<String>,
}

fn out_of(v: &Value) -> Out {
    Out {
        id: v["id"].as_str().unwrap_or("").into(),
        status: v["status"].as_i64().unwrap_or(0),
        headers: v["headers"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").into()))
                    .collect()
            })
            .unwrap_or_default(),
        body: v["body"].as_str().map(str::to_string),
        lines: v["lines"].as_array().map(|a| {
            a.iter()
                .map(|l| l.as_str().unwrap_or("").to_string())
                .collect()
        }),
        result: v["result"].as_str().map(str::to_string),
    }
}

/// Masks what differs between any two runs: HLCs, times, ULIDs (numbered
/// in order of first sight), member ids and secrets.
struct Masker {
    ids: HashMap<String, usize>,
    members: HashMap<String, usize>,
    hlc: regex::Regex,
    time: regex::Regex,
    ulid: regex::Regex,
    member: regex::Regex,
    secret: regex::Regex,
}

impl Masker {
    fn new() -> Masker {
        Masker {
            ids: HashMap::new(),
            members: HashMap::new(),
            hlc: regex::Regex::new(r#""hlc":[0-9]+"#).unwrap(),
            time: regex::Regex::new(
                r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?(Z|[+-][0-9]{2}:[0-9]{2})",
            )
            .unwrap(),
            ulid: regex::Regex::new(r"[0-9A-HJKMNP-TV-Z]{26}").unwrap(),
            member: regex::Regex::new(r"grok-[0-9a-f]{12}").unwrap(),
            secret: regex::Regex::new(r"[0-9a-f]{48}").unwrap(),
        }
    }

    fn mask(&mut self, s: &str) -> String {
        let s = self.hlc.replace_all(s, r#""hlc":0"#).into_owned();
        let s = self.time.replace_all(&s, "{time}").into_owned();
        let s = self.secret.replace_all(&s, "{secret}").into_owned();
        let mut out = String::new();
        let mut last = 0;
        for m in self.ulid.find_iter(&s) {
            out.push_str(&s[last..m.start()]);
            let n = self.ids.len() + 1;
            let n = *self.ids.entry(m.as_str().to_string()).or_insert(n);
            out.push_str(&format!("{{id{n}}}"));
            last = m.end();
        }
        out.push_str(&s[last..]);
        let s = out;
        let mut out = String::new();
        let mut last = 0;
        for m in self.member.find_iter(&s) {
            out.push_str(&s[last..m.start()]);
            let n = self.members.len() + 1;
            let n = *self.members.entry(m.as_str().to_string()).or_insert(n);
            out.push_str(&format!("grok-{{member{n}}}"));
            last = m.end();
        }
        out.push_str(&s[last..]);
        out
    }

    fn mask_out(&mut self, o: &Out) -> Out {
        Out {
            id: o.id.clone(),
            status: o.status,
            headers: o.headers.clone(),
            body: o.body.as_deref().map(|b| self.mask(b)),
            lines: o
                .lines
                .as_ref()
                .map(|ls| ls.iter().map(|l| self.mask(l)).collect()),
            result: o.result.as_deref().map(|r| self.mask(r)),
        }
    }
}

fn drop_keys(body: &str, keys: &[String]) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Object(mut m)) => {
            for k in keys {
                m.remove(k);
            }
            serde_json::to_string(&Value::Object(m)).unwrap()
        }
        _ => body.to_string(),
    }
}

/// Go's GOOS for this machine.
fn go_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    }
}

struct Run {
    rig: common::Rig,
    vars: BTreeMap<String, String>,
    streams: HashMap<String, tokio::sync::mpsc::Receiver<Option<String>>>,
}

impl Run {
    fn expand(&self, s: &str) -> String {
        let mut s = s.to_string();
        for (k, v) in &self.vars {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        s
    }

    /// Each run's values back to their names (the Go side's `mask`).
    fn name(&self, s: &str) -> String {
        let mut s = s.replace(
            &format!("127.0.0.1:{}", self.vars["port"]),
            "127.0.0.1:{port}",
        );
        s = s.replace(&format!("\"pid\":{},", self.vars["pid"]), "\"pid\":{pid},");
        let mut list: Vec<(String, String)> = Vec::new();
        for (k, v) in &self.vars {
            if v.is_empty()
                || ["sessA", "sessB", "keyA", "keyB", "port", "pid"].contains(&k.as_str())
            {
                continue;
            }
            list.push((k.clone(), v.clone()));
            let e = serde_json::to_string(v).unwrap();
            let e = &e[1..e.len() - 1];
            let g = String::from_utf8(gocompat::json::marshal(v).unwrap()).unwrap();
            let g = &g[1..g.len() - 1];
            for alt in [e, g] {
                if alt != v {
                    list.push((k.clone(), alt.to_string()));
                }
            }
        }
        list.sort_by_key(|p| std::cmp::Reverse(p.1.len()));
        for (k, v) in list {
            s = s.replace(&v, &format!("{{{k}}}"));
        }
        s
    }

    fn save(&mut self, step: &Value, docs: &[String]) {
        let Some(save) = step["save"].as_object() else {
            return;
        };
        for (name, path) in save {
            let path = path.as_array().unwrap();
            let (mut i, keys) = match path.first().and_then(Value::as_i64) {
                Some(n) => (n, &path[1..]),
                None => (0, &path[..]),
            };
            if i < 0 {
                i += docs.len() as i64;
            }
            let mut v: Value = serde_json::from_str(&docs[i as usize]).unwrap();
            for k in keys {
                v = v[k.as_str().unwrap()].take();
            }
            self.vars
                .insert(name.clone(), v.as_str().unwrap_or_default().to_string());
        }
    }

    fn headers_of(hs: &[(String, String)]) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for h in HEADERS {
            let vals: Vec<&str> = hs
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case(h))
                .map(|(_, v)| v.as_str())
                .collect();
            if !vals.is_empty() {
                out.insert(h.to_string(), vals.join(", "));
            }
        }
        out
    }

    fn req_headers(&self, step: &Value, key: &str) -> Vec<(String, String)> {
        let mut hs = Vec::new();
        if let Some(m) = step[key]["headers"].as_object() {
            for (k, v) in m {
                hs.push((k.clone(), self.expand(v.as_str().unwrap())));
            }
        }
        hs
    }

    fn step(&mut self, step: &Value) -> Out {
        let id = step["id"].as_str().unwrap().to_string();
        let mut o = Out {
            id: id.clone(),
            ..Out::default()
        };
        if step.get("req").is_some() {
            let r = &step["req"];
            let mut hs = self.req_headers(step, "req");
            if r["auth"].as_bool() == Some(false)
                && !hs
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            {
                // No token: an empty Authorization never authorizes.
                hs.push(("Authorization".into(), String::new()));
            }
            let hr: Vec<(&str, &str)> = hs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let path = self.expand(r["path"].as_str().unwrap());
            let body = r["body"].as_str().map(|b| self.expand(b).into_bytes());
            let method = r["method"].as_str().unwrap().to_string();
            let resp = self
                .rig
                .rt
                .block_on(self.rig.client.raw(&method, &path, &hr, body))
                .unwrap_or_else(|e| panic!("{id}: {e}"));
            o.status = i64::from(resp.status);
            o.headers = Run::headers_of(&resp.headers);
            let text = String::from_utf8_lossy(&resp.body).into_owned();
            if resp.status == 200 {
                self.save(step, std::slice::from_ref(&text));
            }
            o.body = Some(self.name(&text));
        } else if step.get("open").is_some() {
            let hs = self.req_headers(step, "open");
            let hr: Vec<(&str, &str)> = hs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let path = self.expand(step["open"]["path"].as_str().unwrap());
            let got = self
                .rig
                .rt
                .block_on(self.rig.client.open(&path, &hr))
                .unwrap_or_else(|e| panic!("{id}: {e}"));
            match got {
                Opened::Refused(r) => {
                    o.status = i64::from(r.status);
                    o.headers = Run::headers_of(&r.headers);
                    o.body = Some(self.name(&String::from_utf8_lossy(&r.body)));
                }
                Opened::Stream(lines, hs) => {
                    o.status = 200;
                    o.headers = Run::headers_of(&hs);
                    let (tx, rx) = tokio::sync::mpsc::channel(256);
                    self.rig.rt.spawn(pump(lines, tx));
                    self.streams
                        .insert(step["open"]["stream"].as_str().unwrap().into(), rx);
                }
            }
        } else if step.get("read").is_some() {
            let r = &step["read"];
            let name = r["stream"].as_str().unwrap();
            let n = r["n"].as_u64().unwrap_or(0) as usize;
            let until = r["until"].as_str().unwrap_or("").to_string();
            let drain = r["drain"].as_u64().unwrap_or(0);
            let wait = if drain > 0 {
                Duration::from_millis(drain)
            } else {
                Duration::from_secs(10)
            };
            let rx = self
                .streams
                .get_mut(name)
                .unwrap_or_else(|| panic!("{id}: no stream {name}"));
            let mut got: Vec<String> = Vec::new();
            while n == 0 || got.len() < n {
                let line = self
                    .rig
                    .rt
                    .block_on(async { tokio::time::timeout(wait, rx.recv()).await });
                match line {
                    Ok(Some(Some(l))) => {
                        let done =
                            !until.is_empty() && l.contains(&format!("{{\"type\":\"{until}\""));
                        got.push(l);
                        if done {
                            break;
                        }
                    }
                    Ok(_) if drain > 0 => break,
                    Ok(_) => panic!("{id}: stream {name} ended after {got:?}"),
                    Err(_) if drain > 0 => break,
                    Err(_) => panic!("{id}: stream {name}: no line in {wait:?} after {got:?}"),
                }
            }
            self.save(step, &got);
            o.lines = Some(got.iter().map(|l| self.name(l)).collect());
        } else if let Some(name) = step["close"].as_str() {
            self.streams.remove(name);
        } else if let Some(a) = step["act"].as_object() {
            let a: HashMap<String, String> = a
                .iter()
                .map(|(k, v)| (k.clone(), self.expand(v.as_str().unwrap())))
                .collect();
            let core = &self.rig.core;
            let res = match a["do"].as_str() {
                "send" => {
                    let (m, d) = core
                        .inbox()
                        .send(
                            &inbox::Sender::new(&a["from"], &a["fromName"], "claude"),
                            &inbox::Address::new("agent", &a["to"]),
                            &a["text"],
                            "",
                        )
                        .unwrap_or_else(|e| panic!("{id}: {e}"));
                    serde_json::json!({"delivery": d.id, "message": m.id, "state": d.state})
                }
                "task" => {
                    let t = core
                        .coord()
                        .create_task(
                            &a["by"],
                            coord::NewTask {
                                title: a["title"].clone(),
                                ..Default::default()
                            },
                        )
                        .unwrap_or_else(|e| panic!("{id}: {e}"));
                    serde_json::json!({"id": t.id})
                }
                "comment" => {
                    let (c, _) = core
                        .coord()
                        .add_comment(&a["by"], &a["task"], &a["text"], false)
                        .unwrap_or_else(|e| panic!("{id}: {e}"));
                    serde_json::json!({"id": c.id})
                }
                other => panic!("{id}: unknown act {other}"),
            };
            // Go marshals a map: keys sorted, as serde_json's map is.
            let text = String::from_utf8(gocompat::json::marshal(&res).unwrap()).unwrap();
            self.save(step, std::slice::from_ref(&text));
            o.result = Some(self.name(&text));
        }
        o
    }
}

async fn pump(mut lines: Lines, tx: tokio::sync::mpsc::Sender<Option<String>>) {
    while let Some(l) = lines.next().await {
        if l.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        if tx
            .send(Some(String::from_utf8_lossy(&l).into_owned()))
            .await
            .is_err()
        {
            return;
        }
    }
    let _ = tx.send(None).await;
}

#[test]
fn api_replays_equal_to_go() {
    let dir = golden_dir();
    let scenario: Value =
        serde_json::from_slice(&std::fs::read(dir.join("scenario.json")).unwrap()).unwrap();
    let golden: Value =
        serde_json::from_slice(&std::fs::read(dir.join("go.json")).unwrap()).unwrap();
    let go: Vec<Out> = golden["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(out_of)
        .collect();
    let steps = scenario["steps"].as_array().unwrap();
    assert_eq!(steps.len(), go.len(), "go.json is stale: re-run export.sh");

    let rig = common::Rig::new();
    let dev = rig.core.device().clone();
    let mut vars = BTreeMap::new();
    for (k, v) in [
        ("device", dev.id.clone()),
        ("port", rig.port.to_string()),
        ("token", rig.token.clone()),
        ("person", rig.server.person_secret()),
        ("keyA", rig.key_a.clone()),
        ("keyB", rig.key_b.clone()),
        ("sessA", common::SESS_A.into()),
        ("sessB", common::SESS_B.into()),
        ("cwd", rig.cwd.to_string_lossy().replace('\\', "/")),
        ("cwdNative", rig.cwd.to_string_lossy().into_owned()),
        ("pid", std::process::id().to_string()),
        ("personId", dev.person_id.clone()),
        ("deviceName", dev.name.clone()),
        ("devicePub", dev.device_pub.clone()),
        ("data", rig.data.to_string_lossy().into_owned()),
        ("claude", rig.claude.to_string_lossy().into_owned()),
    ] {
        vars.insert(k.to_string(), v);
    }
    let mut run = Run {
        rig,
        vars,
        streams: HashMap::new(),
    };

    let (mut gm, mut rm) = (Masker::new(), Masker::new());
    let mut diffs = Vec::new();
    let mut skipped = Vec::new();
    for (step, g) in steps.iter().zip(&go) {
        let id = step["id"].as_str().unwrap();
        assert_eq!(id, g.id, "go.json is stale: re-run export.sh");
        let needs = step["needs"].as_str().unwrap_or("");
        if !needs.is_empty() && !HAVE.contains(&needs) {
            skipped.push(format!("{id} ({needs})"));
            continue;
        }
        let r = run.step(step);
        let (mut gw, mut rw) = (gm.mask_out(g), rm.mask_out(&r));
        if !cfg!(windows) {
            // Go recorded these on Windows: its GOOS, and a native cwd that
            // differs from the slashed one there but not here.
            gw.body = gw.body.map(|b| {
                b.replace("{cwdNative}", "{cwd}")
                    .replace(r#""os":"windows""#, &format!(r#""os":"{}""#, go_os()))
            });
        }
        if let Some(keys) = step["ignore"].as_array() {
            let keys: Vec<String> = keys.iter().map(|k| k.as_str().unwrap().into()).collect();
            gw.body = gw.body.map(|b| drop_keys(&b, &keys));
            rw.body = rw.body.map(|b| drop_keys(&b, &keys));
        }
        if gw != rw {
            diffs.push(format!("step {id}:\n  go:   {gw:?}\n  rust: {rw:?}"));
        }
    }
    eprintln!(
        "api golden: {} steps, {} skipped: {}",
        steps.len(),
        skipped.len(),
        skipped.join(", ")
    );
    assert!(
        diffs.is_empty(),
        "{} of {} steps differ from Go:\n{}",
        diffs.len(),
        steps.len(),
        diffs.join("\n")
    );
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
