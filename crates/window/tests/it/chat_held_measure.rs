//! One assistant, card 8's measurement (one-assistant.md §8, card #1108):
//! the chat's per-question turn (card 3: a fresh `claude -p` each question,
//! the history in its prompt) against a held session (one `claude -p` kept
//! open, one stream-json user line per question), with the chat's tools,
//! on the chat's model, over 20 questions.
//!
//! Both modes run the exact argv `Job::assistant` builds (`job_args`, which
//! already reads one stream-json line), the chat's system prompt and MCP
//! config. The held mode only keeps stdin open. The questions alternate
//! which mode goes first, so load drifts fall on both.
//!
//! It needs a running Core whose assistant secret it can use: a scratch
//! copy (`scripts/scratch-mac.sh start`, offline) that has run one chat
//! turn, so its `assistant/mcp.json` holds the secret, the exe and the
//! data dir. Spends the person's usage; run by hand, with the calling
//! session's `CLAUDE_*` variables cleared:
//!
//! ```sh
//! env -u CLAUDE_EFFORT -u CLAUDECODE ... \
//!   IRORI_MEASURE_MCP=<scratch>/home/assistant/mcp.json \
//!   IRORI_MEASURE_OUT=<file>.jsonl \
//!   IRORI_MEASURE_MUTATE=<script> \
//!   cargo test -p irori-window --test it chat_held_measure -- --ignored --nocapture
//! ```
//!
//! `IRORI_MEASURE_MUTATE` is run as `<script> A` after question 4 and
//! `<script> B` after question 15 and must return once the change is in:
//! A makes a new card, B comments on #1095. A held answer that doesn't look
//! again shows there.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use irori_window::assistant::{
    AssistantTurn, Input, Job, SYSTEM, chat_content_blocks, job_args, prepare, render_prompt,
    stream_json_line,
};

/// The questions, one conversation. Some lean on earlier answers (2, 6, 8,
/// 13, 17, 20); 14 and 18 follow a change made mid-run.
pub const QUESTIONS: [&str; 20] = [
    "What's blocked on my board right now?",
    "Which of those has been waiting the longest?",
    "What is card #1093 about?",
    "What are the three newest cards on my board?",
    "Who's running right now, and what is each one working on?",
    "What did the first of those agents say last?",
    "Which cards came out of #1093?",
    "Which of them are done, and which are still open?",
    "Find anything about Windows signing.",
    "Are there any pull requests waiting on me?",
    "What does decision 52 say?",
    "Where do I change which model this chat runs on?",
    "Go back to the blocked cards you listed at the start: is the second one still blocked?",
    "What's the newest card on my board right now?",
    "Sum up where the one-assistant work stands, in a few lines.",
    "Draft a thread to add a copy button to each of your replies, but check with me first.",
    "Also say it should work on the phone.",
    "What's the latest comment on #1095?",
    "What notes do we have about scratch builds?",
    "Last thing: what were the first two questions I asked you today?",
];

/// Past this much context a held session starts again from the rows
/// (§8, as call-agents §7 does).
const RESEED_AT: u64 = 60_000;
/// How long one question may take, all in.
const TURN_TIMEOUT: Duration = Duration::from_secs(240);

/// What one question measured in one mode.
#[derive(Default, Debug)]
struct Measured {
    first_line_ms: Option<u128>,
    first_text_ms: Option<u128>,
    first_tool_ms: Option<u128>,
    total_ms: u128,
    duration_ms: u64,
    duration_api_ms: u64,
    num_turns: u64,
    cost_usd: f64,
    context: u64,
    tools: Vec<serde_json::Value>,
    tool_results: Vec<String>,
    text: String,
    error: String,
}

fn ms(t0: Instant) -> u128 {
    t0.elapsed().as_millis()
}

