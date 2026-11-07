//! "Waiting for your permission" (cards #449, #682; docs/design/
//! spawned-agents.md §7 "Seeing a permission prompt", §14 part 5).
//!
//! A spawned agent nobody is attached to can stop on a permission prompt
//! and would look busy forever. Two things tell Irori:
//!
//! - **The permission hook** (card #682). Every agent Irori starts gets a
//!   `PermissionRequest` hook in its settings file that runs
//!   `irori permission-hook`. Claude Code runs it beside the prompt with
//!   the tool and its input; the hook hands them to the Core
//!   ([`Services::permission_wait`]) and waits. The Core raises a typed
//!   item ([`coord::PermissionWait`]) on the card the agent was started
//!   for, with Allow once, Always allow (when a rule can help) and Deny.
//!   The person's pick is a board row, so it reaches the device that runs
//!   the agent wherever they clicked; that device hands it to the waiting
//!   hook, which answers the live prompt. The hook only reports and
//!   waits: no decision comes in through it.
//! - **Presence**. Claude Code's session file says status `waiting`,
//!   `waitingFor: "permission prompt"` (card #445). When no hook reported
//!   the prompt (an agent started before #682, or an ask Claude Code keeps
//!   for a person at the terminal), the sweep raises the same typed item
//!   without a request: the card shows the tool if it knows it, and
//!   Attach.
//!
//! Each sweep resolves the item once the agent is past the prompt or gone.
//! Every device sweeps, but presence only knows this device's agents. So
//! only a device that runs one of the card's agents raises or resolves its
//! item (card #603): before that, the PC resolved the Mac agent's item
//! every sweep and the Mac raised it again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use coord::roster::Agents as _;
use coord::{
    BlockedBy, Blocker, BlockerAnswer, BlockerOption, NewBlocker, PERMISSION_ALLOW_ONCE,
    PERMISSION_ALWAYS, PERMISSION_DENY, PERMISSION_INPUT_MAX, PermissionWait, TaskFilter,
};

use super::Services;
use crate::logf;

/// The item's question starts with this; an older Irori's item is found by
/// it, a newer one by its kind.
pub const WAITING_PERMISSION: &str = "Waiting for your permission";

/// What Claude Code writes while a tool call waits on a person.
pub const PERMISSION_PROMPT: &str = "permission prompt";

/// How long one call of [`Services::permission_wait`] waits for the
/// person before it answers "pending" (the hook asks again). Under the
/// API's 30 s write deadline.
pub const PERMISSION_POLL: Duration = Duration::from_secs(20);

/// How long a hook may go without asking before its request is dropped.
const HOOK_GONE: Duration = Duration::from_secs(90);

/// How long presence's prompt waits for the hook before the sweep raises
/// the item without one: the hook fires with the prompt, and its item says
/// more.
const HOOK_GRACE: Duration = Duration::from_secs(3);

/// How often a waiting call looks at the card.
const LOOK_EVERY: Duration = Duration::from_millis(250);

const SWEEPER: &str = "irori";

/// What Claude Code's `PermissionRequest` hook said, as
/// `irori permission-hook` passes it on.
#[derive(Debug, Clone, Default)]
pub struct PermissionAsk {
    /// The hook's own id for this prompt, the same on every call it makes.
    pub id: String,
    /// The Claude Code session on the prompt.
    pub session_id: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub permission_mode: String,
    pub permission_suggestions: serde_json::Value,
}

/// What the hook tells Claude Code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Nobody has answered yet: ask again.
    Pending,
    /// Nothing to say: leave the prompt to the terminal (not a spawned
    /// agent, or the prompt was answered there).
    Nothing,
    AllowOnce,
    /// Allow, and keep allowing calls the rule matches in this session.
    AllowAlways {
        rule: String,
    },
    Deny,
}

/// A hook waiting on this device.
#[derive(Debug, Clone)]
struct Pending {
    agent: String,
    task_id: String,
    blocker_id: String,
    rule: String,
    seen: Instant,
}

/// The waits this run knows of.
#[derive(Default)]
pub(crate) struct PermState {
    /// By the hook's request id.
    pending: Mutex<HashMap<String, Pending>>,
    /// Since when presence has shown each agent on a prompt.
    prompt_seen: Mutex<HashMap<String, Instant>>,
    /// The `irori` the hook runs; `None` installs no hook.
    pub(crate) hook_exe: Mutex<Option<std::path::PathBuf>>,
}

