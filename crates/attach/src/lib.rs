//! `irori-attach`: the client side of the attach stream, `irori attach`'s
//! runner (I:internal/attach/runner, `api.Dial`/`OpenStream`; surface.md
//! §2.6, §3 "CLI rules"; architecture.md §1.1 `api::attach` + app role).
//!
//! This is what the Irori Claude Code mod spawns: `irori attach` (the
//! [`runner`]) copies the Core's NDJSON wake stream to the mod's pipe. The
//! server side, `GET /v1/attach` and `POST /v1/deliveries/*`, is the API
//! crate's (`irori_api`, stream.rs and v02.rs) and only there; the wire
//! types are [`irori_api::attach`], re-exported here.
//!
//! | Module | Go | What |
//! |---|---|---|
//! | [`client`] | `api.Dial`, `OpenStream` | lockfile, health probe, stream open |
//! | [`runner`] | `attach/runner/runner.go` | `irori attach`'s reconnect loop |
//!
//! Tests (crates/attach/tests) run the ported Go attach tests and the Go
//! attach goldens against the real `irori_api::Server` over a Core, the
//! same server `irori serve` hosts.

pub mod client;
pub mod runner;

pub use irori_api::attach::{
    APPEND_SINCE, Api, Event, HEADER_ATTACH, HEADER_MEMBER, HEADER_ORIGIN, VERSION, mod_at_least,
};
pub use irori_api::preview;
pub use runner::NOT_RUNNING;
