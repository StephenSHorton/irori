//! A channel or a DM (conversation.tsx `ConversationView`, `PostRow`,
//! `DayLine`, `Members`, `CardChip`, `ThreadFrom`, `Composer`, and
//! replying-row.tsx): the posts oldest first with a line between days, the
//! members with how each stands, and the box to post. An agent↔agent DM is
//! read-only. Opening one marks it read.

use std::collections::HashMap;
use std::rc::Rc;

use irori_window::cards::CardMention;
use irori_window::talk::{TalkConversation, TalkMember, TalkPost};
use rune::{El, GrowthSpec, Key, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, MenuItem, TextInput};

use super::list::ts;
use super::logic::{self, ReplyLine};
use super::{icons, post_box, post_box_mut};
use crate::logic::agent_status::now_ms;
use crate::logic::names::{UNKNOWN_AGENT, name_or};
use crate::logic::plain::plain_line;
use crate::logic::thread::{LiveAgent, live_roster};
use crate::text::{ComposerOpts, File, RichOpts, Sent};
use crate::ui::theme::status;
use crate::ui::{Frame, State};
use crate::widgets::faces::{FaceOpts, face};

/// A window this wide (Tailwind's xl) opens a channel's members column by
/// itself (card #639).
const MEMBERS_ROOMY: f32 = 1280.0;
/// Narrower than this (Tailwind's lg) the members column never shows.
const MEMBERS_WIDE: f32 = 1024.0;

