//! What the person can do to an agent Irori started for a card (the React
//! app's cards/spawned-actions.tsx, card #456, mockup sa-2): Attach opens
//! it in the card's terminal tab, Logs shows its output so far, Stop cuts
//! its turn and End hands off and removes it. Attach and Logs work on the
//! device it runs on; elsewhere the row says where, and End asks that
//! device to end it.

use irori_window::cards::AgentRef;
use rune::{El, ScrollSpec, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use super::confirm::{Ask, confirm_button};
use crate::logic::names::display_name;
use crate::logic::plain::plain_line;
use crate::ui::{Frame, State, icons};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Attach,
    Logs,
    Stop,
    End,
}

pub const ALL: [Action; 4] = [Action::Attach, Action::Logs, Action::Stop, Action::End];

/// The Logs dialog's state.
#[derive(Clone, Debug)]
pub struct Logs {
    pub who: AgentRef,
    /// `None` while it loads.
    pub text: Option<String>,
    pub error: String,
}

/// The agent runs on this device, where Attach and Logs work.
pub fn spawned_here(who: &AgentRef) -> bool {
    who.here
}

fn line(s: &mut State, key: &str, ok: bool, t: String) {
    s.widgets.lines.insert(key.to_string(), (ok, t));
}

fn attach(s: &mut State, who: AgentRef, card_id: String) {
    let key = who.key.clone();
    let name = display_name(&who);
    s.widgets.lines.remove(&key);
    s.spawn(
        {
            let key = key.clone();
            move |w| w.agents_attach(&key, 100, 24)
        },
        move |s, r| match r {
            Ok(term) => {
                // The card's terminal shows the new tab (lib/agents.ts
                // TermOpened): opening the card loads its terminals first.
                s.open_card(&card_id);
                crate::term::opened(s, &card_id, term);
                line(
                    s,
                    &key,
                    true,
                    format!("{name} is open in this card's terminal. Typing there talks to it; it stays spawned."),
                );
            }
            Err(e) => line(s, &key, false, e),
        },
    );
}

/// Opens Logs for an agent and reads them.
pub fn open_logs(s: &mut State, who: AgentRef) {
    let key = who.key.clone();
    s.widgets.logs = Some(Logs {
        who,
        text: None,
        error: String::new(),
    });
    s.spawn(
        move |w| w.agents_logs(&key),
        |s, r| {
            if let Some(l) = &mut s.widgets.logs {
                match r {
                    Ok(t) => {
                        l.text = Some(t);
                        l.error.clear();
                    }
                    Err(e) => l.error = e,
                }
            }
        },
    );
}

