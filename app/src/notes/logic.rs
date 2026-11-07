//! Notes' pure logic (the React app's lib/notes.ts, non-sample, and
//! components/notes/parts.tsx's `sourceLabel`), with lib/notes.test.ts's
//! cases.

use irori_window::notes::NoteView;

/// The kinds a note may be, in the chips' order.
pub const NOTE_KINDS: [&str; 5] = ["rule", "gotcha", "decision", "pointer", "preference"];

/// The most characters a note's text may have (§11: refused over 500).
pub const NOTE_MAX_CHARS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagShape {
    Everywhere,
    Board,
    Repo,
    Path,
    Machine,
    Topic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub shape: TagShape,
    pub value: String,
    pub raw: String,
}

/// One tag; None when it isn't a known shape. Repo keys and paths keep
/// their case; the rest fold to lower case, as the fold does.
pub fn parse_tag(raw: &str) -> Option<Tag> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    if t.eq_ignore_ascii_case("everywhere") {
        return Some(Tag {
            shape: TagShape::Everywhere,
            value: String::new(),
            raw: "everywhere".into(),
        });
    }
    let i = t.find(':')?;
    if i == 0 {
        return None;
    }
    let head = t[..i].to_lowercase();
    let rest = t[i + 1..].trim();
    if rest.is_empty() {
        return None;
    }
    match head.as_str() {
        "repo" => Some(Tag {
            shape: TagShape::Repo,
            value: rest.into(),
            raw: format!("repo:{rest}"),
        }),
        "path" => {
            // path:<repoKey>:<path>: the path is after the last colon.
            let j = rest.rfind(':')?;
            if j == 0 || j == rest.len() - 1 {
                return None;
            }
            Some(Tag {
                shape: TagShape::Path,
                value: rest.into(),
                raw: format!("path:{rest}"),
            })
        }
        "board" | "machine" | "topic" => {
            let v = rest.to_lowercase();
            let shape = match head.as_str() {
                "board" => TagShape::Board,
                "machine" => TagShape::Machine,
                _ => TagShape::Topic,
            };
            Some(Tag {
                shape,
                raw: format!("{head}:{v}"),
                value: v,
            })
        }
        _ => None,
    }
}

/// A repo key's short name: remote:github.com/o/irori is irori.
pub fn repo_name(key: &str) -> String {
    let k = key.strip_prefix("remote:").unwrap_or(key);
    let k = k.strip_suffix(".git").unwrap_or(k);
    if let Some(r) = k.strip_prefix("root:") {
        return r.chars().take(7).collect();
    }
    match k.rsplit('/').next() {
        Some(last) if !last.is_empty() => last.to_string(),
        _ => k.to_string(),
    }
}

/// How a tag reads on a row: "repo irori", "on the PC", "irori frontend/"
/// for a path, the space's name for a board: tag.
pub fn tag_label(raw: &str, board_name: &dyn Fn(&str) -> String) -> String {
    let Some(t) = parse_tag(raw) else {
        return raw.to_string();
    };
    match t.shape {
        TagShape::Everywhere => "everywhere".into(),
        TagShape::Repo => format!("repo {}", repo_name(&t.value)),
        TagShape::Path => {
            let j = t.value.rfind(':').unwrap_or(0);
            format!("{} {}", repo_name(&t.value[..j]), &t.value[j + 1..])
        }
        TagShape::Board => {
            let n = board_name(&t.value);
            if n.is_empty() { t.value } else { n }
        }
        TagShape::Machine => format!("on {}", t.value),
        TagShape::Topic => format!("#{}", t.value),
    }
}

/// Tags as typed, repeats dropped; `bad` holds the ones that aren't a
/// known shape.
pub fn clean_tags<S: AsRef<str>>(input: &[S]) -> (Vec<String>, Vec<String>) {
    let (mut tags, mut bad) = (Vec::new(), Vec::new());
    for raw in input {
        let raw = raw.as_ref();
        match parse_tag(raw) {
            Some(t) => {
                if !tags.contains(&t.raw) {
                    tags.push(t.raw);
                }
            }
            None => {
                if !raw.trim().is_empty() {
                    bad.push(raw.trim().to_string());
                }
            }
        }
    }
    (tags, bad)
}

