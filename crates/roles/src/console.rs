//! The parent's console for the command-line roles (Go console.go,
//! console_windows.go; surface.md 4, 7.15).
//!
//! A release exe is GUI-subsystem, so one typed at a prompt starts with no
//! stdout or stderr at all: `irori version` would print nothing. The
//! command-line roles attach to the parent's console instead, but only for
//! the streams that lead nowhere, so a pipe or a file the caller set up
//! (the mod's `irori attach`, `irori version | more`) is left alone.

/// Go `fixStdio`: which of stdout and stderr to replace with the parent's
/// console. `out_ok` and `err_ok` say whether the inherited handles lead
/// somewhere; `attach` joins the parent's console and reports success.
/// Returns `(replace_out, replace_err)`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn fix_stdio(out_ok: bool, err_ok: bool, attach: impl FnOnce() -> bool) -> (bool, bool) {
    if out_ok && err_ok {
        return (false, false);
    }
    if !attach() {
        // Started without a console (Explorer, a service): nothing to join.
        return (false, false);
    }
    (!out_ok, !err_ok)
}

/// Go `useParentConsole`: gives a command-line role the console it was
/// typed in. `pair` also reads the console it attached to.
#[cfg(windows)]
pub fn use_parent_console(cmd: &str) {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_UNKNOWN, GetFileType,
        OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE,
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };

    if !crate::cli::wants_console(cmd) {
        return;
    }
    let usable = |which: STD_HANDLE| -> bool {
        // SAFETY: GetStdHandle and GetFileType only read the handle value.
        unsafe {
            let h = GetStdHandle(which);
            !h.is_null() && h != INVALID_HANDLE_VALUE && GetFileType(h) != FILE_TYPE_UNKNOWN
        }
    };
    let open = |name: &str, access: u32| {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: a NUL-terminated name; the handle is kept for the
        // process's life as its standard stream.
        let h = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        (h != INVALID_HANDLE_VALUE).then_some(h)
    };
    let (out, err) = fix_stdio(usable(STD_OUTPUT_HANDLE), usable(STD_ERROR_HANDLE), || {
        // SAFETY: plain Win32 call.
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
    });
    if (out || err)
        && let Some(h) = open("CONOUT$", GENERIC_READ | GENERIC_WRITE)
    {
        // Rust's stdout and stderr ask GetStdHandle on every write, so
        // replacing the process handles is enough.
        // SAFETY: h is a valid console handle.
        unsafe {
            if out {
                SetStdHandle(STD_OUTPUT_HANDLE, h);
            }
            if err {
                SetStdHandle(STD_ERROR_HANDLE, h);
            }
        }
    }
    if cmd == "pair"
        && !usable(STD_INPUT_HANDLE)
        && let Some(h) = open("CONIN$", GENERIC_READ | GENERIC_WRITE)
    {
        // SAFETY: h is a valid console handle.
        unsafe {
            SetStdHandle(STD_INPUT_HANDLE, h);
        }
    }
}

#[cfg(not(windows))]
pub fn use_parent_console(_cmd: &str) {}

#[cfg(test)]
mod tests {
    use super::fix_stdio;

    // Go TestFixStdio's cases.
    #[test]
    fn fix_stdio_like_go() {
        let mut called = false;
        assert_eq!(
            fix_stdio(true, true, || {
                called = true;
                true
            }),
            (false, false)
        );
        assert!(!called, "both streams fine: no attach");
        assert_eq!(fix_stdio(false, false, || false), (false, false));
        assert_eq!(fix_stdio(false, false, || true), (true, true));
        assert_eq!(fix_stdio(true, false, || true), (false, true));
        assert_eq!(fix_stdio(false, true, || true), (true, false));
    }
}
