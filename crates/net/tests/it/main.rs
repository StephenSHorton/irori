//! All of this crate's integration tests, in one binary (card #566): each
//! tests/*.rs used to be its own binary carrying its own copy of the crate's
//! debug info. Tests from every module run in this one process, side by
//! side; a test that changes process state lives outside this binary.

mod board_gossip;
mod board_iroh;
#[path = "../../../mesh/tests/it/common/mod.rs"]
mod common;
mod fromdb;
mod invite_iroh;
mod iroh_key;
mod loopback;
mod mesh_iroh;
mod node_iroh;
mod public_live;
#[path = "../../../identity/tests/it/support/mod.rs"]
mod support;
