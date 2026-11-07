//! The pieces of the Spaces pages (parts.tsx): New space (make one, or
//! join with an invite), an open invite either side (`InvitePanel`,
//! `SendLink`, `Fingerprints`, `TypeInviteWords`, `ReadInviteWords`), how
//! an invite works, and Space settings (`BoardInvite`, `SpaceAgentSwitch`,
//! `BoardSettingsPanel`, `Reviewers`); settings/space-agents.tsx's
//! "agents are off" line.

use rune::{El, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, Checkbox, Dialog, Select, SelectAppearance, Switch,
    TextInput,
};

use irori_window::Window;
use irori_window::agents::AgentsChange;
use irori_window::boards::{BoardSettings, BoardSummary};
use irori_window::invites::{INVITE_DONE, INVITE_FAILED, INVITE_JOINING, INVITE_WAITING, INVITE_WORDS, InviteStatus};

use super::logic::{self, MAX_BOARD_TAG, plain_line};
use super::page::{clamp, lower};
use super::ui::{self as u, t12, t13};
use super::{MemberAsk, NewSpace, NewTab, Settings};
use crate::ui::{Frame, State, icons};

// ---- New space ---------------------------------------------------------------

/// New space (card #126): make a space, or join one with an invite, as two
/// tabs. Joining closes it; the invite then runs over the Spaces list. One
/// invite at a time: Join is offered only while none is open.
pub fn new_space_dialog(s: &State, f: &mut Frame) {
    let Some(n) = &s.spaces.new_space else { return };
    let k = f.k;
    let join = s.spaces.invite.as_ref().is_none_or(|i| i.state.is_empty());
    let tab = if join { n.tab } else { NewTab::Make };
    let (title, desc) = match tab {
        NewTab::Make => (
            "New space",
            "A group of people, with its boards and Talk. Only the people you invite see it.",
        ),
        NewTab::Join => ("Join a space", "Paste the invite link someone sent you."),
    };
    let mut body = col().key("spaces-new-body").w_full().gap(12.0).px(16.0).pb(16.0);
    if join {
        let mut tabs = row().key("spaces-new-tabs").gap(4.0).role("tablist").label("New space or join one");
        for (t, label) in [(NewTab::Make, "Make a space"), (NewTab::Join, "Join with an invite")] {
            tabs = tabs.child(
                Button::new(label)
                    .variant(if tab == t { ButtonVariant::Neutral } else { ButtonVariant::Ghost })
                    .on_press(move |s: &mut State| {
                        if let Some(n) = s.spaces.new_space.as_mut() {
                            n.tab = t;
                            n.error = None;
                            // Each tab's field takes the keyboard (autoFocus).
                            match t {
                                NewTab::Make => n.name.focused = true,
                                NewTab::Join => n.link.focused = true,
                            }
                        }
                    })
                    .build(k)
                    .key(if t == NewTab::Make { "spaces-new-tab-make" } else { "spaces-new-tab-join" })
                    .role("tab")
                    .checked(tab == t),
            );
        }
        body = body.child(tabs);
    }
    body = body.child(match tab {
        NewTab::Make => make_space(s, f, n),
        NewTab::Join => join_invite(f, n),
    });
    Dialog::new("spaces-new", title)
        .description(desc)
        .width(512.0)
        .fit()
        .open(true)
        .on_close(|s: &mut State| s.spaces.new_space = None)
        .child(body)
        .build(k, f.layer);
}

fn new_tag(s: &State, n: &NewSpace) -> (String, bool) {
    let t = n.tag.text.trim().to_lowercase();
    let clash = !t.is_empty() && s.data.spaces.iter().any(|b| b.tag == t);
    (t, clash)
}

fn make_space(s: &State, f: &Frame, n: &NewSpace) -> El {
    let k = f.k;
    let (t, clash) = new_tag(s, n);
    let ok = !n.busy && !n.name.text.trim().is_empty() && logic::valid_tag(&t) && !clash;
    let (hint, warn) = if clash {
        (format!("Another board is tagged {t} already."), true)
    } else if !t.is_empty() && !logic::valid_tag(&t) {
        (
            format!("A tag is a lowercase letter, then letters, digits or '-' ({MAX_BOARD_TAG} at most)."),
            true,
        )
    } else {
        let tt = if t.is_empty() { "tag" } else { t.as_str() };
        (format!("Its cards are {tt}#1, {tt}#2, …"), false)
    };
    let mut c = col()
        .w_full()
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(
                    TextInput::new(&n.name)
                        .placeholder("Space name")
                        .label("New space's name")
                        .width(None)
                        .on_edit(|s: &mut State| &mut s.spaces.new_space.get_or_insert_with(NewSpace::default).name)
                        .on_change(|s: &mut State, _| {
                            if let Some(n) = s.spaces.new_space.as_mut() {
                                clamp(&mut n.name, logic::MAX_BOARD_NAME);
                                if !n.typed_tag {
                                    let t = logic::suggest_tag(&n.name.text);
                                    n.tag.replace_text(t);
                                }
                            }
                        })
                        .on_submit(|s: &mut State, _| create_space(s))
                        .build(k)
                        .key("spaces-new-name")
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    TextInput::new(&n.tag)
                        .placeholder("tag")
                        .label("Tag for its first board, the short name in card refs like tag#12")
                        .width(Some(96.0))
                        .text_style(t13(k).mono())
                        .on_edit(|s: &mut State| &mut s.spaces.new_space.get_or_insert_with(NewSpace::default).tag)
                        .on_change(|s: &mut State, _| {
                            if let Some(n) = s.spaces.new_space.as_mut() {
                                n.typed_tag = true;
                                lower(&mut n.tag);
                                clamp(&mut n.tag, MAX_BOARD_TAG);
                            }
                        })
                        .on_submit(|s: &mut State, _| create_space(s))
                        .build(k)
                        .key("spaces-new-tag"),
                )
                .child(
                    Button::new("Create")
                        .disabled(!ok)
                        .on_press(create_space)
                        .build(k),
                ),
        )
        .child(u::line(hint, t12(k), if warn { u::warning(k) } else { k.c.text_muted }));
    if let Some(e) = &n.error {
        c = c.child(u::error_line(k, plain_line(e)));
    }
    c
}

