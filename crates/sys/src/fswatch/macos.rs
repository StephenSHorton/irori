//! I:internal/fswatch/fswatch_darwin.{go,c}: one FSEvents stream for the
//! tree. FSEvents always watches the whole subtree; a non-recursive watch
//! drops events below root's direct children. Events dropped by the kernel
//! or fseventsd, or a moved root, arrive as Overflow, the same contract as
//! ReadDirectoryChangesW's.
//!
//! The stream runs on a CFRunLoop of a thread of its own, not on a
//! dispatch queue: callbacks run only inside that loop, so the context they
//! get is the thread's, and the thread ends with the watcher.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::coalesce::{coalesce, send};
use super::{Error, Event, QUEUE, Watcher};
use crate::cancel::Cancel;

type CFRef = *const c_void;

#[repr(C)]
struct StreamContext {
    version: isize,
    info: *mut c_void,
    retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<extern "C" fn(*const c_void)>,
    copy_description: Option<extern "C" fn(*const c_void) -> CFRef>,
}

type Callback = extern "C" fn(CFRef, *mut c_void, usize, *mut c_void, *const u32, *const u64);

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeArrayCallBacks: [usize; 5];
    static kCFRunLoopDefaultMode: CFRef;
    fn CFStringCreateWithCString(alloc: CFRef, s: *const c_char, encoding: u32) -> CFRef;
    fn CFArrayCreate(alloc: CFRef, values: *const CFRef, n: isize, cb: *const c_void) -> CFRef;
    fn CFRetain(r: CFRef) -> CFRef;
    fn CFRelease(r: CFRef);
    fn CFRunLoopGetCurrent() -> CFRef;
    fn CFRunLoopRunInMode(mode: CFRef, seconds: f64, return_after_source: u8) -> i32;
    fn CFRunLoopStop(rl: CFRef);
    fn CFRunLoopWakeUp(rl: CFRef);
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn FSEventStreamCreate(
        alloc: CFRef,
        cb: Callback,
        ctx: *mut StreamContext,
        paths: CFRef,
        since: u64,
        latency: f64,
        flags: u32,
    ) -> CFRef;
    fn FSEventStreamScheduleWithRunLoop(s: CFRef, rl: CFRef, mode: CFRef);
    fn FSEventStreamStart(s: CFRef) -> u8;
    fn FSEventStreamStop(s: CFRef);
    fn FSEventStreamInvalidate(s: CFRef);
    fn FSEventStreamRelease(s: CFRef);
}

const UTF8: u32 = 0x0800_0100;
const SINCE_NOW: u64 = u64::MAX;
const CREATE_NO_DEFER: u32 = 0x02;
const CREATE_WATCH_ROOT: u32 = 0x04;
const CREATE_FILE_EVENTS: u32 = 0x10;

// Event flags (FSEvents.h).
const MUST_SCAN_SUB_DIRS: u32 = 0x01;
const USER_DROPPED: u32 = 0x02;
const KERNEL_DROPPED: u32 = 0x04;
const HISTORY_DONE: u32 = 0x10;
const ROOT_CHANGED: u32 = 0x20;

/// How long FSEvents may hold events before calling back. With NoDefer the
/// first event of a quiet spell is delivered at once; coalesce does the
/// real debouncing.
const LATENCY: f64 = 0.01;
/// Bounds how late a stop signalled before the loop ran is noticed;
/// CFRunLoopStop ends a running wait at once.
const LOOP_SLICE: f64 = 0.25;

/// What the callback reaches through the stream's info pointer.
struct Stream {
    /// As the caller gave it, made absolute.
    root: String,
    /// root with symlinks resolved: what FSEvents reports.
    real_root: String,
    recurse: bool,
    raw: SyncSender<Event>,
    lost: Arc<AtomicBool>,
}

impl Stream {
    fn event(&self, p: &str, flags: u32) {
        if flags & (MUST_SCAN_SUB_DIRS | USER_DROPPED | KERNEL_DROPPED | ROOT_CHANGED) != 0 {
            let ov = Event {
                path: self.root.clone(),
                overflow: true,
            };
            send(&self.raw, ov, &self.lost);
            return;
        }
        if flags & HISTORY_DONE != 0 {
            return;
        }
        let Some(p) = self.caller_path(p) else {
            return;
        };
        if !self.recurse && p != self.root && Path::new(&p).parent() != Some(Path::new(&self.root))
        {
            return;
        }
        let ev = Event {
            path: p,
            overflow: false,
        };
        send(&self.raw, ev, &self.lost);
    }

