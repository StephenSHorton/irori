//! What the Rune window's inbox package (Notifications, Held, Message;
//! app/src/inbox) names from the crates under the window: the inbox's
//! delivery types and constants, Go's time, and the given question.
//! The Tauri layer sent these as JSON; the Rune window reads them as
//! Rust, so it needs the names.

pub use ::coord::board::GIVEN_QUESTION;
pub use ::gocompat::time::GoTime;
pub use ::inbox::{Delivery, Message, SYSTEM_KEY, Sender, hold, mode, st};