/// The pane for conversation `c`.
pub fn view(s: &State, f: &mut Frame, c: &TalkConversation) -> El {
    let k = f.k;
    let live = live_roster(&s.data.agents);
    let now = now_ms();
    let other = if c.kind == "dm" {
        logic::dm_with(c)
    } else {
        vec![]
    };
    let person = logic::dm_person(c);
    let person_name = person
        .map(|p| {
            name_or(
                &plain_line(if p.name.is_empty() { &c.name } else { &p.name }),
                UNKNOWN_AGENT,
            )
        })
        .unwrap_or_default();
    let waiting = if person.is_some() {
        logic::waiting_text(&c.waiting, &person_name)
    } else {
        String::new()
    };
    let body_w = f.cx.window_layout("body").map(|r| r[2]);
    let roomy = body_w.is_some_and(|w| w >= MEMBERS_ROOMY);
    // Toggled on, the column still shows only from Tailwind's lg up.
    let want_members = c.kind == "channel" && s.talk.members.unwrap_or(roomy);
    let show_members = want_members && body_w.is_none_or(|w| w >= MEMBERS_WIDE);

    // The header.
    let mut title = row()
        .key("talk-conv-title")
        .gap(8.0)
        .center_y()
        .min_w(0.0)
        .shrink();
    let title_font = ts(15.0, 600.0);
    let small = |t: String| {
        text(t)
            .font(ts(12.0, 400.0))
            .color(k.c.text_muted)
            .max_lines(1)
    };
    if c.kind == "channel" {
        title = title
            .child(icon(&icons::HASH, 16.0).color(k.c.text_muted))
            .child(
                text(plain_line(&c.name))
                    .font(title_font)
                    .color(k.c.text_base)
                    .max_lines(1)
                    .shrink(),
            );
    } else if person.is_some() {
        title = title
            .child(icon(&crate::ui::icons::USER, 16.0).color(k.c.text_muted))
            .child(
                text(person_name.clone())
                    .font(title_font)
                    .color(k.c.text_base)
                    .max_lines(1)
                    .shrink(),
            );
        if !waiting.is_empty() {
            title = title.child(small(waiting.clone()));
        }
    } else if c.read_only {
        let names: Vec<String> = other
            .iter()
            .map(|m| crate::logic::names::member_name(&m.kind, &m.name))
            .collect();
        title = title
            .child(icon(&icons::EYE, 16.0).color(k.c.text_muted))
            .child(
                text(names.join(" ↔ "))
                    .font(title_font)
                    .color(k.c.text_base)
                    .max_lines(1)
                    .shrink(),
            );
    } else {
        if let Some(m) = other.first() {
            let who = logic::ref_of(m, &live);
            title = title.child(face(s, f, "talk-conv-face", &who, FaceOpts::default()));
        }
        let name = other
            .first()
            .map(|m| crate::logic::names::member_name(&m.kind, &m.name))
            .unwrap_or_else(|| name_or(&c.name, UNKNOWN_AGENT));
        title = title.child(
            text(name)
                .font(title_font)
                .color(k.c.text_base)
                .max_lines(1)
                .shrink(),
        );
        if let Some(m) = other.first() {
            title = title.child(small(logic::member_status(m, &live, now)));
        }
    }
    let mut header = row()
        .key("talk-conv-header")
        .w_full()
        .px(16.0)
        .py(10.0)
        .gap(8.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_muted)
        .child(title);
    if let Some(topic) = c.topic.as_deref().filter(|t| !t.is_empty()) {
        header = header.child(small(plain_line(topic)).min_w(0.0).shrink());
    }
    header = header.child(el().grow(1.0));
    // A channel, or a DM with a person that has reached them, has a huddle
    // (#654); an agent's DM and a read-only one never do.
    let can_huddle =
        !c.read_only && (c.kind == "channel" || (person.is_some() && waiting.is_empty()));
    if can_huddle {
        let sheet_here = s
            .call
            .sheet
            .as_ref()
            .is_some_and(|sh| sh.conv_id == c.conv_id);
        if !s.call.error.is_empty() && s.call.at == c.conv_id && !sheet_here {
            header = header.child(
                text(s.call.error.clone())
                    .key("talk-huddle-error")
                    .role("alert")
                    .help(s.call.error.clone())
                    .font(ts(11.0, 400.0))
                    .color(status::failed())
                    .max_lines(1)
                    .max_w(240.0)
                    .shrink(),
            );
        }
        header = header.child(crate::call::huddle_button(
            s,
            f,
            &c.conv_id,
            &c.kind,
            &c.name,
            !c.space_id.is_empty(),
            &person_name,
        ));
    }
    if c.kind == "channel" {
        header = header.child(
            Button::new(c.members.len().to_string())
                .icon(&icons::USERS)
                .variant(if want_members {
                    ButtonVariant::Neutral
                } else {
                    ButtonVariant::Ghost
                })
                .control_size(ControlSize::Small)
                .help("Who's in this channel")
                .on_press(move |s: &mut State| s.talk.members = Some(!want_members))
                .build(k)
                .key("talk-members-toggle"),
        );
    }
    if person.is_some() {
        header = header.child(leave_controls(s, f, c, &person_name));
    }

    // The posts, or the share on in this huddle in their place (#658).
    let here = crate::call::in_call_here(s, &c.conv_id);
    let stage = if here {
        crate::call::share_stage(s, f)
    } else {
        None
    };
    let posts = match stage {
        Some(st) => st,
        None => posts_view(s, f, c, &live, now),
    };
    let foot = if c.read_only {
        row()
            .key("talk-read-only")
            .w_full()
            .px(16.0)
            .py(10.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_muted)
            .child(
                text("Agents talking to each other: you can read it, not post. Nothing between agents is hidden.")
                    .font(ts(12.0, 400.0))
                    .color(k.c.text_muted)
                    .w_full()
                    .wrap(),
            )
    } else {
        composer(s, f, c, other.first().copied())
    };
    let main = col()
        .key("talk-conv-main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .child(header)
        .child(posts.grow(1.0).min_h(0.0))
        .child(foot);
    let mut out = row()
        .key("talk-conv")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(main);
    // Beside the posts: the huddle while this device is in it here, else an
    // ended huddle's transcript, else the members.
    if here {
        out = out.child(crate::call::huddle_panel(s, f));
    } else if let Some(t) = crate::call::transcript_pane(s, f) {
        out = out.child(t);
    } else if show_members {
        out = out.child(members(s, f, &c.members, &live, now));
    }
    out
}

/// Leave on a DM with another person (card #538), with its confirmation.
fn leave_controls(s: &State, f: &Frame, c: &TalkConversation, person: &str) -> El {
    let k = f.k;
    let mut r = row().key("talk-leave").gap(6.0).center_y();
    if !s.talk.leave_error.is_empty() {
        r = r.child(
            text(s.talk.leave_error.clone())
                .font(ts(11.0, 400.0))
                .color(status::failed())
                .max_lines(1),
        );
    }
    if s.talk.leaving {
        let id = c.conv_id.clone();
        r = r
            .child(
                text("Its messages go from your devices.")
                    .font(ts(12.0, 400.0))
                    .color(k.c.text_muted),
            )
            .child(
                Button::new("Leave")
                    .variant(ButtonVariant::Danger)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut State| {
                        let id = id.clone();
                        s.spawn(
                            move |w| w.talk_dm_leave(&id),
                            |s, r| match r {
                                Ok(()) => {
                                    s.talk.leaving = false;
                                    super::open(s, "");
                                }
                                Err(e) => s.talk.leave_error = e,
                            },
                        );
                    })
                    .build(k),
            )
            .child(
                Button::new("Cancel")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut State| s.talk.leaving = false)
                    .build(k),
            );
    } else {
        r = r.child(
            Button::new("Leave")
                .icon(&icons::LOG_OUT)
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .help(format!("Leave your DM with {person}"))
                .on_press(|s: &mut State| s.talk.leaving = true)
                .build(k),
        );
    }
    r
}