fn create_space(s: &mut State) {
    let Some(n) = s.spaces.new_space.as_ref() else { return };
    let (t, clash) = new_tag(s, n);
    let name = n.name.text.clone();
    if n.busy || name.trim().is_empty() || !logic::valid_tag(&t) || clash {
        return;
    }
    if let Some(n) = s.spaces.new_space.as_mut() {
        n.busy = true;
        n.error = None;
    }
    s.spawn(
        move |w| w.boards_create(&name, &t),
        |s, r| match r {
            Ok(_) => {
                s.spaces.new_space = None;
                super::reload_spaces(s);
            }
            Err(e) => {
                if let Some(n) = s.spaces.new_space.as_mut() {
                    n.busy = false;
                    n.error = Some(logic::error_text(&e));
                }
            }
        },
    );
}

/// Join with an invite: the irori-invite: link someone sent.
fn join_invite(f: &Frame, n: &NewSpace) -> El {
    let k = f.k;
    let ok = !n.busy && !n.link.text.trim().is_empty();
    let mut c = col()
        .w_full()
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(
                    TextInput::new(&n.link)
                        .placeholder("irori-invite:…")
                        .label("The invite link someone sent you")
                        .width(None)
                        .text_style(t13(k).mono())
                        .on_edit(|s: &mut State| &mut s.spaces.new_space.get_or_insert_with(NewSpace::default).link)
                        .on_submit(|s: &mut State, _| join_submit(s))
                        .build(k)
                        .key("spaces-join-link")
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    Button::new("Join")
                        .disabled(!ok)
                        .on_press(join_submit)
                        .build(k),
                ),
        )
        .child(u::muted(k, "Paste the link, then give whoever sent it the five words you'll see."));
    if let Some(e) = &n.error {
        c = c.child(u::error_line(k, plain_line(e)));
    }
    c
}

fn join_submit(s: &mut State) {
    let Some(n) = s.spaces.new_space.as_mut() else { return };
    let link = n.link.text.trim().to_string();
    if n.busy || link.is_empty() {
        return;
    }
    n.busy = true;
    n.error = None;
    s.spawn(
        move |w| w.boards_join_invite(&link),
        |s, r| match r {
            Ok(st) => {
                s.spaces.invite = Some(st);
                s.spaces.words.clear();
                s.spaces.words_error = None;
                s.spaces.new_space = None;
            }
            Err(e) => {
                if let Some(n) = s.spaces.new_space.as_mut() {
                    n.busy = false;
                    n.error = Some(logic::error_text(&e));
                }
            }
        },
    );
}

// ---- the invite --------------------------------------------------------------

/// What an invite does, and why the link may not reach far (card #96).
pub fn invite_how(f: &Frame, reach: &str) -> El {
    let k = f.k;
    let mut c = col()
        .w_full()
        .gap(4.0)
        .min_w(0.0)
        .shrink()
        .child(
            rune::rich(
                "How an invite works: you invite a person, not a device. Send them the one-use link (good for an hour), then they give you the five words their screen shows to check it's them. Their other devices follow once they're in.",
                vec![rune::Span::new(0, 20).color(k.c.text_base)],
            )
            .font(t12(k))
            .color(k.c.text_muted)
            .w_full()
            .wrap(),
        );
    if !reach.is_empty() {
        c = c.child(u::line(reach, t12(k), u::warning(k)));
    }
    row()
        .w_full()
        .gap(10.0)
        .child(el().w(2.0).self_stretch().fill(k.alpha(k.c.text_accent, 0.6)))
        .child(c)
}

