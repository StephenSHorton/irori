//! All of this crate's integration tests, in one binary (CLAUDE.md, card
//! #566). They need real terminals, so desktop only.
#![cfg(not(any(target_os = "android", target_os = "ios")))]

mod pty;
mod tail;
