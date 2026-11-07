//! WholeNumberField (agent-starts.tsx, card #703): a box for a whole
//! number from 0 to a ceiling, saved on Enter or when it loses focus;
//! anything else is refused with a message and, on leaving, goes back to
//! the saved value. It shows the saved value except while it is being
//! edited or saved: once the save settles it shows the saved value again,
//! the new one or, if the save was refused, the old one. The field keeps
//! its focus through a save (its text changes in place). The "At most"
//! of "My agents may start agents" and each device's cap in Limits use it.

use rune::{El, TextEdit, Tx, col, row, text};
use rune_kit::TextInput;

use crate::ui::{Frame, State};

use super::error_line;

#[derive(Default)]
pub struct WholeNumber {
    pub edit: TextEdit,
    /// The person typed since the box last showed the saved value (React's
    /// `draft !== null`).
    pub draft: bool,
    /// The text being saved now, so Enter then the blur sends it once.
    pub saving: Option<String>,
    /// The box had the focus when last looked at (`poll`): losing it saves.
    pub had_focus: bool,
}

/// A typed number: a whole number from 0 to the ceiling.
pub fn parse(text: &str, ceiling: u32) -> Option<u32> {
    let t = text.trim();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse::<u32>().ok().filter(|n| *n <= ceiling)
}

impl WholeNumber {
    /// Shows the saved value unless the box is being edited.
    pub fn follow(&mut self, value: u32) {
        let v = value.to_string();
        if !self.draft && self.saving.is_none() && self.edit.text != v {
            self.edit.replace_text(v);
        }
    }

    /// Typing makes a draft.
    pub fn typed(&mut self) {
        self.draft = true;
    }

    pub fn valid(&self, ceiling: u32) -> bool {
        parse(&self.edit.text, ceiling).is_some()
    }

    /// Enter or a blur: the number to save, if any. A bad or unchanged
    /// entry goes back to the saved value instead.
    pub fn save(&mut self, value: u32, ceiling: u32) -> Option<u32> {
        if !self.draft || self.saving.as_deref() == Some(self.edit.text.as_str()) {
            return None;
        }
        match parse(&self.edit.text, ceiling) {
            Some(n) if n != value => {
                self.saving = Some(self.edit.text.clone());
                Some(n)
            }
            _ => {
                self.draft = false;
                self.edit.replace_text(value.to_string());
                None
            }
        }
    }

    /// The save of `sent` settled; `value` is the saved one now (the new
    /// number, or the old after a refusal). Typing again while it saved
    /// keeps the new draft.
    pub fn settle(&mut self, sent: &str, value: u32) {
        if self.saving.as_deref() == Some(sent) {
            self.saving = None;
        }
        if self.edit.text == sent {
            self.draft = false;
            self.edit.replace_text(value.to_string());
        }
    }

    /// Whether the box just lost its focus (looked at once a frame).
    pub fn blurred(&mut self) -> bool {
        let was = std::mem::replace(&mut self.had_focus, self.edit.focused);
        was && !self.edit.focused
    }
}

/// The box with its label, and the line under it while the entry isn't a
/// whole number in range.
#[allow(clippy::too_many_arguments)]
pub fn field(
    f: &Frame,
    key: &str,
    label: &str,
    name: &str,
    n: &WholeNumber,
    ceiling: u32,
    disabled: bool,
    at: impl Fn(&mut State) -> &mut WholeNumber + Clone + 'static,
    save: impl Fn(&mut State) + 'static,
) -> El {
    let k = f.k;
    let valid = n.valid(ceiling);
    let (a, b) = (at.clone(), at);
    let mut out = col().items_end().gap(4.0).child(
        row()
            .gap(8.0)
            .center_y()
            .child(text(label.to_string()).font(k.ty.ui).color(k.c.text_muted))
            .child(
                TextInput::new(&n.edit)
                    .label(name.to_string())
                    .width(Some(72.0))
                    .disabled(disabled)
                    .invalid(!valid)
                    .on_edit(move |s: &mut State| &mut a(s).edit)
                    .on_change(move |s: &mut State, _: &mut Tx| b(s).typed())
                    .on_submit(move |s: &mut State, _: &mut Tx| save(s))
                    .build(k)
                    .key(key.to_string()),
            ),
    );
    if !valid {
        out = out.child(error_line(f, &format!("A whole number from 0 to {ceiling}.")).w(200.0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(n: &mut WholeNumber, t: &str) {
        n.edit.replace_text(t);
        n.typed();
    }

    #[test]
    fn only_whole_numbers_in_range() {
        assert_eq!(parse("5", 10), Some(5));
        assert_eq!(parse(" 0 ", 10), Some(0));
        assert_eq!(parse("10", 10), Some(10));
        assert_eq!(parse("11", 10), None);
        assert_eq!(parse("", 10), None);
        assert_eq!(parse("-1", 10), None);
        assert_eq!(parse("2.5", 10), None);
        assert_eq!(parse("+3", 10), None);
        assert_eq!(parse("abc", 10), None);
    }

    #[test]
    fn it_follows_the_saved_value_until_typed_in() {
        let mut n = WholeNumber::default();
        n.follow(4);
        assert_eq!(n.edit.text, "4");
        typed(&mut n, "7");
        n.follow(5);
        assert_eq!(n.edit.text, "7", "a draft isn't overwritten by a refresh");
    }

    #[test]
    fn a_save_goes_once_then_shows_the_saved_value() {
        let mut n = WholeNumber::default();
        n.follow(4);
        typed(&mut n, "7");
        assert_eq!(n.save(4, 10), Some(7));
        assert_eq!(n.save(4, 10), None, "Enter then blur sends it once");
        n.settle("7", 7);
        assert_eq!((n.edit.text.as_str(), n.draft, n.saving.as_deref()), ("7", false, None));
        n.follow(7);
        assert_eq!(n.edit.text, "7");
    }

    #[test]
    fn a_refused_save_shows_the_old_value() {
        let mut n = WholeNumber::default();
        n.follow(4);
        typed(&mut n, "9");
        assert_eq!(n.save(4, 10), Some(9));
        n.settle("9", 4);
        assert_eq!(n.edit.text, "4");
        assert!(!n.draft);
    }

    #[test]
    fn typing_while_it_saves_keeps_the_new_draft() {
        let mut n = WholeNumber::default();
        n.follow(4);
        typed(&mut n, "7");
        assert_eq!(n.save(4, 10), Some(7));
        typed(&mut n, "8");
        n.settle("7", 7);
        assert_eq!(n.edit.text, "8");
        assert!(n.draft);
        assert_eq!(n.save(7, 10), Some(8));
    }

    #[test]
    fn a_bad_or_unchanged_entry_goes_back() {
        let mut n = WholeNumber::default();
        n.follow(4);
        typed(&mut n, "abc");
        assert!(!n.valid(10));
        assert_eq!(n.save(4, 10), None);
        assert_eq!(n.edit.text, "4");
        assert!(n.valid(10));
        typed(&mut n, "4");
        assert_eq!(n.save(4, 10), None);
        assert!(!n.draft);
        // Nothing typed: nothing to save.
        assert_eq!(n.save(4, 10), None);
    }

    #[test]
    fn losing_focus_is_seen_once() {
        let mut n = WholeNumber::default();
        assert!(!n.blurred());
        n.edit.focused = true;
        assert!(!n.blurred());
        n.edit.focused = false;
        assert!(n.blurred());
        assert!(!n.blurred());
    }
}
