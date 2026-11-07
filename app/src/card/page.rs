//! The card page's frame (card-detail.tsx CardPage): Back, the working
//! or queue line, the header with its buttons (Pin, Not yet, Pause,
//! Delete, Share, Move, Transfer, Withdraw), the ask's or close bar, and
//! the two columns: objective, blockers, terminal and thread on the left,
//! progress, people, reviews, PRs, links and files on the right. Narrow
//! (below 900 points) it is one column with a short progress strip.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use irori_window::cards::{CardDetail, CardSummary};
use rune::{El, Key, KeyEvent, TextEdit, Tx, col, el, row, text, text_input};
use rune_kit::{Button, ButtonVariant, ControlSize, Placement, Popover};

use super::{bar, err_line, labelled, note, panel, section, spacer, ts};
use crate::logic::agent_status::{Who, agent_status, ms, now_ms};
use crate::logic::cards::{self as lc, Work, state};
use crate::logic::later;
use crate::logic::names::display_name;
use crate::logic::plain::{plain_line, short_repo};
use crate::logic::spawn_line::{SpawnState, SpawnTone, spawn_line, start_past_limit as past_limit_tip, usage_resets};
use crate::text::{self as tt, RichOpts};
use crate::ui::{Dialog, Frame, State, icons};
use crate::widgets::{bits, confirm, loading};

/// The page is two columns from this wide.
const WIDE: f32 = 900.0;

/// A card with its holders' status as the live roster has it now (card
/// #317), so the page agrees with Home.
pub(crate) fn live_card(s: &State, c: &CardSummary) -> CardSummary {
    let live: HashMap<String, String> =
        s.data.agents.iter().map(|a| (a.card.key.clone(), a.card.status.clone())).collect();
    match lc::with_live(&c.working, &live) {
        Some(w) => CardSummary {
            working: w,
            ..c.clone()
        },
        None => c.clone(),
    }
}

fn btn(label: &str) -> Button {
    Button::new(label.to_string())
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
}

pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let id = s.nav.card.clone().unwrap_or_default();
    let back = row().key("card/back-row").w_full().child(
        btn("Cards")
            .variant(ButtonVariant::GhostMuted)
            .icon(&icons::ARROW_LEFT)
            .help("Back to the cards (Esc)")
            .on_press(|s: &mut State| s.back())
            .build(k),
    );
    let wide = f.cx.layout_size("card/page").is_none_or(|[w, _]| w >= WIDE + 40.0);
    let mut body = col().key("card/body").w_full().min_w(0.0).gap(12.0).child(back);

    let detail = s.card.detail.as_ref().filter(|d| d.summary.id == id);
    match detail {
        None => {
            let summary = s.data.card(&id).map(|c| live_card(s, c));
            if let Some(c) = &summary {
                if let Some(r) = working_row(s, f, c) {
                    body = body.child(r);
                }
                let spin = lc::busy(&Work::from(c));
                body = body.child(header(s, f, c, spin, Vec::new()));
            } else if s.card.error.is_none() {
                body = body.child(header_shape(f));
            }
            body = body.child(match &s.card.error {
                Some(e) => panel(f, "card/error")
                    .gap(8.0)
                    .padding(20.0)
                    .child(text("Could not open the card").font(k.ty.ui_medium).color(k.c.text_base))
                    .child(note(f, plain_line(e)))
                    .child(
                        row().child(
                            Button::new("Back to the cards")
                                .variant(ButtonVariant::Outline)
                                .control_size(ControlSize::Small)
                                .on_press(|s: &mut State| s.back())
                                .build(k),
                        ),
                    ),
                None => body_shape(f, wide),
            });
        }
        Some(d) => body = page_body(s, f, d, body, wide),
    }

    let scroll_to = s.card.scroll_to.clone();
    let mut page = col()
        .key("card/page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(
            col()
                .key("card/scroll")
                .w_full()
                .h_full()
                .min_h(0.0)
                .padding(20.0)
                .child(body)
                .scroll(rune::ScrollSpec { stick_bottom: false }),
        )
        .on_key(|s: &mut State, ev: &KeyEvent, _tx: &mut Tx| {
            if ev.key == Key::Escape && s.dialog.is_none() && s.widgets.confirm.is_none() {
                s.back();
                return true;
            }
            false
        });
    if let Some(key) = scroll_to {
        // Once the page has laid out the entry, bring it into view.
        page = page.on_layout_tx(move |s: &mut State, _r, tx: &mut Tx| {
            if s.card.scroll_to.as_deref() == Some(key.as_str()) && s.card.detail.is_some() {
                tx.scroll_into_view(key.clone());
                s.card.scroll_to = None;
            }
        });
    }
    super::share::dialogs(s, f);
    page
}

