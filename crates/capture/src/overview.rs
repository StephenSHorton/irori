//! I:internal/capture/overview.go: the overview tool's data. Capture fills
//! what it owns (live agents and what each is doing, recent commits and
//! PRs) and copies in what the caller gathered from coord and inbox
//! ([`OverviewInput`]), so capture depends on neither. Every line carries a
//! ref and every list is short.

use std::collections::{HashMap, HashSet};

use gocompat::time::GoTime;
use serde::Serialize;

use crate::find::{HIDE_ALL, hide_messages};
use crate::handle::{clip_runes, first_line};
use crate::index::one_line;
use crate::rows::{TurnRow, open_key};
use crate::service::Capture;
use crate::show::{agent_ref, commit_ref, turn_ref};
use crate::{AgentCard, gotime};

/// Live agents listed (others counted in `more_agents`).
pub const OVERVIEW_AGENTS: usize = 12;
/// Lines per list (commits, PRs, tasks, blockers, inbox).
pub const OVERVIEW_LINES: usize = 5;
/// Runes per line.
pub const OVERVIEW_LINE_MAX: usize = 120;

fn is_zero_time(t: &GoTime) -> bool {
    t.is_zero()
}

/// One overview line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Line {
    /// What to show for more.
    #[serde(rename = "ref")]
    pub r#ref: String,
    /// One line, at most [`OVERVIEW_LINE_MAX`] runes.
    pub text: String,
    #[serde(skip_serializing_if = "is_zero_time")]
    pub at: GoTime,
}

/// One live agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AgentLine {
    pub key: String,
    pub name: String,
    /// Presence status (busy, idle, ...).
    pub status: String,
    #[serde(rename = "repoKey", skip_serializing_if = "String::is_empty")]
    pub repo_key: String,
    /// The open turn's prompt if it has one (ref `agent:<key>`); else its
    /// current task ([`OverviewInput::current`]); else its last reply.
    pub doing: Line,
}

/// What the caller adds from the services capture doesn't own. Lists are
/// capped at [`OVERVIEW_LINES`] here anyway.
#[derive(Debug, Clone, Default)]
pub struct OverviewInput {
    /// The caller's open deliveries.
    pub inbox_open: i64,
    /// Held from the caller.
    pub inbox_held: i64,
    /// Its newest open deliveries (`msg:<id>`).
    pub inbox: Vec<Line>,
    /// Blocked tasks in the caller's repo (`task:<id>`).
    pub blockers: Vec<Line>,
    /// Other open tasks in the caller's repo, the caller's own first.
    pub tasks: Vec<Line>,
    /// Open tasks not listed.
    pub more_tasks: i64,
    /// Agent key -> the task it holds that changed last.
    pub current: HashMap<String, Line>,
    /// The caller's repo ("" = none: its live card's repo is used).
    pub repo_key: String,
    /// Every open card on the Board, any repo.
    pub board_count: i64,
}

fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}

/// The overview for one agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Overview {
    /// The caller's agent key.
    #[serde(rename = "for")]
    pub r#for: String,
    #[serde(rename = "repoKey", skip_serializing_if = "String::is_empty")]
    pub repo_key: String,
    /// Live: the caller first, then the same repo, then by name. Always an
    /// array.
    pub agents: Vec<AgentLine>,
    #[serde(rename = "moreAgents", skip_serializing_if = "is_zero_i64")]
    pub more_agents: i64,
    #[serde(rename = "inboxOpen")]
    pub inbox_open: i64,
    #[serde(rename = "inboxHeld", skip_serializing_if = "is_zero_i64")]
    pub inbox_held: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inbox: Vec<Line>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<Line>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<Line>,
    #[serde(rename = "moreTasks", skip_serializing_if = "is_zero_i64")]
    pub more_tasks: i64,
    /// The caller's repo, newest first (`commit:<sha>`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<Line>,
    /// The caller's repo (`pr:<key>`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub prs: Vec<Line>,
    /// The closing line telling how to go deeper.
    pub hint: String,
}

/// An agent's display name: its session name, else its title, else its key.
pub(crate) fn card_name(name: &str, title: &str, key: &str) -> String {
    if !name.is_empty() {
        name.into()
    } else if !title.is_empty() {
        title.into()
    } else {
        key.into()
    }
}

/// The first non-blank line of `s`, at most [`OVERVIEW_LINE_MAX`] runes.
fn clip_line(s: &str) -> String {
    clip_runes(&one_line(&first_line(s)), OVERVIEW_LINE_MAX)
}

/// At most [`OVERVIEW_LINES`] lines, each clipped to one line.
fn cap_lines(ls: &[Line]) -> Vec<Line> {
    ls.iter()
        .take(OVERVIEW_LINES)
        .map(|l| Line {
            text: clip_line(&l.text),
            ..l.clone()
        })
        .collect()
}

