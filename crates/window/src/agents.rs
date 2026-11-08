//! Settings › Agents (card #448, docs/design/spawned-agents.md §10, sa-6):
//! a board's spawned-agent switch and the model and effort its agents
//! start with. Set in the app only, never by a tool (decision 23). Each
//! write changes only its own fields of the board's `spawn.set` row, so
//! the permission fields part 5 keeps there are left as they are.

use irori_core::spawn::{self, SpawnSetting};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Window;

/// One agent Irori started, or is starting, for a card (card #448): a row
/// of Settings › Agents' Running agents list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunningAgent {
    /// The agent's key; `""` while it is still starting.
    pub key: String,
    pub name: String,
    /// starting, busy, idle, unresponsive.
    pub status: String,
    pub card_id: String,
    /// `#N`.
    pub card_short: String,
    pub card_title: String,
    /// "<its own name> (this one)" or the other device's name (card #489).
    pub device: String,
    /// It runs here, so End works from this window.
    pub here: bool,
    pub model: String,
    pub effort: String,
    /// When Irori asked for it, in ms.
    #[specta(type = f64)]
    pub started_at: i64,
}

/// Token counts for one session (card #1020), a transcript's, so only this
/// device's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentTokens {
    /// Every kind added up.
    #[specta(type = f64)]
    pub total: u64,
    #[specta(type = f64)]
    pub input: u64,
    #[specta(type = f64)]
    pub output: u64,
    #[specta(type = f64)]
    pub cache_read: u64,
    #[specta(type = f64)]
    pub cache_write: u64,
}

/// What is known of one agent's model, effort and tokens, for its hover card
/// (card #1020). A field that isn't known is empty or left out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsage {
    /// The model Irori asked for ("sonnet"), else the one its transcript
    /// last answered with ("claude-opus-5-5"); `""` when unknown.
    pub model: String,
    /// The effort Irori asked for; `""` for an agent it didn't start.
    pub effort: String,
    /// Read from its transcript: only for an agent on this device.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub tokens: Option<AgentTokens>,
}

/// What Settings › Agents shows for one board.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentsSettings {
    /// `""`: the private board.
    pub board_id: String,
    /// Irori starts an agent for each card instead of waiting for a free one.
    pub on: bool,
    /// He has set the switch himself; until then it is the default (on
    /// for the private board in the app), and Home shows the first-run
    /// notice.
    pub chosen: bool,
    /// What a spawn passes: the board's pick, or the default.
    pub model: String,
    pub effort: String,
    /// The choices Settings › Agents offers.
    pub models: Vec<String>,
    pub efforts: Vec<String>,
    pub default_model: String,
    pub default_effort: String,
    /// What a light card's agent starts with (card #963): the board's
    /// pick, or what the light row works out by itself (Sonnet at no more
    /// than medium effort).
    pub light_model: String,
    pub light_effort: String,
    /// A heavy card's: the board's pick, or Opus at no less than high.
    pub heavy_model: String,
    pub heavy_effort: String,
    /// The office assistant's chat model and effort (card #1014), and the
    /// models it offers (the efforts are `efforts`).
    pub assistant_model: String,
    pub assistant_effort: String,
    pub assistant_models: Vec<String>,
    /// The idle limit in minutes (card #456, sa-6): how long a spawned
    /// agent waits on an unanswered question before it hands off and goes.
    pub idle_mins: u32,
    /// The idle limits Settings › Agents offers, in minutes.
    pub idle_choices: Vec<u32>,
    /// How many of a card's agents may crash in a row before Irori stops
    /// and asks (card #576).
    pub crash_limit: u32,
    /// How many failed starts in a row before Irori stops and asks.
    pub start_tries: u32,
    /// Their defaults, for the choices' labels.
    pub default_crash_limit: u32,
    pub default_start_tries: u32,
}

/// "My agents may start agents on my cards" (card #575): the person's, on
/// all their devices, set here and never by a tool.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentStartsView {
    pub on: bool,
    /// The most agents their agents may have started at once.
    pub max: u32,
    /// How many count toward it now.
    pub used: u32,
    /// The highest maximum the app accepts.
    pub ceiling: u32,
}

/// agents:paused: Pause agents was turned on or off here (card #665).
pub const EVENT_AGENTS_PAUSED: &str = "agents:paused";