fn page_body(s: &State, f: &mut Frame, d: &CardDetail, mut body: El, wide: bool) -> El {
    let now_ = live_card(s, &d.summary);
    let c = &d.summary;
    let spinning = lc::busy(&Work::from(&now_));
    if let Some(r) = working_row(s, f, &now_) {
        body = body.child(r);
    }
    let mut extra = board_actions(s, f, d);
    if let Some(b) = not_yet_button(s, f, c) {
        extra.push(b);
    }
    if d.on_board.is_none() {
        extra.extend(pause_button(s, f, c));
        extra.push(delete_button(s, f, d));
    }
    body = body.child(header(s, f, c, spinning, extra));
    let open_ask = lc::open_ask(c.ask.as_ref());
    if open_ask {
        if let Some(a) = ask_status(s, f, d) {
            body = body.child(a);
        }
    } else if !wide {
        body = body.child(progress_strip(s, f, d, spinning));
    }
    let over = lc::over(&c.state);
    let ready_now = if c.ready {
        d.blockers.iter().find(|b| b.done_check && b.resolved.is_none())
    } else {
        None
    };
    if !over && !open_ask {
        body = body.child(close_bar(s, f, d, ready_now.is_some()));
    }
    if over {
        body = body.child(match &d.shared_to {
            Some(t) if !t.withdrawn => shared_to_bar(f, d),
            _ => reopen_bar(s, f, d),
        });
    }

    let main = super::blockers::main_panel(s, f, d, ready_now);
    let side = super::side::view(s, f, d, &now_);
    let thread = super::thread::card_thread(s, f, d, &now_);
    let lower = col()
        .key("card/lower")
        .w_full()
        .min_w(0.0)
        .gap(16.0)
        .child(crate::term::section(s, f, &c.id))
        .child(
            panel(f, "card/thread-panel").child(section(
                f,
                "card/thread-section",
                "Thread",
                true,
                vec![thread],
            )),
        );
    if wide {
        let mut left = col().key("card/left").grow(1.0).min_w(0.0).gap(16.0);
        if let Some(m) = main {
            left = left.child(m);
        }
        left = left.child(lower);
        let side_w = if f.cx.layout_size("card/page").is_some_and(|[w, _]| w >= 1640.0) { 400.0 } else { 360.0 };
        body.child(
            row()
                .key("card/columns")
                .w_full()
                .min_w(0.0)
                .gap(16.0)
                .items_start()
                .child(left)
                .child(col().key("card/aside").w(side_w).child(side)),
        )
    } else {
        let mut one = col().key("card/one").w_full().min_w(0.0).gap(16.0);
        if let Some(m) = main {
            one = one.child(m);
        }
        body.child(one.child(side).child(lower))
    }
}

// ---- the top lines ---------------------------------------------------------

/// The page's top line while an agent works on it (card #317): who, with
/// the spinner; between turns with a job running, how long. Else the queue
/// line.
fn working_row(s: &State, f: &mut Frame, c: &CardSummary) -> Option<El> {
    let k = f.k;
    let w = Work::from(c);
    let busy_now = lc::working_now(&w);
    let who = if busy_now.is_empty() { lc::running_job_now(&w) } else { busy_now.clone() };
    if who.is_empty() {
        return queue_row(s, f, c);
    }
    let names = lc::names_line(&who.iter().map(display_name).collect::<Vec<_>>());
    let doing = if !busy_now.is_empty() {
        "working on this card now".to_string()
    } else {
        let since = (who.len() == 1)
            .then(|| s.data.agents.iter().find(|a| a.card.key == who[0].key).and_then(|a| ms(&a.card.status_since)))
            .flatten();
        agent_status(&Who::status(lc::RUNNING_JOB), since, now_ms()).text
    };
    let verb = if who.len() > 1 { "are" } else { "is" };
    let line = format!("{names} {verb} {doing}");
    let spans = vec![rune::Span::new(0, names.len()).weight(600.0)];
    Some(
        row()
            .key("card/working-row")
            .w_full()
            .min_w(0.0)
            .gap(8.0)
            .px(12.0)
            .py(8.0)
            .center_y()
            .radius(8.0)
            .border(1.0, k.c.info.border)
            .fill(k.c.info.bg)
            .role("status")
            .child(bits::spinner(f, "card/working-spin"))
            .child(
                rune::rich(line, spans)
                    .font(ts(13.0, 400.0))
                    .color(k.c.text_base)
                    .max_lines(1)
                    .shrink()
                    .min_w(0.0),
            ),
    )
}

