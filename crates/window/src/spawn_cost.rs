//! What today's spawned agents cost (card #450, docs/design/spawned-agents.md
//! §6, sa-6). `claude agents --json` reports no usage, so the cost is read
//! from each agent's transcript (`<claude dir>/projects/*/<session>.jsonl`
//! and its subagents' files): every assistant row's `message.usage`,
//! counted once per `message.id` (Claude Code writes one row per content
//! block, each with the same usage), priced by `message.model` at API
//! rates. On a Claude plan nothing is billed per token: the figure is what
//! the same use would cost at API prices, a measure of how much plan the
//! agents use, not a bill.
//!
//! Only this device's transcripts are here; another device's agents are
//! counted but not costed (part 11).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Dollars per million tokens: input, output, cache read, cache write for
/// 5 minutes, cache write for an hour. A snapshot of Anthropic's API
/// prices (the claude-api skill's table, cached 2026-10-06): writes are
/// 1.25x and 2x input, reads as listed or 0.1x input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub write_5m: f64,
    pub write_1h: f64,
}

const fn price(input: f64, output: f64, cache_read: f64) -> Price {
    Price {
        input,
        output,
        cache_read,
        write_5m: input * 1.25,
        write_1h: input * 2.0,
    }
}

/// Model id prefixes and their prices, most specific first.
const PRICES: &[(&str, Price)] = &[
    ("claude-fable-5", price(10.0, 50.0, 0.25)),
    ("claude-mythos-5", price(10.0, 50.0, 0.25)),
    ("claude-opus-5-5", price(4.0, 20.0, 0.20)),
    ("claude-opus-5", price(5.0, 25.0, 0.50)),
    ("claude-opus-4-8", price(5.0, 25.0, 0.50)),
    ("claude-opus-4-7", price(5.0, 25.0, 0.50)),
    ("claude-opus-4-6", price(5.0, 25.0, 0.50)),
    ("claude-opus-4-5", price(5.0, 25.0, 0.50)),
    ("claude-opus-4", price(15.0, 75.0, 1.50)),
    ("claude-sonnet-5-5", price(2.0, 10.0, 0.20)),
    ("claude-sonnet-5", price(2.0, 10.0, 0.20)),
    ("claude-sonnet-4", price(3.0, 15.0, 0.30)),
    ("claude-haiku-5-5", price(0.10, 0.50, 0.01)),
    ("claude-haiku-4-5", price(1.0, 5.0, 0.10)),
    ("claude-3-5-haiku", price(0.80, 4.0, 0.08)),
];

/// A model's price; an unknown one is priced as Opus 5.5, the default.
pub fn price_of(model: &str) -> Price {
    PRICES
        .iter()
        .find(|(p, _)| model.starts_with(p))
        .map(|(_, p)| *p)
        .unwrap_or(PRICES[2].1)
}

/// One transcript's tokens, by model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub write_5m: u64,
    pub write_1h: u64,
    pub model: String,
}

impl Usage {
    /// The row's price, at its prompt's tier: Haiku 5.5 costs more past
    /// 100K prompt tokens.
    pub fn cost(&self) -> f64 {
        let prompt = self.input + self.cache_read + self.write_5m + self.write_1h;
        let p = if self.model.starts_with("claude-haiku-5-5") && prompt > 100_000 {
            price(0.50, 2.50, 0.05)
        } else {
            price_of(&self.model)
        };
        let m = |n: u64, d: f64| n as f64 * d / 1_000_000.0;
        m(self.input, p.input)
            + m(self.output, p.output)
            + m(self.cache_read, p.cache_read)
            + m(self.write_5m, p.write_5m)
            + m(self.write_1h, p.write_1h)
    }
}

/// How far one transcript file has been read: its byte offset (always at
/// a line's end), the cost so far, and the messages already priced.
#[derive(Debug, Clone, Default)]
pub struct FileState {
    pub offset: u64,
    pub total: f64,
    seen: HashSet<String>,
}

