//! Settings > Agents (agents-pane.tsx, agent-permissions.tsx,
//! agent-starts.tsx, space-agents.tsx; lib/agents.ts and
//! lib/agent-permissions.ts; cards #448, #449, #450, #456, #573, #575,
//! #576, #702, #703): your board's switch for agents Irori starts per
//! card, each space's switch, what a new agent starts with, the themes new
//! agents' names come from, every limit on them, the
//! agents running now, what they may do and how each repo's work reaches
//! main, and which of this computer's repos are trusted. Only you set
//! these, here; no agent's tool can.

use std::collections::{BTreeSet, HashMap, HashSet};

use irori_window::agents::{
    AgentStartsView, AgentsChange, AgentsLimits, AgentsSettings, NamesThemesView, RunningAgent,
};
use irori_window::agents_trust::{AgentPermissions, TrustedRepo};
use irori_window::board::AskRepo;
use irori_window::events::EVENT_ROSTER_CHANGED;
use rune::{Color, El, TextEdit, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Switch, TextArea};

use crate::ui::theme::status;
use crate::ui::{Event, Frame, State, icons};

use super::number::{self, WholeNumber};
use super::{
    Act, Confirm, boxed, choice, confirm_button, error_line, heading, hint, loading, muted_line,
    toggle,
};

#[derive(Default)]
pub struct Model {
    /// Personal's settings ("" board).
    pub settings: Option<AgentsSettings>,
    pub err: String,
    /// Each space's own switch, by board id.
    pub spaces: HashMap<String, AgentsSettings>,
    pub space_err: HashMap<String, String>,
    pub limits: Option<AgentsLimits>,
    pub limits_err: String,
    pub starts: Option<AgentStartsView>,
    pub starts_err: String,
    /// "At most" of My agents may start agents (card #703).
    pub max: WholeNumber,
    /// Each device's cap box in Limits, by device id (card #703).
    pub caps: HashMap<String, WholeNumber>,
    /// The themes new agents' names come from (card #702).
    pub names: Option<NamesThemesView>,
    pub names_err: String,
    pub running: Option<Vec<RunningAgent>>,
    pub running_err: String,
    /// Agents whose End went through, until the list drops them.
    pub ending: HashSet<String>,
    /// Open logs, by agent key.
    pub logs: HashMap<String, String>,
    pub perms: Option<AgentPermissions>,
    pub perms_err: String,
    /// The rule boxes; None shows what's saved.
    pub allow: Option<TextEdit>,
    pub deny: Option<TextEdit>,
    pub repos: Vec<AskRepo>,
    pub trusted: Option<Vec<TrustedRepo>>,
    pub trusted_err: String,
}

// ---- labels (lib/agents.ts) --------------------------------------------------------

fn model_name(v: &str) -> Option<&'static str> {
    Some(match v {
        "sonnet" => "Sonnet",
        "opus" => "Opus",
        "haiku" => "Haiku",
        _ => return None,
    })
}

fn effort_name(v: &str) -> Option<&'static str> {
    Some(match v {
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra high",
        "max" => "Max",
        _ => return None,
    })
}

fn choice_label(v: &str, name: Option<&str>, def: &str) -> String {
    let n = name.unwrap_or(v);
    if v == def {
        format!("{n} (default)")
    } else {
        n.to_string()
    }
}

pub fn model_label(v: &str, def: &str) -> String {
    choice_label(v, model_name(v), def)
}

pub fn effort_label(v: &str, def: &str) -> String {
    choice_label(v, effort_name(v), def)
}

/// An idle limit in minutes or hours, 30 marked.
pub fn idle_label(m: u32, def: u32) -> String {
    let n = if m.is_multiple_of(60) {
        format!("{} hour{}", m / 60, if m == 60 { "" } else { "s" })
    } else {
        format!("{m} minutes")
    };
    if m == def {
        format!("{n} (default)")
    } else {
        n
    }
}

/// The crashes or failed starts before Irori asks you (card #576).
pub const TRIES_CHOICES: &[u32] = &[1, 2, 3, 5, 10, 20];

/// The choices with n among them, in order.
pub fn with_choice(choices: &[u32], n: u32) -> Vec<u32> {
    let mut v = choices.to_vec();
    if !v.contains(&n) {
        v.push(n);
        v.sort_unstable();
    }
    v
}

pub fn count_label(n: u32, def: u32, one: &str, many: &str) -> String {
    let s = format!("{n} {}", if n == 1 { one } else { many });
    if n == def {
        format!("{s} (default)")
    } else {
        s
    }
}

/// What today's agents used at API prices: "$1.24", "under $0.01", "$0".
pub fn cost_label(usd: f64) -> String {
    if usd <= 0.0 {
        "$0".into()
    } else if usd < 0.01 {
        "under $0.01".into()
    } else {
        format!("${usd:.2}")
    }
}

/// How long ago an agent was asked for: "just now", "6 min", "2 h".
pub fn running_since(at: i64, now: i64) -> String {
    let m = (now - at).div_euclid(60_000);
    if m < 1 {
        "just now".into()
    } else if m < 60 {
        format!("{m} min")
    } else {
        format!("{} h", m / 60)
    }
}

/// When a usage hold ends, in local time: "resets 4:30 PM".
pub fn usage_resets(at_ms: i64) -> String {
    let secs = at_ms.div_euclid(1000);
    let local = secs + i64::from(sys::tz::local_offset(secs));
    let day = local.rem_euclid(86_400);
    let (h, m) = (day / 3600, (day % 3600) / 60);
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("resets {h12}:{m:02} {ampm}")
}

// An agent's status in words and a tone: lib/agent-status.ts, shared
// (crate::logic::agent_status), with the job's age from the live roster.
use crate::logic::agent_status::{Tone, Who, agent_status, ms};

// ---- permissions (lib/agent-permissions.ts) --------------------------------------

