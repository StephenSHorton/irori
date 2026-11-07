//! The card page's side column ("About the card"): Progress (the bar or
//! who holds it, the figures, subtasks, steps, where the work is and where
//! its holder left off), Who's on it, Review, Pull requests, Links and
//! Files.

use std::collections::HashSet;

use irori_window::cards::{CardDetail, CardPR, CardReview, CardSubtask, CardSummary};
use rune::{El, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use super::{note, panel, section, ts};
use crate::logic::agent_status::{ms, now_ms};
use crate::logic::cards::{self as lc, Tone};
use crate::logic::names::display_name;
use crate::logic::plain::{plain_line, relative_time};
use crate::logic::thread::is_wait;
use crate::logic::wait_name::{Wait, wait_label};
use crate::text::{self as tt, File};
use crate::ui::{Frame, State, icons};
use crate::widgets::{bits, faces};

/// Every file on the card, once each: its blockers', their threads' and
/// the thread's. The side lists them by name; pictures show where posted.
pub fn card_files(d: &CardDetail) -> Vec<File> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut add = |fs: Vec<File>| {
        for f in fs {
            if seen.insert(f.sha256.clone()) {
                out.push(f);
            }
        }
    };
    for b in &d.blockers {
        add(File::all(&b.attachments));
        for it in &b.thread {
            add(File::all(&it.attachments));
        }
    }
    for it in &d.thread {
        add(File::all(&it.attachments));
    }
    out
}

/// The side's links: those not already on screen (a PR's, an open
/// blocker's or one of its options').
fn side_links(d: &CardDetail) -> Vec<(String, String, String)> {
    let mut shown: HashSet<String> = d.prs.iter().map(|p| format!("url:{}", p.url)).collect();
    for b in d.blockers.iter().filter(|b| b.resolved.is_none()) {
        if !is_wait(b) {
            for l in &b.links {
                shown.insert(format!("{}:{}", l.kind, l.reference));
            }
        }
        for o in &b.options {
            if let Some(l) = &o.link {
                shown.insert(format!("{}:{}", l.kind, l.reference));
            }
        }
    }
    d.links
        .iter()
        .filter(|l| !shown.contains(&format!("{}:{}", l.kind, l.reference)))
        .map(|l| (l.kind.clone(), l.reference.clone(), l.label.clone()))
        .collect()
}

