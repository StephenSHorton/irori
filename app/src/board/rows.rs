//! Home's agent progress (progress-rows.tsx, card #261): one compact row
//! per card with its state (Needs you, Stuck, Working, Waiting, Idle,
//! Done), the line that says why, its space only when it isn't yours, and
//! its progress. Grouped (card #487), one headed list per state; a card
//! that follows another hangs under it (card #570).

use std::collections::{HashMap, HashSet};

use irori_window::cards::CardSummary;
use rune::{El, col, el, row, text};

use super::{heading, live_map};
use crate::logic::agent_status::now_ms;
use crate::logic::card_status::card_ref;
use crate::logic::cards::{Tone, percent, with_live};
use crate::logic::later::follows_of;
use crate::logic::needs::{RowLine, RowState, group_rows, row_state};
use crate::logic::plain::plain_line;
use crate::ui::theme::status;
use crate::ui::{Frame, State};
use crate::widgets::bits::{badge, pr_chip, spinner, tone_solid};

fn tone(s: RowState) -> Tone {
    match s {
        RowState::Needs => Tone::Destructive,
        RowState::Stuck | RowState::Idle => Tone::Warning,
        RowState::Working => Tone::Info,
        RowState::Waiting | RowState::Later => Tone::Slate,
        RowState::Done => Tone::Success,
    }
}

/// The rows of `cards` (grouped by state when `grouped`), in a panel each.
pub fn progress_rows(
    s: &State,
    f: &mut Frame,
    key: &str,
    label: &str,
    cards: &[&CardSummary],
    grouped: bool,
) -> El {
    if cards.is_empty() {
        return el().key(key.to_string());
    }
    let live = live_map(s);
    let now = now_ms();
    // The live roster's status over the card's (card #317).
    let all: Vec<(&CardSummary, RowLine)> = cards
        .iter()
        .map(|c| {
            let r = match with_live(&c.working, &live) {
                Some(w) => row_state(
                    &CardSummary {
                        working: w,
                        ..(*c).clone()
                    },
                    now,
                ),
                None => row_state(c, now),
            };
            (*c, r)
        })
        .collect();
    let by_id: HashMap<&str, usize> = all
        .iter()
        .enumerate()
        .map(|(i, (c, _))| (c.id.as_str(), i))
        .collect();
    let under = |id: &str| -> Vec<usize> {
        let Some(&i) = by_id.get(id) else {
            return vec![];
        };
        follows_of(id, all[i].0.order.as_ref())
            .iter()
            .filter_map(|st| by_id.get(st.id.as_str()).copied())
            .collect()
    };
    let hung: HashSet<usize> = (0..all.len()).flat_map(|i| under(&all[i].0.id)).collect();
    let rows: Vec<usize> = (0..all.len()).filter(|i| !hung.contains(i)).collect();
    // The ref column fits the widest ref (4.5 to 10rem).
    let ref_w = all
        .iter()
        .map(|(c, _)| card_ref(c).chars().count() + 2)
        .max()
        .unwrap_or(4) as f32
        * 7.2;
    let ref_w = ref_w.clamp(72.0, 160.0);

    let list = |f: &mut Frame, name: &str, rs: &[usize], quiet: bool| -> El {
        let k = f.k;
        let mut ul = col()
            .key(format!("{key}-list-{name}"))
            .role("list")
            .label(format!("{label}: {name}"))
            .w_full()
            .min_w(0.0)
            .radius(12.0)
            .clip_rounded(12.0)
            .fill(k.c.layer_01)
            .border(1.0, k.c.border_muted);
        let n = rs.len();
        for (j, &i) in rs.iter().enumerate() {
            let (c, r) = &all[i];
            let mut li = col().key(format!("{key}-li-{}", c.id)).w_full();
            if j + 1 < n {
                li = li.border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_muted.alpha(0.6));
            }
            li = li.child(progress_row(s, f, c, r, ref_w, quiet, false));
            for h in under(&c.id) {
                let (hc, hr) = &all[h];
                li = li.child(progress_row(s, f, hc, hr, ref_w, true, true));
            }
            ul = ul.child(li);
        }
        ul
    };
    if !grouped {
        return col()
            .key(key.to_string())
            .w_full()
            .child(list(f, label, &rows, false));
    }
    let mut out = col().key(key.to_string()).w_full().min_w(0.0).gap(12.0);
    for (st, rs) in group_rows(&rows, |&i| all[i].1.state) {
        if st == RowState::Later {
            let g = list(f, "Not yet", &rs, true);
            out = out.child(super::order::not_yet_fold(
                s,
                f,
                &format!("{key}-notyet"),
                rs.len(),
                g,
            ));
        } else {
            out = out.child(
                col()
                    .key(format!("{key}-group-{}", st.label()))
                    .w_full()
                    .gap(6.0)
                    .child(heading(
                        f,
                        &format!("{key}-h-{}", st.label()),
                        st.label(),
                        Some(rs.len()),
                    ))
                    .child(list(f, st.label(), &rs, false)),
            );
        }
    }
    out
}