/// The top line while nobody is on the card and Irori is getting it an
/// agent (card #450): starting, its queue place, a usage limit with Start
/// an agent, or why one can't start.
fn queue_row(s: &State, f: &mut Frame, c: &CardSummary) -> Option<El> {
    let k = f.k;
    let st = SpawnState::from(c);
    let line = spawn_line(&st)?;
    let resets = c.usage_limit.filter(|t| !t.is_zero()).map(|t| usage_resets(&t)).unwrap_or_default();
    let mut words = {
        let mut ch = line.text.chars();
        match ch.next() {
            Some(x) => x.to_uppercase().collect::<String>() + ch.as_str(),
            None => String::new(),
        }
    };
    if !resets.is_empty() {
        words.push_str(&format!(" ({resets})"));
    }
    let warn = line.tone == SpawnTone::Warn;
    let mut r = row()
        .key("card/queue-row")
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .center_y()
        .flex_wrap()
        .radius(8.0)
        .border(1.0, if warn { k.c.warning.border } else { k.c.border_base })
        .fill(if warn { k.c.warning.bg } else { k.c.layer_02 })
        .role("status")
        .child(text(words).font(ts(13.0, 400.0)).color(k.c.text_base).shrink().min_w(0.0).wrap());
    if let Some(b) = start_past_limit(s, f, "card/past-limit", &c.id, &st, false) {
        r = r.child(b);
    }
    Some(r)
}

/// Start an agent on a card or thread a usage limit keeps waiting (cards
/// #450, #576): it tries now rather than wait for the reset. None when no
/// usage limit stops it.
pub fn start_past_limit(s: &State, f: &mut Frame, key: &str, id: &str, st: &SpawnState, small: bool) -> Option<El> {
    let tip = past_limit_tip(st);
    if tip.is_empty() {
        return None;
    }
    let asked = s.card.ui.start_line.get(id).cloned().unwrap_or_default();
    let idc = id.to_string();
    let mut r = row().key(key.to_string()).gap(8.0).center_y().child(
        Button::new("Start an agent")
            .variant(ButtonVariant::Outline)
            .control_size(if small { ControlSize::Mini } else { ControlSize::Small })
            .help(tip)
            .disabled(!asked.is_empty())
            .on_press(move |s: &mut State| start_agent(s, &idc, "", "Asked: an agent starts in a moment."))
            .build(f.k),
    );
    if !asked.is_empty() {
        r = r.child(text(asked).font(ts(13.0, 400.0)).color(f.k.c.text_muted));
    }
    Some(r)
}

/// Start an agent (`note` joins its brief); its line says how it went.
pub(crate) fn start_agent(s: &mut State, id: &str, note: &str, ok: &'static str) {
    s.card.ui.start_line.insert(id.to_string(), "Starting…".into());
    let (idc, note) = (id.to_string(), note.to_string());
    let key = id.to_string();
    s.spawn(
        move |w| w.start_agent(&idc, &note),
        move |s, r| {
            let line = match r {
                Ok(()) => ok.to_string(),
                Err(e) => e,
            };
            s.card.ui.start_line.insert(key, line);
        },
    );
}

// ---- the header --------------------------------------------------------------

/// The title block: short ref, state, scope and repo over the title, with
/// the spinner while an agent works on it, Pin and the page's buttons.
fn header(s: &State, f: &mut Frame, c: &CardSummary, spinning: bool, extra: Vec<El>) -> El {
    let k = f.k;
    let mut chips = row()
        .key("card/header-chips")
        .w_full()
        .min_w(0.0)
        .gap(6.0)
        .center_y()
        .flex_wrap();
    if let Some(r) = bits::short_ref(f, "card/ref", c) {
        chips = chips.child(r);
    }
    chips = chips.child(bits::state_pill(f, "card/pill", c));
    if spinning {
        chips = chips.child(bits::spinner(f, "card/spin"));
    }
    chips = chips.child(bits::scope_badge(f, "card/scope", c));
    if let Some(b) = bits::side_badge(f, "card/side", c) {
        chips = chips.child(b);
    }
    if let Some(b) = bits::owner_chip(s, f, "card/owner", c) {
        chips = chips.child(b);
    }
    if let Some(b) = bits::pr_chip(f, "card/pr", c) {
        chips = chips.child(b);
    }
    if !c.repo_key.is_empty() {
        chips = chips.child(
            text(short_repo(&c.repo_key))
                .font(k.ty.text_xsmall.mono())
                .color(k.c.text_muted)
                .help(c.repo_key.clone()),
        );
    }
    chips = chips.child(spacer()).child(pin_button(s, f, c));
    for e in extra {
        chips = chips.child(e);
    }
    let title = plain_line(&c.title);
    let mut h = col()
        .key("card/header")
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .child(chips)
        .child(
            text(if title.is_empty() { "(untitled)".into() } else { title })
                .font(ts(24.0, 600.0).tracking(-0.4).lh(30.0))
                .color(k.c.text_base)
                .w_full()
                .wrap()
                .selectable(true)
                .role("heading"),
        );
    if let Some(p) = &c.parent {
        h = h.child(ref_line(f, "card/parent", "Part of", &p.id, &lc::ref_text(&p.tag, &p.short), &p.title));
    }
    if let Some(w) = when_line(s, f, c) {
        h = h.child(w);
    }
    if let Some(t) = &c.from_thread {
        h = h.child(ref_line(f, "card/from-thread", "From the thread", &t.id, &lc::ref_text(&t.tag, &t.short), &t.title));
    }
    h
}

