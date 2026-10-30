//! Connection close reasons, Go's strings (mesh.md §5.2 "Close reasons").
//! On iroh they travel in-band as the QUIC application close reason.

pub const VERSION: &str = "version";
pub const PERSON: &str = "person";
pub const KEY: &str = "key";
pub const PROTOCOL: &str = "protocol";
pub const FRAME_BEFORE_HELLO: &str = "protocol: frame before hello";
pub const HELLO: &str = "hello";
pub const REVOKED: &str = "revoked";
pub const EPOCH: &str = "epoch";
pub const LEAVING: &str = "leaving";
pub const SHUTTING_DOWN: &str = "shutting down";
pub const NOT_RUNNING: &str = "not running";
pub const REPLACED: &str = "replaced";
pub const REFUSED: &str = "refused";
pub const TIMEOUT: &str = "timeout";

/// The QUIC application error code a reason closes with: refusals and
/// replacements get their own codes so they read without parsing text.
pub fn code(reason: &str) -> u32 {
    match reason {
        REFUSED => 1,
        REPLACED => 2,
        REVOKED => 3,
        EPOCH => 4,
        _ => 0,
    }
}
