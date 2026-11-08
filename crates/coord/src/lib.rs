//! `irori-coord`: the Board and the small folds beside it (coord.md;
//! architecture.md §1.1).
//!
//! | Module | Go package | What |
//! |---|---|---|
//! | [`board`] | `internal/coord` | the Board: tasks, leases, comments, blockers, steps, asks, subtasks, pins |
//! | [`approvals`] | `internal/ask` | the person's approvals and revive-across-devices |
//! | [`decision`] | `internal/decision` | decision records with re-checked code anchors |
//! | [`roster`] | `internal/roster` | other devices' agent cards with liveness |
//! | [`names`] | `internal/names` | friendly session names |
//! | [`person_avatar`] | (Rust only, card #393) | the person's picture |
//! | [`assistant_name`] | (Rust only, card #1045) | the name of their Office Assistant |
//! | [`devcolor`] | `internal/devcolor` | device colours |
//! | [`devname`] | (Rust only, card #184) | the person's names for their devices |
//! | [`talk`] | (Rust only, card #389) | channels, DMs and read cursors |

pub mod approvals;
pub mod assistant_name;
pub mod avatar;
pub mod board;
pub mod decision;
pub mod devcolor;
pub mod devname;
pub mod gojson;
pub(crate) mod gotime;
pub mod mentions;
pub mod names;
pub mod notifications;
pub mod person_avatar;
pub mod roster;
pub mod sides;
pub mod talk;

pub use board::*;
/// The one spelling of a repoKey, and its short name for people (#480).
pub use ledger::{canonical_repo_key, repo_name, same_repo_key};
