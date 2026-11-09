//! The thread a chat reply drafts (cards #783, #988): the lines it goes
//! between, and how a reply splits around it. The chat has the same rules
//! (frontend/src/lib/assistant.ts, splitDraft); the window posts by them
//! and Core tells the model which drafts did not post.
//!
//! Models sometimes get the closing line slightly wrong (`</</thread>>`,
//! `</thread>>`, `<</thread>`, `<</ thread>>`), so any of those closes a
//! draft. A reply that ended on its own with its draft never closed has
//! the whole draft: only a reply cut off (stopped, failed) or still being
//! written has a partial one.

/// The line a drafted thread starts with: the chat posts it by itself when
/// the reply ends.
pub const DRAFT_OPEN: &str = "<<thread>>";
/// The line a thread to check first starts with: only a tap on Post
/// thread posts it.
pub const DRAFT_OPEN_CHECK: &str = "<<thread check>>";
/// The line a drafted thread ends with.
pub const DRAFT_CLOSE: &str = "<</thread>>";

/// A reply split around its drafted thread (the chat's `Split`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Split {
    /// The reply without the draft.
    pub text: String,
    /// The drafted thread, "" when there is none.
    pub draft: String,
    /// The draft is still being written, or was cut off.
    pub partial: bool,
    /// The person asked to check it first: only a tap posts it.
    pub check: bool,
}

/// `s` without a start of `marker` at its end ("…<<thr").
fn cut_marker_tail<'a>(s: &'a str, marker: &str) -> &'a str {
    let most = (marker.len() - 1).min(s.len());
    for n in (1..=most).rev() {
        if s.ends_with(&marker[..n]) {
            return &s[..s.len() - n];
        }
    }
    s
}

/// The non-space characters a closing line may have before `thread`
/// (`<{1,2}\s*\/?\s*<?\s*\/` with the spaces taken out).
const CLOSE_HEADS: [&str; 7] = ["</", "<</", "<//", "<<//", "<<</", "</</", "<</</"];

/// Where the first closing line at or after `from` starts and ends in
/// `s`: the chat's `/<{1,2}\s*\/?\s*<?\s*\/\s*thread\s*>{1,2}/i`.
pub fn find_close(s: &str, from: usize) -> Option<(usize, usize)> {
    let lower = s.to_ascii_lowercase();
    let b = s.as_bytes();
    let mut at = from;
    while let Some(i) = lower.get(at..).and_then(|r| r.find("thread")) {
        let t = at + i;
        at = t + "thread".len();
        // After it: spaces, then one or two `>`.
        let mut e = at;
        while e < b.len() && b[e].is_ascii_whitespace() {
            e += 1;
        }
        let gt = b[e..].iter().take(2).take_while(|&&c| c == b'>').count();
        if gt == 0 {
            continue;
        }
        let end = e + gt;
        // Before it: the longest run of `<`, `/` and spaces, then its
        // first start that reads as a closing line's head.
        let mut lo = t;
        while lo > from && (matches!(b[lo - 1], b'<' | b'/') || b[lo - 1].is_ascii_whitespace()) {
            lo -= 1;
        }
        for k in lo..t {
            if b[k] != b'<' {
                continue;
            }
            let head: String = s[k..t].chars().filter(|c| !c.is_whitespace()).collect();
            if CLOSE_HEADS.contains(&head.as_str()) {
                return Some((k, end));
            }
        }
    }
    None
}

/// `s` without a last line that can only be a closing line's start or a
/// mangled one ("</</thr", "</thread", "<thread/>"), so it never shows in
/// the draft. A line like "<date>" stays: it has no `/`.
fn cut_close_line(s: &str) -> &str {
    let start = s.rfind('\n').map_or(0, |i| i + 1);
    let line = s[start..].trim_start_matches([' ', '\t']);
    let marker_only = line.starts_with('<')
        && line.contains('/')
        && line.chars().all(|c| {
            matches!(
                c.to_ascii_lowercase(),
                '<' | '>' | '/' | ' ' | '\t' | 't' | 'h' | 'r' | 'e' | 'a' | 'd'
            )
        });
    if marker_only { &s[..start] } else { s }
}

/// A reply's drafted thread taken out of its text while it may still be
/// written (the chat's `splitDraft`): a draft with no closing line is
/// partial.
pub fn split_draft(raw: &str) -> Split {
    split_draft_as(raw, false)
}