/// The posts, oldest first, in a scroller that keeps to the newest. The
/// rows are a virtualized stack: only those on screen are laid out.
fn posts_view(
    s: &State,
    f: &mut Frame,
    c: &TalkConversation,
    live: &HashMap<String, LiveAgent>,
    now: i64,
) -> El {
    let k = f.k;
    let p = &s.talk.posts;
    let scroller = col()
        .key("talk-posts")
        .w_full()
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: true });
    if p.conv != c.conv_id || !p.loaded {
        return scroller.child(
            col()
                .key("talk-posts-loading")
                .w_full()
                .padding(12.0)
                .child(crate::widgets::loading::post_rows_shape(
                    f,
                    "talk-posts-shape",
                    4,
                )),
        );
    }
    let mut rows: Vec<El> = Vec::new();
    if let Some(e) = &p.error {
        rows.push(
            row()
                .key("talk-posts-error")
                .w_full()
                .px(20.0)
                .pt(12.0)
                .child(
                    text(format!("Could not load the posts: {e}"))
                        .font(ts(12.0, 400.0))
                        .color(status::failed())
                        .w_full()
                        .wrap(),
                ),
        );
    }
    if p.list.is_empty() && p.error.is_none() {
        let what = if c.kind == "channel" {
            "Nothing here yet. Agents wake only when you @-mention them; the rest read it at their next turn.".to_string()
        } else if c.read_only {
            "Nothing between them yet.".to_string()
        } else if let Some(pp) = logic::dm_person(c) {
            format!(
                "Nothing yet. Only you and {} can read this.",
                name_or(
                    &plain_line(if pp.name.is_empty() {
                        &c.name
                    } else {
                        &pp.name
                    }),
                    UNKNOWN_AGENT
                )
            )
        } else {
            "Nothing yet. Every message here wakes the agent.".to_string()
        };
        rows.push(
            row()
                .key("talk-posts-empty")
                .w_full()
                .px(20.0)
                .pt(12.0)
                .child(
                    text(what)
                        .font(ts(12.5, 400.0))
                        .color(k.c.text_muted)
                        .w_full()
                        .wrap(),
                ),
        );
    }
    let mut prev_day = None;
    for (i, post) in p.list.iter().enumerate() {
        let at = logic::at_ms(&post.at);
        let (day, clock) = if at > 0 {
            logic::local(at)
        } else {
            ((0, 0, 0), String::new())
        };
        let today = logic::local(now).0;
        let new_day = at > 0 && prev_day != Some(day);
        prev_day = Some(day);
        let mut cell = col()
            .key(format!("talk-row-{}", post.post_id))
            .w_full()
            .px(12.0);
        if i == 0 {
            cell = cell.pt(12.0);
        }
        if new_day {
            cell = cell.child(day_line(f, &post.post_id, &logic::day_label(day, today)));
        }
        cell = cell.child(post_row(s, f, c, post, &clock, live));
        rows.push(cell);
    }
    if !c.read_only
        && let Some(line) = logic::replying_line(
            s.talk
                .replying
                .iter()
                .find(|r| r.conv_id == c.conv_id)
                .map_or(&[][..], |r| &r.agents[..]),
            now,
        )
    {
        rows.push(replying_row(f, &line));
    }
    rows.push(el().key("talk-posts-end").h(12.0).w_full());
    scroller.child(
        col()
            .key("talk-post-rows")
            .w_full()
            .growth(GrowthSpec::default())
            .virtualize(64.0)
            .children(rows),
    )
}

