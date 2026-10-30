//! I:internal/capture/transcript/parser.go: turns, summaries and PR links
//! from transcript rows.

use std::collections::{HashMap, HashSet};

use gocompat::json::{self, Out, Schema};
use gocompat::time::GoTime;

use super::result::ToolUse;
use super::target::{clip, target};
use super::{
    Item, MAX_PROMPT, MAX_REPLY, MAX_SUMMARY, MAX_TOOLS_SHOWN, PrLink, Source, Summary, ToolCall,
    Turn,
};
use crate::gotime::after;
use crate::jsonx::{out_bool, out_raw, out_str, trim_space, trim_space_bytes};

/// The subset of a transcript record the parser reads.
#[derive(Debug, Default)]
pub(crate) struct Row {
    pub typ: String,
    pub subtype: String,
    pub uuid: String,
    pub prompt_id: String,
    pub timestamp: String,
    pub cwd: String,
    pub git_branch: String,
    pub agent_id: String,
    pub is_meta: bool,
    pub is_compact_summary: bool,
    pub prompt_source: String,
    pub origin: Option<Vec<u8>>,
    pub message_content: Option<Vec<u8>>,
    pub tool_use_result: Option<Vec<u8>>,
    pub content: Option<Vec<u8>>,
    pub ai_title: String,
    pub custom_title: String,
    pub pr_number: Option<Vec<u8>>,
    pub pr_url: String,
    pub pr_repository: String,
}

fn row_schema() -> Schema {
    Schema::object(vec![
        ("type", Schema::String),
        ("subtype", Schema::String),
        ("uuid", Schema::String),
        ("promptId", Schema::String),
        ("timestamp", Schema::String),
        ("cwd", Schema::String),
        ("gitBranch", Schema::String),
        ("agentId", Schema::String),
        ("isMeta", Schema::Bool),
        ("isCompactSummary", Schema::Bool),
        ("promptSource", Schema::String),
        ("origin", Schema::Raw),
        ("message", Schema::object(vec![("content", Schema::Raw)])),
        ("toolUseResult", Schema::Raw),
        ("content", Schema::Raw),
        ("aiTitle", Schema::String),
        ("customTitle", Schema::String),
        ("prNumber", Schema::Raw),
        ("prUrl", Schema::String),
        ("prRepository", Schema::String),
    ])
}

/// `json.Unmarshal(line, &r)`: any type mismatch drops the whole line.
fn decode_row(line: &[u8]) -> Option<Row> {
    let o = json::unmarshal(line, &row_schema()).ok()?;
    let s = |n: &str| out_str(o.get(n));
    Some(Row {
        typ: s("type"),
        subtype: s("subtype"),
        uuid: s("uuid"),
        prompt_id: s("promptId"),
        timestamp: s("timestamp"),
        cwd: s("cwd"),
        git_branch: s("gitBranch"),
        agent_id: s("agentId"),
        is_meta: out_bool(o.get("isMeta")),
        is_compact_summary: out_bool(o.get("isCompactSummary")),
        prompt_source: s("promptSource"),
        origin: out_raw(o.get("origin")),
        message_content: out_raw(o.get("message").and_then(|m| m.get("content"))),
        tool_use_result: out_raw(o.get("toolUseResult")),
        content: out_raw(o.get("content")),
        ai_title: s("aiTitle"),
        custom_title: s("customTitle"),
        pr_number: out_raw(o.get("prNumber")),
        pr_url: s("prUrl"),
        pr_repository: s("prRepository"),
    })
}

/// One message.content entry.
#[derive(Debug, Clone, Default)]
pub(crate) struct Block {
    pub typ: String,
    pub text: String,
    pub id: String,
    pub name: String,
    pub input: Option<Vec<u8>>,
    pub tool_use_id: String,
    pub is_error: bool,
    pub content: Option<Vec<u8>>,
}

fn block_schema() -> Schema {
    Schema::object(vec![
        ("type", Schema::String),
        ("text", Schema::String),
        ("id", Schema::String),
        ("name", Schema::String),
        ("input", Schema::Raw),
        ("tool_use_id", Schema::String),
        ("is_error", Schema::Bool),
        ("content", Schema::Raw),
    ])
}

fn block_of(o: &Out) -> Block {
    Block {
        typ: out_str(o.get("type")),
        text: out_str(o.get("text")),
        id: out_str(o.get("id")),
        name: out_str(o.get("name")),
        input: out_raw(o.get("input")),
        tool_use_id: out_str(o.get("tool_use_id")),
        is_error: out_bool(o.get("is_error")),
        content: out_raw(o.get("content")),
    }
}