impl Capture {
    /// The overview for agent `for_key`. When `input.repo_key` is "" the
    /// caller's live card's repo is used for the agent order, commits and
    /// PRs.
    pub fn overview(&self, for_key: &str, input: &OverviewInput) -> Overview {
        let mut input = input.clone();
        let cards: Vec<AgentCard> = self.inner.live_cards().into_values().collect();
        let mut agents: Vec<AgentLine> = Vec::with_capacity(cards.len());
        for cd in &cards {
            agents.push(AgentLine {
                key: cd.key.clone(),
                name: card_name(&cd.name, &cd.title, &cd.key),
                status: cd.status.clone(),
                repo_key: cd.repo_key.clone(),
                doing: Line::default(),
            });
            if cd.key == for_key && input.repo_key.is_empty() {
                input.repo_key = cd.repo_key.clone();
            }
        }
        let rank = |a: &AgentLine| {
            if a.key == for_key {
                0
            } else if !input.repo_key.is_empty() && a.repo_key == input.repo_key {
                1
            } else {
                2
            }
        };
        agents.sort_by(|a, b| {
            rank(a)
                .cmp(&rank(b))
                .then_with(|| {
                    gocompat::text::to_lower_str(&a.name)
                        .cmp(&gocompat::text::to_lower_str(&b.name))
                })
                .then_with(|| a.key.cmp(&b.key))
        });
        let mut out = Overview {
            r#for: for_key.to_string(),
            repo_key: input.repo_key.clone(),
            inbox_open: input.inbox_open,
            inbox_held: input.inbox_held,
            inbox: cap_lines(&input.inbox),
            blockers: cap_lines(&input.blockers),
            tasks: cap_lines(&input.tasks),
            more_tasks: input.more_tasks
                + (input.tasks.len() as i64 - OVERVIEW_LINES as i64).max(0),
            ..Overview::default()
        };
        if agents.len() > OVERVIEW_AGENTS {
            out.more_agents = (agents.len() - OVERVIEW_AGENTS) as i64;
            agents.truncate(OVERVIEW_AGENTS);
        }
        out.agents = agents;

        for a in out.agents.iter_mut() {
            a.doing = self.doing(&a.key, &input.current);
        }

        if !input.repo_key.is_empty() {
            for v in self.commits(&input.repo_key, OVERVIEW_LINES) {
                out.commits.push(Line {
                    r#ref: commit_ref(&v.row.commit.sha),
                    text: clip_line(&v.row.commit.subject),
                    at: v.row.commit.at,
                });
            }
            let repo = ledger::repo_from_key(&input.repo_key);
            if !repo.is_empty() {
                // "" would mean every repo.
                let mut seen = HashSet::new();
                for e in self.prs(repo) {
                    if !seen.insert(e.key.clone()) {
                        continue;
                    }
                    if out.prs.len() == OVERVIEW_LINES {
                        break;
                    }
                    let text = format!(
                        "PR {}#{} {} {}",
                        e.repo,
                        e.number,
                        e.role.as_str(),
                        name_of(&e.session, &cards)
                    );
                    out.prs.push(Line {
                        r#ref: e.key.clone(),
                        text: clip_line(&text),
                        at: e.at,
                    });
                }
            }
        }
        out.hint =
            "show <ref> for any line; search <words> for older history; board for every card"
                .into();
        if input.board_count > 0 {
            out.hint += &format!(" ({} open)", input.board_count);
        }
        if out.more_agents > 0 {
            out.hint += &format!("; {} more agents live (search by agent)", out.more_agents);
        }
        out
    }

    /// An agent's one line: the open turn's prompt (ref `agent:<key>`; the
    /// open turn has no row yet), else the task it holds, else its last
    /// reply.
    fn doing(&self, key: &str, current: &HashMap<String, Line>) -> Line {
        let f = self.inner.read();
        if let Some(t) = f.open.get(&open_key(key, ""))
            && !crate::jsonx::trim_space(&t.prompt).is_empty()
        {
            return Line {
                r#ref: agent_ref(key),
                text: clip_line(&hide_messages(
                    &self.inner.opts.scrub.string(&t.prompt),
                    HIDE_ALL,
                    key,
                )),
                at: t.started_at,
            };
        }
        if let Some(l) = current.get(key)
            && !l.r#ref.is_empty()
        {
            return Line {
                text: clip_line(&l.text),
                ..l.clone()
            };
        }
        let mut last: Option<&TurnRow> = None;
        for t in f.turns.get(key).map(Vec::as_slice).unwrap_or(&[]) {
            if t.turn.agent_id.is_empty()
                && !crate::jsonx::trim_space(&t.turn.reply).is_empty()
                && last.is_none_or(|l| gotime::after(&t.turn.started_at, &l.turn.started_at))
            {
                last = Some(t);
            }
        }
        match last {
            Some(t) => Line {
                r#ref: turn_ref(&t.id),
                text: clip_line(&t.turn.reply),
                at: t.turn.started_at,
            },
            None => Line {
                r#ref: agent_ref(key),
                ..Line::default()
            },
        }
    }
}

/// A live agent's display name, else its key.
fn name_of(key: &str, cards: &[AgentCard]) -> String {
    cards
        .iter()
        .find(|c| c.key == key)
        .map(|c| card_name(&c.name, &c.title, &c.key))
        .unwrap_or_else(|| key.to_string())
}
