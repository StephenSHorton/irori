//! Windows run metering (live-tasks.md §4.1): Restart Manager names the
//! processes holding a run's `.output` file, one ToolHelp snapshot per
//! sample adds their native descendants by ppid, and each member is read
//! with `GetProcessTimes` and `GetProcessMemoryInfo`.

use std::collections::{HashMap, HashSet};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_MORE_DATA, ERROR_SUCCESS, FILETIME, HANDLE, INVALID_HANDLE_VALUE,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources,
    RmStartSession,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

use super::{RunKey, TreeSample};
use crate::proc::filetime_ticks;

/// A holder that started this long before its output file was created
/// still counts: the shell that opens the file by redirection starts a
/// moment before the file exists. Older ones (a person's `tail -F`) don't.
const BEFORE_FILE_TICKS: u64 = 2 * 10_000_000;

/// With more runs than this, Restart Manager (about 57 ms a file) is asked
/// only every [`RM_EVERY`] samples; cached holders and ToolHelp in between.
const RM_EACH_TICK_UP_TO: usize = 4;
const RM_EVERY: u32 = 3;

/// The exit code a stopped member gets.
const STOPPED_CODE: u32 = 1;

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful Open/Create call.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn open(access: u32, pid: u32) -> Option<Handle> {
    // SAFETY: OpenProcess has no pointer arguments.
    let h = unsafe { OpenProcess(access, 0, pid) };
    (!h.is_null()).then_some(Handle(h))
}

fn ft(f: FILETIME) -> u64 {
    (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
}

/// Whether the process is still running: an exited one stays openable
/// while anyone (its parent) keeps a handle to it.
fn running(h: &Handle) -> bool {
    const STILL_ACTIVE: u32 = 259;
    let mut code = 0u32;
    // SAFETY: out-parameter, live handle.
    unsafe { GetExitCodeProcess(h.0, &mut code) != 0 && code == STILL_ACTIVE }
}

/// `(created, user + kernel)` in 100 ns ticks.
fn times(h: &Handle) -> Option<(u64, u64)> {
    let z = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut c, mut e, mut k, mut u) = (z, z, z, z);
    // SAFETY: out-parameters on the stack, a live process handle.
    if unsafe { GetProcessTimes(h.0, &mut c, &mut e, &mut k, &mut u) } == 0 {
        return None;
    }
    Some((ft(c), ft(k) + ft(u)))
}

/// Private bytes (commit charge), 0 when it can't be read.
fn private_bytes(h: &Handle) -> u64 {
    let mut m = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: the _EX struct starts with the plain one and cb says its full
    // size; a live process handle.
    let ok = unsafe {
        GetProcessMemoryInfo(
            h.0,
            (&mut m as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            m.cb,
        )
    };
    if ok == 0 { 0 } else { m.PrivateUsage as u64 }
}

/// When `pid` started, if it is running and can be opened.
fn started(pid: u32) -> Option<u64> {
    let h = open(PROCESS_QUERY_LIMITED_INFORMATION, pid)?;
    if !running(&h) {
        return None;
    }
    times(&h).map(|t| t.0)
}

/// Every process now, as `pid -> ppid`.
fn snapshot() -> HashMap<u32, u32> {
    let mut out = HashMap::new();
    // SAFETY: a process snapshot; no pointer arguments.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE || snap.is_null() {
        return out;
    }
    let snap = Handle(snap);
    // SAFETY: PROCESSENTRY32W is plain data; dwSize is set before use.
    let mut e: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    // SAFETY: a live snapshot handle and an entry with dwSize set.
    let mut ok = unsafe { Process32FirstW(snap.0, &mut e) } != 0;
    while ok {
        out.insert(e.th32ProcessID, e.th32ParentProcessID);
        // SAFETY: as above.
        ok = unsafe { Process32NextW(snap.0, &mut e) } != 0;
    }
    out
}

/// Whether children with ppid `pid` may be counted as children of the
/// process that started at `start`: the process at `pid` is still that one
/// (exited but held open counts, as `end_tree`'s second pass needs), or the
/// pid is free and its children are orphans. A pid now held by another
/// process (reused) is never expanded: its children aren't the run's.
fn may_expand(pid: u32, start: u64, procs: &HashMap<u32, u32>) -> bool {
    match open(PROCESS_QUERY_LIMITED_INFORMATION, pid) {
        Some(h) => times(&h).map(|t| t.0) == Some(start),
        None => !procs.contains_key(&pid),
    }
}

