//! The session id irori_join binds (I:internal/mcpserver/session.go): the
//! call's `_meta`, then the environment, then a random id fixed for the
//! process. The Core gives the same session id the same member key, so a
//! restarted Core or a reconnect rebinds the same member; a sessionless
//! process is its own member (the key never comes from the join name).

use gocompat::json::Value;

/// `_meta` keys, in precedence order.
pub const SESSION_META_KEYS: [&str; 4] = [
    "session_id",
    "grok_session_id",
    "conversation_id",
    "mcp_session_id",
];
/// Environment variables, in precedence order.
pub const SESSION_ENV_KEYS: [&str; 3] = ["IRORI_SESSION_ID", "GROK_SESSION_ID", "XAI_SESSION_ID"];

/// A client-supplied session id's bound.
const MAX_SESSION_LEN: usize = 200;

/// One line, at most 200 runes, spaces as `_`.
pub fn clean_session(s: &str) -> String {
    tools::one_line(s, MAX_SESSION_LEN).replace(' ', "_")
}

pub(crate) fn from_meta(meta: &Value) -> String {
    let Value::Object(kv) = meta else {
        return String::new();
    };
    for k in SESSION_META_KEYS {
        if let Some((_, Value::String(v))) = kv.iter().rev().find(|(n, _)| n.text == k) {
            let v = clean_session(&v.text);
            if !v.is_empty() {
                return v;
            }
        }
    }
    String::new()
}

pub(crate) fn from_env(getenv: Option<&(dyn Fn(&str) -> String + Send + Sync)>) -> String {
    let Some(getenv) = getenv else {
        return String::new();
    };
    for k in SESSION_ENV_KEYS {
        let v = clean_session(&getenv(k));
        if !v.is_empty() {
            return v;
        }
    }
    String::new()
}

pub(crate) fn random() -> String {
    let mut b = [0u8; 12];
    let _ = getrandom::fill(&mut b);
    format!("sess_{}", hex::encode(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence() {
        let meta =
            gocompat::json::parse(br#"{"grok_session_id":"g 1","session_id":"  "}"#).unwrap();
        assert_eq!(from_meta(&meta), "g_1");
        let env = |k: &str| {
            if k == "GROK_SESSION_ID" {
                "env-2".to_string()
            } else {
                String::new()
            }
        };
        assert_eq!(from_env(Some(&env)), "env-2");
        assert_eq!(from_env(None), "");
        let r = random();
        assert!(r.starts_with("sess_") && r.len() == 29, "{r}");
    }
}
