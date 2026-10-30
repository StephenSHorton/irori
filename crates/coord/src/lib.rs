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
//! | [`devcolor`] | `internal/devcolor` | device colours |

pub mod approvals;
pub mod board;
pub mod decision;
pub mod devcolor;
pub mod gojson;
pub(crate) mod gotime;
pub mod names;
pub mod roster;

pub use board::*;