/// Pause agents (card #665): this device starts or restarts no agent while
/// it is on. Each device has its own; the person's agents may set it too.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentsPauseView {
    pub on: bool,
    /// When it went on, in ms; 0 when off.
    #[specta(type = f64)]
    pub since: i64,
    /// Who set it last, for people: "the person on <device>" or an agent's
    /// name; empty when never set.
    pub by: String,
}

/// One names theme in Settings › Agents (card #702), with its switch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NamesThemeChoice {
    pub id: String,
    pub label: String,
    /// New names may come from it: every theme is on until the person
    /// turns it off.
    pub on: bool,
    /// How many words it has.
    pub words: u32,
    /// A few of them, for the hint.
    pub sample: Vec<String>,
}

/// The themes new agents' names come from (card #702): the person's, on all
/// their devices, set here and never by a tool.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NamesThemesView {
    pub themes: Vec<NamesThemeChoice>,
}

/// The idle limits offered, in minutes; 30 is the default.
pub const IDLE_CHOICES: [u32; 5] = [10, 30, 60, 120, 240];

/// The person's change: the fields left `None` keep what is stored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentsChange {
    #[serde(default)]
    pub board_id: String,
    #[serde(default)]
    pub on: Option<bool>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// A light or heavy card's model and effort (card #963); `""`, or the
    /// value the row works out by itself, stores nothing, so the row keeps
    /// following the board's normal pick and the defaults.
    #[serde(default)]
    pub light_model: Option<String>,
    #[serde(default)]
    pub light_effort: Option<String>,
    #[serde(default)]
    pub heavy_model: Option<String>,
    #[serde(default)]
    pub heavy_effort: Option<String>,
    /// The office assistant's model and effort (card #1014); the default
    /// stores nothing.
    #[serde(default)]
    pub assistant_model: Option<String>,
    #[serde(default)]
    pub assistant_effort: Option<String>,
    #[serde(default)]
    pub idle_mins: Option<u32>,
    #[serde(default)]
    pub crash_limit: Option<u32>,
    #[serde(default)]
    pub start_tries: Option<u32>,
}

/// One row of Settings › Agents' history (card #966): attempts the person
/// answered on one weight, model and effort.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunHistoryRow {
    pub weight: String,
    pub model: String,
    pub effort: String,
    pub runs: u32,
    pub first_try: u32,
    pub sent_back: u32,
    pub advised: u32,
}

pub(crate) fn view(s: &SpawnSetting, chosen: bool) -> AgentsSettings {
    let (light_model, light_effort) = s.pick(coord::WEIGHT_LIGHT);
    let (heavy_model, heavy_effort) = s.pick(coord::WEIGHT_HEAVY);
    AgentsSettings {
        board_id: s.board_id.clone(),
        on: s.on,
        chosen,
        model: s.model().into(),
        effort: s.effort().into(),
        models: spawn::MODELS.iter().map(|m| (*m).into()).collect(),
        efforts: spawn::EFFORTS.iter().map(|e| (*e).into()).collect(),
        default_model: spawn::DEFAULT_MODEL.into(),
        default_effort: spawn::DEFAULT_EFFORT.into(),
        light_model: light_model.into(),
        light_effort: light_effort.into(),
        heavy_model: heavy_model.into(),
        heavy_effort: heavy_effort.into(),
        assistant_model: s.assistant_model().into(),
        assistant_effort: s.assistant_effort().into(),
        assistant_models: spawn::ASSISTANT_MODELS
            .iter()
            .map(|m| (*m).into())
            .collect(),
        idle_mins: (s.idle_limit().as_secs() / 60) as u32,
        idle_choices: IDLE_CHOICES.to_vec(),
        crash_limit: or_default(s.crash_limit, i64::from(coord::MAX_SPAWN_CRASHES)),
        start_tries: or_default(
            s.start_tries,
            i64::from(irori_core::spawn_stop::SPAWN_TRIES),
        ),
        default_crash_limit: coord::MAX_SPAWN_CRASHES,
        default_start_tries: irori_core::spawn_stop::SPAWN_TRIES,
    }
}

/// A limit as shown: its stored number, or its default when none is.
fn or_default(n: i64, def: i64) -> u32 {
    (if n > 0 { n } else { def }).clamp(0, i64::from(u32::MAX)) as u32
}