/// "Part of #12 Title": a line naming another card, which opens it.
fn ref_line(f: &Frame, key: &str, lead: &str, id: &str, r: &str, title: &str) -> El {
    let k = f.k;
    let id = id.to_string();
    row()
        .key(key.to_string())
        .w_full()
        .min_w(0.0)
        .gap(4.0)
        .center_y()
        .child(text(lead.to_string()).font(ts(14.0, 400.0)).color(k.c.text_muted))
        .child(card_link(f, &format!("{key}/link"), &id, r, title))
}

/// A card's ref and title as a link to its page.
pub(crate) fn card_link(f: &Frame, key: &str, id: &str, r: &str, title: &str) -> El {
    let k = f.k;
    let id = id.to_string();
    row()
        .key(key.to_string())
        .gap(5.0)
        .min_w(0.0)
        .shrink()
        .cursor(rune::Cursor::Pointer)
        .role("link")
        .on_press(move |s: &mut State| s.open_card(&id))
        .child(text(plain_line(r)).font(ts(13.0, 400.0).mono()).color(k.c.text_base))
        .child(text(plain_line(title)).font(ts(14.0, 400.0)).color(k.c.text_base).max_lines(1).shrink().min_w(0.0))
}

/// Under the title, as "Part of" is: when a held card starts (with Start
/// now), or the cards before and after it in its order (order.tsx
/// WhenLine).
fn when_line(s: &State, f: &mut Frame, c: &CardSummary) -> Option<El> {
    let k = f.k;
    if let Some(l) = &c.later {
        let id = c.id.clone();
        let busy = s.card.ui.start_now_busy;
        let mut col_ = col().key("card/when").w_full().gap(2.0).child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .flex_wrap()
                .child(text(plain_line(&later::when_later(l))).font(ts(14.0, 400.0)).color(k.c.text_muted).shrink().wrap())
                .child(
                    text("Start now")
                        .font(ts(14.0, 500.0))
                        .color(if busy { k.c.text_muted } else { k.c.text_base })
                        .role("button")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |s: &mut State| {
                            if s.card.ui.start_now_busy {
                                return;
                            }
                            s.card.ui.start_now_busy = true;
                            s.card.ui.start_now_error = None;
                            let id = id.clone();
                            s.spawn(
                                move |w| w.start_now(&id),
                                |s, r| {
                                    s.card.ui.start_now_busy = false;
                                    s.card.ui.start_now_error = r.err();
                                },
                            );
                        }),
                ),
        );
        if let Some(e) = &s.card.ui.start_now_error {
            col_ = col_.child(err_line(f, e.clone()));
        }
        return Some(col_);
    }
    let (before, after) = later::around(&c.id, c.order.as_ref());
    if before.is_none() && after.is_none() {
        return None;
    }
    let mut r = row().key("card/order").w_full().min_w(0.0).gap(4.0).center_y().flex_wrap();
    let muted = |t: &str| text(t.to_string()).font(ts(14.0, 400.0)).color(k.c.text_muted);
    if let Some(b) = before {
        r = r
            .child(muted("After"))
            .child(card_link(f, "card/order-before", &b.id, &lc::ref_text(&b.tag, &b.short), &b.title));
        if after.is_some() {
            r = r.child(muted(", then"));
        }
    }
    if let Some(a) = after {
        if before.is_none() {
            r = r.child(muted("Then"));
        }
        r = r.child(card_link(f, "card/order-after", &a.id, &lc::ref_text(&a.tag, &a.short), &a.title));
    }
    Some(r)
}

/// Pin or unpin: pinned cards show in their own group above the grid.
fn pin_button(s: &State, f: &Frame, c: &CardSummary) -> El {
    let pinned = c.pinned;
    let id = c.id.clone();
    btn(if pinned { "Unpin" } else { "Pin" })
        .icon(if pinned { &icons::PIN_OFF } else { &icons::PIN })
        .disabled(s.card.ui.pin_busy)
        .help(if pinned {
            "Unpin: back to the regular list"
        } else {
            "Pin: keep it in the Pinned group at the top"
        })
        .on_press(move |s: &mut State| {
            s.card.ui.pin_busy = true;
            let id = id.clone();
            s.spawn(
                move |w| w.pin(&id, !pinned),
                |s, r| {
                    s.card.ui.pin_busy = false;
                    if let Err(e) = r {
                        s.error("The pin didn't change", &e);
                    }
                },
            );
        })
        .build(f.k)
}

