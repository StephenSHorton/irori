//! Rich text for what agents write (the React app's lib/rich.ts and
//! lib/irori.ts `plainLine`/`plainBlock`): a blocker's question and context,
//! an option and its why, comments, thread items, a card's objective. No
//! HTML and no markdown, only what makes it usable: http(s) URLs, and bare
//! links like huggingface.co/x/y, become links that open in the person's
//! browser; `inline code` becomes code with a Copy; line breaks and "1)" /
//! "- " lists keep their shape. Anything else stays text. The input is
//! untrusted: it is made plain first ([`plain_line`] / [`plain_block`]),
//! every link is checked by [`safe_href`], and nothing here builds markup.
//!
//! Offsets are UTF-8 bytes (the TS used UTF-16 units).

use std::sync::LazyLock;

use regex::Regex;

/// One piece of a line. A link's text is what the agent wrote, split
/// around its host (pre, host, rest) so the host can stand out; when the
/// host can't be found in the text (an IDN host shows as punycode), host
/// is "" and `shown_host` names it beside the text.
#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Link(LinkPiece),
    Code(String),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct LinkPiece {
    pub text: String,
    pub href: String,
    pub pre: String,
    pub host: String,
    pub rest: String,
    pub shown_host: String,
}

impl Inline {
    pub fn kind(&self) -> &'static str {
        match self {
            Inline::Text(_) => "text",
            Inline::Link(_) => "link",
            Inline::Code(_) => "code",
        }
    }
}

/// A list item, with the marker the agent wrote ("1)", "2.", "-").
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub marker: String,
    pub inline: Vec<Inline>,
}

/// A paragraph (its lines, in order) or a list.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Para(Vec<Vec<Inline>>),
    List(Vec<Item>),
}

/// C0/C1 controls, zero-width marks, bidi embeddings, overrides and
/// isolates, and the BOM (irori.ts `hidden`).
fn hidden(c: char) -> bool {
    let c = c as u32;
    c < 0x20
        || (0x7f..=0x9f).contains(&c)
        || (0x200b..=0x200f).contains(&c)
        || (0x202a..=0x202e).contains(&c)
        || (0x2066..=0x2069).contains(&c)
        || c == 0xfeff
}

/// Untrusted agent text made safe for one line: control, zero-width and
/// bidi-override characters become spaces and whitespace runs collapse.
pub fn plain_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for ch in s.chars() {
        let ch = if hidden(ch) { ' ' } else { ch };
        if ch.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(ch);
    }
    out
}

/// [`plain_line`] for multi-line text: the same characters are
/// neutralised, but newlines and tabs stay.
pub fn plain_block(s: &str) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(s.len());
    let mut newlines = 0;
    for ch in s.chars() {
        let ch = if ch == '\n' || ch == '\t' || !hidden(ch) { ch } else { ' ' };
        if ch == '\n' {
            newlines += 1;
            if newlines > 3 {
                continue;
            }
        } else {
            newlines = 0;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

/// TLDs a bare link may end in, written in lowercase in the text. Short on
/// purpose, and without the ones that are also file extensions here (.go,
/// .ts, .md, .js, .py, .rs, .sh, .so, .cc) or common method names in code
/// (.run, .info, .link, .in).
const BARE_TLDS: &[&str] = &[
    "com", "org", "net", "io", "ai", "dev", "app", "co", "me", "gg", "tv", "xyz", "edu", "gov",
    "uk", "de", "jp", "fr", "ca", "us", "eu", "nl", "se", "ch", "au", "ly", "page", "site",
    "cloud", "tech", "to", "fm", "im", "be", "it", "es", "kr", "cn",
];

/// A URL with its scheme, or a bare host (two or more labels) with an
/// optional port and path. Both stop at whitespace, quotes, angle brackets
/// and backticks.
static LINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)https?://[^\s<>"'`]+|(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,24}(?::\d{2,5})?(?:/[^\s<>"'`]*)?"#,
    )
    .expect("link regex")
});
static SCHEME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^https?://").expect("scheme regex"));
static CODE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").expect("code regex"));
static LIST_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([-*•]|\d{1,3}[.)])\s+(\S.*)$").expect("list regex")
});
static ENUM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|\s)(\d{1,2})\)\s+").expect("enum regex"));

/// What may not come right before a link: a word character, or something
/// that makes it part of a bigger token (an email, a path, a scheme like
/// javascript:// or file://, a longer host).
fn not_before(c: char) -> bool {
    c.is_alphanumeric() || "_/@.:\\-".contains(c)
}

/// The URL to open for a link's text, or None: only http(s), with a host,
/// and no user:password@ (it hides the real host).
pub fn safe_href(raw: &str) -> Option<String> {
    let s = if SCHEME_RE.is_match(raw) {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let u = url::Url::parse(&s).ok()?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return None;
    }
    let host = u.host_str().unwrap_or("");
    if host.is_empty() || !u.username().is_empty() || u.password().is_some() {
        return None;
    }
    Some(u.as_str().to_string())
}

