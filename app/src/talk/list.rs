//! Talk's list (talk-page.tsx `TalkPage`'s nav, `Group`, `ConvItem`,
//! `BroadcastItem`, `ThreadItem`, `NewChannel`, `NewDm`): the search, then
//! Threads, Channels, Direct messages, Agents talking, the Archive, and the
//! resolved threads folded at its foot.

use std::collections::HashMap;

use irori_window::cards::AgentRef;
use irori_window::talk::{TalkConversation, TalkMemberRef};
use irori_window::threads::{THREAD_HANDED, THREAD_RESOLVED, THREAD_WAITING, ThreadSummary};
use rune::tok::TextStyle;
use rune::{El, Key, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, TextInput};

use super::logic::{self, AgentRow, Broadcast, SectionKey};
use super::{Adding, folded, icons, open, toggle_fold};
use crate::logic::agent_counts::{agents_of, why_waiting};
use crate::logic::agent_status::{Who, agent_status, now_ms};
use crate::logic::names::{member_name, name_or, UNKNOWN_AGENT};
use crate::logic::plain::{plain_line, relative_time};
use crate::logic::spaces::{conv_in_space, talk_in_space, threads_space};
use crate::logic::thread::{LiveAgent, live_roster};
use crate::ui::{Frame, State};
use crate::widgets::faces::{FaceOpts, face};

const WIDTH: f32 = 260.0;

pub(crate) fn ts(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight)
}

