//! The command palette (P9, card #213): the Rune port of the React app's
//! components/command-palette.tsx, lib/commands.ts and lib/fuzzy.ts (on
//! main, under frontend/src/). Cmd/Ctrl+K or P opens it from anywhere. One
//! search covers cards (a ref like #212 or wind#51, or title words),
//! spaces, agents, and the commands ([`commands::commands`]). Results are
//! grouped; an empty query shows what was picked last. Arrows and Enter
//! pick, Esc closes.

pub mod commands;
pub mod fuzzy;

use std::collections::HashMap;
use std::rc::Rc;

use irori_window::cards::CardSummary;
use irori_window::threads::RefTarget;
use rune::{El, Icon, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, el, icon, row, text};
use rune_kit::{Keybind, TextInput};

use crate::ui::{Dialog, Event, Frame, Route, State, icons};

use commands::{Run, shortcut_keys};

/// Where recents are kept (the React app's localStorage irori.palette.recent).
const RECENT: &str = "palette.recent";
const MAX_RECENT: usize = 6;
/// How many rows each group shows for a query.
const PER_GROUP: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Command,
    Card,
    Board,
    Agent,
}

/// One row: what it shows and what Enter does.
#[derive(Clone)]
pub struct Item {
    pub kind: Kind,
    /// Kind-prefixed, stable across opens: recents keep it.
    pub key: String,
    pub title: String,
    /// A muted line after the title (a card's ref, a space's tag).
    pub hint: String,
    pub group: &'static str,
    pub shortcut: Option<&'static str>,
    pub order: i32,
    pub quiet: bool,
    pub others: Vec<String>,
    pub run: Run,
}

#[derive(Default)]
pub struct Model {
    pub query: TextEdit,
    pub active: usize,
    /// A ref no live card has (a done card, another board's, a thread),
    /// looked up across every board: the query it answers, and its card.
    pub found: Option<(String, RefTarget)>,
}

/// Whether a query is one card ref: #12, wind#51.
pub fn is_ref(q: &str) -> bool {
    let q = q.trim();
    let Some((tag, rest)) = q.split_once('#') else { return false };
    if !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return false;
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].chars().all(|c| c.is_ascii_alphabetic())
}

/// Whether a key is the palette's: Cmd+K or Cmd+P on a Mac, Ctrl+K or
/// Ctrl+P elsewhere, with no Shift or Alt.
pub fn open_key(ev: &KeyEvent) -> bool {
    !ev.mods.shift && !ev.mods.alt && (ev.is_command('k') || ev.is_command('p'))
}

fn card_ref_text(c: &CardSummary) -> String {
    if c.tag.is_empty() { format!("#{}", c.short) } else { format!("{}#{}", c.tag, c.short) }
}

/// Each agent Irori started for a card, to that card: a working row
/// marked spawned. The first card wins for a stale second row.
pub fn spawned_cards(cards: &[CardSummary]) -> HashMap<&str, &CardSummary> {
    let mut m = HashMap::new();
    for c in cards {
        for w in &c.working {
            if w.spawned {
                m.entry(w.key.as_str()).or_insert(c);
            }
        }
    }
    m
}

