//! The ask box over Home (the React app's cards/ask-box.tsx, cards #116,
//! #124, #134, #391, #512): a question or a conversation with the room,
//! not a card. The agents on the cards it names and the agents it
//! @mentions hear it; at once an idle agent with Offer it cards on gets
//! it, or with none free Irori starts its own. Enter sends, Shift+Enter is
//! a new line; files go with it (the clip, a drop or a pasted picture).
//! "Agents answer now" starts off when the text names only people.
//!
//! On a shared space (its page, or Home with that space picked) the box
//! starts the thread on that space: only its people and agents see it, it
//! takes no files yet, and on the space's page it opens in the Threads
//! panel. Each place keeps its own draft ("ask/room", "ask/board/<id>").

use std::rc::Rc;

use rune::{El, col, text};

use super::pure::agents_now_default;
use crate::text::{Composer, ComposerOpts, Sent};
use crate::ui::{Frame, Route, State};
use crate::widgets::agents_now::agents_now_toggle;

/// The drafts key Home's box keeps its words under.
pub const DRAFT: &str = "ask/room";

/// The drafts key of the box on `board` ("" your own).
pub fn draft_key(board: &str) -> String {
    if board.is_empty() {
        DRAFT.to_string()
    } else {
        format!("ask/board/{board}")
    }
}

/// Points the box at `board`'s draft when it was on another one's.
pub fn follow(s: &mut State, board: &str) {
    let key = draft_key(board);
    if s.home.ask.key != key && !s.home.ask.sending {
        crate::text::open_draft(s, ask_mut, &key);
        s.home.ask_line = None;
        s.home.agents_now = None;
    }
}

/// The board the box is on now: the open space page's, else Home's pick.
pub fn here(s: &State) -> String {
    match &s.nav.route {
        Route::Board { space, .. } if space != crate::spaces::PERSONAL => space.clone(),
        Route::Board { .. } => String::new(),
        _ => crate::logic::spaces::threads_space(&crate::spaces::current(s)).to_string(),
    }
}

pub fn ask(s: &State) -> &Composer {
    &s.home.ask
}
pub fn ask_mut(s: &mut State) -> &mut Composer {
    &mut s.home.ask
}

/// The toggle: the default for its mentions until the person flips it.
pub fn agents_now(s: &State) -> bool {
    s.home
        .agents_now
        .unwrap_or_else(|| agents_now_default(s.home.ask.mentions.iter().map(|m| m.key.as_str())))
}

/// What a send said: whom it reached, or that nobody was woken.
pub fn sent_line(now: bool, handed: bool, n: usize) -> String {
    let more = if handed {
        ""
    } else {
        " An idle agent with Offer it cards on picks it up, or Irori starts one when spawning is on."
    };
    if !now {
        "Sent. Agents answer now was off: no agent was woken or handed it.".into()
    } else if handed && n <= 1 {
        "Sent. An agent with Offer it cards on has it.".into()
    } else if n == 0 {
        format!("Sent.{more}")
    } else {
        format!(
            "Sent to {}.{more}",
            if n == 1 {
                "1 agent".to_string()
            } else {
                format!("{n} agents")
            }
        )
    }
}

/// The page's board while it is still a ready board of ours ("" none).
fn on_board(s: &State, board: &str) -> bool {
    s.data
        .spaces
        .iter()
        .any(|b| b.board_id == board && b.ready && !b.leaving)
}

fn send(s: &mut State, m: Sent) {
    if s.home.ask.sending || m.text.trim().is_empty() {
        return;
    }
    let board = here(s);
    if !board.is_empty() && !on_board(s, &board) {
        s.home.ask_line = Some(("This space isn't ready here yet.".into(), true));
        return;
    }
    if !board.is_empty() && !m.files.is_empty() {
        s.home.ask_line = Some((
            "Files don't go on a space thread yet: remove them, or attach them on a card.".into(),
            true,
        ));
        return;
    }
    // The Threads panel opens the new thread on the space's own page.
    let on_page = !board.is_empty() && matches!(s.nav.route, Route::Board { .. });
    s.home.ask.sending = true;
    s.home.ask_line = None;
    let now = agents_now(s);
    let ms: Vec<irori_window::ts::Mention> = m.mentions.iter().map(|x| x.to_ts()).collect();
    s.spawn(
        move |w| {
            w.start_thread_with(&m.text, &ms, &m.files, &board, Some(now))
                .map(|r| (r, board))
        },
        move |s, r| {
            s.home.ask.sending = false;
            match r {
                Ok((res, board)) => {
                    let handed = res.task.ask.as_ref().is_some_and(|a| !a.taker.is_empty());
                    // On a board the other people's agents are told by
                    // their own devices: count every agent the round names.
                    let n = if board.is_empty() {
                        res.told.to.len()
                    } else {
                        res.task.ask.as_ref().map_or(0, |a| a.told.max(0) as usize)
                    };
                    s.home.ask_line = Some((sent_line(now, handed, n), false));
                    crate::text::sent(s, ask_mut);
                    s.home.ask.missed(&res.mentioned);
                    s.home.agents_now = None;
                    if on_page {
                        s.board.threads.open = res.task.id.clone();
                        s.board.threads.hidden = false;
                        crate::board::threads::load(s, &board);
                    } else {
                        // The new thread opens in Talk.
                        s.go(Route::Talk(Some(res.task.id.clone())));
                    }
                }
                Err(e) => s.home.ask_line = Some((e, true)),
            }
        },
    );
}

pub fn ask_box(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let busy = s.home.ask.sending;
    let board = here(s);
    let opts = ComposerOpts {
        placeholder: "Ask about anything…".into(),
        label: "Start a thread".into(),
        send_button: true,
        extra: vec![agents_now_toggle(
            f,
            "home/ask/now",
            agents_now(s),
            busy,
            |s, on| s.home.agents_now = Some(on),
        )],
        on_send: Some(Rc::new(send)),
        rows: Some((1, 8)),
        mic: Some(crate::call::mic_button(s, f, "home/ask/box", |s| {
            &mut s.home.ask.edit
        })),
        no_files: (!board.is_empty())
            .then(|| "Files don't go on a space thread yet: attach them on a card".to_string()),
        board,
        ..ComposerOpts::default()
    };
    let mut c = col()
        .key("home/ask")
        .role("form")
        .w_full()
        .gap(4.0)
        .child(crate::text::composer(
            s,
            f,
            "home/ask/box",
            ask,
            ask_mut,
            &opts,
        ));
    if let Some((t, err)) = &s.home.ask_line {
        c = c.child(
            text(t.clone())
                .key("home/ask/line")
                .role(if *err { "alert" } else { "status" })
                .font(k.ty.text_xsmall)
                .color(if *err { k.c.danger.fg } else { k.c.text_muted })
                .px(4.0)
                .w_full()
                .wrap(),
        );
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_send_says() {
        assert_eq!(
            sent_line(false, false, 3),
            "Sent. Agents answer now was off: no agent was woken or handed it."
        );
        assert_eq!(
            sent_line(true, true, 1),
            "Sent. An agent with Offer it cards on has it."
        );
        assert_eq!(
            sent_line(true, false, 0),
            "Sent. An idle agent with Offer it cards on picks it up, or Irori starts one when spawning is on."
        );
        assert_eq!(sent_line(true, true, 3), "Sent to 3 agents.");
        assert_eq!(
            sent_line(true, false, 1),
            "Sent to 1 agent. An idle agent with Offer it cards on picks it up, or Irori starts one when spawning is on."
        );
    }
}