pub const BYPASS: &str = "bypass";
pub const LIMITED: &str = "limited";
pub const EDITS: &str = "edits";
pub const MERGE_PR: &str = "pr";
pub const MERGE_MAIN: &str = "main";

pub const PERMISSION_CHOICES: &[(&str, &str)] = &[
    (BYPASS, "Anything, without asking"),
    (LIMITED, "Its checks, git and Irori without asking"),
    (EDITS, "Edit files, ask before any command"),
];

/// The shown choice: an empty setting is bypass.
pub fn choice_of(p: &AgentPermissions) -> &str {
    if p.permission.is_empty() {
        BYPASS
    } else {
        &p.permission
    }
}

/// The box's text as one rule per non-empty line, trimmed, repeats dropped.
pub fn rules_of(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let r = line.trim();
        if !r.is_empty() && !out.iter().any(|x| x == r) {
            out.push(r.to_string());
        }
    }
    out
}

/// "owner/name" for a repo key (lib/irori.ts shortRepo).
pub fn short_repo(key: &str) -> String {
    if let Some(root) = key.strip_prefix("root:") {
        return format!("local repo {}", root.chars().take(7).collect::<String>());
    }
    let parts: Vec<&str> = key.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() > 2 {
        parts[parts.len() - 2..].join("/")
    } else {
        key.to_string()
    }
}

// ---- reads ------------------------------------------------------------------------

pub fn load(s: &mut State) {
    s.spawn(
        |w| w.agents_settings(""),
        |s, v| {
            s.settings.agents.settings = Some(v);
        },
    );
    load_spaces(s);
    load_limits(s);
    load_starts(s);
    load_names(s);
    load_running(s);
    s.spawn(
        |w| (w.agents_permissions(""), w.ask_repos()),
        |s, (p, repos)| {
            let a = &mut s.settings.agents;
            a.perms = Some(p);
            a.repos = repos;
        },
    );
    s.spawn(
        |w| w.agents_trusted(),
        |s, r| match r {
            Ok(rows) => {
                s.settings.agents.trusted = Some(rows);
                s.settings.agents.trusted_err.clear();
            }
            Err(e) => s.settings.agents.trusted_err = e,
        },
    );
}

fn load_spaces(s: &mut State) {
    let ids: Vec<String> = s
        .data
        .spaces
        .iter()
        .filter(|b| !b.leaving)
        .map(|b| b.board_id.clone())
        .collect();
    s.spawn(
        move |w| {
            ids.iter()
                .map(|id| (id.clone(), w.agents_settings(id)))
                .collect::<Vec<_>>()
        },
        |s, v| s.settings.agents.spaces = v.into_iter().collect(),
    );
}

fn load_limits(s: &mut State) {
    s.spawn(
        |w| w.agents_limits(),
        |s, l| {
            set_limits(s, l);
            s.settings.agents.limits_err.clear();
        },
    );
}

/// New limits; each cap box shows its device's saved cap unless it is
/// being edited.
fn set_limits(s: &mut State, l: AgentsLimits) {
    let a = &mut s.settings.agents;
    a.caps
        .retain(|dev, _| l.devices.iter().any(|d| &d.device == dev));
    for d in &l.devices {
        a.caps.entry(d.device.clone()).or_default().follow(d.cap);
    }
    a.limits = Some(l);
}

/// The names themes (card #702): read when the pane opens.
fn load_names(s: &mut State) {
    s.spawn(
        |w| w.names_themes(),
        |s, v| {
            s.settings.agents.names = Some(v);
            s.settings.agents.names_err.clear();
        },
    );
}

/// Turns one names theme on or off: shown at once, then saved; a refusal
/// says why and reads the themes again.
fn set_names_theme(s: &mut State, id: String, on: bool) {
    if let Some(v) = &mut s.settings.agents.names {
        for t in v.themes.iter_mut().filter(|t| t.id == id) {
            t.on = on;
        }
    }
    s.spawn(
        move |w| w.names_theme_set(&id, on),
        |s, r| match r {
            Ok(v) => {
                s.settings.agents.names = Some(v);
                s.settings.agents.names_err.clear();
            }
            Err(e) => {
                s.settings.agents.names_err = e;
                s.spawn(
                    |w| w.names_themes(),
                    |s, v| s.settings.agents.names = Some(v),
                );
            }
        },
    );
}

/// 1021 as "1,021" (toLocaleString in English).
pub fn thousands(n: u32) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn load_starts(s: &mut State) {
    s.spawn(|w| w.agents_starts(), set_starts);
}

fn set_starts(s: &mut State, v: AgentStartsView) {
    let a = &mut s.settings.agents;
    a.max.follow(v.max);
    a.starts = Some(v);
    a.starts_err.clear();
}

pub fn load_running(s: &mut State) {
    s.spawn(
        |w| w.agents_running(),
        |s, rows| {
            let a = &mut s.settings.agents;
            a.ending.retain(|k| rows.iter().any(|r| &r.key == k));
            a.running = Some(rows);
            a.running_err.clear();
        },
    );
}

/// Every few seconds while the pane is open: limits and running agents.
pub fn tick(s: &mut State) {
    load_limits(s);
    load_running(s);
}

pub fn on_event(s: &mut State, ev: &Event) {
    if super::open_pane(s) != Some("agents") {
        return;
    }
    if ev.is(EVENT_ROSTER_CHANGED) {
        load_running(s);
    } else if ev.is(irori_window::boards::EVENT_BOARDS_CHANGED) {
        load_spaces(s);
    }
}

