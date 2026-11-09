//! Card #1249 L3: a run's tree found from its `.output` file, sampled and
//! ended (live-tasks.md §4, §5.3).

use sys::runmeter::{self, RunKey, TreeSampler};

/// A file nobody holds has no tree, on every OS.
#[test]
fn unheld_file_has_no_tree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bnone.output");
    std::fs::write(&path, "done\n\n[exited with code 0]\n").unwrap();
    assert!(runmeter::holders(&path).is_empty());
    let mut s = TreeSampler::new();
    let key = RunKey {
        output: path.clone(),
        session_pid: 0,
    };
    let got = s.sample(std::slice::from_ref(&key));
    if runmeter::SUPPORTED {
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, path);
        assert!(got[0].1.members.is_empty() && got[0].1.procs == 0);
    }
    assert!(runmeter::end_tree(&[]).is_ok());
}

#[cfg(windows)]
mod win {
    use std::fs::File;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use sys::proc;
    use sys::runmeter::{self, RunKey, TreeSampler};

    /// Git for Windows' own `sh` (not WSL's `bash`, whose processes have
    /// no Windows pids).
    fn git_sh() -> Option<PathBuf> {
        let out = Command::new("git").arg("--exec-path").output().ok()?;
        let exec = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let root = Path::new(&exec).ancestors().nth(3)?.to_path_buf();
        let sh = root.join("usr").join("bin").join("sh.exe");
        sh.is_file().then_some(sh)
    }

    fn name_of(pid: u32) -> String {
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        text.split(',')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_ascii_lowercase()
    }

    /// `pid` is running (not merely a handle someone keeps) and is the
    /// process that started at `start`.
    fn same(pid: u32, start: u64) -> bool {
        proc::alive(i64::from(pid))
            && proc::start_time(i64::from(pid)).map(proc::filetime_ticks) == Some(start)
    }

    /// Ends whatever the test started even when an assert fails, before
    /// the temp folder goes (fields drop in order: TempDir last).
    struct Run {
        child: Child,
        path: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl Drop for Run {
        fn drop(&mut self) {
            let _ = runmeter::end_tree(&runmeter::holders(&self.path));
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Git Bash's `sh -c` writing to a `.output` file, as Claude Code's
    /// Bash tool runs a background command: a backgrounded `sleep.exe`
    /// (an MSYS fork, whose ppid points at a dead intermediate) and a busy
    /// loop in the shell itself.
    fn start(script: &str) -> Option<Run> {
        let sh = git_sh()?;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("btest1249.output");
        let f = File::create(&path).unwrap();
        let child = Command::new(sh)
            .args(["-c", script])
            .stdin(Stdio::null())
            .stdout(f.try_clone().unwrap())
            .stderr(f)
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap();
        Some(Run {
            child,
            path,
            _dir: dir,
        })
    }

    #[test]
    fn git_bash_tree_is_held_sampled_and_ended() {
        let Some(run) =
            start("sleep 60 & i=0; while [ $i -lt 100000000 ]; do i=$((i+1)); done; wait")
        else {
            eprintln!("no Git sh here; skipped");
            return;
        };

        // The holders include sleep.exe, found from the file alone.
        let deadline = Instant::now() + Duration::from_secs(15);
        let held = loop {
            let h = runmeter::holders(&run.path);
            if h.iter().any(|&(pid, _)| name_of(pid) == "sleep.exe") {
                break h;
            }
            assert!(Instant::now() < deadline, "no sleep.exe among {h:?}");
            std::thread::sleep(Duration::from_millis(200));
        };
        assert!(held.iter().all(|&(pid, s)| same(pid, s)), "{held:?}");

        // Our own handle on the file never makes us a holder.
        let _mine = File::open(&run.path).unwrap();
        let me = std::process::id();
        assert!(runmeter::holders(&run.path).iter().all(|&(p, _)| p != me));

        // Samples: the first primes CPU, the second reads the busy loop.
        let mut s = TreeSampler::new();
        let key = RunKey {
            output: run.path.clone(),
            session_pid: me,
        };
        let first = s.sample(std::slice::from_ref(&key));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].1.cores, None);
        std::thread::sleep(Duration::from_millis(1500));
        let second = s.sample(std::slice::from_ref(&key)).remove(0).1;
        eprintln!("sample: {second:?}");
        for h in &held {
            assert!(second.members.contains(h), "{h:?} not in {second:?}");
        }
        assert_eq!(second.procs as usize, second.members.len());
        assert!(second.mem_bytes > 0, "{second:?}");
        let cores = second.cores.expect("cores on the second sample");
        assert!(cores > 0.2 && cores < 64.0, "cores {cores}");
        assert!(!second.members.iter().any(|&(p, _)| p == me));

        // Forgotten when no longer passed.
        assert!(s.sample(&[]).is_empty());

        // Stop: nothing of the tree is left.
        let members = second.members.clone();
        runmeter::end_tree(&members).expect("end_tree");
        for &(pid, start) in &members {
            assert!(!same(pid, start), "{pid} ({}) still running", name_of(pid));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while !runmeter::holders(&run.path).is_empty() {
            assert!(Instant::now() < deadline, "file still held");
            std::thread::sleep(Duration::from_millis(100));
        }
        drop(_mine);
    }

    /// A stale `(pid, start)` is never ended: here the pid is our own with
    /// a wrong start, and our own process is never ended anyway.
    #[test]
    fn end_tree_skips_stale_and_self() {
        let me = std::process::id();
        let st = proc::filetime_ticks(proc::start_time(i64::from(me)).unwrap());
        assert!(runmeter::end_tree(&[(me, st), (me, st + 1)]).is_ok());
        assert!(proc::alive(i64::from(me)));
    }

    /// A member whose pid now belongs to another process (its start no
    /// longer matches) is not ended, and neither are that process's
    /// children: they aren't the run's, even though they started after the
    /// stale start. Here `cmd` waits on a native `ping` child; if `ping`
    /// were ended, `cmd` would exit.
    #[test]
    fn end_tree_leaves_children_of_a_reused_pid() {
        struct Kill(Child);
        impl Drop for Kill {
            fn drop(&mut self) {
                let _ = Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &self.0.id().to_string()])
                    .creation_flags(0x0800_0000)
                    .output();
                let _ = self.0.wait();
            }
        }
        let mut cmd = Kill(
            Command::new("cmd")
                .args(["/c", "ping -n 30 127.0.0.1 >nul"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(0x0800_0000)
                .spawn()
                .unwrap(),
        );
        let pid = cmd.0.id();
        // Give cmd a moment to start ping.
        std::thread::sleep(Duration::from_millis(1500));
        assert!(cmd.0.try_wait().unwrap().is_none(), "cmd ended early");
        // A start earlier than cmd's real one: the pid was "reused".
        assert!(runmeter::end_tree(&[(pid, 1)]).is_ok());
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            cmd.0.try_wait().unwrap().is_none(),
            "cmd exited: its child was ended"
        );
    }
}
