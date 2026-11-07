//! Pure derivations several pages share (F0c: the React app's lib/*.ts
//! helpers with no Rust counterpart in crates/window). No Rune here, so
//! each module is unit-tested without a window; the `*.test.ts` cases are
//! ported as each module's tests.
//!
//! It is the screen packages' API: parts stay unused until they land.
pub mod agent_counts;
pub mod agent_status;
pub mod card_status;
pub mod cards;
pub mod fuzzy;
pub mod later;
pub mod names;
pub mod needs;
pub mod owners;
pub mod plain;
pub mod roster_groups;
pub mod rounds;
pub mod search;
pub mod sidebar;
pub mod spaces;
pub mod spawn_line;
pub mod thread;
pub mod wait_name;