/// The line between days.
fn day_line(f: &Frame, id: &str, label: &str) -> El {
    let k = f.k;
    let rule = || el().h(1.0).grow(1.0).fill(k.c.border_muted);
    row()
        .key(format!("talk-day-{id}"))
        .w_full()
        .px(8.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(rule())
        .child(
            text(label.to_string())
                .font(ts(11.5, 400.0))
                .color(k.c.text_muted),
        )
        .child(rule())
}

/// "heron and otter are replying…" under the posts (card #645).
fn replying_row(f: &Frame, line: &ReplyLine) -> El {
    let k = f.k;
    row()
        .key("talk-replying")
        .w_full()
        .px(16.0)
        .py(4.0)
        .gap(8.0)
        .center_y()
        .role("status")
        .label(line.text.clone())
        .child(crate::widgets::bits::spinner(f, "talk-replying-spin"))
        .child(
            text(line.who.clone())
                .font(ts(12.5, 500.0))
                .color(k.c.text_base),
        )
        .child(
            text(format!("{} replying…", line.verb))
                .font(ts(12.5, 400.0))
                .color(k.c.text_muted),
        )
}

/// One post: its author's face, name, time and text, its files and the
/// card made from it; a huddle's start or end, or a thread's pointer, is
/// one line.
fn post_row(
    s: &State,
    f: &mut Frame,
    c: &TalkConversation,
    post: &TalkPost,
    clock: &str,
    live: &HashMap<String, LiveAgent>,
) -> El {
    let k = f.k;
    let key = format!("talk-p-{}", post.post_id);
    let m = logic::member_of(&c.members, &post.by).unwrap_or_else(|| TalkMember {
        kind: if post.by.starts_with("person:") {
            "person".into()
        } else {
            "agent".into()
        },
        key: post.by.clone(),
        ..TalkMember::default()
    });
    let name = logic::shown_name(&m);

    // A huddle's start or end (#654).
    if let Some(call) = &post.call {
        return if call.op == "end" {
            crate::call::huddle_ended(
                f,
                &key,
                call.duration_ms,
                &call.people,
                &call.call_id,
                call.transcript,
            )
        } else {
            crate::call::huddle_started(
                s,
                f,
                &key,
                &name,
                &call.call_id,
                crate::call::HuddleIn {
                    conv: &c.conv_id,
                    kind: &c.kind,
                    name: &plain_line(&c.name),
                },
            )
        };
    }

    // A thread started from a message here leaves one line (design b2).
    if post.thread_from.is_some() {
        let r = crate::text::rich_with(
            s,
            f,
            &format!("{key}-t"),
            &post.text,
            &RichOpts {
                font: Some(ts(12.0, 400.0)),
                max_lines: 1,
                ..RichOpts::default()
            },
        );
        return row()
            .key(format!("{key}-wrap"))
            .w_full()
            .pl(36.0)
            .py(2.0)
            .child(
                row()
                    .key(key)
                    .min_w(0.0)
                    .px(8.0)
                    .py(4.0)
                    .gap(6.0)
                    .center_y()
                    .radius(6.0)
                    .border(1.0, k.c.border_muted)
                    .child(icon(&icons::MESSAGE_TEXT, 14.0).color(k.c.text_muted))
                    .child(
                        text(format!("{name} started a thread:"))
                            .font(ts(12.0, 400.0))
                            .color(k.c.text_muted),
                    )
                    .child(r.min_w(0.0).shrink())
                    .child(
                        text("· in Threads")
                            .font(ts(12.0, 400.0))
                            .color(k.c.text_muted),
                    ),
            );
    }

    let who = logic::ref_of(&m, live);
    let mentions: Vec<CardMention> = post
        .mentions
        .iter()
        .map(|key| {
            let mm = logic::member_of(&c.members, key).unwrap_or_else(|| TalkMember {
                kind: if key.starts_with("person:") {
                    "person".into()
                } else {
                    "agent".into()
                },
                key: key.clone(),
                ..TalkMember::default()
            });
            let r = logic::ref_of(&mm, live);
            CardMention {
                name: if mm.kind == "person" && mm.name == "you" {
                    "you".into()
                } else {
                    crate::logic::names::member_name(&mm.kind, &mm.name)
                },
                gone: r.status == "gone",
                who: r,
            }
        })
        .collect();
    let can_post = !c.read_only;
    let channel = c.kind == "channel";
    let can_card = can_post && post.card.is_none();

    let mut head = row()
        .key(format!("{key}-head"))
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(name.clone())
                .font(ts(13.0, 500.0))
                .color(k.c.text_base)
                .max_lines(1)
                .shrink(),
        );
    if m.kind == "agent" {
        head = head.child(text("AGENT").font(ts(10.5, 400.0)).color(k.c.text_muted));
    }
    if !clock.is_empty() {
        head = head.child(
            text(clock.to_string())
                .font(ts(11.0, 400.0))
                .color(k.c.text_muted),
        );
    }
    head = head.child(el().grow(1.0));
    let hover_btn = |key: String, label: &str, ic: &'static rune::Icon, tip: &str| {
        row()
            .key(key)
            .role("button")
            .focusable(true)
            .px(6.0)
            .py(2.0)
            .gap(4.0)
            .center_y()
            .radius(4.0)
            .hover(k.c.hover)
            .hover_alone()
            .group_opacity(0.0, 1.0)
            .help(tip.to_string())
            .child(icon(ic, 14.0).color(k.c.text_muted))
            .child(
                text(label.to_string())
                    .font(ts(11.0, 400.0))
                    .color(k.c.text_muted),
            )
    };
    if channel && can_post {
        let pid = post.post_id.clone();
        let author = name.clone();
        head = head.child(
            hover_btn(
                format!("{key}-ask"),
                "Start a thread",
                &icons::MESSAGE_TEXT,
                "Ask about this in a thread: it goes in Threads, and this channel gets a one-line pointer",
            )
            .on_press(move |s: &mut State| toggle_asking(s, &pid, &author)),
        );
    }
    if can_card {
        let pid = post.post_id.clone();
        head = head.child(
            hover_btn(
                format!("{key}-card"),
                "Make a card",
                &icons::SQUARE_PLUS,
                "Work goes on a card: make one from this message",
            )
            .on_press(move |s: &mut State| start_making(s, &pid)),
        );
    }

    let mut body = col()
        .key(format!("{key}-body"))
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(head);
    if !post.text.trim().is_empty() {
        body = body.child(crate::text::rich_with(
            s,
            f,
            &format!("{key}-text"),
            &post.text,
            &RichOpts {
                font: Some(ts(13.0, 400.0)),
                mentions: &mentions,
                ..RichOpts::default()
            },
        ));
    }
    if !post.attachments.is_empty() {
        body = body.child(crate::text::attachment_list(
            s,
            f,
            &format!("{key}-files"),
            &File::all(&post.attachments),
            false,
        ));
    }
    if let Some(card) = &post.card {
        body = body.child(card_chip(s, f, &key, card));
    }
    if let Some(a) = s
        .talk
        .asking
        .as_ref()
        .filter(|a| a.post_id == post.post_id && channel)
    {
        body = body.child(thread_from(f, &key, a, c, post));
    }
    let face_el = face(s, f, &format!("{key}-face"), &who, FaceOpts::default());
    let mut r = row()
        .key(key.clone())
        .w_full()
        .px(8.0)
        .py(6.0)
        .gap(10.0)
        .radius(8.0)
        .hover(k.c.hover)
        .hover_group()
        .child(el().pt(2.0).child(face_el))
        .child(body);
    if !(channel && can_post) && !can_card {
        return r;
    }
    // Right-click: the same two actions as the hover buttons.
    let pid = post.post_id.clone();
    r = r.on_secondary_press(move |s: &mut State, p, _tx: &mut Tx| {
        s.talk.menu = Some((pid.clone(), p));
    });
    let Some((_, point)) = s.talk.menu.as_ref().filter(|(id, _)| *id == post.post_id) else {
        return r;
    };
    let mut menu = rune_kit::context_menu(format!("{key}-menu"), r, Some(*point))
        .on_close(|s: &mut State| s.talk.menu = None);
    if channel && can_post {
        let (pid, author) = (post.post_id.clone(), name.clone());
        menu = menu.item(
            MenuItem::new("Reply in thread")
                .icon(&icons::MESSAGE_TEXT)
                .on_select(move |s: &mut State| {
                    s.talk.menu = None;
                    if s.talk.asking.as_ref().is_none_or(|a| a.post_id != pid) {
                        toggle_asking(s, &pid, &author);
                    }
                }),
        );
    }
    if can_card {
        let pid = post.post_id.clone();
        menu = menu.item(
            MenuItem::new("Make a card")
                .icon(&icons::SQUARE_PLUS)
                .on_select(move |s: &mut State| {
                    s.talk.menu = None;
                    start_making(s, &pid);
                }),
        );
    }
    menu.build(k, f.layer)
}