pub fn view(s: &State, f: &mut Frame, d: &CardDetail, now_: &CardSummary) -> El {
    let c = &d.summary;
    let links = side_links(d);
    let files = card_files(d);
    let spinning = lc::busy(&lc::Work::from(now_));
    let mut p = panel(f, "card/side").label("About the card").clip_rounded(12.0);

    // Progress.
    let mut prog = Vec::new();
    match &c.ask {
        Some(a) if lc::open_ask(Some(a)) => prog.push(bits::ask_line(f, "card/side-ask", a)),
        _ => match lc::percent(c.progress.done, c.progress.total) {
            None => prog.push(bits::holder_line(s, f, "card/side-holder", c)),
            Some(_) => prog.extend(bits::progress_bar(f, "card/side-bar", c)),
        },
    }
    prog.extend(progress_line(f, "card/side-line", d));
    if !d.subtask_list.is_empty() {
        prog.push(subtasks(s, f, &d.subtask_list));
        if !d.steps.is_empty() {
            let open = s.card.ui.old_steps;
            let mut fold = col().key("card/old-steps").w_full().gap(6.0).child(
                text(format!("{} Steps done before the split", if open { "▾" } else { "▸" }))
                    .font(f.k.ty.text_xsmall)
                    .color(f.k.c.text_muted)
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|s: &mut State| s.card.ui.old_steps = !s.card.ui.old_steps),
            );
            if open {
                fold = fold.child(steps(f, d, false));
            }
            prog.push(fold);
        }
    } else if !d.steps.is_empty() {
        prog.push(steps(f, d, spinning));
    }
    prog.extend(where_lines(f, d));
    p = p.child(section(f, "card/side-progress", "Progress", false, prog));

    let last_people = d.reviews.is_empty() && d.prs.is_empty() && links.is_empty() && files.is_empty();
    let people = super::people::people(s, f, d, now_);
    p = p.child(section(f, "card/side-people", "Who's on it", last_people, vec![people]));

    if !d.reviews.is_empty() {
        let mut g = col().w_full().gap(8.0);
        for (i, r) in d.reviews.iter().enumerate() {
            g = g.child(review_row(s, f, &format!("card/review{i}"), r));
        }
        p = p.child(section(f, "card/side-review", "Review", d.prs.is_empty() && links.is_empty() && files.is_empty(), vec![g]));
    }
    if !d.prs.is_empty() {
        let mut g = col().w_full().gap(8.0);
        for (i, pr) in d.prs.iter().enumerate() {
            g = g.child(pr_row(f, &format!("card/pr{i}"), pr));
        }
        p = p.child(section(f, "card/side-prs", "Pull requests", links.is_empty() && files.is_empty(), vec![g]));
    }
    if !links.is_empty() {
        let mut g = row().w_full().gap(6.0).flex_wrap();
        for (i, (kind, r, label)) in links.iter().enumerate() {
            g = g.child(tt::link_chip(f, &format!("card/link{i}"), kind, r, label));
        }
        p = p.child(section(f, "card/side-links", "Links", files.is_empty(), vec![g]));
    }
    if !files.is_empty() {
        let list = tt::attachment_list(s, f, "card/side-file-list", &files, true).w_full();
        p = p.child(section(
            f,
            "card/side-files",
            "Files",
            true,
            vec![list],
        ));
    }
    p
}

/// The figures under the bar: how far along, and what it waits on.
pub fn progress_line(f: &Frame, key: &str, d: &CardDetail) -> Option<El> {
    let k = f.k;
    let c = &d.summary;
    let mut parts: Vec<(String, rune::Color)> = Vec::new();
    if let Some(p) = lc::percent(c.progress.done, c.progress.total) {
        parts.push((
            format!(
                "{} of {} {} · {p}%",
                c.progress.done,
                c.progress.total,
                if c.subtasks > 0 { "subtasks done" } else { "steps" }
            ),
            k.c.text_muted,
        ));
    }
    if !c.subtask_blocked.is_empty() && c.open_blockers == 0 {
        parts.push((format!("blocked by {}", plain_line(&c.subtask_blocked)), k.c.warning.fg));
    }
    if c.open_blockers > 0 {
        parts.push((format!("{} waiting on a decision", c.open_blockers), k.c.danger.fg));
    }
    if c.waiting_on > 0 && c.later.is_none() {
        let l = wait_label(&Wait::from(c));
        let l = match l.strip_prefix("Waiting") {
            Some(rest) => format!("waiting{rest}"),
            None => l,
        };
        parts.push((l, k.c.text_muted));
    }
    if c.waiting > 0 {
        parts.push((format!("{} waiting on an agent", c.waiting), k.c.text_muted));
    }
    if parts.is_empty() {
        return None;
    }
    let mut line = String::new();
    let mut spans = Vec::new();
    for (i, (t, color)) in parts.into_iter().enumerate() {
        if i > 0 {
            line.push_str(" · ");
        }
        spans.push(rune::Span::new(line.len(), line.len() + t.len()).color(color));
        line.push_str(&t);
    }
    Some(rune::rich(line, spans).key(key.to_string()).font(k.ty.text_xsmall).color(k.c.text_muted).w_full().wrap())
}

