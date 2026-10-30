//! `irori-pair`: the add-a-device flow (I:internal/pair, identity.md §4.7,
//! architecture §3.4). A device already in the mesh (A, [`Pairer::add`])
//! shows a one-use code; the new device (B, [`Pairer::join`]) enters it;
//! both meet in a pairing room derived from the code, prove they hold it,
//! and derive five SAS words over both device keys and the channel binding
//! of their one connection. B shows the words, the person types them into
//! A, and only then does A certify B and hand it the person. B in turn takes
//! that grant only once the person typed A's key fingerprint into B, so a
//! code holder that reached B first can't hand it a person of its own.
//!
//! | Module | Go | What |
//! |---|---|---|
//! | [`code`] | `code.go` | code alphabet, scrypt room secret, Info, SAS, proofs, fingerprints |
//! | `wordlist` | `wordlist.go` | the PGP word lists, generated from the Go file |
//! | [`saved`] | `saved.go` | `pair-rows.json` and `learn_saved` |
//! | [`pairer`] | `pair.go` | the flow, its states and texts |
//! | [`wire`] | `pair.go` frames | the connection frames |
//!
//! One change from Go (D5): `Conn::fingerprints` on iroh is the session's
//! TLS exporter (`EXPORTER-irori-sas`) on both sides, in place of DTLS
//! certificate fingerprints, so the words bind the QUIC session. A Go device
//! and a Rust device can't pair with each other.
//!
//! The room is a `irori_meshseam` room with epoch 0. The iroh transport
//! carries it on ALPN `irori/pair/1` (`irori_net::transport`): A announces
//! the room on the LAN lookup under the code-derived key, and a hello from B
//! is a dial that A takes up with `connect` (Go's offerer rule inverted,
//! since iroh has no signaling bus).
//!
//! Adding a device is window-only: the loopback API reaches only the joining
//! side (join, status, the other device's key, `cancel_join`), so an agent
//! with a shell can't add a device to the person.

pub mod code;
pub mod pairer;
pub mod saved;
pub mod wire;
mod wordlist;

pub use code::{
    APP, ERR_BAD_CODE, SAS_SALT, SAS_WORDS, check_proof, fingerprint, info, key_match, new_code,
    normalize_code, parse_words, proof, room_secret, room_secret_for, sas, sas_bytes,
    sas_bytes_with, sas_with, words_match,
};
pub use pairer::*;
pub use saved::{ROWS_FILE, learn_saved};

#[cfg(test)]
mod tests;
