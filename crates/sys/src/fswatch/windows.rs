//! I:internal/fswatch/fswatch_windows.go: one overlapped
//! `ReadDirectoryChangesW` handle, a 64 KiB buffer, the first read armed
//! before [`watch`] returns.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_NOTIFY_ENUM_DIR, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    ReadDirectoryChangesW,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects,
};

use super::coalesce::{coalesce, send};
use super::{Error, Event, QUEUE, Watcher};
use crate::cancel::Cancel;

/// The largest buffer ReadDirectoryChangesW takes over a network share.
const BUF_SIZE: usize = 64 << 10;

const NOTIFY_MASK: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
    | FILE_NOTIFY_CHANGE_DIR_NAME
    | FILE_NOTIFY_CHANGE_SIZE
    | FILE_NOTIFY_CHANGE_LAST_WRITE;

#[derive(Clone, Copy)]
struct H(HANDLE);
// SAFETY: kernel handles may be used from any thread.
unsafe impl Send for H {}

/// What the reader thread owns. The buffer and OVERLAPPED are boxed so they
/// stay put while a read is pending.
struct Reader {
    dir: H,
    io: H,
    buf: Box<[u8]>,
    ov: Box<OVERLAPPED>,
    recursive: bool,
}

// SAFETY: the OVERLAPPED's event pointer is a kernel handle, usable from the
// one thread that owns the Reader after it is moved there.
unsafe impl Send for Reader {}

