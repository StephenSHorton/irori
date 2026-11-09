//! All of this crate's integration tests, in one binary (card #566): each
//! tests/*.rs used to be its own binary carrying its own copy of the crate's
//! debug info. Tests from every module run in this one process, side by
//! side; a test that changes process state lives outside this binary.

mod board;
mod boards2;
mod capture;
mod common;
mod device_free;
mod finished_test;
mod golden;
mod later;
mod mention;
mod no_limit_tools;
mod notify;
mod personal_boards;
mod ported;
mod shared;
mod sides;
mod talk;
mod waits;
mod weight;
