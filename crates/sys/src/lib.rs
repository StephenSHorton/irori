//! `irori-sys`: OS glue (architecture.md §1.1). This crate holds the parts
//! the capture area needs, ports of I:internal/proc and I:internal/fswatch
//! (capture.md §4), plus two small helpers they share:
//!
//! | Module | Go | Reproduces |
//! |---|---|---|
//! | [`proc`] | `internal/proc` | `Alive`, `StartTime`, `WaitExit`, `FiletimeTicks` |
//! | [`awake`] | `internal/awake` | the home server's keep-awake hold (Windows execution state, macOS IOPM assertion) |
//! | [`fswatch`] | `internal/fswatch` | `Watch` (ReadDirectoryChangesW on Windows), the coalescer with its sticky Overflow, `ErrUnsupported` elsewhere |
//! | [`cancel`] | `context.Context` cancellation | a token a blocking wait can be woken by |
//! | [`tz`] | `time.Local` | the local UTC offset at an instant |
//! | [`shellpath`] | root `guipath_darwin.go` | the desktop app's PATH from the login shell (macOS) |
//!
//! Not here yet: macOS FSEvents (fswatch returns [`fswatch::Error::Unsupported`]
//! there, so capture polls), and the paths/Mark-of-the-Web glue other
//! areas add.

pub mod awake;
pub mod cancel;
pub mod fswatch;
pub mod proc;
pub mod shellpath;
pub mod tz;
