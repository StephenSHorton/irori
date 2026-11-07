//! Who's on it (card-detail.tsx People): working agents with their live
//! dot, then watchers, each with its device; an agent Irori started with
//! its Attach, Logs, Stop and End; a gone agent's Revive; and the waits
//! (WaitRow).

use irori_window::cards::{AgentRef, CardDetail, CardSummary, CardWait};
use irori_window::roster::GoneSession;
use rune::{El, col, el, row, text};

use super::{note, ts};
use crate::logic::agent_status::{Who, agent_status, ms, now_ms};
use crate::logic::cards::{self as lc, PeopleRow, Role, Tone, Work};
use crate::logic::names::display_name;
use crate::logic::plain::short_path;
use crate::ui::{Frame, State};
use crate::widgets::{bits, confirm, faces, spawned_actions};

/// One wait as Who's on it says it (card #87).
pub fn wait_words(w: &CardWait) -> String {
    if w.on_this {
        format!("waits {}", w.text)
    } else {
        format!("waiting on {}", w.text)
    }
}

const NOBODY: &str = "Nobody yet. An agent that takes the card shows here.";

pub fn people(s: &State, f: &mut Frame, d: &CardDetail, now_: &CardSummary) -> El {
    let rows: Vec<PeopleRow> = lc::people_rows(&now_.working, &now_.watching, &d.waits, |id| {
        d.steps.iter().find(|x| x.id == id).map(|x| x.text.clone())
    });
    if rows.is_empty() {
        return note(f, NOBODY);
    }
    let mut out = col().key("card/people").w_full().gap(6.0);
    if !rows.iter().any(|r| r.role != Role::Waiting) {
        out = out.child(note(f, NOBODY));
    }
    for r in &rows {
        out = out.child(if r.role == Role::Waiting {
            wait_row(s, f, &r.waits[0], &r.waits[1..])
        } else {
            person_row(s, f, d, now_, r)
        });
    }
    out
}

fn face_cell(s: &State, f: &mut Frame, key: &str, who: &AgentRef, watching: bool) -> El {
    el().w(22.0).center().child(faces::face(
        s,
        f,
        key,
        who,
        faces::FaceOpts {
            watching,
            no_dot: true,
            ..Default::default()
        },
    ))
}

/// Name · where, with offline and conflict chips.
fn name_line(f: &Frame, key: &str, who: &AgentRef, tail: &str) -> El {
    let k = f.k;
    let name = display_name(who);
    let line = format!("{name} {tail}");
    let mut r = row().grow(1.0).min_w(0.0).gap(6.0).center_y().child(
        rune::rich(
            line,
            vec![
                rune::Span::new(name.len(), name.len() + 1 + tail.len())
                    .size(12.0)
                    .color(k.c.text_muted),
            ],
        )
        .font(ts(13.0, 400.0))
        .color(k.c.text_base)
        .shrink()
        .min_w(0.0)
        .max_lines(1),
    );
    if who.offline {
        r = r.child(bits::badge(
            f,
            &format!("{key}/offline"),
            "offline",
            Tone::Slate,
            false,
            "",
        ));
    }
    if !who.conflict.is_empty() {
        r = r.child(bits::badge(
            f,
            &format!("{key}/conflict"),
            "conflict",
            Tone::Warning,
            false,
            &format!("Two devices leased {} while apart", who.conflict),
        ));
    }
    r
}

