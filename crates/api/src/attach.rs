//! The attach wire (I:internal/attach/protocol.go, version.go; surface.md
//! §2.6): one [`Event`] per NDJSON line on `GET /v1/attach`, which `irori
//! attach` copies to its stdout unchanged and a bound `irori mcp` reads.
//!
//! This is the one home for these types and for the server side of the
//! stream (stream.rs, v02.rs). The `irori attach` runner (crates/attach)
//! and `irori mcp` use them from here.

use serde::Serialize;

/// Protocol version carried in hello's `v`. 2 adds the append event.
pub const VERSION: i64 = 2;

/// The first mod version (the `mod=` query) that handles append events.
pub const APPEND_SINCE: &str = "0.4.0";

pub const TYPE_HELLO: &str = "hello";
pub const TYPE_DELIVER: &str = "deliver";
pub const TYPE_APPEND: &str = "append";
pub const TYPE_NAME: &str = "name";
pub const TYPE_HELD: &str = "held";
pub const TYPE_MOVED: &str = "moved";
pub const TYPE_INBOX: &str = "inbox";
pub const TYPE_SUPERSEDED: &str = "superseded";
pub const TYPE_ERROR: &str = "error";

/// AttachSecret from hello (mod).
pub const HEADER_ATTACH: &str = "X-Irori-Attach";
/// Member secret from irori_join (Grok).
pub const HEADER_MEMBER: &str = "X-Irori-Member";
/// `composer` when a person typed the /irori command.
pub const HEADER_ORIGIN: &str = "X-Irori-Origin";

fn zero_i64(n: &i64) -> bool {
    *n == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One NDJSON line. Only the fields for its type are set; every field but
/// `type` is `omitempty`, in Go's key order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Event {
    #[serde(rename = "type")]
    pub kind: String,

    // hello
    #[serde(skip_serializing_if = "zero_i64")]
    pub v: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session: String,
    #[serde(rename = "agentKey", skip_serializing_if = "String::is_empty")]
    pub agent_key: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "nameSource", skip_serializing_if = "String::is_empty")]
    pub name_source: String,
    #[serde(rename = "aiTitle", skip_serializing_if = "String::is_empty")]
    pub ai_title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<Api>,
    #[serde(rename = "attachSecret", skip_serializing_if = "String::is_empty")]
    pub attach_secret: String,

    // deliver / append
    #[serde(rename = "wakeId", skip_serializing_if = "String::is_empty")]
    pub wake_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(skip_serializing_if = "is_false")]
    pub reminder: bool,
    #[serde(rename = "deliveryIds", skip_serializing_if = "Vec::is_empty")]
    pub delivery_ids: Vec<String>,
    #[serde(rename = "messageIds", skip_serializing_if = "Vec::is_empty")]
    pub message_ids: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub from: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mode: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub overflow: Vec<String>,

    // held / moved
    #[serde(rename = "deliveryId", skip_serializing_if = "String::is_empty")]
    pub delivery_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub preview: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub to: String,

    // inbox
    #[serde(skip_serializing_if = "zero_i64")]
    pub unread: i64,
    #[serde(rename = "held", skip_serializing_if = "zero_i64")]
    pub held_n: i64,

    // error
    #[serde(skip_serializing_if = "String::is_empty")]
    pub message: String,
}

impl Event {
    pub fn of(kind: &str) -> Event {
        Event {
            kind: kind.into(),
            ..Event::default()
        }
    }
}

/// Where the mod sends control calls.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Api {
    /// `http://127.0.0.1:<port>`.
    pub url: String,
    /// The bearer token.
    pub token: String,
}

/// Go `attach.ModAtLeast`: whether mod version `v` is at least `min`. Both
/// read as three dot-separated ints: a missing part is 0, whatever follows
/// a part's digits ends the version. An empty or unreadable `v` is never at
/// least anything; an unreadable `min` is always met.
pub fn mod_at_least(v: &str, min: &str) -> bool {
    let Some(have) = parse_version(v) else {
        return false;
    };
    let Some(want) = parse_version(min) else {
        return true;
    };
    for i in 0..3 {
        if have[i] != want[i] {
            return have[i] > want[i];
        }
    }
    true
}

fn parse_version(v: &str) -> Option<[i64; 3]> {
    let mut out = [0i64; 3];
    let v = v.trim_matches(gocompat::text::is_space);
    let v = v.strip_prefix('v').unwrap_or(v);
    for (i, p) in v.splitn(3, '.').enumerate() {
        let (mut n, mut digits) = (0i64, 0usize);
        for c in p.chars() {
            if !c.is_ascii_digit() || digits == 6 {
                break;
            }
            n = n * 10 + i64::from(c as u8 - b'0');
            digits += 1;
        }
        if digits == 0 {
            if i == 0 {
                return None;
            }
            break;
        }
        out[i] = n;
        if digits < p.len() {
            break; // a suffix ends the version
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // I:internal/attach/version_test.go.
    #[test]
    fn mod_at_least_table() {
        for (v, want) in [
            ("", false),
            ("junk", false),
            (".4.0", false),
            ("0.2.0-test", false),
            ("0.3.0", false),
            ("0.3.9", false),
            ("0.3.99999", false),
            ("0.4.0", true),
            ("0.4", true),
            ("0.4.0-rc1", true),
            ("0.4.1", true),
            ("0.10.0", true),
            ("1.0", true),
            ("1", true),
            ("v0.4.0", true),
            (" 0.4.0 ", true),
            ("0.4-rc1.9", true),
            ("0-rc.9.9", false),
        ] {
            assert_eq!(mod_at_least(v, APPEND_SINCE), want, "mod_at_least({v:?})");
        }
        assert!(
            mod_at_least("0.0.1", "") && !mod_at_least("", ""),
            "empty min"
        );
    }

    #[test]
    fn event_omits_empty_fields() {
        let ev = Event::of(TYPE_SUPERSEDED);
        assert_eq!(
            gocompat::json::marshal(&ev).unwrap(),
            br#"{"type":"superseded"}"#
        );
        let ev = Event {
            unread: 2,
            held_n: 1,
            ..Event::of(TYPE_INBOX)
        };
        assert_eq!(
            gocompat::json::marshal(&ev).unwrap(),
            br#"{"type":"inbox","unread":2,"held":1}"#
        );
    }
}
