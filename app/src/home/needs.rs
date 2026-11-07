//! Needs you (the React app's cards/needs-you.tsx, cards #261, #449, #456,
//! #481, #488, #509, #520, #541, #573, #644): every done check, decision,
//! held message and wait no agent will end, newest first, each answerable
//! on its row. The card's own page still has the full forms (mentions,
//! files); a row has the everyday answers.
//!
//! [`needs_row`] builds one row and [`blocker_answer`] one item's answers,
//! for the other pages that list them (Notifications).

use std::sync::{Arc, Mutex};

use inbox::{Delivery, hold};
use irori_window::Window;
use irori_window::cards::{AgentRef, CardBlocker, CardRef, CardSummary, GoneWait, WaitPr};
use rune::{El, KeyEvent, Key, TextEdit, Tx, col, el, row, text, text_input};
use rune_kit::{Button, ButtonVariant, ControlSize};

use crate::inbox::words::{Names, Repos, allow_lifts, hold_text, started_line};
use super::pure::{answer_failed, answer_key, answer_said, by_recommendation, trust_from_reason, waits_on_you};
use crate::logic::agent_status::ms;
use crate::logic::card_status::card_ref;
use crate::logic::cards::{Tone, blocker};
use crate::logic::names::display_name;
use crate::logic::needs::{
    GIVEN_QUESTION, Need, NeedKind, SpawnNeed, WAITING_PERMISSION, gone_title, loop_line, merge_line, ref_of,
    spawn_need, stuck_line,
};
use crate::logic::plain::{plain_line, relative_time};
use crate::ui::{Dialog, Frame, State};
use crate::widgets::bits::badge;
use crate::widgets::confirm::{self, Ask};
use crate::widgets::done_rounds::earlier_round;
use crate::widgets::spawned_actions::{Action, spawned_actions};

/// One row action's state: in flight, its error, or what it did.
#[derive(Clone, Debug, Default)]
pub struct Act {
    pub busy: bool,
    pub error: Option<String>,
    pub done: String,
}

/// The kind chip each row leads with.
fn kind_chip(kind: &str) -> (&'static str, Tone) {
    match kind {
        "check" => ("Done check", Tone::Success),
        "decision" => ("Decision", Tone::Destructive),
        "merge" => ("Merge", Tone::Warning),
        "gone" => ("Gone", Tone::Warning),
        "noAgent" => ("No agent", Tone::Warning),
        "trust" => ("Trust", Tone::Warning),
        "permission" => ("Permission", Tone::Warning),
        "crashed" => ("Crashed", Tone::Destructive),
        "given" => ("Given to you", Tone::Info),
        "held" => ("Held", Tone::Warning),
        "loop" | "stuck" => ("Stuck", Tone::Warning),
        _ => ("Message", Tone::Info),
    }
}

fn small(f: &Frame) -> rune::tok::TextStyle {
    let _ = f;
    rune::tok::TextStyle::new(12.5, 400.0)
}

fn btn(label: impl Into<String>, v: ButtonVariant) -> Button {
    Button::new(label).variant(v).control_size(ControlSize::Small)
}

/// Runs one row action off the UI thread; `said` makes its outcome's
/// words from the answer (on the UI thread).
pub fn run_act<T: Send + 'static>(
    s: &mut State,
    key: String,
    work: impl FnOnce(&Window) -> Result<T, String> + Send + 'static,
    said: impl FnOnce(&mut State, T) -> String + Send + 'static,
) {
    let a = s.home.acts.entry(key.clone()).or_default();
    if a.busy {
        return;
    }
    a.busy = true;
    a.error = None;
    s.spawn(work, move |s, r| {
        let done = match r {
            Ok(v) => Ok(said(s, v)),
            Err(e) => Err(e),
        };
        let a = s.home.acts.entry(key).or_default();
        a.busy = false;
        match done {
            Ok(d) => a.done = d,
            Err(e) => a.error = Some(e),
        }
    });
}

fn act<'a>(s: &'a State, key: &str) -> Option<&'a Act> {
    s.home.acts.get(key)
}

/// A row's outcome while the list catches up, or its error.
pub fn said(f: &Frame, key: &str, error: Option<&str>, done: &str) -> Option<El> {
    let k = f.k;
    if let Some(e) = error {
        return Some(
            text(e.to_string())
                .key(key.to_string())
                .role("alert")
                .font(small(f))
                .color(k.c.danger.fg)
                .w_full()
                .wrap(),
        );
    }
    (!done.is_empty()).then(|| {
        text(done.to_string())
            .key(key.to_string())
            .role("status")
            .font(small(f))
            .color(k.c.text_muted)
            .w_full()
            .wrap()
    })
}

/// A body line (text-sm), muted or not.
fn body(f: &Frame, t: impl Into<String>, muted: bool) -> El {
    let k = f.k;
    text(t.into())
        .font(k.ty.ui)
        .color(if muted { k.c.text_muted } else { k.c.text_base })
        .w_full()
        .wrap()
}

