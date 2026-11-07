//! Home (P1 Home and Needs you): the Rune port of the React app's
//! App.tsx home branch (BoardsNeedYou, IdleLine), cards/needs-you.tsx,
//! cards/ask-box.tsx, home/spawn-notice.tsx, home/wind-down-line.tsx,
//! wind-down-dialog.tsx and lib/needs.ts's use (through `logic::needs`),
//! on main under frontend/src/.
//!
//! The page: the first-run spawn notice, the wind-down line, the ask box,
//! the idle line, then Needs you, the spaces with decisions Home doesn't
//! list, and the person's cards (the board package's list, without the
//! cards Needs you already shows).
//!
//! Answers are instant (lib/optimistic.ts, card #644) as an in-flight flag
//! and a re-read: a row says what it did at once, the save runs off the UI
//! thread, a refusal takes it back with a toast, and `board:changed`
//! re-reads the cards.
//!
//! For other pages: [`needs::needs_row`] (one Needs you row) and
//! [`needs::blocker_answer`] (one item's answers).

pub mod ask;
mod icons;
pub mod needs;
pub mod permission;
pub mod pure;
pub mod sample;
pub mod wind_down;

use std::collections::{HashMap, HashSet};

use inbox::Delivery;
use irori_window::agents::{AgentsChange, AgentsSettings};
use irori_window::agents_trust::TrustNeed;
use irori_window::cards::{CardDetail, CardSummary, IdleReport};
use irori_window::events::{EVENT_BOARD_CHANGED, EVENT_INBOX_CHANGED, EVENT_ROSTER_CHANGED};
use irori_window::freshness::LogFreshness;
use irori_window::ts::RepoName;
use irori_window::wind_down::{EVENT_WIND_DOWN, WindDownStatus};
use rune::{El, TextEdit, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use crate::logic::agent_counts::{Untaken, idle_line};
use crate::logic::agent_status::now_ms;
use crate::logic::card_status::card_ref;
use crate::logic::cards::with_live;
use crate::logic::needs::{Need, NeedKind, needs_of, nobody_took};
use crate::text::Composer;
use crate::ui::{Dialog, Event, Frame, Route, State};

#[derive(Default)]
pub struct Model {
    /// The ask box.
    pub ask: Composer,
    /// What the last send said, or its error (true).
    pub ask_line: Option<(String, bool)>,
    /// "Agents answer now" as the person set it; None follows the mentions.
    pub agents_now: Option<bool>,
    /// Held messages (Needs you's held rows and quiet line).
    pub held: Vec<Delivery>,
    pub held_loaded: bool,
    pub repo_names: Vec<RepoName>,
    /// The idle line's input.
    pub idle: IdleReport,
    /// How fresh each log is (the freshness line under the idle line).
    pub fresh: Vec<LogFreshness>,
    /// Needs you's cards' pages (their open items), by card id.
    pub details: HashMap<String, CardDetail>,
    /// Trust rows: who must trust the repo (None: the host said nothing).
    pub trust: HashMap<String, Option<TrustNeed>>,
    /// Settings › Agents for the private board (the spawn notice).
    pub spawn: Option<AgentsSettings>,
    pub spawn_error: String,
    /// The latest Wind down run, and the run whose line was dismissed.
    pub wd: WindDownStatus,
    pub wd_seen: String,
    pub wd_dlg: wind_down::Dlg,
    /// Answers in flight, by answer key (pure::answer_key).
    pub busy: HashSet<String>,
    /// What an answer said at once, by answer key.
    pub said: HashMap<String, String>,
    /// Open own-words boxes, by answer key.
    pub own: HashMap<String, TextEdit>,
    /// Row actions (Release, Trust and start, ...), by row key.
    pub acts: HashMap<String, needs::Act>,
    /// Start an agent's outcome, kept under the rows.
    pub started: String,
    /// Release all on the quiet holds.
    pub quiet_busy: bool,
    pub quiet_said: String,
    /// Bumped on each card re-read: an older read's answer is dropped.
    rev: u64,
    /// The freshness poll runs.
    ticker: bool,
}

/// The space the switcher picked (card #393), and Home's own scope in it:
/// your cards (the private ones and the shared ones your agents hold or
/// watch), or a shared space's when one is picked.
pub fn home_scope(s: &State) -> (String, String) {
    let space = crate::spaces::current(s);
    let shared = space != crate::spaces::ALL_SPACES && space != crate::spaces::PERSONAL;
    let scope = if shared {
        space.clone()
    } else {
        crate::board::SCOPE_OWN.to_string()
    };
    (space, scope)
}

/// The cards in the picked space.
fn space_cards<'a>(s: &'a State, space: &str) -> Vec<&'a CardSummary> {
    s.data
        .cards
        .iter()
        .filter(|c| crate::logic::spaces::card_in_space(c, space))
        .collect()
}

