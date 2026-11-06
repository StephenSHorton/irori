//! All of this crate's integration tests, in one binary (card #566): each
//! tests/*.rs used to be its own binary carrying its own copy of the crate's
//! debug info. Tests from every module run in this one process, side by
//! side; a test that changes process state lives outside this binary.

mod bridge;
mod common;
mod edit;
mod golden;
mod inbox;
mod mention;
mod mesh;
mod not_answering;
mod notes;
mod notify;
mod policy;
mod pr;
mod replay;
mod stop;
mod sweep;
mod untaken;
mod wake;
mod wind_down;
