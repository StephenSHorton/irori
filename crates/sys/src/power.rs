//! Hears the OS put this device to sleep and wake it (card #580,
//! no-server.md §4.3 "Leave nothing stranded"), so Irori can send its last
//! rows first and reconnect at once after.
//!
//! - macOS: `IORegisterForSystemPower` on a thread of its own running a
//!   CFRunLoop. `kIOMessageSystemWillSleep` runs the handler, then
//!   `IOAllowPowerChange` (the OS waits up to 30 s for it);
//!   `kIOMessageCanSystemSleep` is allowed at once, never vetoed.
//! - Windows: `PowerRegisterSuspendResumeNotification` with a callback, no
//!   window needed. `PBT_APMSUSPEND` runs the handler on the OS's thread
//!   (it allows about 2 s); a resume says woke.
//! - Elsewhere: nothing to hear.
//!
//! A watch lasts as long as the process; there is no unwatching.

use std::sync::Arc;

/// What the OS is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// About to sleep: the handler may block a little; sleep waits for it.
    WillSleep,
    /// Back from sleep.
    Woke,
}

/// Called on each [`Change`], on a thread of the OS's or the watch's own.
pub type Handler = Arc<dyn Fn(Change) + Send + Sync>;

/// Starts hearing sleep and wake for the rest of the process.
pub fn watch(on: Handler) -> Result<(), String> {
    imp::watch(on)
}

/// Runs `on` without letting a panic cross into the OS's frame.
#[cfg(any(target_os = "macos", windows))]
fn call(on: &Handler, c: Change) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| on(c)));
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;

    use super::{Change, Handler, call};

    type IoConnect = u32;
    type IoObject = u32;
    type NotifyPort = *mut c_void;
    type CFRef = *const c_void;
    type Callback = extern "C" fn(*mut c_void, IoObject, u32, *mut c_void);

    // iokit_common_msg(m): sys_iokit (0x38 << 26) | m (IOMessage.h).
    const CAN_SYSTEM_SLEEP: u32 = 0xE000_0270;
    const SYSTEM_WILL_SLEEP: u32 = 0xE000_0280;
    const SYSTEM_HAS_POWERED_ON: u32 = 0xE000_0300;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IORegisterForSystemPower(
            refcon: *mut c_void,
            port: *mut NotifyPort,
            callback: Callback,
            notifier: *mut IoObject,
        ) -> IoConnect;
        fn IONotificationPortGetRunLoopSource(port: NotifyPort) -> CFRef;
        fn IOAllowPowerChange(kernel_port: IoConnect, notification_id: isize) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRunLoopGetCurrent() -> CFRef;
        fn CFRunLoopAddSource(rl: CFRef, source: CFRef, mode: CFRef);
        fn CFRunLoopRun();
        static kCFRunLoopDefaultMode: CFRef;
    }

    struct Ctx {
        root: AtomicU32,
        on: Handler,
    }

    extern "C" fn heard(refcon: *mut c_void, _svc: IoObject, msg: u32, arg: *mut c_void) {
        // SAFETY: refcon is the leaked Ctx from watch, alive for the process.
        let ctx = unsafe { &*(refcon as *const Ctx) };
        let allow = || {
            // SAFETY: the root port IORegisterForSystemPower returned, and
            // the message's own id.
            unsafe { IOAllowPowerChange(ctx.root.load(Ordering::SeqCst), arg as isize) };
        };
        match msg {
            CAN_SYSTEM_SLEEP => allow(),
            SYSTEM_WILL_SLEEP => {
                call(&ctx.on, Change::WillSleep);
                allow();
            }
            SYSTEM_HAS_POWERED_ON => call(&ctx.on, Change::Woke),
            _ => {}
        }
    }

    pub(super) fn watch(on: Handler) -> Result<(), String> {
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        std::thread::Builder::new()
            .name("irori-power".into())
            .spawn(move || {
                let ctx: &'static Ctx = Box::leak(Box::new(Ctx {
                    root: AtomicU32::new(0),
                    on,
                }));
                let mut port: NotifyPort = std::ptr::null_mut();
                let mut notifier: IoObject = 0;
                // SAFETY: valid out pointers; ctx lives for the process.
                let root = unsafe {
                    IORegisterForSystemPower(
                        ctx as *const Ctx as *mut c_void,
                        &mut port,
                        heard,
                        &mut notifier,
                    )
                };
                if root == 0 || port.is_null() {
                    let _ = tx.send(Err("IORegisterForSystemPower failed".into()));
                    return;
                }
                ctx.root.store(root, Ordering::SeqCst);
                // SAFETY: the port just made; this thread's own run loop.
                unsafe {
                    let src = IONotificationPortGetRunLoopSource(port);
                    CFRunLoopAddSource(CFRunLoopGetCurrent(), src, kCFRunLoopDefaultMode);
                }
                let _ = tx.send(Ok(()));
                // SAFETY: runs this thread's loop for the rest of the process.
                unsafe { CFRunLoopRun() };
            })
            .map_err(|e| e.to_string())?;
        rx.recv()
            .unwrap_or_else(|_| Err("the power thread ended".into()))
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    use windows_sys::Win32::System::Power::{
        DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerRegisterSuspendResumeNotification,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
    };

    use super::{Change, Handler, call};

    unsafe extern "system" fn heard(ctx: *const c_void, kind: u32, _setting: *const c_void) -> u32 {
        // SAFETY: ctx is the leaked Handler from watch, alive for the process.
        let on = unsafe { &*(ctx as *const Handler) };
        match kind {
            PBT_APMSUSPEND => call(on, Change::WillSleep),
            PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => call(on, Change::Woke),
            _ => {}
        }
        0
    }

    pub(super) fn watch(on: Handler) -> Result<(), String> {
        let on: &'static Handler = Box::leak(Box::new(on));
        let params: &'static mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS =
            Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
                Callback: Some(heard),
                Context: on as *const Handler as *mut c_void,
            }));
        let mut reg: *mut c_void = std::ptr::null_mut();
        // SAFETY: params lives for the process, as the registration does.
        let r = unsafe {
            PowerRegisterSuspendResumeNotification(
                DEVICE_NOTIFY_CALLBACK,
                params as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS as *mut c_void,
                &mut reg,
            )
        };
        if r != 0 {
            return Err(format!("PowerRegisterSuspendResumeNotification: {r}"));
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use super::Handler;

    pub(super) fn watch(_on: Handler) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_watch_starts() {
        watch(Arc::new(|_| {})).unwrap();
    }
}