/// Home's cards: the picked space's, in Home's scope.
pub fn home_cards(s: &State) -> Vec<CardSummary> {
    let (space, scope) = home_scope(s);
    space_cards(s, &space)
        .into_iter()
        .filter(|c| crate::board::filter::in_scope(c, &scope))
        .cloned()
        .collect()
}

/// The ids Needs you lists as cards (All leaves them out of the list).
fn listed(needs: &[Need]) -> HashSet<String> {
    needs
        .iter()
        .filter(|n| {
            matches!(
                n.kind,
                NeedKind::Check(_)
                    | NeedKind::Decision(_)
                    | NeedKind::Merge(..)
                    | NeedKind::Gone(..)
                    | NeedKind::NoAgent(_)
            )
        })
        .map(|n| n.id.clone())
        .collect()
}

fn load_held(s: &mut State) {
    s.spawn(
        |w| (w.inbox_held(), w.inbox_repo_names()),
        |s, (held, names)| {
            s.home.held = held;
            s.home.repo_names = names;
            s.home.held_loaded = true;
        },
    );
}

/// Re-reads what Needs you's card rows show: the idle report, the open
/// items of each check or decision, and who must trust a repo.
fn load_cards(s: &mut State) {
    s.home.rev += 1;
    let rev = s.home.rev;
    let cards = home_cards(s);
    let (pages, trust): (Vec<String>, Vec<String>) = {
        let needs = needs_of(&cards, &[]);
        let pages = needs
            .iter()
            .filter(|n| matches!(n.kind, NeedKind::Check(_) | NeedKind::Decision(_)))
            .map(|n| n.id.clone())
            .collect();
        let trust = needs
            .iter()
            .filter(|n| matches!(n.kind, NeedKind::Trust(_)))
            .map(|n| n.id.clone())
            .collect();
        (pages, trust)
    };
    s.spawn(
        move |w| {
            let idle = w.idle_report();
            let fresh = w.freshness();
            let pages: Vec<(String, CardDetail)> = pages
                .into_iter()
                .filter_map(|id| w.card(&id).ok().map(|d| (id, d)))
                .collect();
            let trust: Vec<(String, Option<TrustNeed>)> = trust
                .into_iter()
                .map(|id| {
                    let t = w.agents_trust_need(&id).ok().flatten();
                    (id, t)
                })
                .collect();
            (idle, fresh, pages, trust)
        },
        move |s, (idle, fresh, pages, trust)| {
            if s.home.rev != rev {
                return;
            }
            s.home.idle = idle;
            s.home.fresh = fresh;
            s.home.details = pages.into_iter().collect();
            s.home.trust = trust.into_iter().collect();
            // A said that the cards caught up with goes; a kept draft
            // opens the box it was typed in.
            let open: Vec<(String, String)> = s
                .home
                .details
                .values()
                .flat_map(|d| {
                    d.blockers
                        .iter()
                        .filter(|b| pure::waits_on_you(b))
                        .map(|b| {
                            (
                                pure::answer_key(&d.summary.id, &b.id),
                                needs::words_key(&d.summary.id, &b.id),
                            )
                        })
                })
                .collect();
            let busy = s.home.busy.clone();
            s.home
                .said
                .retain(|k, _| busy.contains(k) || open.iter().any(|(a, _)| a == k));
            s.home.own.retain(|k, _| open.iter().any(|(a, _)| a == k));
            for (ak, dk) in open {
                if !s.home.own.contains_key(&ak)
                    && !s.home.said.contains_key(&ak)
                    && !crate::text::drafts::draft(s, &dk).text.is_empty()
                {
                    let d = crate::text::drafts::draft(s, &dk);
                    s.home.own.insert(ak, TextEdit::new(&d.text));
                }
            }
        },
    );
}

