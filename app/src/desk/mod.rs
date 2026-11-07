//! Merge desks (P9): the Rune port of the React app's
//! components/desk/desk-page.tsx and lib/desk.ts (on main, under
//! frontend/src/). Every desk down the left, and the open desk's queue:
//! every open PR of ours on GitHub with its state and author card, for
//! review. The desk never merges (card #418): each agent asks the person
//! "Can I merge PR #N?" on its own card. A desk with none of ours open is
//! left off the list (card #625).

pub mod queue;

use irori_window::desk::DeskView;
use irori_window::events::EVENT_BOARD_CHANGED;
use rune::{El, Icon, ScrollSpec, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};

use crate::inbox::notify::Tone;
use crate::inbox::words;
use crate::ui::{Dialog, Event, Frame, Route, State, icons};

use queue::{
    DeskState, QueueLine, desk_on, desk_queue, desk_shown, inputs_of, owner_name, queue_count,
    state_label,
};

macro_rules! lucide {
    ($name:ident, $id:literal, $body:literal) => {
        static $name: Icon = Icon {
            name: concat!("lucide-", $id),
            view: 24.0,
            body: concat!(
                r#"<g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"#,
                $body,
                "</g>"
            ),
        };
    };
}

lucide!(
    PR,
    "git-pull-request",
    r#"<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M13 6h3a2 2 0 0 1 2 2v7"/><line x1="6" x2="6" y1="9" y2="21"/>"#
);
lucide!(
    PR_DRAFT,
    "git-pull-request-draft",
    r#"<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M18 6V5"/><path d="M18 11v-1"/><line x1="6" x2="6" y1="9" y2="21"/>"#
);
lucide!(
    CIRCLE_CHECK,
    "circle-check",
    r#"<circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/>"#
);
lucide!(
    CLOUD_OFF,
    "cloud-off",
    r#"<path d="m2 2 20 20"/><path d="M5.782 5.782A7 7 0 0 0 9 19h8.5a4.5 4.5 0 0 0 1.307-.193"/><path d="M21.532 16.5A4.5 4.5 0 0 0 17.5 10h-1.79A7.008 7.008 0 0 0 10 5.07"/>"#
);

#[derive(Default)]
pub struct Model {
    pub desks: Vec<DeskView>,
    /// The first read has answered (card #598).
    pub loaded: bool,
    /// Opening a PR on GitHub failed.
    pub error: String,
    /// The 30 s re-read runs.
    ticking: bool,
}

/// Reads every desk again.
pub fn load(s: &mut State) {
    s.spawn(
        |w| w.desks(),
        |s, desks| {
            s.desk.desks = desks;
            s.desk.loaded = true;
        },
    );
}

/// The poll's own times live on the polling device only, so the list is
/// read again every half minute while the page shows.
fn start_ticking(s: &mut State) {
    if s.desk.ticking || s.headless {
        return;
    }
    let Some(h) = s.handle.clone() else { return };
    s.desk.ticking = true;
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(30));
            h.update(|s: &mut State, _| {
                if matches!(s.nav.route, Route::Desk(_)) && s.nav.card.is_none() {
                    load(s);
                }
            });
        }
    });
}

pub fn on_event(s: &mut State, ev: &Event) {
    match ev.name.as_str() {
        "app:start" => load(s),
        // The desk rows are coord rows, so a board change may be one.
        EVENT_BOARD_CHANGED => load(s),
        _ => {}
    }
    if s.handle.is_some() {
        start_ticking(s);
    }
}

/// The headless scene that shows it (`irori --snap x.png --scene desk`).
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Desk(None));
    load(s);
}

/// Names an agent key (the roster's), for a PR's author.
fn name_of(s: &State, key: &str) -> String {
    s.data
        .agents
        .iter()
        .find(|a| a.card.key == key)
        .map(|a| words::name_or(&a.card.name, words::UNKNOWN_AGENT))
        .unwrap_or_else(|| words::UNKNOWN_AGENT.to_string())
}

fn settings(s: &mut State) {
    s.open_dialog(Dialog::Settings("desks".into()));
}

pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let open = match &s.nav.route {
        Route::Desk(Some(r)) => r.as_str(),
        _ => "",
    };
    let on: Vec<&DeskView> = s.desk.desks.iter().filter(|d| desk_shown(d)).collect();
    // A link to a desk with none of ours left opens it anyway, with its
    // calm empty queue.
    let desk = s
        .desk
        .desks
        .iter()
        .find(|d| desk_on(d) && !open.is_empty() && d.settings.repo_key == open)
        .or_else(|| on.first().copied());
    let page = col()
        .key("desk-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(crate::ui::page_header(f, "Merge desks", vec![]));
    let Some(desk) = desk else {
        let body = if !s.desk.loaded {
            text("Loading the merge desks…")
                .font(k.ty.ui)
                .color(k.c.text_muted)
        } else {
            col()
                .key("desk-none")
                .w_full()
                .max_w(640.0)
                .gap(8.0)
                .padding(20.0)
                .radius(10.0)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_muted)
                .child(text("No open PRs right now").font(k.ty.body_medium).color(k.c.text_base))
                .child(
                    text("A merge desk lists a repo's open PRs from GitHub for review. It never merges: each agent asks you “Can I merge PR #N?” on its own card. A desk shows here while you or your people have a PR open in a repo you work in.")
                        .font(k.ty.ui)
                        .color(k.c.text_muted)
                        .w_full()
                        .wrap(),
                )
                .child(
                    row().child(
                        Button::new("Merge desk settings")
                            .variant(ButtonVariant::Submit)
                            .on_press(settings)
                            .build(k)
                            .key("desk-settings-empty"),
                    ),
                )
        };
        return page.child(col().w_full().px(20.0).child(body));
    };

    let mut main = row()
        .key("desk-main")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(20.0)
        .gap(16.0);
    if on.len() > 1 {
        let mut nav = col()
            .key("desk-nav")
            .role("navigation")
            .label("Merge desks")
            .w(220.0)
            .gap(2.0);
        for d in &on {
            let here = std::ptr::eq(*d, desk);
            let repo = d.settings.repo_key.clone();
            let n = d.prs.as_ref().map(|p| p.prs.len());
            let mut b = col()
                .key(format!("desk-nav-{repo}"))
                .role("button")
                .label(owner_name(&repo))
                .focusable(true)
                .w_full()
                .px(10.0)
                .py(6.0)
                .radius(6.0)
                .fill(if here { k.c.layer_02 } else { k.c.bg_base })
                .on_press({
                    let repo = repo.clone();
                    move |s: &mut State| s.go(Route::Desk(Some(repo.clone())))
                })
                .child(
                    text(owner_name(&repo))
                        .font(k.ty.ui.mono())
                        .color(if here { k.c.text_base } else { k.c.text_muted })
                        .max_lines(1)
                        .w_full(),
                );
            if let Some(n) = n {
                b = b.child(
                    text(format!("{n} open"))
                        .font(k.ty.small)
                        .color(k.c.text_muted),
                );
            }
            nav = nav.child(b);
        }
        main = main.child(
            nav.scroll(ScrollSpec {
                stick_bottom: false,
            })
            .min_h(0.0),
        );
    }
    let header = row()
        .key("desk-head")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(icon(&icons::MERGE, 20.0).color(k.c.icon_base))
        .child(
            text(owner_name(&desk.settings.repo_key))
                .font(rune::tok::TextStyle::new(17.0, 600.0).mono())
                .color(k.c.text_base),
        )
        .child(el().grow(1.0))
        .child(
            IconButton::new(&icons::SETTINGS)
                .variant(IconButtonVariant::Ghost)
                .label("Merge desk settings")
                .help("Merge desk settings")
                .on_press(settings)
                .build(k)
                .key("desk-settings"),
        );
    let content = col()
        .key("desk-content")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .gap(12.0)
        .pb(24.0)
        .child(header)
        .child(panel(s, f, desk))
        .child(
            text("Order: Ready for review oldest first, then the rest. Each PR's thread is its author's card. The desk never merges: each agent asks you before it merges its own PR.")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        )
        .scroll(ScrollSpec { stick_bottom: false });
    page.child(main.child(content))
}

