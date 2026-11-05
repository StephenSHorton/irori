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
use std::time::SystemTime;

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
    pub fn cost(&self) -> f64 {
        let p = price_of(&self.model);
        let m = |n: u64, d: f64| n as f64 * d / 1_000_000.0;
        m(self.input, p.input)
            + m(self.output, p.output)
            + m(self.cache_read, p.cache_read)
            + m(self.write_5m, p.write_5m)
            + m(self.write_1h, p.write_1h)
    }
}

/// The cost of one transcript file's assistant rows, once per message.
pub fn file_cost(path: &Path) -> f64 {
    let Ok(text) = std::fs::read_to_string(path) else {
        return 0.0;
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut total = 0.0;
    for line in text.lines() {
        // Cheap filter before parsing: only rows with usage.
        if !line.contains("\"usage\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        if let Some(id) = msg.get("id").and_then(|i| i.as_str())
            && !seen.insert(id.to_string())
        {
            continue;
        }
        let Some(u) = msg.get("usage") else { continue };
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
        total += Usage {
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
        .cost();
    }
    total
}

/// Session `id`'s transcript and its subagents' under `claude_dir`, if
/// found (the project folder's name is the agent's cwd, lossily, so every
/// one is looked in).
pub fn transcripts(claude_dir: &Path, id: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if id.is_empty() || id.contains(['/', '\\', '.']) {
        return out;
    }
    let Ok(dirs) = std::fs::read_dir(claude_dir.join("projects")) else {
        return out;
    };
    for d in dirs.flatten() {
        let main = d.path().join(format!("{id}.jsonl"));
        if !main.is_file() {
            continue;
        }
        out.push(main);
        if let Ok(subs) = std::fs::read_dir(d.path().join(id).join("subagents")) {
            out.extend(
                subs.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "jsonl")),
            );
        }
        break;
    }
    out
}

/// Costs by file, kept while the file's size and time are unchanged, so
/// Settings › Agents polling reads only what grew.
#[derive(Default)]
pub struct CostCache {
    files: Mutex<HashMap<PathBuf, (u64, Option<SystemTime>, f64)>>,
}

impl CostCache {
    /// Session `id`'s cost in dollars, and whether any transcript was found.
    pub fn session(&self, claude_dir: &Path, id: &str) -> (f64, bool) {
        let files = transcripts(claude_dir, id);
        let found = !files.is_empty();
        let mut total = 0.0;
        let mut cache = self.files.lock().unwrap_or_else(|e| e.into_inner());
        for f in files {
            let meta = std::fs::metadata(&f).ok();
            let len = meta.as_ref().map_or(0, |m| m.len());
            let at = meta.and_then(|m| m.modified().ok());
            let cost = match cache.get(&f) {
                Some((l, t, c)) if *l == len && *t == at => *c,
                _ => {
                    let c = file_cost(&f);
                    cache.insert(f.clone(), (len, at, c));
                    c
                }
            };
            total += cost;
        }
        (total, found)
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
    fn models_find_their_price() {
        assert_eq!(price_of("claude-opus-5-5").input, 4.0);
        assert_eq!(price_of("claude-opus-5").input, 5.0);
        assert_eq!(price_of("claude-sonnet-5-5").output, 10.0);
        assert_eq!(price_of("claude-haiku-4-5-20251001").input, 1.0);
        assert_eq!(price_of("claude-fable-5-1").output, 50.0);
        assert_eq!(price_of("something-new").input, 4.0);
    }
}
