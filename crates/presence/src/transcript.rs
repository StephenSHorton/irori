//! Go `transcript.go`: finding a session's transcript and reading its tail.

use std::io::{Read, Seek, SeekFrom};
use std::sync::LazyLock;

use gocompat::json::lenient::{Out, Schema, unmarshal};
use gocompat::time::GoTime;
use gocompat::utf8::decode_rune;

/// How much of a transcript's end is read by default.
pub(crate) const DEFAULT_TAIL_BYTES: u64 = 256 << 10;
/// The one-time backward look for a permission mode.
pub(crate) const DEFAULT_MODE_SCAN_BYTES: u64 = 8 << 20;
/// The longest LastText, in runes.
pub(crate) const MAX_LAST_TEXT: usize = 240;

/// Go `Slug`: Claude Code's project folder name for `cwd`: every rune that
/// is not `[A-Za-z0-9]` becomes '-'.
pub fn slug(cwd: &str) -> String {
    cwd.chars()
        .map(|r| if r.is_ascii_alphanumeric() { r } else { '-' })
        .collect()
}

/// Go `safeSessionID`: keeps glob patterns free of metacharacters.
pub(crate) fn safe_session_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn abs_path(p: &str) -> String {
    gocompat::filepath::abs(p).unwrap_or_else(|_| p.to_string())
}

/// Go `findTranscript`: `<claudeDir>/projects/<slug>/<id>.jsonl`, else the
/// first `projects/*/<id>.jsonl` (directory names in byte order, as Go's
/// Glob sorts them), else "".
pub(crate) fn find_transcript(claude_dir: &str, cwd: &str, id: &str) -> String {
    if !safe_session_id(id) {
        return String::new();
    }
    let file = format!("{id}.jsonl");
    let projects = gocompat::filepath::join(&[claude_dir, "projects"]);
    if !cwd.is_empty() {
        let p = gocompat::filepath::join(&[projects.as_str(), &slug(cwd), &file]);
        if std::fs::metadata(&p).is_ok_and(|m| m.is_file()) {
            return abs_path(&p);
        }
    }
    let Ok(rd) = std::fs::read_dir(&projects) else {
        return String::new();
    };
    let mut names: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for d in names {
        let p = gocompat::filepath::join(&[projects.as_str(), &d, &file]);
        if std::fs::symlink_metadata(&p).is_ok() {
            return abs_path(&p);
        }
    }
    String::new()
}

/// What the end of one transcript says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tail {
    /// Custom, else AI.
    pub title: String,
    /// The newest custom-title.
    pub custom: String,
    /// The newest ai-title.
    pub ai: String,
    pub last_text: String,
    pub last_activity: GoTime,
    pub permission_mode: String,
}

/// Go `transcriptRow`: the tolerant subset of a transcript line.
static ROW: LazyLock<Schema> = LazyLock::new(|| {
    Schema::object(vec![
        ("type", Schema::String),
        ("timestamp", Schema::String),
        ("isSidechain", Schema::Bool),
        ("aiTitle", Schema::String),
        ("customTitle", Schema::String),
        ("permissionMode", Schema::String),
        ("message", Schema::Raw),
    ])
});

static MESSAGE: LazyLock<Schema> = LazyLock::new(|| {
    Schema::object(vec![(
        "content",
        Schema::Slice(Box::new(Schema::object(vec![
            ("type", Schema::String),
            ("text", Schema::String),
        ]))),
    )])
});

struct Row {
    kind: String,
    timestamp: String,
    is_sidechain: bool,
    ai_title: String,
    custom_title: String,
    permission_mode: String,
    message: Option<Vec<u8>>,
}

fn parse_row(line: &[u8]) -> Option<Row> {
    let o = unmarshal(line, &ROW).ok()?;
    Some(Row {
        kind: o.get_str("type").to_string(),
        timestamp: o.get_str("timestamp").to_string(),
        is_sidechain: matches!(o.get("isSidechain"), Some(Out::Bool(true))),
        ai_title: o.get_str("aiTitle").to_string(),
        custom_title: o.get_str("customTitle").to_string(),
        permission_mode: o.get_str("permissionMode").to_string(),
        message: match o.get("message") {
            Some(Out::Raw(r)) => r.clone(),
            _ => None,
        },
    })
}

/// Go `isModeRow`: a "permission-mode" row, or a non-sidechain "user" row
/// with a top-level permissionMode.
fn is_mode_row(r: &Row) -> bool {
    !r.permission_mode.is_empty()
        && !r.is_sidechain
        && (r.kind == "permission-mode" || r.kind == "user")
}

/// Go `bytes.TrimSpace` (Unicode white space, on bytes that may not be
/// UTF-8).
fn trim_space(mut b: &[u8]) -> &[u8] {
    loop {
        let (r, n) = decode_rune(b);
        if n == 0 || !gocompat::text::is_space(r) || (r == '\u{FFFD}' && n == 1) {
            break;
        }
        b = &b[n..];
    }
    'outer: loop {
        for n in 1..=3.min(b.len()) {
            let tail = &b[b.len() - n..];
            let (r, k) = decode_rune(tail);
            if k == n && !(r == '\u{FFFD}' && k == 1 && tail[0] >= 0x80) {
                if gocompat::text::is_space(r) {
                    b = &b[..b.len() - n];
                    continue 'outer;
                }
                break 'outer;
            }
        }
        break;
    }
    b
}

/// Go `bufio.ScanLines` over `data`: lines split at '\n', one trailing '\r'
/// dropped, a final unterminated line kept when non-empty.
fn scan_lines(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let (line, next) = match rest.iter().position(|&b| b == b'\n') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, &rest[rest.len()..]),
        };
        out.push(line.strip_suffix(b"\r").unwrap_or(line));
        rest = next;
    }
    out
}