/// The list column.
pub fn view(s: &State, f: &mut Frame, open_id: &str) -> El {
    let k = f.k;
    let space = &super::picked_space(s);
    let query = s.talk.query.text.clone();
    let searching = !query.trim().is_empty();
    let live = live_roster(&s.data.agents);
    let now = now_ms();
    let fold = |key: SectionKey| !searching && folded(s, key);

    let threads: Vec<&ThreadSummary> = s
        .talk
        .threads
        .iter()
        .filter(|t| conv_in_space(&t.space_id, "thread", &[], space))
        .collect();
    let shown: Vec<&ThreadSummary> = threads.iter().copied().filter(|t| logic::talk_matches(t, &query)).collect();
    let live_threads: Vec<&ThreadSummary> = shown.iter().copied().filter(|t| t.state != THREAD_RESOLVED).collect();
    let resolved: Vec<&ThreadSummary> = shown.iter().copied().filter(|t| t.state == THREAD_RESOLVED).collect();
    let pairs: Vec<(&str, &str)> = threads.iter().map(|t| (t.id.as_str(), t.state.as_str())).collect();
    let show_resolved = logic::resolved_shown(s.talk.show_resolved, open_id, &pairs) || searching;
    let spawned: Vec<_> = s.data.agents.iter().filter(|a| !a.spawned).cloned().collect();
    let why = why_waiting(&agents_of(&spawned), "");

    let convs: Vec<&TalkConversation> = s
        .talk
        .convs
        .iter()
        .filter(|c| talk_in_space(c, space) && logic::conv_matches(c, &query))
        .collect();
    let (channels, dms) = logic::listed(&convs);
    let gone = |c: &TalkConversation| logic::dm_gone(c, &live);
    let keep = kept_for(s, open_id, &live);
    let sections = logic::dm_sections(&dms, gone, &keep);

    let mut nav = col()
        .key("talk-list")
        .w(WIDTH)
        .h_full()
        .min_h(0.0)
        .px(8.0)
        .py(12.0)
        .gap(2.0)
        .scroll(ScrollSpec { stick_bottom: false });

    // The search and the pen.
    nav = nav.child(
        row()
            .key("talk-search")
            .w_full()
            .px(4.0)
            .gap(6.0)
            .center_y()
            .child(
                TextInput::new(&s.talk.query)
                    .placeholder("Search talk")
                    .label("Search talk")
                    .leading_icon(&crate::ui::icons::SEARCH)
                    .height(30.0)
                    .width(None)
                    .on_edit(|s: &mut State| &mut s.talk.query)
                    .build(k)
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(
                IconButton::new(&icons::SQUARE_PEN)
                    .variant(IconButtonVariant::Ghost)
                    .label("New thread")
                    .help("Start a thread: ask the room about anything")
                    .on_press(|s: &mut State| open(s, ""))
                    .build(k),
            ),
    );

    // Threads.
    let unread = live_threads.iter().filter(|t| logic::needs_you(&t.state, &t.no_agent)).count();
    nav = nav.child(group(
        f,
        "talk-g-threads",
        "Threads",
        Some(live_threads.len()),
        unread,
        fold(SectionKey::Threads),
        (!searching).then_some(SectionKey::Threads),
        None,
        None,
    ));
    if !fold(SectionKey::Threads) {
        if !s.talk.threads_loaded {
            nav = nav.child(crate::widgets::loading::list_rows_shape(f, "talk-threads-loading", 3, true));
        } else if live_threads.is_empty() {
            nav = nav.child(empty(
                f,
                "talk-threads-empty",
                if searching { "No open thread matches." } else { "No open threads. Start one with the pen above." },
            ));
        }
        for t in &live_threads {
            nav = nav.child(thread_item(f, t, t.id == open_id, &why));
        }
    }

    // Channels.
    let extra = (s.talk.adding == Adding::Channel).then(|| new_channel(s, f));
    nav = conv_section(
        s,
        f,
        nav,
        SectionKey::Channels,
        "Channels",
        &channels,
        if searching { "No channel matches." } else { "No channels yet. Make one with +." },
        extra,
        Some(("New channel", Adding::Channel)),
        open_id,
        searching,
        &live,
        now,
    );
    // Direct messages.
    let extra = (s.talk.adding == Adding::Dm).then(|| new_dm(s, f));
    nav = conv_section(
        s,
        f,
        nav,
        SectionKey::Dms,
        "Direct messages",
        &sections.yours,
        if searching { "No direct message matches." } else { "No direct messages. Start one with +." },
        extra,
        Some(("New direct message", Adding::Dm)),
        open_id,
        searching,
        &live,
        now,
    );
    if !sections.agents.is_empty() {
        nav = agents_section(s, f, nav, &sections.agents, open_id, searching, &live, now);
    }
    if !sections.gone.is_empty() {
        nav = conv_section(
            s,
            f,
            nav,
            SectionKey::Gone,
            "Archive",
            &sections.gone,
            "",
            None,
            None,
            open_id,
            searching,
            &live,
            now,
        );
    }

    // Resolved threads, folded at the foot.
    if !resolved.is_empty() {
        let label = format!("Resolved threads: {}{}", resolved.len(), if show_resolved { " · hide" } else { " · show" });
        let pairs_owned: Vec<(String, String)> =
            threads.iter().map(|t| (t.id.clone(), t.state.clone())).collect();
        let open_now = open_id.to_string();
        let mut foot = col().key("talk-resolved").w_full().gap(2.0).pt(12.0).child(
            row()
                .key("talk-resolved-toggle")
                .role("button")
                .focusable(true)
                .px(8.0)
                .py(2.0)
                .radius(4.0)
                .hover(k.c.hover)
                .on_press(move |s: &mut State| {
                    let pairs: Vec<(&str, &str)> = pairs_owned.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                    let (pick, close) = logic::resolved_toggle(s.talk.show_resolved, &open_now, &pairs);
                    s.talk.show_resolved = pick;
                    if close {
                        open(s, "");
                    }
                })
                .child(text(label).font(ts(11.0, 400.0)).color(k.c.text_muted)),
        );
        if show_resolved {
            for t in &resolved {
                foot = foot.child(thread_item(f, t, t.id == open_id, &why));
            }
        }
        nav = nav.child(el().key("talk-list-spacer").grow(1.0)).child(foot);
    }

    col()
        .key("talk-list-col")
        .w(WIDTH)
        .h_full()
        .min_h(0.0)
        .border_sides([0.0, 1.0, 0.0, 0.0], k.c.border_muted)
        .fill(k.c.bg_base)
        .child(nav.grow(1.0))
}

/// The open DM kept with yours while open, though its agent went (card
/// #473); one opened from the Archive stays there.
fn kept_for(s: &State, open_id: &str, live: &HashMap<String, LiveAgent>) -> String {
    if s.talk.kept.0 == open_id
        && let Some(k) = &s.talk.kept.1
    {
        return k.clone();
    }
    match s.talk.convs.iter().find(|c| c.conv_id == open_id) {
        Some(c) if c.kind == "dm" && !c.read_only && !logic::dm_gone(c, live) => open_id.to_string(),
        _ => String::new(),
    }
}

/// Settles which DM is kept, once the list has the open one: called on
/// each open.
pub fn keep_open(s: &mut State) {
    let id = super::open_id(s).to_string();
    if s.talk.kept.0 == id && s.talk.kept.1.is_some() {
        return;
    }
    let live = live_roster(&s.data.agents);
    let keep = s.talk.convs.iter().find(|c| c.conv_id == id).map(|c| {
        if c.kind == "dm" && !c.read_only && !logic::dm_gone(c, &live) { id.clone() } else { String::new() }
    });
    s.talk.kept = (id, keep);
}

fn empty(f: &Frame, key: &str, what: &str) -> El {
    row()
        .key(key.to_string())
        .w_full()
        .px(8.0)
        .py(4.0)
        .child(text(what.to_string()).font(ts(12.0, 400.0)).color(f.k.c.text_muted).w_full().wrap())
}

/// One section's name: its chevron (folds it), count, the "N new" pill
/// while folded, and its + or Mark all read.
#[allow(clippy::too_many_arguments)]
fn group(
    f: &Frame,
    key: &str,
    label: &str,
    count: Option<usize>,
    unread: usize,
    is_folded: bool,
    toggle: Option<SectionKey>,
    add: Option<(&'static str, Adding)>,
    mark: Option<Vec<TalkConversation>>,
) -> El {
    let k = f.k;
    let muted = k.c.text_muted;
    let mut name = row().key(format!("{key}-name")).gap(4.0).center_y().min_w(0.0);
    if toggle.is_some() {
        name = name.child(
            icon(if is_folded { &icons::CHEVRON_RIGHT } else { &icons::CHEVRON_DOWN }, 12.0).color(muted),
        );
    }
    name = name.child(
        text(label.to_uppercase())
            .font(ts(11.0, 500.0))
            .color(muted)
            .max_lines(1)
            .shrink(),
    );
    if let Some(n) = count.filter(|n| *n > 0) {
        name = name.child(text(format!("· {n}")).font(ts(11.0, 400.0)).color(muted));
    }
    if is_folded && unread > 0 {
        name = name.child(
            row()
                .key(format!("{key}-new"))
                .h(16.0)
                .px(6.0)
                .radius(8.0)
                .center_y()
                .fill(k.c.icon_accent)
                .help(format!("{unread} with something new"))
                .child(text(format!("{unread} new")).font(ts(10.0, 600.0)).color(k.c.white)),
        );
    }
    if let Some(sk) = toggle {
        name = name
            .role("button")
            .focusable(true)
            .radius(4.0)
            .expanded(!is_folded)
            .on_press(move |s: &mut State| toggle_fold(s, sk));
    }
    let mut h = row().key(key.to_string()).w_full().px(8.0).pt(12.0).pb(4.0).gap(4.0).center_y().child(name);
    if let Some((what, which)) = add {
        h = h.child(el().grow(1.0)).child(
            IconButton::new(&crate::ui::icons::PLUS)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(rune_kit::ControlSize::Mini)
                .label(what)
                .help(what)
                .on_press(move |s: &mut State| {
                    s.talk.adding = if s.talk.adding == which { Adding::None } else { which };
                    s.talk.new_error.clear();
                    s.talk.new_channel.clear();
                    if s.talk.adding == Adding::Channel {
                        s.talk.new_channel.focused = true;
                    }
                    if s.talk.adding == Adding::Dm {
                        s.spawn(|w| w.talk_dm_people(), |s, p| s.talk.people = p);
                    }
                })
                .build(k),
        );
    }
    if let Some(items) = mark {
        h = h.child(el().grow(1.0)).child(
            row()
                .key(format!("{key}-mark"))
                .role("button")
                .focusable(true)
                .px(4.0)
                .radius(4.0)
                .hover(k.c.hover)
                .help("Mark every conversation here read")
                .on_press(move |s: &mut State| mark_all(s, items.clone()))
                .child(text("Mark all read").font(ts(10.0, 400.0)).color(muted)),
        );
    }
    h
}

/// The Archive's Mark all read (card #473): each unread one read up to its
/// newest post; one that fails stays unread.
fn mark_all(s: &mut State, items: Vec<TalkConversation>) {
    if s.talk.marking {
        return;
    }
    s.talk.marking = true;
    s.spawn(
        move |w| {
            for c in items.iter().filter(|c| !c.read_only && c.unread > 0) {
                if let Ok(ps) = w.talk_posts(&c.conv_id, None, logic::POSTS_PAGE)
                    && let Some(last) = ps.last()
                {
                    let _ = w.talk_read(&c.conv_id, &last.post_id);
                }
            }
        },
        |s, ()| s.talk.marking = false,
    );
}

#[allow(clippy::too_many_arguments)]
fn conv_section(
    s: &State,
    f: &mut Frame,
    mut nav: El,
    k_: SectionKey,
    label: &str,
    items: &[TalkConversation],
    empty_line: &str,
    extra: Option<El>,
    add: Option<(&'static str, Adding)>,
    open_id: &str,
    searching: bool,
    live: &HashMap<String, LiveAgent>,
    now: i64,
) -> El {
    let is_folded = !searching && folded(s, k_);
    let (unread, mark) = logic::section_header(k_, is_folded, items);
    nav = nav.child(group(
        f,
        &format!("talk-g-{}", k_.name()),
        label,
        Some(items.len()),
        unread,
        is_folded,
        (!searching).then_some(k_),
        add,
        (mark && !s.talk.marking).then(|| items.to_vec()),
    ));
    let has_extra = extra.is_some();
    if let Some(x) = extra {
        nav = nav.child(x);
    }
    if is_folded {
        return nav;
    }
    if !s.talk.convs_loaded {
        return nav.child(crate::widgets::loading::list_rows_shape(f, &format!("talk-{}-loading", k_.name()), 3, true));
    }
    // Its own new-channel or new-DM form stands in for the empty line.
    if items.is_empty() && !has_extra && !empty_line.is_empty() {
        nav = nav.child(empty(f, &format!("talk-{}-empty", k_.name()), empty_line));
    }
    let (rows, hidden) =
        logic::section_rows(items, false, open_id, searching || s.talk.all_shown.contains(&k_));
    for c in rows {
        nav = nav.child(conv_item(s, f, c, c.conv_id == open_id, live, now));
    }
    if hidden > 0 {
        nav = nav.child(show_more(f, &format!("talk-{}-more", k_.name()), hidden, k_));
    }
    nav
}

fn show_more(f: &Frame, key: &str, hidden: usize, k_: SectionKey) -> El {
    row()
        .key(key.to_string())
        .role("button")
        .focusable(true)
        .px(8.0)
        .py(2.0)
        .radius(4.0)
        .hover(f.k.c.hover)
        .on_press(move |s: &mut State| {
            s.talk.all_shown.insert(k_);
        })
        .child(text(format!("Show {hidden} more")).font(ts(11.0, 400.0)).color(f.k.c.text_muted))
}

#[allow(clippy::too_many_arguments)]
fn agents_section(
    s: &State,
    f: &mut Frame,
    mut nav: El,
    items: &[TalkConversation],
    open_id: &str,
    searching: bool,
    live: &HashMap<String, LiveAgent>,
    now: i64,
) -> El {
    let all = logic::agent_rows(items);
    let is_folded = !searching && folded(s, SectionKey::Agents);
    let sec = logic::agent_section_rows(
        &all,
        is_folded,
        open_id,
        s.talk.all_shown.contains(&SectionKey::Agents),
        searching,
    );
    nav = nav.child(group(
        f,
        "talk-g-agents",
        "Agents talking",
        Some(all.len()),
        0,
        is_folded,
        (!searching).then_some(SectionKey::Agents),
        None,
        None,
    ));
    for r in &sec.rows {
        nav = nav.child(match r {
            AgentRow::Dm(c) => conv_item(s, f, c, c.conv_id == open_id, live, now),
            AgentRow::Broadcast(b) => broadcast_item(s, f, b, open_id, now),
        });
    }
    if !is_folded {
        if sec.hidden > 0 {
            nav = nav.child(show_more(f, "talk-agents-more", sec.hidden, SectionKey::Agents));
        } else if sec.capped > 0 {
            nav = nav.child(empty(
                f,
                "talk-agents-capped",
                &format!("{} older not listed. Search finds them.", sec.capped),
            ));
        }
    }
    nav
}

/// A row's time beside its name.
fn when(f: &Frame, key: &str, t: &str) -> El {
    text(t.to_string()).key(key.to_string()).font(ts(10.5, 400.0)).color(f.k.c.text_muted)
}

/// A list row's frame: icon, name line and sub line, then the trailing
/// mark; `current` lights it.
#[allow(clippy::too_many_arguments)]
fn item_row(
    f: &Frame,
    key: &str,
    current: bool,
    lead: Option<El>,
    name: El,
    sub: Option<El>,
    trail: Option<El>,
    on: impl Fn(&mut State) + 'static,
) -> El {
    let k = f.k;
    let mut body = col().key(format!("{key}-body")).grow(1.0).min_w(0.0).gap(1.0).child(name);
    if let Some(sb) = sub {
        body = body.child(sb);
    }
    let mut r = row()
        .key(key.to_string())
        .role("button")
        .focusable(true)
        .w_full()
        .px(8.0)
        .py(6.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .hover(k.c.hover)
        .on_press(on);
    if current {
        r = r.fill(k.c.layer_02);
    }
    if let Some(l) = lead {
        r = r.child(l);
    }
    r = r.child(body);
    if let Some(t) = trail {
        r = r.child(t);
    }
    r
}

fn sub_line(f: &Frame, key: &str, s: &str, color: rune::Color) -> El {
    let _ = f;
    text(s.to_string()).key(key.to_string()).font(ts(11.0, 400.0)).color(color).max_lines(1).w_full()
}

fn lead_icon(f: &Frame, i: &'static rune::Icon) -> El {
    icon(i, 16.0).color(f.k.c.text_muted)
}

/// A channel or a DM in the list, with its unread count (a red @ when it
/// mentions you).
pub fn conv_item(
    s: &State,
    f: &mut Frame,
    c: &TalkConversation,
    current: bool,
    live: &HashMap<String, LiveAgent>,
    now: i64,
) -> El {
    let k = f.k;
    let key = format!("talk-c-{}", c.conv_id);
    let other = if c.kind == "dm" { logic::dm_with(c) } else { vec![] };
    let person = logic::dm_person(c);
    let lead = if c.kind == "channel" {
        Some(lead_icon(f, &icons::HASH))
    } else if person.is_some() {
        Some(lead_icon(f, &crate::ui::icons::USER))
    } else if c.read_only {
        Some(lead_icon(f, &icons::EYE))
    } else {
        other.first().map(|m| {
            let st = logic::status_of(m, live);
            let who = AgentRef {
                key: m.key.clone(),
                name: member_name(&m.kind, &m.name),
                live: st != "gone" && st != "unknown",
                status: st,
                ..AgentRef::default()
            };
            row()
                .key(format!("{key}-face"))
                .size(16.0, 16.0)
                .center()
                .child(face(s, f, &format!("{key}-f"), &who, FaceOpts { inline: true, no_hover: true, ..FaceOpts::default() }))
        })
    };
    let label = if c.kind == "channel" {
        plain_line(&c.name)
    } else if let Some(p) = person {
        name_or(&plain_line(if p.name.is_empty() { &c.name } else { &p.name }), UNKNOWN_AGENT)
    } else {
        let names: Vec<String> = other.iter().map(|m| member_name(&m.kind, &m.name)).collect();
        if names.is_empty() { name_or(&c.name, UNKNOWN_AGENT) } else { names.join(" ↔ ") }
    };
    let waiting = if person.is_some() { logic::waiting_text(&c.waiting, &label) } else { String::new() };
    let last = c.kind == "dm"
        && (c.read_only
            || (person.is_some() && waiting.is_empty())
            || other.first().is_some_and(|m| logic::status_of(m, live) == "gone"));
    let (line, at) = if last { logic::last_line(c, now) } else { (String::new(), String::new()) };
    let sub = if c.kind == "channel" {
        c.topic.as_deref().map(plain_line).unwrap_or_default()
    } else if !waiting.is_empty() {
        waiting
    } else if last {
        line
    } else if let Some(m) = other.first() {
        logic::member_status(m, live, now)
    } else {
        String::new()
    };
    let ink = if c.read_only { k.c.text_muted } else { k.c.text_base };
    let mut name = row().key(format!("{key}-n")).w_full().gap(8.0).center_y().child(
        text(label)
            .font(if c.unread > 0 { ts(13.0, 500.0) } else { ts(13.0, 400.0) })
            .color(ink)
            .max_lines(1)
            .grow(1.0)
            .min_w(0.0),
    );
    if !at.is_empty() {
        name = name.child(when(f, &format!("{key}-when"), &at));
    }
    // A huddle on here (#654) shows who's in it in place of the sub line.
    let sub = crate::call::huddle_line(s, f, &c.conv_id)
        .or_else(|| (!sub.is_empty()).then(|| sub_line(f, &format!("{key}-sub"), &sub, k.c.text_muted)));
    let trail = if c.mentions > 0 {
        Some(
            row()
                .key(format!("{key}-at"))
                .size(20.0, 20.0)
                .radius(10.0)
                .center()
                .fill(crate::ui::theme::status::failed())
                .help(format!("{} mentioning you", c.mentions))
                .child(text("@").font(ts(10.0, 600.0)).color(k.c.white)),
        )
    } else if c.unread > 0 && !c.read_only {
        Some(
            row()
                .key(format!("{key}-unread"))
                .h(18.0)
                .px(6.0)
                .radius(9.0)
                .center()
                .fill(k.c.icon_accent)
                .child(text(c.unread.to_string()).font(ts(10.5, 600.0)).color(k.c.white)),
        )
    } else {
        None
    };
    let id = c.conv_id.clone();
    item_row(f, &key, current, lead, name, sub, trail, move |s: &mut State| {
        open(s, &id);
        keep_open(s);
    })
}

/// One message an agent sent to many agents (card #486): "owl → 16
/// agents", its text and when; it unfolds to the DMs it went to.
fn broadcast_item(s: &State, f: &Frame, b: &Broadcast, open_id: &str, now: i64) -> El {
    let k = f.k;
    let key = format!("talk-b-{}", short_hash(&b.id));
    let from = b.convs.first().and_then(|c| logic::member_of(&c.members, &b.by));
    let to = logic::broadcast_to(b);
    let unfolded = s
        .talk
        .unfolded
        .get(&b.id)
        .copied()
        .unwrap_or_else(|| b.convs.iter().any(|c| c.conv_id == open_id));
    let who = from.map_or("an agent".to_string(), |m| member_name(&m.kind, &m.name));
    let name = row()
        .key(format!("{key}-n"))
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(format!("{who} → {} agents", to.len()))
                .font(ts(13.0, 400.0))
                .color(k.c.text_muted)
                .max_lines(1)
                .grow(1.0)
                .min_w(0.0),
        )
        .child(when(f, &format!("{key}-when"), &relative_time(b.last_at, now)));
    let sub = sub_line(f, &format!("{key}-sub"), &plain_line(&b.text), k.c.text_muted);
    let chev = icon(if unfolded { &icons::CHEVRON_DOWN } else { &icons::CHEVRON_RIGHT }, 12.0).color(k.c.text_muted);
    let id = b.id.clone();
    let mut out = col().key(format!("{key}-wrap")).w_full().child(
        item_row(f, &key, false, Some(lead_icon(f, &icons::EYE)), name, Some(sub), Some(chev), move |s: &mut State| {
            s.talk.unfolded.insert(id.clone(), !unfolded);
        })
        .expanded(unfolded),
    );
    if unfolded {
        for (c, m) in b.convs.iter().zip(to.iter()) {
            let current = c.conv_id == open_id;
            let cid = c.conv_id.clone();
            let mut r = row()
                .key(format!("{key}-to-{}", c.conv_id))
                .role("button")
                .focusable(true)
                .w_full()
                .pl(32.0)
                .pr(8.0)
                .py(4.0)
                .radius(6.0)
                .hover(k.c.hover)
                .on_press(move |s: &mut State| open(s, &cid))
                .child(
                    text(format!("→ {}", member_name(&m.kind, &m.name)))
                        .font(ts(12.0, 400.0))
                        .color(if current { k.c.text_base } else { k.c.text_muted })
                        .max_lines(1),
                );
            if current {
                r = r.fill(k.c.layer_02);
            }
            out = out.child(r);
        }
    }
    out
}

/// A short stable id for a key built from text.
fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// A thread in the list: its title and where it stands; a dot while it
/// waits on the person.
fn thread_item(f: &Frame, t: &ThreadSummary, current: bool, why: &str) -> El {
    let k = f.k;
    let key = format!("talk-t-{}", t.id);
    let waiting = t.state == THREAD_WAITING || t.state == THREAD_HANDED;
    let needs = logic::needs_you(&t.state, &t.no_agent);
    let name = text(plain_line(&t.title))
        .key(format!("{key}-n"))
        .font(if needs { ts(13.0, 500.0) } else { ts(13.0, 400.0) })
        .color(k.c.text_base)
        .max_lines(1)
        .w_full();
    let sub = sub_line(
        f,
        &format!("{key}-sub"),
        &logic::state_line(t, why),
        if waiting { crate::ui::theme::status::decide() } else { k.c.text_muted },
    );
    let dot = needs.then(|| {
        el().key(format!("{key}-dot"))
            .size(8.0, 8.0)
            .radius(4.0)
            .fill(k.c.icon_accent)
            .help("Answered: waiting on you")
    });
    let id = t.id.clone();
    item_row(f, &key, current, Some(lead_icon(f, &icons::MESSAGE_TEXT)), name, Some(sub), dot, move |s: &mut State| {
        open(s, &id)
    })
}

/// New channel: its name (lowercase, no spaces or #); it goes in the
/// picked shared space, else Personal (card #461).
fn new_channel(s: &State, f: &Frame) -> El {
    let k = f.k;
    let space = threads_space(&super::picked_space(s)).to_string();
    let mut c = col().key("talk-new-channel").w_full().px(4.0).pb(4.0).gap(4.0).child(
        TextInput::new(&s.talk.new_channel)
            .placeholder("channel-name")
            .label("New channel's name")
            .height(28.0)
            .width(None)
            .on_edit(|s: &mut State| &mut s.talk.new_channel)
            .on_key(|s: &mut State, ev: &rune::KeyEvent, _: &mut Tx| {
                if matches!(ev.key, Key::Escape) {
                    s.talk.adding = Adding::None;
                    return true;
                }
                false
            })
            .on_submit(move |s: &mut State, _: &mut Tx| {
                let clean = logic::channel_name(&s.talk.new_channel.text);
                if clean.is_empty() {
                    return;
                }
                let sp = space.clone();
                s.spawn(
                    move |w| w.talk_channel_create(&clean, None, (!sp.is_empty()).then_some(sp.as_str())),
                    |s, r| match r {
                        Ok(id) => {
                            s.talk.adding = Adding::None;
                            s.talk.new_channel.clear();
                            s.talk.convs_loaded = true;
                            s.spawn(|w| w.talk_conversations(), move |s, cs| {
                                if let Ok(cs) = cs {
                                    s.talk.convs = cs;
                                }
                                open(s, &id);
                            });
                        }
                        Err(e) => s.talk.new_error = e,
                    },
                );
            })
            .build(k),
    );
    if !s.talk.new_error.is_empty() {
        c = c.child(text(s.talk.new_error.clone()).font(ts(11.0, 400.0)).color(crate::ui::theme::status::failed()).w_full().wrap());
    }
    c
}

/// New direct message: a person you share a space with (card #538) or a
/// live agent; the DM opens (or reopens).
fn new_dm(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let mut c = col()
        .key("talk-new-dm")
        .w_full()
        .padding(4.0)
        .gap(2.0)
        .radius(6.0)
        .border(1.0, k.c.border_muted)
        .fill(k.c.layer_01)
        .role("listbox")
        .label("Message someone");
    let label = |t: &str| text(t.to_string()).font(ts(11.0, 500.0)).color(k.c.text_muted);
    if !s.talk.people.is_empty() {
        c = c.child(row().key("talk-dm-people-h").px(8.0).pt(2.0).child(label("People")));
        for p in &s.talk.people {
            let key = p.key.clone();
            c = c.child(
                row()
                    .key(format!("talk-dm-p-{}", p.key))
                    .role("option")
                    .focusable(true)
                    .w_full()
                    .px(8.0)
                    .py(4.0)
                    .gap(8.0)
                    .center_y()
                    .radius(4.0)
                    .hover(k.c.hover)
                    .on_press(move |s: &mut State| start_dm(s, "person", &key))
                    .child(lead_icon(f, &crate::ui::icons::USER))
                    .child(
                        text(name_or(&plain_line(&p.name), UNKNOWN_AGENT))
                            .font(ts(12.5, 400.0))
                            .color(k.c.text_base)
                            .max_lines(1)
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
        }
        c = c.child(row().key("talk-dm-agents-h").px(8.0).pt(4.0).child(label("Agents")));
    }
    let agents: Vec<_> = s.data.agents.iter().filter(|a| a.card.status != "gone").collect();
    if agents.is_empty() {
        c = c.child(empty(f, "talk-dm-none", "No agents are running."));
    }
    for a in agents {
        let key = a.card.key.clone();
        let who = AgentRef {
            key: a.card.key.clone(),
            name: a.card.name.clone(),
            status: a.card.status.clone(),
            live: true,
            ..AgentRef::default()
        };
        let face_el = face(s, f, &format!("talk-dm-a-{}-f", a.card.key), &who, FaceOpts {
            inline: true,
            no_hover: true,
            ..FaceOpts::default()
        });
        c = c.child(
            row()
                .key(format!("talk-dm-a-{}", a.card.key))
                .role("option")
                .focusable(true)
                .w_full()
                .px(8.0)
                .py(4.0)
                .gap(8.0)
                .center_y()
                .radius(4.0)
                .hover(k.c.hover)
                .on_press(move |s: &mut State| start_dm(s, "agent", &key))
                .child(face_el)
                .child(
                    text(plain_line(&a.card.name))
                        .font(ts(12.5, 400.0))
                        .color(k.c.text_base)
                        .max_lines(1)
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    text(agent_status(&Who::from(a), None, now_ms()).text)
                        .font(ts(11.0, 400.0))
                        .color(k.c.text_muted),
                ),
        );
    }
    if !s.talk.new_error.is_empty() {
        c = c.child(
            row().key("talk-dm-err").px(8.0).child(
                text(s.talk.new_error.clone())
                    .font(ts(11.0, 400.0))
                    .color(crate::ui::theme::status::failed())
                    .w_full()
                    .wrap(),
            ),
        );
    }
    c
}

fn start_dm(s: &mut State, kind: &str, key: &str) {
    let m = TalkMemberRef {
        kind: kind.to_string(),
        key: key.to_string(),
    };
    s.spawn(
        move |w| w.talk_dm(&m),
        |s, r| match r {
            Ok(id) => {
                s.talk.adding = Adding::None;
                s.spawn(|w| w.talk_conversations(), move |s, cs| {
                    if let Ok(cs) = cs {
                        s.talk.convs = cs;
                    }
                    open(s, &id);
                    keep_open(s);
                });
            }
            Err(e) => s.talk.new_error = e,
        },
    );
}