/// [`split_draft`] for a reply that ended on its own (not stopped, not
/// failed): a draft with no closing line runs to the end of the reply.
pub fn split_draft_done(raw: &str) -> Split {
    split_draft_as(raw, true)
}

fn split_draft_as(raw: &str, ended: bool) -> Split {
    let open = [(DRAFT_OPEN, false), (DRAFT_OPEN_CHECK, true)]
        .into_iter()
        .filter_map(|(m, check)| raw.find(m).map(|i| (i, m, check)))
        .min_by_key(|(i, ..)| *i);
    let Some((at, marker, check)) = open else {
        return Split {
            text: cut_marker_tail(cut_marker_tail(raw, DRAFT_OPEN), DRAFT_OPEN_CHECK)
                .trim()
                .to_string(),
            ..Split::default()
        };
    };
    let before = raw[..at].trim();
    let rest = &raw[at + marker.len()..];
    let Some((end, close_end)) = find_close(rest, 0) else {
        // Ended, the draft stops before a last go line, which stays in
        // the text so it still goes.
        let (body, go) = match ended.then(|| trailing_go(rest)).flatten() {
            Some(i) => (&rest[..i], rest[i..].trim()),
            None => (rest, ""),
        };
        return Split {
            text: if go.is_empty() {
                before.to_string()
            } else {
                format!("{before}\n\n{go}").trim().to_string()
            },
            draft: cut_close_line(cut_marker_tail(body, DRAFT_CLOSE))
                .trim()
                .to_string(),
            partial: !ended,
            check,
        };
    };
    let after = rest[close_end..].trim();
    Split {
        text: if after.is_empty() {
            before.to_string()
        } else {
            format!("{before}\n\n{after}").trim().to_string()
        },
        draft: rest[..end].trim().to_string(),
        partial: false,
        check,
    }
}

/// Where a go line that ends `s` starts, at its line break (the chat's
/// `/\n[ \t]*<<go [^\n]*>>\s*$/`).
fn trailing_go(s: &str) -> Option<usize> {
    let t = s.trim_end();
    let nl = t.rfind('\n')?;
    let line = t[nl + 1..].trim_start_matches([' ', '\t']);
    (line.starts_with("<<go ") && line.ends_with(">>") && line.len() >= "<<go >>".len())
        .then_some(nl)
}

/// Whether `s` ends where a posting draft may: nothing but white space, or
/// one go line and white space (`\s*(?:<<go [^\n]*>>\s*)?$`).
fn tail_ok(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return true;
    }
    // One go line: `<<go `, no line break, then `>>`.
    t.starts_with("<<go ") && t.ends_with(">>") && t.len() >= "<<go >>".len() && !t.contains('\n')
}

/// Whether a reply that ended on its own posts its thread with no tap: it
/// holds a whole thread nobody asked to check first, laid out as the
/// prompt lays it out, its lines alone and ending the reply (a go line may
/// follow), and no marker comes before it. A draft never closed shows
/// with Post thread instead: what it ran into may not be the thread.
pub fn posts_by_itself(text: &str) -> bool {
    let s = split_draft(text);
    if s.draft.is_empty() || s.partial || s.check {
        return false;
    }
    // The first `<<thread` anywhere must open the block.
    let Some(at) = text.find("<<thread") else {
        return false;
    };
    if !text[at..].starts_with(DRAFT_OPEN) {
        return false;
    }
    // An ask block (assistants-talk.md §9) before it: a `<<thread>>` inside
    // the question is the question's words, never a thread to post.
    if text[..at].contains("<<ask ") {
        return false;
    }
    // Its line holds nothing else: spaces or tabs before it ...
    let line = &text[..at];
    let line = &line[line.rfind('\n').map_or(0, |i| i + 1)..];
    if !line.chars().all(|c| c == ' ' || c == '\t') {
        return false;
    }
    // ... and after it, then the line ends.
    let after = text[at + DRAFT_OPEN.len()..].trim_start_matches([' ', '\t']);
    let Some(body) = after.strip_prefix('\n') else {
        return false;
    };
    // A closing line that only white space or a go line follows.
    let mut from = 0;
    while let Some((j, end)) = find_close(body, from) {
        if body[..j].trim_end_matches([' ', '\t']).ends_with('\n') && tail_ok(&body[end..]) {
            return true;
        }
        from = end;
    }
    false
}