/// Go `readTail`: parses the last `n` bytes of `f` (`size` bytes long).
/// When the window starts mid-file the first, partial line is skipped.
/// Reading starts one byte early so a window that opens exactly on a row
/// keeps that row.
pub fn read_tail<R: Read + Seek>(f: &mut R, size: u64, n: u64) -> Tail {
    let off = size.saturating_sub(n + 1);
    if f.seek(SeekFrom::Start(off)).is_err() {
        return Tail::default();
    }
    let mut buf = Vec::new();
    if f.take(size - off).read_to_end(&mut buf).is_err() {
        return Tail::default();
    }
    // Go's Scanner (64 KiB buffer, max token n+1) gives up when one
    // unterminated line fills a full-size buffer: only a whole window with
    // no newline at all, which yields nothing either way except here.
    if off == 0 && n + 1 >= 64 << 10 && buf.len() as u64 == n + 1 && !buf.contains(&b'\n') {
        return Tail::default();
    }
    let mut t = Tail::default();
    let (mut custom, mut ai) = (String::new(), String::new());
    for (i, line) in scan_lines(&buf).into_iter().enumerate() {
        if i == 0 && off > 0 {
            continue;
        }
        let line = trim_space(line);
        if line.first() != Some(&b'{') {
            continue;
        }
        let Some(row) = parse_row(line) else {
            continue;
        };
        if let Some(ts) = GoTime::parse_rfc3339(&row.timestamp)
            && instant(&ts) > instant(&t.last_activity)
        {
            t.last_activity = ts;
        }
        if is_mode_row(&row) {
            t.permission_mode = row.permission_mode.clone();
        }
        match row.kind.as_str() {
            "custom-title" if !row.custom_title.is_empty() => custom = row.custom_title,
            "ai-title" if !row.ai_title.is_empty() => ai = row.ai_title,
            // Subagent prose isn't what the session itself last said.
            "assistant" if !row.is_sidechain => {
                let text = last_text_block(row.message.as_deref());
                if !text.is_empty() {
                    t.last_text = text;
                }
            }
            _ => {}
        }
    }
    t.title = if custom.is_empty() {
        ai.clone()
    } else {
        custom.clone()
    };
    t.custom = custom;
    t.ai = ai;
    t
}

/// The `"permissionMode"` key, which pre-filters lines in scan_mode.
const MODE_KEY: &[u8] = b"\"permissionMode\"";

/// Go `scanMode`: the permission mode of the newest mode row in the last
/// `n` bytes of `f`, "" if none.
pub fn scan_mode<R: Read + Seek>(f: &mut R, size: u64, n: u64) -> String {
    let off = size.saturating_sub(n + 1);
    if f.seek(SeekFrom::Start(off)).is_err() {
        return String::new();
    }
    let mut buf = vec![0u8; (size - off) as usize];
    if f.read_exact(&mut buf).is_err() {
        return String::new();
    }
    let mut buf: &[u8] = &buf;
    if off > 0 {
        let Some(i) = buf.iter().position(|&b| b == b'\n') else {
            return String::new();
        };
        buf = &buf[i + 1..];
    }
    while !buf.is_empty() {
        let line;
        if let Some(j) = buf.iter().rposition(|&b| b == b'\n') {
            line = &buf[j + 1..];
            buf = &buf[..j];
        } else {
            line = buf;
            buf = &[];
        }
        let line = trim_space(line);
        if line.first() != Some(&b'{') || !line.windows(MODE_KEY.len()).any(|w| w == MODE_KEY) {
            continue;
        }
        if let Some(row) = parse_row(line)
            && is_mode_row(&row)
        {
            return row.permission_mode;
        }
    }
    String::new()
}

/// Go `lastTextBlock`: the last non-empty text block of an assistant
/// message, on one line and capped at MAX_LAST_TEXT runes.
fn last_text_block(raw: Option<&[u8]>) -> String {
    let Some(raw) = raw.filter(|r| !r.is_empty()) else {
        return String::new();
    };
    let Ok(msg) = unmarshal(raw, &MESSAGE) else {
        return String::new();
    };
    let Some(content) = msg.get("content") else {
        return String::new();
    };
    for b in content.items().iter().rev() {
        if b.get_str("type") == "text" {
            let s = one_line(b.get_str("text"), MAX_LAST_TEXT);
            if !s.is_empty() {
                return s;
            }
        }
    }
    String::new()
}

/// Go `oneLine`: runs of white space become single spaces, capped at
/// `limit` runes with an ellipsis marking a cut.
pub fn one_line(s: &str, limit: usize) -> String {
    let s = s
        .split(gocompat::text::is_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let r: Vec<char> = s.chars().collect();
    if r.len() <= limit {
        return s;
    }
    let cut: String = r[..limit - 1].iter().collect();
    format!("{}…", cut.trim_end_matches(' '))
}

/// An instant, for ordering (Go's Before/After/Compare).
pub(crate) fn instant(t: &GoTime) -> (i64, u32) {
    (t.secs, t.nanos)
}

/// Go `filepath.Base` of an event path, enough for names: the text after
/// the last separator.
pub(crate) fn base_name(path: &str) -> &str {
    let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let p = path.trim_end_matches(seps);
    match p.rfind(seps) {
        Some(i) => &p[i + 1..],
        None => p,
    }
}

/// Go `transcriptID`: the session id of a top-level transcript path
/// (`<slug>/<id>.jsonl`); subagent transcripts and other files don't map.
pub(crate) fn transcript_id(path: &str) -> Option<String> {
    let id = base_name(path).strip_suffix(".jsonl")?;
    safe_session_id(id).then(|| id.to_string())
}
