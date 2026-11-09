//! All of this crate's integration tests, in one binary (card #566): each
//! tests/*.rs used to be its own binary carrying its own copy of the crate's
//! debug info. Tests from every module run in this one process, side by
//! side; a test that changes process state lives outside this binary.

mod assistant_turn;
mod boards;
mod cards;
mod cards_cache;
mod chat_held;
mod chat_held_measure;
mod common;
mod crash_answer;
mod gone_lead;
mod huddle_held;
mod later;
mod long_text;
mod mentions;
mod mesh;
mod no_copy_answer;
mod notes;
mod permission_answer;
mod pumps;
mod reopened_thread;
mod running_agents;
mod services;
mod spawned_actions;
mod storage;
mod terms;
mod transcribe_e2e;
mod trust_and_start;
mod trust_repo;
mod trust_writers;
mod weights;
mod wind_down;