/// The state chip (StateChip).
fn chip(f: &Frame, key: String, st: DeskState) -> El {
    let k = f.k;
    let label = state_label(st);
    match st {
        DeskState::Ready => crate::inbox::tone_badge(k, key, label, Tone::Secondary),
        DeskState::Running => crate::inbox::tone_badge(k, key, label, Tone::Info),
        DeskState::Rebase => crate::inbox::tone_badge(k, key, label, Tone::Warning),
        DeskState::Failed => crate::inbox::tone_badge(k, key, label, Tone::Destructive),
        DeskState::Merged => crate::inbox::tone_badge(k, key, label, Tone::Success),
        DeskState::Draft => row()
            .key(key)
            .h(18.0)
            .px(6.0)
            .radius(4.0)
            .center()
            .border(1.0, k.c.border_base)
            .child(text(label).font(k.ty.small_medium).color(k.c.text_muted)),
    }
}

/// One desk's queue (DeskPanel): never blank. Before the first poll it says
/// so; with no open PRs of ours it says so and when it last asked; when
/// GitHub can't be asked it keeps the last list.
fn panel(s: &State, f: &Frame, desk: &DeskView) -> El {
    let k = f.k;
    let now = words::now_ms();
    let lines = desk_queue(&inputs_of(desk));
    let snap = desk.prs.as_ref();
    let repo = owner_name(&desk.settings.repo_key);
    let poll = desk.poll.here.then_some(&desk.poll);
    let last_poll = poll
        .map(|p| &p.last_poll_at)
        .filter(|t| !t.is_zero())
        .or(snap.map(|s| &s.at));
    let down = poll.map(|p| &p.unreachable_since).filter(|t| !t.is_zero());
    let count = match (snap, down) {
        (Some(sn), Some(_)) => format!("{} open at {}", sn.prs.len(), words::clock(&sn.at)),
        (Some(_), None) => format!(
            "{} · polled {} (every 3 min)",
            queue_count(desk),
            last_poll.map_or_else(|| "never".into(), |t| words::ago(t, now))
        ),
        (None, _) => queue_count(desk),
    };
    let head = row()
        .key("desk-panel-head")
        .w_full()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .center_y()
        .child(text("Queue").font(k.ty.small_medium).color(k.c.text_base))
        .child(text(count).font(k.ty.small).color(k.c.text_muted).shrink());
    let mut p = col()
        .key(format!("desk-panel-{}", desk.settings.repo_key))
        .w_full()
        .radius(12.0)
        .fill(k.c.layer_01)
        .border(
            1.0,
            if down.is_some() {
                k.c.warning.border
            } else {
                k.c.border_muted
            },
        )
        .child(head)
        .child(el().w_full().h(1.0).fill(k.c.border_muted));
    if let Some(d) = down {
        let shown = snap.map_or(String::new(), |sn| {
            format!("Showing the {} list; ", words::clock(&sn.at))
        });
        p = p.child(
            row()
                .key("desk-down")
                .role("status")
                .w_full()
                .gap(8.0)
                .px(12.0)
                .pt(10.0)
                .items_start()
                .child(icon(&CLOUD_OFF, 16.0).color(k.c.warning.fg))
                .child(
                    text(format!(
                        "GitHub unreachable since {}. {shown}never an empty queue while GitHub can't be asked.",
                        words::clock(d)
                    ))
                    .font(k.ty.small)
                    .color(k.c.warning.fg)
                    .shrink()
                    .wrap(),
                ),
        );
    }
    if let Some(sn) = snap
        && sn.truncated
    {
        p = p.child(
            text(format!(
                "Showing {} of ours; GitHub has more open PRs than it listed.",
                sn.prs.len()
            ))
            .font(k.ty.small)
            .color(k.c.warning.fg)
            .px(12.0)
            .pt(10.0)
            .w_full()
            .wrap(),
        );
    }
    if !s.desk.error.is_empty() {
        p = p.child(
            text(s.desk.error.clone())
                .key("desk-error")
                .role("alert")
                .font(k.ty.small)
                .color(k.c.danger.fg)
                .px(12.0)
                .pt(10.0)
                .w_full()
                .wrap(),
        );
    }
    if snap.is_none() {
        let t = if desk.settings.poll_device.is_empty() {
            "No device polls GitHub for this desk: pick one in Settings › Merge desks."
        } else {
            "Waiting for the first poll of GitHub's open PRs."
        };
        p = p.child(
            col()
                .w_full()
                .px(16.0)
                .py(24.0)
                .center()
                .child(text(t).font(k.ty.ui).color(k.c.text_muted).wrap()),
        );
    } else if lines.is_empty() {
        p = p.child(
            col()
                .key("desk-calm")
                .w_full()
                .px(16.0)
                .py(24.0)
                .gap(4.0)
                .center()
                .child(icon(&CIRCLE_CHECK, 24.0).color(k.c.success.icon))
                .child(
                    text(format!("No open PRs of yours on {repo}"))
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base),
                )
                .child(
                    text(format!(
                        "Last poll {} · the desk shows here again once you or your people open one",
                        last_poll.map_or_else(|| "—".into(), words::clock)
                    ))
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .wrap(),
                ),
        );
    } else {
        let mut list = col().key("desk-lines").w_full().gap(2.0).padding(6.0);
        for l in &lines {
            list = list.child(line_row(s, f, l));
        }
        p = p.child(list);
    }
    p
}