/// The stored row with the change applied.
pub(crate) fn apply(mut s: SpawnSetting, c: AgentsChange) -> SpawnSetting {
    if let Some(on) = c.on {
        s.on = on;
    }
    if let Some(m) = c.model {
        s.model = m;
    }
    if let Some(e) = c.effort {
        s.effort = e;
    }
    // After the normal pick, which the light and heavy rows follow.
    let rows: [(Option<String>, &str, bool); 4] = [
        (c.light_model, coord::WEIGHT_LIGHT, true),
        (c.light_effort, coord::WEIGHT_LIGHT, false),
        (c.heavy_model, coord::WEIGHT_HEAVY, true),
        (c.heavy_effort, coord::WEIGHT_HEAVY, false),
    ];
    for (v, level, model) in rows {
        let Some(v) = v else { continue };
        // What the row works out with nothing stored: picking it stores
        // nothing, so a later change to the normal pick or a default
        // reaches it (as the idle limit's default is stored as 0).
        let mut empty = s.clone();
        *weight_field(&mut empty, level, model) = String::new();
        let (m, e) = empty.pick(level);
        let by_itself = if model { m } else { e };
        let v = if v == by_itself { String::new() } else { v };
        *weight_field(&mut s, level, model) = v;
    }
    // Stored as "" at the default, so a later default change reaches it.
    let unless = |v: String, def: &str| if v == def { String::new() } else { v };
    if let Some(m) = c.assistant_model {
        s.assistant_model = unless(m, spawn::ASSISTANT_MODEL);
    }
    if let Some(e) = c.assistant_effort {
        s.assistant_effort = unless(e, spawn::ASSISTANT_EFFORT);
    }
    if let Some(m) = c.idle_mins {
        // The default is stored as 0, so a later default change reaches it.
        let def = (spawn::IDLE_LIMIT.as_secs() / 60) as u32;
        s.idle_mins = if m == def { 0 } else { i64::from(m) };
    }
    // A default is stored as 0, as the idle limit's is, so a later default
    // change reaches it.
    let stored = |n: u32, def: i64| {
        let n = i64::from(n);
        if n == def { 0 } else { n }
    };
    if let Some(n) = c.crash_limit {
        s.crash_limit = stored(n, i64::from(coord::MAX_SPAWN_CRASHES));
    }
    if let Some(n) = c.start_tries {
        s.start_tries = stored(n, i64::from(irori_core::spawn_stop::SPAWN_TRIES));
    }
    s
}

/// The light or heavy row's model or effort field.
fn weight_field<'a>(s: &'a mut SpawnSetting, level: &str, model: bool) -> &'a mut String {
    match (level == coord::WEIGHT_HEAVY, model) {
        (false, true) => &mut s.light_model,
        (false, false) => &mut s.light_effort,
        (true, true) => &mut s.heavy_model,
        (true, false) => &mut s.heavy_effort,
    }
}

/// One device's cap in Settings › Agents (card #450, sa-6).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCap {
    pub device: String,
    /// "<its own name> (this one)" or the other device's name.
    pub name: String,
    pub here: bool,
    /// The cap: his pick, else its default (6 on a Windows PC, 12 on a
    /// Mac). 0 is his: that device starts none.
    pub cap: u32,
    /// He picked it.
    pub chosen: bool,
    /// Agents Irori started that run there now, and those starting.
    pub running: u32,
}

/// Settings › Agents' limits (card #450, sa-6): caps per device, today's
/// count and cost, and a usage-limit hold.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentsLimits {
    pub devices: Vec<DeviceCap>,
    /// The highest cap offered.
    pub max_cap: u32,
    /// Agents Irori started today for the private board, every device's.
    pub today: u32,
    /// Of those, the ones on this device, which are costed.
    pub today_here: u32,
    /// What today's agents here used, in dollars at API prices (on a Claude
    /// plan that is plan usage, not a bill).
    pub cost_usd: f64,
    /// While a Claude usage limit holds new spawns here: until when, in ms.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional, type = f64)]
    pub held_until: Option<i64>,
    /// Cards queued here for a slot.
    pub queued: u32,
}

