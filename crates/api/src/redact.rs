//! Go `tools.Redact` (I:internal/tools/observe.go:207-246; surface.md
//! §2.8): a row as an agent may see it. Message text and reasons are left
//! out, and so is a turn's prompt when it is an Irori fence. The loopback
//! API shows rows whole only to the person (`X-Irori-Person`).
//!
//! Merge note: the tools crate's events tool needs the same function; one
//! of the two copies should go.

use gocompat::json::{AnyValue, Schema};

const LEFT_OUT: &str = "[left out]";
const FALLBACK: &[u8] = br#"{"text":"[left out]"}"#;

/// The payload of a `kind` row as an agent may see it.
pub fn redact(kind: &str, raw: &[u8]) -> Vec<u8> {
    let fields: &[&str] = match kind {
        store::kinds::MESSAGE_SENT => &["text", "reason"],
        store::kinds::CAPTURE_TURN => {
            // The stored JSON may escape "<"; read the prompt itself.
            let schema = Schema::object(vec![("prompt", Schema::String)]);
            if let Ok(o) = gocompat::json::unmarshal(raw, &schema)
                && !gocompat::text::to_lower_str(o.get_str("prompt")).contains("<irori-message")
            {
                return raw.to_vec();
            }
            &["prompt"]
        }
        _ => return raw.to_vec(),
    };
    let mut any = match gocompat::json::unmarshal_any(raw) {
        Ok(v @ AnyValue::Object(_)) => v,
        _ => return FALLBACK.to_vec(),
    };
    if let AnyValue::Object(m) = &mut any {
        for k in fields {
            if let Some(v) = m.get_mut(*k) {
                *v = AnyValue::String(LEFT_OUT.into());
            }
        }
    }
    gocompat::json::marshal(&any).unwrap_or_else(|_| FALLBACK.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_out_text_and_fences() {
        let got = redact(
            "message.sent",
            br#"{"text":"hi","reason":"r","to":"x","n":12345678901234567890}"#,
        );
        assert_eq!(
            got,
            br#"{"n":12345678901234567000,"reason":"[left out]","text":"[left out]","to":"x"}"#
        );
        let plain = br#"{"prompt":"hello","b":1}"#;
        assert_eq!(redact("capture.turn", plain), plain);
        let fence = br#"{"prompt":"<IRORI-MESSAGE from=x>","b":1}"#;
        assert_eq!(
            redact("capture.turn", fence),
            br#"{"b":1,"prompt":"[left out]"}"#
        );
        assert_eq!(redact("agent.seen", b"{\"a\":1}"), b"{\"a\":1}");
        assert_eq!(redact("message.sent", b"[1]"), FALLBACK);
    }
}
