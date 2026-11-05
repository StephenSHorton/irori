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
    /// The idle limit in minutes (card #456, sa-6): how long a spawned
    /// agent waits on an unanswered question before it hands off and goes.
    pub idle_mins: u32,
    /// The idle limits Settings › Agents offers, in minutes.
    pub idle_choices: Vec<u32>,
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
    #[serde(default)]
    pub idle_mins: Option<u32>,
}

pub(crate) fn view(s: &SpawnSetting, chosen: bool) -> AgentsSettings {
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
        idle_mins: (s.idle_limit().as_secs() / 60) as u32,
        idle_choices: IDLE_CHOICES.to_vec(),
    }
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
    if let Some(m) = c.idle_mins {
        // The default is stored as 0, so a later default change reaches it.
        let def = (spawn::IDLE_LIMIT.as_secs() / 60) as u32;
        s.idle_mins = if m == def { 0 } else { i64::from(m) };
    }
    s
}

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
    }
}