/// Saves a change to a board's settings ("" for Personal).
fn set(s: &mut State, board: &str, c: AgentsChange) {
    let c = AgentsChange {
        board_id: board.to_string(),
        ..c
    };
    let board = board.to_string();
    s.spawn(
        move |w| w.agents_set(c),
        move |s, r| {
            let a = &mut s.settings.agents;
            match r {
                Ok(v) if board.is_empty() => {
                    a.settings = Some(v);
                    a.err.clear();
                }
                Ok(v) => {
                    a.space_err.remove(&board);
                    a.spaces.insert(board, v);
                }
                Err(e) if board.is_empty() => a.err = e,
                Err(e) => {
                    a.space_err.insert(board, e);
                }
            }
        },
    );
}

fn save_perms(s: &mut State, p: AgentPermissions, clear_rules: bool) {
    s.spawn(
        move |w| w.agents_permissions_set(p),
        move |s, r| {
            let a = &mut s.settings.agents;
            match r {
                Ok(p) => {
                    a.perms = Some(p);
                    a.perms_err.clear();
                    if clear_rules {
                        a.allow = None;
                        a.deny = None;
                    }
                }
                Err(e) => a.perms_err = e,
            }
        },
    );
}

// ---- the pane -------------------------------------------------------------------------

pub fn view(s: &State, f: &mut Frame) -> El {
    let a = &s.settings.agents;
    let mut out = col().key("settings-agents").w_full().gap(16.0);
    match &a.settings {
        None if !a.err.is_empty() => {
            out = out.child(muted_line(f, &format!("Could not load: {}", a.err)))
        }
        None => {
            out = out.child(loading(
                f,
                "settings-agents-loading",
                "Loading agent settings",
            ))
        }
        Some(st) => {
            out = out
                .child(toggle(
                    f,
                    "settings-agents-on",
                    "Start an agent for each card",
                    Some("In Personal, Irori starts a fresh agent for every card you post instead of waiting for a free one, and ends it when the card is done."),
                    st.on,
                    false,
                    |s, v| set(s, "", AgentsChange { on: Some(v), ..Default::default() }),
                ))
                .child(spaces_section(s, f))
                .child(starts_with(s, f, st))
                .child(names_section(s, f))
                .child(limits_section(s, f, st));
        }
    }
    out = out
        .child(running_section(s, f))
        .child(permissions_section(s, f))
        .child(trusted_section(s, f));
    if a.settings.is_some() && !a.err.is_empty() {
        out = out.child(error_line(f, &a.err));
    }
    out
}

/// Each shared space's own switch (card #573), off until turned on.
fn spaces_section(s: &State, f: &Frame) -> El {
    let spaces: Vec<_> = s.data.spaces.iter().filter(|b| !b.leaving).collect();
    if spaces.is_empty() {
        return el();
    }
    let a = &s.settings.agents;
    let mut sec = col()
        .key("settings-agents-spaces")
        .w_full()
        .gap(8.0)
        .role("group")
        .label("Spaces")
        .child(heading(f, "Spaces"))
        .child(muted_line(
            f,
            "Each space has its own switch, off until you turn it on. It runs your agents on your computers, for the cards you make there. A card someone gives you always asks you first, whatever the switch says.",
        ));
    for b in spaces {
        let name = super::plain_line(&b.name);
        let name = if name.is_empty() {
            "(unnamed)".to_string()
        } else {
            name
        };
        let Some(st) = a.spaces.get(&b.board_id) else {
            if let Some(e) = a.space_err.get(&b.board_id) {
                sec = sec.child(error_line(f, e));
            }
            continue;
        };
        let id = b.board_id.clone();
        let hint_text = if st.on {
            "On: Irori starts your agent for each card you make here."
        } else {
            "Off: Irori doesn't start agents for the cards you make here."
        };
        sec = sec.child(toggle(
            f,
            &format!("settings-space-agents-{}", b.board_id),
            &name,
            Some(hint_text),
            st.on,
            false,
            move |s, v| {
                set(
                    s,
                    &id,
                    AgentsChange {
                        on: Some(v),
                        ..Default::default()
                    },
                )
            },
        ));
        if let Some(e) = a.space_err.get(&b.board_id) {
            sec = sec.child(error_line(f, e));
        }
    }
    sec
}

fn starts_with(s: &State, f: &mut Frame, st: &AgentsSettings) -> El {
    let k = f.k;
    let models: Vec<(String, String)> = st
        .models
        .iter()
        .map(|m| (m.clone(), model_label(m, &st.default_model)))
        .collect();
    let efforts: Vec<(String, String)> = st
        .efforts
        .iter()
        .map(|e| (e.clone(), effort_label(e, &st.default_effort)))
        .collect();
    let model = choice(
        s,
        f,
        "settings-agents-model",
        "Model",
        &st.model,
        &models,
        180.0,
        false,
        |s, v| {
            set(
                s,
                "",
                AgentsChange {
                    model: Some(v.to_string()),
                    ..Default::default()
                },
            )
        },
    );
    let effort = choice(
        s,
        f,
        "settings-agents-effort",
        "Effort",
        &st.effort,
        &efforts,
        180.0,
        false,
        |s, v| {
            set(
                s,
                "",
                AgentsChange {
                    effort: Some(v.to_string()),
                    ..Default::default()
                },
            )
        },
    );
    let labeled = |name: &str, c: El| {
        col()
            .gap(4.0)
            .child(
                text(name.to_string())
                    .font(k.ty.ui_medium)
                    .color(k.c.text_base),
            )
            .child(c)
    };
    col()
        .key("settings-agents-starts-with")
        .w_full()
        .gap(8.0)
        .child(heading(f, "What a new agent starts with"))
        .child(muted_line(
            f,
            "Every agent Irori starts uses these. One already working keeps what it started with.",
        ))
        .child(
            row()
                .gap(16.0)
                .flex_wrap()
                .child(labeled("Model", model))
                .child(labeled("Effort", effort)),
        )
}

