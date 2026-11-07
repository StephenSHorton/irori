//! The app's one-of-a-list control (the React app's
//! common/choice-select.tsx): rune-kit's Select over choices with values,
//! optionally grouped. "" is an ordinary value ("All repos"). The widgets
//! model owns which one is open.

use std::rc::Rc;

use rune::{El, Tx};
use rune_kit::Select;

use crate::ui::{Frame, State};

/// One choice: its value and its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub label: String,
    /// The group it sits under ("" for none).
    pub group: String,
}

impl Choice {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Choice {
        Choice {
            value: value.into(),
            label: label.into(),
            group: String::new(),
        }
    }
    pub fn in_group(mut self, g: impl Into<String>) -> Choice {
        self.group = g.into();
        self
    }
}

/// A select named `label` (its accessible name), showing `value`'s
/// words; `on_change` gets the picked value.
pub fn choice_select(
    s: &State,
    f: &mut Frame,
    key: &str,
    label: &str,
    value: &str,
    choices: &[Choice],
    disabled: bool,
    width: f32,
    on_change: impl Fn(&mut State, &str) + 'static,
) -> El {
    let grouped = choices.iter().any(|c| !c.group.is_empty());
    let pairs: Vec<(&str, &str)> = choices.iter().map(|c| (c.group.as_str(), c.label.as_str())).collect();
    let labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
    let current = choices.iter().find(|c| c.value == value).map(|c| c.label.as_str());
    let by_label: Rc<Vec<(String, String)>> =
        Rc::new(choices.iter().map(|c| (c.label.clone(), c.value.clone())).collect());
    let on_change = Rc::new(on_change);
    let open = s.widgets.select_open.as_deref() == Some(key);
    let (k1, k2) = (key.to_string(), key.to_string());
    let sel = if grouped { Select::grouped(key.to_string(), &pairs) } else { Select::new(key.to_string(), &labels) };
    sel.label(label.to_string())
        .width(width)
        .current(current)
        .open(open)
        .disabled(disabled)
        .on_toggle(move |s: &mut State| {
            s.widgets.select_open = if s.widgets.select_open.as_deref() == Some(k1.as_str()) { None } else { Some(k1.clone()) };
        })
        .on_close(move |s: &mut State| {
            if s.widgets.select_open.as_deref() == Some(k2.as_str()) {
                s.widgets.select_open = None;
            }
        })
        .on_choose(move |s: &mut State, picked: &str, _: &mut Tx| {
            s.widgets.select_open = None;
            if let Some((_, v)) = by_label.iter().find(|(l, _)| l == picked) {
                on_change(s, v);
            }
        })
        .build(f.k, f.layer)
}
