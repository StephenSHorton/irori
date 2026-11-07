//! A card's thread (cards/thread.tsx): comments, messages about the card
//! ("A -> B") and one muted line per thing that happened, oldest first,
//! with who wrote each and where. The newest page shows first; "Show
//! earlier" opens the rest. A message the card cut, held or with files
//! opens in full in place. Under the entries: the live working line, then
//! the composer (the comment box, or a blocker's answer form).

use std::collections::HashSet;

use irori_window::cards::{AgentRef, CardDetail, CardMessage, CardSummary, CardThreadItem, EDIT_CAN, EDIT_EDITING};
use irori_window::cards::Holding;
use rune::{El, ScrollSpec, TextEdit, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, TextArea};

use super::{err_line, ts};
use crate::logic::agent_status::{ms, now_ms};
use crate::logic::cards::{self as lc, Tone, Work, state};
use crate::logic::plain::{plain_block, plain_line, relative_time};
use crate::logic::thread::{
    self as lt, LineTone, THREAD_PAGE, blocker_item_id, is_revision, live_roster, opens_in_full, ponder_word,
    start_agent_item, thread_entry_id, thread_label, visible, working_line,
};
use crate::text::{self as tt, File, RichOpts};
use crate::ui::{Frame, State};
use crate::widgets::{bits, faces};

/// The agents on a thread and whether it is over, for its working line
/// (card #299); `working` and `jobs`, when given, are who the card's own
/// rule says works on it now (card #317).
#[derive(Clone, Copy, Default)]
pub struct On<'a> {
    pub agents: &'a [AgentRef],
    pub over: bool,
    pub working: Option<&'a [AgentRef]>,
    pub jobs: Option<&'a [AgentRef]>,
}

/// What a thread shows.
#[derive(Default)]
pub struct ThreadOpts<'a> {
    /// The card (or Talk thread) it is on: a message opens in full there,
    /// a comment's Modify and Start an agent act on it.
    pub card_id: String,
    pub items: &'a [CardThreadItem],
    /// Entries older than the ones sent (BoardService.Card sends the
    /// newest MaxThreadView).
    pub earlier: i64,
    /// The card's oldest activity lines are no longer kept.
    pub history_trimmed: bool,
    /// What an empty thread shows: a line ("Nothing yet." by default).
    pub empty_text: Option<String>,
    /// Or an element (a loading shape).
    pub empty: Option<El>,
    pub compact: bool,
    /// The items sit under their blocker, so they don't name it.
    pub in_blocker: bool,
    /// Under the entries, newest last.
    pub composer: Option<El>,
    /// The entries scroll on their own, newest in view, with the composer
    /// under them (Talk's pane; give the thread a bounded height).
    pub scroll: bool,
    pub on: Option<On<'a>>,
    /// The person's latest comment offers "Start an agent with this"
    /// (card #452): a card with nobody on it.
    pub start_agent: bool,
    /// The blockers the page shows: an activity line about one links to it.
    pub blockers_here: Option<&'a HashSet<String>>,
}

/// One thread row's own state.
#[derive(Default)]
pub struct RowUi {
    /// The message opened in full.
    pub full: Option<CardMessage>,
    pub busy: bool,
    pub error: Option<String>,
    /// Released here: its text joins the card once the inbox lets it go.
    pub released: bool,
    /// "Start an agent with this" was pressed.
    pub started: bool,
    /// What the person just did to their comment (its edit state), until
    /// the card's next read changes it: (edit, what it was seen as).
    pub shown: Option<(String, String)>,
    pub draft: TextEdit,
}

fn row_key(card: &str, id: &str) -> String {
    format!("{card}/{id}")
}

fn row_mut<'a>(s: &'a mut State, k: &str) -> &'a mut RowUi {
    s.card.rows.entry(k.to_string()).or_default()
}

/// A card's thread under its page, with the comment box.
pub fn card_thread(s: &State, f: &mut Frame, d: &CardDetail, now_: &CardSummary) -> El {
    let c = &d.summary;
    let here: HashSet<String> = d.blockers.iter().map(|b| b.id.clone()).collect();
    let w = Work::from(now_);
    let working = lc::working_now(&w);
    let jobs = lc::running_job_now(&w);
    let composer = super::comment::comment_box(s, f, &c.id, false, d.on_board.is_some());
    let opts = ThreadOpts {
        card_id: c.id.clone(),
        items: &d.thread,
        earlier: d.thread_earlier,
        history_trimmed: d.history_trimmed,
        composer: Some(composer),
        on: Some(On {
            agents: &c.working,
            over: lc::over(&c.state),
            working: Some(&working),
            jobs: Some(&jobs),
        }),
        start_agent: startable(d),
        blockers_here: Some(&here),
        ..ThreadOpts::default()
    };
    thread_view(s, f, "card/thread", &opts)
}