/// A row's frame: its kind, where it's from, when, its title.
fn frame(f: &Frame, key: &str, kind: &str, where_: El, at: i64, now: i64, title: &str, kids: Vec<El>) -> El {
    let k = f.k;
    let (label, tone) = kind_chip(kind);
    let when = if at > 0 { relative_time(at, now) } else { String::new() };
    let mut c = col()
        .key(key.to_string())
        .role("article")
        .label(title.to_string())
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .px(16.0)
        .py(12.0)
        .radius(10.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(
            row()
                .key(format!("{key}/head"))
                .w_full()
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(badge(f, &format!("{key}/kind"), label, tone, false, ""))
                .child(row().min_w(0.0).shrink().child(where_))
                .child(el().grow(1.0))
                .child(text(when).font(small(f)).color(k.c.text_muted)),
        )
        .child(
            text(title.to_string())
                .key(format!("{key}/title"))
                .font(rune::tok::TextStyle::new(15.0, 600.0))
                .color(k.c.text_base)
                .w_full()
                .max_lines(2),
        );
    for e in kids {
        c = c.child(e);
    }
    c
}

/// A card's ref, and its space only when it isn't the person's own.
fn where_card(f: &Frame, c: &CardSummary) -> El {
    let k = f.k;
    let mut r = row()
        .min_w(0.0)
        .gap(0.0)
        .child(text(card_ref(c)).font(small(f).mono()).color(k.c.text_muted));
    if c.scope == "board" {
        let b = plain_line(&c.scope_label);
        if !b.is_empty() {
            r = r.child(text(format!(" · {b}")).font(small(f)).color(k.c.text_muted).max_lines(1));
        }
    }
    r
}

fn buttons(key: &str, kids: Vec<El>) -> El {
    let mut r = row().key(key.to_string()).w_full().gap(6.0).flex_wrap().center_y();
    for e in kids {
        r = r.child(e);
    }
    r
}

fn open_card_btn(f: &Frame, label: &str, id: &str, v: ButtonVariant) -> El {
    let id = id.to_string();
    btn(label.to_string(), v).on_press(move |s: &mut State| s.open_card(&id)).build(f.k)
}

/// Needs you: its heading with the count, its rows, Start an agent's
/// outcome and the quiet holds' line.
pub fn needs_section(s: &State, f: &mut Frame, needs: &[Need], now: i64) -> El {
    let k = f.k;
    let rows: Vec<&Need> = needs.iter().filter(|n| !matches!(n.kind, NeedKind::HeldQuiet(_))).collect();
    let mut head = row()
        .key("home/needs/h")
        .role("heading")
        .px(4.0)
        .gap(0.0)
        .child(text("Needs you").font(rune::tok::TextStyle::new(16.0, 600.0)).color(k.c.text_base));
    if !rows.is_empty() {
        head = head.child(
            text(format!(" · {}", rows.len()))
                .font(rune::tok::TextStyle::new(16.0, 500.0))
                .color(k.c.text_muted),
        );
    }
    let mut sec = col()
        .key("home/needs")
        .role("region")
        .label("Needs you")
        .w_full()
        .min_w(0.0)
        .gap(10.0)
        .child(head);
    if rows.is_empty() {
        sec = sec.child(
            text("Nothing waits on you right now.")
                .key("home/needs/none")
                .font(small(f))
                .color(k.c.text_muted)
                .px(4.0),
        );
    } else {
        let mut list = col().key("home/needs/list").role("list").label("What needs you").w_full().gap(10.0);
        for n in rows {
            list = list.child(needs_row(s, f, "home/need", n, now));
        }
        sec = sec.child(list);
    }
    if !s.home.started.is_empty() {
        sec = sec.child(
            text(s.home.started.clone())
                .key("home/needs/started")
                .role("status")
                .font(small(f))
                .color(k.c.text_muted)
                .px(4.0)
                .w_full()
                .wrap(),
        );
    }
    if let Some(Need {
        kind: NeedKind::HeldQuiet(ds),
        ..
    }) = needs.iter().find(|n| matches!(n.kind, NeedKind::HeldQuiet(_)))
    {
        sec = sec.child(held_quiet(s, f, ds));
    }
    sec
}

/// One Needs you row (keyed under `prefix`), any kind but the quiet holds'
/// line.
pub fn needs_row(s: &State, f: &mut Frame, prefix: &str, n: &Need, now: i64) -> El {
    let key = format!("{prefix}/{}:{}", n.kind_name(), n.id);
    match &n.kind {
        NeedKind::Check(c) | NeedKind::Decision(c) => card_need(s, f, &key, n.kind_name(), c, n.at, now),
        NeedKind::Merge(c, pr) => merge_need(s, f, &key, c, pr, n.at, now),
        NeedKind::Gone(c, g) => gone_need(s, f, &key, c, g, n.at, now),
        NeedKind::NoAgent(c) => no_agent_need(f, &key, c, n.at, now),
        NeedKind::Trust(c) => trust_row(s, f, &key, c, n.at, now),
        NeedKind::Loop(ring, cards) => stuck_need(f, &key, "loop", ring, None, cards, n.at, now),
        NeedKind::Stuck(end, cards) => stuck_need(f, &key, "stuck", &[], Some(end), cards, n.at, now),
        NeedKind::Held(d) => held_need(s, f, &key, d, now),
        NeedKind::HeldQuiet(ds) => held_quiet(s, f, ds),
    }
}

/// Waits nothing will end (card #481): cards that wait on each other, or
/// on one nobody is on. It says what's stuck and opens the cards.
#[allow(clippy::too_many_arguments)]
fn stuck_need(
    f: &Frame,
    key: &str,
    kind: &str,
    ring: &[CardRef],
    end: Option<&CardRef>,
    cards: &[&CardSummary],
    at: i64,
    now: i64,
) -> El {
    let k = f.k;
    let lp = end.is_none();
    let title = match end {
        None => loop_line(ring),
        Some(e) => stuck_line(e, cards),
    };
    let line = match end {
        None => "None of them can start until one stops waiting. Drop a wait on one of them, or finish one.".to_string(),
        Some(e) => format!(
            "Nobody holds {} and Irori won't hand it out, so the waits on it never end.",
            ref_of(e)
        ),
    };
    let open: Vec<&CardRef> = if lp { ring.iter().collect() } else { end.into_iter().collect() };
    let where_ = cards.first().map_or_else(|| el(), |c| where_card(f, c));
    let bs = open
        .iter()
        .map(|r| {
            let id = r.id.clone();
            btn(format!("Open {}", ref_of(r)), ButtonVariant::Outline)
                .help(plain_line(&r.title))
                .on_press(move |s: &mut State| s.open_card(&id))
                .build(k)
        })
        .collect();
    frame(
        f,
        key,
        kind,
        where_,
        at,
        now,
        &title,
        vec![body(f, line, true), buttons(&format!("{key}/acts"), bs)],
    )
}

/// An agent waits on the person to merge its PR (card #509).
fn merge_need(s: &State, f: &Frame, key: &str, c: &CardSummary, pr: &WaitPr, at: i64, now: i64) -> El {
    let k = f.k;
    let q = plain_line(&c.wait_question);
    let line = format!(
        "{} in {}: its agent waits on you to merge it.{}",
        merge_line(pr),
        pr.repo,
        if q.is_empty() { String::new() } else { format!(" “{q}”") }
    );
    let url = pr.url.clone();
    let open_pr = btn(format!("Open PR #{}", pr.number), ButtonVariant::Contrast)
        .help(pr.url.clone())
        .on_press_tx(move |_s: &mut State, tx: &mut Tx| { let _ = tx.open_url(&url); })
        .build(k);
    let mut kids = vec![
        body(f, line, true),
        buttons(
            &format!("{key}/acts"),
            vec![open_pr, open_card_btn(f, "Open card", &c.id, ButtonVariant::Outline)],
        ),
    ];
    let a = act(s, key);
    kids.extend(said(f, &format!("{key}/said"), a.and_then(|a| a.error.as_deref()), a.map_or("", |a| &a.done)));
    frame(f, key, "merge", where_card(f, c), at, now, &plain_line(&c.title), kids)
}

/// A card nobody holds whose wait no agent will end (card #520): Hand to
/// an agent puts it back on the Board with what was open said on it.
fn gone_need(s: &State, f: &Frame, key: &str, c: &CardSummary, g: &GoneWait, at: i64, now: i64) -> El {
    let k = f.k;
    let name = display_name(&g.agent);
    let q = plain_line(&g.question);
    let r = card_ref(c);
    let mut line = plain_line(&c.title);
    line.push_str(&if q.is_empty() { ".".to_string() } else { format!(": “{q}”.") });
    if !g.waited_on && !g.on.is_empty() {
        line.push_str(&format!(" It waits on {}, and {name}, who raised it, is gone.", plain_line(&g.on)));
    }
    line.push_str(&format!(
        " Nobody holds the card, so nothing will move it{}.",
        if g.more > 0 { format!(" ({} more like it)", g.more) } else { String::new() }
    ));
    let mut kids = vec![body(f, line, true)];
    if !g.no_hand_on.is_empty() {
        kids.push(
            text(format!("It can't be handed to an agent from here: {}.", plain_line(&g.no_hand_on)))
                .font(small(f))
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    }
    let mut bs = Vec::new();
    if g.no_hand_on.is_empty() {
        let out: Arc<Mutex<String>> = Arc::default();
        let (o1, o2) = (out.clone(), out);
        let id = c.id.clone();
        let akey = key.to_string();
        let ask = Ask::new(
            format!(
                "Hand {r} to a new agent? {name} is gone. Its open items close and are said on the card, and Irori starts an agent for it, or the next free agent takes it."
            ),
            "Hand to an agent",
            move |w| {
                let said = w.hand_on(&id)?;
                *o1.lock().unwrap_or_else(|e| e.into_inner()) = said;
                Ok(())
            },
        )
        .rich()
        .done(move |s| {
            let said = o2.lock().unwrap_or_else(|e| e.into_inner()).clone();
            s.home.acts.entry(akey.clone()).or_default().done = said;
        });
        let ck = format!("{key}/hand");
        bs.push(
            btn("Hand to an agent", ButtonVariant::Contrast)
                .on_press(move |s: &mut State| confirm::open(s, &ck, ask.clone()))
                .build(k),
        );
    }
    bs.push(open_card_btn(f, &format!("Open {r}"), &c.id, ButtonVariant::Outline));
    kids.push(buttons(&format!("{key}/acts"), bs));
    let a = act(s, key);
    let err = s.widgets.confirm_errors.get(&format!("{key}/hand")).map(String::as_str);
    kids.extend(said(
        f,
        &format!("{key}/said"),
        err.or(a.and_then(|a| a.error.as_deref())),
        a.map_or("", |a| &a.done),
    ));
    frame(f, key, "gone", where_card(f, c), at, now, &gone_title(c, g), kids)
}

/// No computer can start the card's agent (card #541): nothing to decide.
fn no_agent_need(f: &Frame, key: &str, c: &CardSummary, at: i64, now: i64) -> El {
    let kids = vec![
        body(f, plain_line(&c.no_agent), false),
        buttons(
            &format!("{key}/acts"),
            vec![open_card_btn(f, "Open card", &c.id, ButtonVariant::Outline)],
        ),
    ];
    frame(f, key, "noAgent", where_card(f, c), at, now, &plain_line(&c.title), kids)
}

/// A card whose agent can't start because its repo isn't trusted for
/// agents on that device (card #449): Trust and start, or Ask the device
/// it starts on.
fn trust_row(s: &State, f: &Frame, key: &str, c: &CardSummary, at: i64, now: i64) -> El {
    let k = f.k;
    let info = s.home.trust.get(&c.id).cloned().flatten();
    let folder = info
        .as_ref()
        .map(|i| i.folder.clone())
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| trust_from_reason(&c.cant_start).folder);
    let id = c.id.clone();
    let where_ = row()
        .key(format!("{key}/ref"))
        .role("button")
        .label(format!("Open {}", card_ref(c)))
        .focusable(true)
        .on_press(move |s: &mut State| s.open_card(&id))
        .child(text(card_ref(c)).font(small(f).mono()).color(k.c.text_muted));
    let on = match &info {
        Some(i) if i.here => " on this computer".to_string(),
        Some(i) => format!(" on {}", i.device),
        None => String::new(),
    };
    let what = info.as_ref().map(|i| i.what.clone()).filter(|w| !w.is_empty());
    let line = row()
        .w_full()
        .flex_wrap()
        .child(text("Can't start an agent: ").font(k.ty.ui).color(k.c.text_muted))
        .child(text(folder).font(k.ty.ui.mono()).color(k.c.text_muted))
        .child(text(format!(" isn't trusted for agents{on}.")).font(k.ty.ui).color(k.c.text_muted));
    let mut kids = vec![line];
    if let Some(w) = what {
        kids.push(body(f, w, true));
    }
    if let Some(by) = info.as_ref().filter(|i| i.here).and_then(|i| i.asked_by.clone()) {
        kids.push(
            text(format!("{by} asked you to trust this."))
                .font(small(f))
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    }
    let a = act(s, key).cloned().unwrap_or_default();
    let off = a.busy || !a.done.is_empty();
    let id = c.id.clone();
    let akey = key.to_string();
    let main = match &info {
        Some(i) if !i.here => btn(
            if a.busy { "Asking…".to_string() } else { format!("Ask {} to trust it", i.device) },
            ButtonVariant::Contrast,
        )
        .disabled(off)
        .on_press(move |s: &mut State| {
            let id = id.clone();
            run_act(s, akey.clone(), move |w| w.agents_trust_request(&id), |_, d| {
                format!("Asked {d}: confirm it there.")
            })
        }),
        _ => btn(if a.busy { "Trusting…" } else { "Trust and start" }, ButtonVariant::Contrast)
            .disabled(off)
            .on_press(move |s: &mut State| {
                let id = id.clone();
                run_act(s, akey.clone(), move |w| w.agents_trust_and_start(&id), |_, _| {
                    "Trusted. Starting its agent…".to_string()
                })
            }),
    };
    kids.push(buttons(
        &format!("{key}/acts"),
        vec![main.build(k), open_card_btn(f, "Open card", &c.id, ButtonVariant::Ghost)],
    ));
    kids.extend(said(f, &format!("{key}/said"), a.error.as_deref(), &a.done));
    frame(f, key, "trust", where_, at, now, &plain_line(&c.title), kids)
}

/// A card waiting on the person: its done check (Done, or Not done with a
/// reason) or each of its decisions (the options, recommended first, or
/// an answer in their own words).
fn card_need(s: &State, f: &mut Frame, key: &str, kind: &str, c: &CardSummary, at: i64, now: i64) -> El {
    let k = f.k;
    let spawn = if kind == "decision" { spawn_need(c) } else { None };
    let perm = spawn == Some(SpawnNeed::Permission);
    // The permission wait is answered at the agent's prompt, not here.
    let blockers: Vec<&CardBlocker> = s
        .home
        .details
        .get(&c.id)
        .map(|d| {
            d.blockers
                .iter()
                .filter(|b| waits_on_you(b) && !(perm && b.question.starts_with(WAITING_PERMISSION)))
                .collect()
        })
        .unwrap_or_default();
    let kind = match spawn {
        Some(SpawnNeed::Permission) => "permission",
        Some(SpawnNeed::Crashed) => "crashed",
        Some(SpawnNeed::Given) => "given",
        None => kind,
    };
    let mut kids = Vec::new();
    if perm {
        kids.push(permission_wait(s, f, key, c));
    }
    if blockers.is_empty() {
        if !perm {
            if !c.blocker_question.is_empty() {
                kids.push(body(f, plain_line(&c.blocker_question), false));
            }
            kids.push(buttons(
                &format!("{key}/acts"),
                vec![open_card_btn(f, "Open card", &c.id, ButtonVariant::Outline)],
            ));
        }
    } else {
        for b in blockers.iter().take(3) {
            kids.push(blocker_answer(s, f, &format!("{key}/b-{}", b.id), &c.id, b, false));
        }
    }
    if blockers.len() > 3 {
        let id = c.id.clone();
        kids.push(
            row().child(
                text(format!("{} more on the card", blockers.len() - 3))
                    .key(format!("{key}/more"))
                    .role("link")
                    .focusable(true)
                    .font(k.ty.small_medium)
                    .color(k.c.text_accent)
                    .on_press(move |s: &mut State| s.open_card(&id)),
            ),
        );
    }
    frame(f, key, kind, where_card(f, c), at, now, &plain_line(&c.title), kids)
}

/// "Waiting for your permission" (card #456): the agent Irori started is
/// stopped on a permission prompt nobody is attached to.
fn permission_wait(s: &State, f: &mut Frame, key: &str, c: &CardSummary) -> El {
    let k = f.k;
    let who: Option<&AgentRef> = c.working.iter().find(|w| w.spawned && !w.assigned);
    let name = who.map_or_else(|| "Its agent".to_string(), display_name);
    let mut acts = row().key(format!("{key}/perm-acts")).w_full().gap(6.0).flex_wrap();
    if let Some(w) = who {
        acts = acts.child(spawned_actions(s, f, &format!("{key}/spawned"), w, &c.id, &[Action::Attach, Action::Logs]));
    }
    let k2 = f.k;
    let _ = k2;
    acts = acts
        .child(
            btn("Add to allow-list…", ButtonVariant::Outline)
                .on_press(|s: &mut State| s.open_dialog(Dialog::Settings("agents".into())))
                .build(k),
        )
        .child(open_card_btn(f, "Open card", &c.id, ButtonVariant::Ghost));
    col()
        .key(format!("{key}/perm"))
        .w_full()
        .gap(6.0)
        .child(body(
            f,
            format!("{name} is stopped on a permission prompt nobody is there to answer."),
            false,
        ))
        .child(acts)
}

/// The drafts key an item's own words go under (shared with the card
/// page's box for the same item).
pub fn words_key(card_id: &str, blocker_id: &str) -> String {
    format!("{card_id}/blocker/{blocker_id}")
}

/// Sends an answer at once: the row says so now, the save runs in the
/// background, and a refusal brings the item back (with the words typed
/// still in the box) and says why.
pub fn answer(s: &mut State, card_id: &str, b: &CardBlocker, option: i64, words: String) {
    let key = answer_key(card_id, &b.id);
    if !s.home.busy.insert(key.clone()) {
        return;
    }
    s.home.said.insert(key.clone(), answer_said(b, option).to_string());
    s.home.own.remove(&key);
    let failed = answer_failed(b);
    let (cid, bid) = (card_id.to_string(), b.id.clone());
    let dkey = words_key(card_id, &b.id);
    s.spawn(
        move |w| w.answer(&cid, &bid, option, &words, true, &[]).map(|_| ()),
        move |s, r| {
            s.home.busy.remove(&key);
            match r {
                Ok(()) => {
                    if option < 0 {
                        crate::text::drafts::set_draft(s, &dkey, "", &[]);
                    }
                }
                Err(e) => {
                    s.home.said.remove(&key);
                    if option < 0 {
                        open_words(s, &key, &dkey);
                    }
                    s.error(failed, &e);
                }
            }
        },
    );
}

/// Opens an item's own-words box with its draft.
pub fn open_words(s: &mut State, key: &str, dkey: &str) {
    let d = crate::text::drafts::draft(s, dkey);
    let mut e = TextEdit::new(&d.text);
    e.focused = true;
    s.home.own.insert(key.to_string(), e);
}

/// An item's answers in a row (Needs you, Notifications). A pick or an
/// own-words answer shows at once and saves in the background (card #644).
/// `answer_opens`: "Answer…" opens the card to answer there instead.
pub fn blocker_answer(s: &State, f: &Frame, key: &str, card_id: &str, b: &CardBlocker, answer_opens: bool) -> El {
    let k = f.k;
    let akey = answer_key(card_id, &b.id);
    let dkey = words_key(card_id, &b.id);
    let question = plain_line(&b.question);
    let mut c = col().key(key.to_string()).w_full().min_w(0.0).gap(6.0);
    if !b.done_check {
        let mut q = col().w_full().gap(2.0);
        // Irori asks this one in the person's own name (card #573).
        if b.question != GIVEN_QUESTION {
            q = q.child(crate::widgets::bits::asker(f, &format!("{key}/asker"), &b.raised_by));
        }
        c = c.child(q.child(body(f, question.clone(), false)));
    } else if let Some(r) = earlier_round(s, f, &format!("{key}/round"), b, true) {
        c = c.child(r);
    }
    if !b.context.is_empty() {
        c = c.child(
            text(plain_line(&b.context))
                .font(rune::tok::TextStyle::new(13.0, 400.0))
                .color(k.c.text_muted)
                .w_full()
                .max_lines(3),
        );
    }
    let replied = b.state == blocker::REPLIED;
    if replied && !b.missing.is_empty() {
        c = c.child(
            text(format!(
                "They still need: {}",
                b.missing.iter().map(|m| plain_line(m)).collect::<Vec<_>>().join("; ")
            ))
            .font(rune::tok::TextStyle::new(13.0, 400.0))
            .color(k.c.text_base)
            .w_full()
            .wrap(),
        );
    }
    let said_now = s.home.said.get(&akey).cloned().unwrap_or_default();
    if said_now.is_empty() {
        if let Some(edit) = s.home.own.get(&akey).filter(|_| !answer_opens) {
            c = c.child(own_words(f, key, card_id, b, edit, &akey, &dkey, &question));
        } else {
            let mut bs = Vec::new();
            if !b.options_aside {
                for (n, (o, i)) in by_recommendation(&b.options).into_iter().enumerate() {
                    let primary = n == 0 && (o.recommended || b.done_check);
                    let mut label = plain_line(&o.answer);
                    let rec = o.recommended && !b.done_check;
                    if rec {
                        label.push_str(" (recommended)");
                    }
                    let (cid, bb) = (card_id.to_string(), b.clone());
                    let mut bt = btn(label.clone(), if primary { ButtonVariant::Contrast } else { ButtonVariant::Outline })
                        .label_with(move |st, col| text(label).font(st).color(col).max_w(520.0).wrap())
                        .on_press(move |s: &mut State| answer(s, &cid, &bb, i as i64, String::new()));
                    if !o.why.is_empty() {
                        bt = bt.help(plain_line(&o.why));
                    }
                    bs.push(bt.build(k).key(format!("{key}/opt-{i}")));
                }
            }
            // The given question is its two answers: words aren't a yes.
            if b.question != GIVEN_QUESTION {
                let label = if b.done_check {
                    "Not done…"
                } else if replied {
                    "Reply…"
                } else {
                    "Answer…"
                };
                let (cid, ak, dk) = (card_id.to_string(), akey.clone(), dkey.clone());
                bs.push(
                    btn(label, ButtonVariant::Outline)
                        .on_press(move |s: &mut State| {
                            if answer_opens {
                                s.open_card(&cid);
                            } else {
                                open_words(s, &ak, &dk);
                            }
                        })
                        .build(k)
                        .key(format!("{key}/own")),
                );
            }
            bs.push(open_card_btn(f, "Open card", card_id, ButtonVariant::Ghost).key(format!("{key}/open")));
            c = c.child(
                buttons(&format!("{key}/answers"), bs)
                    .role("group")
                    .label(format!("Answers to: {question}")),
            );
        }
    }
    if let Some(e) = said(f, &format!("{key}/said"), None, &said_now) {
        c = c.child(e);
    }
    c
}

/// The fold-out box for an answer, a reply or a reason: Enter sends,
/// Shift+Enter is a new line, Esc cancels.
#[allow(clippy::too_many_arguments)]
fn own_words(
    f: &Frame,
    key: &str,
    card_id: &str,
    b: &CardBlocker,
    edit: &TextEdit,
    akey: &str,
    dkey: &str,
    question: &str,
) -> El {
    let k = f.k;
    let label = format!(
        "{}: {question}",
        if b.done_check { "Why it isn't done" } else { "Your answer to" }
    );
    let placeholder = if b.done_check {
        if b.gone_agent.is_some() {
            "Say why it isn't done; the next agent on it gets it"
        } else {
            "Say why it isn't done; it goes back to the agent"
        }
    } else {
        "Answer in your own words"
    };
    let send_label = if b.done_check { "Not done" } else { "Answer" };
    let style = rune::tok::TextStyle {
        line_height: (13.0f32 * 1.35 * 2.0).round() / 2.0,
        ..k.ty.ui
    };
    let (a1, a2, a3, a4) = (akey.to_string(), akey.to_string(), akey.to_string(), akey.to_string());
    let d1 = dkey.to_string();
    let (c1, b1) = (card_id.to_string(), b.clone());
    let (c2, b2) = (card_id.to_string(), b.clone());
    let send = move |s: &mut State, cid: &str, bb: &CardBlocker, ak: &str| {
        let t = s.home.own.get(ak).map(|e| e.text.clone()).unwrap_or_default();
        if !t.trim().is_empty() {
            answer(s, cid, bb, -1, t);
        }
    };
    let field = text_input(edit, style)
        .key(format!("{key}/words"))
        .w_full()
        .color(k.c.text_base)
        .placeholder(placeholder.to_string(), k.c.text_faint)
        .caret(k.c.text_base, k.selection())
        .label(label)
        .on_edit(rune::edit_access(move |s: &mut State| s.home.own.entry(a1.clone()).or_default()))
        .on_input_change(move |s: &mut State, _tx: &mut Tx| {
            let t = s.home.own.get(&a2).map(|e| e.text.clone()).unwrap_or_default();
            crate::text::drafts::set_draft(s, &d1, &t, &[]);
        })
        .on_input_key(move |s: &mut State, ev: &KeyEvent, _tx: &mut Tx| {
            if ev.key == Key::Escape {
                s.home.own.remove(&a3);
                return true;
            }
            false
        })
        .on_submit(move |s: &mut State, _tx: &mut Tx| send(s, &c1, &b1, &a4));
    let boxed = col()
        .key(format!("{key}/words-box"))
        .grow(1.0)
        .min_w(256.0)
        .px(10.0)
        .py(6.0)
        .radius(k.radius.md)
        .fill(k.c.bg_base)
        .border(1.0, if edit.focused { k.c.border_focus } else { k.c.border_base })
        .child(rune::auto_grow(field, 1, 8));
    let empty = edit.text.trim().is_empty();
    let a5 = akey.to_string();
    let a6 = akey.to_string();
    row()
        .key(format!("{key}/form"))
        .w_full()
        .gap(6.0)
        .flex_wrap()
        .items_start()
        .child(boxed)
        .child(
            btn(send_label, ButtonVariant::Contrast)
                .disabled(empty)
                .on_press(move |s: &mut State| send(s, &c2, &b2, &a5))
                .build(k),
        )
        .child(
            btn("Cancel", ButtonVariant::Ghost)
                .on_press(move |s: &mut State| {
                    s.home.own.remove(&a6);
                })
                .build(k),
        )
}

fn sender(d: &Delivery) -> String {
    let n = plain_line(&d.message.from.name);
    if n.is_empty() { "an agent".into() } else { n }
}

/// The names the held rows use: the roster's, else the messages'
/// (lib/talk.ts nameLookup, the inbox package's copy).
fn names(s: &State) -> Names {
    let mut n = Names::default();
    for a in &s.data.agents {
        n.add(&a.card.key, &a.card.name);
    }
    for d in &s.home.held {
        n.add(&d.message.from.key, &d.message.from.name);
    }
    n
}

/// Repo keys as this device's checkouts name them.
fn repos(s: &State) -> Repos {
    Repos(s.home.repo_names.iter().map(|r| (r.repo_key.clone(), r.name.clone())).collect())
}

/// A held message that asks the person (card #488): their own message no
/// agent in its repo can take yet (Start an agent there), or one from
/// another person (Release, Dismiss, or Allow sender).
fn held_need(s: &State, f: &Frame, key: &str, d: &Delivery, now: i64) -> El {
    let k = f.k;
    let names = names(s);
    let repos = repos(s);
    let name = |key: &str| names.name(key);
    let repo = |key: &str| repos.name(key);
    let unowned = d.hold == hold::UNOWNED && d.to.is_empty();
    let mine = d.message.from.key.starts_with("user:");
    let where_repo = {
        let r = plain_line(&repo(&d.repo_key));
        if d.repo_key.is_empty() || r.is_empty() { "its repo".to_string() } else { r }
    };
    let to = if d.to.is_empty() {
        where_repo.clone()
    } else {
        let n = plain_line(&name(&d.to));
        if n.is_empty() { "an agent".into() } else { n }
    };
    let from = if mine { "You".to_string() } else { sender(d) };
    let where_ = text(format!("{from} → {to}")).font(small(f)).color(k.c.text_muted).max_lines(1);
    let title = hold_text(d, &names, &repos);
    let mut kids = vec![
        text(d.message.text.clone())
            .font(k.ty.ui)
            .color(k.c.text_base)
            .w_full()
            .max_lines(3),
    ];
    let a = act(s, key).cloned().unwrap_or_default();
    if a.done.is_empty() {
        let mut bs = Vec::new();
        let id = d.id.clone();
        let akey = key.to_string();
        if unowned {
            if mine {
                let (id, akey) = (id.clone(), akey.clone());
                bs.push(
                    btn(format!("Start an agent in {where_repo}"), ButtonVariant::Contrast)
                        .disabled(a.busy)
                        .help(format!("Makes your message a card in {where_repo} and gets an agent on it"))
                        .on_press(move |s: &mut State| {
                            let id = id.clone();
                            run_act(s, akey.clone(), move |w| w.inbox_start_agent(&id), |s, r| {
                                let said = started_line(&r);
                                s.home.started = said.clone();
                                said
                            })
                        })
                        .build(k),
                );
            }
        } else {
            let (id, akey) = (id.clone(), akey.clone());
            bs.push(
                btn("Release", ButtonVariant::Contrast)
                    .disabled(a.busy)
                    .on_press(move |s: &mut State| {
                        let id = id.clone();
                        run_act(s, akey.clone(), move |w| w.inbox_release(&id), |_, _| "Released.".into())
                    })
                    .build(k),
            );
        }
        {
            let (id, akey) = (id.clone(), akey.clone());
            bs.push(
                btn("Dismiss", ButtonVariant::Outline)
                    .disabled(a.busy)
                    .on_press(move |s: &mut State| {
                        let id = id.clone();
                        run_act(s, akey.clone(), move |w| w.inbox_dismiss(&id), |_, _| "Dismissed.".into())
                    })
                    .build(k),
            );
        }
        if allow_lifts(d) {
            let who = sender(d);
            bs.push(
                btn("Allow sender", ButtonVariant::Outline)
                    .disabled(a.busy)
                    .on_press(move |s: &mut State| {
                        let id = id.clone();
                        let who = who.clone();
                        run_act(s, akey.clone(), move |w| w.inbox_allow_and_release(&id), move |_, _| {
                            format!("Allowed {who} and released.")
                        })
                    })
                    .build(k),
            );
        }
        kids.push(buttons(&format!("{key}/acts"), bs));
    }
    kids.extend(said(f, &format!("{key}/said"), a.error.as_deref(), &a.done));
    let at = ms(&d.created_at).unwrap_or(0);
    frame(f, key, "held", where_, at, now, &title, kids)
}

/// The held messages that don't ask the person one by one (card #488):
/// one line, with Release all and See them.
fn held_quiet(s: &State, f: &Frame, ds: &[&Delivery]) -> El {
    let k = f.k;
    // Ones the block or allow list holds aren't released in bulk.
    let listed = |d: &Delivery| d.hold == hold::BLOCKED || d.hold == hold::NOT_ALLOWED;
    let bulk: Vec<String> = ds.iter().filter(|d| !listed(d)).map(|d| d.id.clone()).collect();
    let lists = ds.len() - bulk.len();
    let paused = ds.iter().filter(|d| !listed(d) && d.hold == hold::PAUSED).count();
    let between = bulk.len() - paused;
    let parts: Vec<String> = [
        (between > 0).then(|| format!("{between} between agents")),
        (paused > 0).then(|| format!("{paused} while wakes were paused")),
        (lists > 0).then(|| format!("{lists} from senders your lists hold back")),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut line = row()
        .key("home/needs/quiet")
        .role("group")
        .label("Held messages")
        .w_full()
        .px(4.0)
        .gap(8.0)
        .flex_wrap()
        .center_y()
        .child(
            row()
                .min_w(0.0)
                .child(
                    text(format!("{} held", ds.len()))
                        .font(rune::tok::TextStyle::new(12.5, 500.0))
                        .color(k.c.text_base),
                )
                .child(
                    text(if parts.is_empty() { String::new() } else { format!(": {}", parts.join(", ")) })
                        .font(small(f))
                        .color(k.c.text_muted),
                ),
        );
    let busy = s.home.quiet_busy;
    if !bulk.is_empty() {
        let mut b = Button::new(if busy { "Releasing…" } else { "Release all" })
            .variant(ButtonVariant::Outline)
            .control_size(ControlSize::Mini)
            .disabled(busy);
        if lists > 0 {
            b = b.help("Leaves the ones your block or allow list holds");
        }
        line = line.child(
            b.on_press(move |s: &mut State| release_all(s, bulk.clone()))
                .build(k)
                .key("home/needs/quiet/release"),
        );
    }
    line = line.child(
        Button::new("See them")
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Mini)
            .on_press(|s: &mut State| s.open_dialog(Dialog::Held { from: None }))
            .build(k)
            .key("home/needs/quiet/see"),
    );
    if !s.home.quiet_said.is_empty() {
        line = line.child(
            text(s.home.quiet_said.clone())
                .role("status")
                .font(small(f))
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    }
    line
}

/// Release all: delivers every quiet hold that can go now.
fn release_all(s: &mut State, ids: Vec<String>) {
    if s.home.quiet_busy {
        return;
    }
    s.home.quiet_busy = true;
    s.home.quiet_said.clear();
    let n = ids.len();
    s.spawn(
        move |w| ids.iter().filter(|id| w.inbox_release(id).is_err()).count(),
        move |s, left| {
            s.home.quiet_busy = false;
            s.home.quiet_said = if left == 0 {
                if n == 1 { "Released.".into() } else { format!("Released all {n}.") }
            } else {
                format!("Released {}; {left} can't go yet (See them says why).", n - left)
            };
        },
    );
}

/// The held rows' words as Home says them (these were home::held's cases
/// before it was folded into the inbox package's copy).
#[cfg(test)]
mod tests {
    use super::*;
    use inbox::st;
    use irori_window::ts::StartedAgent;

    fn d(f: impl FnOnce(&mut Delivery)) -> Delivery {
        let mut d = Delivery {
            state: st::HELD.into(),
            ..Delivery::default()
        };
        d.message.from.key = "dev/otter".into();
        d.message.from.name = "otter".into();
        d.to = "dev/heron".into();
        f(&mut d);
        d
    }

    #[test]
    fn holds_name_who_is_involved() {
        let mut names = Names::default();
        names.add("dev/heron", "heron");
        let repos = Repos::default();
        let w = |x: &Delivery| hold_text(x, &names, &repos);
        let loop_ = d(|d| {
            d.hold = hold::HOP_LIMIT.into();
            d.message.hop = 8;
        });
        assert_eq!(w(&loop_), "otter and heron have messaged each other 8 times without stopping.");
        assert_eq!(w(&d(|d| d.hold = hold::HOP_LIMIT.into())), "otter and heron kept messaging each other without stopping.");
        assert_eq!(w(&d(|d| d.hold = hold::BLOCKED.into())), "otter is on your block list.");
        let mine = d(|d| {
            d.hold = hold::UNOWNED.into();
            d.message.from.key = "user:me".into();
            d.repo_key = "github.com/acme/example-app.git".into();
            d.to = String::new();
        });
        assert_eq!(w(&mine), "No agent is working in example-app to take your message.");
        assert_eq!(w(&d(|d| d.hold = hold::UNOWNED.into())), "No agent is working in its repo to take it yet.");
        assert_eq!(
            w(&d(|d| d.held_for = "off_side".into())),
            "Irori picked heron for a card on a side heron's device doesn't run. Release delivers it."
        );
        assert_eq!(w(&d(|d| d.hold = "weird".into())), "Irori held it.");
        assert_eq!(w(&d(|_| {})), "");
        // A checkout's name over the key's last part.
        let repos = Repos([("root:abc".to_string(), "irori".to_string())].into_iter().collect());
        assert_eq!(repos.name("root:abc"), "irori");
        assert_eq!(repos.name("root:abcdef12"), "local repo abcdef1");
    }

    #[test]
    fn started_lines_and_allow() {
        let s = |started, offered| StartedAgent {
            card_ref: "#12".into(),
            repo: "irori".into(),
            started,
            offered,
        };
        assert_eq!(started_line(&s(false, true)), "Your message is card #12 in irori, handed to a free agent.");
        assert_eq!(started_line(&s(false, false)), "Your message is card #12 in irori; it waits for a free agent.");
        assert!(allow_lifts(&d(|d| d.hold = hold::NOT_ALLOWED.into())));
        assert!(!allow_lifts(&d(|d| {
            d.hold = hold::NOT_ALLOWED.into();
            d.state = st::DISMISSED.into();
        })));
    }
}