/// The cost of one transcript line, if it is an assistant row not priced
/// yet (once per `message.id`).
fn line_cost(line: &str, seen: &mut HashSet<String>) -> f64 {
    // Cheap filter before parsing: only rows with usage.
    if !line.contains("\"usage\"") {
        return 0.0;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return 0.0;
    };
    if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return 0.0;
    }
    let Some(msg) = v.get("message") else {
        return 0.0;
    };
    if let Some(id) = msg.get("id").and_then(|i| i.as_str())
        && !seen.insert(id.to_string())
    {
        return 0.0;
    }
    let Some(u) = msg.get("usage") else {
        return 0.0;
    };
    let n = |p: &str| u.pointer(p).and_then(|x| x.as_u64()).unwrap_or(0);
    let creation = n("/cache_creation_input_tokens");
    let (w5, w1) = match u.get("cache_creation") {
        Some(_) => (
            n("/cache_creation/ephemeral_5m_input_tokens"),
            n("/cache_creation/ephemeral_1h_input_tokens"),
        ),
        None => (creation, 0),
    };
    // A split that doesn't add up: the rest is priced as 5-minute.
    let w5 = w5 + creation.saturating_sub(w5 + w1);
    Usage {
        input: n("/input_tokens"),
        output: n("/output_tokens"),
        cache_read: n("/cache_read_input_tokens"),
        write_5m: w5,
        write_1h: w1,
        model: msg
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string(),
    }
    .cost()
}

/// Reads what `path` gained since `st`: only whole lines past its offset
/// (a line still being written waits for the next read). A file that got
/// shorter is read again from the start.
pub fn read_more(path: &Path, st: &mut FileState) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return;
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len < st.offset {
        *st = FileState::default();
    }
    if len == st.offset || f.seek(SeekFrom::Start(st.offset)).is_err() {
        return;
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return;
    }
    let Some(end) = buf.iter().rposition(|&b| b == b'\n') else {
        return;
    };
    for line in String::from_utf8_lossy(&buf[..end]).lines() {
        st.total += line_cost(line, &mut st.seen);
    }
    st.offset += end as u64 + 1;
}

/// The cost of one whole transcript file.
pub fn file_cost(path: &Path) -> f64 {
    let mut st = FileState::default();
    read_more(path, &mut st);
    st.total
}

/// Session `id`'s project folder under `claude_dir`, if found (the
/// folder's name is the agent's cwd, lossily, so every one is looked in).
fn project_of(claude_dir: &Path, id: &str) -> Option<PathBuf> {
    if id.is_empty() || id.contains(['/', '\\', '.']) {
        return None;
    }
    std::fs::read_dir(claude_dir.join("projects"))
        .ok()?
        .flatten()
        .map(|d| d.path())
        .find(|d| d.join(format!("{id}.jsonl")).is_file())
}

/// Session `id`'s transcript and its subagents' in project folder `dir`.
fn files_in(dir: &Path, id: &str) -> Vec<PathBuf> {
    let mut out = vec![dir.join(format!("{id}.jsonl"))];
    if let Ok(subs) = std::fs::read_dir(dir.join(id).join("subagents")) {
        out.extend(
            subs.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl")),
        );
    }
    out
}

/// Session `id`'s transcript and its subagents' under `claude_dir`.
pub fn transcripts(claude_dir: &Path, id: &str) -> Vec<PathBuf> {
    project_of(claude_dir, id)
        .map(|d| files_in(&d, id))
        .unwrap_or_default()
}

/// What each transcript cost so far, read on from where it was left, and
/// each session's project folder, so Settings › Agents polling reads only
/// what was added.
#[derive(Default)]
pub struct CostCache {
    files: Mutex<HashMap<PathBuf, FileState>>,
    dirs: Mutex<HashMap<String, PathBuf>>,
}