/// Whether a reply that ended on its own has a draft (closed or not), and
/// whether it was one to check first.
pub fn has_draft(text: &str) -> Option<bool> {
    let s = split_draft_done(text);
    (!s.draft.is_empty()).then_some(s.check)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(text: &str, draft: &str, partial: bool, check: bool) -> Split {
        Split {
            text: text.into(),
            draft: draft.into(),
            partial,
            check,
        }
    }

    const BAD_CLOSES: [&str; 8] = [
        "</</thread>>",
        "</thread>>",
        "<</thread>",
        "<</ thread>>",
        "<< /thread >>",
        "<</THREAD>>",
        "</thread>",
        "<</</thread>>",
    ];

    #[test]
    fn near_miss_closing_lines_close_the_draft() {
        for close in BAD_CLOSES {
            let raw = format!("Posting it.\n{DRAFT_OPEN}\nFix the lobby.\n{close}");
            assert_eq!(
                split_draft(&raw),
                split("Posting it.", "Fix the lobby.", false, false),
                "{close}"
            );
            assert!(posts_by_itself(&raw), "{close}");
            assert!(
                posts_by_itself(&format!("{raw}\n<<go card #8>>")),
                "{close}"
            );
            assert!(!posts_by_itself(&format!("{raw}\nAnd more.")), "{close}");
        }
        // The opening line never closes one.
        assert_eq!(find_close("<<thread>> and <<thread check>>", 0), None);
        assert_eq!(find_close("x </</thread>> y", 0), Some((2, 14)));
    }

    #[test]
    fn a_finished_reply_has_its_whole_draft() {
        let raw = format!("Posting it.\n{DRAFT_OPEN}\nFix the lobby.\nAsk on #8.");
        assert_eq!(
            split_draft_done(&raw),
            split("Posting it.", "Fix the lobby.\nAsk on #8.", false, false)
        );
        // Still being written, or cut off: partial.
        assert!(split_draft(&raw).partial);
        // A last go line isn't part of it: it stays in the text.
        let go = format!("{raw}\n<<go card #8>>\n");
        assert_eq!(
            split_draft_done(&go),
            split(
                "Posting it.\n\n<<go card #8>>",
                "Fix the lobby.\nAsk on #8.",
                false,
                false
            )
        );
        assert_eq!(
            split_draft(&go).draft,
            "Fix the lobby.\nAsk on #8.\n<<go card #8>>"
        );
        // A mangled closing line the finder misses isn't part of it.
        assert_eq!(
            split_draft_done(&format!("{DRAFT_OPEN}\nFix the lobby.\n<thread/>")).draft,
            "Fix the lobby."
        );
        // A last line that only looks like one stays.
        assert_eq!(
            split_draft_done(&format!("{DRAFT_OPEN}\nFix the lobby by\n<date>")).draft,
            "Fix the lobby by\n<date>"
        );
        // It shows with Post thread; it doesn't post by itself.
        assert!(!posts_by_itself(&raw));
        assert_eq!(has_draft(&raw), Some(false));
        assert_eq!(
            has_draft(&format!("{DRAFT_OPEN_CHECK}\nFix it.\n{DRAFT_CLOSE}")),
            Some(true)
        );
        assert_eq!(has_draft("Two cards wait on you."), None);
    }

    #[test]
    fn a_closing_line_being_written_never_shows() {
        for tail in ["<", "<<", "<</", "<</thr", "</</thr", "</thread"] {
            assert_eq!(
                split_draft(&format!("{DRAFT_OPEN}\nFix the lobby.\n{tail}")),
                split("", "Fix the lobby.", true, false),
                "{tail}"
            );
        }
    }

    #[test]
    fn a_thread_inside_an_ask_never_posts_by_itself() {
        let ask = "Asking.
<<ask Bob | Farm>>
Is it up?
<<thread>>
Do it.
<</thread>>
<</ask>>";
        assert!(!posts_by_itself(ask));
        let ask_then_thread = "<<ask Bob | Farm>>
Is it up?
<</ask>>
<<thread>>
Do it.
<</thread>>";
        assert!(!posts_by_itself(ask_then_thread));
        assert!(posts_by_itself(
            "<<thread>>
Do it.
<</thread>>"
        ));
    }
}