/// The open invite, either side (§4.1, B4). The inviter sends the link,
/// then types the five words the invitee gives them; both check each
/// other's fingerprint against their own screen.
pub fn invite_panel(s: &State, f: &Frame, inv: &InviteStatus) -> El {
    let k = f.k;
    let open = inv.open();
    let host = inv.role == "host";
    let board = Some(plain_line(&inv.board_name)).filter(|b| !b.is_empty()).unwrap_or("the space".into());
    let peer = Some(plain_line(&inv.peer_name)).filter(|b| !b.is_empty()).unwrap_or("them".into());
    let mut c = col()
        .key("spaces-invite-panel")
        .w_full()
        .gap(12.0)
        .label("Invite")
        .child(
            u::line(
                if host { format!("Inviting someone to {board}") } else { format!("Joining {board}") },
                rune::tok::TextStyle::new(13.0, 600.0),
                k.c.text_base,
            ),
        );
    match inv.state.as_str() {
        INVITE_WAITING if host => c = c.child(send_link(s, f, inv)),
        INVITE_WORDS => {
            c = c.child(fingerprints(f, inv, &peer));
            c = c.child(if host { type_words(s, f, inv, &peer) } else { read_words(f, inv, &peer) });
        }
        INVITE_JOINING => c = c.child(u::line("The words match. Joining…", t13(k), k.c.text_muted)),
        INVITE_DONE => {
            c = c.child(u::line(
                if host { format!("{peer} joined {board}.") } else { format!("You joined {board}.") },
                t13(k),
                k.c.text_base,
            ))
        }
        INVITE_FAILED => {
            let e = plain_line(&inv.error);
            c = c.child(u::line(
                if e.is_empty() { "The invite ended.".to_string() } else { e },
                t13(k),
                u::danger(k),
            ))
        }
        _ => {}
    }
    let close = if open {
        Button::new("Cancel invite")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| {
                s.spawn(
                    |w| w.boards_cancel_invite(),
                    |s, ()| {
                        s.spaces.invite = None;
                        super::reload_spaces(s);
                    },
                )
            })
            .build(k)
    } else {
        Button::new("Close")
            .on_press(|s: &mut State| {
                s.spaces.invite = None;
                super::reload_spaces(s);
            })
            .build(k)
    };
    c.child(row().w_full().justify_end().child(close))
}

fn send_link(s: &State, f: &Frame, inv: &InviteStatus) -> El {
    let k = f.k;
    let until = logic::clock_time(&inv.expires_at);
    let until = if until.is_empty() { String::new() } else { format!(", until {until}") };
    let link = inv.link.clone();
    let mut c = col()
        .w_full()
        .gap(8.0)
        .child(u::line(
            format!("Send this link to the person you're inviting. It works once{until}."),
            t13(k),
            k.c.text_muted,
        ))
        .child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(
                    el().key("spaces-invite-link")
                        .grow(1.0)
                        .min_w(0.0)
                        .px(8.0)
                        .py(6.0)
                        .radius(6.0)
                        .fill(k.c.layer_03)
                        .help(inv.link.clone())
                        .child(
                            text(inv.link.clone())
                                .font(t12(k).mono())
                                .color(k.c.text_base)
                                .max_lines(1)
                                .selectable(true),
                        ),
                )
                .child(
                    Button::new(if s.spaces.copied { "Copied" } else { "Copy" })
                        .variant(ButtonVariant::Outline)
                        .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                            tx.copy(link.clone());
                            s.spaces.copied = true;
                        })
                        .build(k),
                ),
        );
    if !s.spaces.reach.is_empty() {
        c = c.child(u::line(s.spaces.reach.clone(), t12(k), u::warning(k)));
    }
    c.child(u::muted(k, "Waiting for them to join…"))
}

fn fingerprints(f: &Frame, inv: &InviteStatus, peer: &str) -> El {
    let k = f.k;
    let pair = |who: &str, fp: &str| {
        row()
            .w_full()
            .gap(12.0)
            .child(text(who.to_string()).font(t12(k)).color(k.c.text_muted).w(80.0).max_lines(1))
            .child(
                text(plain_line(fp))
                    .font(t12(k).mono())
                    .color(k.c.text_base)
                    .selectable(true),
            )
    };
    col()
        .key("spaces-fingerprints")
        .w_full()
        .gap(4.0)
        .px(12.0)
        .py(8.0)
        .radius(6.0)
        .fill(k.c.layer_03)
        .child(pair(peer, &inv.peer_fingerprint))
        .child(pair("You", &inv.my_fingerprint))
        .child(
            u::muted(
                k,
                format!("Give {peer} your fingerprint, and check that theirs matches what's shown here."),
            )
            .pt(4.0),
        )
}

