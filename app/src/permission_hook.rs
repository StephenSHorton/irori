//! `irori permission-hook` (card #682): the `PermissionRequest` hook Irori
//! puts in every spawned agent's settings file. Claude Code runs it beside
//! a permission prompt with the prompt as JSON on stdin. It hands the
//! prompt to the Core (`POST /v1/permission/wait`), which puts it on the
//! agent's card, and waits for the person's pick there. Then it prints
//! Claude Code's answer (allow, allow with a session rule, or deny) and
//! exits.
//!
//! It never stands in the way: with no Core, a session that isn't a
//! spawned agent, an answer at the terminal, or any error, it prints
//! nothing and exits 0, and the prompt stays for the terminal. stdout
//! carries only the answer; logs go to stderr.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use crate::datadir;

/// One call's wait, beyond the Core's own hold.
const CALL_WAIT: Duration = Duration::from_secs(40);
/// Between tries while the Core can't be reached.
const RETRY_EVERY: Duration = Duration::from_secs(5);
/// How long it keeps trying an unreachable Core (an app restart).
const DOWN_FOR: Duration = Duration::from_secs(10 * 60);

pub fn main(_args: &[String]) -> Result<(), String> {
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut input) {
        eprintln!("permission-hook: stdin: {e}");
        return Ok(());
    }
    match run(&input) {
        Ok(Some(out)) => {
            let mut so = std::io::stdout();
            let _ = so.write_all(out.as_bytes());
            let _ = so.write_all(b"\n");
            let _ = so.flush();
        }
        Ok(None) => {}
        Err(e) => eprintln!("permission-hook: {e}"),
    }
    Ok(())
}

/// The body the Core takes, from the hook's input; `None` when it is no
/// prompt.
pub fn body(input: &[u8], id: &str) -> Option<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_slice(input).ok()?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    if s("session_id").is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "id": id,
        "sessionId": s("session_id"),
        "toolName": s("tool_name"),
        "toolInput": v.get("tool_input").cloned().unwrap_or(serde_json::Value::Null),
        "permissionMode": s("permission_mode"),
        "permissionSuggestions": v
            .get("permission_suggestions")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    }))
}

/// The hook's stdout from the Core's answer: `Ok(None)` to say nothing,
/// `Err(())` to ask again.
pub fn answer(status: u16, body: &[u8]) -> Result<Option<String>, ()> {
    if status != 200 {
        return Ok(None);
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Ok(None);
    };
    match v.get("decision").and_then(|d| d.as_str()) {
        Some("pending") => Err(()),
        Some("allow") | Some("deny") => Ok(v.get("output").map(|o| o.to_string())),
        _ => Ok(None),
    }
}

fn run(input: &[u8]) -> Result<Option<String>, String> {
    let id = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    let Some(body) = body(input, &id) else {
        return Ok(None);
    };
    let body = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let dir = datadir::process_data_dir()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let mut down_since: Option<Instant> = None;
        let mut answered = false;
        loop {
            let got = match attach::client::dial(&dir).await {
                Ok(c) => {
                    c.post_json("/v1/permission/wait", body.clone(), CALL_WAIT)
                        .await
                }
                Err(e) => Err(e),
            };
            match got {
                Ok((status, b)) => {
                    down_since = None;
                    answered = true;
                    match answer(status, &b) {
                        Ok(out) => return Ok(out),
                        Err(()) => continue,
                    }
                }
                Err(e) => {
                    // Never reached: no Irori here, so nothing to wait on.
                    if !answered {
                        return Err(e.to_string());
                    }
                    let since = *down_since.get_or_insert_with(Instant::now);
                    if since.elapsed() > DOWN_FOR {
                        return Err(e.to_string());
                    }
                    tokio::time::sleep(RETRY_EVERY).await;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_from_the_hooks_input() {
        let input = br#"{"session_id":"s1","tool_name":"Bash","tool_input":{"command":"ls"},"permission_mode":"default","permission_suggestions":[{"type":"setMode"}],"hook_event_name":"PermissionRequest"}"#;
        let b = body(input, "i1").unwrap();
        assert_eq!(b["id"], "i1");
        assert_eq!(b["sessionId"], "s1");
        assert_eq!(b["toolName"], "Bash");
        assert_eq!(b["toolInput"]["command"], "ls");
        assert_eq!(b["permissionMode"], "default");
        assert_eq!(b["permissionSuggestions"][0]["type"], "setMode");
        assert!(body(b"not json", "i").is_none());
        assert!(body(br#"{"tool_name":"Bash"}"#, "i").is_none());
    }

    #[test]
    fn answer_reads_the_cores_decision() {
        assert_eq!(answer(200, br#"{"decision":"pending"}"#), Err(()));
        assert_eq!(answer(200, br#"{"decision":"none"}"#), Ok(None));
        assert_eq!(answer(503, b"{}"), Ok(None));
        assert_eq!(answer(200, b"junk"), Ok(None));
        let out = answer(200, br#"{"decision":"allow","output":{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}}"#)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "allow");
    }
}
