//! The macOS twin of awake.rs: a real hold is an IOPM
//! PreventUserIdleSystemSleep assertion for this process, as `pmset -g
//! assertions` lists it, and two holds are two assertions. Each test
//! releases what it takes.
#![cfg(target_os = "macos")]

/// Both tests count this process's assertions, so they take turns.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// This process's PreventUserIdleSystemSleep assertions.
fn held() -> usize {
    let out = std::process::Command::new("pmset")
        .args(["-g", "assertions"])
        .output()
        .expect("pmset");
    let me = format!("pid {}(", std::process::id());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(&me) && l.contains("PreventUserIdleSystemSleep"))
        .count()
}

#[test]
fn real_hold() {
    let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let before = held();
    let h = sys::awake::system().acquire().expect("acquire");
    let during = held();
    h.release();
    h.release(); // a second release does nothing
    assert_eq!(during, before + 1, "no assertion while held");
    assert_eq!(held(), before, "the assertion outlived its release");
}

#[test]
fn two_holds() {
    let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let before = held();
    let a = sys::awake::system().acquire().expect("a");
    let b = sys::awake::system().acquire().expect("b");
    a.release();
    let one = held();
    b.release();
    assert!(one > before, "releasing one hold dropped the other");
}
