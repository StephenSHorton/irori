//! A button that asks before it acts (the React app's
//! common/confirm-button.tsx): the press opens a dialog with the question
//! and Confirm / Cancel; Confirm is the default button, so Return
//! confirms and Escape cancels. The work runs off the UI thread once;
//! its error shows under the button until the next try.

use std::sync::Arc;

use irori_window::Window;
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use crate::ui::{Frame, State};

/// A confirm step's work, off the UI thread.
pub type Work = Arc<dyn Fn(&Window) -> Result<(), String> + Send + Sync>;

/// What a confirm step asks and does.
#[derive(Clone)]
pub struct Ask {
    /// The question: plain, it is the dialog's title; with `rich`, the
    /// title is `confirm_label` and the question its description
    /// (confirm-button.tsx).
    pub question: String,
    pub rich: bool,
    pub confirm_label: String,
    pub cancel_label: String,
    /// The red confirm (End, Delete).
    pub danger: bool,
    /// The work, off the UI thread.
    pub work: Work,
    /// What follows a success, on the UI thread.
    pub done: Arc<dyn Fn(&mut State) + Send + Sync>,
}

impl Ask {
    pub fn new(
        question: impl Into<String>,
        confirm_label: impl Into<String>,
        work: impl Fn(&Window) -> Result<(), String> + Send + Sync + 'static,
    ) -> Ask {
        Ask {
            question: question.into(),
            rich: false,
            confirm_label: confirm_label.into(),
            cancel_label: "Cancel".into(),
            danger: false,
            work: Arc::new(work),
            done: Arc::new(|_| {}),
        }
    }
    pub fn rich(mut self) -> Ask {
        self.rich = true;
        self
    }
    pub fn danger(mut self) -> Ask {
        self.danger = true;
        self
    }
    pub fn done(mut self, f: impl Fn(&mut State) + Send + Sync + 'static) -> Ask {
        self.done = Arc::new(f);
        self
    }
}

/// An open confirm dialog: the key of what opened it (its error shows
/// there) and what it asks.
#[derive(Clone)]
pub struct Open {
    pub key: String,
    pub ask: Ask,
}

/// Opens a confirm dialog from anywhere (a menu item, say): the
/// ConfirmDialog half of the React component.
pub fn open(s: &mut State, key: &str, ask: Ask) {
    s.widgets.confirm_errors.remove(key);
    s.widgets.confirm = Some(Open {
        key: key.to_string(),
        ask,
    });
}

fn run(s: &mut State) {
    let Some(o) = s.widgets.confirm.clone() else {
        return;
    };
    if !s.widgets.busy.insert(o.key.clone()) {
        return;
    }
    let work = o.ask.work.clone();
    let done = o.ask.done.clone();
    let key = o.key.clone();
    s.spawn(
        move |w| work(w),
        move |s, r| {
            s.widgets.busy.remove(&key);
            if s.widgets.confirm.as_ref().is_some_and(|c| c.key == key) {
                s.widgets.confirm = None;
            }
            match r {
                Ok(()) => done(s),
                Err(e) => {
                    s.widgets.confirm_errors.insert(key, e);
                }
            }
        },
    );
}

/// The button: `label` (an icon optional), and its error under it.
pub fn confirm_button(
    s: &State,
    f: &Frame,
    key: &str,
    label: &str,
    ic: Option<&'static rune::Icon>,
    disabled: bool,
    ask: Ask,
) -> El {
    let k = f.k;
    let kk = key.to_string();
    let mut b = Button::new(label.to_string())
        .variant(ButtonVariant::Outline)
        .control_size(ControlSize::Small)
        .disabled(disabled)
        .on_press(move |s: &mut State| open(s, &kk, ask.clone()));
    if let Some(i) = ic {
        b = b.icon(i);
    }
    let mut c = col().key(key.to_string()).gap(4.0).child(b.build(k));
    if let Some(e) = s.widgets.confirm_errors.get(key) {
        c = c.child(
            text(e.clone())
                .font(k.ty.text_xsmall)
                .color(k.c.danger.fg)
                .max_w(320.0)
                .wrap(),
        );
    }
    c
}

/// The open dialog, in the overlay layer.
pub fn dialog(s: &State, f: &mut Frame) {
    let Some(o) = &s.widgets.confirm else { return };
    let k = f.k;
    let busy = s.widgets.busy.contains(&o.key);
    let a = &o.ask;
    // The kit Dialog wraps a long title or description in its header.
    let title = if a.rich {
        a.confirm_label.clone()
    } else {
        a.question.clone()
    };
    let confirm = Button::new(if busy {
        "Working…".to_string()
    } else {
        a.confirm_label.clone()
    })
    .variant(if a.danger {
        ButtonVariant::Danger
    } else {
        ButtonVariant::Contrast
    })
    .control_size(ControlSize::Small)
    .disabled(busy)
    .on_press(run)
    .build(k)
    .default_button();
    let cancel = Button::new(a.cancel_label.clone())
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .disabled(busy)
        .on_press(|s: &mut State| s.widgets.confirm = None)
        .build(k);
    let mut d = rune_kit::Dialog::new("widgets-confirm", title)
        .open(true)
        .width(420.0)
        .fit()
        .on_close(move |s: &mut State| {
            if !s.widgets.busy.contains(
                &s.widgets
                    .confirm
                    .as_ref()
                    .map(|c| c.key.clone())
                    .unwrap_or_default(),
            ) {
                s.widgets.confirm = None;
            }
        });
    if a.rich {
        d = d.description(a.question.clone());
    }
    d.child(
        row()
            .key("widgets-confirm-foot")
            .w_full()
            .px(16.0)
            .pb(16.0)
            .gap(8.0)
            .child(rune::el().grow(1.0))
            .child(cancel)
            .child(confirm),
    )
    .build(k, f.layer);
}
