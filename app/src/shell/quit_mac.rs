//! Quitting on macOS without the shutdown deadlock (card #588).
//!
//! The shell's shutdown waits on the one tokio runtime (the API server's
//! task, the Core's loops). That runtime also answers the page's IPC
//! calls, and on macOS each answer waits for the main thread: WebKit's
//! `-[WKURLSchemeTask didReceiveData:]` off the main thread hops to it and
//! blocks until it runs. So the shutdown must never run on a main thread
//! that has stopped serving its run loop, as it did inside
//! `applicationWillTerminate:` and tao's `LoopDestroyed`: with a few calls
//! in flight at quit, every worker sat in WebKit waiting for the main
//! thread while the main thread waited for a worker, and Irori never
//! exited.
//!
//! So both ways out run the shutdown on its own thread while the loop
//! keeps going, and quit once it is done:
//! - `-[NSApplication terminate:]` (an Apple Event, Command-Q, the app
//!   menu): `applicationShouldTerminate:` starts it and answers
//!   `NSTerminateLater`; AppKit keeps the run loop going until the reply.
//! - `AppHandle::exit` (the tray's Quit, the updater, a reset, the probe):
//!   the shell holds the exit request ([`begin`]) and exits again after.
//!
//! `RunEvent::Exit` still calls the shutdown, which by then is done.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::sel;

type Shutdown = Box<dyn Fn() + Send + Sync>;

static SHUTDOWN: OnceLock<Shutdown> = OnceLock::new();
static STATE: AtomicU8 = AtomicU8::new(IDLE);

const IDLE: u8 = 0;
const RUNNING: u8 = 1;
const DONE: u8 = 2;

/// `NSApplicationTerminateReply`.
const TERMINATE_CANCEL: usize = 0;
const TERMINATE_NOW: usize = 1;
const TERMINATE_LATER: usize = 2;

/// What [`begin`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Begin {
    /// The shutdown started on its own thread; `then` runs after it.
    Started,
    /// One is already running; `then` was dropped.
    Running,
    /// It is done (or there is none, or no thread could start): quit now.
    Done,
}

/// Sets the shutdown and teaches the app delegate
/// `applicationShouldTerminate:`. Main thread, once the event loop is
/// built (tao sets the delegate then).
pub fn install(shutdown: impl Fn() + Send + Sync + 'static) -> Result<(), String> {
    if SHUTDOWN.set(Box::new(shutdown)).is_err() {
        return Err("quit: the shutdown is already set".into());
    }
    let class = AnyClass::get(c"NSApplication").ok_or("quit: no NSApplication")?;
    // SAFETY: AppKit's documented selectors on the main thread; the method
    // added has the type its encoding names (NSUInteger, self, _cmd, id).
    unsafe {
        let app: *mut AnyObject = msg_send![class, sharedApplication];
        let delegate: *mut AnyObject = msg_send![app, delegate];
        if delegate.is_null() {
            return Err("quit: the app has no delegate".into());
        }
        let cls = objc2::ffi::object_getClass(delegate) as *mut AnyClass;
        let imp: Imp = std::mem::transmute::<
            extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize,
            Imp,
        >(should_terminate);
        let added = objc2::ffi::class_addMethod(
            cls,
            sel!(applicationShouldTerminate:),
            imp,
            c"Q@:@".as_ptr(),
        );
        if !added.as_bool() {
            return Err(
                "quit: the app delegate already answers applicationShouldTerminate:".into(),
            );
        }
    }
    Ok(())
}

/// Starts the shutdown on its own thread, then runs `then` on that thread.
pub fn begin(then: impl FnOnce() + Send + 'static) -> Begin {
    let Some(shutdown) = SHUTDOWN.get() else {
        return Begin::Done;
    };
    match STATE.compare_exchange(IDLE, RUNNING, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(_) => {}
        Err(DONE) => return Begin::Done,
        Err(_) => return Begin::Running,
    }
    let spawned = std::thread::Builder::new()
        .name("irori-quit".into())
        .spawn(move || {
            shutdown();
            STATE.store(DONE, Ordering::SeqCst);
            then();
        });
    match spawned {
        Ok(_) => Begin::Started,
        Err(e) => {
            // The quit goes on, and RunEvent::Exit shuts down inline.
            irori_core::log(&format!("irori: quit: no shutdown thread: {e}"));
            STATE.store(IDLE, Ordering::SeqCst);
            Begin::Done
        }
    }
}

/// `-[NSApplicationDelegate applicationShouldTerminate:]`.
extern "C-unwind" fn should_terminate(_: *mut AnyObject, _: Sel, _: *mut AnyObject) -> usize {
    match begin(reply_on_main) {
        Begin::Started => TERMINATE_LATER,
        // Another quit is shutting down and ends the app itself.
        Begin::Running => TERMINATE_CANCEL,
        Begin::Done => TERMINATE_NOW,
    }
}

/// Answers the pending `NSTerminateLater` on the main thread. The main
/// queue runs in the modal mode AppKit waits in.
fn reply_on_main() {
    // SAFETY: libdispatch's main queue and a plain function with no context.
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, std::ptr::null_mut(), reply) };
}

extern "C" fn reply(_: *mut c_void) {
    let Some(class) = AnyClass::get(c"NSApplication") else {
        return;
    };
    // SAFETY: AppKit's documented selector, on the main thread (the main
    // queue).
    unsafe {
        let app: *mut AnyObject = msg_send![class, sharedApplication];
        let () = msg_send![app, replyToApplicationShouldTerminate: Bool::YES];
    }
}

#[repr(C)]
struct DispatchQueue {
    _private: [u8; 0],
}

unsafe extern "C" {
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_async_f(
        queue: *const DispatchQueue,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}