fn person_row(s: &State, f: &mut Frame, d: &CardDetail, now_: &CardSummary, r: &PeopleRow) -> El {
    let k = f.k;
    let w = &r.who;
    let key = format!("card/person-{}", w.key);
    let small = |t: String| text(t).font(k.ty.text_xsmall).color(k.c.text_muted);
    let mut right = row().gap(8.0).center_y();
    if w.status == "gone" {
        right = right.child(small("gone".into()));
        if !w.spawned
            && let Some(rv) = revive(s, f, &format!("{key}/revive"), w, &s.card.gone)
        {
            right = right.child(rv);
        }
    } else if r.role == Role::Working && !r.step_titles.is_empty() {
        let more = if r.step_titles.len() > 1 {
            format!(" +{}", r.step_titles.len() - 1)
        } else {
            String::new()
        };
        let tip = format!(
            "Owns {}: on the card until they're done",
            r.step_titles
                .iter()
                .map(|t| format!("“{t}”"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        right = right.child(
            small(format!("on {}{more}", r.step_titles[0]))
                .max_w(224.0)
                .max_lines(1)
                .help(tip),
        );
    } else if r.role == Role::Working {
        if w.assigned && w.status == crate::logic::agent_counts::NOT_ANSWERING {
            // Card #699: not a slow agent about to take it up.
            right = right.child(
                text("isn't answering")
                    .font(k.ty.text_xsmall)
                    .color(k.c.warning.fg),
            );
        } else if w.assigned && w.offline {
            right = right.child(
                text("its device is offline")
                    .font(k.ty.text_xsmall)
                    .color(k.c.warning.fg),
            );
        } else if w.assigned {
            right = right.child(if w.waiting {
                text("waiting for it")
                    .font(k.ty.text_xsmall)
                    .color(k.c.warning.fg)
                    .help("The hand-over is still queued or held: it hasn't reached this agent")
            } else {
                let t = small("offered, not taken up yet".into());
                if w.handoff.is_empty() {
                    t
                } else {
                    t.help(w.handoff.clone())
                }
            });
        } else if let Some(p) = person_status(s, f, now_, w) {
            right = right.child(p);
        }
    } else {
        right = right.child(small("watching".into()));
    }
    let top = row()
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .center_y()
        .child(face_cell(
            s,
            f,
            &format!("{key}/face"),
            w,
            r.role == Role::Watching,
        ))
        .child(name_line(f, &key, w, &format!("· {}", w.where_)))
        .child(right);
    let mut c = col().key(key.clone()).w_full().gap(4.0).child(top);
    if r.role == Role::Working && w.spawned && !w.assigned && w.status != "gone" {
        // Card #456: an agent Irori started for this card.
        let mut sub = col()
            .w_full()
            .gap(4.0)
            .pl(30.0)
            .child(small("Irori started it for this card".into()));
        if !lc::over(&d.summary.state) {
            sub = sub.child(spawned_actions::spawned_actions(
                s,
                f,
                &format!("{key}/spawned"),
                w,
                &d.summary.id,
                &spawned_actions::ALL,
            ));
        }
        c = c.child(sub);
    }
    if r.role == Role::Working && w.assigned && !w.handoff.is_empty() {
        c = c.child(
            text(format!("Hand-off note: {}", w.handoff))
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted)
                .pl(30.0)
                .w_full()
                .wrap_anywhere(),
        );
    }
    for x in &r.waits {
        c = c.child(
            small(wait_words(x))
                .pl(30.0)
                .w_full()
                .wrap()
                .help("Irori wakes this agent once when it happens"),
        );
    }
    c
}

/// A holder's word, by the card's spinner rule: "working" with its live
/// dot while the card spins for it; nothing once over; else its status.
fn person_status(s: &State, f: &Frame, card: &CardSummary, who: &AgentRef) -> Option<El> {
    let k = f.k;
    if lc::busy(&Work::from(card)) && lc::agent_busy(who) {
        return Some(
            row()
                .gap(6.0)
                .center_y()
                .child(faces::live_dot())
                .child(text("working").font(k.ty.text_xsmall).color(k.c.text_muted)),
        );
    }
    if lc::over(&card.state) {
        return None;
    }
    let since = s
        .data
        .agents
        .iter()
        .find(|a| a.card.key == who.key)
        .and_then(|a| ms(&a.card.status_since));
    let t = agent_status(&Who::from(who), since, now_ms()).text;
    Some(
        text(if t.is_empty() { "on it".into() } else { t })
            .font(k.ty.text_xsmall)
            .color(k.c.text_muted),
    )
}

/// One wait (card #87): an agent waiting on this card, or a holder
/// waiting on other work. Irori wakes it once when it fires.
fn wait_row(s: &State, f: &mut Frame, w: &CardWait, more: &[CardWait]) -> El {
    let k = f.k;
    let key = format!("card/wait-{}", w.id);
    let words = std::iter::once(w)
        .chain(more)
        .map(wait_words)
        .collect::<Vec<_>>()
        .join("; ");
    row()
        .key(key.clone())
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .center_y()
        .help("Irori wakes this agent once when it happens")
        .child(face_cell(s, f, &format!("{key}/face"), &w.who, true))
        .child(name_line(f, &key, &w.who, &format!("· {words}")))
        .child(text("waiting").font(k.ty.text_xsmall).color(k.c.text_muted))
}

/// Revive: `claude --bg --resume` for a gone agent's session, after a
/// confirm. Only sessions Irori still lists as gone, with a transcript.
fn revive(s: &State, f: &Frame, key: &str, who: &AgentRef, gone: &[GoneSession]) -> Option<El> {
    let g = gone.iter().find(|x| x.key == who.key && x.transcript)?;
    if s.card.ui.revived.contains(&who.key) {
        return Some(
            text("revived")
                .font(f.k.ty.text_xsmall)
                .color(f.k.c.text_muted),
        );
    }
    let mut q = format!(
        "Run claude --bg --resume for {} in {}?",
        display_name(who),
        short_path(&g.cwd)
    );
    if !g.trusted {
        q.push_str("\n\nClaude Code may refuse: this folder isn't marked trusted.");
    }
    let (sid, wk) = (g.session_id.clone(), who.key.clone());
    Some(confirm::confirm_button(
        s,
        f,
        key,
        "Revive",
        None,
        false,
        confirm::Ask::new(q, "Revive", move |w| w.revive(&sid).map(|_| ()))
            .rich()
            .done(move |s| {
                s.card.ui.revived.insert(wk.clone());
            }),
    ))
}