/// Decodes message.content: a string becomes one text block; if the array
/// fails as a whole, each element is decoded alone and odd ones dropped.
pub(crate) fn content_blocks(raw: Option<&[u8]>) -> Vec<Block> {
    let raw = trim_space_bytes(raw.unwrap_or(&[]));
    if raw.is_empty() || raw[0] == b'n' {
        return Vec::new();
    }
    if raw[0] == b'"' {
        return match json::unmarshal(raw, &Schema::String) {
            Ok(Out::String(s)) => vec![Block {
                typ: "text".into(),
                text: s,
                ..Block::default()
            }],
            _ => Vec::new(),
        };
    }
    let schema = Schema::Slice(Box::new(block_schema()));
    if let Ok(o) = json::unmarshal(raw, &schema) {
        return o.items().iter().map(block_of).collect();
    }
    // One odd block should not hide the others.
    let Ok(Out::Slice(Some(anys))) = json::unmarshal(raw, &Schema::Slice(Box::new(Schema::Raw)))
    else {
        return Vec::new();
    };
    let bs = block_schema();
    anys.iter()
        .filter_map(|a| match a {
            Out::Raw(Some(b)) => json::unmarshal(b, &bs).ok().map(|o| block_of(&o)),
            _ => None,
        })
        .collect()
}