/// The inviter: the words the invitee gives them. The invite goes on only
/// when they match; two misses end it.
fn type_words(s: &State, f: &Frame, inv: &InviteStatus, peer: &str) -> El {
    let k = f.k;
    let mut c = col()
        .w_full()
        .gap(6.0)
        .child(u::line(format!("Type the five words {peer} gives you:"), t13(k), k.c.text_base))
        .child(
            TextInput::new(&s.spaces.words)
                .placeholder("Type the words, then Enter")
                .label(format!("The words {peer} gives you"))
                .width(None)
                .text_style(t13(k).mono())
                .on_edit(|s: &mut State| &mut s.spaces.words)
                .on_submit(|s: &mut State, _| {
                    let words = s.spaces.words.text.trim().to_string();
                    if words.is_empty() {
                        return;
                    }
                    s.spaces.words_error = None;
                    s.spawn(
                        move |w| w.boards_confirm_invite_words(&words),
                        |s, r| match r {
                            Ok(st) => {
                                s.spaces.words.clear();
                                s.spaces.invite = Some(st);
                            }
                            Err(e) => s.spaces.words_error = Some(logic::error_text(&e)),
                        },
                    );
                })
                .build(k)
                .key("spaces-invite-words")
                .w_full(),
        );
    if inv.misses > 0 {
        c = c.child(u::line(
            "Those words didn't match. One more miss ends the invite.",
            t12(k),
            u::warning(k),
        ));
    }
    if let Some(e) = &s.spaces.words_error {
        c = c.child(u::error_line(k, e.clone()));
    }
    c
}

fn read_words(f: &Frame, inv: &InviteStatus, peer: &str) -> El {
    let k = f.k;
    col()
        .w_full()
        .gap(6.0)
        .child(u::line(format!("Give these words to {peer}:"), t13(k), k.c.text_base))
        .child(
            el().w_full().px(12.0).py(8.0).radius(6.0).fill(k.c.layer_03).child(
                text(inv.words.join(" "))
                    .font(rune::tok::TextStyle::new(15.0, 400.0).mono())
                    .color(k.c.text_base)
                    .wrap()
                    .selectable(true),
            ),
        )
}

// ---- the space's agents ------------------------------------------------------

fn set_agents_on(s: &mut State, board_id: String, on: bool, in_settings: bool) {
    s.spawn(
        move |w| {
            w.agents_set(AgentsChange {
                board_id,
                on: Some(on),
                ..AgentsChange::default()
            })
        },
        move |s, r| match r {
            Ok(v) => {
                if let Some(st) = s.spaces.settings.as_mut().filter(|st| st.board_id == v.board_id) {
                    st.agents = Some(v.clone());
                    st.agents_error = None;
                }
                s.spaces.agents.insert(v.board_id.clone(), v);
                s.spaces.agents_error = None;
            }
            Err(e) => {
                let e = logic::error_text(&e);
                match s.spaces.settings.as_mut().filter(|_| in_settings) {
                    Some(st) => st.agents_error = Some(e),
                    None => s.spaces.agents_error = Some(e),
                }
            }
        },
    );
}