/// How often Home re-reads how fresh each log is (lib/freshness.ts
/// RefreshMs): a catch-up that brings nothing new fires no event, so a
/// quiet log is polled, and "3h ago" moves on.
const FRESH_POLL_SECS: u64 = 30;

/// Re-reads the freshness line every 30 s while Home shows.
fn start_ticker(s: &mut State) {
    if s.home.ticker || s.headless {
        return;
    }
    let Some(h) = s.handle.clone() else { return };
    s.home.ticker = true;
    std::thread::Builder::new()
        .name("irori-home-fresh".into())
        .spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(FRESH_POLL_SECS));
                h.update(|s: &mut State, _| {
                    if s.nav.card.is_none() && s.nav.route == Route::Home {
                        s.spawn(|w| w.freshness(), |s, v| s.home.fresh = v);
                    }
                });
            }
        })
        .ok();
}

fn load_spawn(s: &mut State) {
    s.spawn(|w| w.agents_settings(""), |s, v| s.home.spawn = Some(v));
}

pub fn on_event(s: &mut State, ev: &Event) {
    start_ticker(s);
    match ev.name.as_str() {
        "app:start" => {
            s.home.wd_seen = s.prefs.get(wind_down::SEEN_PREF).unwrap_or_default();
            let here = ask::here(s);
            crate::text::open_draft(s, ask::ask_mut, &ask::draft_key(&here));
            load_held(s);
            load_cards(s);
            load_spawn(s);
            wind_down::reload(s);
        }
        EVENT_INBOX_CHANGED => load_held(s),
        EVENT_BOARD_CHANGED | EVENT_ROSTER_CHANGED => load_cards(s),
        EVENT_WIND_DOWN => {
            wind_down::reload(s);
        }
        // However it opens (Home's button or line, the tray, the palette),
        // the dialog reads the run afresh, as React's did on mount.
        "app:dialog-opened" if s.dialog == Some(Dialog::WindDown) => wind_down::reload(s),
        irori_window::boards::EVENT_BOARDS_CHANGED => {
            load_spawn(s);
            load_cards(s);
        }
        // The ask box keeps a draft per place (Home, or a space's page).
        "app:route-changed" => {
            let here = ask::here(s);
            ask::follow(s, &here);
        }
        // Another space: its Needs you rows and its own ask draft.
        crate::spaces::EVENT_SPACE_PICKED => {
            let here = ask::here(s);
            ask::follow(s, &here);
            load_cards(s);
        }
        _ => {}
    }
}