/// Pause an open card of the person's own board, or resume it (card
/// #207): pausing stops every holder until Resume.
fn pause_button(s: &State, f: &Frame, c: &CardSummary) -> Vec<El> {
    let paused = c.paused;
    if lc::over(&c.state) && !paused {
        return vec![];
    }
    let id = c.id.clone();
    let mut out = vec![
        btn(if paused { "Paused · Resume" } else { "Pause" })
            .variant(if paused { ButtonVariant::Danger } else { ButtonVariant::Ghost })
            .icon(if paused { &icons::PLAY } else { &icons::PAUSE })
            .disabled(s.card.ui.pause_busy)
            .help(if paused {
                "Resume: its holders' tools come back and they carry on"
            } else {
                "Pause: stop every holder now; they may only answer you on the card until you resume"
            })
            .on_press(move |s: &mut State| {
                s.card.ui.pause_busy = true;
                s.card.ui.pause_error = None;
                let id = id.clone();
                s.spawn(
                    move |w| w.pause(&id, !paused, ""),
                    |s, r| {
                        s.card.ui.pause_busy = false;
                        s.card.ui.pause_error = r.err();
                    },
                );
            })
            .build(f.k),
    ];
    if let Some(e) = &s.card.ui.pause_error {
        out.push(text(e.clone()).font(f.k.ty.text_xsmall).color(f.k.c.danger.fg).max_w(260.0).wrap());
    }
    out
}

/// Delete a card for good (card #57); its subtasks stay.
fn delete_button(s: &State, f: &Frame, d: &CardDetail) -> El {
    let kids = !d.subtask_list.is_empty();
    let c = &d.summary;
    let what = if c.short.is_empty() { "this card".to_string() } else { card_status::card_ref(c) };
    let id = c.id.clone();
    confirm::confirm_button(
        s,
        f,
        "card/delete",
        "Delete",
        None,
        false,
        confirm::Ask::new(
            format!(
                "Delete {what} for good? It disappears from the Board on all your devices and agents can no longer reach it. It can't be undone.{}",
                if kids { " Its subtasks stay, as cards of their own." } else { "" }
            ),
            "Delete it",
            move |w| w.delete_card(&id),
        )
        .rich()
        .danger()
        .done(|s| s.back()),
    )
}

use crate::logic::card_status;

/// Not yet: hold a card back until a card, a date or a version, or the
/// person's word (order.tsx NotYetButton).
fn not_yet_button(s: &State, f: &mut Frame, c: &CardSummary) -> Option<El> {
    if c.later.is_some() || lc::over(&c.state) || c.ask.is_some() {
        return None;
    }
    let k = f.k;
    let ui = &s.card.ui;
    let id = c.id.clone();
    let hold = move |s: &mut State| {
        if s.card.ui.not_yet_busy {
            return;
        }
        s.card.ui.not_yet_busy = true;
        s.card.ui.not_yet_error = None;
        let (id, until) = (id.clone(), s.card.ui.not_yet.text.trim().to_string());
        s.spawn(
            move |w| w.not_yet(&id, &until, ""),
            |s, r| {
                s.card.ui.not_yet_busy = false;
                match r {
                    Ok(()) => {
                        s.card.ui.not_yet.clear();
                        s.card.ui.not_yet_open = false;
                    }
                    Err(e) => s.card.ui.not_yet_error = Some(e),
                }
            },
        );
    };
    let hold2 = hold.clone();
    let mut body = col()
        .key("card/not-yet-body")
        .w(260.0)
        .gap(8.0)
        .child(text("Hold it back until").font(ts(13.0, 400.0)).color(k.c.text_base))
        .child(
            text_input(&ui.not_yet, k.ty.ui)
                .key("card/not-yet-until")
                .w_full()
                .px(8.0)
                .py(8.0)
                .radius(6.0)
                .border(1.0, k.c.border_base)
                .fill(k.c.layer_02)
                .color(k.c.text_base)
                .placeholder("#83, 1 Dec or v0.13.0", k.c.text_faint)
                .label("Until")
                .on_edit(|s: &mut State| &mut s.card.ui.not_yet)
                .on_submit(move |s: &mut State, _tx: &mut Tx| hold(s)),
        )
        .child(note(f, "Leave it empty to hold it until you say. No agent gets it before then."));
    if let Some(e) = &ui.not_yet_error {
        body = body.child(err_line(f, e.clone()));
    }
    body = body.child(
        row().w_full().child(spacer()).child(
            Button::new("Hold back")
                .variant(ButtonVariant::Submit)
                .control_size(ControlSize::Small)
                .disabled(ui.not_yet_busy)
                .on_press(hold2)
                .build(k),
        ),
    );
    Some(
        Popover::new("card/not-yet")
            .trigger(
                btn("Not yet")
                    .on_press(|s: &mut State| s.card.ui.not_yet_open = !s.card.ui.not_yet_open)
                    .build(k),
            )
            .open(ui.not_yet_open)
            .placement(Placement::BottomEnd)
            .padding(12.0)
            .child(body)
            .on_close(|s: &mut State| {
                s.card.ui.not_yet_open = false;
                s.card.ui.not_yet_error = None;
            })
            .build(k, f.layer),
    )
}