/// One limit: what it is, what it does, and its control (LimitRow).
fn limit_row(f: &Frame, key: &str, title: &str, hint_el: El, control: El) -> El {
    let k = f.k;
    row()
        .key(key.to_string())
        .gap(8.0)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    text(title.to_string())
                        .font(k.ty.ui)
                        .color(k.c.text_base)
                        .truncate(),
                )
                .child(hint_el),
        )
        .child(control)
}

fn limits_section(s: &State, f: &mut Frame, st: &AgentsSettings) -> El {
    let a = &s.settings.agents;
    let k = f.k;
    let mut sec = col()
        .key("settings-agents-limits")
        .w_full()
        .gap(8.0)
        .child(heading(f, "Limits"));
    match &a.limits {
        None => {
            sec = sec.child(muted_line(
                f,
                &if a.limits_err.is_empty() {
                    "Loading…".to_string()
                } else {
                    format!("Could not load: {}", a.limits_err)
                },
            ))
        }
        Some(l) => {
            let mut line = if l.today == 0 {
                "No agents started today.".to_string()
            } else {
                let split = l.today_here != l.today;
                format!(
                    "{} agent{} started today{}, using about {} at API prices{}. On a Claude plan that is plan usage, not a bill.",
                    l.today,
                    if l.today == 1 { "" } else { "s" },
                    if split {
                        format!(", {} on this computer", l.today_here)
                    } else {
                        String::new()
                    },
                    cost_label(l.cost_usd),
                    if split { " here" } else { "" },
                )
            };
            if l.queued > 0 {
                line.push_str(&format!(" {} queued for a slot.", l.queued));
            }
            sec = sec.child(muted_line(f, &line));
            if let Some(until) = l.held_until {
                sec = sec.child(
                    row()
                        .key("settings-agents-held")
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .flex_wrap()
                        .px(12.0)
                        .py(8.0)
                        .radius(6.0)
                        .fill(k.c.warning.bg)
                        .border(1.0, k.c.warning.border)
                        .role("status")
                        .child(
                            text(format!(
                                "A Claude usage limit is holding new agents on this computer ({}). Queued cards start then; agents that stopped on it aren't counted as crashes.",
                                usage_resets(until)
                            ))
                            .font(k.ty.ui)
                            .color(k.c.text_base)
                            .grow(1.0)
                            .min_w(0.0)
                            .wrap(),
                        )
                        .child(
                            Button::new("Try now")
                                .variant(ButtonVariant::Outline)
                                .help("Starts queued cards now. If the limit still stands, Irori holds again at the first one.")
                                .on_press(|s: &mut State| s.spawn(|w| w.agents_usage_release(), set_limits))
                                .build(k),
                        ),
                );
            }
            let mut rows = Vec::new();
            for d in &l.devices {
                let dev = d.device.clone();
                let dev2 = d.device.clone();
                let control = match a.caps.get(&d.device) {
                    Some(n) => number::field(
                        f,
                        number::Spec {
                            key: &format!("settings-cap-{}", d.device),
                            name: &format!("At most on {}", d.name),
                            ceiling: l.max_cap,
                            disabled: false,
                        },
                        n,
                        move |s: &mut State| s.settings.agents.caps.entry(dev.clone()).or_default(),
                        move |s: &mut State| save_cap(s, &dev2),
                    ),
                    None => el(),
                };
                let h = if d.cap > 0 {
                    format!("{} of {} running", d.running, d.cap)
                } else {
                    format!("{} running · starts no more", d.running)
                };
                rows.push(limit_row(
                    f,
                    &format!("settings-cap-row-{}", d.device),
                    &d.name,
                    hint(f, &h),
                    control,
                ));
            }
            sec = sec.child(boxed(f, "settings-agents-caps", rows));
        }
    }
    let idles: Vec<(String, String)> = st
        .idle_choices
        .iter()
        .map(|m| (m.to_string(), idle_label(*m, 30)))
        .collect();
    let crashes: Vec<(String, String)> = with_choice(TRIES_CHOICES, st.crash_limit)
        .into_iter()
        .map(|n| {
            (
                n.to_string(),
                count_label(n, st.default_crash_limit, "crash", "crashes"),
            )
        })
        .collect();
    let tries: Vec<(String, String)> = with_choice(TRIES_CHOICES, st.start_tries)
        .into_iter()
        .map(|n| {
            (
                n.to_string(),
                count_label(n, st.default_start_tries, "try", "tries"),
            )
        })
        .collect();
    let idle_c = choice(
        s,
        f,
        "settings-idle",
        "Idle limit",
        &st.idle_mins.to_string(),
        &idles,
        176.0,
        false,
        |s, v| {
            set(
                s,
                "",
                AgentsChange {
                    idle_mins: v.parse().ok(),
                    ..Default::default()
                },
            )
        },
    );
    let crash_c = choice(
        s,
        f,
        "settings-crashes",
        "Crashes in a row",
        &st.crash_limit.to_string(),
        &crashes,
        176.0,
        false,
        |s, v| {
            set(
                s,
                "",
                AgentsChange {
                    crash_limit: v.parse().ok(),
                    ..Default::default()
                },
            )
        },
    );
    let tries_c = choice(
        s,
        f,
        "settings-tries",
        "Failed starts in a row",
        &st.start_tries.to_string(),
        &tries,
        176.0,
        false,
        |s, v| {
            set(
                s,
                "",
                AgentsChange {
                    start_tries: v.parse().ok(),
                    ..Default::default()
                },
            )
        },
    );
    let mut rows = vec![
        limit_row(
            f,
            "settings-limit-idle",
            "Idle limit",
            hint(
                f,
                "An agent that waits this long on a question for you writes where it left off and goes. Your answer starts a fresh one.",
            ),
            idle_c,
        ),
        limit_row(
            f,
            "settings-limit-crashes",
            "Crashes in a row",
            hint(
                f,
                "After this many of a card's agents go away before finishing, Irori asks you before starting another. Each computer has its own number, for the cards it hands out.",
            ),
            crash_c,
        ),
        limit_row(
            f,
            "settings-limit-tries",
            "Failed starts in a row",
            hint(
                f,
                "After this many tries that couldn't start an agent for a card, Irori asks you before trying again.",
            ),
            tries_c,
        ),
    ];
    if let Some(r) = agent_starts_row(s, f) {
        rows.push(r);
    }
    sec = sec.child(boxed(f, "settings-agents-limits-list", rows));
    if a.limits.is_some() && !a.limits_err.is_empty() {
        sec = sec.child(error_line(f, &a.limits_err));
    }
    sec
}

