//! Keep this PC awake while the home server runs (I:internal/awake,
//! inbox.md §4, card #38). Only idle system sleep is held off: the display
//! may still turn off, and a lid close or the Start menu's Sleep still
//! sleeps the PC.
//!
//! - Windows: `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`.
//!   The state belongs to the calling thread, so each hold owns a dedicated
//!   `std::thread` that sets it, blocks until release, and clears it with
//!   `ES_CONTINUOUS` on that same thread (inbox.md §7.15: never a tokio task).
//! - macOS: an IOKit `PreventUserIdleSystemSleep` assertion named
//!   [`REASON`], released once.
//! - Elsewhere: a hold that does nothing.

use std::fmt;
use std::sync::Arc;

/// What macOS shows for the assertion (`pmset -g assertions`).
pub const REASON: &str = "Irori home server";

/// Why a hold could not be taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Acquires holds (Go `awake.Holder`).
pub trait Holder: Send + Sync {
    /// Holds off idle system sleep until the hold is released.
    fn acquire(&self) -> Result<Box<dyn Hold>, Error>;
}

/// One acquired hold (Go `awake.Hold`).
pub trait Hold: Send + Sync {
    /// Lets the PC sleep again; a second call does nothing.
    fn release(&self);
}

/// This OS's holder (Go `awake.System()`).
pub fn system() -> Arc<dyn Holder> {
    Arc::new(System)
}

/// This OS's holder as a type.
#[derive(Debug, Clone, Copy, Default)]
pub struct System;

impl Holder for System {
    fn acquire(&self) -> Result<Box<dyn Hold>, Error> {
        imp::acquire()
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::thread::JoinHandle;

    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };

    use super::{Error, Hold};

    struct WindowsHold {
        inner: Mutex<Option<(mpsc::Sender<()>, JoinHandle<()>)>>,
    }

    pub(super) fn acquire() -> Result<Box<dyn Hold>, Error> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (res_tx, res_rx) = mpsc::channel::<Result<(), Error>>();
        let th = std::thread::Builder::new()
            .name("irori-awake".into())
            .spawn(move || {
                // SAFETY: plain Win32 call with flag bits; no pointers.
                let r = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
                if r == 0 {
                    let mut e = std::io::Error::last_os_error();
                    if e.raw_os_error() == Some(0) {
                        // Go maps ERROR_SUCCESS to ERROR_INVALID_PARAMETER.
                        e = std::io::Error::from_raw_os_error(87);
                    }
                    let _ = res_tx.send(Err(Error(e.to_string())));
                    return;
                }
                let _ = res_tx.send(Ok(()));
                // Blocks until release (or the sender is dropped).
                let _ = stop_rx.recv();
                // SAFETY: as above.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            })
            .map_err(|e| Error(e.to_string()))?;
        match res_rx.recv() {
            Ok(Ok(())) => Ok(Box::new(WindowsHold {
                inner: Mutex::new(Some((stop_tx, th))),
            })),
            Ok(Err(e)) => {
                let _ = th.join();
                Err(e)
            }
            Err(_) => {
                let _ = th.join();
                Err(Error("the keep-awake thread ended".into()))
            }
        }
    }

    impl Hold for WindowsHold {
        fn release(&self) {
            let taken = self.inner.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some((stop, th)) = taken {
                let _ = stop.send(());
                let _ = th.join();
            }
        }
    }

    impl Drop for WindowsHold {
        fn drop(&mut self) {
            self.release();
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;
    use std::sync::Mutex;

    use super::{Error, Hold, REASON};

    type CFStringRef = *const c_void;
    type CFAllocatorRef = *const c_void;
    type IOPMAssertionID = u32;
    type IOReturn = i32;
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithBytes(
            alloc: CFAllocatorRef,
            bytes: *const u8,
            num_bytes: isize,
            encoding: u32,
            is_external: u8,
        ) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: CFStringRef,
            level: u32,
            name: CFStringRef,
            out_id: *mut IOPMAssertionID,
        ) -> IOReturn;
        fn IOPMAssertionRelease(id: IOPMAssertionID) -> IOReturn;
    }

    fn cfstr(s: &str) -> CFStringRef {
        // SAFETY: bytes and length come from a live &str.
        unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                s.as_ptr(),
                s.len() as isize,
                K_CF_STRING_ENCODING_UTF8,
                0,
            )
        }
    }

    struct DarwinHold(Mutex<Option<IOPMAssertionID>>);

    pub(super) fn acquire() -> Result<Box<dyn Hold>, Error> {
        let kind = cfstr("PreventUserIdleSystemSleep");
        let name = cfstr(REASON);
        let mut id: IOPMAssertionID = 0;
        // SAFETY: both CFStrings are live; id is a valid out pointer.
        let r =
            unsafe { IOPMAssertionCreateWithName(kind, K_IOPM_ASSERTION_LEVEL_ON, name, &mut id) };
        // SAFETY: created above, released once.
        unsafe {
            CFRelease(kind);
            CFRelease(name);
        }
        if r != 0 {
            return Err(Error(format!(
                "IOPMAssertionCreateWithName: 0x{:x}",
                r as u32
            )));
        }
        Ok(Box::new(DarwinHold(Mutex::new(Some(id)))))
    }

    impl Hold for DarwinHold {
        fn release(&self) {
            if let Some(id) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
                // SAFETY: an assertion id this hold created.
                unsafe { IOPMAssertionRelease(id) };
            }
        }
    }

    impl Drop for DarwinHold {
        fn drop(&mut self) {
            self.release();
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::{Error, Hold};

    struct NoHold;

    impl Hold for NoHold {
        fn release(&self) {}
    }

    pub(super) fn acquire() -> Result<Box<dyn Hold>, Error> {
        Ok(Box::new(NoHold))
    }
}
