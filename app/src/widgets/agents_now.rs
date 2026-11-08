//! "Agents answer now" (the React app's common/agents-now-toggle.tsx,
//! cards #391, #512) on a composer or a thread: on, the agents it names are
//! woken and an idle agent with Offer it cards on (or a new one) picks it
//! up; off, nobody is woken and agents read it at their next turn.

use rune::{El, row, text};
use rune_kit::{ControlSize, Switch};

use crate::ui::theme::status;
use crate::ui::{Frame, State};

pub fn agents_now_toggle(
    f: &Frame,
    key: &str,
    on: bool,
    disabled: bool,
    on_change: impl Fn(&mut State, bool) + 'static,
) -> El {
    let k = f.k;
    let tip = if on {
        "Agents answer now: the agents it names are woken, and at once an idle agent with Offer it cards on picks it up, or Irori starts one when spawning is on"
    } else {
        "Agents answer later: nobody is woken or handed it; agents in it read it at their next turn and may reply"
    };
    row()
        .key(key.to_string())
        .gap(6.0)
        .center_y()
        .help(tip)
        .opacity(if disabled { 0.6 } else { 1.0 })
        .child(
            Switch::new(on)
                .label("Agents answer now")
                .hide_label()
                .control_size(ControlSize::Mini)
                .disabled(disabled)
                .on_toggle(move |s: &mut State| on_change(s, !on))
                .build(k),
        )
        .child(
            text("Agents answer now")
                .font(rune::tok::TextStyle::new(11.5, 400.0))
                .color(if on { status::review() } else { k.c.text_muted }),
        )
}