/// The header's board buttons (v0.5 §8): Share to a space on a private
/// card in play; Move, Transfer, Withdraw and a given card's Start on a
/// shared one.
fn board_actions(s: &State, f: &mut Frame, d: &CardDetail) -> Vec<El> {
    let c = &d.summary;
    let k = f.k;
    let mut out = Vec::new();
    let over = lc::over(&c.state);
    let open_ask = lc::open_ask(c.ask.as_ref());
    if let Some(ob) = &d.on_board {
        if over {
            return out;
        }
        let shared = crate::logic::owners::shared_board(&c.scope, &c.board, &s.data.spaces);
        if c.given_no {
            let id = c.id.clone();
            out.push(
                Button::new("Start an agent…")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut State| start_agent(s, &id, "", ""))
                    .build(k),
            );
            if let Some(l) = s.card.ui.start_line.get(&c.id).filter(|l| !l.is_empty() && l.as_str() != "Starting…") {
                out.push(text(l.clone()).font(ts(12.5, 400.0)).color(k.c.danger.fg).max_w(260.0).wrap());
            }
        }
        if let Some(sb) = shared {
            if ob.can_transfer {
                out.extend(super::share::transfer_button(s, f, d, sb));
            }
            let boards = crate::logic::spaces::boards_of(sb);
            if c.parent.is_none() && !open_ask && boards.len() > 1 {
                let current = if c.space_board.is_empty() || c.space_board == sb.board_id {
                    boards[0].board_id.clone()
                } else {
                    c.space_board.clone()
                };
                out.push(super::share::move_button(f, d, boards, current));
            }
        }
        if ob.can_withdraw && c.parent.is_none() {
            out.push(withdraw_button(s, f, d));
        }
        return out;
    }
    if over || d.shared_to.is_some() || open_ask || c.parent.is_some() {
        return out;
    }
    if s.card.personal.len() > 1 {
        let current = if c.space_board.is_empty() {
            crate::logic::spaces::PERSONAL.to_string()
        } else {
            c.space_board.clone()
        };
        out.push(super::share::move_button(f, d, s.card.personal.clone(), current));
    }
    if s.data.spaces.iter().any(|b| b.ready) {
        let id = c.id.clone();
        out.push(
            btn("Share to a space")
                .icon(&icons::SHARE)
                .help("Share to a space: put a scrubbed copy in one of your spaces")
                .on_press(move |s: &mut State| {
                    s.open_dialog(Dialog::Share { card: id.clone() });
                    super::share::opened(s);
                })
                .build(k),
        );
    }
    out
}