impl Reader {
    /// Issues one read. Changes are only buffered on the handle once a read
    /// is pending, so the first one is armed before `watch` returns.
    fn arm(&mut self) -> Result<(), u32> {
        // SAFETY: zeroing a plain C struct; then a read into buffers that
        // outlive it (they are freed only after the read completes or is
        // canceled and drained).
        unsafe {
            *self.ov = std::mem::zeroed();
            self.ov.hEvent = self.io.0;
            ResetEvent(self.io.0);
            let mut n = 0u32;
            let ok = ReadDirectoryChangesW(
                self.dir.0,
                self.buf.as_mut_ptr().cast(),
                self.buf.len() as u32,
                i32::from(self.recursive),
                NOTIFY_MASK,
                &mut n,
                &mut *self.ov,
                None,
            );
            if ok == 0 {
                let e = GetLastError();
                if e != ERROR_IO_PENDING {
                    return Err(e);
                }
            }
        }
        Ok(())
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn close(h: H) {
    // SAFETY: each handle is closed once, by its owner.
    unsafe {
        CloseHandle(h.0);
    }
}

fn last_err() -> Error {
    Error::Io(std::io::Error::last_os_error())
}

pub(super) fn watch(
    cancel: &Cancel,
    root: &str,
    recursive: bool,
    debounce: Duration,
) -> Result<Watcher, Error> {
    let root = std::path::absolute(root)
        .map_err(Error::Io)?
        .to_string_lossy()
        .into_owned();
    let p = wide(&root);
    // SAFETY: a NUL-terminated path; no security attributes or template.
    let dir = unsafe {
        CreateFileW(
            p.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        )
    };
    if dir == INVALID_HANDLE_VALUE || dir.is_null() {
        return Err(last_err());
    }
    let dir = H(dir);
    // SAFETY: unnamed manual-reset events.
    let io = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
    if io.is_null() {
        let e = last_err();
        close(dir);
        return Err(e);
    }
    let io = H(io);
    // SAFETY: as above.
    let stop_ev = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
    if stop_ev.is_null() {
        let e = last_err();
        close(io);
        close(dir);
        return Err(e);
    }
    let stop_ev = H(stop_ev);
    let mut reader = Reader {
        dir,
        io,
        buf: vec![0u8; BUF_SIZE].into_boxed_slice(),
        // SAFETY: OVERLAPPED is a plain C struct; zero is its initial state.
        ov: Box::new(unsafe { std::mem::zeroed() }),
        recursive,
    };
    if let Err(code) = reader.arm() {
        close(stop_ev);
        close(io);
        close(dir);
        return Err(Error::Io(std::io::Error::from_raw_os_error(code as i32)));
    }

    // The stop event is signalled only while it is open (Go's sync.Once
    // around SetEvent, used up before the handle closes).
    let stop_slot: Arc<Mutex<Option<H>>> = Arc::new(Mutex::new(Some(stop_ev)));
    let signal = {
        let slot = stop_slot.clone();
        move || {
            if let Some(h) = *slot.lock().unwrap_or_else(|p| p.into_inner()) {
                // SAFETY: the event is open while it is in the slot.
                unsafe {
                    SetEvent(h.0);
                }
            }
        }
    };

    let (raw_tx, raw_rx) = sync_channel::<Event>(QUEUE);
    let (out_tx, out_rx) = sync_channel::<Event>(QUEUE);
    let raw_lost = Arc::new(AtomicBool::new(false));
    let coalescer = {
        let lost = raw_lost.clone();
        let root = root.clone();
        std::thread::Builder::new()
            .name("fswatch-coalesce".into())
            .spawn(move || coalesce(raw_rx, out_tx, debounce, root, lost))
            .map_err(Error::Io)?
    };
    let reader_thread = {
        let lost = raw_lost;
        let root = root.clone();
        let slot = stop_slot.clone();
        std::thread::Builder::new()
            .name("fswatch-rdcw".into())
            .spawn(move || {
                let mut r = reader;
                loop {
                    let hs = [r.io.0, stop_ev.0];
                    // SAFETY: two open handles.
                    let w = unsafe { WaitForMultipleObjects(2, hs.as_ptr(), 0, INFINITE) };
                    let mut n = 0u32;
                    if w != WAIT_OBJECT_0 {
                        // SAFETY: cancel the pending read and wait for it to
                        // finish, so the buffers are no longer written.
                        unsafe {
                            CancelIoEx(r.dir.0, &*r.ov);
                            GetOverlappedResult(r.dir.0, &*r.ov, &mut n, 1);
                        }
                        break;
                    }
                    // SAFETY: the read completed (its event is set).
                    let ok = unsafe { GetOverlappedResult(r.dir.0, &*r.ov, &mut n, 0) };
                    let err = if ok == 0 {
                        // SAFETY: reads this thread's last error.
                        Some(unsafe { GetLastError() })
                    } else {
                        None
                    };
                    let ov = Event {
                        path: root.clone(),
                        overflow: true,
                    };
                    match err {
                        Some(ERROR_NOTIFY_ENUM_DIR) => send(&raw_tx, ov, &lost),
                        None if n == 0 => send(&raw_tx, ov, &lost),
                        Some(_) => {
                            send(&raw_tx, ov, &lost);
                            break;
                        }
                        None => parse(&r.buf[..n as usize], &root, &raw_tx, &lost),
                    }
                    if r.arm().is_err() {
                        send(
                            &raw_tx,
                            Event {
                                path: root.clone(),
                                overflow: true,
                            },
                            &lost,
                        );
                        break;
                    }
                }
                // Use up the slot before the event closes.
                slot.lock().unwrap_or_else(|p| p.into_inner()).take();
                close(stop_ev);
                close(r.io);
                close(r.dir);
                drop(raw_tx);
            })
            .map_err(Error::Io)?
    };

    let reg = cancel.on_cancel(signal.clone());
    let threads = Mutex::new(Some((reader_thread, coalescer)));
    let stop = Box::new(move || {
        signal();
        if let Some((r, c)) = threads.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = r.join();
            let _ = c.join();
        }
        drop(reg);
    });
    Ok(Watcher::new(out_rx, stop))
}

/// Walks FILE_NOTIFY_INFORMATION records: NextEntryOffset (u32), Action
/// (u32), FileNameLength (u32, bytes), then the UTF-16 name.
fn parse(b: &[u8], root: &str, raw: &std::sync::mpsc::SyncSender<Event>, lost: &AtomicBool) {
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let mut off = 0usize;
    while off + 12 <= b.len() {
        let next = u32_at(off) as usize;
        let name_len = u32_at(off + 8) as usize;
        let start = off + 12;
        let end = start + name_len;
        if end > b.len() {
            return;
        }
        let units: Vec<u16> = b[start..end]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        let name = String::from_utf16_lossy(&units);
        send(
            raw,
            Event {
                path: join(root, &name),
                overflow: false,
            },
            lost,
        );
        if next == 0 {
            return;
        }
        off += next;
    }
}

/// `filepath.Join(root, name)` for an absolute, clean root and a relative
/// name as the OS reports it.
fn join(root: &str, name: &str) -> String {
    if name.is_empty() {
        return root.to_string();
    }
    if root.ends_with('\\') || root.ends_with('/') {
        format!("{root}{name}")
    } else {
        format!("{root}\\{name}")
    }
}
