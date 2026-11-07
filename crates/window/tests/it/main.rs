//! All of this crate's integration tests, in one binary (card #566): each
//! tests/*.rs used to be its own binary carrying its own copy of the crate's
//! debug info. Tests from every module run in this one process, side by
//! side; a test that changes process state lives outside this binary.

mod boards;
mod cards;
mod common;
mod crash_answer;
mod gone_lead;
mod later;
mod long_text;
mod mentions;
mod mesh;
mod no_copy_answer;
mod notes;
mod pumps;
mod services;
mod spawned_actions;
mod terms;
mod trust_and_start;
mod trust_repo;
mod trust_writers;
mod wind_down;
