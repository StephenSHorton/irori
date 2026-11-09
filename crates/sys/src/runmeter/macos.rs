//! macOS run metering (live-tasks.md §4.2). The processes holding a run's
//! `.output` file are the ones whose fd 1 is it (`proc_pidfdinfo`). A
//! holder that leads its own process group (the `zsh -c` root does) brings
//! its group, and a ppid walk over one `proc_listallpids` snapshot adds
//! children that moved to another group. Each member is read with
//! `proc_pid_rusage` and stopped with `kill`.
//!
//! libc lacks `PROC_PIDFDVNODEPATHINFO` and `vnode_fdinfowithpath`, so the
//! buffer for it is declared here from `<sys/proc_info.h>`, checked by a
//! size test against the structs libc does have.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{RunKey, TreeSample};
use crate::proc::filetime_ticks;

/// `proc_pidfdinfo` flavor: `struct vnode_fdinfowithpath`.
const PROC_PIDFDVNODEPATHINFO: libc::c_int = 2;
/// `sizeof(struct vnode_fdinfowithpath)`: `proc_fileinfo` (24) +
/// `vnode_info` (152: `vinfo_stat` 136, `vi_type`, `vi_pad`, `vi_fsid`) +
/// `vip_path[MAXPATHLEN]` (1024).
const FDINFO_SIZE: usize = 24 + 152 + 1024;
/// Where `vip_path` starts in it.
const FDINFO_PATH: usize = 24 + 152;
/// `p_stat` / `pbi_status` of an exited, unreaped process (SZOMB).
const SZOMB: u32 = 5;
/// How long SIGTERM gets before SIGKILL.
const TERM_GRACE: Duration = Duration::from_secs(3);

/// What one pid looks like in a snapshot.
#[derive(Clone, Copy)]
struct Proc {
    ppid: u32,
    pgid: u32,
    /// Start as Windows FILETIME ticks, the unit members use on every OS.
    start: u64,
    zombie: bool,
}

fn bsdinfo(pid: u32) -> Option<Proc> {
    // SAFETY: proc_bsdinfo is plain data; the kernel fills it.
    let mut b: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: the buffer is `size` bytes.
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut b as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if n != size || b.pbi_pid != pid {
        return None;
    }
    let t = UNIX_EPOCH + Duration::new(b.pbi_start_tvsec, (b.pbi_start_tvusec as u32) * 1000);
    Some(Proc {
        ppid: b.pbi_ppid,
        pgid: b.pbi_pgid,
        start: filetime_ticks(t),
        zombie: b.pbi_status == SZOMB,
    })
}

/// Runs a `proc_list*` call that returns a pid count, growing the buffer
/// until it fits.
fn list_pids(call: impl Fn(*mut libc::c_void, libc::c_int) -> libc::c_int) -> Vec<u32> {
    let mut cap = 1024usize;
    loop {
        let mut buf = vec![0i32; cap];
        let n = call(buf.as_mut_ptr().cast(), (cap * 4) as libc::c_int);
        if n <= 0 {
            return Vec::new();
        }
        let n = n as usize;
        if n >= cap && cap < 1 << 20 {
            cap *= 4;
            continue;
        }
        buf.truncate(n.min(cap));
        return buf
            .into_iter()
            .filter(|&p| p > 0)
            .map(|p| p as u32)
            .collect();
    }
}

fn all_pids() -> Vec<u32> {
    // SAFETY: the callee writes at most the size it is given.
    list_pids(|b, n| unsafe { libc::proc_listallpids(b, n) })
}

fn group_pids(pgid: u32) -> Vec<u32> {
    // SAFETY: as above.
    list_pids(|b, n| unsafe { libc::proc_listpgrppids(pgid as libc::pid_t, b, n) })
}

/// Every live process now.
fn snapshot() -> HashMap<u32, Proc> {
    all_pids()
        .into_iter()
        .filter_map(|p| bsdinfo(p).map(|i| (p, i)))
        .collect()
}

/// The path pid's fd 1 is open on, if it is a file.
fn fd1_path(pid: u32) -> Option<PathBuf> {
    let mut buf = vec![0u8; FDINFO_SIZE];
    // SAFETY: the buffer is FDINFO_SIZE bytes.
    let n = unsafe {
        libc::proc_pidfdinfo(
            pid as libc::c_int,
            1,
            PROC_PIDFDVNODEPATHINFO,
            buf.as_mut_ptr().cast(),
            FDINFO_SIZE as libc::c_int,
        )
    };
    if n != FDINFO_SIZE as libc::c_int {
        return None;
    }
    let raw = &buf[FDINFO_PATH..];
    let len = raw.iter().position(|&c| c == 0)?;
    if len == 0 {
        return None;
    }
    use std::os::unix::ffi::OsStrExt;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&raw[..len])))
}