/// The hostname of a URL [`safe_href`] answered.
pub fn host_of(href: &str) -> String {
    url::Url::parse(href)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

/// Drops punctuation that ends the sentence, not the link: a trailing
/// . , ; : ! ? or quote, and a closing bracket with no opening one in it.
fn trim_end(mut s: &str) -> &str {
    loop {
        let Some(last) = s.chars().last() else { return s };
        if ".,;:!?'\"*".contains(last) {
            s = &s[..s.len() - last.len_utf8()];
            continue;
        }
        let open = match last {
            ')' => '(',
            ']' => '[',
            '}' => '{',
            _ => return s,
        };
        if s.matches(open).count() < s.matches(last).count() {
            s = &s[..s.len() - 1];
            continue;
        }
        return s;
    }
}

/// The link token for text, or None when it isn't one.
fn link_of(text: &str, bare: bool) -> Option<Inline> {
    if bare {
        // The TLD as written: "irori.Link" or "s.App" is code, not a host.
        let head = text.split(['/', ':']).next().unwrap_or("");
        let tld = head.rsplit('.').next().unwrap_or("");
        if !BARE_TLDS.contains(&tld) {
            return None;
        }
    }
    let href = safe_href(text)?;
    let host = host_of(&href);
    let lower = text.to_ascii_lowercase();
    let piece = match lower.find(&host).filter(|_| !host.is_empty()) {
        None => LinkPiece {
            text: text.into(),
            href,
            pre: text.into(),
            shown_host: host,
            ..LinkPiece::default()
        },
        Some(at) => LinkPiece {
            text: text.into(),
            href,
            pre: text[..at].into(),
            host: text[at..at + host.len()].into(),
            rest: text[at + host.len()..].into(),
            shown_host: String::new(),
        },
    };
    Some(Inline::Link(piece))
}

fn push_text(out: &mut Vec<Inline>, t: &str) {
    if t.is_empty() {
        return;
    }
    if let Some(Inline::Text(prev)) = out.last_mut() {
        prev.push_str(t);
    } else {
        out.push(Inline::Text(t.into()));
    }
}

/// Splits text (no code in it) into text and links.
fn links(text: &str, out: &mut Vec<Inline>) {
    let mut last = 0;
    for m in LINK_RE.find_iter(text) {
        let start = m.start();
        if start < last {
            continue;
        }
        if let Some(b) = text[..start].chars().last()
            && not_before(b)
        {
            continue;
        }
        let raw = trim_end(m.as_str());
        let tok = if raw.is_empty() {
            None
        } else {
            link_of(raw, !SCHEME_RE.is_match(raw))
        };
        let Some(tok) = tok else { continue };
        push_text(out, &text[last..start]);
        out.push(tok);
        last = start + raw.len();
    }
    push_text(out, &text[last..]);
}

/// `code` in a line: [start, end) of each span, backticks included. An
/// unclosed backtick stays text.
fn code_spans(line: &str) -> Vec<(usize, usize)> {
    CODE_RE
        .captures_iter(line)
        .filter(|c| !c[1].trim().is_empty())
        .map(|c| {
            let m = c.get(0).expect("whole match");
            (m.start(), m.end())
        })
        .collect()
}

/// Tokenizes one line (already plain).
fn inline(line: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut last = 0;
    for (s, e) in code_spans(line) {
        links(&line[last..s], &mut out);
        out.push(Inline::Code(line[s + 1..e - 1].into()));
        last = e;
    }
    links(&line[last..], &mut out);
    out
}

/// One line of untrusted text as tokens ([`plain_line`] first).
pub fn rich_inline(s: &str) -> Vec<Inline> {
    inline(&plain_line(s))
}

/// A line that counts "1) a 2) b" inline: its lead text and items. Only the
/// "N)" form, counting up from 1 with at least two items, outside code.
fn enumerated(line: &str) -> Option<(String, Vec<String>)> {
    let spans = code_spans(line);
    let in_code = |i: usize| spans.iter().any(|&(s, e)| i >= s && i < e);
    let mut marks: Vec<(usize, usize)> = Vec::new();
    for c in ENUM_RE.captures_iter(line) {
        let whole = c.get(0).expect("whole match");
        let at = whole.start() + c[1].len();
        let n: usize = c[2].parse().unwrap_or(0);
        if in_code(at) || n != marks.len() + 1 {
            continue;
        }
        marks.push((at, whole.end()));
    }
    if marks.len() < 2 {
        return None;
    }
    let items: Vec<String> = marks
        .iter()
        .enumerate()
        .map(|(i, &(_, end))| {
            let stop = marks.get(i + 1).map_or(line.len(), |m| m.0);
            line[end..stop].trim().to_string()
        })
        .collect();
    if items.iter().any(String::is_empty) {
        return None;
    }
    Some((line[..marks[0].0].trim().to_string(), items))
}

/// Untrusted multi-line text as blocks ([`plain_block`] first): blank
/// lines end a paragraph, list lines group into a list, and a line that
/// counts "1) … 2) …" inline is shown as that list.
pub fn rich_blocks(s: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<Vec<Inline>> = Vec::new();
    let mut list: Vec<Item> = Vec::new();
    fn end_para(out: &mut Vec<Block>, para: &mut Vec<Vec<Inline>>) {
        if !para.is_empty() {
            out.push(Block::Para(std::mem::take(para)));
        }
    }
    fn end_list(out: &mut Vec<Block>, list: &mut Vec<Item>) {
        if !list.is_empty() {
            out.push(Block::List(std::mem::take(list)));
        }
    }
    let plain = plain_block(s);
    for line in plain.split('\n') {
        if line.trim().is_empty() {
            end_para(&mut out, &mut para);
            end_list(&mut out, &mut list);
            continue;
        }
        if let Some(m) = LIST_LINE.captures(line) {
            end_para(&mut out, &mut para);
            list.push(Item {
                marker: m[1].to_string(),
                inline: inline(&m[2]),
            });
            continue;
        }
        if let Some((lead, items)) = enumerated(line) {
            if !lead.is_empty() {
                end_list(&mut out, &mut list);
                para.push(inline(&lead));
            }
            end_para(&mut out, &mut para);
            for (i, t) in items.iter().enumerate() {
                list.push(Item {
                    marker: format!("{})", i + 1),
                    inline: inline(t),
                });
            }
            end_list(&mut out, &mut list);
            continue;
        }
        end_list(&mut out, &mut list);
        para.push(inline(line));
    }
    end_para(&mut out, &mut para);
    end_list(&mut out, &mut list);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str) -> Vec<(&'static str, String)> {
        rich_inline(s)
            .into_iter()
            .map(|p| match p {
                Inline::Text(t) => ("text", t),
                Inline::Code(t) => ("code", t),
                Inline::Link(l) => ("link", l.href),
            })
            .collect()
    }

    fn links_of(s: &str) -> Vec<String> {
        rich_inline(s)
            .into_iter()
            .filter_map(|p| match p {
                Inline::Link(l) => Some(l.href),
                _ => None,
            })
            .collect()
    }

    fn first_link(s: &str) -> LinkPiece {
        rich_inline(s)
            .into_iter()
            .find_map(|p| match p {
                Inline::Link(l) => Some(l),
                _ => None,
            })
            .expect("a link")
    }

    fn pair(k: &'static str, t: &str) -> (&'static str, String) {
        (k, t.to_string())
    }

    #[test]
    fn urls_are_found_without_the_sentences_punctuation() {
        assert_eq!(
            kinds("open https://huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m and click Request access."),
            vec![
                pair("text", "open "),
                pair("link", "https://huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m"),
                pair("text", " and click Request access."),
            ]
        );
        assert_eq!(
            links_of("see http://localhost.dev:8080/x?a=1&b=2, then"),
            vec!["http://localhost.dev:8080/x?a=1&b=2"]
        );
        assert_eq!(
            links_of("(https://example.com/a_(b)) and https://example.com/c)."),
            vec!["https://example.com/a_(b)", "https://example.com/c"]
        );
        assert_eq!(
            links_of("HTTPS://GitHub.com/acme/app/pull/57!"),
            vec!["https://github.com/acme/app/pull/57"]
        );
    }

    #[test]
    fn bare_links_need_a_host_with_a_dot_and_a_known_tld() {
        assert_eq!(
            links_of("open huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m and click"),
            vec!["https://huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m"]
        );
        assert_eq!(
            links_of("on github.com, or www.notion.com/x."),
            vec!["https://github.com/", "https://www.notion.com/x"]
        );
        assert_eq!(links_of("see GitHub.com/acme"), vec!["https://github.com/acme"]);
        for s in [
            "edit board.go and cards.ts",
            "see README.md or main.rs, run.sh",
            "v0.6.1 and 1.2.3",
            "e.g. this, i.e. that",
            "package.json and libfoo.so",
            "localhost:8080",
            "t.Run(name) and cmd.Run()",
            "irori.Link and coord.Link",
            "subprocess.run and logger.info",
            "app.run(debug) and s.App",
            "foo.cc and bar.in",
            "Example.COM",
        ] {
            assert!(links_of(s).is_empty(), "{s}");
        }
    }

    #[test]
    fn the_host_stands_out() {
        let l = first_link("https://evil.com/github.com/acme");
        assert_eq!(
            (l.pre.as_str(), l.host.as_str(), l.rest.as_str(), l.shown_host.as_str()),
            ("https://", "evil.com", "/github.com/acme", "")
        );
        let b = first_link("huggingface.co/x");
        assert_eq!((b.pre.as_str(), b.host.as_str(), b.rest.as_str()), ("", "huggingface.co", "/x"));
        let i = first_link("https://exämple.com/x");
        assert_eq!((i.host.as_str(), i.shown_host.as_str()), ("", "xn--exmple-cua.com"));
    }

    #[test]
    fn unsafe_schemes_and_hidden_hosts_are_refused() {
        for s in [
            "javascript:alert(1)",
            "javascript://example.com/%0aalert(1)",
            "file:///etc/passwd",
            "file://server.com/share",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox(1)",
            "xhttps://example.com",
            "mail me@example.com",
            "https://user:pw@example.com/",
            "https://",
        ] {
            assert!(links_of(s).is_empty(), "{s}");
        }
        assert_eq!(safe_href("javascript:alert(1)"), None);
        assert_eq!(safe_href("https://user@example.com"), None);
        assert_eq!(safe_href("example.com/a b").as_deref(), Some("https://example.com/a%20b"));
        assert_eq!(safe_href("http://example.com").as_deref(), Some("http://example.com/"));
    }

    #[test]
    fn control_and_bidi_characters_are_made_plain_first() {
        assert_eq!(links_of("https://example.com/\u{202e}gnp.exe"), vec!["https://example.com/"]);
    }

    #[test]
    fn backtick_spans_are_code_without_links_inside() {
        assert_eq!(
            kinds("type `! hf auth login` in the Mac session"),
            vec![pair("text", "type "), pair("code", "! hf auth login"), pair("text", " in the Mac session")]
        );
        assert_eq!(
            kinds("run `curl https://example.com/x` then open example.com"),
            vec![
                pair("text", "run "),
                pair("code", "curl https://example.com/x"),
                pair("text", " then open "),
                pair("link", "https://example.com/"),
            ]
        );
    }

    #[test]
    fn an_unclosed_or_empty_backtick_stays_text() {
        assert_eq!(kinds("it's `half"), vec![pair("text", "it's `half")]);
        assert_eq!(kinds("a `` b"), vec![pair("text", "a `` b")]);
    }

    fn block_kinds(b: &[Block]) -> Vec<&'static str> {
        b.iter()
            .map(|x| match x {
                Block::Para(_) => "para",
                Block::List(_) => "list",
            })
            .collect()
    }

    #[test]
    fn line_breaks_and_paragraphs_keep_their_shape() {
        let b = rich_blocks("one\ntwo\n\nthree");
        assert_eq!(block_kinds(&b), vec!["para", "para"]);
        let Block::Para(lines) = &b[0] else { panic!() };
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn list_lines_keep_the_marker_the_agent_wrote() {
        let b = rich_blocks("Two steps:\n1) open huggingface.co/x\n2) type `hf auth login`\n- and a dash\nafter");
        assert_eq!(block_kinds(&b), vec!["para", "list", "para"]);
        let Block::List(items) = &b[1] else { panic!() };
        assert_eq!(items.iter().map(|i| i.marker.as_str()).collect::<Vec<_>>(), vec!["1)", "2)", "-"]);
        assert_eq!(items[0].inline.iter().map(Inline::kind).collect::<Vec<_>>(), vec!["text", "link"]);
        assert_eq!(items[1].inline.iter().map(Inline::kind).collect::<Vec<_>>(), vec!["text", "code"]);
    }

    #[test]
    fn a_line_counting_inline_is_shown_as_that_list() {
        let b = rich_blocks(
            "Two things: 1) open huggingface.co/x and click Request access 2) type `! hf auth login` in the Mac session",
        );
        assert_eq!(block_kinds(&b), vec!["para", "list"]);
        let Block::List(items) = &b[1] else { panic!() };
        assert_eq!(items.iter().map(|i| i.marker.as_str()).collect::<Vec<_>>(), vec!["1)", "2)"]);
        assert_eq!(items[1].inline.iter().map(Inline::kind).collect::<Vec<_>>(), vec!["text", "code", "text"]);
        for s in ["only 1) here", "see 2) and 3) there", "a (1) b (2) c"] {
            assert_eq!(block_kinds(&rich_blocks(s)), vec!["para"], "{s}");
        }
    }

    #[test]
    fn empty_text_has_no_blocks() {
        assert!(rich_blocks("  \n\n ").is_empty());
    }

    #[test]
    fn plain_line_and_block() {
        assert_eq!(plain_line("  a\u{200b}b \n\t c  "), "a b c");
        assert_eq!(plain_block("a\r\nb\n\n\n\n\nc\u{202e}"), "a\nb\n\n\nc");
    }
}