    /// Maps a path FSEvents reports (symlinks resolved, so /private/var/...
    /// for /var/...) back under the root the caller asked for.
    fn caller_path(&self, p: &str) -> Option<String> {
        let p = p.trim_end_matches('/');
        let real = self.real_root.as_str();
        if p == real {
            return Some(self.root.clone());
        }
        let rest = match p.strip_prefix(real).and_then(|r| r.strip_prefix('/')) {
            Some(r) => r,
            // FSEvents can report a case variant of the root on APFS; match
            // it case-insensitively before giving up.
            None => match (p.get(..real.len()), p.get(real.len()..)) {
                (Some(head), Some(tail)) if head.eq_ignore_ascii_case(real) => {
                    tail.strip_prefix('/')?
                }
                _ => return None,
            },
        };
        Some(
            Path::new(&self.root)
                .join(rest)
                .to_string_lossy()
                .into_owned(),
        )
    }
}

extern "C" fn callback(
    _: CFRef,
    info: *mut c_void,
    n: usize,
    paths: *mut c_void,
    flags: *const u32,
    _: *const u64,
) {
    // SAFETY: info is the Stream the loop thread owns while the stream
    // lives; without UseCFTypes paths is an array of n C strings, and flags
    // has n entries.
    unsafe {
        let s = &*(info as *const Stream);
        let paths = std::slice::from_raw_parts(paths as *const *const c_char, n);
        let flags = std::slice::from_raw_parts(flags, n);
        for (p, f) in paths.iter().zip(flags) {
            s.event(&CStr::from_ptr(*p).to_string_lossy(), *f);
        }
    }
}

/// A CFRunLoop moved to the stopper.
struct RunLoop(CFRef);
// SAFETY: CFRunLoopStop and CFRunLoopWakeUp may be called from any thread.
unsafe impl Send for RunLoop {}

/// The loop thread's run loop, held (retained) while it runs.
struct Shared {
    stop: AtomicBool,
    run_loop: Mutex<Option<RunLoop>>,
}

impl Shared {
    fn signal(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(rl) = &*self.run_loop.lock().unwrap_or_else(|p| p.into_inner()) {
            // SAFETY: the run loop is retained while it is in the slot.
            unsafe {
                CFRunLoopStop(rl.0);
                CFRunLoopWakeUp(rl.0);
            }
        }
    }
}

/// Creates, schedules and starts the stream on this thread's run loop.
fn start(real_root: &str, info: *mut c_void) -> Result<CFRef, String> {
    let croot = CString::new(real_root).map_err(|e| e.to_string())?;
    // SAFETY: CF objects created here are released on every path; the
    // context is copied by FSEventStreamCreate.
    unsafe {
        let path = CFStringCreateWithCString(std::ptr::null(), croot.as_ptr(), UTF8);
        if path.is_null() {
            return Err(format!("fswatch: bad path {real_root}"));
        }
        let paths = CFArrayCreate(
            std::ptr::null(),
            &path,
            1,
            (&raw const kCFTypeArrayCallBacks).cast(),
        );
        CFRelease(path);
        if paths.is_null() {
            return Err(format!("fswatch: CFArrayCreate failed for {real_root}"));
        }
        let mut ctx = StreamContext {
            version: 0,
            info,
            retain: None,
            release: None,
            copy_description: None,
        };
        let s = FSEventStreamCreate(
            std::ptr::null(),
            callback,
            &mut ctx,
            paths,
            SINCE_NOW,
            LATENCY,
            CREATE_FILE_EVENTS | CREATE_NO_DEFER | CREATE_WATCH_ROOT,
        );
        CFRelease(paths);
        if s.is_null() {
            return Err(format!(
                "fswatch: FSEventStreamCreate failed for {real_root}"
            ));
        }
        FSEventStreamScheduleWithRunLoop(s, CFRunLoopGetCurrent(), kCFRunLoopDefaultMode);
        if FSEventStreamStart(s) == 0 {
            FSEventStreamInvalidate(s);
            FSEventStreamRelease(s);
            return Err(format!(
                "fswatch: FSEventStreamStart failed for {real_root}"
            ));
        }
        Ok(s)
    }
}