/// `p` with symlinks resolved (`/tmp` is `/private/tmp`), as the kernel
/// reports fd paths; unchanged when it can't be.
fn resolved(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

pub(super) fn holders(output: &Path) -> Vec<(u32, u64)> {
    let me = std::process::id();
    let want = resolved(output);
    let mut out: Vec<(u32, u64)> = all_pids()
        .into_iter()
        .filter(|&p| p != me)
        .filter_map(|p| {
            let info = bsdinfo(p)?;
            (!info.zombie && fd1_path(p).is_some_and(|f| f == want)).then_some((p, info.start))
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Whether children of `pid` may be counted as children of the process
/// that started at `start`: the process at `pid` is still that one, or the
/// pid is free and its children are orphans. A pid held by another process
/// (reused) is never expanded.
fn may_expand(pid: u32, start: u64, procs: &HashMap<u32, Proc>) -> bool {
    match procs.get(&pid) {
        Some(p) => p.start == start,
        None => true,
    }
}

/// `roots` plus their process groups (for roots leading their own) and
/// ppid descendants, roots first. A child started before its parent is a
/// reused ppid and left out, as are `skip` pids and anything in a skipped
/// pid's group.
fn with_descendants(
    roots: &[(u32, u64)],
    procs: &HashMap<u32, Proc>,
    skip: &[u32],
) -> Vec<(u32, u64)> {
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, p) in procs {
        if pid != p.ppid {
            kids.entry(p.ppid).or_default().push(pid);
        }
    }
    let skip_groups: HashSet<u32> = skip
        .iter()
        .filter_map(|s| procs.get(s).map(|p| p.pgid))
        .collect();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut out: Vec<(u32, u64)> = Vec::new();
    let mut add = |pid: u32, start: u64, out: &mut Vec<(u32, u64)>| {
        if !skip.contains(&pid) && seen.insert(pid) {
            out.push((pid, start));
        }
    };
    for &(pid, start) in roots {
        add(pid, start, &mut out);
    }
    let mut i = 0;
    while i < out.len() {
        let (pid, start) = out[i];
        i += 1;
        if !may_expand(pid, start, procs) {
            continue;
        }
        // A member leading its own group brings the group, unless that
        // group is one we must not touch.
        if let Some(p) = procs.get(&pid)
            && p.pgid == pid
            && !skip_groups.contains(&pid)
        {
            for m in group_pids(pid) {
                if let Some(mp) = procs.get(&m)
                    && mp.start >= start
                    && !mp.zombie
                {
                    add(m, mp.start, &mut out);
                }
            }
        }
        for &c in kids.get(&pid).map(Vec::as_slice).unwrap_or_default() {
            let Some(cp) = procs.get(&c) else { continue };
            if cp.start >= start && !cp.zombie {
                add(c, cp.start, &mut out);
            }
        }
    }
    out
}

/// `(cpu ns, phys footprint bytes)` of a process.
fn usage(pid: u32, numer: u64, denom: u64) -> Option<(u64, u64)> {
    // SAFETY: rusage_info_v2 is plain data.
    let mut r: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    // SAFETY: proc_pid_rusage takes a pointer to the flavor's struct.
    let rc = unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V2,
            (&mut r as *mut libc::rusage_info_v2).cast(),
        )
    };
    if rc != 0 {
        return None;
    }
    // Mach absolute units; nanoseconds on Intel, not on Apple Silicon.
    let ticks = u128::from(r.ri_user_time) + u128::from(r.ri_system_time);
    let ns = ticks * u128::from(numer) / u128::from(denom.max(1));
    Some((ns.min(u128::from(u64::MAX)) as u64, r.ri_phys_footprint))
}

#[allow(deprecated)]
fn timebase() -> (u64, u64) {
    let mut t = libc::mach_timebase_info { numer: 0, denom: 0 };
    // SAFETY: out-parameter.
    if unsafe { libc::mach_timebase_info(&mut t) } != 0 || t.numer == 0 || t.denom == 0 {
        return (1, 1);
    }
    (u64::from(t.numer), u64::from(t.denom))
}

struct RunState {
    /// CPU ns per member at the last sample.
    cpu: HashMap<(u32, u64), u64>,
    /// When the last sample was taken, wall and FILETIME.
    last: Option<(Instant, u64)>,
    holders: Vec<(u32, u64)>,
}

pub(super) struct Sampler {
    runs: HashMap<PathBuf, RunState>,
    timebase: (u64, u64),
}

impl Sampler {
    pub(super) fn new() -> Sampler {
        Sampler {
            runs: HashMap::new(),
            timebase: timebase(),
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
        let (numer, denom) = self.timebase;
        let mut out = Vec::with_capacity(runs.len());
        for run in runs {
            let st = self.runs.entry(run.output.clone()).or_insert(RunState {
                cpu: HashMap::new(),
                last: None,
                holders: Vec::new(),
            });
            // Cached holders that are gone (or whose pid was reused) drop;
            // the file is asked again only when none is left.
            st.holders
                .retain(|&(pid, start)| procs.get(&pid).is_some_and(|p| p.start == start));
            if st.holders.is_empty() {
                st.holders = holders(&run.output);
            }
            let members = with_descendants(&st.holders, &procs, &[me, run.session_pid]);

            let now = Instant::now();
            let now_ft = filetime_ticks(SystemTime::now());
            let mut s = TreeSample::default();
            let mut cpu = HashMap::with_capacity(members.len());
            let mut delta: u64 = 0;
            for &(pid, start) in &members {
                let Some((ns, footprint)) = usage(pid, numer, denom) else {
                    continue;
                };
                s.mem_bytes += footprint;
                s.members.push((pid, start));
                match (st.cpu.get(&(pid, start)), st.last) {
                    (Some(&prev), _) => delta += ns.saturating_sub(prev),
                    // New since the last sample: all its CPU is new.
                    (None, Some((_, last_ft))) if start >= last_ft => delta += ns,
                    _ => {}
                }
                cpu.insert((pid, start), ns);
            }
            s.procs = s.members.len() as u32;
            s.cores = st.last.and_then(|(t, _)| {
                let wall = now.duration_since(t).as_nanos() as f64;
                (wall > 0.0).then(|| (delta as f64 / wall) as f32)
            });
            st.cpu = cpu;
            st.last = Some((now, now_ft));
            out.push((run.output.clone(), s));
        }
        out
    }
}

/// Whether `(pid, start)` is still that running process.
fn live(pid: u32, start: u64) -> bool {
    bsdinfo(pid).is_some_and(|p| p.start == start && !p.zombie)
}

fn send(pid: u32, sig: libc::c_int) {
    // SAFETY: kill with a pid we verified a moment ago.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

pub(super) fn end_tree(members: &[(u32, u64)]) -> Result<(), String> {
    let me = std::process::id();
    // SAFETY: getpgrp has no arguments.
    let my_group = unsafe { libc::getpgrp() } as u32;
    let procs = snapshot();
    let mut targets: Vec<(u32, u64)> = with_descendants(members, &procs, &[me])
        .into_iter()
        .filter(|&(pid, start)| pid > 1 && live(pid, start))
        .collect();
    // A group led by a member goes in one signal (`kill(-pgid)`), never
    // Irori's own.
    let groups: HashSet<u32> = targets
        .iter()
        .filter(|&&(pid, _)| procs.get(&pid).is_some_and(|p| p.pgid == pid) && pid != my_group)
        .map(|&(pid, _)| pid)
        .collect();
    let signal_all = |sig: libc::c_int, targets: &[(u32, u64)]| {
        for &g in &groups {
            // SAFETY: a group led by a member verified live above.
            unsafe {
                libc::kill(-(g as libc::pid_t), sig);
            }
        }
        for &(pid, start) in targets {
            if live(pid, start) {
                send(pid, sig);
            }
        }
    };
    let wait = |targets: &mut Vec<(u32, u64)>, limit: Duration| {
        let deadline = Instant::now() + limit;
        loop {
            targets.retain(|&(pid, start)| live(pid, start));
            if targets.is_empty() || Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    signal_all(libc::SIGTERM, &targets);
    wait(&mut targets, TERM_GRACE);
    if !targets.is_empty() {
        signal_all(libc::SIGKILL, &targets);
        wait(&mut targets, Duration::from_secs(1));
    }
    if targets.is_empty() {
        Ok(())
    } else {
        let names: Vec<String> = targets
            .iter()
            .map(|(p, _)| format!("{p} (still running)"))
            .collect();
        Err(format!("couldn't end {}", names.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fdinfo_layout() {
        // vnode_fdinfowithpath = proc_fileinfo + vnode_info_path, where
        // vnode_info_path = vnode_info (152) + vip_path[MAXPATHLEN].
        assert_eq!(std::mem::size_of::<libc::vnode_info_path>(), 152 + 1024);
        assert_eq!(
            FDINFO_SIZE,
            24 + std::mem::size_of::<libc::vnode_info_path>()
        );
        assert_eq!(std::mem::size_of::<libc::proc_bsdinfo>(), 136);
    }

    #[test]
    fn own_start_matches_proc_start_time() {
        let me = bsdinfo(std::process::id()).expect("own bsdinfo");
        let t = crate::proc::start_time(i64::from(std::process::id())).expect("start_time");
        let diff = me.start.abs_diff(filetime_ticks(t));
        assert!(diff < 20_000, "ticks apart: {diff}"); // 2 ms
    }
}