fn toggle_asking(s: &mut State, post_id: &str, author: &str) {
    if s.talk.asking.as_ref().is_some_and(|a| a.post_id == post_id) {
        s.talk.asking = None;
        return;
    }
    let mut edit = rune::TextEdit::default();
    edit.focused = true;
    s.talk.asking = Some(super::Asking {
        post_id: post_id.to_string(),
        author: author.to_string(),
        edit,
        ..super::Asking::default()
    });
}

/// Ask about a channel message in a new thread (design b2): the thread
/// goes in Threads, and the channel gets a pointer to it.
fn thread_from(
    f: &Frame,
    key: &str,
    a: &super::Asking,
    c: &TalkConversation,
    post: &TalkPost,
) -> El {
    let k = f.k;
    let conv = c.conv_id.clone();
    let channel = c.name.clone();
    let post_id = post.post_id.clone();
    let line = logic::post_line(Some(post));
    let author = a.author.clone();
    let mut out = col()
        .key(format!("{key}-ask-form"))
        .w_full()
        .pt(4.0)
        .gap(4.0)
        .child(
            TextInput::new(&a.edit)
                .placeholder("Ask about this in a thread…")
                .label("Your question for the thread")
                .height(28.0)
                .width(None)
                .disabled(a.busy)
                .on_edit(|s: &mut State| {
                    &mut s.talk.asking.get_or_insert_with(Default::default).edit
                })
                .on_key(|s: &mut State, ev: &rune::KeyEvent, _: &mut Tx| {
                    if matches!(ev.key, Key::Escape) {
                        s.talk.asking = None;
                        return true;
                    }
                    false
                })
                .on_submit(move |s: &mut State, _: &mut Tx| {
                    let Some(a) = s.talk.asking.as_mut() else {
                        return;
                    };
                    let q = a.edit.text.clone();
                    if q.trim().is_empty() || a.busy {
                        return;
                    }
                    a.busy = true;
                    a.error.clear();
                    let ask = logic::thread_text(&q, &author, &line, &channel);
                    let (conv, post_id) = (conv.clone(), post_id.clone());
                    s.spawn(
                        move |w| w.talk_thread_from(&conv, &post_id, &ask).map(|_| ()),
                        |s, r| match r {
                            Ok(()) => s.talk.asking = None,
                            Err(e) => {
                                if let Some(a) = s.talk.asking.as_mut() {
                                    a.busy = false;
                                    a.error = e;
                                }
                            }
                        },
                    );
                })
                .build(k),
        );
    if !a.error.is_empty() {
        out = out.child(
            text(a.error.clone())
                .font(ts(11.0, 400.0))
                .color(status::failed())
                .w_full()
                .wrap(),
        );
    }
    out
}