pub const AGENT_STARTS_TITLE: &str = "My agents may start agents on my cards";
pub const AGENT_STARTS_HINT: &str = "Lets an agent start, list and end agents on your cards, by the same rules as your Start. Whatever those agents go on to start counts too. Agents can't change this.";

fn set_starts_remote(s: &mut State, on: bool, max: u32) {
    s.spawn(
        move |w| w.agents_starts_set(on, max),
        |s, r| match r {
            Ok(v) => set_starts(s, v),
            Err(e) => s.settings.agents.starts_err = e,
        },
    );
}

/// Enter or a blur in "At most": saves a new maximum (card #703).
fn save_max(s: &mut State) {
    let a = &mut s.settings.agents;
    let Some(st) = a.starts.clone() else { return };
    let Some(n) = a.max.save(st.max, st.ceiling) else {
        return;
    };
    let sent = a.max.edit.text.clone();
    s.spawn(
        move |w| w.agents_starts_set(st.on, n),
        move |s, r| {
            let a = &mut s.settings.agents;
            match r {
                Ok(v) => {
                    a.max.settle(&sent, v.max);
                    set_starts(s, v);
                }
                Err(e) => {
                    let old = a.starts.as_ref().map_or(st.max, |v| v.max);
                    a.max.settle(&sent, old);
                    a.starts_err = e;
                }
            }
        },
    );
}

/// Enter or a blur in a device's cap box: saves its new cap (card #703).
fn save_cap(s: &mut State, dev: &str) {
    let a = &mut s.settings.agents;
    let Some(l) = &a.limits else { return };
    let Some(d) = l.devices.iter().find(|d| d.device == dev) else {
        return;
    };
    let (cap, ceiling) = (d.cap, l.max_cap);
    let Some(box_) = a.caps.get_mut(dev) else {
        return;
    };
    let Some(n) = box_.save(cap, ceiling) else {
        return;
    };
    let sent = box_.edit.text.clone();
    let dev = dev.to_string();
    s.spawn(
        {
            let dev = dev.clone();
            move |w| w.agents_cap_set(&dev, n)
        },
        move |s, r| match r {
            Ok(l) => {
                let now = l
                    .devices
                    .iter()
                    .find(|d| d.device == dev)
                    .map_or(n, |d| d.cap);
                if let Some(b) = s.settings.agents.caps.get_mut(&dev) {
                    b.settle(&sent, now);
                }
                set_limits(s, l);
                s.settings.agents.limits_err.clear();
            }
            Err(e) => {
                let a = &mut s.settings.agents;
                if let Some(b) = a.caps.get_mut(&dev) {
                    b.settle(&sent, cap);
                }
                a.limits_err = e;
            }
        },
    );
}

/// Once a frame while the Agents pane is open: a number box that lost
/// its focus saves (WholeNumberField's onBlur).
pub fn poll(s: &mut State) -> bool {
    let max = s.settings.agents.max.blurred();
    if max {
        save_max(s);
    }
    let gone: Vec<String> = s
        .settings
        .agents
        .caps
        .iter_mut()
        .filter_map(|(dev, b)| b.blurred().then(|| dev.clone()))
        .collect();
    let any = max || !gone.is_empty();
    for dev in gone {
        save_cap(s, &dev);
    }
    any
}

/// "Agent names" (names-themes.tsx, card #702): which themes a new
/// agent's name comes from, a switch each, all on until turned off. When
/// the themes still on have no word free, or every theme is off, names
/// come from all of them; nobody already named is renamed.
fn names_section(s: &State, f: &Frame) -> El {
    let a = &s.settings.agents;
    let Some(v) = &a.names else {
        if a.names_err.is_empty() {
            return el();
        }
        return error_line(f, &a.names_err);
    };
    let mut sec = col()
        .key("settings-agents-names")
        .w_full()
        .gap(8.0)
        .role("group")
        .label("Agent names")
        .child(heading(f, "Agent names"))
        .child(muted_line(
            f,
            "Each new agent gets a short name nobody has used lately, from the themes that are on. When they run out, or every theme is off, names come from all of them. Agents already named keep their names.",
        ));
    for t in &v.themes {
        let id = t.id.clone();
        let h = format!("{} names, like {}", thousands(t.words), t.sample.join(", "));
        sec = sec.child(toggle(
            f,
            &format!("settings-names-{}", t.id),
            &t.label,
            Some(&h),
            t.on,
            false,
            move |s, on| set_names_theme(s, id.clone(), on),
        ));
    }
    if !a.names_err.is_empty() {
        sec = sec.child(error_line(f, &a.names_err));
    }
    sec
}