impl CostCache {
    /// Session `id`'s cost in dollars, and whether any transcript was found.
    pub fn session(&self, claude_dir: &Path, id: &str) -> (f64, bool) {
        let dir = {
            let mut dirs = self.dirs.lock().unwrap_or_else(|e| e.into_inner());
            match dirs
                .get(id)
                .filter(|d| d.join(format!("{id}.jsonl")).is_file())
            {
                Some(d) => Some(d.clone()),
                None => {
                    let d = project_of(claude_dir, id);
                    if let Some(d) = &d {
                        dirs.insert(id.to_string(), d.clone());
                    }
                    d
                }
            }
        };
        let Some(dir) = dir else {
            return (0.0, false);
        };
        let mut files = self.files.lock().unwrap_or_else(|e| e.into_inner());
        let mut total = 0.0;
        for f in files_in(&dir, id) {
            let st = files.entry(f.clone()).or_default();
            read_more(&f, st);
            total += st.total;
        }
        (total, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_priced_once_per_message() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects").join("C--repo");
        std::fs::create_dir_all(proj.join("s1").join("subagents")).unwrap();
        // Two content blocks of one message, each row with the same usage:
        // 1M input and 1M output on Opus 5.5 = $4 + $20.
        let row = r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":1000000,"output_tokens":1000000,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        // A subagent on Sonnet 5.5: 1M cache read ($0.20) and 1M written
        // for an hour ($4).
        let sub = r#"{"type":"assistant","message":{"id":"m2","model":"claude-sonnet-5-5","usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":1000000,"cache_creation_input_tokens":1000000,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":1000000}}}}"#;
        std::fs::write(
            proj.join("s1.jsonl"),
            format!("{row}\n{row}\n{{\"type\":\"user\"}}\n"),
        )
        .unwrap();
        std::fs::write(proj.join("s1").join("subagents").join("agent-a.jsonl"), sub).unwrap();
        let c = CostCache::default();
        let (cost, found) = c.session(dir.path(), "s1");
        assert!(found);
        assert!((cost - 28.20).abs() < 1e-9, "{cost}");
        assert_eq!(c.session(dir.path(), "nope"), (0.0, false));
        assert_eq!(c.session(dir.path(), "../s1"), (0.0, false));
    }

    #[test]
    fn a_growing_file_is_read_on_from_where_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects").join("C--repo");
        std::fs::create_dir_all(&proj).unwrap();
        let p = proj.join("s2.jsonl");
        let row = |id: &str| {
            format!(
                r#"{{"type":"assistant","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":0,"output_tokens":1000000}}}}}}"#
            )
        };
        std::fs::write(&p, format!("{}\n", row("a"))).unwrap();
        let c = CostCache::default();
        assert!((c.session(dir.path(), "s2").0 - 20.0).abs() < 1e-9);
        // A line still being written isn't counted yet; once whole it is,
        // and a repeat of a message already priced isn't.
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        let b = row("b");
        write!(f, "{}\n{}", row("a"), &b[..20]).unwrap();
        f.flush().unwrap();
        assert!((c.session(dir.path(), "s2").0 - 20.0).abs() < 1e-9);
        writeln!(f, "{}", &b[20..]).unwrap();
        f.flush().unwrap();
        assert!((c.session(dir.path(), "s2").0 - 40.0).abs() < 1e-9);
    }

    #[test]
    fn haiku_5_5_past_100k_prompt_tokens_costs_more() {
        let u = |input: u64| Usage {
            input,
            model: "claude-haiku-5-5".into(),
            ..Usage::default()
        };
        assert!((u(100_000).cost() - 0.01).abs() < 1e-12);
        assert!((u(200_000).cost() - 0.10).abs() < 1e-12);
    }

    #[test]
    fn models_find_their_price() {
        assert_eq!(price_of("claude-opus-5-5").input, 4.0);
        assert_eq!(price_of("claude-opus-5").input, 5.0);
        assert_eq!(price_of("claude-sonnet-5-5").output, 10.0);
        assert_eq!(price_of("claude-haiku-4-5-20251001").input, 1.0);
        assert_eq!(price_of("claude-fable-5-1").output, 50.0);
        assert_eq!(price_of("something-new").input, 4.0);
    }
}