/// The card made from a message (design §4): one click to it.
fn card_chip(s: &State, f: &Frame, key: &str, id: &str) -> El {
    let k = f.k;
    let c = s.data.card(id);
    let r = c
        .map(crate::logic::card_status::card_ref)
        .unwrap_or_default();
    let title = c.map_or("a card".to_string(), |c| plain_line(&c.title));
    let open = id.to_string();
    row()
        .key(format!("{key}-made"))
        .role("button")
        .focusable(true)
        .gap(6.0)
        .center_y()
        .min_w(0.0)
        .shrink()
        .on_press(move |s: &mut State| s.open_card(&open))
        .child(icon(&icons::ARROW_UP_RIGHT, 14.0).color(k.c.text_muted))
        .child(
            text("Made a card:")
                .font(ts(11.5, 400.0))
                .color(k.c.text_muted),
        )
        .child(
            row()
                .key(format!("{key}-made-chip"))
                .gap(4.0)
                .px(6.0)
                .py(2.0)
                .radius(6.0)
                .min_w(0.0)
                .shrink()
                .center_y()
                .fill(k.c.layer_02)
                .hover(k.c.hover)
                .child(text(r).font(ts(11.5, 400.0).mono()).color(k.c.text_accent))
                .child(
                    text(title)
                        .font(ts(11.5, 400.0))
                        .color(k.c.text_base)
                        .max_lines(1)
                        .shrink(),
                ),
        )
}

