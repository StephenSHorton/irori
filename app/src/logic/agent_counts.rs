//! Agent counts that match who a thread or ask can go to (the React app's
//! lib/agent-counts.ts, cards #172, #453, #512, #341, #487). Irori hands
//! work only to an agent whose roster status is exactly "idle" on a device
//! that isn't offline, and only to a terminal agent with "Offer it cards"
//! on. "unresponsive" is neither free nor working. Gone isn't counted.

use std::collections::{HashMap, HashSet};

use irori_window::cards::{IdleReport, UnofferedCard};
use irori_window::roster::RosterAgent;

/// The roster status of an agent whose wakes went unread (#110).
pub const NOT_ANSWERING: &str = "unresponsive";

/// What the counts read off an agent.
#[derive(Clone, Debug, Default)]
pub struct Agent {
    pub key: String,
    pub status: String,
    pub device_state: String,
    pub repo_key: String,
    pub offers_cards: bool,
    pub spawned: bool,
}

impl From<&RosterAgent> for Agent {
    fn from(a: &RosterAgent) -> Agent {
        Agent {
            key: a.card.key.clone(),
            status: a.card.status.clone(),
            device_state: a.card.device_state.clone(),
            repo_key: a.card.repo_key.clone(),
            offers_cards: a.offers_cards,
            spawned: a.spawned,
        }
    }
}

/// The roster as [`Agent`]s.
pub fn agents_of(r: &[RosterAgent]) -> Vec<Agent> {
    r.iter().map(Agent::from).collect()
}