/// The person's comment on this card may "Start an agent with this" (card
/// #452): their own board's card, open, not an ask, and nobody on it.
pub fn startable(d: &CardDetail) -> bool {
    let c = &d.summary;
    if d.on_board.is_some() || c.ask.is_some() {
        return false;
    }
    if matches!(c.state.as_str(), state::COMPLETED | state::CANCELED | state::FAILED) {
        return false;
    }
    c.holder.state == Holding::Nobody
}

/// A thread: `key` names it on the page (prefix it with your package).
pub fn thread_view(s: &State, f: &mut Frame, key: &str, o: &ThreadOpts) -> El {
    let k = f.k;
    let items = o.items;
    let now = now_ms();
    let live = live_roster(&s.data.agents);
    let status = o
        .on
        .and_then(|on| working_line(on.agents, items, &live, now, on.over, on.working, on.jobs));
    // A comment a notification opened the card at: shown even when older
    // than the first page.
    let focus = s.card.focus_comment.as_deref();
    let holds = focus.is_some_and(|fc| items.iter().any(|it| it.id == fc));
    let start_id = if !o.in_blocker && o.start_agent && !o.card_id.is_empty() {
        start_agent_item(items)
    } else {
        String::new()
    };
    let show_all = s.card.show_all.contains(key);
    let paged = visible(items, show_all, THREAD_PAGE);
    let shown = if holds && !paged.iter().any(|it| Some(it.id.as_str()) == focus) { items } else { paged };
    let hidden = items.len() - shown.len();
    let gap = if o.compact { 4.0 } else { 8.0 };
    let mut entries = col().key(format!("{key}/entries")).w_full().min_w(0.0).gap(gap);
    if hidden > 0 {
        let kk = key.to_string();
        entries = entries.child(
            row().child(
                Button::new(format!("Show {hidden} earlier"))
                    .variant(ButtonVariant::GhostMuted)
                    .control_size(ControlSize::Mini)
                    .on_press(move |s: &mut State| {
                        s.card.show_all.insert(kk.clone());
                    })
                    .build(k),
            ),
        );
    } else if (o.earlier > 0 || o.history_trimmed) && !items.is_empty() {
        entries = entries.child(
            text(if o.earlier > 0 {
                format!(
                    "{} earlier {} shown here.",
                    o.earlier,
                    if o.earlier == 1 { "entry isn't" } else { "entries aren't" }
                )
            } else {
                "Earlier history not kept.".into()
            })
            .font(k.ty.text_xsmall)
            .color(k.c.text_muted),
        );
    }
    if shown.is_empty() {
        if status.is_none() {
            if let Some(e) = &o.empty {
                entries = entries.child(e.clone());
            } else {
                entries = entries.child(
                    text(o.empty_text.clone().unwrap_or_else(|| "Nothing yet.".into()))
                        .font(ts(12.5, 400.0))
                        .color(k.c.text_muted)
                        .w_full()
                        .wrap(),
                );
            }
        }
    } else {
        for it in shown {
            let focused = holds && Some(it.id.as_str()) == focus;
            let mut e = col()
                .key(thread_entry_id(&it.id))
                .w_full()
                .min_w(0.0)
                .child(thread_row(s, f, key, o, it, it.id == start_id));
            if focused {
                e = e
                    .mx(-6.0)
                    .px(6.0)
                    .py(2.0)
                    .radius(3.0)
                    .fill(k.c.text_accent.alpha(0.1))
                    .border(1.0, k.c.text_accent.alpha(0.4));
            }
            entries = entries.child(e);
        }
    }
    if let Some(st) = &status {
        entries = entries.child(match st.tone {
            LineTone::Working => {
                s.card.working_line.set(true);
                let doing = if st.doing.is_empty() { ponder_word(s.card.ponder).to_string() } else { st.doing.clone() };
                let line = format!("{} {} {doing}…", st.who, st.verb);
                row()
                    .key(format!("{key}/working"))
                    .w_full()
                    .gap(8.0)
                    .py(2.0)
                    .center_y()
                    .role("status")
                    .label(st.text.clone())
                    .child(bits::spinner(f, &format!("{key}/working-spin")))
                    .child(
                        rune::rich(line, vec![rune::Span::new(0, st.who.len()).weight(500.0)])
                            .font(ts(12.5, 400.0))
                            .color(k.c.text_base)
                            .shrink()
                            .max_lines(1),
                    )
            }
            LineTone::Stopped => text(st.text.clone())
                .key(format!("{key}/stopped"))
                .font(ts(11.5, 400.0))
                .color(k.c.warning.fg)
                .role("status")
                .w_full()
                .wrap(),
        });
    }
    if o.scroll {
        let mut c = col()
            .key(key.to_string())
            .w_full()
            .h_full()
            .min_h(0.0)
            .gap(8.0)
            .child(
                col()
                    .key(format!("{key}/scroll"))
                    .w_full()
                    .grow(1.0)
                    .min_h(if shown.len() > 2 { 64.0 } else { 0.0 })
                    .scroll(ScrollSpec { stick_bottom: true })
                    .child(entries),
            );
        if let Some(cmp) = &o.composer {
            c = c.child(cmp.clone());
        }
        return c;
    }
    let mut c = col().key(key.to_string()).w_full().min_w(0.0).gap(gap).child(entries);
    if let Some(cmp) = &o.composer {
        c = c.child(cmp.clone());
    }
    c
}

