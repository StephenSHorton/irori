//! All of this crate's integration tests, in one binary (card #566): each
//! tests/*.rs used to be its own binary carrying its own copy of the crate's
//! debug info. Tests from every module run in this one process, side by
//! side; a test that changes process state lives outside this binary.

mod agentnames;
mod ask_spawn;
mod attachments;
mod board_servers;
mod boards;
mod boards2;
mod bridge2;
mod buildclean;
mod common;
mod core;
mod desk_default;
mod desk_opened;
mod desk_wake;
mod deskmap;
mod deskmap_live;
mod deskwatch;
mod dm_people;
mod expiry;
mod five_cores;
mod helpertree;
mod invites;
mod lockfile;
mod migrate;
mod names;
mod notes;
mod notifications;
mod offline;
mod pair;
mod personal_boards;
mod prefetch;
mod prwatch;
mod search;
mod space_boards;
mod spawn;
mod spawn_capable;
mod spawn_checkout;
mod spawn_devices;
mod spawn_helpers;
mod spawn_paths;
mod spawn_queue;
mod spawn_settings_devices;
mod talk_notify;
