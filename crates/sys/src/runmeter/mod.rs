//! Metering and stopping one background run's process tree (card #1249,
//! docs/design/live-tasks.md §4, §5.3).
//!
//! A run is known by the `.output` file Claude Code writes its stdout to.
//! Its members are found from that file, not by walking down from the
//! session: on Windows MSYS fork/exec leaves `ParentProcessId` pointing at a
//! dead intermediate, so a walk from `claude.exe` never reaches the real
//! work, while every member inherited the file as stdout.
//!
//! | | Windows | macOS | other |
//! |---|---|---|---|
//! | [`holders`] | Restart Manager on the file | fd 1 of the session's children (L4) | empty |
//! | members | holders + native descendants by ppid (ToolHelp) | process group + ppid walk (L4) | empty |
//! | CPU, memory | `GetProcessTimes`, `PROCESS_MEMORY_COUNTERS_EX.PrivateUsage` | `proc_pid_rusage` (L4) | none |
//! | [`end_tree`] | `TerminateProcess` per member | `kill(-pgid)` (L4) | refuses |
//!
//! A member is `(pid, start)`: `start` is when the process started, as
//! Windows FILETIME ticks (100 ns since 1601, [`crate::proc::filetime_ticks`])
//! on every OS, so a reused pid never matches. Irori's own process and the
//! session's (claude's) are never members.
//!
//! Approximate by nature: a process that starts and exits between two
//! samples is never seen, and a member first seen late adds nothing until
//! its next sample.

use std::path::{Path, PathBuf};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use self::windows as imp;

// ---- macOS (card #1249 L4, branch card-1249-L4) -------------------------
// L4 adds macos.rs with the same `imp` items as windows.rs and the no-op
// below, then makes three edits here:
//   1. uncomment these two lines:
// #[cfg(target_os = "macos")]
// mod macos;
// #[cfg(target_os = "macos")]
// use self::macos as imp;
//   2. narrow the no-op's cfg to `not(any(windows, target_os = "macos"))`;
//   3. widen SUPPORTED to `cfg!(any(windows, target_os = "macos"))`.
// Until then macOS gets the no-op, so the crate builds on every target.
// -------------------------------------------------------------------------

/// Linux, Android, iOS (and macOS until L4): nothing to meter, nothing to
/// end, so core's calls compile everywhere.
#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    pub(super) struct Sampler;
    impl Sampler {
        pub(super) fn new() -> Sampler {
            Sampler
        }
        pub(super) fn sample(
            &mut self,
            _runs: &[super::RunKey],
        ) -> Vec<(std::path::PathBuf, super::TreeSample)> {
            Vec::new()
        }
    }
    pub(super) fn holders(_output: &Path) -> Vec<(u32, u64)> {
        Vec::new()
    }
    pub(super) fn end_tree(members: &[(u32, u64)]) -> Result<(), String> {
        if members.is_empty() {
            Ok(())
        } else {
            Err("can't end processes on this system".into())
        }
    }
}

/// Whether this OS meters runs at all.
pub const SUPPORTED: bool = cfg!(windows);

/// One run to sample: its output file and the claude process that started
/// it (never counted as a member).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunKey {
    pub output: PathBuf,
    pub session_pid: u32,
}

/// One run's tree at one sample.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TreeSample {
    /// CPU in cores (1.0 = one logical processor busy), summed over the
    /// tree; `None` on the run's first sample.
    pub cores: Option<f32>,
    /// Private bytes summed over the tree (summed working sets would count
    /// shared pages once per process).
    pub mem_bytes: u64,
    /// How many processes the tree has.
    pub procs: u32,
    /// Every member as `(pid, start)`.
    pub members: Vec<(u32, u64)>,
}

/// Keeps what CPU deltas need between samples, per run. Build one and keep
/// it: a run's first sample only primes its CPU.
pub struct TreeSampler {
    imp: imp::Sampler,
}

impl TreeSampler {
    pub fn new() -> TreeSampler {
        TreeSampler {
            imp: imp::Sampler::new(),
        }
    }

    /// Samples every run in `runs` now. Runs not passed are forgotten. A
    /// run whose tree is gone comes back with no members.
    pub fn sample(&mut self, runs: &[RunKey]) -> Vec<(PathBuf, TreeSample)> {
        self.imp.sample(runs)
    }
}

impl Default for TreeSampler {
    fn default() -> Self {
        TreeSampler::new()
    }
}

/// The processes holding `output` open, as `(pid, start)`, Irori's own
/// process left out. Reads nothing from the file. Empty when it can't be
/// told.
pub fn holders(output: &Path) -> Vec<(u32, u64)> {
    imp::holders(output)
}

/// Ends every member still running, each only if the process at its pid
/// still started at its `start`, plus native descendants started since.
/// Never ends Irori itself. `Err` names the ones that couldn't be ended.
pub fn end_tree(members: &[(u32, u64)]) -> Result<(), String> {
    imp::end_tree(members)
}