fn time_cell(f: &Frame, it: &CardThreadItem) -> El {
    let k = f.k;
    let at = ms(&it.at);
    let t = at.map(|t| relative_time(t, now_ms())).unwrap_or_default();
    text(t.clone())
        .font(ts(12.5, 400.0))
        .color(k.c.text_muted)
        .w(56.0)
        .help(t)
}

/// The device an entry was written on.
fn device_tag(f: &Frame, key: &str, device: &str) -> El {
    let d = plain_line(device);
    bits::badge(f, key, &d, Tone::Slate, false, &format!("Written on {d}"))
}

/// Who made an activity line's change (card #117), dimmed; Irori's own
/// lines have none.
fn actor_face(s: &State, f: &mut Frame, key: &str, who: &AgentRef) -> Option<El> {
    if who.key.is_empty() || who.status == "irori" {
        return None;
    }
    Some(
        el().opacity(0.75)
            .child(faces::face(s, f, key, who, faces::FaceOpts { no_dot: true, inline: true, ..Default::default() })),
    )
}

fn thread_row(s: &State, f: &mut Frame, tkey: &str, o: &ThreadOpts, it: &CardThreadItem, startable: bool) -> El {
    let k = f.k;
    let key = format!("{tkey}/row-{}", it.id);
    let rk = row_key(&o.card_id, &it.id);
    let empty = RowUi::default();
    let ui = s.card.rows.get(&rk).unwrap_or(&empty);
    let seen = format!("{}\n{}", it.edit, it.text);
    let edit = match &ui.shown {
        Some((e, sn)) if *sn == seen => e.clone(),
        _ => it.edit.clone(),
    };
    let base = row().key(key.clone()).w_full().min_w(0.0).gap(8.0).items_start().child(time_cell(f, it));

    // A revision of its blocker: what the item asks now.
    if is_revision(it) {
        let mut r = row().grow(1.0).min_w(0.0).gap(6.0).items_start().flex_wrap();
        if let Some(fc) = actor_face(s, f, &format!("{key}/face"), &it.who) {
            r = r.child(fc);
        }
        r = r
            .child(bits::badge(f, &format!("{key}/revised"), "Revised", Tone::Warning, false, ""))
            .child(tt::rich_line(s, f, &format!("{key}/text"), &it.text, &RichOpts { font: Some(ts(12.5, 400.0)), ..RichOpts::default() }).shrink());
        if !it.device.is_empty() {
            r = r.child(device_tag(f, &format!("{key}/device"), &it.device));
        }
        return base.child(r);
    }
    // An activity line (card #117): dim, a log not a comment; one about a
    // blocker the page shows links to it (card #490).
    if it.kind == lt::KIND_EVENT {
        let mut r = row().grow(1.0).min_w(0.0).gap(6.0).items_start();
        if let Some(fc) = actor_face(s, f, &format!("{key}/face"), &it.who) {
            r = r.child(fc);
        }
        let muted = RichOpts {
            font: Some(ts(12.5, 400.0)),
            color: Some(k.c.text_muted),
            ..RichOpts::default()
        };
        let linked = !it.blocker_id.is_empty() && !o.in_blocker && o.blockers_here.is_some_and(|h| h.contains(&it.blocker_id));
        let body = if linked {
            let target = blocker_item_id(&it.blocker_id);
            el().grow(1.0)
                .min_w(0.0)
                .role("button")
                .help("Show this blocker")
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |_: &mut State, tx: &mut Tx| tx.scroll_into_view(target.clone()))
                .child(tt::rich_line(
                    s,
                    f,
                    &format!("{key}/text"),
                    &it.text,
                    &RichOpts {
                        interactive: false,
                        selectable: false,
                        ..muted.clone()
                    },
                ))
        } else {
            tt::rich_line(s, f, &format!("{key}/text"), &it.text, &muted).grow(1.0).min_w(0.0)
        };
        r = r.child(body);
        if !it.device.is_empty() {
            r = r.child(device_tag(f, &format!("{key}/device"), &it.device));
        }
        return base.child(r);
    }

    let l = thread_label(it, o.in_blocker);
    let mut head = row()
        .w_full()
        .min_w(0.0)
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(faces::face(s, f, &format!("{key}/face"), &it.who, faces::FaceOpts { inline: true, ..Default::default() }))
        .child(text(plain_line(&l.author)).font(ts(12.5, 500.0)).color(k.c.text_base));
    if let (Some(to), Some(arrow)) = (&it.to, &l.arrow) {
        head = head
            .child(text("->").font(ts(12.5, 400.0)).color(k.c.text_muted).a11y_hidden())
            .child(faces::face(s, f, &format!("{key}/to"), to, faces::FaceOpts { inline: true, ..Default::default() }))
            .child(text(plain_line(arrow)).font(ts(12.5, 500.0)).color(k.c.text_base));
    }
    if !l.device.is_empty() {
        head = head.child(device_tag(f, &format!("{key}/device"), &l.device));
    }
    for (i, b) in l.badges.iter().enumerate() {
        head = head.child(bits::badge(f, &format!("{key}/badge{i}"), &b.label, b.tone, false, ""));
    }
    let mut body = col().grow(1.0).min_w(0.0).gap(4.0).child(head);
    let text_opts = RichOpts {
        font: Some(ts(12.5, 400.0)),
        mentions: &it.mentions,
        ..RichOpts::default()
    };
    let muted = |t: &str| text(t.to_string()).font(ts(12.5, 400.0)).color(k.c.text_muted).w_full().wrap();
    if let Some(full) = &ui.full {
        body = body.child(tt::rich_with(s, f, &format!("{key}/full"), &full.text, &text_opts));
        if !full.files.is_empty() {
            let mut fl = col().w_full().gap(2.0);
            for (i, x) in full.files.iter().enumerate() {
                fl = fl.child(
                    text(plain_line(x))
                        .key(format!("{key}/ff{i}"))
                        .font(ts(11.0, 400.0).mono())
                        .color(k.c.text_muted)
                        .w_full()
                        .wrap_anywhere(),
                );
            }
            body = body.child(fl);
        }
    } else if it.held && ui.released {
        body = body.child(muted("Released. Its text joins the card once it is on its way."));
    } else if it.held && it.dismissed {
        let to = l.arrow.as_deref().map(plain_line).unwrap_or_else(|| "its recipient".into());
        let mut r = row().w_full().gap(8.0).center_y().flex_wrap().child(
            muted(&format!("Held, then dismissed: it never reached {to}, and its text isn't on the card.")).shrink(),
        );
        if !it.delivery_id.is_empty() {
            let (rk2, did) = (rk.clone(), it.delivery_id.clone());
            r = r.child(
                Button::new("Release")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Mini)
                    .disabled(ui.busy)
                    .on_press(move |s: &mut State| release(s, &rk2, &did))
                    .build(k),
            );
        }
        body = body.child(r);
    } else if it.held {
        body = body.child(muted("Not let through the wake policy yet, so its text isn't on the card."));
    } else if edit == EDIT_EDITING {
        body = body.child(edit_form(s, f, &key, &o.card_id, it, ui, &seen));
    } else {
        let t = plain_block(&it.text);
        if !t.is_empty() {
            body = body.child(tt::rich_with(s, f, &format!("{key}/text"), &t, &text_opts));
        }
    }
    if !it.missing.is_empty() {
        let mut m = col().w_full().gap(2.0).child(text("Still needed:").font(k.ty.text_xsmall).color(k.c.text_muted));
        for (i, x) in it.missing.iter().enumerate() {
            m = m.child(
                row()
                    .w_full()
                    .gap(6.0)
                    .pl(12.0)
                    .child(text("•").font(ts(12.5, 400.0)).color(k.c.text_muted))
                    .child(tt::rich_line(s, f, &format!("{key}/miss{i}"), x, &RichOpts { font: Some(ts(12.5, 400.0)), ..RichOpts::default() }).grow(1.0).min_w(0.0)),
            );
        }
        body = body.child(m);
    }
    if !it.attachments.is_empty() {
        body = body.child(tt::attachment_list(s, f, &format!("{key}/files"), &File::all(&it.attachments), false));
    }
    for (i, w) in l.waiting.iter().enumerate() {
        body = body.child(text(plain_line(w)).key(format!("{key}/wait{i}")).font(ts(11.0, 400.0)).color(k.c.text_muted).w_full().wrap());
    }
    let card = o.card_id.clone();
    if !card.is_empty() && edit == EDIT_CAN {
        let (rk2, cid, iid, t) = (rk.clone(), card.clone(), it.id.clone(), it.text.clone());
        let seen2 = seen.clone();
        body = body.child(
            row().child(
                Button::new("Modify")
                    .variant(ButtonVariant::GhostMuted)
                    .control_size(ControlSize::Mini)
                    .disabled(ui.busy)
                    .help("No agent has read it yet: take it back to edit it")
                    .on_press(move |s: &mut State| {
                        let r = row_mut(s, &rk2);
                        r.draft = TextEdit::new(&t);
                        let (cid, iid) = (cid.clone(), iid.clone());
                        edit_action(s, &rk2, &seen2, EDIT_EDITING, move |w| w.comment_modify(&cid, &iid));
                    })
                    .build(k),
            ),
        );
    }
    if startable && !card.is_empty() && !ui.started {
        let (rk2, cid) = (rk.clone(), card.clone());
        let brief = ui.full.as_ref().map(|m| m.text.clone()).unwrap_or_else(|| it.text.clone());
        body = body.child(
            row().child(
                Button::new("Start an agent with this")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Mini)
                    .disabled(ui.busy)
                    .help("Nobody is on this card: start an agent, with this comment in its brief")
                    .on_press(move |s: &mut State| {
                        let r = row_mut(s, &rk2);
                        r.busy = true;
                        r.error = None;
                        let (cid, brief, rk3) = (cid.clone(), brief.clone(), rk2.clone());
                        s.spawn(
                            move |w| w.start_agent(&cid, &brief),
                            move |s, res| {
                                let r = row_mut(s, &rk3);
                                r.busy = false;
                                match res {
                                    Ok(()) => r.started = true,
                                    Err(e) => r.error = Some(e),
                                }
                            },
                        );
                    })
                    .build(k),
            ),
        );
    }
    if ui.started {
        body = body.child(text("Starting an agent with this comment in its brief.").font(ts(11.0, 400.0)).color(k.c.text_muted));
    }
    if ui.full.is_none() && !card.is_empty() && opens_in_full(it) {
        let (rk2, cid, iid) = (rk.clone(), card.clone(), it.id.clone());
        body = body.child(
            row().child(
                Button::new(if ui.busy { "Opening…" } else { "Show the whole message" })
                    .variant(ButtonVariant::GhostMuted)
                    .control_size(ControlSize::Mini)
                    .disabled(ui.busy)
                    .on_press(move |s: &mut State| {
                        let r = row_mut(s, &rk2);
                        if r.busy {
                            return;
                        }
                        r.busy = true;
                        r.error = None;
                        let (cid, iid, rk3) = (cid.clone(), iid.clone(), rk2.clone());
                        s.spawn(
                            move |w| w.message(&cid, &iid),
                            move |s, res| {
                                let r = row_mut(s, &rk3);
                                r.busy = false;
                                match res {
                                    Ok(m) => r.full = Some(m),
                                    Err(e) => r.error = Some(e),
                                }
                            },
                        );
                    })
                    .build(k),
            ),
        );
    }
    if let Some(e) = &ui.error {
        body = body.child(err_line(f, e.clone()));
    }
    base.child(body)
}