/// Whether presence says the agent is stopped on a permission prompt.
pub fn on_permission_prompt(a: &presence::AgentCard) -> bool {
    a.status == "waiting" && a.waiting_for == PERMISSION_PROMPT
}

/// Whether b is Irori's permission item: typed, or an older Irori's by its
/// question.
pub fn is_permission_wait(b: &Blocker) -> bool {
    b.raised_by == SWEEPER && (b.permission.is_some() || b.question.starts_with(WAITING_PERMISSION))
}

/// The settings that make a spawned agent's prompts reach Irori: a
/// `PermissionRequest` command hook running `<exe> permission-hook`. Its
/// timeout is a day: the hook holds the prompt open while the person
/// decides, and the prompt stays answerable at the terminal meanwhile.
pub fn permission_hook_settings(exe: &std::path::Path) -> serde_json::Value {
    let cmd = format!(
        "\"{}\" permission-hook",
        exe.to_string_lossy().replace('\\', "/")
    );
    serde_json::json!({
        "hooks": {
            "PermissionRequest": [
                {"hooks": [{"type": "command", "command": cmd, "timeout": 86400}]}
            ]
        }
    })
}

/// The hook's answer to Claude Code: its stdout, or `None` for nothing.
pub fn hook_output(d: &PermissionDecision) -> Option<serde_json::Value> {
    let decision = match d {
        PermissionDecision::Pending | PermissionDecision::Nothing => return None,
        PermissionDecision::AllowOnce => serde_json::json!({"behavior": "allow"}),
        PermissionDecision::AllowAlways { rule } => {
            let (tool, content) = split_rule(rule);
            let mut r = serde_json::json!({"toolName": tool});
            if let Some(c) = content {
                r["ruleContent"] = c.into();
            }
            serde_json::json!({
                "behavior": "allow",
                "updatedPermissions": [{
                    "type": "addRules",
                    "rules": [r],
                    "behavior": "allow",
                    "destination": "session",
                }],
            })
        }
        PermissionDecision::Deny => serde_json::json!({
            "behavior": "deny",
            "message": "The person denied this from the card in Irori. Do something else, or raise a blocker saying why you need it.",
        }),
    };
    Some(serde_json::json!({
        "hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}
    }))
}

/// `Bash(cargo test)` as `("Bash", Some("cargo test"))`; `WebSearch` as
/// `("WebSearch", None)`.
fn split_rule(rule: &str) -> (String, Option<String>) {
    match rule.split_once('(') {
        Some((t, rest)) if rest.ends_with(')') => {
            (t.to_string(), Some(rest[..rest.len() - 1].to_string()))
        }
        _ => (rule.to_string(), None),
    }
}

/// What the tool would run or touch, for the card: the command, the path,
/// the URL or the query, else its input as JSON.
pub fn input_summary(tool: &str, input: &serde_json::Value) -> String {
    let field = |k: &str| input.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let s = match tool {
        "Bash" | "PowerShell" => field("command"),
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        _ => None,
    }
    .or_else(|| field("file_path"))
    .or_else(|| field("notebook_path"))
    .or_else(|| field("path"))
    .unwrap_or_else(|| match input {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Object(m) if m.is_empty() => String::new(),
        v => v.to_string(),
    });
    cut(s.trim(), PERMISSION_INPUT_MAX)
}

/// The rule Always allow adds, or "" when none should be offered: a bypass
/// agent only prompts on Claude Code's safety checks, which no rule skips.
/// Claude Code's own suggestion when it has a rule; else the exact command
/// for a shell, the domain for a fetch, the tool for an MCP tool.
pub fn allow_rule(
    tool: &str,
    input: &serde_json::Value,
    mode: &str,
    suggestions: &serde_json::Value,
) -> String {
    if mode == "bypassPermissions" {
        return String::new();
    }
    if let Some(r) = suggested_rule(suggestions) {
        return r;
    }
    let field = |k: &str| input.get(k).and_then(|v| v.as_str()).unwrap_or("").trim();
    match tool {
        "Bash" | "PowerShell" => {
            let c = field("command");
            if c.is_empty() || c.contains('\n') || c.chars().count() > 300 {
                String::new()
            } else {
                format!("{tool}({c})")
            }
        }
        "WebFetch" => host_of(field("url"))
            .map(|h| format!("WebFetch(domain:{h})"))
            .unwrap_or_default(),
        t if t.starts_with("mcp__") => t.to_string(),
        _ => String::new(),
    }
}

