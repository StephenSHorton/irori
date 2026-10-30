//! The `node.addr` row payload: where a home server can be reached
//! (I:internal/mesh/servers.go:21-88, mesh.md §2.6). Only the server's own
//! device writes it, and only when something in it changes.
//!
//! Go keeps this type in `mesh` (servers.go), and so does the port: this
//! module re-exports the one copy in the mesh crate's `servers` (its JSON
//! bytes, `parse` as Go's `json.Unmarshal`, `same`, `dial`, `outside`),
//! which the mesh crate's `Servers` fold reads.

pub use mesh::servers::{NodeAddr, PublicAddr, REACH_CGNAT, REACH_OFF, REACH_OK, REACH_UNMAPPED};
