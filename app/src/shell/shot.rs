//! Pictures of the app's own window, for checking a UI change in the real
//! app on a Mac (card #587). An agent session there has no Screen
//! Recording permission, but a process may always capture its own windows
//! (`CGWindowListCreateImage` on its own window number).
//!
//! A debug build on macOS with `IRORI_DEV_WINDOW_SHOTS=<dir>` saves
//! `<dir>/window-<n>.png` of the main window, `IRORI_DEV_WINDOW_SHOT_COUNT`
//! times (default 1), 1.5 s apart, starting `IRORI_DEV_WINDOW_SHOT_AFTER`
//! seconds after the window opens (default 15). `IRORI_DEV_WINDOW_QUERY`,
//! when set, opens the window on `index.html?dev&<query>`, so the
//! frontend's preview switches apply in the app too (`view=canvas&full`;
//! frontend/src/lib/preview.ts). `IRORI_DEV_WINDOW_SIZE=390x844` opens it
//! at that size.
//! Run it on a copy of a data dir (`IRORI_HOME`) with `IRORI_DEV_OFFLINE=1`.
//! A release build ignores all of it.

use tauri::AppHandle;

/// Where the main window opens for a window check: `index.html?dev&` and
/// `IRORI_DEV_WINDOW_QUERY`, with `IRORI_DEV_WINDOW_SHOTS` set; never in a
/// release build.
pub fn window_url() -> Option<tauri::WebviewUrl> {
    if !cfg!(all(target_os = "macos", debug_assertions)) {
        return None;
    }
    std::env::var_os("IRORI_DEV_WINDOW_SHOTS")?;
    let q = std::env::var("IRORI_DEV_WINDOW_QUERY")
        .ok()
        .filter(|s| !s.is_empty())?;
    Some(tauri::WebviewUrl::App(format!("index.html?dev&{q}").into()))
}

/// The main window's size for a window check: `IRORI_DEV_WINDOW_SIZE` as
/// `<width>x<height>` (`390x844` for phone width, card #783), with
/// `IRORI_DEV_WINDOW_SHOTS` set; never in a release build.
pub fn window_size() -> Option<(f64, f64)> {
    if !cfg!(all(target_os = "macos", debug_assertions)) {
        return None;
    }
    std::env::var_os("IRORI_DEV_WINDOW_SHOTS")?;
    let v = std::env::var("IRORI_DEV_WINDOW_SIZE").ok()?;
    let (w, h) = v.split_once('x')?;
    let (w, h): (f64, f64) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w >= 200.0 && h >= 200.0).then_some((w, h))
}

pub fn start(app: &AppHandle) {
    #[cfg(all(target_os = "macos", debug_assertions))]
    mac::start(app);
    #[cfg(not(all(target_os = "macos", debug_assertions)))]
    let _ = app;
}

#[cfg(all(target_os = "macos", debug_assertions))]
mod mac {
    use std::ffi::c_void;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use tauri::{AppHandle, Manager};

    pub fn start(app: &AppHandle) {
        let Some(dir) = std::env::var_os("IRORI_DEV_WINDOW_SHOTS").map(PathBuf::from) else {
            return;
        };
        let secs = |k: &str, d: f64| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(d)
        };
        let after = secs("IRORI_DEV_WINDOW_SHOT_AFTER", 15.0);
        let count = secs("IRORI_DEV_WINDOW_SHOT_COUNT", 1.0).max(1.0) as usize;
        let app = app.clone();
        let _ = std::thread::Builder::new()
            .name("irori-shot".into())
            .spawn(move || {
                let _ = std::fs::create_dir_all(&dir);
                std::thread::sleep(Duration::from_secs_f64(after.max(0.0)));
                for n in 1..=count {
                    let path = dir.join(format!("window-{n}.png"));
                    match grab(&app, &path) {
                        Ok(()) => irori_core::log(&format!("shot: saved {}", path.display())),
                        Err(e) => irori_core::log(&format!("shot: {e}")),
                    }
                    std::thread::sleep(Duration::from_millis(1500));
                }
            });
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCreateImage(r: CGRect, opt: u32, id: u32, img: u32) -> *mut c_void;
        fn CGImageRelease(i: *mut c_void);
    }
    #[link(name = "ImageIO", kind = "framework")]
    unsafe extern "C" {
        fn CGImageDestinationCreateWithURL(
            url: *const c_void,
            ty: *const c_void,
            count: usize,
            opts: *const c_void,
        ) -> *mut c_void;
        fn CGImageDestinationAddImage(d: *mut c_void, i: *mut c_void, p: *const c_void);
        fn CGImageDestinationFinalize(d: *mut c_void) -> bool;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            a: *const c_void,
            buf: *const u8,
            len: isize,
            dir: bool,
        ) -> *const c_void;
        fn CFStringCreateWithCString(a: *const c_void, s: *const i8, enc: u32) -> *const c_void;
        fn CFRelease(p: *const c_void);
    }

    // The main window's number, read on the main thread (AppKit's rule).
    fn window_number(app: &AppHandle) -> Result<u32, String> {
        let w = app
            .get_webview_window(super::super::MAIN)
            .ok_or("no main window")?;
        let (tx, rx) = std::sync::mpsc::channel();
        let w2 = w.clone();
        w.run_on_main_thread(move || {
            let n = w2.ns_window().ok().map(|p| {
                let obj = p as *mut AnyObject;
                // SAFETY: ns_window is the live NSWindow, used on the main thread.
                let n: isize = unsafe { msg_send![&*obj, windowNumber] };
                n
            });
            let _ = tx.send(n);
        })
        .map_err(|e| e.to_string())?;
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Some(n)) if n > 0 => Ok(n as u32),
            _ => Err("no window number".into()),
        }
    }

    fn grab(app: &AppHandle, path: &Path) -> Result<(), String> {
        let id = window_number(app)?;
        // SAFETY: plain CoreGraphics/ImageIO calls; every object made here
        // is released here.
        unsafe {
            // CGRectNull: the window's own bounds.
            let null = CGRect {
                x: f64::INFINITY,
                y: f64::INFINITY,
                w: 0.0,
                h: 0.0,
            };
            // kCGWindowListOptionIncludingWindow, kCGWindowImageBoundsIgnoreFraming.
            let img = CGWindowListCreateImage(null, 1 << 3, id, 1 << 0);
            if img.is_null() {
                return Err("no image".into());
            }
            let p = path.as_os_str().as_bytes();
            let url = CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                p.as_ptr(),
                p.len() as isize,
                false,
            );
            let ty =
                CFStringCreateWithCString(std::ptr::null(), c"public.png".as_ptr(), 0x0800_0100);
            let dest = CGImageDestinationCreateWithURL(url, ty, 1, std::ptr::null());
            let ok = !dest.is_null() && {
                CGImageDestinationAddImage(dest, img, std::ptr::null());
                CGImageDestinationFinalize(dest)
            };
            if !dest.is_null() {
                CFRelease(dest);
            }
            if !ty.is_null() {
                CFRelease(ty);
            }
            if !url.is_null() {
                CFRelease(url);
            }
            CGImageRelease(img);
            if ok {
                Ok(())
            } else {
                Err("can't write the PNG".into())
            }
        }
    }
}