/// "My agents may start agents on my cards" (card #575).
fn agent_starts_row(s: &State, f: &Frame) -> Option<El> {
    let a = &s.settings.agents;
    let st = a.starts.as_ref()?;
    let k = f.k;
    let used = if st.on {
        format!("{} of {} in use.", st.used, st.max)
    } else {
        format!("Off; {} in use.", st.used)
    };
    let mut h = col().child(hint(f, &format!("{AGENT_STARTS_HINT} {used}")));
    if !a.starts_err.is_empty() {
        h = h.child(error_line(f, &a.starts_err));
    }
    let (on, max) = (st.on, st.max);
    let field = number::field(
        f,
        number::Spec {
            key: "settings-starts-max",
            name: "At most",
            ceiling: st.ceiling,
            disabled: !on,
        },
        &a.max,
        |s: &mut State| &mut s.settings.agents.max,
        save_max,
    );
    let control = row().gap(12.0).center_y().child(field).child(
        Switch::new(on)
            .label(AGENT_STARTS_TITLE)
            .hide_label()
            .on_toggle(move |s: &mut State| set_starts_remote(s, !on, max))
            .build(k),
    );
    Some(limit_row(
        f,
        "settings-limit-starts",
        AGENT_STARTS_TITLE,
        h,
        control,
    ))
}

fn tone_color(f: &Frame, t: Tone) -> Color {
    match t {
        Tone::Working | Tone::Job => status::working(),
        Tone::Warn => status::decide(),
        Tone::Idle => status::idle(),
        Tone::Muted => f.k.c.text_muted,
    }
}

/// A small status pill (the Badge in RunningRow).
fn pill(f: &Frame, label: &str, tone: Tone) -> El {
    let c = tone_color(f, tone);
    text(label.to_string())
        .font(f.k.ty.small)
        .color(c)
        .px(6.0)
        .radius(8.0)
        .fill(f.k.alpha(c, 0.14))
}

/// Every agent Irori started that is still running, and the cards one is
/// starting for (card #448): its card, device, model, Logs and End.
fn running_section(s: &State, f: &Frame) -> El {
    let a = &s.settings.agents;
    let k = f.k;
    let n = a.running.as_ref().map_or(0, Vec::len);
    let mut sec = col()
        .key("settings-agents-running")
        .w_full()
        .gap(8.0)
        .child(heading(f, &if n > 0 { format!("Running agents · {n}") } else { "Running agents".into() }))
        .child(muted_line(
            f,
            "Agents Irori started for your cards. End asks one to write where it left off on its card, then stops it.",
        ));
    let Some(rows) = &a.running else {
        return sec.child(if a.running_err.is_empty() {
            loading(f, "settings-running-loading", "Loading running agents")
        } else {
            muted_line(f, &format!("Could not load: {}", a.running_err))
        });
    };
    if rows.is_empty() {
        return sec.child(muted_line(f, "None running."));
    }
    let now = s.settings.now_ms;
    let mut out = Vec::new();
    for r in rows {
        let starting = r.status == "starting";
        // The live roster knows when a job began ("running a job · 12 min").
        let since = s
            .data
            .agents
            .iter()
            .find(|x| x.card.key == r.key)
            .and_then(|x| ms(&x.card.status_since));
        let st = agent_status(&Who::status(&r.status), since, now);
        let (word, tone) = (st.text.as_str(), st.tone);
        let mut name_row = row().gap(8.0).center_y().child(
            text(if starting {
                "Starting…".into()
            } else {
                r.name.clone()
            })
            .font(k.ty.ui_medium)
            .color(k.c.text_base)
            .truncate(),
        );
        if !word.is_empty() {
            name_row = name_row.child(pill(f, word, tone));
        }
        let since = if r.started_at > 0 { r.started_at } else { now };
        let words = col()
            .grow(1.0)
            .min_w(0.0)
            .gap(2.0)
            .child(name_row)
            .child(
                text(format!("{} {}", r.card_short, r.card_title))
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .truncate()
                    .w_full(),
            )
            .child(
                text(format!(
                    "{} · {} · {} · {}",
                    r.device,
                    r.model,
                    r.effort,
                    running_since(since, now)
                ))
                .font(k.ty.small)
                .color(k.c.text_muted)
                .truncate()
                .w_full(),
            );
        let actions = if a.ending.contains(&r.key) {
            text("ending").font(k.ty.small).color(k.c.text_muted)
        } else if starting {
            el()
        } else {
            let mut act = row().gap(6.0).center_y();
            if r.here {
                let key = r.key.clone();
                let open = a.logs.contains_key(&r.key);
                act = act.child(
                    Button::new(if open { "Hide logs" } else { "Logs" })
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut State| {
                            if s.settings.agents.logs.remove(&key).is_some() {
                                return;
                            }
                            let k2 = key.clone();
                            let k3 = key.clone();
                            s.spawn(
                                move |w| w.agents_logs(&k2),
                                move |s, r| {
                                    let text = r.unwrap_or_else(|e| e);
                                    s.settings.agents.logs.insert(k3, text);
                                },
                            );
                        })
                        .build(k),
                );
            } else {
                act = act.child(
                    text(format!("on {}", r.device))
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .help("Logs are on the device it runs on"),
                );
            }
            let there = if r.here {
                String::new()
            } else {
                format!("Irori asks {} to end it. ", r.device)
            };
            act.child(confirm_button(
                s,
                f,
                Confirm {
                    id: format!("end-{}", r.key),
                    question: format!(
                        "End {} on {}? {there}It writes where it left off on the card first, then Irori stops it and removes its session.",
                        r.name, r.card_short
                    ),
                    label: "End".into(),
                    danger: false,
                    act: Act::EndAgent(r.key.clone()),
                },
                "End",
                false,
            ))
        };
        let mut item = col().gap(6.0).child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(words)
                .child(actions),
        );
        if let Some(log) = a.logs.get(&r.key) {
            item = item.child(
                text(if log.trim().is_empty() {
                    "No output yet.".to_string()
                } else {
                    log.clone()
                })
                .font(k.ty.small.mono())
                .color(k.c.text_muted)
                .w_full()
                .wrap_anywhere()
                .selectable(true)
                .padding(8.0)
                .radius(6.0)
                .fill(k.c.layer_02),
            );
        }
        out.push(item.key(format!(
            "settings-running-{}",
            if r.key.is_empty() { &r.card_id } else { &r.key }
        )));
    }
    sec = sec.child(boxed(f, "settings-running-list", out));
    sec
}

