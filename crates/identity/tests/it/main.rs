//! All of this crate's integration tests, in one binary (card #566): each
//! tests/*.rs used to be its own binary carrying its own copy of the crate's
//! debug info. Tests from every module run in this one process, side by
//! side; a test that changes process state lives outside this binary.

mod board;
mod boardsecret;
mod chain_keys;
mod files;
mod golden;
mod identity;
mod keychain;
mod keystore;
mod membership;
mod rotate;
mod support;