/// The card's checklist: done ones ticked, the current one marked (the
/// spinner while an agent works on it), a step's owner named.
fn steps(f: &Frame, d: &CardDetail, spinning: bool) -> El {
    let mut l = col().key("card/steps").w_full().gap(6.0);
    for st in &d.steps {
        let owner = if st.owner.is_empty() {
            String::new()
        } else {
            d.summary
                .working
                .iter()
                .find(|w| w.key == st.owner)
                .map(display_name)
                .unwrap_or_else(|| st.owner.clone())
        };
        l = l.child(step_row(f, &format!("card/step-{}", st.id), StepView { text: &st.text, done: st.done, doing: st.doing }, spinning, false, &owner));
    }
    l
}

/// A step's words and state (coord's Step, which the app can't name).
pub struct StepView<'a> {
    pub text: &'a str,
    pub done: bool,
    pub doing: bool,
}

/// One step: its mark and words.
pub fn step_row(f: &Frame, key: &str, st: StepView, spinning: bool, strip: bool, owner: &str) -> El {
    let k = f.k;
    let current = st.doing && !st.done;
    let mark = if current && spinning {
        bits::spinner(f, &format!("{key}/spin"))
    } else if st.done {
        el().size(14.0, 14.0)
            .fill(k.c.success.fg)
            .center()
            .child(text("✓").font(ts(10.0, 700.0)).color(k.c.bg_base))
    } else if st.doing {
        el().size(14.0, 14.0)
            .border(1.0, k.c.text_base.alpha(0.7))
            .center()
            .child(el().size(6.0, 6.0).fill(k.c.text_accent))
    } else {
        el().size(14.0, 14.0).border(1.0, k.c.text_muted.alpha(0.5))
    };
    let words = plain_line(st.text);
    let (line, spans) = if owner.is_empty() {
        (words.clone(), vec![])
    } else {
        let w = format!("{words}  · {owner}");
        (w, vec![rune::Span::new(words.len(), words.len() + owner.len() + 4).size(12.0).color(k.c.text_muted)])
    };
    let font = if current || strip { ts(13.0, 500.0) } else { ts(13.0, 400.0) };
    let color = if current || strip {
        k.c.text_base
    } else if st.done {
        k.c.text_muted
    } else {
        k.c.text_base
    };
    let state = if st.done {
        "Done: "
    } else if st.doing {
        "In progress: "
    } else {
        "To do: "
    };
    row()
        .key(key.to_string())
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .items_start()
        .label(format!("{state}{words}"))
        .child(el().w(16.0).pt(2.0).child(mark))
        .child(rune::rich(line, spans).font(font).color(color).grow(1.0).min_w(0.0).wrap())
}

/// A parent's subtasks: each with its ref, title, state, holder and
/// progress, opening its page on a click.
fn subtasks(s: &State, f: &Frame, list: &[CardSubtask]) -> El {
    let k = f.k;
    let mut l = col().key("card/subtasks").w_full().gap(4.0).label("Subtasks");
    for st in list {
        let over = lc::over(&st.state);
        let id = st.id.clone();
        let p = lc::percent(st.progress.done, st.progress.total);
        let live = s
            .data
            .agents
            .iter()
            .find(|a| a.card.key == st.holder.who.key)
            .map(|a| (a.card.status.as_str(), ms(&a.card.status_since)));
        let h = lc::holder_line(&st.holder, None, live, now_ms());
        let title = plain_line(&st.title);
        let mut sub = row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(text(h.text).font(ts(11.0, 400.0)).color(if h.warn { k.c.warning.fg } else { k.c.text_muted }).shrink().max_lines(1));
        if let Some(p) = p {
            sub = sub.child(text(format!("{p}%")).font(ts(11.0, 400.0)).color(k.c.text_muted));
        }
        l = l.child(
            col()
                .key(format!("card/subtask-{}", st.id))
                .w_full()
                .gap(2.0)
                .px(6.0)
                .py(4.0)
                .radius(4.0)
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| s.open_card(&id))
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(text(lc::ref_text(&st.tag, &st.short)).font(k.ty.text_xsmall.mono()).color(k.c.text_muted))
                        .child({
                            let t = text(if title.is_empty() { "(untitled)".into() } else { title.clone() })
                                .font(ts(13.0, 400.0))
                                .color(if over { k.c.text_muted } else { k.c.text_base })
                                .grow(1.0)
                                .min_w(0.0)
                                .max_lines(1);
                            if over { t.strike(0, title.len().max(10), None) } else { t }
                        })
                        .child(
                            text(if st.blocked { "BLOCKED".to_string() } else { st.state.replace('_', " ").to_uppercase() })
                                .font(ts(11.0, 400.0).tracking(0.4))
                                .color(if st.blocked { k.c.danger.fg } else { k.c.text_muted }),
                        ),
                )
                .child(sub.pl(30.0)),
        );
    }
    l
}