/// "What a new agent may do" (card #449): the choice, the extra rules,
/// and whether each repo's agents open a PR or push to main.
fn permissions_section(s: &State, f: &mut Frame) -> El {
    let a = &s.settings.agents;
    let k = f.k;
    let mut sec = col()
        .key("settings-agents-perms")
        .w_full()
        .gap(8.0)
        .child(heading(f, "What a new agent may do"));
    let Some(p) = &a.perms else {
        return sec.child(if a.perms_err.is_empty() {
            loading(f, "settings-perms-loading", "Loading agent permissions")
        } else {
            muted_line(
                f,
                &format!("Could not load what a new agent may do: {}", a.perms_err),
            )
        });
    };
    let current = choice_of(p).to_string();
    let choices: Vec<(String, String)> = PERMISSION_CHOICES
        .iter()
        .map(|(v, l)| (v.to_string(), l.to_string()))
        .collect();
    let pc = p.clone();
    sec = sec
        .child(
            text("Permissions")
                .font(k.ty.ui_medium)
                .color(k.c.text_base),
        )
        .child(choice(
            s,
            f,
            "settings-permission",
            "Permissions",
            &current,
            &choices,
            300.0,
            false,
            move |s, v| {
                save_perms(
                    s,
                    AgentPermissions {
                        permission: v.to_string(),
                        ..pc.clone()
                    },
                    false,
                )
            },
        ));
    let mut note = "In a shared space, or when a card quotes someone else, an agent never runs without asking: it gets the checks, git and Irori rules instead.".to_string();
    if current != BYPASS {
        note.push_str(" Force pushes and rm -rf are never allowed.");
    }
    sec = sec.child(muted_line(f, &note));
    if current == LIMITED {
        let saved_allow = p.allow.join("\n");
        let saved_deny = p.deny.join("\n");
        let allow = a
            .allow
            .clone()
            .unwrap_or_else(|| TextEdit::new(saved_allow.clone()));
        let deny = a
            .deny
            .clone()
            .unwrap_or_else(|| TextEdit::new(saved_deny.clone()));
        let dirty = allow.text != saved_allow || deny.text != saved_deny;
        let boxf = |label: &str, edit: &TextEdit, ph: &str, which: bool| {
            let sa = saved_allow.clone();
            let sd = saved_deny.clone();
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(muted_line(f, label))
                .child(
                    TextArea::new(edit)
                        .label(label.to_string())
                        .placeholder(ph.to_string())
                        .rows(3)
                        .on_edit(move |s: &mut State| {
                            let a = &mut s.settings.agents;
                            if which {
                                a.allow.get_or_insert_with(|| TextEdit::new(sa.clone()))
                            } else {
                                a.deny.get_or_insert_with(|| TextEdit::new(sd.clone()))
                            }
                        })
                        .build(k),
                )
        };
        sec = sec.child(
            row()
                .key("settings-perm-rules")
                .w_full()
                .gap(8.0)
                .child(boxf(
                    "Also allow, one rule per line",
                    &allow,
                    "Bash(cargo test:*)",
                    true,
                ))
                .child(boxf(
                    "Never allow, one rule per line",
                    &deny,
                    "Bash(cargo publish:*)",
                    false,
                )),
        );
        if dirty {
            let pc = p.clone();
            sec = sec.child(
                row()
                    .gap(6.0)
                    .child(
                        Button::new("Save rules")
                            .variant(ButtonVariant::Contrast)
                            .on_press(move |s: &mut State| {
                                let a = &s.settings.agents;
                                let allow = a
                                    .allow
                                    .as_ref()
                                    .map_or_else(|| pc.allow.clone(), |e| rules_of(&e.text));
                                let deny = a
                                    .deny
                                    .as_ref()
                                    .map_or_else(|| pc.deny.clone(), |e| rules_of(&e.text));
                                save_perms(
                                    s,
                                    AgentPermissions {
                                        allow,
                                        deny,
                                        ..pc.clone()
                                    },
                                    true,
                                )
                            })
                            .build(k),
                    )
                    .child(
                        Button::new("Cancel")
                            .variant(ButtonVariant::Ghost)
                            .on_press(|s: &mut State| {
                                s.settings.agents.allow = None;
                                s.settings.agents.deny = None;
                            })
                            .build(k),
                    ),
            );
        }
    }
    let keys: BTreeSet<String> = a
        .repos
        .iter()
        .map(|r| r.repo_key.clone())
        .chain(p.merge.keys().cloned())
        .collect();
    if !keys.is_empty() {
        let label = |key: &str| {
            a.repos
                .iter()
                .find(|r| r.repo_key == key)
                .map(|r| r.label.clone())
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| short_repo(key))
        };
        let merges: Vec<(String, String)> = vec![
            (MERGE_PR.into(), "Opens a PR".into()),
            (MERGE_MAIN.into(), "Pushes to main".into()),
        ];
        let mut list = col().gap(6.0).child(
            text("When its work is done")
                .font(k.ty.small_medium)
                .color(k.c.text_base),
        );
        for key in keys {
            let l = label(&key);
            let value = p
                .merge
                .get(&key)
                .cloned()
                .unwrap_or_else(|| MERGE_PR.into());
            let pc = p.clone();
            let kk = key.clone();
            let c = choice(
                s,
                f,
                &format!("settings-merge-{key}"),
                &format!("When an agent in {l} is done"),
                &value,
                &merges,
                160.0,
                false,
                move |s, v| {
                    let mut merge = pc.merge.clone();
                    merge.insert(kk.clone(), v.to_string());
                    save_perms(
                        s,
                        AgentPermissions {
                            merge,
                            ..pc.clone()
                        },
                        false,
                    )
                },
            );
            list = list.child(
                row()
                    .key(format!("settings-merge-row-{key}"))
                    .w_full()
                    .gap(12.0)
                    .center_y()
                    .child(
                        text(l)
                            .font(k.ty.ui.mono())
                            .color(k.c.text_base)
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0)
                            .help(key.clone()),
                    )
                    .child(c),
            );
        }
        sec = sec.child(list);
    }
    if !a.perms_err.is_empty() {
        sec = sec.child(error_line(f, &a.perms_err));
    }
    sec
}