/// The space's own line while its agents switch is off (card #573).
pub fn agents_off(s: &State, f: &Frame, board_id: &str) -> Option<El> {
    let k = f.k;
    let a = s.spaces.agents.get(board_id)?;
    if a.on {
        return None;
    }
    let id = board_id.to_string();
    let mut r = row()
        .key("spaces-agents-off")
        .role("status")
        .radius(10.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(12.0)
        .center_y()
        .child(icon(&icons::BOT_OFF, 16.0).color(k.c.text_muted))
        .child(
            text("Your agents are off in this space: Irori won't start one for the cards you make here.")
                .font(u::t125(k))
                .color(k.c.text_muted)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        );
    if let Some(e) = &s.spaces.agents_error {
        r = r.child(text(e.clone()).font(u::t125(k)).color(u::danger(k)));
    }
    Some(
        r.child(
            Button::new("Turn on")
                .variant(ButtonVariant::Outline)
                .on_press(move |s: &mut State| set_agents_on(s, id.clone(), true, false))
                .build(k),
        ),
    )
}

// ---- Space settings ----------------------------------------------------------

/// Opens Space settings on a space, reading its settings and agents
/// switch.
pub fn open_settings(s: &mut State, board_id: &str) {
    s.spaces.settings = Some(Settings {
        board_id: board_id.to_string(),
        ..Settings::default()
    });
    let id = board_id.to_string();
    s.spawn(
        move |w| (w.boards_settings(&id), w.agents_settings(&id)),
        |s, (r, a)| {
            let Some(st) = s.spaces.settings.as_mut() else { return };
            if st.board_id != a.board_id {
                return;
            }
            match r {
                Ok(v) => st.settings = Some(v),
                Err(e) => st.error = Some(logic::error_text(&e)),
            }
            st.agents = Some(a);
        },
    );
    super::reload_reach(s);
    super::reload_invite_now(s);
}

fn save(s: &mut State, work: impl FnOnce(&Window) -> Result<BoardSettings, String> + Send + 'static) {
    if let Some(st) = s.spaces.settings.as_mut() {
        st.error = None;
    }
    s.spawn(work, |s, r| {
        let Some(st) = s.spaces.settings.as_mut() else { return };
        match r {
            Ok(v) => st.settings = Some(v),
            Err(e) => st.error = Some(logic::error_text(&e)),
        }
    });
}

fn close_settings(s: &mut State) {
    s.spaces.settings = None;
    super::reload_spaces(s);
}

/// The header's cog (card #312): the space's settings in a dialog.
pub fn settings_dialog(s: &State, f: &mut Frame, b: &BoardSummary) {
    let Some(st) = s.spaces.settings.as_ref().filter(|st| st.board_id == b.board_id) else {
        return;
    };
    let k = f.k;
    let name = plain_line(&b.name);
    let name = if name.is_empty() { "(unnamed)".to_string() } else { name };
    // The reviewer pickers go above the dialog: an overlay finds its
    // anchor only in what was laid out before it.
    let n = f.layer.len();
    let mut body = col().key("spaces-settings-body").w_full().gap(14.0).px(16.0).pb(16.0);
    // The kit Dialog's description doesn't wrap in a fitted panel: the
    // line goes in the body, where it wraps.
    body = body.child(
        u::line(
            format!("{name}: who is on it, what reaches your agents and what they may do here."),
            t13(k),
            k.c.text_muted,
        )
        .key("spaces-settings-desc"),
    );
    body = body.child(board_invite(s, f, b, st));
    if let Some(panel) = settings_panel(f, st) {
        body = body.child(panel);
    } else if let Some(e) = &st.error {
        body = body.child(u::error_line(k, e.clone()));
    }
    let over: Vec<El> = f.layer.drain(n..).collect();
    let h = f.cx.layout_size("main").map_or(720.0, |s| s[1]);
    Dialog::new("spaces-settings", "Space settings")
        .width(512.0)
        .fit()
        .open(true)
        .on_close(close_settings)
        .child(
            col()
                .key("spaces-settings-scroll")
                .w_full()
                .max_h((h * 0.85 - 90.0).max(240.0))
                .scroll(ScrollSpec { stick_bottom: false })
                .child(body),
        )
        .build(k, f.layer);
    f.layer.extend(over);
    // A member step that asks first sits over the dialog.
    if let (Some(ask), Some(set)) = (&st.asking, &st.settings) {
        let board = set.board_id.clone();
        let cancel = |s: &mut State| {
            if let Some(st) = s.spaces.settings.as_mut() {
                st.asking = None;
            }
        };
        match ask.clone() {
            MemberAsk::StepDown(person) => u::confirm(
                f,
                "spaces-stepdown",
                "Step down to member",
                "Step down to a member? You can no longer invite, remove people or change roles here; another owner would have to make you an owner again.",
                false,
                move |s: &mut State| {
                    let (b, p) = (board.clone(), person.clone());
                    member_step(s, p.clone(), move |w| w.boards_set_role_settings(&b, &p, "member"));
                },
                cancel,
            ),
            MemberAsk::Remove { person, name } => {
                let who = if name.is_empty() { "They keep".to_string() } else { format!("{name} keeps") };
                u::confirm(
                    f,
                    "spaces-remove",
                    &format!("Remove {}", if name.is_empty() { "them" } else { name.as_str() }),
                    &format!(
                        "{who} everything already on their devices. Removing them stops them syncing and writing anything this space accepts."
                    ),
                    true,
                    move |s: &mut State| {
                        let (b, p) = (board.clone(), person.clone());
                        member_step(s, p.clone(), move |w| w.boards_remove_member_settings(&b, &p));
                    },
                    cancel,
                )
            }
        }
    }
}

/// A confirmed member step: its error shows under that member's row.
fn member_step(
    s: &mut State,
    person: String,
    work: impl FnOnce(&Window) -> Result<BoardSettings, String> + Send + 'static,
) {
    if let Some(st) = s.spaces.settings.as_mut() {
        st.asking = None;
        st.member_error = None;
    }
    s.spawn(work, move |s, r| {
        if let Some(st) = s.spaces.settings.as_mut() {
            match r {
                Ok(v) => st.settings = Some(v),
                Err(e) => st.member_error = Some((person, logic::error_text(&e))),
            }
        }
        super::reload_spaces(s);
    });
}

/// How to get more people on the space (card #371): Invite someone (an
/// owner, once ready) with how an invite works, else why there is no
/// Invite. An open invite stands in for the button until it ends.
fn board_invite(s: &State, f: &Frame, b: &BoardSummary, st: &Settings) -> El {
    let k = f.k;
    let mut c = col().key("spaces-board-invite").w_full().gap(8.0).label("Invite people");
    if let Some(inv) = s.spaces.invite.as_ref().filter(|i| !i.state.is_empty()) {
        return c.child(invite_panel(s, f, inv));
    }
    if !b.owner {
        c = c.child(u::muted(k, "Only an owner can invite people to this space: ask one of them."));
    } else if !b.ready {
        c = c.child(u::muted(
            k,
            "You can invite people once its cards have arrived from your other device.",
        ));
    } else {
        let id = b.board_id.clone();
        c = c
            .child(
                Button::new("Invite someone")
                    .variant(ButtonVariant::Submit)
                    .on_press(move |s: &mut State| {
                        let id = id.clone();
                        if let Some(st) = s.spaces.settings.as_mut() {
                            st.invite_error = None;
                        }
                        s.spawn(
                            move |w| w.boards_invite(&id),
                            |s, r| match r {
                                Ok(v) => {
                                    s.spaces.copied = false;
                                    s.spaces.invite = Some(v);
                                }
                                Err(e) => {
                                    if let Some(st) = s.spaces.settings.as_mut() {
                                        st.invite_error = Some(logic::invite_error(&e));
                                    }
                                }
                            },
                        );
                    })
                    .build(k)
                    .key("spaces-invite-button")
                    .self_start(),
            )
            .child(invite_how(f, &s.spaces.reach));
    }
    if let Some(e) = &st.invite_error {
        c = c.child(u::error_line(k, e.clone()));
    }
    c
}

fn section_title(f: &Frame, t: &str) -> El {
    text(t.to_string())
        .font(rune::tok::TextStyle::new(12.0, 600.0))
        .color(f.k.c.text_base)
}

/// A radio row of presets (buttons, the picked one filled).
fn presets(
    f: &Frame,
    key: &str,
    label: &str,
    list: &'static [logic::Preset],
    current: &str,
    on: impl Fn(&mut State, &'static str) + Clone + 'static,
) -> El {
    let k = f.k;
    let mut r = row().key(key.to_string()).gap(4.0).flex_wrap().role("radiogroup").label(label.to_string());
    for p in list {
        let on = on.clone();
        let v = p.value;
        r = r.child(
            Button::new(p.label)
                .variant(if current == p.value { ButtonVariant::Neutral } else { ButtonVariant::Ghost })
                .on_press(move |s: &mut State| on(s, v))
                .build(k)
                .role("radio")
                .checked(current == p.value),
        );
    }
    r
}

/// The space's settings (§8): the agents switch, its members, what
/// reaches your agents (§7.2), what your agents may do (§7.3), your
/// threads, and reviews.
fn settings_panel(f: &mut Frame, st: &Settings) -> Option<El> {
    let k = f.k;
    let set = st.settings.as_ref()?;
    let board = set.board_id.clone();
    let mut c = col().key("spaces-settings-panel").w_full().gap(14.0);

    // SpaceAgentSwitch.
    if let Some(a) = &st.agents {
        let on = a.on;
        let id = board.clone();
        let mut left = col()
            .grow(1.0)
            .min_w(0.0)
            .child(u::line("Start an agent for each card you make", t13(k), k.c.text_base))
            .child(u::muted(
                k,
                if on {
                    "On: Irori starts your agent for each card you make here."
                } else {
                    "Off: Irori doesn't start agents for the cards you make here."
                },
            ));
        if let Some(e) = &st.agents_error {
            left = left.child(u::error_line(k, e.clone()));
        }
        c = c.child(
            row()
                .key("spaces-agents-switch")
                .w_full()
                .gap(12.0)
                .items_start()
                .child(left)
                .child(
                    Switch::new(on)
                        .label("Start an agent for each card you make")
                        .hide_label()
                        .on_toggle(move |s: &mut State| set_agents_on(s, id.clone(), !on, true))
                        .build(k),
                ),
        );
    } else if let Some(e) = &st.agents_error {
        c = c.child(u::error_line(k, e.clone()));
    }

    // Members.
    let others: Vec<_> = set.members.iter().filter(|m| !m.me).collect();
    let am_owner = set.members.iter().any(|m| m.me && m.owner);
    let owners = set.members.iter().filter(|m| m.owner).count();
    let mut members = col().key("spaces-members").w_full().gap(6.0).label("Members").child(section_title(f, "Members"));
    for m in &set.members {
        let name = plain_line(&m.name);
        let shown = if name.is_empty() { "someone".to_string() } else { name.clone() };
        let mut top = row()
            .w_full()
            .gap(6.0)
            .center_y()
            .child(text(shown).font(t12(k)).color(k.c.text_base));
        if m.me {
            top = top.child(text("(you)").font(t12(k)).color(k.c.text_muted));
        }
        if m.owner {
            top = top.child(u::chip(k, "owner"));
        }
        top = top.child(el().grow(1.0)).child(
            text(if m.devices == 1 { "1 device".to_string() } else { format!("{} devices", m.devices) })
                .font(t12(k))
                .color(k.c.text_muted),
        );
        let mut second = row().w_full().gap(6.0).center_y().child(
            text(plain_line(&m.fingerprint))
                .font(t12(k).mono())
                .color(k.c.text_muted)
                .help("Their person fingerprint"),
        );
        if am_owner {
            let mut acts = row().ml(0.0).gap(4.0).center_y();
            let p = m.person_id.clone();
            if m.me {
                if owners > 1 {
                    acts = acts.child(
                        Button::new("Step down")
                            .variant(ButtonVariant::Outline)
                            .on_press(move |s: &mut State| {
                                if let Some(st) = s.spaces.settings.as_mut() {
                                    st.asking = Some(MemberAsk::StepDown(p.clone()));
                                }
                            })
                            .build(k),
                    );
                }
            } else if !m.owner {
                let (b2, p2) = (board.clone(), p.clone());
                acts = acts.child(
                    Button::new("Make an owner")
                        .variant(ButtonVariant::Ghost)
                        .on_press(move |s: &mut State| {
                            let (b, p) = (b2.clone(), p2.clone());
                            save(s, move |w| w.boards_set_role_settings(&b, &p, "owner"));
                        })
                        .build(k),
                );
                let n2 = name.clone();
                acts = acts.child(
                    Button::new("Remove")
                        .variant(ButtonVariant::Danger)
                        .on_press(move |s: &mut State| {
                            if let Some(st) = s.spaces.settings.as_mut() {
                                st.asking = Some(MemberAsk::Remove {
                                    person: p.clone(),
                                    name: n2.clone(),
                                });
                            }
                        })
                        .build(k),
                );
            }
            second = second.child(el().grow(1.0)).child(acts);
        }
        let mut item = col().key(format!("spaces-member-{}", m.person_id)).w_full().gap(2.0).child(top).child(second);
        if let Some((p, e)) = &st.member_error {
            if *p == m.person_id {
                item = item.child(row().w_full().justify_end().child(text(e.clone()).font(t12(k)).color(u::danger(k)).wrap().max_w(320.0)));
            }
        }
        members = members.child(item);
    }
    for conflict in &set.conflicts {
        members = members.child(u::line(plain_line(conflict), t12(k), u::warning(k)));
    }
    c = c.child(members);

    // What reaches my agents.
    let b2 = board.clone();
    let people = set.people.clone();
    let mut receive = col()
        .key("spaces-receive")
        .w_full()
        .gap(6.0)
        .label("What reaches my agents")
        .child(section_title(f, "What reaches my agents"))
        .child(presets(f, "spaces-receive-presets", "Receive preset", &logic::RECEIVE_PRESETS, &set.receive, move |s, v| {
            let (b, p) = (b2.clone(), if v == "members" { people.clone() } else { vec![] });
            save(s, move |w| w.boards_set_receive(&b, v, p));
        }))
        .child(u::muted(k, logic::hint(&logic::RECEIVE_PRESETS, &set.receive)));
    if set.receive == "members" {
        if others.is_empty() {
            receive = receive.child(u::muted(k, "Nobody else is in this space yet."));
        } else {
            let mut list = col().gap(4.0).label("People who wake your agents");
            for m in &others {
                let on = set.people.contains(&m.person_id);
                let next: Vec<String> = if on {
                    set.people.iter().filter(|p| **p != m.person_id).cloned().collect()
                } else {
                    set.people.iter().cloned().chain([m.person_id.clone()]).collect()
                };
                let b3 = board.clone();
                let name = plain_line(&m.name);
                list = list.child(
                    Checkbox::new(on)
                        .label(if name.is_empty() { "someone".to_string() } else { name })
                        .on_toggle(move |s: &mut State| {
                            let (b, p) = (b3.clone(), next.clone());
                            save(s, move |w| w.boards_set_receive(&b, "members", p));
                        })
                        .build(k),
                );
            }
            receive = receive.child(list);
        }
    }
    c = c.child(receive);

    // What my agents may do.
    let b4 = board.clone();
    c = c.child(
        col()
            .key("spaces-autonomy")
            .w_full()
            .gap(6.0)
            .label("What my agents may do")
            .child(section_title(f, "What my agents may do"))
            .child(presets(f, "spaces-autonomy-presets", "Autonomy preset", &logic::AUTONOMY_PRESETS, &set.autonomy, move |s, v| {
                let b = b4.clone();
                save(s, move |w| w.boards_set_autonomy(&b, v));
            }))
            .child(u::muted(k, logic::hint(&logic::AUTONOMY_PRESETS, &set.autonomy))),
    );

    // My threads.
    let b5 = board.clone();
    c = c.child(
        col()
            .key("spaces-threads")
            .w_full()
            .gap(6.0)
            .label("My threads")
            .child(section_title(f, "My threads"))
            .child(presets(f, "spaces-threads-modes", "Where my threads live", &logic::THREADS_MODES, &set.threads, move |s, v| {
                let b = b5.clone();
                save(s, move |w| w.boards_set_threads(&b, v));
            }))
            .child(u::muted(k, logic::hint(&logic::THREADS_MODES, &set.threads))),
    );

    c = c.child(reviewers(f, st, set));
    if let Some(e) = &st.error {
        c = c.child(u::error_line(k, e.clone()));
    }
    Some(c)
}

fn agent_label(a: &irori_window::boards::ReviewerChoice) -> String {
    let n = plain_line(&a.name);
    if a.repo.is_empty() { n } else { format!("{n} ({})", logic::short_repo(&a.repo)) }
}

/// The space's peer-review setting (design §10): any free agent reviews;
/// optionally always one agent, for the whole space or one repo.
fn reviewers(f: &mut Frame, st: &Settings, set: &BoardSettings) -> El {
    let k = f.k;
    let mut c = col()
        .key("spaces-reviews")
        .w_full()
        .gap(6.0)
        .label("Reviews")
        .child(section_title(f, "Reviews"))
        .child(u::muted(
            k,
            "Any free agent of yours reviews other people's PRs in this space, under your GitHub account. Nothing to set up.",
        ))
        .child(u::muted(k, "Optional: always use one agent, for the whole space or one repo."));
    if !set.reviewers.is_empty() {
        let mut list = col().w_full().gap(2.0).label("Pinned reviewers");
        for r in &set.reviewers {
            let name = plain_line(&r.name);
            let (b, repo) = (set.board_id.clone(), r.repo.clone());
            list = list.child(
                row()
                    .w_full()
                    .gap(6.0)
                    .center_y()
                    .child(text(if name.is_empty() { r.agent.clone() } else { name }).font(t12(k)).color(k.c.text_base))
                    .child(
                        text(if r.repo.is_empty() {
                            "for the whole space".to_string()
                        } else {
                            format!("for {}", logic::short_repo(&r.repo))
                        })
                        .font(t12(k))
                        .color(k.c.text_muted),
                    )
                    .child(el().grow(1.0))
                    .child(
                        Button::new("Remove")
                            .variant(ButtonVariant::Ghost)
                            .on_press(move |s: &mut State| {
                                let (b, repo) = (b.clone(), repo.clone());
                                save(s, move |w| w.boards_set_reviewer(&b, "", &repo));
                            })
                            .build(k),
                    ),
            );
        }
        c = c.child(list);
    }
    if set.agents.is_empty() {
        return c.child(u::muted(k, "No agent of yours is running to pin."));
    }
    let mut agent_opts = vec!["Pick an agent…".to_string()];
    agent_opts.extend(set.agents.iter().map(agent_label));
    let cur_agent = set
        .agents
        .iter()
        .find(|a| a.agent == st.agent)
        .map(agent_label)
        .unwrap_or_else(|| agent_opts[0].clone());
    let mut repos: Vec<String> = Vec::new();
    for a in &set.agents {
        if !a.repo.is_empty() && !repos.contains(&a.repo) {
            repos.push(a.repo.clone());
        }
    }
    let mut repo_opts = vec!["the whole space".to_string()];
    repo_opts.extend(repos.iter().map(|r| logic::short_repo(r)));
    let cur_repo = repos
        .iter()
        .position(|r| *r == st.repo)
        .map_or_else(|| repo_opts[0].clone(), |i| repo_opts[i + 1].clone());
    let agents = set.agents.clone();
    let agent_sel = Select::new("spaces-reviewer-agent", &agent_opts)
        .label("Agent to always use")
        .appearance(SelectAppearance::Field)
        .width(200.0)
        .current(Some(&cur_agent))
        .open(st.agent_open)
        .on_toggle(|s: &mut State| {
            if let Some(st) = s.spaces.settings.as_mut() {
                st.agent_open = !st.agent_open;
                st.repo_open = false;
            }
        })
        .on_close(|s: &mut State| {
            if let Some(st) = s.spaces.settings.as_mut() {
                st.agent_open = false;
            }
        })
        .on_choose(move |s: &mut State, v: &str, _: &mut Tx| {
            if let Some(st) = s.spaces.settings.as_mut() {
                st.agent = agents.iter().find(|a| agent_label(a) == v).map(|a| a.agent.clone()).unwrap_or_default();
                st.agent_open = false;
            }
        })
        .build(k, f.layer);
    let repo_sel = Select::new("spaces-reviewer-repo", &repo_opts)
        .label("For which repo")
        .appearance(SelectAppearance::Field)
        .width(160.0)
        .current(Some(&cur_repo))
        .open(st.repo_open)
        .on_toggle(|s: &mut State| {
            if let Some(st) = s.spaces.settings.as_mut() {
                st.repo_open = !st.repo_open;
                st.agent_open = false;
            }
        })
        .on_close(|s: &mut State| {
            if let Some(st) = s.spaces.settings.as_mut() {
                st.repo_open = false;
            }
        })
        .on_choose(move |s: &mut State, v: &str, _: &mut Tx| {
            if let Some(st) = s.spaces.settings.as_mut() {
                st.repo = repos.iter().find(|r| logic::short_repo(r) == v).cloned().unwrap_or_default();
                st.repo_open = false;
            }
        })
        .build(k, f.layer);
    let (b, agent, repo) = (set.board_id.clone(), st.agent.clone(), st.repo.clone());
    c.child(
        row()
            .key("spaces-reviewer-pick")
            .w_full()
            .gap(6.0)
            .flex_wrap()
            .center_y()
            .child(agent_sel)
            .child(repo_sel)
            .child(
                Button::new("Always use")
                    .variant(ButtonVariant::Outline)
                    .disabled(agent.is_empty())
                    .on_press(move |s: &mut State| {
                        let (b, a, r) = (b.clone(), agent.clone(), repo.clone());
                        if let Some(st) = s.spaces.settings.as_mut() {
                            st.agent.clear();
                        }
                        save(s, move |w| w.boards_set_reviewer(&b, &a, &r));
                    })
                    .build(k),
            ),
    )
}