/// `roots` plus their native descendants by ppid, roots first, then
/// breadth first. A child created before its parent is a reused ppid and is
/// left out, as are `skip` pids and the children of a pid now held by
/// another process.
fn with_descendants(
    roots: &[(u32, u64)],
    procs: &HashMap<u32, u32>,
    skip: &[u32],
) -> Vec<(u32, u64)> {
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, &ppid) in procs {
        if pid != 0 && pid != ppid {
            kids.entry(ppid).or_default().push(pid);
        }
    }
    let mut seen: HashSet<u32> = HashSet::new();
    let mut out = Vec::new();
    for &(pid, start) in roots {
        if !skip.contains(&pid) && seen.insert(pid) {
            out.push((pid, start));
        }
    }
    let mut i = 0;
    while i < out.len() {
        let (pid, start) = out[i];
        i += 1;
        let Some(cs) = kids.get(&pid) else { continue };
        if !may_expand(pid, start, procs) {
            continue;
        }
        for &c in cs {
            if skip.contains(&c) || seen.contains(&c) {
                continue;
            }
            let Some(cs) = started(c) else { continue };
            if cs >= start {
                seen.insert(c);
                out.push((c, cs));
            }
        }
    }
    out
}

/// `p` as a NUL-terminated wide path, `/` turned into `\`, losing nothing
/// (a lossy UTF-8 round trip would mangle unpaired surrogates).
fn wide(p: &Path) -> Vec<u16> {
    p.as_os_str()
        .encode_wide()
        .map(|c| {
            if c == u16::from(b'/') {
                u16::from(b'\\')
            } else {
                c
            }
        })
        .chain(std::iter::once(0))
        .collect()
}

struct RmSession(u32);

impl Drop for RmSession {
    fn drop(&mut self) {
        // SAFETY: a session handle from a successful RmStartSession.
        unsafe {
            RmEndSession(self.0);
        }
    }
}

/// Every process Restart Manager says holds `path`, as `(pid, start)`.
fn rm_holders(path: &Path) -> Vec<(u32, u64)> {
    let mut handle = 0u32;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    // SAFETY: out-parameter and a key buffer of CCH_RM_SESSION_KEY + 1.
    if unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) } != ERROR_SUCCESS {
        return Vec::new();
    }
    let session = RmSession(handle);
    let name = wide(path);
    let names = [name.as_ptr()];
    // SAFETY: one NUL-terminated wide path that outlives the call.
    let r = unsafe {
        RmRegisterResources(
            session.0,
            1,
            names.as_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
        )
    };
    if r != ERROR_SUCCESS {
        return Vec::new();
    }
    let mut list: Vec<RM_PROCESS_INFO> = Vec::new();
    for _ in 0..8 {
        let mut needed = 0u32;
        let mut count = list.len() as u32;
        let mut reasons = 0u32;
        let ptr = if list.is_empty() {
            std::ptr::null_mut()
        } else {
            list.as_mut_ptr()
        };
        // SAFETY: `ptr` has room for `count` entries (or is null with 0).
        let r = unsafe { RmGetList(session.0, &mut needed, &mut count, ptr, &mut reasons) };
        if r == ERROR_SUCCESS {
            list.truncate(count as usize);
            return list
                .iter()
                .map(|p| (p.Process.dwProcessId, ft(p.Process.ProcessStartTime)))
                .collect();
        }
        if r != ERROR_MORE_DATA {
            return Vec::new();
        }
        // The list can grow between calls: leave room.
        list = vec![RM_PROCESS_INFO::default(); needed as usize + 4];
    }
    Vec::new()
}

