//! All of this crate's integration tests, in one binary (card #566): each
//! tests/*.rs used to be its own binary carrying its own copy of the crate's
//! debug info. Tests from every module run in this one process, side by
//! side; a test that changes process state lives outside this binary.

mod capture_golden;
mod capture_search;
mod capture_service;
mod common;
mod gitlink_git;
mod gitlink_golden;
mod repo_git;
mod transcript_golden;