/// Every row a query can find, commands first.
pub fn build_items(s: &State, dark: bool) -> Vec<Item> {
    let mut out = Vec::new();
    for c in commands::commands(s, dark) {
        out.push(Item {
            kind: Kind::Command,
            key: format!("command:{}", c.id),
            title: c.title,
            hint: String::new(),
            group: c.group,
            shortcut: c.shortcut,
            order: c.order,
            quiet: c.quiet,
            others: c.keywords.iter().map(|k| k.to_string()).collect(),
            run: c.run,
        });
    }
    let card = |c: &CardSummary, kind, key: String, title: String, hint: String, group, others| {
        let id = c.id.clone();
        Item {
            kind,
            key,
            title,
            hint,
            group,
            shortcut: None,
            order: 100,
            quiet: false,
            others,
            run: Rc::new(move |s: &mut State| s.open_card(&id)),
        }
    };
    for c in &s.data.cards {
        let r = card_ref_text(c);
        let others = vec![
            format!("#{}", c.short),
            r.clone(),
            if c.short_was.is_empty() { String::new() } else { format!("#{}", c.short_was) },
            c.scope_label.clone(),
        ];
        out.push(card(c, Kind::Card, format!("card:{}", c.id), c.title.clone(), r, "Cards", others));
    }
    for b in &s.data.spaces {
        let id = b.board_id.clone();
        out.push(Item {
            kind: Kind::Board,
            key: format!("board:{}", b.board_id),
            title: b.name.clone(),
            hint: if b.tag.is_empty() { String::new() } else { format!("{}#", b.tag) },
            group: "Spaces",
            shortcut: None,
            order: 100,
            quiet: false,
            others: vec![b.tag.clone(), "space".into(), "board".into()],
            run: Rc::new(move |s: &mut State| {
                s.close_card();
                s.go(Route::Board {
                    space: id.clone(),
                    board: None,
                })
            }),
        });
    }
    // Agents Irori started for a card are listed under their card (card
    // #456); terminal agents keep "cards with this agent".
    let spawned = spawned_cards(&s.data.cards);
    let mut seen: Vec<&str> = Vec::new();
    for a in &s.data.agents {
        let a = &a.card;
        if let Some(c) = spawned.get(a.key.as_str())
            && a.status != "gone"
        {
            let r = card_ref_text(c);
            let others = vec![r.clone(), format!("#{}", c.short), c.title.clone(), "agent".into()];
            let title = if a.name.is_empty() { a.key.clone() } else { a.name.clone() };
            out.push(card(c, Kind::Agent, format!("agent:{}", a.key), title, format!("on {r} {}", c.title), "Agents on cards", others));
            continue;
        }
        if a.name.is_empty() || seen.contains(&a.name.as_str()) {
            continue;
        }
        seen.push(&a.name);
        out.push(Item {
            kind: Kind::Agent,
            key: format!("agent:{}", a.name),
            title: a.name.clone(),
            hint: "cards with this agent".into(),
            group: "Agents",
            shortcut: None,
            order: 100,
            quiet: false,
            others: vec![a.title.clone(), a.ai_title.clone(), "agent".into()],
            // Home's cards narrowed to that agent's (App.tsx onPaletteAgent:
            // the shared search box takes the name).
            run: {
                let name = a.name.clone();
                Rc::new(move |s: &mut State| {
                    s.board.query = TextEdit::new(name.clone());
                    s.close_card();
                    s.go(Route::Home)
                })
            },
        });
    }
    out
}

/// The order groups show in; any other group comes after Commands.
const GROUP_ORDER: &[&str] = &["Recent", "Go to", "Cards", "Spaces", "Commands", "Agents", "Agents on cards"];

fn group_index(g: &str) -> f64 {
    match GROUP_ORDER.iter().position(|x| *x == g) {
        Some(i) => i as f64,
        None => 4.5,
    }
}