static COSTS: std::sync::LazyLock<crate::spawn_cost::CostCache> =
    std::sync::LazyLock::new(Default::default);

/// The person's choice of idle limit: one of [`IDLE_CHOICES`].
fn valid_idle(c: &AgentsChange) -> Result<(), String> {
    match c.idle_mins {
        Some(m) if !IDLE_CHOICES.contains(&m) => Err(format!(
            "an idle limit is one of {IDLE_CHOICES:?} minutes, not {m}"
        )),
        _ => Ok(()),
    }
}

impl Window {
    /// Settings › Agents for a board (`""`: the private board).
    /// How this person's cards ended, by weight, model and effort (card
    /// #966): their private board's.
    pub fn agents_history(&self) -> Vec<RunHistoryRow> {
        self.core
            .coord()
            .run_tally()
            .into_iter()
            .map(|r| RunHistoryRow {
                weight: r.weight,
                model: r.model,
                effort: r.effort,
                runs: r.runs,
                first_try: r.first_try,
                sent_back: r.sent_back,
                advised: r.advised,
            })
            .collect()
    }

    pub fn agents_settings(&self, board_id: &str) -> AgentsSettings {
        view(
            &self.core.spawn_setting(board_id),
            self.core.spawn_switch_set(board_id),
        )
    }

    /// The person's End on an agent Irori started (card #448, sa-2): its
    /// hand-off, then stop and rm. From any of the person's devices: one
    /// that doesn't run it asks the device that does (part 11).
    pub fn agents_end(&self, key: &str) -> Result<(), String> {
        self.core.request_end(key)
    }

    /// A spawned agent's key, checked: Irori started it and it runs on
    /// this device. Its card's id and its session id.
    fn spawned_here(&self, key: &str) -> Result<(String, String), String> {
        let (dev, session) = key.split_once('/').unwrap_or(("", key));
        if dev != self.core.device_id() {
            return Err("Open it from the device it runs on".into());
        }
        let sp = self
            .core
            .coord()
            .spawned(key)
            .ok_or_else(|| format!("{key} wasn't started by Irori"))?;
        Ok((sp.task_id, session.to_string()))
    }

    /// Logs on an agent Irori started (card #456, sa-2): its recent
    /// terminal output, read only.
    pub fn agents_logs(&self, key: &str) -> Result<String, String> {
        let (_, session) = self.spawned_here(key)?;
        crate::spawn::logs(&session)
    }

    /// Attach on an agent Irori started (card #456, sa-2): a new tab in its
    /// card's terminal running `claude attach <id>`, so typing there talks
    /// to it. The agent stays spawned; closing the tab detaches.
    pub fn agents_attach(&self, key: &str, cols: u16, rows: u16) -> Result<term::TermInfo, String> {
        let (task_id, session) = self.spawned_here(key)?;
        let argv = crate::spawn::attach_argv(&session)?;
        let cwd = self
            .core
            .mesh_agents()
            .into_iter()
            .find(|a| a.key == key)
            .map(|a| a.cwd)
            .filter(|d| !d.is_empty() && std::path::Path::new(d).is_dir());
        self.core.terms().open(term::OpenSpec {
            scope: term::Scope::Card(task_id),
            cols,
            rows,
            cwd,
            command: Some(argv),
        })
    }

    /// An agent's model, effort and tokens (card #1020). The model and effort
    /// are the spawn record's, which every device has for an agent Irori
    /// started; the tokens come from the transcript, so only for an agent on
    /// this device. What isn't known is left empty.
    pub fn agent_usage(&self, key: &str) -> AgentUsage {
        let co = self.core.coord();
        let req = co.spawned(key).and_then(|sp| {
            co.get_task(&sp.task_id).and_then(|t| {
                t.spawns
                    .iter()
                    .find(|s| s.key == key)
                    .or_else(|| t.spawn_request())
                    .cloned()
            })
        });
        let mut out = AgentUsage {
            model: req.as_ref().map(|r| r.model.clone()).unwrap_or_default(),
            effort: req.as_ref().map(|r| r.effort.clone()).unwrap_or_default(),
            tokens: None,
        };
        let (dev, session) = key.split_once('/').unwrap_or(("", key));
        if dev == self.core.device_id()
            && let Some(u) = COSTS.usage(&self.core.claude_dir(), session)
        {
            if out.model.is_empty() {
                out.model = u.model;
            }
            out.tokens = Some(AgentTokens {
                total: u.tokens.total(),
                input: u.tokens.input,
                output: u.tokens.output,
                cache_read: u.tokens.cache_read,
                cache_write: u.tokens.cache_write,
            });
        }
        out
    }

