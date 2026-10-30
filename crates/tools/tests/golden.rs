//! The tool-text goldens (surface.md §7.10): testdata/golden/tools/
//! scenario.json replayed over the tools_test.go harness must give Go's
//! transcript.json byte for byte, with only ids masked (each distinct id
//! becomes <id N> by first appearance, so the same id must recur where Go's
//! did). The clock is fixed, so every time and "ago" is compared as is.
//! Also: the spec lists and the shared instructions equal Go's.

mod common;

use std::collections::HashMap;
use std::time::Duration;

use common::*;
use serde_json::Value;

fn golden(name: &str) -> Value {
    let p = manifest_dir()
        .join("../../testdata/golden/tools")
        .join(name);
    let data = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_slice(&data).unwrap()
}

fn caller(c: &str) -> tools::Caller {
    match c {
        "A" => call_a(),
        "B" => call_b(),
        "C" => call_c(),
        "G" => grok_caller(),
        _ => tools::Caller::default(),
    }
}

fn key(k: &str) -> String {
    match k {
        "A" => KEY_A,
        "B" => KEY_B,
        "C" => KEY_C,
        "G" => KEY_G,
        other => other,
    }
    .to_string()
}

/// Masks ids (ULIDs, with or without msg_/dlv_/wake_) by first appearance.
struct Masker {
    re: regex::Regex,
    seen: HashMap<String, usize>,
}

impl Masker {
    fn new() -> Masker {
        Masker {
            re: regex::Regex::new(
                r"(?:msg_|dlv_|wake_)?(?:[0-9A-HJKMNP-TV-Z]{26}|[0-9A-HJKMNP-TV-Z]{11,25}…)",
            )
            .unwrap(),
            seen: HashMap::new(),
        }
    }
    fn mask(&mut self, s: &str) -> String {
        let mut out = String::new();
        let mut last = 0;
        for m in self.re.find_iter(s) {
            out.push_str(&s[last..m.start()]);
            if m.as_str().ends_with('…') {
                // An id one_line clipped: its random part can't be matched.
                out.push_str("<clipped id>…");
                last = m.end();
                continue;
            }
            let n = self.seen.len() + 1;
            let id = *self.seen.entry(m.as_str().to_string()).or_insert(n);
            out.push_str(&format!("<id {id}>"));
            last = m.end();
        }
        out.push_str(&s[last..]);
        out
    }
}

/// Plays scenario steps (Go's goldenRun).
struct Run {
    h: Harness,
    vars: HashMap<String, String>,
    out: Vec<(usize, String, String)>,
}

impl Run {
    fn subst(&self, s: &str) -> String {
        let mut s = s.to_string();
        for (k, v) in &self.vars {
            s = s.replace(&format!("${k}"), v);
        }
        s
    }

    fn steps(&mut self, base: Option<usize>, steps: &[Value]) {
        for (j, st) in steps.iter().enumerate() {
            let step = base.unwrap_or(j);
            let s = |k: &str| st[k].as_str().unwrap_or("").to_string();
            match st["op"].as_str().unwrap() {
                "call" => {
                    self.h.clk.advance(Duration::from_secs(1));
                    let tool = s("tool");
                    let args = self.subst(&s("args"));
                    let text = match self
                        .h
                        .set
                        .call(&caller(&s("caller")), &tool, args.as_bytes())
                    {
                        Ok(t) => t,
                        Err(e) => format!("call error: {e}"),
                    };
                    if let Some(save) = st["save"].as_object() {
                        for (name, label) in save {
                            let name = self.subst(name);
                            self.vars
                                .insert(name, field(&text, label.as_str().unwrap()));
                        }
                    }
                    self.out.push((step, tool, text));
                }
                "repeat" => {
                    for n in 0..st["n"].as_u64().unwrap() {
                        self.vars.insert("i".into(), format!("{n:02}"));
                        self.steps(Some(step), st["steps"].as_array().unwrap());
                    }
                }
                "person_send" => {
                    let from = inbox::Sender::new("user:dev1", "you", "user");
                    let to = inbox::Address::new(inbox::addr::AGENT, &self.subst(&s("to")));
                    self.h
                        .b
                        .ib
                        .send(&from, &to, &self.subst(&s("text")), "")
                        .unwrap();
                }
                "advance" => self
                    .h
                    .clk
                    .advance(Duration::from_secs(st["seconds"].as_u64().unwrap())),
                "reachable" => self.h.b.ib.set_reachable(&key(&s("key")), &s("path"), true),
                "owner" => {
                    let keys = st["keys"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|k| key(k.as_str().unwrap()))
                        .collect();
                    self.h.owners.lock().unwrap().insert(s("pr"), keys);
                }
                "status" => self.h.b.roster.set_status(&key(&s("key")), &s("status")),
                "release" => {
                    let d = self.subst(&s("delivery"));
                    self.h.b.ib.release(&d, "user:dev1").unwrap()
                }
                op => panic!("step {step}: unknown op {op}"),
            }
        }
    }
}

/// Runs `<name>.json` and compares it with Go's `<name>.transcript.json`.
fn check(name: &str) {
    let sc = golden(&format!("{name}.json"));
    let mut r = Run {
        h: new_harness(),
        vars: HashMap::new(),
        out: vec![],
    };
    // Go held Grok's messages to bypass sessions (bypass_receiver). That
    // hold is gone (card #71); the person blocking Grok holds the same
    // messages, so the transcript keeps its shape with "blocked" as the
    // reason.
    r.h.b
        .ib
        .set_policy(
            inbox::Policy {
                block: vec![KEY_G.into()],
                ..Default::default()
            },
            "user:dev1",
        )
        .unwrap();
    r.steps(None, sc["steps"].as_array().unwrap());
    let want = golden(&format!("{name}.transcript.json"));
    let want = want.as_array().unwrap();
    let got = r.out;
    assert_eq!(got.len(), want.len(), "{name}: result count");
    let (mut mg, mut mw) = (Masker::new(), Masker::new());
    let mut bad = vec![];
    for ((step, tool, text), w) in got.iter().zip(want) {
        assert_eq!(*step as u64, w["step"].as_u64().unwrap());
        let (g, wt) = (mg.mask(text), mw.mask(w["text"].as_str().unwrap()));
        if g != wt {
            bad.push(format!(
                "step {step} ({tool}):\n--- go\n{wt}\n--- rust\n{g}\n"
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{name}: {} of {} results differ:\n{}",
        bad.len(),
        got.len(),
        bad.join("\n")
    );
}

#[test]
fn tool_texts_match_go() {
    check("scenario");
}

#[test]
fn budget_texts_match_go() {
    check("budget");
}

#[test]
fn specs_and_instructions_match_go() {
    let want = golden("specs.json");
    let h = new_harness();
    for client in ["", "claude", "grok", "other"] {
        let got = String::from_utf8(h.set.specs_json(client)).unwrap();
        assert_eq!(
            got,
            want["specs"][client].as_str().unwrap(),
            "specs for {client:?}"
        );
    }
    assert_eq!(tools::INSTRUCTIONS, want["instructions"].as_str().unwrap());
    assert_eq!(
        tools::BOARD_INSTRUCTIONS,
        want["boardInstructions"].as_str().unwrap()
    );
    assert_eq!(
        tools::ASK_INSTRUCTIONS,
        want["askInstructions"].as_str().unwrap()
    );
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