/// Where the card's work is and where its holder left off (card #137).
fn where_lines(f: &Frame, d: &CardDetail) -> Option<El> {
    let k = f.k;
    let c = &d.summary;
    if c.where_.is_none() && c.left_off.is_none() {
        return None;
    }
    let now = now_ms();
    let mut out = col()
        .key("card/where")
        .w_full()
        .gap(6.0)
        .pt(10.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_base);
    if let Some(l) = &c.left_off {
        let lead = "Left off: ";
        let body = plain_line(&l.text);
        let tail = format!(" · {}, {}", display_name(&l.by), ms(&l.at).map(|t| relative_time(t, now)).unwrap_or_default());
        let line = format!("{lead}{body}{tail}");
        let spans = vec![
            rune::Span::new(0, lead.len()).weight(500.0).color(k.c.text_base),
            rune::Span::new(lead.len() + body.len(), line.len()).color(k.c.text_muted.alpha(0.7)),
        ];
        out = out.child(rune::rich(line, spans).font(k.ty.text_xsmall).color(k.c.text_muted).w_full().wrap());
    }
    if let Some(w) = &c.where_ {
        let branch = plain_line(&w.branch);
        let mut r = row()
            .w_full()
            .gap(6.0)
            .center_y()
            .flex_wrap()
            .child(text("Where:").font(ts(12.0, 500.0)).color(k.c.text_base))
            .child(
                text(if branch.is_empty() { "detached".into() } else { branch })
                    .font(k.ty.text_xsmall.mono())
                    .color(k.c.text_base),
            );
        if !w.head.is_empty() {
            r = r.child(
                text(format!("@ {}", plain_line(&w.head).chars().take(7).collect::<String>()))
                    .font(k.ty.text_xsmall.mono())
                    .color(k.c.text_muted),
            );
        }
        r = r.child(bits::badge(
            f,
            "card/where-pushed",
            if w.pushed { "Pushed" } else { "Not pushed" },
            if w.pushed { Tone::Success } else { Tone::Warning },
            false,
            "",
        ));
        let device = plain_line(&w.who.where_);
        out = out
            .child(r)
            .child(
                text(plain_line(&w.worktree))
                    .font(ts(11.0, 400.0).mono())
                    .color(k.c.text_muted)
                    .help(plain_line(&w.worktree))
                    .w_full()
                    .max_lines(1),
            )
            .child(
                text(format!(
                    "on {} · {} · seen {}",
                    if device.is_empty() { "this device".into() } else { device },
                    display_name(&w.who),
                    ms(&w.at).map(|t| relative_time(t, now)).unwrap_or_default()
                ))
                .font(ts(11.0, 400.0))
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
            );
    }
    Some(out)
}