pub(crate) fn join_text(bs: &[Block]) -> String {
    bs.iter()
        .filter(|b| b.typ == "text")
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `raw` as a JSON string, or "" when it is not one.
pub(crate) fn raw_string(raw: Option<&[u8]>) -> String {
    match raw.map(|r| json::unmarshal(r, &Schema::String)) {
        Some(Ok(Out::String(s))) => s,
        _ => String::new(),
    }
}

fn is_peer(s: &str) -> bool {
    let s = trim_space(s);
    s.starts_with("Another Claude session sent a message") || s.starts_with("<teammate-message")
}

fn is_local_output(s: &str) -> bool {
    let s = trim_space(s);
    s.starts_with("<local-command-stdout>") || s.starts_with("<local-command-stderr>")
}

pub(crate) fn is_interrupt(s: &str) -> bool {
    let s = trim_space(s);
    s == "[Request interrupted by user]" || s == "[Request interrupted by user for tool use]"
}

/// A prompt row's origin (design §3).
fn origin(r: &Row, text: &str) -> (String, String) {
    let raw = trim_space_bytes(r.origin.as_deref().unwrap_or(&[]));
    if raw.first() == Some(&b'{') {
        let s = Schema::object(vec![("kind", Schema::String), ("name", Schema::String)]);
        if let Ok(o) = json::unmarshal(raw, &s) {
            let kind = out_str(o.get("kind"));
            if !kind.is_empty() {
                if kind == "plugin" {
                    return (kind, out_str(o.get("name")));
                }
                return (kind, String::new());
            }
        }
    } else if raw.first() == Some(&b'"')
        && let Ok(Out::String(s)) = json::unmarshal(raw, &Schema::String)
        && !s.is_empty()
    {
        return (s, String::new());
    }
    let k = if r.prompt_source == "sdk" {
        "sdk"
    } else if trim_space(text).starts_with("<command-name>") {
        "command"
    } else if is_peer(text) {
        "peer"
    } else {
        "human"
    };
    (k.into(), String::new())
}

fn parse_time(s: &str) -> GoTime {
    if s.is_empty() {
        return GoTime::ZERO;
    }
    GoTime::parse_rfc3339(s).unwrap_or(GoTime::ZERO)
}

/// The turn being built and its bookkeeping. `gen` stands in for Go's
/// pointer identity of the open turn.
pub(crate) struct OpenTurn {
    pub t: Turn,
    pub last: GoTime,
    pub paths: HashSet<String>,
    pub serial: u64,
}

/// One parser per transcript file; not safe for concurrent use.
pub struct Parser {
    pub(crate) src: Source,
    pub(crate) cur: Option<OpenTurn>,
    pub(crate) calls: HashMap<String, ToolUse>,
    newest: GoTime,
    pending: Vec<Item>,
    seen: HashSet<String>,
    next_gen: u64,
}

impl Parser {
    /// A parser for one file.
    pub fn new(src: Source) -> Parser {
        Parser {
            src,
            cur: None,
            calls: HashMap::new(),
            newest: GoTime::ZERO,
            pending: Vec::new(),
            seen: HashSet::new(),
            next_gen: 0,
        }
    }

    pub(crate) fn emit(&mut self, it: Item) {
        self.pending.push(it);
    }

    /// Feeds one complete line (a trailing `\r\n` is stripped) and returns
    /// what it finished, in order. A closed Turn comes once, after the edits
    /// and bash runs of that turn.
    pub fn line(&mut self, line: &[u8]) -> Vec<Item> {
        let mut end = line.len();
        while end > 0 && (line[end - 1] == b'\r' || line[end - 1] == b'\n') {
            end -= 1;
        }
        let line = &line[..end];
        if super::skip_line(line) {
            return Vec::new();
        }
        let Some(r) = decode_row(line) else {
            return Vec::new();
        };
        if r.agent_id != self.src.agent_id {
            return Vec::new();
        }
        let at = parse_time(&r.timestamp);
        if after(&at, &self.newest) {
            self.newest = at;
        }
        match r.typ.as_str() {
            "user" => self.user(&r, at),
            "assistant" => self.assistant(&r, at),
            "system" => self.system(&r, at),
            "ai-title" => self.summary("ai_title", &r.ai_title, "", GoTime::ZERO),
            "custom-title" => self.summary("custom_title", &r.custom_title, "", GoTime::ZERO),
            "pr-link" => self.pr_link(&r, at),
            _ => {}
        }
        std::mem::take(&mut self.pending)
    }

    /// Ends the open turn, if any, with `ended_at = at` and `reason`
    /// ("quiet" or "eof"), and returns it.
    pub fn close(&mut self, at: GoTime, reason: &str) -> Vec<Item> {
        self.end(at, reason);
        std::mem::take(&mut self.pending)
    }

    /// A copy of the open turn.
    pub fn open(&self) -> Option<Turn> {
        self.cur.as_ref().map(|c| c.t.clone())
    }

    /// Whether a turn is open.
    pub fn is_open(&self) -> bool {
        self.cur.is_some()
    }

    /// The newest row timestamp seen.
    pub fn last_activity(&self) -> GoTime {
        self.newest
    }

    fn end(&mut self, at: GoTime, reason: &str) {
        let Some(mut c) = self.cur.take() else {
            return;
        };
        c.t.ended_at = at;
        c.t.end_reason = reason.into();
        self.emit(Item::Turn(c.t));
    }

    /// Records activity on the open turn and fills its cwd and branch.
    pub(crate) fn touch(&mut self, r: &Row, at: GoTime) {
        let Some(c) = self.cur.as_mut() else {
            return;
        };
        if after(&at, &c.last) {
            c.last = at;
        }
        if c.t.cwd.is_empty() {
            c.t.cwd = r.cwd.clone();
        }
        if c.t.branch.is_empty() {
            c.t.branch = r.git_branch.clone();
        }
    }

    fn start(
        &mut self,
        id: &str,
        origin: &str,
        origin_name: &str,
        prompt: &str,
        r: &Row,
        at: GoTime,
    ) {
        self.seen.insert(id.to_string());
        self.next_gen += 1;
        self.cur = Some(OpenTurn {
            t: Turn {
                id: id.into(),
                session_id: self.src.session_id.clone(),
                agent_id: self.src.agent_id.clone(),
                origin: origin.into(),
                origin_name: origin_name.into(),
                prompt: clip(prompt, MAX_PROMPT),
                cwd: r.cwd.clone(),
                branch: r.git_branch.clone(),
                started_at: at,
                ..Turn::default()
            },
            last: at,
            paths: HashSet::new(),
            serial: self.next_gen,
        });
    }

    /// Starts a turn with origin unknown when none is open: for a tool
    /// result or tool_use with no prompt before it.
    pub(crate) fn implicit(&mut self, r: &Row, at: GoTime) {
        if self.cur.is_none() {
            let id = r.uuid.clone();
            self.start(&id, "unknown", "", "", r, at);
        }
    }

    fn user(&mut self, r: &Row, at: GoTime) {
        let bs = content_blocks(r.message_content.as_deref());
        let results: Vec<&super::parser::Block> =
            bs.iter().filter(|b| b.typ == "tool_result").collect();
        let tur = r
            .tool_use_result
            .as_deref()
            .map(trim_space_bytes)
            .unwrap_or(&[]);
        let has_tur = !tur.is_empty() && tur != b"null";
        let text = join_text(&bs);

        if has_tur || !results.is_empty() {
            // A result row is never a prompt. One with a toolUseResult but
            // no tool_result block has nothing to match and is dropped.
            if results.is_empty() {
                return;
            }
            for (i, b) in results.iter().enumerate() {
                let res = if has_tur && i == 0 { Some(tur) } else { None };
                self.result(r, at, b, res);
            }
            // A result for a closed turn leaves no turn open.
            self.touch(r, at);
            for b in &bs {
                if b.typ == "text"
                    && is_interrupt(&b.text)
                    && let Some(c) = self.cur.as_mut()
                {
                    c.t.interrupted = true;
                }
            }
            return;
        }
        if r.is_compact_summary {
            self.summary("compact_summary", &text, &r.uuid, at);
            return;
        }
        if r.is_meta {
            return;
        }
        if is_local_output(&text) {
            // A local slash command's output: activity, not a prompt.
            self.touch(r, at);
            return;
        }
        if is_interrupt(&text) {
            if let Some(c) = self.cur.as_mut() {
                c.t.interrupted = true;
                self.touch(r, at);
            }
            return;
        }
        let mut id = if r.prompt_id.is_empty() {
            r.uuid.clone()
        } else {
            r.prompt_id.clone()
        };
        if self.cur.as_ref().is_some_and(|c| c.t.id == id) {
            // Another row of the same prompt (a queued or peer message):
            // the turn goes on.
            self.touch(r, at);
            return;
        }
        if self.seen.contains(&id) {
            // Its turn already closed and was emitted: keep ids unique.
            id = r.uuid.clone();
        }
        let (mut kind, name) = origin(r, &text);
        if kind == "human"
            && !self.src.agent_id.is_empty()
            && trim_space_bytes(r.origin.as_deref().unwrap_or(&[])).is_empty()
        {
            // A subagent's or workflow agent's prompts carry no origin.
            kind = "agent".into();
        }
        if let Some(c) = self.cur.as_ref() {
            let end = if c.last.is_zero() { at } else { c.last };
            self.end(end, "next");
        }
        self.start(&id, &kind, &name, &text, r, at);
    }

    fn assistant(&mut self, r: &Row, at: GoTime) {
        let bs = content_blocks(r.message_content.as_deref());
        for b in &bs {
            match b.typ.as_str() {
                "text" => {
                    if let Some(c) = self.cur.as_mut()
                        && !trim_space(&b.text).is_empty()
                    {
                        c.t.reply = clip(&b.text, MAX_REPLY);
                    }
                }
                "tool_use" => {
                    self.implicit(r, at);
                    let c = self.cur.as_mut().expect("implicit opened a turn");
                    let mut call = ToolUse {
                        name: b.name.clone(),
                        input: b.input.clone(),
                        turn_gen: c.serial,
                        turn_id: c.t.id.clone(),
                        index: None,
                        at,
                    };
                    if c.t.tools.len() < MAX_TOOLS_SHOWN {
                        call.index = Some(c.t.tools.len());
                        c.t.tools.push(ToolCall {
                            id: b.id.clone(),
                            name: b.name.clone(),
                            target: target(&b.name, b.input.as_deref(), &r.cwd),
                            at,
                            ..ToolCall::default()
                        });
                    } else {
                        c.t.tools_more += 1;
                    }
                    if !b.id.is_empty() {
                        self.calls.insert(b.id.clone(), call);
                    }
                }
                _ => {}
            }
        }
        self.touch(r, at);
    }

    fn system(&mut self, r: &Row, at: GoTime) {
        match r.subtype.as_str() {
            "turn_duration" => {
                if self.cur.is_some() {
                    self.end(at, "turn_duration");
                }
            }
            "away_summary" | "compact_boundary" => {
                let text = raw_string(r.content.as_deref());
                self.summary(&r.subtype.clone(), &text, &r.uuid.clone(), at);
            }
            _ => {}
        }
    }

    fn summary(&mut self, kind: &str, text: &str, uuid: &str, at: GoTime) {
        let clipped = clip(text, MAX_SUMMARY);
        let full = if clipped != text {
            text.to_string()
        } else {
            String::new()
        };
        let s = Summary {
            session_id: self.src.session_id.clone(),
            agent_id: self.src.agent_id.clone(),
            kind: kind.into(),
            text: clipped,
            full,
            row_uuid: uuid.into(),
            at,
        };
        self.emit(Item::Summary(s));
    }

    fn pr_link(&mut self, r: &Row, at: GoTime) {
        let Some(n) = flex_int(r.pr_number.as_deref()) else {
            return;
        };
        let l = PrLink {
            session_id: self.src.session_id.clone(),
            repo: r.pr_repository.clone(),
            number: n,
            url: r.pr_url.clone(),
            at,
        };
        self.emit(Item::PrLink(l));
    }
}

/// A JSON number, or a string holding one ("#" trimmed); must be > 0.
fn flex_int(raw: Option<&[u8]>) -> Option<i64> {
    let raw = trim_space_bytes(raw?);
    if raw.is_empty() {
        return None;
    }
    let owned;
    let raw = if raw[0] == b'"' {
        let Ok(Out::String(s)) = json::unmarshal(raw, &Schema::String) else {
            return None;
        };
        owned = trim_space(&s).trim_start_matches_once('#').to_string();
        owned.as_bytes()
    } else {
        raw
    };
    match json::unmarshal(raw, &Schema::Int(64)) {
        Ok(Out::Int(n)) if n > 0 => Some(n),
        _ => None,
    }
}

trait TrimOnce {
    fn trim_start_matches_once(&self, c: char) -> &str;
}

impl TrimOnce for str {
    /// Go `strings.TrimPrefix(s, "#")`: one occurrence.
    fn trim_start_matches_once(&self, c: char) -> &str {
        self.strip_prefix(c).unwrap_or(self)
    }
}