/// A channel's people and agents with how each stands now.
fn members(
    s: &State,
    f: &mut Frame,
    ms: &[TalkMember],
    live: &HashMap<String, LiveAgent>,
    now: i64,
) -> El {
    let k = f.k;
    let mut sorted: Vec<&TalkMember> = ms.iter().collect();
    sorted.sort_by(|a, b| {
        if a.kind == b.kind {
            logic::shown_name(a)
                .to_lowercase()
                .cmp(&logic::shown_name(b).to_lowercase())
        } else if a.kind == "person" {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    });
    let mut list = col().key("talk-members-list").w_full().gap(6.0);
    for m in sorted {
        let st = logic::status_of(m, live);
        let gone = st == "gone";
        let who = logic::ref_of(m, live);
        let key = format!("talk-m-{}", m.key);
        let shown = logic::shown_name(m);
        let len = shown.len();
        let mut name = text(shown)
            .font(ts(12.5, 400.0))
            .color(if gone { k.c.text_muted } else { k.c.text_base })
            .max_lines(1)
            .shrink();
        if gone {
            name = name.strike(0, len, None);
        }
        list = list.child(
            row()
                .key(key.clone())
                .w_full()
                .gap(8.0)
                .center_y()
                .child(face(s, f, &format!("{key}-f"), &who, FaceOpts::default()))
                .child(name)
                .child(el().grow(1.0))
                .child(
                    text(logic::member_status(m, live, now))
                        .font(ts(11.0, 400.0))
                        .color(k.c.text_muted)
                        .max_lines(1),
                ),
        );
    }
    col()
        .key("talk-members")
        .w(220.0)
        .h_full()
        .min_h(0.0)
        .px(12.0)
        .py(12.0)
        .border_sides([0.0, 0.0, 0.0, 1.0], k.c.border_muted)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            text(format!("MEMBERS · {}", ms.len()))
                .font(ts(11.0, 500.0))
                .color(k.c.text_muted)
                .pb(8.0),
        )
        .child(list)
        .child(
            text("@-mention an agent to bring it in.")
                .font(ts(11.0, 400.0))
                .color(k.c.text_muted)
                .pt(12.0)
                .w_full()
                .wrap(),
        )
}