/// The first-run notice for spawned agents (card #448): Irori now starts
/// an agent for each card on the person's own board. Goes once they keep
/// it on or turn it off.
fn spawn_notice(s: &State, f: &Frame) -> Option<El> {
    let k = f.k;
    let st = s.home.spawn.as_ref()?;
    if st.chosen || !st.on {
        return None;
    }
    let set = |on: bool| {
        move |s: &mut State| {
            let c = AgentsChange {
                on: Some(on),
                ..AgentsChange::default()
            };
            s.spawn(
                move |w| w.agents_set(c),
                |s, r| match r {
                    Ok(v) => {
                        s.home.spawn = Some(v);
                        s.home.spawn_error.clear();
                    }
                    Err(e) => s.home.spawn_error = e,
                },
            );
        }
    };
    let mut words = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(4.0)
        .child(
            text("Irori now starts an agent for each open card in Personal.")
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .w_full()
                .wrap(),
        )
        .child(
            text("You no longer wait for a free agent: open cards nobody holds get one now, up to this device's cap, and each new card gets its own. It starts on the card, in its repo, and ends once the card is done. Each repo must be trusted in Claude Code first; a card whose repo isn't shows \"can't start\" with Trust and start. Change the model or turn this off in Settings › Agents.")
                .font(rune::tok::TextStyle::new(12.5, 400.0))
                .color(k.c.text_muted)
                .w_full()
                .wrap(),
        );
    if !s.home.spawn_error.is_empty() {
        words = words.child(
            text(s.home.spawn_error.clone())
                .font(rune::tok::TextStyle::new(12.5, 400.0))
                .color(k.c.danger.fg)
                .w_full()
                .wrap(),
        );
    }
    let b = |label: &str, v: ButtonVariant| {
        Button::new(label.to_string())
            .variant(v)
            .control_size(ControlSize::Small)
    };
    Some(
        col()
            .key("home/spawn-notice")
            .role("status")
            .w_full()
            .gap(8.0)
            .px(12.0)
            .py(10.0)
            .radius(10.0)
            .fill(k.c.layer_01)
            .border(1.0, k.c.border_muted)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_start()
                    .child(icon(&crate::ui::icons::BOT, 16.0).color(k.c.text_accent))
                    .child(words),
            )
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .child(rune::el().grow(1.0))
                    .child(
                        b("Settings › Agents", ButtonVariant::Ghost)
                            .on_press(|s: &mut State| {
                                s.open_dialog(Dialog::Settings("agents".into()))
                            })
                            .build(k),
                    )
                    .child(
                        b("Turn it off", ButtonVariant::Outline)
                            .on_press(set(false))
                            .build(k),
                    )
                    .child(
                        b("Keep it on", ButtonVariant::Contrast)
                            .on_press(set(true))
                            .build(k),
                    ),
            ),
    )
}

/// Home's freshness line (HomeFreshness, card #580): when the picked
/// space last caught up with another device, amber after a day; on All
/// spaces only the stalest log, once it is stale.
fn fresh_line(s: &State, f: &Frame, space: &str, now: i64) -> Option<El> {
    let k = f.k;
    let name = |log: &str| {
        s.data.spaces.iter().find(|b| b.board_id == log).map(|b| {
            if b.name.is_empty() {
                b.tag.clone()
            } else {
                b.name.clone()
            }
        })
    };
    let (t, old) = pure::home_fresh_line(space, &s.home.fresh, name, now)?;
    let color = if old {
        k.c.warning.icon
    } else {
        k.c.text_muted
    };
    Some(
        row()
            .key("home/fresh")
            .w_full()
            .px(4.0)
            .gap(6.0)
            .center_y()
            .child(icon(&crate::ui::icons::REFRESH, 12.0).color(color))
            .child(
                text(t)
                    .font(k.ty.text_xsmall)
                    .color(color)
                    .wrap()
                    .shrink()
                    .min_w(0.0),
            ),
    )
}

/// One line naming each space with decisions Home doesn't list as cards
/// (cards #237, #509, #520).
fn boards_need_you(s: &State, f: &Frame, space: &str) -> Option<El> {
    let k = f.k;
    let cards: Vec<CardSummary> = space_cards(s, space).into_iter().cloned().collect();
    let by = pure::boards_need_you(&cards);
    if by.is_empty() {
        return None;
    }
    let mut line = row()
        .key("home/boards-need-you")
        .w_full()
        .px(4.0)
        .gap(12.0)
        .flex_wrap()
        .center_y()
        .child(
            text("Waiting on you in your spaces:")
                .font(rune::tok::TextStyle::new(12.5, 400.0))
                .color(k.c.text_muted),
        );
    for (id, n) in by {
        let name = s
            .data
            .spaces
            .iter()
            .find(|b| b.board_id == id)
            .map(|b| b.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "a space".into());
        let space = id.clone();
        line = line.child(
            text(format!("{name} ({n})"))
                .key(format!("home/boards-need-you/{id}"))
                .role("button")
                .focusable(true)
                .font(rune::tok::TextStyle::new(12.5, 400.0))
                .color(k.c.danger.fg)
                .on_press(move |s: &mut State| {
                    s.go(Route::Board {
                        space: space.clone(),
                        board: None,
                    })
                }),
        );
    }
    Some(line)
}