/// Whether Irori hands this agent unheld cards, asks and threads (#453).
pub fn takes_offers(a: &Agent) -> bool {
    a.offers_cards && !a.spawned
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgentCounts {
    /// Agents still around (not gone).
    pub total: usize,
    /// Busy, on their own command ("shell"), or any other status.
    pub working: usize,
    /// Of working: running a job ("shell").
    pub running: usize,
    /// Idle on an online device with Offer it cards on.
    pub free: usize,
    /// Idle on an online device, not taking offered work.
    pub own_work: usize,
    /// Idle, wakes unread.
    pub silent: usize,
    pub offline: usize,
    /// On a permission prompt or a card with an open blocker (#512).
    pub blocked: usize,
}

pub fn agent_counts(agents: &[Agent], blocked: &HashSet<String>) -> AgentCounts {
    let mut c = AgentCounts::default();
    for a in agents {
        if a.status == "gone" {
            continue;
        }
        c.total += 1;
        if a.device_state == "offline" {
            c.offline += 1;
        } else if blocked.contains(&a.key) {
            c.blocked += 1;
        } else if a.status == NOT_ANSWERING {
            c.silent += 1;
        } else if a.status == "idle" {
            if takes_offers(a) {
                c.free += 1;
            } else {
                c.own_work += 1;
            }
        } else {
            c.working += 1;
            if a.status == "shell" {
                c.running += 1;
            }
        }
    }
    c
}

/// How many free agents work in `repo_key` ("" for any repo).
pub fn free_in(agents: &[Agent], repo_key: &str) -> usize {
    agents
        .iter()
        .filter(|a| {
            a.status == "idle"
                && a.device_state != "offline"
                && takes_offers(a)
                && (repo_key.is_empty() || a.repo_key == repo_key)
        })
        .count()
}

fn n(k: usize, one: &str, many: &str) -> String {
    format!("{k} {}", if k == 1 { one } else { many })
}

/// Why a thread or ask nobody has taken still waits, by the rule Irori
/// hands work out by; "" when someone is free.
pub fn why_waiting(agents: &[Agent], repo_key: &str) -> String {
    let c = agent_counts(agents, &HashSet::new());
    if c.total == 0 {
        return "No agents are running.".into();
    }
    if !agents.iter().any(|a| a.status != "gone" && takes_offers(a)) {
        return "No agent takes offered work: Irori starts one for it when spawning is on.".into();
    }
    if c.free > 0 {
        if !repo_key.is_empty() && free_in(agents, repo_key) == 0 {
            return format!(
                "{} idle, but none works in {repo_key}.",
                n(c.free, "agent is", "agents are")
            );
        }
        return String::new();
    }
    let mut parts = Vec::new();
    if c.working > 0 {
        parts.push(if c.running > 0 {
            format!("{} working ({} running a job)", c.working, c.running)
        } else {
            format!("{} working", c.working)
        });
    }
    if c.own_work > 0 {
        parts.push(format!("{} idle, not taking offered work", c.own_work));
    }
    if c.silent > 0 {
        parts.push(format!("{} idle but not answering", c.silent));
    }
    if c.offline > 0 {
        parts.push(format!("{} on an offline device", c.offline));
    }
    format!(
        "Nobody is free: {}. Irori starts an agent for it when spawning is on.",
        parts.join(", ")
    )
}

/// How many unhanded cards the idle line names before "and N more".
pub const IDLE_LINE_SHOWN: usize = 3;

/// An open card Home files under Stuck because nobody took it.
#[derive(Clone, Debug, Default)]
pub struct Untaken {
    pub id: String,
    pub short: String,
}

/// Home's idle line: "3 agents idle, no open work" only when no open card
/// waits for a taker; else "1 agent idle; 3 cards nobody took: #25, #6,
/// #14". "" when nobody is idle and nothing waits.
pub fn idle_line(r: &IdleReport, untaken: &[Untaken], home_ids: &HashSet<String>) -> String {
    let why: HashMap<&str, &str> = r
        .unoffered
        .iter()
        .map(|u: &UnofferedCard| (u.id.as_str(), u.why.as_str()))
        .collect();
    let mut cards: Vec<(String, String)> = untaken
        .iter()
        .map(|u| {
            (
                u.short.clone(),
                why.get(u.id.as_str()).unwrap_or(&"").to_string(),
            )
        })
        .collect();
    let listed: HashSet<&str> = untaken.iter().map(|u| u.id.as_str()).collect();
    for u in &r.unoffered {
        if !listed.contains(u.id.as_str()) && !home_ids.contains(&u.id) {
            cards.push((u.short.clone(), u.why.clone()));
        }
    }
    let idle = r.idle as usize;
    let k = cards.len();
    if k == 0 {
        return if idle > 0 {
            format!("{} idle, no open work.", n(idle, "agent", "agents"))
        } else {
            String::new()
        };
    }
    let shown = cards
        .iter()
        .take(IDLE_LINE_SHOWN)
        .map(|(s, w)| {
            if w.is_empty() {
                s.clone()
            } else {
                format!("{s} ({w})")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let more = if k > IDLE_LINE_SHOWN {
        format!(", and {} more", k - IDLE_LINE_SHOWN)
    } else {
        String::new()
    };
    let who = if idle > 0 {
        format!("{} idle", n(idle, "agent", "agents"))
    } else {
        "No agent is free".to_string()
    };
    format!(
        "{who}; {} nobody took: {shown}{more}.",
        n(k, "card", "cards")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(status: &str) -> Agent {
        Agent {
            status: status.into(),
            offers_cards: true,
            ..Agent::default()
        }
    }
    fn with(status: &str, f: impl Fn(&mut Agent)) -> Agent {
        let mut x = a(status);
        f(&mut x);
        x
    }
    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn shell_counts_as_working_not_free_gone_isnt_counted() {
        let c = agent_counts(
            &[
                a("busy"),
                a("shell"),
                a("shell"),
                a("idle"),
                a("unresponsive"),
                with("idle", |x| x.device_state = "offline".into()),
                a("gone"),
            ],
            &none(),
        );
        assert_eq!(
            c,
            AgentCounts {
                total: 6,
                working: 3,
                running: 2,
                free: 1,
                own_work: 0,
                silent: 1,
                offline: 1,
                blocked: 0
            }
        );
    }

    #[test]
    fn a_blocked_agent_is_neither_working_nor_free() {
        let k = |key: &str, status: &str, offers: bool| Agent {
            key: key.into(),
            status: status.into(),
            offers_cards: offers,
            ..Agent::default()
        };
        let blocked: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let c = agent_counts(
            &[
                k("a", "busy", false),
                k("b", "idle", true),
                k("c", "busy", false),
            ],
            &blocked,
        );
        assert_eq!((c.working, c.free, c.blocked), (1, 0, 2));
    }

    #[test]
    fn an_idle_terminal_agent_without_offer_isnt_free() {
        let agents = [
            with("idle", |x| x.offers_cards = false),
            with("idle", |x| x.spawned = true),
            a("idle"),
        ];
        let c = agent_counts(&agents, &none());
        assert_eq!(c.free, 1);
        assert_eq!(c.own_work, 2);
        assert_eq!(free_in(&agents, ""), 1);
        assert_eq!(free_in(&[with("idle", |x| x.offers_cards = false)], ""), 0);
    }

    #[test]
    fn free_matches_who_a_thread_goes_to() {
        let agents = [
            a("idle"),
            with("idle", |x| x.device_state = "offline".into()),
            a("shell"),
            a("unresponsive"),
        ];
        assert_eq!(agent_counts(&agents, &none()).free, free_in(&agents, ""));
        assert_eq!(free_in(&agents, ""), 1);
    }

    #[test]
    fn why_waiting_cases() {
        assert_eq!(
            why_waiting(&[a("busy"), a("shell"), a("shell")], ""),
            "Nobody is free: 3 working (2 running a job). Irori starts an agent for it when spawning is on."
        );
        assert_eq!(
            why_waiting(&[a("busy"), a("unresponsive"), a("unresponsive")], ""),
            "Nobody is free: 1 working, 2 idle but not answering. Irori starts an agent for it when spawning is on."
        );
        assert_eq!(
            why_waiting(&[with("idle", |x| x.device_state = "offline".into())], ""),
            "Nobody is free: 1 on an offline device. Irori starts an agent for it when spawning is on."
        );
        let mlg = |_: ()| with("idle", |x| x.repo_key = "mlg".into());
        assert_eq!(
            why_waiting(&[mlg(()), mlg(())], "irori"),
            "2 agents are idle, but none works in irori."
        );
        assert_eq!(why_waiting(&[a("idle"), a("busy")], ""), "");
        assert_eq!(
            why_waiting(&[with("idle", |x| x.repo_key = "irori".into())], "irori"),
            ""
        );
        assert_eq!(
            why_waiting(
                &[
                    with("idle", |x| x.offers_cards = false),
                    with("busy", |x| x.spawned = true)
                ],
                ""
            ),
            "No agent takes offered work: Irori starts one for it when spawning is on."
        );
        assert_eq!(
            why_waiting(&[a("busy"), with("idle", |x| x.offers_cards = false)], ""),
            "Nobody is free: 1 working, 1 idle, not taking offered work. Irori starts an agent for it when spawning is on."
        );
        assert_eq!(why_waiting(&[a("gone")], ""), "No agents are running.");
    }

    fn u(short: &str, why: &str) -> UnofferedCard {
        UnofferedCard {
            id: short.into(),
            short: short.into(),
            title: short.into(),
            why: why.into(),
        }
    }
    fn report(idle: u32, unoffered: Vec<UnofferedCard>) -> IdleReport {
        IdleReport { idle, unoffered }
    }

    #[test]
    fn idle_line_cases() {
        assert_eq!(
            idle_line(&report(20, vec![]), &[], &none()),
            "20 agents idle, no open work."
        );
        assert_eq!(
            idle_line(&report(1, vec![]), &[], &none()),
            "1 agent idle, no open work."
        );
        assert_eq!(idle_line(&report(0, vec![]), &[], &none()), "");
        assert_eq!(
            idle_line(
                &report(
                    20,
                    vec![
                        u("wind#87", "nobody in this space is free"),
                        u("wind#88", "nobody in this space is free"),
                        u("#12", "no free agent works in irori"),
                        u("#13", "every free agent passed it"),
                    ]
                ),
                &[],
                &none()
            ),
            "20 agents idle; 4 cards nobody took: wind#87 (nobody in this space is free), wind#88 (nobody in this space is free), #12 (no free agent works in irori), and 1 more."
        );
        assert_eq!(
            idle_line(&report(0, vec![u("#5", "no agent is free")]), &[], &none()),
            "No agent is free; 1 card nobody took: #5 (no agent is free)."
        );
    }

    #[test]
    fn cards_home_files_as_nobody_took_count() {
        let took: Vec<Untaken> = [("c25", "#25"), ("c6", "#6"), ("c14", "#14")]
            .iter()
            .map(|(id, s)| Untaken {
                id: id.to_string(),
                short: s.to_string(),
            })
            .collect();
        assert_eq!(
            idle_line(&report(1, vec![]), &took, &none()),
            "1 agent idle; 3 cards nobody took: #25, #6, #14."
        );
        let mut c25 = u("#25", "every free agent passed it");
        c25.id = "c25".into();
        assert_eq!(
            idle_line(&report(1, vec![c25]), &took, &none()),
            "1 agent idle; 3 cards nobody took: #25 (every free agent passed it), #6, #14."
        );
        let mut c5 = u("#5", "no agent is free");
        c5.id = "c5".into();
        let home: HashSet<String> = ["c5".to_string()].into_iter().collect();
        assert_eq!(
            idle_line(&report(1, vec![c5]), &[], &home),
            "1 agent idle, no open work."
        );
        assert_eq!(
            idle_line(
                &report(2, vec![u("wind#9", "nobody in this space is free")]),
                &took[..1],
                &none()
            ),
            "2 agents idle; 2 cards nobody took: #25, wind#9 (nobody in this space is free)."
        );
    }
}