    /// Every agent Irori started that is still running, and every card an
    /// agent is starting for, newest first (card #448).
    pub fn agents_running(&self) -> Vec<RunningAgent> {
        let p = self.people();
        let co = self.core.coord();
        let me = self.core.device_id().to_string();
        let mut out = Vec::new();
        let mut cards: std::collections::HashSet<String> = Default::default();
        for a in self.core.mesh_agents() {
            if a.status == "gone" || !co.is_spawned(&a.key) {
                continue;
            }
            let Some(sp) = co.spawned(&a.key) else {
                continue;
            };
            let t = co.get_task(&sp.task_id);
            let req = t.as_ref().and_then(|t| {
                t.spawns
                    .iter()
                    .find(|s| s.key == a.key)
                    .or_else(|| t.spawn_request())
                    .cloned()
            });
            let dev = if a.device_id.is_empty() {
                sp.device.clone()
            } else {
                a.device_id.clone()
            };
            cards.insert(sp.task_id.clone());
            out.push(RunningAgent {
                key: a.key.clone(),
                name: if a.name.is_empty() {
                    a.key.clone()
                } else {
                    a.name.clone()
                },
                status: a.status.clone(),
                card_id: sp.task_id.clone(),
                card_short: t.as_ref().map(|t| t.short_ref()).unwrap_or_default(),
                card_title: t.as_ref().map(|t| t.title.clone()).unwrap_or_default(),
                device: p.where_(&dev),
                here: dev == me,
                model: req.as_ref().map(|r| r.model.clone()).unwrap_or_default(),
                effort: req.as_ref().map(|r| r.effort.clone()).unwrap_or_default(),
                started_at: sp.at.unix_milli_value(),
            });
        }
        for t in co.list_tasks(&coord::TaskFilter {
            open: true,
            ..coord::TaskFilter::default()
        }) {
            let Some(r) = t.spawn_request().filter(|r| r.pending()) else {
                continue;
            };
            if cards.contains(&t.id) {
                continue;
            }
            out.push(RunningAgent {
                status: "starting".into(),
                card_id: t.id.clone(),
                card_short: t.short_ref(),
                card_title: t.title.clone(),
                device: p.where_(&r.device),
                here: r.device == me,
                model: r.model.clone(),
                effort: r.effort.clone(),
                started_at: r.at.unix_milli_value(),
                ..RunningAgent::default()
            });
        }
        out.sort_by_key(|r| std::cmp::Reverse(r.started_at));
        out
    }

    /// "My agents may start agents on my cards", as Settings › Agents shows it.
    pub fn agents_starts(&self) -> AgentStartsView {
        let u = self.core.agent_starts();
        AgentStartsView {
            on: u.on,
            max: u.max.max(0) as u32,
            used: u.used.max(0) as u32,
            ceiling: irori_core::spawn_agents::AGENT_STARTS_CEILING as u32,
        }
    }

    /// The person changes it: the app only, never a tool.
    pub fn agents_starts_set(&self, on: bool, max: u32) -> Result<AgentStartsView, String> {
        self.core.set_agent_starts(on, i64::from(max))?;
        Ok(self.agents_starts())
    }

    /// "Agents review other people's PRs without asking" (merge-desk
    /// §12.8): on unless the person turned it off.
    pub fn agents_reviews(&self) -> bool {
        self.core.reviews_unasked()
    }

    /// The person changes it: the app only, never a tool.
    pub fn agents_reviews_set(&self, on: bool) -> Result<bool, String> {
        self.core.set_reviews_unasked(on)?;
        Ok(self.agents_reviews())
    }

    /// Pause agents, as the top nav shows it (card #665).
    pub fn agents_pause(&self) -> AgentsPauseView {
        let p = self.core.agents_pause();
        AgentsPauseView {
            on: p.on,
            since: if p.on { p.since.unix_milli_value() } else { 0 },
            by: self.core.agents_pause_by(&p.by),
        }
    }