/// One PR (QueueLineRow): its number (to GitHub), title, author, author
/// card, and state.
fn line_row(s: &State, f: &Frame, l: &QueueLine) -> El {
    let k = f.k;
    let dim = matches!(l.state, DeskState::Draft | DeskState::Merged);
    let key = format!(
        "{}{}",
        if l.state == DeskState::Merged {
            "m"
        } else {
            "o"
        },
        l.pr
    );
    let ic = match l.state {
        DeskState::Draft => icon(&PR_DRAFT, 16.0).color(k.c.icon_muted),
        DeskState::Merged => icon(&icons::MERGE, 16.0).color(k.c.success.icon),
        _ => icon(&PR, 16.0).color(k.c.success.icon),
    };
    let num = text(format!("#{}", l.pr))
        .font(k.ty.small.mono())
        .color(k.c.text_accent)
        .w(44.0);
    let num = if l.url.is_empty() {
        num
    } else {
        let url = l.url.clone();
        num.key(format!("desk-pr-{key}"))
            .role("link")
            .focusable(true)
            .help("Open on GitHub")
            .on_press_tx(move |s: &mut State, tx| {
                s.desk.error.clear();
                if let Err(e) = tx.open_url(&url) {
                    s.desk.error = e.to_string();
                }
            })
    };
    let agent = if l.author.agent.is_empty() {
        String::new()
    } else {
        name_of(s, &l.author.agent)
    };
    let who = if !agent.is_empty() && agent != words::UNKNOWN_AGENT {
        agent
    } else if !l.login.is_empty() {
        format!("@{}", l.login)
    } else {
        agent
    };
    let mut top = row()
        .w_full()
        .gap(8.0)
        .px(8.0)
        .py(4.0)
        .center_y()
        .opacity(if dim { 0.6 } else { 1.0 })
        .child(ic)
        .child(num)
        .child(
            text(l.title.clone())
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .max_lines(1)
                .grow(1.0)
                .shrink()
                .min_w(0.0),
        )
        .child(text(who).font(k.ty.small).color(k.c.text_muted));
    if !l.author.card.is_empty() && !l.author.card_ref.is_empty() {
        let card = l.author.card.clone();
        top = top.child(
            text(l.author.card_ref.clone())
                .key(format!("desk-card-{key}"))
                .role("button")
                .focusable(true)
                .help("Open the author's card (the PR's thread)")
                .font(k.ty.small.mono())
                .color(k.c.text_accent)
                .px(4.0)
                .radius(3.0)
                .fill(k.c.layer_02)
                .on_press(move |s: &mut State| s.open_card(&card)),
        );
    }
    top = top.child(row().w(132.0).child(el().grow(1.0)).child(chip(
        f,
        format!("desk-chip-{key}"),
        l.state,
    )));
    col()
        .key(format!("desk-line-{key}"))
        .w_full()
        .radius(6.0)
        .child(top)
        .child(
            text(l.note.clone())
                .font(k.ty.small)
                .color(k.c.text_muted)
                .max_lines(1)
                .pl(76.0)
                .pr(8.0)
                .pb(2.0)
                .w_full(),
        )
}
