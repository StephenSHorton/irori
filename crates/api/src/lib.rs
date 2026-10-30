//! `irori-api`: Irori's loopback HTTP API (I:internal/api,
//! I:internal/attach; surface.md §1, §2.4-2.7, §6, §7.8, §7.12, §7.14).
//!
//! Used by `irori mcp`, `irori attach`, the Claude Code mod, `irori events`
//! and suzuri; the window calls the Core directly. Every `/v1` route Go
//! serves is here, with Go's checks in Go's order, Go's JSON bytes (through
//! `gocompat::json`, so `<>&` are escaped and every response ends in
//! `\n`), Go's error texts and statuses, and both NDJSON streams.
//!
//! Security: listen on 127.0.0.1 only; every route except `GET
//! /v1/health` needs `Authorization: Bearer <token>` (constant-time); a
//! Host that isn't `127.0.0.1:<port>` or `localhost:<port>` and any
//! request with an Origin header are 403. The person's view needs this
//! run's `X-Irori-Person` secret ([`Server::person_secret`]), held only by
//! the window; the token alone reads as an unidentified agent.
//!
//! | Module | Go | Routes |
//! |---|---|---|
//! | [`server`] | `server.go`, `api.go` | checks, health, device, agents, devices |
//! | [`events`] | `server.go`, `routes_events.go` | events, events/stream |
//! | `v02` | `routes_v02.go`, `v02.go` | members, deliveries, tools, policy, inbox |
//! | `stream` | `stream.go` | attach |
//! | `v03` | `routes_v03.go`, `v03.go` | timeline, edits, commits, blobs, decisions, prs, capture/status, why |
//! | [`board`] | `routes_board.go`, `board.go`, `board_overview.go` | board, tasks, search, show, overview |
//! | [`pair`] | `routes_pair.go`, `pairing.go`, `client_pair.go` | pair/{join,status,key,cancel} (the joining side only) |
//! | [`attach`] | `internal/attach` | the attach wire types, `ModAtLeast` |
//! | [`redact`] | tools `Redact` | rows as an agent sees them |
//! | [`client`] | `client*.go` | a small client: lockfile dial, GET/POST, the events stream |
//!
//! Wiring notes (what the merge plugs in):
//! - Host: `irori serve --headless` (app crate) starts the [`irori_core::Core`],
//!   then [`Server::new`]`(core, core.token())`, [`listen`],
//!   `core.write_lockfile(port)`, and [`Server::serve`] until Ctrl+C. The GUI
//!   role does the same and keeps [`Server::person_secret`] for the window.
//! - Tools: [`CoreTools`] serves the Core's tools crate `Set` as the
//!   [`ToolSet`]; pass it with [`Server::with_tools`]. Without one,
//!   `GET /v1/tools` is `[]` and `POST /v1/tools/{name}` is 503
//!   (`tools are not running`).
//! - Attach: [`attach::Event`] and the header names are the wire types for
//!   the `irori attach` runner and `irori mcp`; [`client`] dials the
//!   lockfile. [`redact::redact`] is the tools crate's `Redact` too.
//! - Mesh (M5): `/v1/devices[?detail=1]` is [`Backend::mesh_devices`]
//!   (Go `Mesh().Devices`), 503 for a backend without a mesh. Health's
//!   `net` (`{state, peers}`) is [`Backend::net_health`]; a backend
//!   without a mesh leaves it out, as Go does.
//! - Timeouts: Go's ReadHeader 5 s, Read 10 s, Write 30 s and Idle 60 s
//!   are per-connection deadlines on the socket ([`conn`]); the attach and
//!   events streams lift them, as Go's do. A body is read only after the
//!   Origin, Host, method and token checks, and only for a POST.

pub mod attach;
pub mod backend;
pub mod board;
mod body;
pub mod client;
pub mod conn;
pub mod events;
mod http;
pub mod pair;
pub mod redact;
pub mod server;
mod stream;
mod v02;
mod v03;

pub use backend::{Backend, Caller, CoreTools, NetHealth, ToolError, ToolSet};
pub use conn::Timeouts;
pub use irori_core::Member;
pub use server::{DEFAULT_EVENT_LIMIT, KEEPALIVE, MAX_EVENT_LIMIT, Server, listen};
pub use stream::preview;
pub use v02::{MAX_INBOX_LIMIT, MAX_MEMBER_NAME};
pub use v03::{MAX_CAPTURE_LIMIT, MAX_TIMELINE_LIMIT};

/// Carries the person's per-run secret (decision D10(a)).
pub const HEADER_PERSON: &str = "X-Irori-Person";

/// Who a request with no identity header reads as: an unidentified agent.
/// It is nobody's key, so it reads no message and owns no card.
pub const AGENT_VIEWER: &str = "api:agent";

/// Bounds the deliveries scanned for inbox counts.
pub(crate) const COUNT_LIMIT: usize = 1 << 20;

pub(crate) fn logf(args: std::fmt::Arguments<'_>) {
    irori_core::log(&args.to_string());
}