/// Home's page: the board list (board::board_view) over the picked
/// space's cards, with Needs you leading it and the ask box over it.
pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let now = now_ms();
    let (space, scope) = home_scope(s);
    let shared = scope != crate::board::SCOPE_OWN;
    let cards = home_cards(s);
    // Held ones are yours, not a shared space's (card #393).
    let no_held: Vec<Delivery> = Vec::new();
    let held = if shared { &no_held } else { &s.home.held };
    let needs = needs_of(&cards, held);
    let in_needs = listed(&needs);

    // The idle line counts the cards Home files under Stuck because nobody
    // took them (by the live roster), so it never says "no open work"
    // over them.
    let live: HashMap<String, String> = s
        .data
        .agents
        .iter()
        .map(|a| (a.card.key.clone(), a.card.status.clone()))
        .collect();
    let untaken: Vec<Untaken> = cards
        .iter()
        .filter(|c| match with_live(&c.working, &live) {
            Some(w) => nobody_took(
                &CardSummary {
                    working: w,
                    ..(*c).clone()
                },
                now,
            ),
            None => nobody_took(c, now),
        })
        .map(|c| Untaken {
            id: c.id.clone(),
            short: card_ref(c),
        })
        .collect();
    let home_ids: HashSet<String> = cards.iter().map(|c| c.id.clone()).collect();
    let idle = idle_line(&s.home.idle, &untaken, &home_ids);

    let mut top = col().key("home/top").w_full().gap(14.0);
    if let Some(n) = spawn_notice(s, f) {
        top = top.child(n);
    }
    if let Some(l) = wind_down::line(s, f) {
        top = top.child(l);
    }
    top = top.child(ask::ask_box(s, f));
    if !idle.is_empty() {
        top = top.child(
            text(idle)
                .key("home/idle")
                .font(rune::tok::TextStyle::new(12.5, 400.0))
                .color(k.c.text_muted)
                .px(4.0)
                .w_full()
                .wrap(),
        );
    }
    if let Some(l) = fresh_line(s, f, &space, now) {
        top = top.child(l);
    }
    // Held messages read apart from the cards: until they land, "Nothing
    // waits on you" could be false.
    let lead = if !shared && !s.home.held_loaded && needs.is_empty() {
        crate::widgets::loading::needs_you_shape(f, "home/needs-loading", 1)
    } else {
        needs::needs_section(s, f, &needs, now)
    };
    let notice = boards_need_you(s, f, &space);
    let empty = crate::board::grid::empty_cards(f);
    let list = crate::board::board_view(
        s,
        f,
        crate::board::Opts {
            scope,
            cards: space_cards(s, &space),
            home: true,
            header: Some(top),
            lead: Some(lead),
            notice,
            empty: Some(empty),
            rows: true,
            listed: in_needs,
            ..Default::default()
        },
    );
    let header = crate::ui::page_header(
        f,
        "Home",
        vec![
            Button::new("Wind down")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .help("Ask every agent to save its place, hand back its cards and stop")
                .on_press(|s: &mut State| s.open_dialog(Dialog::WindDown))
                .build(k)
                .key("home/wind-down"),
            Button::new("+ New card")
                .variant(ButtonVariant::Contrast)
                .control_size(ControlSize::Small)
                .help("Make a card for an agent to pick up")
                .on_press(|s: &mut State| s.open_dialog(Dialog::NewCard { board: None }))
                .build(k)
                .key("home/new-card"),
        ],
    );
    col()
        .key("home-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(header)
        .child(
            col()
                .key("home/body")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .px(20.0)
                .pt(18.0)
                .child(list.grow(1.0).min_h(0.0)),
        )
}

/// The headless scene that shows it (`irori --snap x.png --scene home`).
pub fn scene(s: &mut State) {
    s.close_card();
    // A headless run keeps its prefs: the canvas scene leaves Canvas picked.
    crate::canvas::set_view(s, "list");
    crate::spaces::wait_for_spaces(s);
    s.go(Route::Home);
}

/// The Wind down dialog (mounted in `f.layer` when `s.dialog` names it).
pub fn wind_down_dialog(s: &State, f: &mut Frame) {
    wind_down::dialog(s, f);
}