/// The first allow rule among Claude Code's `permission_suggestions`.
fn suggested_rule(s: &serde_json::Value) -> Option<String> {
    s.as_array()?.iter().find_map(|u| {
        if u.get("type")?.as_str()? != "addRules" || u.get("behavior")?.as_str()? != "allow" {
            return None;
        }
        let r = u.get("rules")?.as_array()?.first()?;
        let tool = r.get("toolName")?.as_str()?;
        Some(match r.get("ruleContent").and_then(|c| c.as_str()) {
            Some(c) if !c.is_empty() => format!("{tool}({c})"),
            _ => tool.to_string(),
        })
    })
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// What the person's pick on the item means for the hook.
fn decision_of(b: &Blocker, rule: &str) -> Option<PermissionDecision> {
    let r = b.resolved.as_ref()?;
    if r.by == SWEEPER {
        return Some(PermissionDecision::Nothing);
    }
    let picked = usize::try_from(r.option)
        .ok()
        .and_then(|i| b.options.get(i))
        .map(|o| o.answer.as_str())
        .unwrap_or(r.answer.as_str());
    Some(match picked {
        PERMISSION_ALLOW_ONCE => PermissionDecision::AllowOnce,
        PERMISSION_ALWAYS if !rule.is_empty() => PermissionDecision::AllowAlways {
            rule: rule.to_string(),
        },
        PERMISSION_ALWAYS => PermissionDecision::AllowOnce,
        PERMISSION_DENY => PermissionDecision::Deny,
        // Their own words, or a close: the prompt stays for the terminal.
        _ => PermissionDecision::Nothing,
    })
}

impl Services {
    /// The `irori` a spawned agent's permission hook runs; `None` (tests,
    /// the phone) starts agents without one.
    pub(crate) fn set_permission_hook(&self, exe: Option<std::path::PathBuf>) {
        *self.spawn.perm.hook_exe.lock().unwrap() = exe;
    }

    /// The hook's settings for a spawn, if a hook is installed.
    pub(crate) fn permission_hook(&self) -> Option<serde_json::Value> {
        let exe = self.spawn.perm.hook_exe.lock().unwrap().clone()?;
        Some(permission_hook_settings(&exe))
    }

    /// A spawned agent's hook reports its prompt and waits up to `wait` for
    /// the person's pick on the card (card #682). The first call raises
    /// the item; later calls with the same id only wait.
    pub fn permission_wait(&self, ask: &PermissionAsk, wait: Duration) -> PermissionDecision {
        if ask.id.is_empty() || ask.session_id.is_empty() {
            return PermissionDecision::Nothing;
        }
        let key = format!("{}/{}", self.device.id, ask.session_id);
        let known = self
            .spawn
            .perm
            .pending
            .lock()
            .unwrap()
            .get(&ask.id)
            .cloned();
        let p = match known {
            Some(p) => p,
            None => match self.raise_permission_wait(&key, ask) {
                Some(p) => p,
                None => return PermissionDecision::Nothing,
            },
        };
        let deadline = Instant::now() + wait;
        loop {
            if let Some(d) = self.permission_answer(&p) {
                self.spawn.perm.pending.lock().unwrap().remove(&ask.id);
                return d;
            }
            if let Some(q) = self.spawn.perm.pending.lock().unwrap().get_mut(&ask.id) {
                q.seen = Instant::now();
            }
            if Instant::now() >= deadline {
                return PermissionDecision::Pending;
            }
            std::thread::sleep(LOOK_EVERY.min(deadline - Instant::now()));
        }
    }

    /// The pick on p's item, once there is one; gone or closed is Nothing.
    fn permission_answer(&self, p: &Pending) -> Option<PermissionDecision> {
        let Some(t) = self.coord.get_task(&p.task_id) else {
            return Some(PermissionDecision::Nothing);
        };
        match t.blockers.iter().find(|b| b.id == p.blocker_id) {
            Some(b) => decision_of(b, &p.rule),
            None => Some(PermissionDecision::Nothing),
        }
    }

    /// Raises the typed item for a hook's prompt on the card its agent was
    /// started for; `None` when the session isn't a spawned agent here.
    fn raise_permission_wait(&self, key: &str, ask: &PermissionAsk) -> Option<Pending> {
        let task_id = self.coord.spawned_card(key)?;
        let t = self
            .coord
            .get_task(&task_id)
            .filter(|t| !coord::is_terminal(&t.state))?;
        // An item for this agent's earlier prompt, or presence's item for
        // this one, gives way to this one.
        for b in t
            .blockers
            .iter()
            .filter(|b| b.open() && is_permission_wait(b))
        {
            if b.permission.as_ref().is_none_or(|w| w.agent == key) {
                self.close_permission_wait(&t, b, "Superseded by the prompt Irori can answer.");
            }
        }
        let tool = ask.tool_name.trim().to_string();
        let input = input_summary(&tool, &ask.tool_input);
        let rule = allow_rule(
            &tool,
            &ask.tool_input,
            &ask.permission_mode,
            &ask.permission_suggestions,
        );
        let name = self.agent_label(key);
        let mut options = vec![BlockerOption {
            answer: PERMISSION_ALLOW_ONCE.into(),
            why: "Lets the agent run it this time; it asks again next time.".into(),
            ..BlockerOption::default()
        }];
        if !rule.is_empty() {
            options.push(BlockerOption {
                answer: PERMISSION_ALWAYS.into(),
                why: format!("Also adds {rule} to the board's allow list in Settings › Agents."),
                ..BlockerOption::default()
            });
        }
        options.push(BlockerOption {
            answer: PERMISSION_DENY.into(),
            why: "Tells the agent no; it goes on without it.".into(),
            ..BlockerOption::default()
        });
        let what = if tool.is_empty() {
            "a tool".to_string()
        } else {
            tool.clone()
        };
        let nb = NewBlocker {
            question: format!(
                "{WAITING_PERMISSION}: {name} wants to use {what}{}",
                if input.is_empty() {
                    String::new()
                } else {
                    format!(": {}", cut(&first_line(&input), 80))
                }
            ),
            context: format!(
                "{name}, the agent Irori started for {}, is stopped on a permission prompt. \
                 Allow once or Deny answers it from here{}. Or attach to it and answer in its \
                 terminal.",
                t.short_ref(),
                if rule.is_empty() {
                    String::new()
                } else {
                    "; Always allow also adds the rule to the board's allow list".to_string()
                }
            ),
            options,
            blocked_by: Some(BlockedBy {
                kind: coord::BLOCKED_ON_PERSON.into(),
                ..BlockedBy::default()
            }),
            permission: Some(PermissionWait {
                agent: key.to_string(),
                tool,
                input,
                rule: rule.clone(),
                request: ask.id.clone(),
            }),
            ..NewBlocker::default()
        };
        let (_, b) = match self.coord.raise(SWEEPER, &task_id, nb) {
            Ok(got) => got,
            Err(e) => {
                logf(format_args!("permission wait on {}: {e}", t.short_ref()));
                return None;
            }
        };
        let p = Pending {
            agent: key.to_string(),
            task_id,
            blocker_id: b.id,
            rule,
            seen: Instant::now(),
        };
        self.spawn
            .perm
            .pending
            .lock()
            .unwrap()
            .insert(ask.id.clone(), p.clone());
        Some(p)
    }

    fn agent_label(&self, key: &str) -> String {
        let n = self.roster.name_of(key);
        if n.is_empty() {
            "Its agent".to_string()
        } else {
            n
        }
    }

    fn close_permission_wait(&self, t: &coord::Task, b: &Blocker, why: &str) {
        let a = BlockerAnswer {
            text: why.into(),
            resolve: true,
            ..BlockerAnswer::default()
        };
        if let Err(e) = self.coord.answer(SWEEPER, &t.id, &b.id, a) {
            logf(format_args!("permission wait on {}: {e}", t.short_ref()));
        }
    }

    /// Raises or resolves each spawned card's "Waiting for your
    /// permission" item from its agent's presence and the hooks waiting.
    pub(crate) fn sweep_permission_waits(&self) {
        let me = self.device.id.as_str();
        let agents = self.presence.snapshot();
        let now = Instant::now();
        // Hooks that stopped asking are gone (their session ended, or the
        // prompt was answered at the terminal).
        let hooked: HashMap<String, String> = {
            let mut pending = self.spawn.perm.pending.lock().unwrap();
            pending.retain(|_, p| now.duration_since(p.seen) < HOOK_GONE);
            pending
                .values()
                .map(|p| (p.agent.clone(), p.blocker_id.clone()))
                .collect()
        };
        let on_prompt: Vec<&presence::AgentCard> = agents
            .iter()
            .filter(|a| on_permission_prompt(a) && runs_here(&a.key, &a.device_id, me))
            .collect();
        let since: HashMap<String, Instant> = {
            let mut seen = self.spawn.perm.prompt_seen.lock().unwrap();
            seen.retain(|k, _| on_prompt.iter().any(|a| &a.key == k));
            for a in &on_prompt {
                seen.entry(a.key.clone()).or_insert(now);
            }
            seen.clone()
        };
        // Card → its agent on a prompt.
        let waiting: HashMap<String, &presence::AgentCard> = on_prompt
            .iter()
            .filter_map(|a| self.coord.spawned_card(&a.key).map(|t| (t, *a)))
            .collect();
        let open = self.coord.list_tasks(&TaskFilter {
            open: true,
            ..TaskFilter::default()
        });
        for t in open {
            let mine = t
                .blockers
                .iter()
                .find(|b| b.open() && is_permission_wait(b));
            match (waiting.get(&t.id), mine) {
                (Some(a), None) => {
                    // The hook raises its own item with more on it.
                    if hooked.contains_key(&a.key)
                        || since
                            .get(&a.key)
                            .is_some_and(|s| now.duration_since(*s) < HOOK_GRACE)
                    {
                        continue;
                    }
                    self.raise_presence_wait(&t, a);
                }
                (None, Some(b)) if self.decides_permission_wait(&t, me) => {
                    self.close_permission_wait(&t, b, "The agent is past the prompt.");
                }
                _ => {}
            }
        }
    }

    /// The item for a prompt only presence saw: no request, so no answer
    /// from the card; the tool when the transcript names it, and Attach.
    fn raise_presence_wait(&self, t: &coord::Task, a: &presence::AgentCard) {
        let name = self.agent_label(&a.key);
        let tool = a.last_tool.trim().to_string();
        let nb = NewBlocker {
            question: format!("{WAITING_PERMISSION}: {name} is stopped on a prompt"),
            context: format!(
                "{name}, the agent Irori started for {}, is waiting for a permission \
                 nobody is there to give, and Irori can't answer this one from the card. \
                 Attach to it to answer the prompt, or add the command to the space's allow \
                 list in Settings › Agents and start a fresh agent.",
                t.short_ref()
            ),
            blocked_by: Some(BlockedBy {
                kind: coord::BLOCKED_ON_PERSON.into(),
                ..BlockedBy::default()
            }),
            permission: Some(PermissionWait {
                agent: a.key.clone(),
                tool,
                ..PermissionWait::default()
            }),
            ..NewBlocker::default()
        };
        if let Err(e) = self.coord.raise(SWEEPER, &t.id, nb) {
            logf(format_args!("permission wait on {}: {e}", t.short_ref()));
        }
    }

    /// Whether this device may resolve card t's permission wait: it runs
    /// one of the card's agents (its own, a step's or a subtask's), and no
    /// agent of the card on another device is on a prompt by the roster.
    /// A device that runs none of them can't tell whether the agent is
    /// past the prompt, so it leaves the blocker to the one that can.
    fn decides_permission_wait(&self, t: &coord::Task, me: &str) -> bool {
        if !t.spawns.iter().any(|s| runs_here(&s.key, &s.device, me)) {
            return false;
        }
        let elsewhere: Vec<&str> = t
            .spawns
            .iter()
            .filter(|s| !s.key.is_empty() && !runs_here(&s.key, &s.device, me))
            .map(|s| s.key.as_str())
            .collect();
        elsewhere.is_empty()
            || !self
                .roster
                .agents()
                .iter()
                .any(|a| elsewhere.contains(&a.key.as_str()) && roster_on_prompt(a))
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

/// Whether agent `key` (on `device`, when known) runs on device `me`.
/// Agent keys are `<device>/<session>`.
fn runs_here(key: &str, device: &str, me: &str) -> bool {
    if !device.is_empty() {
        return device == me;
    }
    key.split_once('/').is_some_and(|(d, _)| d == me)
}

/// Whether the mesh roster's card of an agent on another device may be on
/// a permission prompt. Its status rows don't carry `waitingFor`, so any
/// wait counts unless the card says it is for something else.
fn roster_on_prompt(a: &coord::roster::AgentCard) -> bool {
    a.status == "waiting" && (a.waiting_for.is_empty() || a.waiting_for == PERMISSION_PROMPT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn input_summary_names_what_runs() {
        assert_eq!(
            input_summary("Bash", &json!({"command": " ls -la ", "description": "x"})),
            "ls -la"
        );
        assert_eq!(
            input_summary("Write", &json!({"file_path": "/a/b", "content": "x"})),
            "/a/b"
        );
        assert_eq!(
            input_summary(
                "WebFetch",
                &json!({"url": "https://x.dev/a", "prompt": "p"})
            ),
            "https://x.dev/a"
        );
        assert_eq!(input_summary("mcp__x__y", &json!({"a": 1})), "{\"a\":1}");
        assert_eq!(input_summary("Foo", &json!({})), "");
        let long = "x".repeat(PERMISSION_INPUT_MAX + 10);
        assert_eq!(
            input_summary("Bash", &json!({"command": long}))
                .chars()
                .count(),
            PERMISSION_INPUT_MAX
        );
    }

    #[test]
    fn allow_rule_is_narrow_and_never_for_bypass() {
        let none = json!([]);
        assert_eq!(
            allow_rule(
                "Bash",
                &json!({"command": "cargo test"}),
                "acceptEdits",
                &none
            ),
            "Bash(cargo test)"
        );
        assert_eq!(
            allow_rule(
                "Bash",
                &json!({"command": "cargo test"}),
                "bypassPermissions",
                &none
            ),
            ""
        );
        assert_eq!(
            allow_rule("Bash", &json!({"command": "a\nb"}), "default", &none),
            ""
        );
        assert_eq!(
            allow_rule(
                "WebFetch",
                &json!({"url": "https://Docs.rs:443/x"}),
                "default",
                &none
            ),
            "WebFetch(domain:docs.rs)"
        );
        assert_eq!(
            allow_rule("mcp__gh__pr", &json!({}), "default", &none),
            "mcp__gh__pr"
        );
        assert_eq!(
            allow_rule("Write", &json!({"file_path": "/a"}), "default", &none),
            ""
        );
        let sugg = json!([
            {"type": "addDirectories", "directories": ["/x"], "destination": "session"},
            {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "npm run:*"}], "behavior": "allow", "destination": "localSettings"}
        ]);
        assert_eq!(
            allow_rule("Bash", &json!({"command": "npm run x"}), "default", &sugg),
            "Bash(npm run:*)"
        );
    }

    #[test]
    fn hook_output_is_claude_codes_shape() {
        assert_eq!(hook_output(&PermissionDecision::Pending), None);
        assert_eq!(hook_output(&PermissionDecision::Nothing), None);
        let o = hook_output(&PermissionDecision::AllowOnce).unwrap();
        assert_eq!(
            o["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(
            o["hookSpecificOutput"]["decision"],
            json!({"behavior": "allow"})
        );
        let o = hook_output(&PermissionDecision::AllowAlways {
            rule: "Bash(cargo test)".into(),
        })
        .unwrap();
        let d = &o["hookSpecificOutput"]["decision"];
        assert_eq!(d["behavior"], "allow");
        assert_eq!(
            d["updatedPermissions"][0]["rules"][0],
            json!({"toolName": "Bash", "ruleContent": "cargo test"})
        );
        assert_eq!(d["updatedPermissions"][0]["destination"], "session");
        let o = hook_output(&PermissionDecision::AllowAlways {
            rule: "mcp__a__b".into(),
        })
        .unwrap();
        assert_eq!(
            o["hookSpecificOutput"]["decision"]["updatedPermissions"][0]["rules"][0],
            json!({"toolName": "mcp__a__b"})
        );
        let o = hook_output(&PermissionDecision::Deny).unwrap();
        assert_eq!(o["hookSpecificOutput"]["decision"]["behavior"], "deny");
    }

    #[test]
    fn hook_settings_quote_the_exe() {
        let v =
            permission_hook_settings(std::path::Path::new("C:\\Program Files\\Irori\\irori.exe"));
        let h = &v["hooks"]["PermissionRequest"][0]["hooks"][0];
        assert_eq!(
            h["command"],
            "\"C:/Program Files/Irori/irori.exe\" permission-hook"
        );
        assert_eq!(h["timeout"], 86400);
    }
}