    /// The themes new agents' names come from (card #702), each with its
    /// switch.
    pub fn names_themes(&self) -> NamesThemesView {
        let off = self.core.names_themes_off();
        NamesThemesView {
            themes: coord::names::THEMES
                .iter()
                .map(|t| NamesThemeChoice {
                    id: t.id.into(),
                    label: t.label.into(),
                    on: !off.iter().any(|o| o == t.id),
                    words: t.words.len() as u32,
                    sample: t.sample.iter().map(|w| (*w).into()).collect(),
                })
                .collect(),
        }
    }

    /// The person turns a theme on or off: the app only, never a tool.
    pub fn names_theme_set(&self, id: &str, on: bool) -> Result<NamesThemesView, String> {
        self.core.set_names_theme(crate::trim(id), on)?;
        Ok(self.names_themes())
    }

    /// The person turns Pause agents on or off for this device: a graceful
    /// stop, its agents wound down (card #787).
    pub fn agents_pause_set(&self, on: bool) -> Result<AgentsPauseView, String> {
        self.pause_here(on)?;
        let v = self.agents_pause();
        self.emit(EVENT_AGENTS_PAUSED, &v);
        Ok(v)
    }

    /// Settings › Agents' caps, today's count and cost, and any usage-limit
    /// hold (card #450).
    pub fn agents_limits(&self) -> AgentsLimits {
        let p = self.people();
        let co = self.core.coord();
        let me = self.core.device_id().to_string();
        let q = self.core.spawn_queue();
        let today = self.core.spawned_today();
        let claude = self.core.claude_dir();
        let mut cost = 0.0;
        let mut here = 0;
        for (key, m) in &today {
            if m.device != me {
                continue;
            }
            here += 1;
            let session = key.rsplit_once('/').map_or(key.as_str(), |(_, s)| s);
            cost += COSTS.session(&claude, session).0;
        }
        // Running and starting, per device.
        let mut running: std::collections::HashMap<String, u32> = Default::default();
        for a in self.core.mesh_agents() {
            if a.status != "gone" && co.is_spawned(&a.key) {
                *running.entry(a.device_id.clone()).or_default() += 1;
            }
        }
        for t in co.list_tasks(&coord::TaskFilter {
            open: true,
            ..coord::TaskFilter::default()
        }) {
            if let Some(r) = t.spawn_request().filter(|r| r.pending()) {
                *running.entry(r.device.clone()).or_default() += 1;
            }
        }
        let mut ids: Vec<String> = self.core.device_labels().into_keys().collect();
        if !ids.contains(&me) {
            ids.push(me.clone());
        }
        let u = |n: i64| n.clamp(0, i64::from(u32::MAX)) as u32;
        let mut devices: Vec<DeviceCap> = ids
            .into_iter()
            .map(|d| {
                let is_here = d == me;
                let chosen = self.core.spawn_cap_set(&d);
                DeviceCap {
                    name: p.where_(&d),
                    here: is_here,
                    cap: u(self.core.spawn_cap_of(&d)),
                    chosen,
                    running: if is_here {
                        u(q.running)
                    } else {
                        running.get(&d).copied().unwrap_or(0)
                    },
                    device: d,
                }
            })
            .collect();
        devices.sort_by(|a, b| b.here.cmp(&a.here).then_with(|| a.name.cmp(&b.name)));
        AgentsLimits {
            devices,
            max_cap: u(irori_core::spawn::MAX_CAP),
            today: u(q.today),
            today_here: here,
            cost_usd: cost,
            held_until: q.held_until.map(|h| h.unix_milli_value()),
            queued: q.places.len() as u32,
        }
    }

    /// The person sets a device's cap (card #450): the app only.
    pub fn agents_cap_set(&self, device: &str, cap: u32) -> Result<AgentsLimits, String> {
        self.core.set_spawn_cap_of(device, i64::from(cap))?;
        Ok(self.agents_limits())
    }

    /// The person ends this device's usage hold (card #576): the app only.
    pub fn agents_usage_release(&self) -> AgentsLimits {
        self.core.release_usage_hold();
        self.agents_limits()
    }

