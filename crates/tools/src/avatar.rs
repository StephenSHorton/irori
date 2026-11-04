//! The avatar tool (card #407): an agent sets or reads its own face, kept
//! by its name. The Backend checks and stores it (coord::avatar).

use crate::{Caller, Inner, err_text};

pub(crate) fn avatar(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let v: serde_json::Value = match serde_json::from_slice(raw) {
        Ok(v) => v,
        Err(_) => return err_text("the arguments must be a JSON object."),
    };
    let name = c.sender.name.trim();
    if name.is_empty() {
        return err_text("you have no name yet, so no face can be kept for you.");
    }
    match v.get("avatar") {
        None | Some(serde_json::Value::Null) => {
            let cur = s.b.avatar(name);
            if cur.is_empty() {
                format!(
                    "You ({name}) have no face of your own; the app shows the pattern from your key, and doesn't draw a kept face now."
                )
            } else {
                format!("Your face ({name}): {cur}")
            }
        }
        Some(a) if a.as_object().is_some_and(|o| o.is_empty()) => {
            match s.b.set_avatar(name, None) {
                Ok(()) => "Your face is back to the default.".into(),
                Err(e) => err_text(&e),
            }
        }
        Some(a) => match coord::avatar::check(a) {
            Err(e) => err_text(&format!("that face isn't valid: {e}.")),
            Ok(face) => match s.b.set_avatar(name, Some(&face)) {
                Ok(()) => format!("Kept your face, {} ({}), by your name.", face.name, face.id),
                Err(e) => err_text(&e),
            },
        },
    }
}