/// Folds one stdout line into `m`; true at the turn's `result`.
fn take_line(m: &mut Measured, line: &str, t0: Instant, held: bool) -> bool {
    m.first_line_ms.get_or_insert_with(|| ms(t0));
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
        return false;
    };
    match v["type"].as_str().unwrap_or("") {
        "stream_event" => {
            if v["event"]["delta"]["type"] == "text_delta" {
                m.first_text_ms.get_or_insert_with(|| ms(t0));
            }
        }
        "assistant" => {
            for b in v["message"]["content"].as_array().into_iter().flatten() {
                if b["type"] == "tool_use" {
                    m.first_tool_ms.get_or_insert_with(|| ms(t0));
                    m.tools
                        .push(serde_json::json!({"name": b["name"], "input": b["input"]}));
                }
            }
        }
        "user" => {
            for b in v["message"]["content"].as_array().into_iter().flatten() {
                if b["type"] == "tool_result" {
                    let s = match &b["content"] {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Array(a) => a
                            .iter()
                            .filter_map(|x| x["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        x => x.to_string(),
                    };
                    m.tool_results.push(s.chars().take(8000).collect());
                }
            }
        }
        "result" => {
            m.total_ms = ms(t0);
            let n = |k: &str| v[k].as_u64().unwrap_or(0);
            m.duration_ms = n("duration_ms");
            m.duration_api_ms = n("duration_api_ms");
            m.num_turns = n("num_turns");
            m.text = v["result"].as_str().unwrap_or("").to_string();
            if v["is_error"].as_bool().unwrap_or(false) || v["subtype"] != "success" {
                m.error = format!("{} {}", v["subtype"], m.text);
            }
            let u = &v["usage"];
            let t = |k: &str| u[k].as_u64().unwrap_or(0);
            m.context = t("input_tokens")
                + t("cache_creation_input_tokens")
                + t("cache_read_input_tokens")
                + t("output_tokens");
            // Per turn: a held session's `total_cost_usd` adds up.
            m.cost_usd = if held {
                v["model_usage_since_last_result"]
                    .as_object()
                    .map(|o| o.values().filter_map(|x| x["costUSD"].as_f64()).sum())
                    .unwrap_or(0.0)
            } else {
                v["total_cost_usd"].as_f64().unwrap_or(0.0)
            };
            return true;
        }
        _ => {}
    }
    false
}

/// `claude` started as the chat starts it (the window's `command_with`).
fn spawn(bin: &Path, job: &Job) -> Child {
    Command::new(bin)
        .args(job_args(job))
        .current_dir(&job.dir)
        .env_remove("CLAUDE_CODE_PLUGIN_DIRS")
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env("ENABLE_CLAUDEAI_MCP_SERVERS", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start claude")
}

/// Stdout lines with the moment each arrived.
fn lines_of(child: &mut Child) -> mpsc::Receiver<String> {
    let out = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for l in BufReader::new(out).lines() {
            let Ok(l) = l else { break };
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    rx
}

fn read_turn(rx: &mpsc::Receiver<String>, t0: Instant, held: bool) -> Measured {
    let mut m = Measured::default();
    loop {
        let left = TURN_TIMEOUT.saturating_sub(t0.elapsed());
        match rx.recv_timeout(left) {
            Ok(l) => {
                if take_line(&mut m, &l, t0, held) {
                    return m;
                }
            }
            Err(_) => {
                m.total_ms = ms(t0);
                m.error = "no result (timed out or ended)".into();
                return m;
            }
        }
    }
}

/// One question as card 3 runs it: a fresh process, the history in the
/// prompt, stdin closed after the one line.
fn fresh_turn(bin: &Path, job: &Job, history: &[AssistantTurn], q: &str) -> Measured {
    let line = stream_json_line(&chat_content_blocks(&render_prompt(history, q, &[]), &[]));
    let t0 = Instant::now();
    let mut child = spawn(bin, job);
    let rx = lines_of(&mut child);
    {
        let mut w = child.stdin.take().unwrap();
        w.write_all(line.as_bytes()).unwrap();
    }
    let m = read_turn(&rx, t0, false);
    let _ = child.kill();
    let _ = child.wait();
    m
}

/// The held session.
struct Held {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<String>,
    turns: usize,
    /// The session's API time so far: its `duration_api_ms` adds up.
    api_ms: u64,
}

impl Held {
    fn start(bin: &Path, job: &Job) -> Held {
        let mut child = spawn(bin, job);
        let rx = lines_of(&mut child);
        let stdin = child.stdin.take().unwrap();
        Held {
            child,
            stdin,
            rx,
            turns: 0,
            api_ms: 0,
        }
    }

    fn turn(&mut self, text: &str) -> Measured {
        let line = stream_json_line(&chat_content_blocks(text, &[]));
        let t0 = Instant::now();
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.flush().unwrap();
        self.turns += 1;
        let mut m = read_turn(&self.rx, t0, true);
        let total = m.duration_api_ms;
        m.duration_api_ms = total.saturating_sub(self.api_ms);
        self.api_ms = total.max(self.api_ms);
        m
    }

    /// The resident memory of `claude` and its children (`irori mcp`), KiB.
    fn rss_kb(&self) -> u64 {
        let pid = self.child.id().to_string();
        let mut pids = vec![pid.clone()];
        if let Ok(o) = Command::new("pgrep").args(["-P", &pid]).output() {
            pids.extend(
                String::from_utf8_lossy(&o.stdout)
                    .split_whitespace()
                    .map(String::from),
            );
        }
        pids.iter()
            .filter_map(|p| {
                let o = Command::new("ps").args(["-o", "rss=", "-p", p]).output().ok()?;
                String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok()
            })
            .sum()
    }

    fn end(mut self) {
        drop(self.stdin);
        std::thread::sleep(Duration::from_millis(500));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn load1() -> f64 {
    Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0.0)
}

fn which_claude() -> PathBuf {
    let out = Command::new("sh")
        .args(["-c", "command -v claude"])
        .output()
        .unwrap();
    PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
}

#[allow(clippy::too_many_arguments)]
fn record(
    out: &mut std::fs::File,
    mode: &str,
    i: usize,
    q: &str,
    m: &Measured,
    rss_kb: u64,
    reseeded: bool,
    load: f64,
) {
    let v = serde_json::json!({
        "mode": mode, "i": i + 1, "q": q,
        "first_line_ms": m.first_line_ms, "first_text_ms": m.first_text_ms,
        "first_tool_ms": m.first_tool_ms, "total_ms": m.total_ms,
        "duration_ms": m.duration_ms, "duration_api_ms": m.duration_api_ms,
        "num_turns": m.num_turns, "cost_usd": m.cost_usd, "context": m.context,
        "tools": m.tools, "tool_results": m.tool_results,
        "text": m.text, "error": m.error,
        "rss_kb": rss_kb, "reseeded": reseeded, "load1": load,
    });
    writeln!(out, "{v}").unwrap();
    out.flush().unwrap();
    println!(
        "{mode:5} q{:02}: {:>6} ms (text at {:?}, api {} ms), {} tools, ${:.4}, {} tok{}{}",
        i + 1,
        m.total_ms,
        m.first_text_ms,
        m.duration_api_ms,
        m.tools.len(),
        m.cost_usd,
        m.context,
        if reseeded { ", reseeded" } else { "" },
        if m.error.is_empty() {
            String::new()
        } else {
            format!(", error: {}", m.error)
        }
    );
}

#[test]
#[ignore = "real Claude Code against a scratch Core: spends usage; run by hand"]
fn chat_held_measure() {
    let mcp = std::env::var("IRORI_MEASURE_MCP").expect("IRORI_MEASURE_MCP");
    let out_path = std::env::var("IRORI_MEASURE_OUT").expect("IRORI_MEASURE_OUT");
    let mutate = std::env::var("IRORI_MEASURE_MUTATE").unwrap_or_default();
    let model = std::env::var("IRORI_MEASURE_MODEL").unwrap_or_else(|_| "sonnet".into());
    let effort = std::env::var("IRORI_MEASURE_EFFORT").unwrap_or_else(|_| "low".into());
    let only: usize = std::env::var("IRORI_MEASURE_ONLY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(QUESTIONS.len());
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp).unwrap()).unwrap();
    let srv = &cfg["mcpServers"]["irori"];
    let exe = PathBuf::from(srv["command"].as_str().unwrap());
    let data_dir = PathBuf::from(srv["env"]["IRORI_HOME"].as_str().unwrap());
    let secret = srv["env"]["IRORI_ASSISTANT_SECRET"]
        .as_str()
        .unwrap()
        .to_string();
    let root = tempfile::tempdir().unwrap();
    let job = |name: &str| {
        let j = Job {
            dir: root.path().join(name),
            data_dir: data_dir.clone(),
            secret: secret.clone(),
            exe: exe.clone(),
            system: SYSTEM.into(),
            prompt: String::new(),
            input: Input::StreamJson,
            model: model.clone(),
            effort: Some(effort.clone()),
            claude_config_dir: None,
            timeout: TURN_TIMEOUT,
            tools: true,
            look: String::new(),
            no_settings: false,
        };
        prepare(&j).unwrap();
        j
    };
    let (fresh_job, held_job) = (job("fresh"), job("held"));
    let bin = which_claude();
    let mut out = std::fs::File::create(&out_path).unwrap();
    println!("argv: {}", job_args(&held_job).join(" "));

    let mut fresh_hist: Vec<AssistantTurn> = Vec::new();
    let mut held_hist: Vec<AssistantTurn> = Vec::new();
    let mut held: Option<Held> = None;
    let turn = |role: &str, text: &str| AssistantTurn {
        role: role.into(),
        text: text.into(),
        files: Vec::new(),
    };
    for (i, q) in QUESTIONS.iter().take(only).enumerate() {
        let run_fresh = |hist: &mut Vec<AssistantTurn>, out: &mut std::fs::File| {
            let load = load1();
            let m = fresh_turn(&bin, &fresh_job, hist, q);
            record(out, "fresh", i, q, &m, 0, false, load);
            if m.error.is_empty() {
                hist.push(turn("person", q));
                hist.push(turn("assistant", &m.text));
            }
        };
        let mut run_held = |hist: &mut Vec<AssistantTurn>, out: &mut std::fs::File| {
            let load = load1();
            // A new session is seeded from the rows: the same prompt a
            // fresh turn gets. A running one gets the question alone.
            let (h, text, reseeded) = match held.take() {
                Some(h) => (h, format!("The person: {}\n", q.trim()), false),
                None => (
                    Held::start(&bin, &held_job),
                    render_prompt(hist, q, &[]),
                    i > 0,
                ),
            };
            let mut h = h;
            let m = h.turn(&text);
            let rss = h.rss_kb();
            record(out, "held", i, q, &m, rss, reseeded, load);
            if m.error.is_empty() {
                hist.push(turn("person", q));
                hist.push(turn("assistant", &m.text));
            }
            if m.error.is_empty() && m.context < RESEED_AT {
                held = Some(h);
            } else {
                h.end();
            }
        };
        if i % 2 == 0 {
            run_fresh(&mut fresh_hist, &mut out);
            run_held(&mut held_hist, &mut out);
        } else {
            run_held(&mut held_hist, &mut out);
            run_fresh(&mut fresh_hist, &mut out);
        }
        let point = match i + 1 {
            4 => Some("A"),
            15 => Some("B"),
            _ => None,
        };
        if let (Some(p), false) = (point, mutate.is_empty()) {
            let ok = Command::new(&mutate).arg(p).status().unwrap().success();
            println!("mutation {p}: {}", if ok { "in" } else { "FAILED" });
            writeln!(out, "{}", serde_json::json!({"mutation": p, "ok": ok})).unwrap();
        }
    }
    if let Some(h) = held {
        h.end();
    }
}
