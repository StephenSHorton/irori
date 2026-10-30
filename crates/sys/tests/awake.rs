//! Ports of I:internal/awake/awake_windows_test.go: a real hold sets
//! ES_SYSTEM_REQUIRED on the system, and two holds are two threads. The
//! hold is a power request only (no notification, no setting changed), and
//! each test releases what it takes.
#![cfg(windows)]

use windows_sys::Win32::System::Power::{CallNtPowerInformation, SystemExecutionState};

const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

/// The system's execution state: ES_SYSTEM_REQUIRED is set while any
/// thread on the PC holds it.
fn system_execution_state() -> u32 {
    let mut st: u32 = 0;
    // SAFETY: no input; the output buffer is a live u32 of the given size.
    let r = unsafe {
        CallNtPowerInformation(
            SystemExecutionState,
            std::ptr::null(),
            0,
            (&mut st as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    assert_eq!(r, 0, "CallNtPowerInformation: 0x{r:x}");
    st
}

#[test]
fn real_hold() {
    let h = sys::awake::system().acquire().expect("acquire");
    let held = system_execution_state();
    if held & ES_SYSTEM_REQUIRED == 0 {
        h.release();
        panic!("system execution state 0x{held:x} while held: no ES_SYSTEM_REQUIRED");
    }
    h.release();
    h.release(); // a second release does nothing
}

#[test]
fn two_holds() {
    let a = sys::awake::system().acquire().expect("a");
    let b = sys::awake::system().acquire().expect("b");
    a.release();
    let st = system_execution_state();
    if st & ES_SYSTEM_REQUIRED == 0 {
        b.release();
        panic!("releasing one hold dropped the other");
    }
    b.release();
}