/// Withdraw a shared card off its board (§5.3): the work carries on in a
/// private card, whose page opens.
fn withdraw_button(s: &State, f: &Frame, d: &CardDetail) -> El {
    let family = d.summary.subtasks > 0;
    let visible = d.on_board.as_ref().map(|b| b.visible_to.iter().map(|v| plain_line(v)).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    let id = d.summary.id.clone();
    let got: Arc<Mutex<String>> = Arc::default();
    let put = got.clone();
    confirm::confirm_button(
        s,
        f,
        "card/withdraw",
        "Withdraw",
        None,
        false,
        confirm::Ask::new(
            format!(
                "Take this card{} out of the space? {visible}'s devices already have what was shared; nothing can be recalled. The work carries on in private{}.",
                if family { " and its subtasks" } else { "" },
                if family { " cards" } else { " a private card" }
            ),
            "Withdraw it",
            move |w| {
                let p = w.boards_withdraw(&id)?;
                *put.lock().unwrap() = p;
                Ok(())
            },
        )
        .rich()
        .done(move |s| {
            let p = got.lock().unwrap().clone();
            if p.is_empty() { s.back() } else { s.open_card(&p) }
        }),
    )
}

// ---- the bars under the header ---------------------------------------------

/// An open ask's line: where it stands, and Close with an optional note.
fn ask_status(s: &State, f: &mut Frame, d: &CardDetail) -> Option<El> {
    let a = d.summary.ask.as_ref()?;
    let id = d.summary.id.clone();
    let note_key = format!("{id}/close-ask");
    Some(
        bar(f, "card/ask-status")
            .flex_wrap()
            .child(el().grow(1.0).min_w(192.0).child(bits::ask_line(f, "card/ask-line", a)))
            .child(note_field(f, &s.card.ui.ask_note, "card/ask-note", "Why close it (optional)", "Why you close the ask", note_key, |s| {
                &mut s.card.ui.ask_note
            }))
            .child(confirm::confirm_button(
                s,
                f,
                "card/close-ask",
                "Close",
                None,
                false,
                confirm::Ask::new(
                    "Close this ask? Nothing comes of it, and the agents stop hearing about it.",
                    "Close it",
                    {
                        let note = s.card.ui.ask_note.text.clone();
                        move |w| w.close_ask(&id, &note).map(|_| ())
                    },
                )
                .done(move |s| {
                    s.card.ui.ask_note.clear();
                    let k = format!("{}/close-ask", s.card.ui.card);
                    crate::text::drafts::set_draft(s, &k, "", &[]);
                    s.back();
                }),
            )),
    )
}

/// A one-line note box kept as a draft under `draft`.
fn note_field(
    f: &Frame,
    edit: &TextEdit,
    key: &str,
    placeholder: &str,
    label: &str,
    draft: String,
    get: fn(&mut State) -> &mut TextEdit,
) -> El {
    let k = f.k;
    text_input(edit, k.ty.text_xsmall)
        .key(key.to_string())
        .w(224.0)
        .px(8.0)
        .py(8.0)
        .radius(6.0)
        .border(1.0, k.c.border_base)
        .fill(k.c.layer_02)
        .color(k.c.text_base)
        .placeholder(placeholder.to_string(), k.c.text_faint)
        .label(label.to_string())
        .on_edit(rune::edit_access(get))
        .on_input_change(move |s: &mut State, _tx: &mut Tx| {
            let t = get(s).text.clone();
            crate::text::drafts::set_draft(s, &draft, &t, &[]);
        })
}

/// Close a card in play by hand (card #57): Done or Cancel, with a note
/// the agents on it hear. While it is Ready for you only Cancel stays.
fn close_bar(s: &State, f: &mut Frame, d: &CardDetail, ready: bool) -> El {
    let id = d.summary.id.clone();
    let kids = !d.subtask_list.is_empty();
    let mut b = bar(f, "card/close-bar")
        .flex_wrap()
        .child(
            text("Close it by hand")
                .font(ts(13.0, 400.0))
                .color(f.k.c.text_muted)
                .grow(1.0)
                .min_w(160.0),
        )
        .child(note_field(f, &s.card.ui.close_note, "card/close-note", "Why (optional)", "Why you close the card", format!("{id}/close"), |s| {
            &mut s.card.ui.close_note
        }));
    let close = |label: &str, state: &'static str, question: String, confirm: &str, cancel: Option<&str>, key: &str| {
        let id = id.clone();
        let note = s.card.ui.close_note.text.clone();
        let failed = if state == state::CANCELED {
            "The card wasn't canceled"
        } else {
            "The card wasn't marked done"
        };
        let mut ask = confirm::Ask::new(question, confirm, |_| Ok(())).done(move |s| {
            // The card closes at once and saves in the background (card
            // #644); a refusal puts the note back and says why.
            let (id, note) = (id.clone(), note.clone());
            s.card.ui.close_note.clear();
            let dk = format!("{id}/close");
            crate::text::drafts::set_draft(s, &dk, "", &[]);
            s.spawn(
                move |w| w.close_card(&id, state, &note).map(|_| note),
                move |s, r| {
                    if let Err(e) = r {
                        s.error(failed, &e);
                    }
                },
            );
        });
        if let Some(c) = cancel {
            ask.cancel_label = c.to_string();
        }
        confirm::confirm_button(s, f, key, label, None, false, ask)
    };
    if !ready {
        b = b.child(close(
            "Done",
            state::COMPLETED,
            "Mark this card done? It moves to Closed, and whoever is on it hears.".into(),
            "Mark done",
            None,
            "card/close-done",
        ));
    }
    b.child(close(
        "Cancel card",
        state::CANCELED,
        if kids {
            "Cancel this card and its open subtasks? They move to Closed, and whoever is on them hears.".into()
        } else {
            "Cancel this card? It moves to Closed, and whoever is on it hears.".into()
        },
        "Cancel it",
        Some("Keep it"),
        "card/close-cancel",
    ))
}

/// A finished card's line: how it ended, and Reopen.
fn reopen_bar(s: &State, f: &mut Frame, d: &CardDetail) -> El {
    let c = &d.summary;
    let held = !c.working.is_empty();
    let follows = !d.subtask_list.is_empty() && d.subtask_list.iter().all(|k| lc::over(&k.state));
    let words = format!(
        "{}{}",
        lc::over_label(&c.state, c.ask.as_ref()),
        if follows {
            ". Its subtasks have all ended; reopen one to bring it back."
        } else if held {
            ". A comment reopens it for whoever held it."
        } else {
            ". Nobody holds it; reopening puts it back on the grid, open."
        }
    );
    let mut b = bar(f, "card/reopen-bar")
        .flex_wrap()
        .child(text(words).font(ts(13.0, 400.0)).color(f.k.c.text_muted).grow(1.0).min_w(192.0).wrap());
    if !follows {
        let id = c.id.clone();
        b = b.child(confirm::confirm_button(
            s,
            f,
            "card/reopen",
            "Reopen",
            None,
            false,
            confirm::Ask::new(
                "Reopen this card? It goes back on the grid with whoever held it, and they hear.",
                "Reopen it",
                move |w| w.reopen(&id).map(|_| ()),
            ),
        ));
    }
    b
}

