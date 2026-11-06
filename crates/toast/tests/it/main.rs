//! All of this crate's integration tests, in one binary (card #566): each
//! tests/*.rs used to be its own binary carrying its own copy of the crate's
//! debug info. Tests from every module run in this one process, side by
//! side; a test that changes process state lives outside this binary.

mod confetti;
mod engine;
mod model;
mod smoke_windows;
