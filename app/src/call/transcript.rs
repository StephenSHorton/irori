//! The transcript a huddle leaves (transcript-pane.tsx), beside the
//! conversation: who can read it, a search, and each line by who said it
//! and when.

use irori_window::talk::TalkTranscript;
use rune::{El, ScrollSpec, col, el, icon, row, text};
use rune_kit::{Kit, TextInput};

use super::huddle::{avatar, round_button};
use super::icons as ic;
use super::logic;
use crate::ui::{Frame, State};

/// The open transcript (after [`super::open_transcript`]), 340 points
/// wide; None while none is open.
pub fn transcript_pane(s: &State, f: &mut Frame) -> Option<El> {
    let t = s.call.transcript.as_ref()?;
    let k = f.k;
    Some(match &t.got {
        Some(Ok(tr)) => view(s, k, tr, &t.q),
        Some(Err(e)) => frame(k, None).child(
            text(format!("Could not read the transcript: {e}"))
                .role("alert")
                .font(k.ty.small)
                .color(k.c.danger.icon)
                .w_full()
                .padding(12.0)
                .wrap(),
        ),
        None => frame(k, None).child(
            text("Reading the transcript…")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .padding(12.0),
        ),
    })
}

fn frame(k: &Kit, sub: Option<String>) -> El {
    let mut head = row()
        .key("call-tr-head")
        .w_full()
        .gap(8.0)
        .px(12.0)
        .pt(12.0)
        .center_y()
        .child(icon(&ic::FILE_TEXT, 16.0).color(k.c.text_base))
        .child(text("Huddle transcript").font(rune::tok::TextStyle::new(13.0, 600.0)).color(k.c.text_base));
    if let Some(sub) = sub {
        head = head.child(text(sub).font(k.ty.small).color(k.c.text_muted).truncate().min_w(0.0).shrink());
    }
    head = head.child(el().grow(1.0)).child(round_button(
        k,
        "call-tr-close".into(),
        &ic::X,
        false,
        None,
        k.c.icon_muted,
        "Close the transcript",
        super::close_transcript,
    ));
    col()
        .key("call-transcript")
        .label("Huddle transcript")
        .w(340.0)
        .h_full()
        .min_h(0.0)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_muted)
        .child(head)
}

fn view(_s: &State, k: &Kit, t: &TalkTranscript, q: &rune::TextEdit) -> El {
    let shown = logic::find_lines(&t.lines, &q.text);
    let top = col()
        .key("call-tr-top")
        .w_full()
        .gap(8.0)
        .px(12.0)
        .pt(4.0)
        .pb(12.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_muted)
        .child(
            text(logic::who_can_read(&t.conv_kind, &t.conv_name))
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        )
        .child(
            TextInput::new(q)
                .placeholder("Search the transcript")
                .label("Search the transcript")
                .leading_icon(&ic::SEARCH)
                .on_edit(|s: &mut State| &mut s.call.transcript.as_mut().expect("open transcript").q)
                .build(k)
                .key("call-tr-search"),
        );
    let mut lines = col().key("call-tr-lines").w_full().gap(10.0).px(12.0).py(12.0);
    if t.lines.is_empty() {
        lines = lines.child(text("Nothing was written down.").font(k.ty.small).color(k.c.text_muted));
    } else if shown.is_empty() {
        lines = lines.child(text("No line has that.").font(k.ty.small).color(k.c.text_muted));
    }
    for (i, l) in shown.iter().enumerate() {
        let name = logic::person_word(&l.name);
        lines = lines.child(
            row()
                .key(format!("call-tr-{i}-{}", l.at_ms))
                .w_full()
                .gap(8.0)
                .items_start()
                .child(avatar(k, "a".into(), &name, None, true))
                .child(
                    col()
                        .key("t")
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            row()
                                .key("who")
                                .gap(6.0)
                                .center_y()
                                .child(text(name).font(k.ty.small_medium).color(k.c.text_base))
                                .child(text(logic::clock_text(l.at_ms)).font(super::tnum(k.ty.text_xsmall)).color(k.c.text_muted)),
                        )
                        .child(text(l.text.clone()).font(k.ty.small).color(k.c.text_weak).w_full().wrap().selectable(true)),
                ),
        );
    }
    frame(k, Some(logic::when_text(t, super::now_ms())))
        .child(top)
        .child(
            col()
                .key("call-tr-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(lines),
        )
}