/// A private card once shared to a board (v0.5 §5.2): where it went.
fn shared_to_bar(f: &mut Frame, d: &CardDetail) -> El {
    let Some(t) = &d.shared_to else { return el() };
    let k = f.k;
    let name = plain_line(&t.board_name);
    let r = plain_line(&t.ref_);
    let lead = format!("Shared to {} as ", if name.is_empty() { "a space" } else { &name });
    let words = format!("{lead}{r}. Work it there; this private card is closed.");
    let spans = vec![rune::Span::new(lead.len(), lead.len() + r.len()).mono().color(k.c.text_base)];
    let id = t.task_id.clone();
    bar(f, "card/shared-to")
        .flex_wrap()
        .child(rune::rich(words, spans).font(ts(13.0, 400.0)).color(k.c.text_muted).grow(1.0).min_w(192.0).wrap())
        .child(
            Button::new(format!("Open {r}"))
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut State| s.open_card(&id))
                .build(k),
        )
}

/// The narrow page's "where is this" glance: the bar (or who holds it),
/// the figures and the current step.
fn progress_strip(s: &State, f: &mut Frame, d: &CardDetail, spinning: bool) -> El {
    let c = &d.summary;
    let mut b = col()
        .key("card/progress-strip")
        .w_full()
        .gap(8.0)
        .px(16.0)
        .py(12.0)
        .radius(12.0)
        .border(1.0, f.k.c.border_base)
        .fill(f.k.c.layer_01);
    b = b.child(match bits::progress_bar(f, "card/strip-bar", c) {
        Some(p) if lc::percent(c.progress.done, c.progress.total).is_some() => p,
        _ => bits::holder_line(s, f, "card/strip-holder", c),
    });
    if let Some(l) = super::side::progress_line(f, "card/strip-line", d) {
        b = b.child(l);
    }
    if let Some(st) = d.steps.iter().find(|x| x.doing && !x.done) {
        b = b.child(super::side::step_row(
            f,
            "card/strip-step",
            super::side::StepView { text: &st.text, done: st.done, doing: st.doing },
            spinning,
            true,
            "",
        ));
    }
    b
}

// ---- loading shapes (card #598) -----------------------------------------------

fn header_shape(f: &Frame) -> El {
    col()
        .key("card/header-shape")
        .gap(8.0)
        .child(loading::block(f, "card/hs1", Some(110.0), 12.0, 3.0))
        .child(loading::block(f, "card/hs2", Some(420.0), 26.0, 4.0))
}

fn body_shape(f: &Frame, wide: bool) -> El {
    let k = f.k;
    let objective = panel(f, "card/shape-objective").child(section(
        f,
        "card/shape-obj",
        "Objective",
        true,
        vec![
            loading::block(f, "card/so1", None, 12.0, 3.0),
            loading::block(f, "card/so2", Some(380.0), 12.0, 3.0),
            loading::block(f, "card/so3", Some(240.0), 10.0, 3.0),
        ],
    ));
    let side = panel(f, "card/shape-side")
        .child(section(
            f,
            "card/shape-progress",
            "Progress",
            false,
            vec![
                loading::block(f, "card/sp1", None, 4.0, 2.0),
                loading::block(f, "card/sp2", Some(180.0), 10.0, 3.0),
                loading::block(f, "card/sp3", Some(220.0), 10.0, 3.0),
            ],
        ))
        .child(section(
            f,
            "card/shape-who",
            "Who's on it",
            true,
            vec![
                row()
                    .gap(8.0)
                    .center_y()
                    .child(loading::block(f, "card/sw1", Some(24.0), 24.0, 12.0))
                    .child(loading::block(f, "card/sw2", Some(140.0), 10.0, 3.0)),
            ],
        ));
    let thread = panel(f, "card/shape-thread").child(section(
        f,
        "card/shape-th",
        "Thread",
        true,
        vec![loading::message_rows_shape(f, "card/shape-rows", 4)],
    ));
    let close = bar(f, "card/shape-close")
        .child(text("Close it by hand when the work is done.").font(ts(13.0, 400.0)).color(k.c.text_muted).grow(1.0))
        .child(loading::block(f, "card/sc1", Some(224.0), 32.0, 6.0));
    let cols = if wide {
        row()
            .key("card/shape-cols")
            .w_full()
            .gap(16.0)
            .items_start()
            .child(col().grow(1.0).min_w(0.0).gap(16.0).child(objective).child(thread))
            .child(col().w(360.0).child(side))
    } else {
        col().key("card/shape-cols").w_full().gap(16.0).child(objective).child(side).child(thread)
    };
    col()
        .key("card/body-shape")
        .w_full()
        .gap(12.0)
        .role("progressbar")
        .label("Loading the card")
        .child(close)
        .child(cols)
}

/// A "Label: rich line" line (Note, Done when).
pub(crate) fn label_rich(s: &State, f: &mut Frame, key: &str, label: &str, src: &str) -> El {
    let o = RichOpts {
        font: Some(ts(12.5, 400.0)),
        color: Some(f.k.c.text_muted),
        ..RichOpts::default()
    };
    let line = tt::rich_line(s, f, key, src, &o);
    labelled(f, label, line)
}