/// The rows a query shows, grouped: the group with the best match first.
/// Empty: recents, then the "Go to" commands and the rest of the commands.
/// A card ref puts its card first.
pub fn results(items: &[Item], query: &str, recent: &[String]) -> Vec<Item> {
    if fuzzy::query_parts(query).is_empty() {
        let rec: Vec<Item> = recent
            .iter()
            .filter_map(|k| items.iter().find(|i| &i.key == k))
            .map(|i| Item {
                group: "Recent",
                ..i.clone()
            })
            .collect();
        let mut cmds: Vec<(usize, &Item)> = items
            .iter()
            .filter(|i| i.kind == Kind::Command && !i.quiet && !rec.iter().any(|r| r.key == i.key))
            .enumerate()
            .collect();
        cmds.sort_by(|a, b| a.1.order.cmp(&b.1.order).then(a.0.cmp(&b.0)));
        let mut rows: Vec<Item> = rec;
        rows.extend(cmds.into_iter().map(|(_, i)| i.clone()));
        // Groups in GROUP_ORDER, rows in their order within a group.
        let mut idx: Vec<(usize, Item)> = rows.into_iter().enumerate().collect();
        idx.sort_by(|a, b| group_index(a.1.group).total_cmp(&group_index(b.1.group)).then(a.0.cmp(&b.0)));
        return idx.into_iter().map(|(_, i)| i).collect();
    }
    let r = if is_ref(query) { query.trim().to_lowercase() } else { String::new() };
    let ranked = fuzzy::rank(items, query, |i| (i.title.as_str(), i.others.as_slice()));
    let exact = |i: &Item| !r.is_empty() && i.kind == Kind::Card && i.others.iter().any(|o| o.to_lowercase() == r);
    let mut out: Vec<Item> = ranked.iter().filter(|(i, _)| exact(i)).map(|(i, _)| (*i).clone()).collect();
    // Best first within a group, and the group with the best match first.
    let mut groups: Vec<(&'static str, Vec<Item>)> = Vec::new();
    for (i, _) in ranked.iter().filter(|(i, _)| !exact(i)) {
        match groups.iter_mut().find(|(g, _)| *g == i.group) {
            Some((_, v)) => {
                if v.len() < PER_GROUP {
                    v.push((*i).clone());
                }
            }
            None => groups.push((i.group, vec![(*i).clone()])),
        }
    }
    out.extend(groups.into_iter().flat_map(|(_, v)| v));
    out
}

fn recent(s: &State) -> Vec<String> {
    s.prefs.get::<Vec<String>>(RECENT).unwrap_or_default()
}

fn keep_recent(s: &mut State, key: &str) {
    let mut next = vec![key.to_string()];
    next.extend(recent(s).into_iter().filter(|k| k != key));
    next.truncate(MAX_RECENT);
    s.prefs.set(RECENT, next);
}

/// The rows the palette shows now.
pub fn shown(s: &State, dark: bool) -> Vec<Item> {
    let q = s.palette.query.text.clone();
    let items = build_items(s, dark);
    let mut rows = results(&items, &q, &recent(s));
    if let Some((fq, t)) = &s.palette.found
        && fq == q.trim()
        && !rows.iter().any(|r| r.key == format!("card:{}", t.id))
    {
        let (id, thread) = (t.id.clone(), t.thread);
        let hint = format!("{fq}{}", if t.thread { " · thread" } else if t.done { " · done" } else { "" });
        rows.insert(
            0,
            Item {
                kind: Kind::Card,
                key: format!("card:{}", t.id),
                title: t.title.clone(),
                hint,
                group: "Cards",
                shortcut: None,
                order: 100,
                quiet: false,
                others: Vec::new(),
                run: Rc::new(move |s: &mut State| {
                    if thread {
                        s.close_card();
                        s.go(Route::Talk(Some(id.clone())));
                    } else {
                        s.open_card(&id);
                    }
                }),
            },
        );
    }
    rows
}

/// Fresh on every open: the query empty, the first row active.
fn reset(s: &mut State) {
    s.palette.query = crate::inbox::focused_edit();
    s.palette.active = 0;
    s.palette.found = None;
}

fn close(s: &mut State) {
    if s.dialog == Some(Dialog::Palette) {
        s.close_dialog();
    }
}

/// Runs a row: remembered as recent, the palette closed (and with it any
/// dialog it was over), then the row's action.
fn pick(s: &mut State, item: &Item) {
    keep_recent(s, &item.key);
    close(s);
    (item.run)(s);
}

/// Looks a ref up when no loaded card answers it.
fn look_up(s: &mut State) {
    let q = s.palette.query.text.trim().to_string();
    if !is_ref(&q) {
        return;
    }
    let lower = q.to_lowercase();
    if s.data.cards.iter().any(|c| format!("#{}", c.short) == lower || card_ref_text(c).to_lowercase() == lower) {
        return;
    }
    s.spawn(
        {
            let q = q.clone();
            move |w| w.card_ref(&q)
        },
        move |s, t| {
            if let Some(t) = t
                && s.palette.query.text.trim() == q
            {
                s.palette.found = Some((q, t));
            }
        },
    );
}

/// The window's key (AppSpec::on_key through ui::on_key): Cmd/Ctrl+K or P
/// opens the palette from anywhere, a card's page and other dialogs
/// included, and closes it again.
pub fn on_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if !open_key(ev) {
        return false;
    }
    if s.dialog == Some(Dialog::Palette) {
        s.close_dialog();
    } else {
        s.open_dialog(Dialog::Palette);
    }
    true
}

pub fn on_event(s: &mut State, ev: &Event) {
    if ev.is("app:dialog-opened") && s.dialog == Some(Dialog::Palette) {
        reset(s);
    }
}

/// The palette has no page of its own.
#[allow(dead_code)]
pub fn view(_s: &State, f: &mut Frame) -> El {
    crate::ui::not_yet(f, "Command palette", "The palette is a dialog: Cmd+K.")
}

/// The headless scene that shows it (`irori --snap x.png --scene palette`).
pub fn scene(s: &mut State) {
    s.open_dialog(Dialog::Palette);
}