/// The form's "Applies to" field split into tags (commas or newlines).
pub fn split_tags(text: &str) -> Vec<String> {
    text.split([',', '\n'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Why New note or Edit can't send yet, the same checks the write makes
/// (§11), or "".
pub fn note_problem<S: AsRef<str>>(kind: &str, text: &str, tags: &[S]) -> String {
    if !NOTE_KINDS.contains(&kind) {
        return "Pick a kind.".into();
    }
    if text.trim().is_empty() {
        return "Write the note first.".into();
    }
    if text.trim().chars().count() > NOTE_MAX_CHARS {
        return format!(
            "A note is at most {NOTE_MAX_CHARS} characters; long material belongs in the repo's docs."
        );
    }
    let (tags, bad) = clean_tags(tags);
    if !bad.is_empty() {
        return format!(
            "Not a tag Irori knows: {}. Use everywhere, repo:, path:, board:, machine: or topic:.",
            bad.join(", ")
        );
    }
    if tags.is_empty() {
        return "Say where it applies: at least one tag.".into();
    }
    String::new()
}

/// One filter chip: "all" or a kind, and how many live notes it has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindCount {
    pub kind: &'static str,
    pub count: usize,
}

/// The filter chips: All, then each kind the live list has.
pub fn kind_counts(notes: &[NoteView]) -> Vec<KindCount> {
    let live: Vec<&NoteView> = notes.iter().filter(|n| n.state == "accepted").collect();
    let mut out = vec![KindCount {
        kind: "all",
        count: live.len(),
    }];
    for k in NOTE_KINDS {
        let c = live.iter().filter(|n| n.kind == k).count();
        if c > 0 {
            out.push(KindCount { kind: k, count: c });
        }
    }
    out
}

/// A note's source chip: "b1 answer", "a comment", "done proof", "the
/// card", or "" with none.
pub fn source_label(source: &str) -> String {
    if source.is_empty() {
        return String::new();
    }
    if source.starts_with("blocker:") {
        return format!("{} answer", source.rsplit('/').next().unwrap_or(""));
    }
    if source.starts_with("comment:") {
        return "a comment".into();
    }
    if source.starts_with("proof:") {
        return "done proof".into();
    }
    if source.starts_with("task:") {
        return "the card".into();
    }
    source.to_string()
}

/// The notes a page shows: live ones of the kind, matching the query,
/// newest change first.
pub fn shown<'a>(notes: &'a [NoteView], kind: &str, query: &str) -> Vec<&'a NoteView> {
    let q = query.trim().to_lowercase();
    let mut v: Vec<&NoteView> = notes
        .iter()
        .filter(|n| n.state == "accepted")
        .filter(|n| kind == "all" || n.kind == kind)
        .filter(|n| {
            q.is_empty()
                || n.text.to_lowercase().contains(&q)
                || n.tags.iter().any(|t| t.to_lowercase().contains(&q))
        })
        .collect();
    v.sort_by_key(|n| std::cmp::Reverse(n.updated.unix_milli_value()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const IRORI: &str = "remote:github.com/StephenSHorton/irori";

    fn raw(t: &str) -> Option<String> {
        parse_tag(t).map(|t| t.raw)
    }

    #[test]
    fn known_shapes_parse_and_repo_keys_keep_their_case() {
        assert_eq!(raw("Everywhere").as_deref(), Some("everywhere"));
        assert_eq!(raw(&format!("repo:{IRORI}")), Some(format!("repo:{IRORI}")));
        assert_eq!(
            raw(&format!("path:{IRORI}:frontend/src")),
            Some(format!("path:{IRORI}:frontend/src"))
        );
        assert_eq!(raw("Board:MLG").as_deref(), Some("board:mlg"));
        assert_eq!(raw("machine:The PC").as_deref(), Some("machine:the pc"));
        assert_eq!(raw("topic:Lobby").as_deref(), Some("topic:lobby"));
    }

    #[test]
    fn unknown_shapes_dont() {
        for t in [
            String::new(),
            "irori".into(),
            "repo:".into(),
            "path:nopath".into(),
            format!("path:{IRORI}:"),
            "colour:red".into(),
        ] {
            assert_eq!(parse_tag(&t), None, "{t}");
        }
    }

    #[test]
    fn labels_read_as_the_mockups() {
        let none = |_: &str| String::new();
        assert_eq!(tag_label(&format!("repo:{IRORI}"), &none), "repo irori");
        assert_eq!(
            tag_label(&format!("path:{IRORI}:frontend/"), &none),
            "irori frontend/"
        );
        assert_eq!(tag_label("machine:the pc", &none), "on the pc");
        assert_eq!(tag_label("board:mlg", &|_: &str| "MLG".to_string()), "MLG");
        assert_eq!(tag_label("topic:lobby", &none), "#lobby");
    }

    #[test]
    fn clean_tags_drops_repeats_and_reports_bad_ones() {
        let (tags, bad) = clean_tags(&["everywhere", "EVERYWHERE", "nope", " "]);
        assert_eq!(tags, ["everywhere"]);
        assert_eq!(bad, ["nope"]);
    }

    #[test]
    fn the_forms_checks_match_the_writes() {
        let ok = ["everywhere"];
        assert_eq!(note_problem("rule", "A rule.", &ok), "");
        assert!(note_problem("fact", "A rule.", &ok).contains("kind"));
        assert!(note_problem("rule", "  ", &ok).contains("Write"));
        assert!(
            note_problem("rule", &"x".repeat(NOTE_MAX_CHARS + 1), &ok)
                .contains(&NOTE_MAX_CHARS.to_string())
        );
        assert!(note_problem::<&str>("rule", "A rule.", &[]).contains("at least one tag"));
        assert!(note_problem("rule", "A rule.", &["nope"]).contains("nope"));
    }

    fn note(id: &str, kind: &str, state: &str) -> NoteView {
        NoteView {
            id: id.into(),
            kind: kind.into(),
            state: state.into(),
            text: format!("note {id}"),
            tags: vec!["everywhere".into()],
            ..Default::default()
        }
    }

    #[test]
    fn kind_counts_are_all_then_each_live_kind() {
        let list = [
            note("1", "rule", "accepted"),
            note("2", "rule", "accepted"),
            note("3", "gotcha", "accepted"),
            note("4", "decision", "removed"),
        ];
        let c = kind_counts(&list);
        let pairs: Vec<(&str, usize)> = c.iter().map(|k| (k.kind, k.count)).collect();
        assert_eq!(pairs, [("all", 3), ("rule", 2), ("gotcha", 1)]);
    }

    #[test]
    fn shown_filters_by_kind_and_query() {
        let mut a = note("a", "rule", "accepted");
        a.text = "Never drive the mouse".into();
        let mut b = note("b", "gotcha", "accepted");
        b.tags = vec!["machine:pc".into()];
        let list = [a, b, note("c", "rule", "removed")];
        assert_eq!(shown(&list, "all", "").len(), 2);
        assert_eq!(shown(&list, "rule", "")[0].id, "a");
        assert_eq!(shown(&list, "all", "MOUSE")[0].id, "a");
        assert_eq!(shown(&list, "all", "machine")[0].id, "b");
        assert!(shown(&list, "pointer", "").is_empty());
    }

    #[test]
    fn source_labels() {
        assert_eq!(source_label("blocker:t1/b1"), "b1 answer");
        assert_eq!(source_label("comment:c1"), "a comment");
        assert_eq!(source_label("proof:t1"), "done proof");
        assert_eq!(source_label("task:t1"), "the card");
        assert_eq!(source_label(""), "");
    }

    #[test]
    fn repo_names() {
        assert_eq!(repo_name(IRORI), "irori");
        assert_eq!(repo_name("remote:github.com/o/x.git"), "x");
        assert_eq!(repo_name("root:1a2b3c4d5e"), "1a2b3c4");
        assert_eq!(
            split_tags("everywhere, repo:x\ntopic:y,,"),
            ["everywhere", "repo:x", "topic:y"]
        );
    }
}