/// "Trusted repos for agents" (card #456): this computer's repos and
/// whether Claude Code trusts each.
fn trusted_section(s: &State, f: &Frame) -> El {
    let a = &s.settings.agents;
    let k = f.k;
    let sec = col()
        .key("settings-agents-trusted")
        .w_full()
        .gap(8.0)
        .child(heading(f, "Trusted repos for agents"))
        .child(muted_line(
            f,
            "An agent Irori starts runs only in a repo Claude Code trusts on that computer. These are this computer's; your other computers each list their own.",
        ));
    let Some(rows) = &a.trusted else {
        return sec.child(if a.trusted_err.is_empty() {
            loading(f, "settings-trusted-loading", "Loading trusted repos")
        } else {
            muted_line(f, &format!("Could not load: {}", a.trusted_err))
        });
    };
    if rows.is_empty() {
        return sec.child(muted_line(f, "No repos on this computer yet."));
    }
    let mut out = Vec::new();
    for r in rows {
        let (ic, color) = if r.trusted {
            (&icons::SHIELD_CHECK, status::working())
        } else {
            (&icons::SHIELD_ALERT, status::decide())
        };
        let right = if r.trusted {
            text("trusted").font(k.ty.small).color(k.c.text_muted)
        } else {
            confirm_button(
                s,
                f,
                Confirm {
                    id: format!("trust-{}", r.repo_key),
                    question: format!("Trust {} for agents? {}", r.label, r.what),
                    label: "Trust".into(),
                    danger: false,
                    act: Act::TrustRepo(r.repo_key.clone()),
                },
                "Trust…",
                false,
            )
        };
        out.push(
            row()
                .key(format!("settings-trusted-{}", r.repo_key))
                .gap(12.0)
                .center_y()
                .child(icon(ic, 16.0).color(color))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text(r.label.clone())
                                .font(k.ty.ui.mono())
                                .color(k.c.text_base)
                                .truncate(),
                        )
                        .child(
                            row()
                                .gap(4.0)
                                .center_y()
                                .help(r.folder.clone())
                                .child(icon(&icons::FOLDER, 12.0).color(k.c.text_muted))
                                .child(
                                    text(r.folder.clone())
                                        .font(k.ty.small)
                                        .color(k.c.text_muted)
                                        .truncate()
                                        .min_w(0.0),
                                ),
                        ),
                )
                .child(right),
        );
    }
    sec.child(boxed(f, "settings-trusted-list", out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_marked_the_rest_named() {
        assert_eq!(model_label("sonnet", "sonnet"), "Sonnet (default)");
        assert_eq!(model_label("opus", "sonnet"), "Opus");
        assert_eq!(effort_label("xhigh", "medium"), "Extra high");
        assert_eq!(effort_label("medium", "medium"), "Medium (default)");
    }

    #[test]
    fn idle_limits_in_minutes_or_hours_30_marked() {
        assert_eq!(idle_label(10, 30), "10 minutes");
        assert_eq!(idle_label(30, 30), "30 minutes (default)");
        assert_eq!(idle_label(60, 30), "1 hour");
        assert_eq!(idle_label(240, 30), "4 hours");
    }

    #[test]
    fn an_unknown_value_shows_as_itself() {
        assert_eq!(model_label("fable", "sonnet"), "fable");
    }

    #[test]
    fn running_since_minutes_then_hours() {
        assert_eq!(running_since(0, 30_000), "just now");
        assert_eq!(running_since(0, 6 * 60_000), "6 min");
        assert_eq!(running_since(0, 130 * 60_000), "2 h");
    }

    #[test]
    fn counts_costs_and_choices() {
        assert_eq!(count_label(1, 2, "crash", "crashes"), "1 crash");
        assert_eq!(count_label(2, 2, "crash", "crashes"), "2 crashes (default)");
        assert_eq!(cost_label(0.0), "$0");
        assert_eq!(cost_label(0.004), "under $0.01");
        assert_eq!(cost_label(3.421), "$3.42");
        assert_eq!(with_choice(TRIES_CHOICES, 4), vec![1, 2, 3, 4, 5, 10, 20]);
        assert_eq!(with_choice(TRIES_CHOICES, 3), TRIES_CHOICES.to_vec());
    }

    #[test]
    fn an_empty_permission_shows_as_bypass() {
        let p = AgentPermissions::default();
        assert_eq!(choice_of(&p), BYPASS);
        let p = AgentPermissions {
            permission: LIMITED.into(),
            ..Default::default()
        };
        assert_eq!(choice_of(&p), LIMITED);
    }

    #[test]
    fn rules_are_one_per_line_trimmed_blanks_and_repeats_dropped() {
        assert_eq!(
            rules_of(" Bash(cargo test:*) \n\nBash(cargo test:*)\nBash(bun test:*)\n"),
            vec!["Bash(cargo test:*)", "Bash(bun test:*)"]
        );
    }

    #[test]
    fn word_counts_read_with_thousands() {
        assert_eq!(thousands(29), "29");
        assert_eq!(thousands(256), "256");
        assert_eq!(thousands(1021), "1,021");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn repos_read_short() {
        assert_eq!(
            short_repo("remote:github.com/sample-org/irori"),
            "sample-org/irori"
        );
        assert_eq!(short_repo("root:abcdef0123"), "local repo abcdef0");
    }
}