/// The actions for `who` on card `card_id`, a subset with `only`.
pub fn spawned_actions(s: &State, f: &mut Frame, key: &str, who: &AgentRef, card_id: &str, only: &[Action]) -> El {
    let k = f.k;
    let here = spawned_here(who);
    let where_ = plain_line(&who.where_);
    let wanted = |a: Action| only.contains(&a);
    let show = |a: Action| wanted(a) && (here || matches!(a, Action::Stop | Action::End));
    let name = display_name(who);
    let note = |t: String| text(t).font(k.ty.text_xsmall).color(k.c.text_muted).w_full().wrap();
    if s.widgets.ended.contains(&who.key) {
        return col().key(key.to_string()).child(note(if here {
            "Ending: it writes where it left off first.".into()
        } else {
            format!("Asked {where_} to end it: it writes where it left off first.")
        }));
    }
    let elsewhere = !here && (wanted(Action::Attach) || wanted(Action::Logs));
    if !here && !show(Action::Stop) && !show(Action::End) {
        return col().key(key.to_string()).child(note(format!("Attach and Logs are on {where_}.")));
    }
    let mut buttons = row().key(format!("{key}/buttons")).gap(6.0).center_y().flex_wrap();
    if show(Action::Attach) {
        let (w, c) = (who.clone(), card_id.to_string());
        buttons = buttons.child(
            Button::new("Attach")
                .icon(&icons::SQUARE_TERMINAL)
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .help("Open it in this card's terminal tab. Typing there talks to it; it stays spawned.")
                .on_press(move |s: &mut State| attach(s, w.clone(), c.clone()))
                .build(k),
        );
    }
    if show(Action::Logs) {
        let w = who.clone();
        buttons = buttons.child(
            Button::new("Logs")
                .icon(&icons::SCROLL_TEXT)
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .help("Its output so far, read only")
                .on_press(move |s: &mut State| open_logs(s, w.clone()))
                .build(k),
        );
    }
    if show(Action::Stop) {
        let (akey, n) = (who.key.clone(), name.clone());
        let ask = Ask::new(
            format!("Stop {name}'s turn? It stays on the card and waits for you."),
            "Stop",
            move |w| w.agent_stop(&akey, ""),
        )
        .done({
            let akey = who.key.clone();
            move |s| line(s, &akey, true, format!("{n} is stopped and waits for you."))
        });
        buttons = buttons.child(confirm_button(s, f, &format!("{key}/stop"), "Stop", Some(&icons::SQUARE), false, ask));
    }
    if show(Action::End) {
        let akey = who.key.clone();
        let far = if here { String::new() } else { format!("Irori asks {where_} to end it. ") };
        let ask = Ask::new(
            format!(
                "End {name}? {far}It writes where it left off on this card first, then Irori stops it and removes its session. The card stays open and unheld."
            ),
            "End",
            move |w| w.agents_end(&akey),
        )
        .rich()
        .danger()
        .done({
            let akey = who.key.clone();
            move |s| {
                s.widgets.ended.insert(akey.clone());
            }
        });
        buttons = buttons.child(confirm_button(s, f, &format!("{key}/end"), "End", Some(&icons::X), false, ask));
    }
    let mut c = col().key(key.to_string()).gap(4.0).w_full().child(buttons);
    if elsewhere {
        c = c.child(note(format!("Attach and Logs are on {where_}.")));
    }
    if let Some((ok, t)) = s.widgets.lines.get(&who.key) {
        c = c.child(
            text(t.clone())
                .font(k.ty.text_xsmall)
                .color(if *ok { k.c.text_muted } else { k.c.danger.fg })
                .w_full()
                .wrap(),
        );
    }
    c
}

/// The Logs dialog, in the overlay layer.
pub fn logs_dialog(s: &State, f: &mut Frame) {
    let Some(l) = &s.widgets.logs else { return };
    let k = f.k;
    let body = if !l.error.is_empty() {
        text(l.error.clone()).font(k.ty.mono).color(k.c.danger.fg).w_full().wrap()
    } else {
        let t = match &l.text {
            None => "Loading…".to_string(),
            Some(t) if t.is_empty() => "Nothing yet.".to_string(),
            Some(t) => crate::logic::plain::plain_block(t),
        };
        text(t)
            .font(k.ty.mono)
            .color(k.c.text_base)
            .selectable(true)
            .w_full()
            .wrap_anywhere()
    };
    let who = l.who.clone();
    rune_kit::Dialog::new("widgets-logs", format!("Logs · {}", display_name(&l.who)))
        .open(true)
        .width(760.0)
        .description("Its terminal output so far, read only. Attach to talk to it.")
        .on_close(|s: &mut State| s.widgets.logs = None)
        .child(
            col()
                .key("widgets-logs-scroll")
                .w_full()
                .h(420.0)
                .min_h(0.0)
                .padding(12.0)
                .radius(6.0)
                .fill(k.c.bg_deep)
                .border(1.0, k.c.border_muted)
                .scroll(ScrollSpec { stick_bottom: true })
                .child(body),
        )
        .child(
            row().key("widgets-logs-foot").w_full().child(rune::el().grow(1.0)).child(
                Button::new("Refresh")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut State| open_logs(s, who.clone()))
                    .build(k),
            ),
        )
        .build(k, f.layer);
}
