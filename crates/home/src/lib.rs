//! `irori-home`: the home server (I:internal/home, mesh.md §3.3, §5.4,
//! architecture §3.1).
//!
//! | Module | Go | What |
//! |---|---|---|
//! | [`server`] | `home.Server` (home.go, awake.go) | settings, the run loop, `node.addr` on change, Status lines, the awake hold |
//! | [`relay`] | the hub and TURN (go-room) | the embedded iroh-relay on the device key (D3), admission by cert, dropping revoked clients |
//! | [`portmap`] | go-room `portmap` | UPnP / NAT-PMP mapping of TCP and UDP N, renewals, stale cleanup |
//! | [`settings`] | `home.Settings` | `home-server.json`, tri-state `outside`/`awake` |
//! | [`nodeaddr`] | `mesh.NodeAddr` | the `node.addr` payload, `Same`, `Dial`, `Outside` (re-exported from `irori_mesh::servers`) |
//! | [`hostport`] | `roomnet.HostPort`, `net.SplitHostPort` | typed addresses |
//!
//! The keep-awake hold itself is `sys::awake`, and the network reads
//! (LAN addresses, the default gateway, IPv6, the Windows Public profile)
//! are `irori_lan::netif`.
//!
//! Seams left for the merge with the M5 mesh and net crates:
//! - `server::check_device_by_key` calls `irori_net::admit` (Go's
//!   `mesh.CheckDevice`, one copy). `nodeaddr::NodeAddr` is the mesh
//!   crate's `mesh.NodeAddr` (net's `fromdb` copy is the spike tool's).
//!   The relay's TLS is `irori_net::tls`'s.
//! - `Config::wanted` is mesh's "another certified device or an open
//!   pairing window" (Go core.go:293).
//! - `Config::on_relay` tells net's endpoint the relay's port N, so the
//!   server's own endpoint uses its own relay (Go `Net.SetLocal`), and its
//!   LAN address, which the pairing bridge names (`PairLan::set_relay`).
//! - Pairing guest windows on the relay (mesh.md §5.4) are M7: admission is
//!   members only for now.

pub mod hostport;
pub mod nodeaddr;
pub mod portmap;
pub mod relay;
pub mod server;
pub mod settings;

pub use nodeaddr::{NodeAddr, PublicAddr};
pub use server::{Config, RelayUp, Server, Status};
pub use settings::Settings;