/// Runs a comment edit step (Modify, Cancel, Save); `after` is the edit
/// state it leaves the comment in.
fn edit_action(
    s: &mut State,
    rk: &str,
    seen: &str,
    after: &'static str,
    work: impl FnOnce(&irori_window::Window) -> Result<(), String> + Send + 'static,
) {
    let r = row_mut(s, rk);
    if r.busy {
        return;
    }
    r.busy = true;
    r.error = None;
    let (rk, seen) = (rk.to_string(), seen.to_string());
    s.spawn(work, move |s, res| {
        let r = row_mut(s, &rk);
        r.busy = false;
        match res {
            Ok(()) => r.shown = Some((after.to_string(), seen)),
            Err(e) => r.error = Some(e),
        }
    });
}

fn release(s: &mut State, rk: &str, delivery: &str) {
    let r = row_mut(s, rk);
    if r.busy {
        return;
    }
    r.busy = true;
    r.error = None;
    let (rk, did) = (rk.to_string(), delivery.to_string());
    s.spawn(
        move |w| w.inbox_release(&did),
        move |s, res| {
            let r = row_mut(s, &rk);
            r.busy = false;
            match res {
                Ok(()) => r.released = true,
                Err(e) => r.error = Some(e),
            }
        },
    );
}

/// The person's comment no agent read yet, held back while they edit it
/// (card #105): Save sends the new text, Cancel lets it go as it was.
fn edit_form(s: &State, f: &mut Frame, key: &str, card: &str, it: &CardThreadItem, ui: &RowUi, seen: &str) -> El {
    let k = f.k;
    let rk = row_key(card, &it.id);
    let save = {
        let (rk, cid, iid, seen) = (rk.clone(), card.to_string(), it.id.clone(), seen.to_string());
        move |s: &mut State| {
            let t = row_mut(s, &rk).draft.text.clone();
            if t.trim().is_empty() {
                row_mut(s, &rk).error = Some("Write a comment first.".into());
                return;
            }
            let (cid, iid) = (cid.clone(), iid.clone());
            edit_action(s, &rk, &seen, EDIT_CAN, move |w| w.comment_modify_save(&cid, &iid, &t).map(|_| ()));
        }
    };
    let cancel = {
        let (rk, cid, iid, seen) = (rk.clone(), card.to_string(), it.id.clone(), seen.to_string());
        move |s: &mut State| {
            let (cid, iid) = (cid.clone(), iid.clone());
            edit_action(s, &rk, &seen, EDIT_CAN, move |w| w.comment_modify_cancel(&cid, &iid));
        }
    };
    let draft = if s.card.rows.get(&rk).is_some_and(|r| !r.draft.text.is_empty()) {
        ui.draft.clone()
    } else {
        TextEdit::new(&it.text)
    };
    let rk_e = rk.clone();
    let save2 = save.clone();
    let cancel2 = cancel.clone();
    let area = TextArea::new(&draft)
        .label("Edit your comment")
        .auto_grow(3, 10)
        .on_edit(move |s: &mut State| {
            let r = row_mut(s, &rk_e);
            &mut r.draft
        })
        .on_submit(move |s: &mut State, _tx: &mut Tx| save2(s))
        .build(k)
        .key(format!("{key}/edit"))
        .on_key(move |s: &mut State, ev: &rune::KeyEvent, _tx: &mut Tx| {
            if ev.key == rune::Key::Escape {
                cancel2(s);
                return true;
            }
            false
        });
    col()
        .w_full()
        .gap(6.0)
        .child(area)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .flex_wrap()
                .child(
                    Button::new("Save")
                        .variant(ButtonVariant::Submit)
                        .control_size(ControlSize::Mini)
                        .disabled(ui.busy)
                        .on_press(save)
                        .build(k),
                )
                .child(
                    Button::new("Cancel")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Mini)
                        .disabled(ui.busy)
                        .on_press(cancel)
                        .build(k),
                )
                .child(
                    text("Held back: no agent gets it until you save or cancel.")
                        .font(k.ty.text_xsmall)
                        .color(k.c.text_muted)
                        .shrink()
                        .wrap(),
                ),
        )
}