    /// The person changes a board's switch, model or effort.
    pub fn agents_set(&self, c: AgentsChange) -> Result<AgentsSettings, String> {
        valid_idle(&c)?;
        let board_id = c.board_id.clone();
        let s = apply(self.core.spawn_setting(&board_id), c);
        self.core.set_spawn_setting(s)?;
        Ok(self.agents_settings(&board_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limits_round_trip_and_defaults_store_as_zero() {
        let s = apply(
            SpawnSetting::default(),
            AgentsChange {
                crash_limit: Some(5),
                start_tries: Some(1),
                ..AgentsChange::default()
            },
        );
        assert_eq!((s.crash_limit, s.start_tries), (5, 1));
        let v = view(&s, true);
        assert_eq!((v.crash_limit, v.start_tries), (5, 1));
        let s = apply(
            s,
            AgentsChange {
                crash_limit: Some(v.default_crash_limit),
                start_tries: Some(v.default_start_tries),
                ..AgentsChange::default()
            },
        );
        assert_eq!((s.crash_limit, s.start_tries), (0, 0));
        let v = view(&s, true);
        assert_eq!((v.crash_limit, v.start_tries), (2, 3));
    }

    #[test]
    fn a_change_keeps_the_fields_it_doesnt_name() {
        let stored = SpawnSetting {
            on: true,
            model: "opus".into(),
            ..SpawnSetting::default()
        };
        let s = apply(
            stored,
            AgentsChange {
                effort: Some("high".into()),
                ..AgentsChange::default()
            },
        );
        assert_eq!(
            (s.on, s.model.as_str(), s.effort.as_str()),
            (true, "opus", "high")
        );
    }

    #[test]
    fn the_idle_limit_is_one_of_the_choices_and_30_is_stored_as_the_default() {
        let c = |m| AgentsChange {
            idle_mins: Some(m),
            ..AgentsChange::default()
        };
        assert!(valid_idle(&c(7)).is_err());
        assert!(valid_idle(&c(60)).is_ok());
        let s = apply(SpawnSetting::default(), c(60));
        assert_eq!((s.idle_mins, view(&s, true).idle_mins), (60, 60));
        let s = apply(s, c(30));
        assert_eq!((s.idle_mins, view(&s, true).idle_mins), (0, 30));
        assert_eq!(view(&SpawnSetting::default(), false).idle_mins, 30);
    }

    #[test]
    fn nothing_picked_shows_sonnet_at_medium() {
        let v = view(&SpawnSetting::default(), false);
        assert_eq!(
            (v.model.as_str(), v.effort.as_str(), v.on),
            ("sonnet", "medium", false)
        );
        assert_eq!(v.models[0], "sonnet");
        assert!(!v.models.iter().any(|m| m == "haiku"));
        assert_eq!(
            (
                v.light_model.as_str(),
                v.light_effort.as_str(),
                v.heavy_model.as_str(),
                v.heavy_effort.as_str()
            ),
            ("sonnet", "medium", "opus", "high")
        );
    }

    #[test]
    fn a_weight_row_at_what_it_works_out_by_itself_stores_nothing() {
        let c = |m: &str, e: &str| AgentsChange {
            heavy_model: Some(m.into()),
            heavy_effort: Some(e.into()),
            ..AgentsChange::default()
        };
        let s = apply(SpawnSetting::default(), c("opus", "max"));
        assert_eq!(
            (s.heavy_model.as_str(), s.heavy_effort.as_str()),
            ("", "max")
        );
        let s = apply(s, c("opus", "high"));
        assert_eq!((s.heavy_model.as_str(), s.heavy_effort.as_str()), ("", ""));
        // The normal effort set in the same change is what heavy follows:
        // with normal at xhigh, heavy works out xhigh, so xhigh stores "".
        let s = apply(
            s,
            AgentsChange {
                effort: Some("xhigh".into()),
                heavy_effort: Some("xhigh".into()),
                light_model: Some("opus".into()),
                light_effort: Some("".into()),
                ..AgentsChange::default()
            },
        );
        assert_eq!(
            (
                s.heavy_effort.as_str(),
                s.light_model.as_str(),
                s.light_effort.as_str()
            ),
            ("", "opus", "")
        );
        let v = view(&s, true);
        assert_eq!(
            (v.heavy_effort.as_str(), v.light_effort.as_str()),
            ("xhigh", "medium")
        );
    }
}