pub(super) fn holders(output: &Path) -> Vec<(u32, u64)> {
    let me = std::process::id();
    let born = std::fs::metadata(output)
        .and_then(|m| m.created())
        .map(filetime_ticks)
        .ok();
    let mut out: Vec<(u32, u64)> = rm_holders(output)
        .into_iter()
        .filter(|&(pid, start)| pid != me && born.is_none_or(|b| start + BEFORE_FILE_TICKS >= b))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

struct RunState {
    /// CPU ticks per member at the last sample.
    cpu: HashMap<(u32, u64), u64>,
    /// When the last sample was taken, wall and FILETIME.
    last: Option<(Instant, u64)>,
    holders: Vec<(u32, u64)>,
    tick: u32,
}

pub(super) struct Sampler {
    runs: HashMap<PathBuf, RunState>,
}

impl Sampler {
    pub(super) fn new() -> Sampler {
        Sampler {
            runs: HashMap::new(),
        }
    }

    pub(super) fn sample(&mut self, runs: &[RunKey]) -> Vec<(PathBuf, TreeSample)> {
        let keep: HashSet<&PathBuf> = runs.iter().map(|r| &r.output).collect();
        self.runs.retain(|p, _| keep.contains(p));
        if runs.is_empty() {
            return Vec::new();
        }
        let procs = snapshot();
        let me = std::process::id();
        let many = runs.len() > RM_EACH_TICK_UP_TO;
        let mut out = Vec::with_capacity(runs.len());
        for run in runs {
            let st = self.runs.entry(run.output.clone()).or_insert(RunState {
                cpu: HashMap::new(),
                last: None,
                holders: Vec::new(),
                tick: 0,
            });
            // Cached holders that are gone (or whose pid was reused) drop.
            st.holders
                .retain(|&(pid, start)| procs.contains_key(&pid) && started(pid) == Some(start));
            if !many || st.tick.is_multiple_of(RM_EVERY) || st.holders.is_empty() {
                st.holders = holders(&run.output);
            }
            st.tick = st.tick.wrapping_add(1);
            let members = with_descendants(&st.holders, &procs, &[me, run.session_pid]);

            let now = Instant::now();
            let now_ft = filetime_ticks(SystemTime::now());
            let mut s = TreeSample::default();
            let mut cpu = HashMap::with_capacity(members.len());
            let mut delta: u64 = 0;
            for &(pid, start) in &members {
                let Some(h) = open(PROCESS_QUERY_LIMITED_INFORMATION, pid) else {
                    continue;
                };
                let Some((created, ticks)) = times(&h) else {
                    continue;
                };
                if created != start || !running(&h) {
                    continue;
                }
                s.mem_bytes += private_bytes(&h);
                s.members.push((pid, start));
                match (st.cpu.get(&(pid, start)), st.last) {
                    (Some(&prev), _) => delta += ticks.saturating_sub(prev),
                    // New since the last sample: all its CPU is new.
                    (None, Some((_, last_ft))) if start >= last_ft => delta += ticks,
                    _ => {}
                }
                cpu.insert((pid, start), ticks);
            }
            s.procs = s.members.len() as u32;
            s.cores = st.last.and_then(|(t, _)| {
                let wall = now.duration_since(t).as_nanos() as f64 / 100.0;
                (wall > 0.0).then(|| (delta as f64 / wall) as f32)
            });
            st.cpu = cpu;
            st.last = Some((now, now_ft));
            out.push((run.output.clone(), s));
        }
        out
    }
}

pub(super) fn end_tree(members: &[(u32, u64)]) -> Result<(), String> {
    let me = std::process::id();
    let mut ended: Vec<(u32, u64)> = Vec::new();
    let mut waits: Vec<(u32, Handle)> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    // Twice: the first pass stops the parents, the second catches children
    // a parent spawned while the first was ending it.
    let mut roots: Vec<(u32, u64)> = members.to_vec();
    for _ in 0..2 {
        let procs = snapshot();
        let mut next = Vec::new();
        for (pid, start) in with_descendants(&roots, &procs, &[me]) {
            if ended.contains(&(pid, start)) {
                continue;
            }
            let Some(h) = open(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                pid,
            ) else {
                // Gone already, or not ours to end.
                if started(pid) == Some(start) {
                    failed.push(format!("{pid} (can't open it)"));
                }
                continue;
            };
            if times(&h).map(|t| t.0) != Some(start) || !running(&h) {
                continue; // exited, or the pid now belongs to someone else
            }
            // SAFETY: a live handle opened with PROCESS_TERMINATE.
            if unsafe { TerminateProcess(h.0, STOPPED_CODE) } == 0 {
                let e = std::io::Error::last_os_error();
                failed.push(format!("{pid} ({e})"));
                continue;
            }
            ended.push((pid, start));
            next.push((pid, start));
            waits.push((pid, h));
        }
        if next.is_empty() {
            break;
        }
        roots = next;
    }
    // One 3 s deadline for all of them, not 3 s each.
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    for (pid, h) in waits {
        let left = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u32;
        // SAFETY: a live handle opened with SYNCHRONIZE.
        if unsafe { WaitForSingleObject(h.0, left) } != WAIT_OBJECT_0 {
            failed.push(format!("{pid} (still running)"));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("couldn't end {}", failed.join(", ")))
    }
}