/// The box to post: the @ and # pickers, files, "Agents answer now" on a
/// channel, Send.
fn composer(s: &State, f: &mut Frame, c: &TalkConversation, other: Option<&TalkMember>) -> El {
    let k = f.k;
    let channel = c.kind == "channel";
    let placeholder = if channel {
        format!("Message #{}", plain_line(&c.name))
    } else {
        format!(
            "Message {}",
            other.map_or_else(
                || name_or(&c.name, UNKNOWN_AGENT),
                |m| crate::logic::names::member_name(&m.kind, &m.name)
            )
        )
    };
    let boxc = post_box(s);
    let kinds = logic::mention_kinds(boxc.mentions.iter().map(|m| m.key.as_str()));
    let default_now = logic::agents_now_for(&kinds);
    let now_on = s.talk.agents_now.unwrap_or(default_now);
    let mut extra = Vec::new();
    if channel {
        extra.push(crate::widgets::agents_now::agents_now_toggle(
            f,
            "talk-agents-now",
            now_on,
            boxc.sending,
            |s: &mut State, on| s.talk.agents_now = Some(on),
        ));
    }
    let conv = c.conv_id.clone();
    let on_send: crate::text::SendFn = Rc::new(move |s: &mut State, m: Sent| {
        let kinds = logic::mention_kinds(m.mentions.iter().map(|x| x.key.as_str()));
        let now = if channel {
            s.talk.agents_now.unwrap_or(logic::agents_now_for(&kinds))
        } else {
            logic::agents_now_for(&kinds)
        };
        let keys: Vec<String> = m.mentions.iter().map(|x| x.key.clone()).collect();
        let conv = conv.clone();
        post_box_mut(s).sending = true;
        s.talk.post_error.clear();
        s.spawn(
            move |w| w.talk_post(&conv, m.text.trim(), &keys, now, None, &m.files),
            |s, r| match r {
                Ok(_) => {
                    crate::text::sent(s, post_box_mut);
                    s.talk.agents_now = None;
                    super::load_posts(s);
                }
                Err(e) => {
                    post_box_mut(s).sending = false;
                    s.talk.post_error = e;
                }
            },
        );
    });
    extra.push(send_button(
        f,
        "talk-post-send",
        boxc,
        false,
        ComposerOpts {
            board: c.space_id.clone(),
            on_send: Some(on_send.clone()),
            ..ComposerOpts::default()
        },
        post_box_mut,
    ));
    let opts = ComposerOpts {
        placeholder: placeholder.clone(),
        label: placeholder,
        extra,
        board: c.space_id.clone(),
        rows: Some((1, 8)),
        mic: Some(crate::call::mic_button_in(
            s,
            f,
            "talk-composer",
            |s: &mut State| &mut s.talk.post_box.edit,
            boxc.sending,
        )),
        on_send: Some(on_send),
        ..ComposerOpts::default()
    };
    let mut out = col()
        .key("talk-composer")
        .w_full()
        .px(12.0)
        .py(10.0)
        .gap(4.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_muted)
        .child(crate::text::composer(
            s,
            f,
            "talk-post",
            post_box,
            post_box_mut,
            &opts,
        ));
    if !s.talk.post_error.is_empty() {
        out = out.child(
            text(s.talk.post_error.clone())
                .font(ts(12.0, 400.0))
                .color(status::failed())
                .px(4.0)
                .w_full()
                .wrap(),
        );
    }
    out
}

/// A box's Send, as React's Talk boxes had it: a "Send" button
/// ("Sending…" while it goes), off while the box is empty. It sends what
/// the composer holds, as Enter does.
pub(crate) fn send_button(
    f: &Frame,
    key: &str,
    c: &crate::text::Composer,
    // Files alone don't send (Start a thread asks in words).
    needs_text: bool,
    // What sending needs: the box's board, no_mentions and on_send.
    o: ComposerOpts,
    get_mut: crate::text::GetMut,
) -> El {
    Button::new(if c.sending { "Sending…" } else { "Send" })
        .variant(ButtonVariant::Submit)
        .control_size(ControlSize::Small)
        .disabled(c.sending || c.is_empty() || (needs_text && c.edit.text.trim().is_empty()))
        .on_press(move |s: &mut State| crate::text::submit(s, get_mut, &o))
        .build(f.k)
        .key(key.to_string())
}

/// Make a card from post `post_id` (design §4): New card opens filled
/// from it, in the channel's space (card #558); once made, the post links
/// to the card.
pub fn start_making(s: &mut State, post_id: &str) {
    let Some(c) = s.talk.convs.iter().find(|c| c.conv_id == s.talk.posts.conv) else {
        return;
    };
    let Some(p) = s.talk.posts.list.iter().find(|p| p.post_id == post_id) else {
        return;
    };
    let where_ = if c.kind == "channel" {
        format!("#{}", c.name)
    } else {
        "a DM".to_string()
    };
    let files: Vec<logic::PostFile> = p
        .attachments
        .iter()
        .map(|f| logic::PostFile {
            name: &f.name,
            size: f.size,
            sha256: &f.sha256,
        })
        .collect();
    let (title, objective) = logic::from_post(&p.text, &where_, &files);
    // A space's channel makes its cards in the space; anywhere else they
    // are the person's own.
    let board = s
        .data
        .spaces
        .iter()
        .find(|b| b.board_id == c.space_id)
        .map(|b| b.board_id.clone());
    s.talk.card_from = Some(super::CardFrom {
        post_id: post_id.to_string(),
        title,
        objective,
    });
    s.open_dialog(crate::ui::Dialog::NewCard { board });
}