/// The palette's arrows, Home, End and Enter (in the field, before it
/// moves the caret).
fn field_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx, dark: bool) -> bool {
    let ctrl = |c: char| {
        ev.mods.ctrl
            && !ev.mods.alt
            && match &ev.key {
                Key::Command(k) => *k == c,
                Key::Text(t) => t.eq_ignore_ascii_case(&c.to_string()),
                _ => false,
            }
    };
    let rows = shown(s, dark);
    let n = rows.len();
    let a = s.palette.active.min(n.saturating_sub(1));
    let next = match &ev.key {
        Key::Down => Some(if n == 0 { 0 } else { (a + 1) % n }),
        Key::Up => Some(if n == 0 { 0 } else { (a + n - 1) % n }),
        Key::Home if !ev.mods.shift => Some(0),
        Key::End if !ev.mods.shift => Some(n.saturating_sub(1)),
        Key::Enter => {
            if let Some(i) = rows.get(a) {
                pick(s, i);
            }
            return true;
        }
        _ if ctrl('n') => Some(if n == 0 { 0 } else { (a + 1) % n }),
        _ if ctrl('p') && !ev.mods.cmd => Some(if n == 0 { 0 } else { (a + n - 1) % n }),
        _ => None,
    };
    let Some(next) = next else { return false };
    s.palette.active = next;
    tx.scroll_into_view(format!("palette-row-{next}"));
    true
}

fn kind_icon(k: Kind) -> &'static Icon {
    match k {
        Kind::Command => &icons::TERMINAL,
        Kind::Card => &SQUARE,
        Kind::Board => &icons::GRID,
        Kind::Agent => &icons::BOT,
    }
}

static SQUARE: Icon = Icon {
    name: "lucide-square",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2"/></g>"#,
};

static ENTER: Icon = Icon {
    name: "lucide-corner-down-left",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 10 4 15 9 20"/><path d="M20 4v7a4 4 0 0 1-4 4H4"/></g>"#,
};