fn progress_row(
    s: &State,
    f: &mut Frame,
    c: &CardSummary,
    r: &RowLine,
    ref_w: f32,
    quiet: bool,
    hangs: bool,
) -> El {
    let k = f.k;
    let p = format!("board-row-{}", c.id);
    let line = if hangs { String::new() } else { r.line.clone() };
    let pct = percent(c.progress.done, c.progress.total);
    let space = if c.scope == "board" {
        plain_line(&c.scope_label)
    } else {
        String::new()
    };
    let selected = s.board.selected == c.id;
    let title = plain_line(&c.title);
    let warn = matches!(r.state, RowState::Idle | RowState::Stuck);
    let mut why = row()
        .key(format!("{p}/why"))
        .w(256.0)
        .shrink()
        .min_w(0.0)
        .gap(6.0)
        .center_y();
    if r.spin {
        why = why.child(spinner(f, &format!("{p}/spin")));
    }
    why = why.child(
        text(line.clone())
            .font(rune::tok::TextStyle::new(12.5, 400.0))
            .color(if warn {
                status::decide()
            } else {
                k.c.text_muted
            })
            .max_lines(1)
            .shrink()
            .help(line),
    );
    if let Some(pr) = pr_chip(f, &format!("{p}/pr"), c) {
        why = why.child(pr);
    }
    let mut ttl = row()
        .key(format!("{p}/title"))
        .grow(1.0)
        .min_w(0.0)
        .gap(8.0)
        .center_y()
        .help(title.clone())
        .child(
            text(title.clone())
                .font(rune::tok::TextStyle::new(14.0, 500.0))
                .color(k.c.text_base)
                .max_lines(1)
                .shrink(),
        );
    if !space.is_empty() {
        ttl = ttl.child(
            text(space)
                .font(k.ty.text_xsmall)
                .color(k.c.text_muted)
                .max_lines(1),
        );
    }
    let badge_el = if quiet {
        el()
    } else {
        badge(
            f,
            &format!("{p}/state"),
            r.state.label(),
            tone(r.state),
            false,
            "",
        )
    };
    let bar = match pct {
        Some(pc) => el()
            .key(format!("{p}/bar"))
            .role("progressbar")
            .label(format!("{} progress", card_ref(c)))
            .w(72.0)
            .h(4.0)
            .radius(2.0)
            .clip_rounded(2.0)
            .fill(if quiet {
                rune::Color::TRANSPARENT
            } else {
                k.c.layer_03
            })
            .child(el().w_frac(pc as f32 / 100.0).h_full().fill(match r.state {
                RowState::Waiting | RowState::Later => k.c.text_muted,
                st => tone_solid(f, tone(st)),
            })),
        None => el().w(72.0),
    };
    let reff = card_ref(c);
    let id = c.id.clone();
    row()
        .key(p.clone())
        .role("button")
        .label(title)
        .focusable(true)
        .w_full()
        .min_w(0.0)
        .gap(12.0)
        .px(16.0)
        .py(if hangs { 6.0 } else { 10.0 })
        .center_y()
        .fill(if selected {
            k.c.hover
        } else {
            rune::Color::TRANSPARENT
        })
        .hover(k.c.hover)
        .opacity(if quiet { 0.7 } else { 1.0 })
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut State| s.open_card(&id))
        .child(
            text(if hangs {
                format!("└ {reff}")
            } else {
                reff.clone()
            })
            .font(k.ty.text_xsmall.mono())
            .color(k.c.text_muted)
            .w(ref_w)
            .max_lines(1)
            .help(reff),
        )
        .child(ttl)
        .child(why)
        .child(row().w(88.0).justify_end().child(badge_el))
        .child(bar)
}