/// One reviewer's review of a private card (#88).
fn review_row(s: &State, f: &mut Frame, key: &str, r: &CardReview) -> El {
    let k = f.k;
    let (label, tone) = match r.state.as_str() {
        "approved" => ("approved", Tone::Success),
        "changes" => ("changes asked", Tone::Warning),
        _ => ("requested", Tone::Info),
    };
    let when = ms(&r.at).map(|t| relative_time(t, now_ms())).unwrap_or_default();
    let mut sub = format!(" · asked by {}", display_name(&r.asked_by));
    if !r.pr.is_empty() {
        sub.push_str(&format!(" · {}", plain_line(&r.pr)));
    }
    let name = display_name(&r.reviewer);
    let line = format!("{name}{sub}");
    let face = faces::face(s, f, &format!("{key}/face"), &r.reviewer, faces::FaceOpts { no_dot: true, ..Default::default() });
    let mut c = col()
        .key(key.to_string())
        .w_full()
        .gap(6.0)
        .px(12.0)
        .py(10.0)
        .radius(8.0)
        .border(1.0, k.c.border_base)
        .fill(k.c.bg_base.alpha(0.4))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(el().w(22.0).child(face))
                .child(
                    rune::rich(line, vec![rune::Span::new(name.len(), name.len() + sub.len()).size(12.0).color(k.c.text_muted)])
                        .font(ts(13.0, 400.0))
                        .color(k.c.text_base)
                        .grow(1.0)
                        .min_w(0.0)
                        .max_lines(1),
                )
                .child(bits::badge(f, &format!("{key}/state"), label, tone, false, &when)),
        );
    if !r.text.is_empty() {
        c = c.child(text(r.text.clone()).font(k.ty.text_xsmall).color(k.c.text_muted).w_full().wrap().selectable(true));
    }
    c
}

/// How GitHub last said the PR stands (card #208).
fn pr_state_chip(f: &Frame, key: &str, pr: &CardPR) -> Option<El> {
    let (tone, t) = match pr.state.as_str() {
        "MERGED" => (Tone::Success, "Merged"),
        "CLOSED" => (Tone::Slate, "Closed"),
        "OPEN" => match pr.review.as_str() {
            "CHANGES_REQUESTED" => (Tone::Warning, "Changes requested"),
            "APPROVED" => (Tone::Info, "Open · approved"),
            _ => (Tone::Info, "Open"),
        },
        _ => return None,
    };
    Some(bits::badge(f, key, t, tone, false, ""))
}

/// One linked pull request and who worked on it.
fn pr_row(f: &Frame, key: &str, pr: &CardPR) -> El {
    let k = f.k;
    let url = pr.url.clone();
    let label = plain_line(&pr.label);
    let mut head = row()
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .center_y()
        .child(text(format!("#{}", pr.number)).font(k.ty.text_xsmall.mono()).color(k.c.text_muted))
        .child(
            text(plain_line(&pr.repo))
                .font(ts(13.0, 500.0))
                .color(k.c.text_base)
                .help(plain_line(&pr.repo))
                .shrink()
                .min_w(0.0)
                .max_lines(1),
        );
    if let Some(c) = pr_state_chip(f, &format!("{key}/state"), pr) {
        head = head.child(c);
    }
    head = head.child(el().grow(1.0)).child(
        Button::new(if label.is_empty() { format!("#{}", pr.number) } else { label })
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Mini)
            .icon(&icons::EXTERNAL_LINK)
            .help(pr.url.clone())
            .on_press_tx(move |_: &mut State, tx: &mut Tx| {
                let _ = tx.open_url(&url);
            })
            .build(k),
    );
    let roles = [("Opened by", &pr.opened_by), ("Reviewed by", &pr.reviewed_by), ("Also touched by", &pr.touched_by)];
    let known: Vec<String> = roles
        .iter()
        .filter(|(_, w)| !w.is_empty())
        .map(|(l, w)| format!("{l} {}", w.iter().map(display_name).collect::<Vec<_>>().join(", ")))
        .collect();
    let mut c = col()
        .key(key.to_string())
        .w_full()
        .gap(6.0)
        .px(12.0)
        .py(10.0)
        .radius(8.0)
        .border(1.0, k.c.border_base)
        .fill(k.c.bg_base.alpha(0.4))
        .child(head)
        .child(
            text(if known.is_empty() { "No agent here has worked on it yet.".into() } else { known.join("   ") })
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    if !pr.error.is_empty() {
        c = c.child(note(f, plain_line(&pr.error)));
    }
    c
}
