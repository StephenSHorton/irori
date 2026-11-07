//! Pure derivations several pages share (F0c: the React app's lib/*.ts
//! helpers with no Rust counterpart in crates/window): a card's group and
//! status line, agent status, names, fuzzy matching. No Rune here, so it
//! is unit-tested without a window.

pub mod card_status;