pub(super) fn watch(
    cancel: &Cancel,
    root: &str,
    recursive: bool,
    debounce: Duration,
) -> Result<Watcher, Error> {
    let root = std::path::absolute(root).map_err(Error::Io)?;
    let real_root = std::fs::canonicalize(&root).map_err(Error::Io)?;
    let root = root.to_string_lossy().into_owned();
    let real_root = real_root.to_string_lossy().into_owned();

    let (raw_tx, raw_rx) = sync_channel::<Event>(QUEUE);
    let (out_tx, out_rx) = sync_channel::<Event>(QUEUE);
    let lost = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        run_loop: Mutex::new(None),
    });

    let (started_tx, started_rx) = sync_channel::<Result<(), String>>(1);
    let stream = Box::new(Stream {
        root: root.clone(),
        real_root: real_root.clone(),
        recurse: recursive,
        raw: raw_tx,
        lost: lost.clone(),
    });
    let looper = {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("fswatch-fsevents".into())
            .spawn(move || {
                let info = (&*stream as *const Stream).cast_mut().cast::<c_void>();
                let s = match start(&real_root, info) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = started_tx.send(Err(e));
                        return;
                    }
                };
                // SAFETY: this thread's run loop, retained while the
                // stopper may reach it.
                let rl = unsafe { CFRetain(CFRunLoopGetCurrent()) };
                *shared.run_loop.lock().unwrap_or_else(|p| p.into_inner()) = Some(RunLoop(rl));
                let _ = started_tx.send(Ok(()));
                while !shared.stop.load(Ordering::SeqCst) {
                    // SAFETY: runs this thread's loop; callbacks run in here.
                    unsafe {
                        CFRunLoopRunInMode(kCFRunLoopDefaultMode, LOOP_SLICE, 0);
                    }
                }
                // SAFETY: the stream is ours; no callback runs after it is
                // invalidated, so the Stream may go after it.
                unsafe {
                    FSEventStreamStop(s);
                    FSEventStreamInvalidate(s);
                    FSEventStreamRelease(s);
                }
                if let Some(rl) = shared
                    .run_loop
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take()
                {
                    // SAFETY: retained above.
                    unsafe { CFRelease(rl.0) };
                }
                drop(stream); // closes raw, which ends the coalescer
            })
            .map_err(Error::Io)?
    };
    match started_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = looper.join();
            return Err(Error::Io(std::io::Error::other(e)));
        }
        Err(_) => {
            let _ = looper.join();
            return Err(Error::Io(std::io::Error::other(
                "fswatch: the FSEvents thread died",
            )));
        }
    }
    let coalescer = std::thread::Builder::new()
        .name("fswatch-coalesce".into())
        .spawn(move || coalesce(raw_rx, out_tx, debounce, root, lost))
        .map_err(|e| {
            shared.signal();
            Error::Io(e)
        })?;

    let signal = {
        let shared = shared.clone();
        move || shared.signal()
    };
    let reg = cancel.on_cancel(signal.clone());
    let threads = Mutex::new(Some((looper, coalescer)));
    let stop = Box::new(move || {
        signal();
        if let Some((l, c)) = threads.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = l.join();
            let _ = c.join();
        }
        drop(reg);
    });
    Ok(Watcher::new(out_rx, stop))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(root: &str, real: &str, recurse: bool) -> (Stream, std::sync::mpsc::Receiver<Event>) {
        let (tx, rx) = sync_channel(16);
        let s = Stream {
            root: root.into(),
            real_root: real.into(),
            recurse,
            raw: tx,
            lost: Arc::new(AtomicBool::new(false)),
        };
        (s, rx)
    }

    #[test]
    fn caller_path_maps_the_resolved_root_back() {
        let (s, _) = stream("/var/x", "/private/var/x", true);
        assert_eq!(s.caller_path("/private/var/x").as_deref(), Some("/var/x"));
        assert_eq!(
            s.caller_path("/private/var/x/a/b.jsonl").as_deref(),
            Some("/var/x/a/b.jsonl")
        );
        assert_eq!(
            s.caller_path("/Private/Var/X/a").as_deref(),
            Some("/var/x/a")
        );
        assert_eq!(s.caller_path("/private/var/xy/a"), None);
        assert_eq!(s.caller_path("/elsewhere"), None);
    }

    #[test]
    fn flags_and_depth() {
        let (s, rx) = stream("/r", "/r", false);
        s.event("/r/a", 0);
        s.event("/r/a/b", 0); // below a direct child: dropped
        s.event("/r", HISTORY_DONE);
        s.event("/r/x", KERNEL_DROPPED);
        let got: Vec<Event> = rx.try_iter().collect();
        assert_eq!(
            got,
            vec![
                Event {
                    path: "/r/a".into(),
                    overflow: false
                },
                Event {
                    path: "/r".into(),
                    overflow: true
                },
            ]
        );
    }
}
