//! Voice input's pure parts (main:frontend/src/lib/voice.ts and
//! mic-shortcut.ts): where dictated words go in a box, one session's state
//! from its `voice:event`s, the error lines, and the dictation shortcut.
//! Offsets are byte offsets into the box's text, as `rune::TextEdit`'s.

/// The text either side of where dictation started.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Anchor {
    pub before: String,
    pub after: String,
}

fn floor_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// anchorAt: splits the box's text at the selection; a selection is
/// replaced.
pub fn anchor_at(value: &str, start: usize, end: usize) -> Anchor {
    let a = floor_boundary(value, start);
    let b = floor_boundary(value, end.max(a));
    Anchor {
        before: value[..a].to_string(),
        after: value[b..].to_string(),
    }
}

/// joinWords: b after a with one space between, unless either is empty or
/// a already ends in whitespace.
pub fn join_words(a: &str, b: &str) -> String {
    if a.is_empty() {
        return b.to_string();
    }
    if b.is_empty() {
        return a.to_string();
    }
    if a.ends_with(char::is_whitespace) {
        format!("{a}{b}")
    } else {
        format!("{a} {b}")
    }
}

/// compose: the box's text and caret for the words so far. The caret sits
/// right after the words.
pub fn compose(anchor: &Anchor, spoken: &str) -> (String, usize) {
    if spoken.is_empty() {
        return (
            format!("{}{}", anchor.before, anchor.after),
            anchor.before.len(),
        );
    }
    let head = join_words(&anchor.before, spoken);
    // A word running into the text after gets a space; punctuation does not.
    let tail = match anchor.after.chars().next() {
        Some(c) if !(c.is_whitespace() || ".,;:!?)]}'\"".contains(c)) => format!(" {}", anchor.after),
        _ => anchor.after.clone(),
    };
    let caret = head.len();
    (head + &tail, caret)
}

/// One dictation session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub id: u32,
    /// The settled phrases, joined.
    pub committed: String,
    /// The phrase being spoken; replaced by each partial.
    pub partial: String,
    pub ended: bool,
    /// (code, text) of an error.
    pub error: Option<(String, String)>,
}

impl Session {
    pub fn new(id: u32) -> Session {
        Session { id, ..Default::default() }
    }

    /// spoken: everything said so far, as the box shows it.
    pub fn spoken(&self) -> String {
        join_words(&self.committed, &self.partial)
    }

    /// step: one voice:event; another session's leave it as it is.
    pub fn step(&mut self, session: u32, kind: &str, text: &str, code: &str) {
        if session != self.id || self.ended {
            return;
        }
        match kind {
            "partial" => self.partial = text.trim().to_string(),
            "final" => {
                self.committed = join_words(&self.committed, text.trim());
                self.partial.clear();
            }
            "error" => self.error = Some((code.to_string(), text.to_string())),
            "ended" => {
                self.partial.clear();
                self.ended = true;
            }
            _ => {}
        }
    }
}

/// settingsFix: an error code with an OS settings page to open.
pub fn settings_fix(code: &str) -> bool {
    code == "mic-denied" || code == "no-mic"
}

/// errorLine: the line the box shows for a failed session or start.
pub fn error_line(code: &str, text: &str) -> String {
    if !text.is_empty() {
        return text.to_string();
    }
    match code {
        "mic-denied" => "Microphone access is off for Irori.",
        "no-mic" => "No microphone was found.",
        "no-model" => "The speech model isn't downloaded yet.",
        _ => "Voice input stopped.",
    }
    .to_string()
}

/// megabytes: "148 MB".
pub fn megabytes(n: u64) -> String {
    format!("{} MB", (n as f64 / 1_000_000.0).round() as u64)
}

/// codeOf: a failed start's error code, guessed from its text.
pub fn code_of(text: &str) -> &'static str {
    let t = text.to_lowercase();
    if t.contains("microphone access") || t.contains("allow") {
        "mic-denied"
    } else if t.contains("microphone") {
        "no-mic"
    } else if t.contains("model") {
        "no-model"
    } else {
        "other"
    }
}

/// dictationKey: Cmd+D on a Mac, Ctrl+D elsewhere, with no Shift or Alt.
/// On a Mac, Ctrl+D stays the text field's delete-forward.
pub fn dictation_key(key: &str, meta: bool, ctrl: bool, shift: bool, alt: bool, mac: bool) -> bool {
    let m = if mac { meta && !ctrl } else { ctrl && !meta };
    m && !shift && !alt && key.eq_ignore_ascii_case("d")
}

/// A mic as the shortcut sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct MicSeen {
    pub key: String,
    /// Listening now (or starting).
    pub live: bool,
    /// Its box has focus.
    pub focused: bool,
    /// Can start: not disabled, voice is there, no huddle.
    pub usable: bool,
}