/// The palette over the window (Dialog::Palette).
pub fn dialog(s: &State, f: &mut Frame) {
    let k = f.k;
    let dark = f.dark;
    let mac = cfg!(target_os = "macos");
    let rows = shown(s, dark);
    let active = s.palette.active.min(rows.len().saturating_sub(1));
    let input = TextInput::new(&s.palette.query)
        .plain()
        .width(None)
        .placeholder("Jump to a card, space or agent, or run a command…")
        .label("Search cards, spaces, agents and commands")
        .text_style(rune::tok::TextStyle::new(15.0, 400.0))
        .height(48.0)
        .active_descendant((!rows.is_empty()).then(|| format!("palette-row-{active}")))
        .on_edit(|s: &mut State| &mut s.palette.query)
        .on_change(|s: &mut State, _| {
            s.palette.active = 0;
            look_up(s);
        })
        .on_key(move |s: &mut State, ev: &KeyEvent, tx: &mut Tx| field_key(s, ev, tx, dark))
        .build(k);
    let head = row()
        .key("palette-head")
        .w_full()
        .px(12.0)
        .gap(8.0)
        .center_y()
        .child(icon(&icons::SEARCH, 16.0).color(k.c.icon_muted))
        .child(col().grow(1.0).min_w(0.0).child(input))
        .child(Keybind::new(&["Esc"]).build(k));

    let mut list = col().key("palette-list").role("listbox").label("Results").w_full().padding(6.0);
    if rows.is_empty() {
        list = list.child(
            text(format!("Nothing matches “{}”.", s.palette.query.text.trim()))
                .key("palette-none")
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .w_full()
                .py(24.0)
                .wrap(),
        );
    }
    for (i, it) in rows.iter().enumerate() {
        if i == 0 || rows[i - 1].group != it.group {
            list = list.child(
                text(it.group.to_uppercase())
                    .font(k.ty.small_medium)
                    .color(k.c.text_muted)
                    .px(10.0)
                    .pt(10.0)
                    .pb(4.0),
            );
        }
        let on = i == active;
        let item = it.clone();
        let mut r = row()
            .key(format!("palette-row-{i}"))
            .role("option")
            .label(it.title.clone())
            .checked(on)
            .w_full()
            .h(32.0)
            .px(10.0)
            .gap(10.0)
            .center_y()
            .radius(6.0)
            .fill(if on { k.c.layer_03 } else { k.c.layer_01 })
            .on_hover(move |s: &mut State| {
                if s.palette.active != i {
                    s.palette.active = i;
                }
            })
            .on_press(move |s: &mut State| pick(s, &item))
            .child(icon(kind_icon(it.kind), 14.0).color(k.c.icon_muted))
            .child(text(it.title.clone()).font(k.ty.ui).color(k.c.text_base).max_lines(1).shrink().min_w(0.0));
        if !it.hint.is_empty() {
            r = r.child(text(it.hint.clone()).font(k.ty.small).color(k.c.text_muted).max_lines(1));
        }
        r = r.child(el().grow(1.0));
        if let Some(sc) = it.shortcut {
            r = r.child(Keybind::new(&shortcut_keys(sc, mac)).build(k));
        }
        if on {
            r = r.child(icon(&ENTER, 14.0).color(k.c.icon_muted));
        }
        list = list.child(r);
    }

    let mut panel = col()
        .key("palette-panel")
        .role("dialog")
        .label("Command palette")
        .w_full()
        .max_w(640.0)
        .radius(10.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(head)
        .child(el().w_full().h(1.0).fill(k.c.border_muted))
        // A grown scroll in an auto-height panel collapses: bound it instead.
        .child(col().key("palette-scroll").w_full().max_h(500.0).min_h(0.0).child(list).scroll(ScrollSpec { stick_bottom: false }))
        .modal(true, close);
    for sh in k.shadow_layers(&k.elevation.overlay) {
        panel = panel.box_shadow(sh);
    }
    f.layer.push(el().key("palette-scrim").absolute(0.0, 0.0).w_full().h_full().fill(k.c.scrim).a11y_hidden());
    f.layer.push(
        col()
            .key("palette-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .px(16.0)
            .pt(110.0)
            .pb(16.0)
            .items_center()
            .child(panel),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_narrows_home_to_its_cards() {
        let mut s = State::default();
        s.go(Route::Notes);
        let mut card = coord::roster::AgentCard::default();
        card.key = "sess-1".into();
        card.name = "oak".into();
        s.data.agents.push(irori_window::roster::RosterAgent {
            card,
            offers_cards: true,
            spawned: false,
            here: true,
        });
        let items = build_items(&s, true);
        let oak = items.iter().find(|i| i.key == "agent:oak").expect("oak is listed");
        assert_eq!(oak.group, "Agents");
        (oak.run.clone())(&mut s);
        assert_eq!(s.nav.route, Route::Home);
        assert_eq!(s.board.query.text, "oak");
    }

    #[test]
    fn refs() {
        assert!(is_ref("#12"));
        assert!(is_ref("wind#51"));
        assert!(is_ref(" #663a "));
        assert!(!is_ref("12"));
        assert!(!is_ref("#"));
        assert!(!is_ref("new card"));
    }

    fn item(kind: Kind, key: &str, title: &str, group: &'static str, others: &[&str]) -> Item {
        Item {
            kind,
            key: key.into(),
            title: title.into(),
            hint: String::new(),
            group,
            shortcut: None,
            order: 100,
            quiet: false,
            others: others.iter().map(|x| x.to_string()).collect(),
            run: Rc::new(|_| {}),
        }
    }

    #[test]
    fn empty_shows_recents_then_go_to_then_commands() {
        let items = vec![
            item(Kind::Command, "command:new", "New card", "Commands", &[]),
            item(Kind::Command, "command:home", "Home", "Go to", &[]),
            item(Kind::Card, "card:a", "A card", "Cards", &["#1"]),
        ];
        let got: Vec<(String, &str)> = results(&items, "", &["card:a".into()]).into_iter().map(|i| (i.key, i.group)).collect();
        assert_eq!(
            got,
            [
                ("card:a".to_string(), "Recent"),
                ("command:home".to_string(), "Go to"),
                ("command:new".to_string(), "Commands"),
            ]
        );
    }

    #[test]
    fn a_ref_puts_its_card_first() {
        let items = vec![
            item(Kind::Command, "command:x", "Open #12 notes", "Commands", &[]),
            item(Kind::Card, "card:b", "Other", "Cards", &["#120"]),
            item(Kind::Card, "card:a", "Twelve", "Cards", &["#12", "wind#12"]),
        ];
        let got = results(&items, "#12", &[]);
        assert_eq!(got[0].key, "card:a");
    }

    #[test]
    fn open_keys() {
        let ev = |k: char, shift: bool| KeyEvent {
            key: Key::Command(k),
            mods: rune::Mods {
                cmd: true,
                shift,
                ..rune::Mods::default()
            },
        };
        assert!(open_key(&ev('k', false)));
        assert!(open_key(&ev('p', false)));
        assert!(!open_key(&ev('k', true)));
        assert!(!open_key(&ev('j', false)));
    }
}