/// pickMic: the one mic a press toggles: one that is listening; else the
/// focused box's; else the newest drawn (a dialog's over a page's). With
/// no mic usable, none.
pub fn pick_mic(mics: &[MicSeen]) -> Option<&MicSeen> {
    if let Some(m) = mics.iter().find(|m| m.live) {
        return Some(m);
    }
    if let Some(m) = mics.iter().find(|m| m.focused) {
        return m.usable.then_some(m);
    }
    mics.iter().rev().find(|m| m.usable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictation_goes_in_at_the_caret() {
        let a = anchor_at("Ship it after the review", 8, 8);
        assert_eq!(a, Anchor { before: "Ship it ".into(), after: "after the review".into() });
        assert_eq!(compose(&a, "today"), ("Ship it today after the review".into(), "Ship it today".len()));
    }

    #[test]
    fn a_selection_is_replaced() {
        let a = anchor_at("call it foo now", 8, 11);
        assert_eq!(compose(&a, "the parser").0, "call it the parser now");
    }

    #[test]
    fn spaces_only_where_words_would_touch() {
        assert_eq!(compose(&anchor_at("Hello", 5, 5), "world").0, "Hello world");
        assert_eq!(compose(&anchor_at("", 0, 0), "First words").0, "First words");
        assert_eq!(compose(&anchor_at("end.", 0, 0), "Start").0, "Start end.");
        assert_eq!(compose(&anchor_at("so,", 2, 2), "and").0, "so and,");
        assert_eq!(compose(&anchor_at("a\n", 2, 2), "b").0, "a\nb");
        assert_eq!(compose(&anchor_at("abc", 1, 2), ""), ("ac".into(), 1));
    }

    #[test]
    fn join_words_cases() {
        assert_eq!(join_words("", "x"), "x");
        assert_eq!(join_words("x", ""), "x");
        assert_eq!(join_words("x ", "y"), "x y");
        assert_eq!(join_words("x", "y"), "x y");
    }

    #[test]
    fn partial_replaced_final_settles() {
        let mut s = Session::new(7);
        s.step(7, "partial", "Please", "");
        s.step(7, "partial", "Please review the", "");
        assert_eq!(s.spoken(), "Please review the");
        s.step(7, "final", "Please review the card.", "");
        assert_eq!(s.spoken(), "Please review the card.");
        s.step(7, "partial", "Then send", "");
        assert_eq!(s.spoken(), "Please review the card. Then send");
        s.step(7, "final", "Then send it.", "");
        s.step(7, "ended", "", "");
        assert!(s.ended);
        assert_eq!(s.spoken(), "Please review the card. Then send it.");
    }

    #[test]
    fn empty_final_and_ended_drop_partials() {
        let mut s = Session::new(1);
        s.step(1, "partial", "uh", "");
        s.step(1, "final", "", "");
        assert_eq!(s.spoken(), "");
        s.step(1, "partial", "half a", "");
        s.step(1, "ended", "", "");
        assert_eq!(s.spoken(), "");
    }

    #[test]
    fn other_sessions_and_after_ended_ignored() {
        let mut s = Session::new(2);
        s.step(3, "partial", "not mine", "");
        assert_eq!(s.spoken(), "");
        s.step(2, "ended", "", "");
        s.step(2, "final", "late", "");
        assert_eq!(s.spoken(), "");
    }

    #[test]
    fn errors_keep_code_and_text() {
        let mut s = Session::new(4);
        s.step(4, "error", "Allow the microphone", "mic-denied");
        assert_eq!(s.error, Some(("mic-denied".into(), "Allow the microphone".into())));
        assert!(settings_fix("mic-denied"));
        assert!(!settings_fix("no-model"));
        assert_eq!(error_line("no-mic", ""), "No microphone was found.");
        assert_eq!(error_line("other", "Custom"), "Custom");
        assert_eq!(code_of("Allow Irori to use the microphone"), "mic-denied");
        assert_eq!(code_of("no model"), "no-model");
    }

    #[test]
    fn dictation_key_per_platform() {
        assert!(dictation_key("d", true, false, false, false, true));
        assert!(!dictation_key("d", false, true, false, false, true));
        assert!(dictation_key("D", false, true, false, false, false));
        assert!(!dictation_key("d", true, false, true, false, true));
        assert!(!dictation_key("e", true, false, false, false, true));
    }

    #[test]
    fn pick_mic_order() {
        let m = |key: &str, live: bool, focused: bool, usable: bool| MicSeen { key: key.into(), live, focused, usable };
        let list = vec![m("a", false, false, true), m("b", false, false, true)];
        assert_eq!(pick_mic(&list).map(|x| x.key.as_str()), Some("b"));
        let list = vec![m("a", false, true, true), m("b", false, false, true)];
        assert_eq!(pick_mic(&list).map(|x| x.key.as_str()), Some("a"));
        let list = vec![m("a", false, true, false), m("b", false, false, true)];
        assert_eq!(pick_mic(&list), None);
        let list = vec![m("a", false, true, true), m("b", true, false, true)];
        assert_eq!(pick_mic(&list).map(|x| x.key.as_str()), Some("b"));
        assert_eq!(pick_mic(&[]), None);
        assert_eq!(megabytes(147_964_211), "148 MB");
    }
}
