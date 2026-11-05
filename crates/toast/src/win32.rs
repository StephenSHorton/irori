//! The Win32 cards (go-toast toast_windows.go, win32_windows.go).
//!
//! Each card is a tiny native window, not a webview: it must never take
//! focus from a game. Cards are `WS_EX_NOACTIVATE`, answer
//! `WM_MOUSEACTIVATE` with `MA_NOACTIVATE`, are shown with
//! `SW_SHOWNOACTIVATE` and put back on top of the topmost band whenever a
//! note is posted. Exclusive-fullscreen games bypass the compositor, so
//! nothing draws over them; that is what the sound is for.
//!
//! All the windows live on one dedicated OS thread with its own message
//! loop, started by the first post. Other threads change the model under
//! the engine's lock and post `WM_SYNC` to the thread's message-only host
//! window. With `capture_exclude` the cards are left out of screen capture
//! (`WDA_EXCLUDEFROMCAPTURE`, Windows 10 2004+).
//!
//! Cards follow the person's Settings > Appearance through the engine's
//! [`Look`]: every colour comes from `look.palette` (already resolved for
//! light or dark), the corners from `look.radius`. When the theme, radius,
//! glass or light/dark changes the engine calls [`Surface::draw`] again for
//! the cards already on screen, which re-cuts the window region and
//! repaints. With the scheme on System, the OS mode is read from the
//! registry (`AppsUseLightTheme`); a card hears the `ImmersiveColorSet`
//! setting change when it flips and asks the host to sync, so cards restyle
//! at once.
//!
//! Corners are a window region (`SetWindowRgn`), not DWM's Windows 11
//! corner preference, which isn't reliable on layered windows. Glass is a
//! seam for now ([`apply_backdrop`]): the cards paint the solid
//! `palette.background`, the fallback, until the backdrop lands.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Instant;

use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, BeginPaint,
    BitBlt, CLEARTYPE_QUALITY, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection,
    CreateFontIndirectW, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DIB_RGB_COLORS,
    DT_CALCRECT, DT_CENTER, DT_EDITCONTROL, DT_END_ELLIPSIS, DT_NOCLIP, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DT_WORDBREAK, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect,
    GetDC, GetMonitorInfoW, GetStockObject, HALFTONE, HBITMAP, HDC, HGDIOBJ, InvalidateRect,
    LOGFONTW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromRect,
    MonitorFromWindow, NULL_BRUSH, PAINTSTRUCT, PS_SOLID, ReleaseDC, RoundRect, SRCCOPY,
    SelectClipRgn, SelectObject, SetBkMode, SetBrushOrgEx, SetStretchBltMode, SetTextColor,
    SetWindowRgn, StretchDIBits, TRANSPARENT,
};
use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetForegroundWindow, GetMessageW, HWND_MESSAGE, HWND_TOPMOST, IDC_ARROW, IDC_HAND, KillTimer,
    LWA_ALPHA, LoadCursorW, MA_NOACTIVATE, MSG, PostMessageW, RegisterClassExW,
    SPI_GETCLIENTAREAANIMATION, SPI_SETWORKAREA, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER,
    SetCursor, SetLayeredWindowAttributes, SetTimer, SetWindowDisplayAffinity, SetWindowPos,
    ShowWindow, SystemParametersInfoW, TranslateMessage, ULW_ALPHA, UpdateLayeredWindow,
    WDA_EXCLUDEFROMCAPTURE, WM_APP, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR,
    WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::ENGINE;
use crate::confetti::{self, Burst};
use crate::config::{Color, Config, Kind};
use crate::engine::{Look, Mouse, Surface, TICK_MS};
use crate::layout::{
    BODY_GAP_DIP, BODY_LINE_DIP, BODY_SIZE_DIP, BUTTON_SIZE_DIP, ICON_LEFT_DIP, ICON_SIZE_DIP,
    PAD_TOP_DIP, Part, Rect, TEXT_LEFT_DIP, TEXT_RIGHT_DIP, TITLE_DIP, TITLE_RIGHT_DIP,
    TITLE_SIZE_DIP, px, stripe_rect,
};
use crate::note::Image;

/// The platform draws cards.
pub const DRAWS: bool = true;

const WM_SYNC: u32 = WM_APP + 1;
/// A display or work-area change: re-place the stack.
const WM_REHOME: u32 = WM_APP + 2;
const TIMER_TICK: usize = 1;
/// The confetti overlay's frames.
const TIMER_CONFETTI: usize = 2;
const CONFETTI_FRAME_MS: u32 = 16;
/// WM_MOUSELEAVE (in windows-sys' Controls module, which this crate does not
/// otherwise need).
const WM_MOUSELEAVE: u32 = 0x02A3;

const ICON_FACE: &str = "Segoe MDL2 Assets"; // on every Windows 10/11
const GLYPH_X: &str = "\u{E8BB}"; // ChromeClose

fn icon_for(k: Kind) -> &'static str {
    match k {
        Kind::Success => "\u{E73E}", // CheckMark
        Kind::Warn => "\u{E7BA}",    // Warning
        Kind::Error => "\u{E783}",   // Error
        Kind::Info => "\u{E946}",    // Info
    }
}

/// The host window (0 until the thread starts).
static HOST: AtomicIsize = AtomicIsize::new(0);
static STARTED: Once = Once::new();

/// The buffers handed to PlaySound: async playback reads the memory while
/// it plays, after a volume change has dropped it from the cache.
type Held = (usize, [Option<Arc<[u8]>>; 4]);
static HOLD: Mutex<Held> = Mutex::new((0, [None, None, None, None]));

fn host() -> HWND {
    HOST.load(Ordering::Acquire) as HWND
}

/// Asks the window thread to sync, starting it the first time. A post that
/// races the start is picked up by the thread's first sync.
pub fn wake() {
    let h = host();
    if !h.is_null() {
        unsafe { PostMessageW(h, WM_SYNC, 0, 0) };
        return;
    }
    STARTED.call_once(|| {
        let r = std::thread::Builder::new()
            .name("toast-windows".into())
            .spawn(run);
        if let Err(e) = r {
            eprintln!(
                "{}can't start the card thread: {e}",
                ENGINE.config().log_prefix()
            );
        }
    });
}

/// Syncs only when the thread runs (an options change before any card).
pub fn wake_if_started() {
    let h = host();
    if !h.is_null() {
        unsafe { PostMessageW(h, WM_SYNC, 0, 0) };
    }
}

/// Plays a WAV without waiting.
pub fn play(b: Arc<[u8]>) {
    if b.is_empty() {
        return;
    }
    let mut hold = HOLD.lock().unwrap_or_else(|e| e.into_inner());
    let ptr = b.as_ptr();
    let slot = hold.0;
    hold.1[slot] = Some(b);
    hold.0 = (slot + 1) % 4;
    unsafe {
        PlaySoundW(
            ptr as *const u16,
            null_mut(),
            SND_ASYNC | SND_NODEFAULT | SND_MEMORY,
        )
    };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The window thread. It lives as long as the process.
fn run() {
    let c = ENGINE.config();
    let inst = unsafe { GetModuleHandleW(null()) };
    let host_class = wide(&c.host_class_name());
    let card_class = wide(&c.card_class_name());
    let confetti_class = wide(&confetti_class_name(&c));
    for (class, proc) in [
        (
            &host_class,
            host_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
        ),
        (&card_class, card_proc),
        (&confetti_class, confetti_proc),
    ] {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: inst,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
            hIconSm: null_mut(),
        };
        unsafe { RegisterClassExW(&wc) };
    }
    let title = wide(&c.app_name);
    let h = unsafe {
        CreateWindowExW(
            0,
            host_class.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            inst,
            null(),
        )
    };
    if h.is_null() {
        eprintln!("{}can't create the notification window", c.log_prefix());
        return;
    }
    RUN_CFG.with(|r| *r.borrow_mut() = Some(c));
    HOST.store(h as isize, Ordering::Release);
    sync();

    let mut m: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut m, null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}

/// One card window's thread-side state.
struct Win {
    key: u64,
    look: Option<Look>,
    tracking: bool,
    /// The region the window has: (w, h, radius) px, or all zero for none.
    rgn: (i32, i32, i32),
}

thread_local! {
    /// The Config the thread started with (class names, title).
    static RUN_CFG: RefCell<Option<Config>> = const { RefCell::new(None) };
    static WINS: RefCell<HashMap<isize, Win>> = RefCell::new(HashMap::new());
    static BY_KEY: RefCell<HashMap<u64, isize>> = RefCell::new(HashMap::new());
    /// The OS's dark mode as last read; cleared when a card hears it change.
    static OS_DARK: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Whether Windows' app mode is dark: HKCU\..\Themes\Personalize
/// `AppsUseLightTheme` is 0 (missing, as before Windows 10 1809, is light).
fn read_os_dark() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut v: u32 = 1;
    let mut n = std::mem::size_of::<u32>() as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut v as *mut u32).cast(),
            &mut n,
        )
    };
    r == 0 && v == 0
}

/// Whether lp (a WM_SETTINGCHANGE lParam) is the wide string s. It may be
/// null; it is read only up to its terminator or the first mismatch.
fn lparam_is(lp: LPARAM, s: &str) -> bool {
    let p = lp as *const u16;
    if p.is_null() {
        return false;
    }
    for (i, want) in s.encode_utf16().chain(std::iter::once(0)).enumerate() {
        // SAFETY: every earlier unit matched a non-zero unit of s, so the
        // string goes on at least to i.
        if unsafe { *p.add(i) } != want {
            return false;
        }
    }
    true
}

fn hwnd_of(key: u64) -> HWND {
    BY_KEY.with(|m| m.borrow().get(&key).copied().unwrap_or(0)) as HWND
}

fn sync() {
    ENGINE.sync(&mut Win32Surface, Instant::now());
}

unsafe extern "system" fn host_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SYNC => {
            sync();
            0
        }
        WM_REHOME => {
            ENGINE.rehome();
            sync();
            0
        }
        WM_TIMER if wp == TIMER_CONFETTI => {
            confetti_frame();
            0
        }
        WM_TIMER => {
            ENGINE.tick(Instant::now());
            sync();
            0
        }
        _ => unsafe { DefWindowProcW(h, msg, wp, lp) },
    }
}

fn post_host(msg: u32) {
    let hh = host();
    if !hh.is_null() {
        unsafe { PostMessageW(hh, msg, 0, 0) };
    }
}

unsafe extern "system" fn card_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // Top-level cards get the display broadcasts the message-only host
    // doesn't; each forwards them, and re-placing is idempotent.
    if msg == WM_DISPLAYCHANGE
        || msg == WM_DPICHANGED
        || (msg == WM_SETTINGCHANGE && wp == SPI_SETWORKAREA as usize)
    {
        post_host(WM_REHOME);
    }
    // The OS light/dark mode flipped: re-read it and restyle the cards (a
    // no-op unless the scheme is System).
    if msg == WM_SETTINGCHANGE && lparam_is(lp, "ImmersiveColorSet") {
        OS_DARK.with(|c| c.set(None));
        post_host(WM_SYNC);
    }
    let key = WINS.with(|w| w.borrow().get(&(h as isize)).map(|w| w.key));
    let Some(key) = key else {
        if msg == WM_MOUSEACTIVATE {
            return MA_NOACTIVATE as LRESULT;
        }
        return unsafe { DefWindowProcW(h, msg, wp, lp) };
    };
    let x = (lp & 0xffff) as u16 as i16 as i32;
    let y = ((lp >> 16) & 0xffff) as u16 as i16 as i32;
    let mouse = |ev: Mouse| {
        if ENGINE.mouse(&mut Win32Surface, key, ev, x, y) {
            post_host(WM_SYNC); // not inside the clicked window's own message
        }
    };
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(h);
            0
        }
        WM_SETCURSOR => {
            let hand = WINS.with(|w| {
                w.borrow()
                    .get(&(h as isize))
                    .and_then(|w| w.look.as_ref().map(Look::hand_cursor))
                    .unwrap_or(false)
            });
            unsafe {
                SetCursor(LoadCursorW(
                    null_mut(),
                    if hand { IDC_HAND } else { IDC_ARROW },
                ))
            };
            1
        }
        WM_MOUSEMOVE => {
            let tracking = WINS.with(|w| w.borrow().get(&(h as isize)).is_some_and(|w| w.tracking));
            if !tracking {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: h,
                    dwHoverTime: 0,
                };
                let ok = unsafe { TrackMouseEvent(&mut tme) } != 0;
                WINS.with(|w| {
                    if let Some(w) = w.borrow_mut().get_mut(&(h as isize)) {
                        w.tracking = ok;
                    }
                });
            }
            mouse(Mouse::Move);
            0
        }
        WM_MOUSELEAVE => {
            WINS.with(|w| {
                if let Some(w) = w.borrow_mut().get_mut(&(h as isize)) {
                    w.tracking = false;
                }
            });
            mouse(Mouse::Leave);
            0
        }
        WM_LBUTTONDOWN => {
            mouse(Mouse::Down);
            0
        }
        WM_LBUTTONUP => {
            mouse(Mouse::Up);
            0
        }
        WM_RBUTTONUP => {
            mouse(Mouse::RightUp);
            0
        }
        _ => unsafe { DefWindowProcW(h, msg, wp, lp) },
    }
}

/// The work area (physical px) of mon and its DPI scale.
fn work_area_of(mon: windows_sys::Win32::Graphics::Gdi::HMONITOR) -> (Rect, f64) {
    let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    unsafe { GetMonitorInfoW(mon, &mut mi) };
    let (mut dx, mut dy) = (0u32, 0u32);
    let mut s = 1.0;
    if unsafe { GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) } == 0 && dx > 0 {
        s = dx as f64 / 96.0;
    }
    let w = mi.rcWork;
    (
        Rect {
            l: w.left,
            t: w.top,
            r: w.right,
            b: w.bottom,
        },
        s,
    )
}

fn active_work_area() -> (Rect, f64) {
    work_area_of(unsafe { MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST) })
}

fn win_rect(r: Rect) -> RECT {
    RECT {
        left: r.l,
        top: r.t,
        right: r.r,
        bottom: r.b,
    }
}

/// A font of size_dip at scale; the caller deletes it.
fn new_font(face: &str, size_dip: f64, weight: i32, scale: f64) -> HGDIOBJ {
    let mut lf: LOGFONTW = unsafe { std::mem::zeroed() };
    lf.lfHeight = -((size_dip * scale + 0.5) as i32);
    lf.lfWeight = weight;
    lf.lfQuality = CLEARTYPE_QUALITY;
    for (i, u) in face
        .encode_utf16()
        .take(lf.lfFaceName.len() - 1)
        .enumerate()
    {
        lf.lfFaceName[i] = u;
    }
    unsafe { CreateFontIndirectW(&lf) }
}

fn draw_text(dc: HDC, s: &str, r: &mut RECT, format: u32) {
    let u: Vec<u16> = s.encode_utf16().collect();
    if u.is_empty() {
        return;
    }
    unsafe { DrawTextW(dc, u.as_ptr(), u.len() as i32, r, format) };
}

/// s measured in font (DT_CALCRECT) inside width px; 0 is a single line.
fn text_size(dc: HDC, font: HGDIOBJ, s: &str, width: i32) -> (i32, i32) {
    let old = unsafe { SelectObject(dc, font) };
    let mut r = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: 0,
    };
    let format = if width > 0 {
        DT_CALCRECT | DT_NOPREFIX | DT_WORDBREAK
    } else {
        DT_CALCRECT | DT_NOPREFIX | DT_SINGLELINE
    };
    draw_text(dc, s, &mut r, format);
    unsafe { SelectObject(dc, old) };
    (r.right - r.left, r.bottom - r.top)
}

/// The engine's windows on this thread.
struct Win32Surface;

impl Surface for Win32Surface {
    fn work_area(&mut self, active: bool, old: Rect) -> (Rect, f64) {
        if active {
            return active_work_area();
        }
        // The monitor the stack is on, or the active one when it is gone
        // (unplugged, or moved out from under it by a layout change).
        let r = win_rect(old);
        let mon = unsafe { MonitorFromRect(&r, MONITOR_DEFAULTTONULL) };
        if mon.is_null() {
            return active_work_area();
        }
        work_area_of(mon)
    }

    fn body_lines(&mut self, body: &str, width: i32, scale: f64) -> usize {
        let dc = unsafe { GetDC(null_mut()) };
        if dc.is_null() {
            return 1;
        }
        let f = new_font("Segoe UI", BODY_SIZE_DIP, 400, scale);
        let (_, one) = text_size(dc, f, "X", 0);
        let (_, all) = text_size(dc, f, body, width);
        unsafe {
            DeleteObject(f);
            ReleaseDC(null_mut(), dc);
        }
        if one > 0 && all > one + one / 2 { 2 } else { 1 }
    }

    fn label_width(&mut self, label: &str, scale: f64) -> Option<i32> {
        let dc = unsafe { GetDC(null_mut()) };
        if dc.is_null() {
            return None;
        }
        let f = new_font("Segoe UI", BUTTON_SIZE_DIP, 600, scale);
        let (w, _) = text_size(dc, f, label, 0);
        unsafe {
            DeleteObject(f);
            ReleaseDC(null_mut(), dc);
        }
        Some(w)
    }

    fn create(&mut self, key: u64, cfg: &Config, w: i32, h: i32) -> bool {
        let rc = RUN_CFG
            .with(|r| r.borrow().clone())
            .unwrap_or_else(|| cfg.clone());
        let class = wide(&rc.card_class_name());
        let title = wide(&rc.app_name);
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                0,
                0,
                w,
                h,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        if hwnd.is_null() {
            return false;
        }
        unsafe { SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA) }; // fades in from nothing
        if cfg.capture_exclude {
            unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) };
        }
        WINS.with(|m| {
            m.borrow_mut().insert(
                hwnd as isize,
                Win {
                    key,
                    look: None,
                    tracking: false,
                    rgn: (0, 0, 0),
                },
            )
        });
        BY_KEY.with(|m| m.borrow_mut().insert(key, hwnd as isize));
        lock_handles().push((key, hwnd as isize));
        true
    }

    fn draw(&mut self, look: &Look) {
        let h = hwnd_of(look.key);
        if h.is_null() {
            return;
        }
        // Called again for a card on screen when its look changes (theme,
        // radius, glass, light/dark), so the region follows the radius too.
        let rgn = WINS.with(|m| {
            let mut m = m.borrow_mut();
            let w = m.get_mut(&(h as isize))?;
            w.look = Some(look.clone());
            let want = if look.radius > 0 {
                (look.w, look.h, look.radius)
            } else {
                (0, 0, 0)
            };
            if w.rgn != want {
                w.rgn = want;
                return Some(want);
            }
            None
        });
        // The window owns the region once set: it is not deleted here.
        // DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE) would give
        // Windows 11's antialiased corners, but it isn't reliable on layered
        // windows and does nothing on Windows 10, so the region stays the
        // source of truth (the trade-off: slightly jagged corners).
        match rgn {
            Some((0, 0, 0)) => unsafe {
                SetWindowRgn(h, null_mut(), 1);
            },
            Some((w, ht, r)) => unsafe {
                SetWindowRgn(h, CreateRoundRectRgn(0, 0, w + 1, ht + 1, 2 * r, 2 * r), 1);
            },
            None => {}
        }
        apply_backdrop(h, look);
        unsafe { InvalidateRect(h, null(), 0) };
    }

    fn os_dark(&mut self) -> bool {
        // Read afresh when no card is up (none heard a change) or after one
        // heard ImmersiveColorSet; asked only while the scheme is System.
        let fresh = WINS.with(|w| w.borrow().is_empty());
        OS_DARK.with(|c| match c.get() {
            Some(d) if !fresh => d,
            _ => {
                let d = read_os_dark();
                c.set(Some(d));
                d
            }
        })
    }

    fn place(&mut self, key: u64, x: i32, y: i32, w: i32, h: i32, raise: bool, placed: bool) {
        let hw = hwnd_of(key);
        let (mut after, mut flags) = (HWND_TOPMOST, SWP_NOACTIVATE);
        if placed && !raise {
            after = null_mut();
            flags |= SWP_NOZORDER;
        }
        unsafe { SetWindowPos(hw, after, x, y, w, h, flags) };
    }

    fn alpha(&mut self, key: u64, a: u8) {
        unsafe { SetLayeredWindowAttributes(hwnd_of(key), 0, a, LWA_ALPHA) };
    }

    fn show(&mut self, key: u64) {
        unsafe { ShowWindow(hwnd_of(key), SW_SHOWNOACTIVATE) };
    }

    fn destroy(&mut self, key: u64) {
        let h = hwnd_of(key);
        BY_KEY.with(|m| m.borrow_mut().remove(&key));
        lock_handles().retain(|(k, _)| *k != key);
        WINS.with(|m| m.borrow_mut().remove(&(h as isize)));
        if !h.is_null() {
            unsafe { DestroyWindow(h) };
        }
    }

    fn timer(&mut self, on: bool) {
        unsafe {
            if on {
                SetTimer(host(), TIMER_TICK, TICK_MS, None);
            } else {
                KillTimer(host(), TIMER_TICK);
            }
        }
    }

    fn confetti(&mut self, card: Rect, _work: Rect) {
        // Client-area animation off is Windows' Reduce Motion
        // (Settings > Accessibility > Visual effects > Animation effects).
        let mut anim: i32 = 1;
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                &mut anim as *mut i32 as *mut core::ffi::c_void,
                0,
            )
        } != 0;
        if ok && anim == 0 {
            return;
        }
        throw_confetti(card);
    }
}

/// The glass seam. `look.glass` is `None` for a solid card; with `Some`
/// (the tint's opacity) this does nothing yet either, and the card paints
/// the solid `palette.background`, the fallback, so Windows is right but
/// solid until the backdrop lands.
///
/// TODO(#527): the pending PC spike picks the backdrop for an inactive,
/// `WS_EX_LAYERED` card: acrylic via the undocumented
/// SetWindowCompositionAttribute(WCA_ACCENT_POLICY,
/// ACCENT_ENABLE_ACRYLICBLURBEHIND, tint at `glass`) versus
/// DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW)
/// (Windows 11 22H2+, and it may drop the blur on an inactive window).
/// Either way paint() must then leave the backdrop showing (the palette's
/// tint at `glass`, not an opaque background) and keep the region corners,
/// and a card going back to solid must turn the backdrop off. Track the
/// applied state in `Win` so a redraw doesn't re-apply it.
fn apply_backdrop(_h: HWND, look: &Look) {
    match look.glass {
        None => {}              // solid: nothing behind the card
        Some(_tint_alpha) => {} // glass wanted, not done yet: solid fallback
    }
}

// ---- confetti ----

fn confetti_class_name(c: &Config) -> String {
    format!("{}Confetti", c.class_prefix)
}

/// The overlay the bursts are drawn in: a layered, click-through
/// (`WS_EX_TRANSPARENT`), never-activating topmost window over the whole
/// monitor the card is on, fed premultiplied frames with
/// UpdateLayeredWindow. One at a time; destroyed when the last burst ends.
struct Overlay {
    hwnd: HWND,
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
    /// The monitor, physical px.
    area: Rect,
    bursts: Vec<(Burst, Instant)>,
}

thread_local! {
    static CONFETTI: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

unsafe extern "system" fn confetti_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        _ => unsafe { DefWindowProcW(h, msg, wp, lp) },
    }
}

/// Starts a burst from card (physical px), opening the overlay on the
/// card's monitor if it isn't up there already.
fn throw_confetti(card: Rect) {
    let mon = unsafe { MonitorFromRect(&win_rect(card), MONITOR_DEFAULTTONEAREST) };
    let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(mon, &mut mi) } == 0 {
        return;
    }
    let m = mi.rcMonitor;
    let area = Rect {
        l: m.left,
        t: m.top,
        r: m.right,
        b: m.bottom,
    };
    let (_, scale) = work_area_of(mon);
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    let burst = Burst::new(card, area, scale, seed);
    CONFETTI.with(|o| {
        let mut o = o.borrow_mut();
        if o.as_ref().is_some_and(|ov| ov.area != area) {
            close_overlay(o.take());
        }
        if o.is_none() {
            *o = open_overlay(area);
        }
        if let Some(ov) = o.as_mut() {
            ov.bursts.push((burst, Instant::now()));
        }
    });
    confetti_frame();
}

fn open_overlay(area: Rect) -> Option<Overlay> {
    let (w, h) = (area.r - area.l, area.b - area.t);
    if w <= 0 || h <= 0 {
        return None;
    }
    let cfg = RUN_CFG
        .with(|r| r.borrow().clone())
        .unwrap_or_else(|| ENGINE.config());
    let class = wide(&confetti_class_name(&cfg));
    let title = wide(&cfg.app_name);
    unsafe {
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            area.l,
            area.t,
            w,
            h,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if hwnd.is_null() {
            return None;
        }
        if cfg.capture_exclude {
            SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        }
        let screen = GetDC(null_mut());
        let dc = CreateCompatibleDC(screen);
        ReleaseDC(null_mut(), screen);
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut core::ffi::c_void = null_mut();
        let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if dc.is_null() || bmp.is_null() || bits.is_null() {
            if !bmp.is_null() {
                DeleteObject(bmp);
            }
            if !dc.is_null() {
                DeleteDC(dc);
            }
            DestroyWindow(hwnd);
            return None;
        }
        let old = SelectObject(dc, bmp);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(host(), TIMER_CONFETTI, CONFETTI_FRAME_MS, None);
        Some(Overlay {
            hwnd,
            dc,
            bmp,
            old,
            bits: bits as *mut u32,
            w,
            h,
            area,
            bursts: vec![],
        })
    }
}

fn close_overlay(o: Option<Overlay>) {
    let Some(o) = o else { return };
    unsafe {
        KillTimer(host(), TIMER_CONFETTI);
        DestroyWindow(o.hwnd);
        SelectObject(o.dc, o.old);
        DeleteObject(o.bmp);
        DeleteDC(o.dc);
    }
}

/// A frame: drops finished bursts, draws the rest, and closes the overlay
/// when none are left.
fn confetti_frame() {
    CONFETTI.with(|o| {
        let mut o = o.borrow_mut();
        let Some(ov) = o.as_mut() else { return };
        let now = Instant::now();
        ov.bursts
            .retain(|(_, start)| now.saturating_duration_since(*start) < confetti::DURATION);
        if ov.bursts.is_empty() {
            close_overlay(o.take());
            return;
        }
        let pieces: Vec<confetti::Piece> = ov
            .bursts
            .iter()
            .flat_map(|(b, start)| b.at(now.saturating_duration_since(*start)))
            .collect();
        let (w, h) = (ov.w as usize, ov.h as usize);
        // SAFETY: the DIB section is w*h 32-bit pixels and lives as long as ov.
        let buf = unsafe { std::slice::from_raw_parts_mut(ov.bits, w * h) };
        confetti::rasterize(&pieces, w, h, buf);
        let dst = windows_sys::Win32::Foundation::POINT {
            x: ov.area.l,
            y: ov.area.t,
        };
        let src = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
        let size = windows_sys::Win32::Foundation::SIZE { cx: ov.w, cy: ov.h };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                ov.hwnd,
                null_mut(),
                &dst,
                &size,
                ov.dc,
                &src,
                0,
                &blend,
                ULW_ALPHA,
            );
        }
    });
}

/// Draws the card into an off-screen bitmap and copies it in, so a hover
/// repaint doesn't flicker.
fn paint(h: HWND) {
    let look = WINS.with(|m| m.borrow().get(&(h as isize)).and_then(|w| w.look.clone()));
    let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
    let dc = unsafe { BeginPaint(h, &mut ps) };
    if let Some(look) = look {
        unsafe {
            let mem = CreateCompatibleDC(dc);
            if !mem.is_null() {
                let bmp = CreateCompatibleBitmap(dc, look.w, look.h);
                if !bmp.is_null() {
                    let old = SelectObject(mem, bmp);
                    draw_card(mem, &look);
                    BitBlt(dc, 0, 0, look.w, look.h, mem, 0, 0, SRCCOPY);
                    SelectObject(mem, old);
                    DeleteObject(bmp);
                }
                DeleteDC(mem);
            }
        }
    }
    unsafe { EndPaint(h, &ps) };
}

fn cref(c: Color) -> COLORREF {
    c.colorref()
}

fn fill(dc: HDC, r: RECT, c: Color) {
    unsafe {
        let b = CreateSolidBrush(cref(c));
        FillRect(dc, &r, b);
        DeleteObject(b);
    }
}

fn frame(dc: HDC, r: RECT, c: Color) {
    unsafe {
        let b = CreateSolidBrush(cref(c));
        FrameRect(dc, &r, b);
        DeleteObject(b);
    }
}

/// Fills r with bg (when set) and outlines it with border (when set),
/// rounded by radius px; radius 0 is FillRect/FrameRect.
fn boxed(dc: HDC, r: RECT, bg: Color, border: Color, radius: i32) {
    if radius <= 0 {
        if !bg.is_zero() {
            fill(dc, r, bg);
        }
        if !border.is_zero() {
            frame(dc, r, border);
        }
        return;
    }
    unsafe {
        let brush = if bg.is_zero() {
            GetStockObject(NULL_BRUSH)
        } else {
            CreateSolidBrush(cref(bg))
        };
        let edge = if border.is_zero() { bg } else { border };
        let pen = CreatePen(PS_SOLID, 1, cref(edge));
        let ob = SelectObject(dc, brush);
        let op = SelectObject(dc, pen);
        RoundRect(dc, r.left, r.top, r.right, r.bottom, 2 * radius, 2 * radius);
        SelectObject(dc, ob);
        SelectObject(dc, op);
        if !bg.is_zero() {
            DeleteObject(brush);
        }
        DeleteObject(pen);
    }
}

#[allow(clippy::too_many_arguments)]
fn text(
    dc: HDC,
    s: &str,
    face: &str,
    size_dip: f64,
    weight: i32,
    c: Color,
    mut r: RECT,
    format: u32,
    scale: f64,
) {
    let f = new_font(face, size_dip, weight, scale);
    unsafe {
        let old = SelectObject(dc, f);
        SetTextColor(dc, cref(c));
        draw_text(dc, s, &mut r, format | DT_NOPREFIX);
        SelectObject(dc, old);
        DeleteObject(f);
    }
}

fn rect_of(r: Rect) -> RECT {
    win_rect(r)
}

/// img's pixels as top-down BGRX for a DIB, blended over bg (the card's
/// window is opaque: its fade is whole-window alpha, not per pixel).
fn bgrx_over(img: &Image, bg: Color) -> Vec<u8> {
    let (br, bgg, bb) = bg.rgb();
    let mix = |c: u8, b: u8, a: u32| ((c as u32 * a + b as u32 * (255 - a) + 127) / 255) as u8;
    img.rgba
        .chunks_exact(4)
        .flat_map(|p| {
            let a = p[3] as u32;
            [mix(p[2], bb, a), mix(p[1], bgg, a), mix(p[0], br, a), 255]
        })
        .collect()
}

/// The thumbnail scaled into r (HALFTONE, so a shrink stays smooth),
/// clipped to rounded corners, with a 1px border.
fn draw_thumb(dc: HDC, img: &Image, r: RECT, bg: Color, border: Color, radius: i32) {
    let (dw, dh) = (r.right - r.left, r.bottom - r.top);
    if dw <= 0 || dh <= 0 {
        return;
    }
    let px = bgrx_over(img, bg);
    let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bmi.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: img.w as i32,
        biHeight: -(img.h as i32), // top-down
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..unsafe { std::mem::zeroed() }
    };
    unsafe {
        let clip = if radius > 0 {
            let rgn = CreateRoundRectRgn(
                r.left,
                r.top,
                r.right + 1,
                r.bottom + 1,
                2 * radius,
                2 * radius,
            );
            SelectClipRgn(dc, rgn);
            Some(rgn)
        } else {
            None
        };
        let old = SetStretchBltMode(dc, HALFTONE);
        SetBrushOrgEx(dc, 0, 0, null_mut()); // HALFTONE needs it reset
        StretchDIBits(
            dc,
            r.left,
            r.top,
            dw,
            dh,
            0,
            0,
            img.w as i32,
            img.h as i32,
            px.as_ptr().cast(),
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        SetStretchBltMode(dc, old);
        if let Some(rgn) = clip {
            SelectClipRgn(dc, null_mut());
            DeleteObject(rgn);
        }
    }
    boxed(dc, r, Color(0), border, radius);
}

fn draw_card(dc: HDC, cw: &Look) {
    let sc = cw.scale;
    let pal = cw.palette;
    let p = |dip: f64| px(dip, sc);
    let (w, h) = (cw.w, cw.h);
    unsafe { SetBkMode(dc, TRANSPARENT as i32) };

    let all = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    let (sr, sradius) = stripe_rect(h, cw.radius, sc);
    let stripe = rect_of(sr);
    let accent = pal.accent.for_kind(cw.kind);
    if cw.radius <= 0 {
        fill(dc, all, pal.background);
        frame(dc, all, pal.border);
        fill(dc, stripe, accent);
    } else {
        // The window region clips the corners; a rounded card's stripe is
        // an inset pill (layout::stripe_rect).
        fill(dc, all, pal.background);
        boxed(dc, stripe, accent, accent, sradius);
        boxed(dc, all, Color(0), pal.border, cw.radius);
    }

    let top = p(PAD_TOP_DIP);
    text(
        dc,
        icon_for(cw.kind),
        ICON_FACE,
        ICON_SIZE_DIP,
        400,
        accent,
        RECT {
            left: p(ICON_LEFT_DIP),
            top,
            right: p(TEXT_LEFT_DIP),
            bottom: top + p(TITLE_DIP),
        },
        DT_SINGLELINE | DT_VCENTER | DT_NOCLIP,
        sc,
    );
    text(
        dc,
        &cw.title,
        "Segoe UI",
        TITLE_SIZE_DIP,
        600,
        pal.foreground,
        RECT {
            left: p(TEXT_LEFT_DIP),
            top,
            right: w - p(TITLE_RIGHT_DIP),
            bottom: top + p(TITLE_DIP),
        },
        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        sc,
    );
    if cw.lines > 0 {
        // The body wraps to the lines the card was sized for; DT_EDITCONTROL
        // hides a partial extra line and DT_END_ELLIPSIS marks the cut.
        let bt = top + p(TITLE_DIP + BODY_GAP_DIP);
        text(
            dc,
            &cw.body,
            "Segoe UI",
            BODY_SIZE_DIP,
            400,
            pal.muted,
            RECT {
                left: p(TEXT_LEFT_DIP),
                top: bt,
                right: w - p(TEXT_RIGHT_DIP),
                bottom: bt + p(cw.lines as f64 * BODY_LINE_DIP),
            },
            DT_WORDBREAK | DT_END_ELLIPSIS | DT_EDITCONTROL,
            sc,
        );
    }

    // Inner pieces (the thumbnail, the x's hover fill, buttons) get a
    // smaller radius.
    let inner = cw.radius * 6 / 10;
    if let Some((img, r)) = &cw.image {
        draw_thumb(dc, img, rect_of(*r), pal.background, pal.border, inner);
    }
    let mut xc = pal.faint;
    if cw.hot == Part::Close {
        boxed(dc, rect_of(cw.close), pal.hover, Color(0), inner);
        xc = pal.foreground;
    } else if cw.hovered {
        xc = pal.muted;
    }
    text(
        dc,
        GLYPH_X,
        ICON_FACE,
        9.0,
        400,
        xc,
        rect_of(cw.close),
        DT_SINGLELINE | DT_VCENTER | DT_CENTER,
        sc,
    );

    for (i, b) in cw.buttons.iter().enumerate() {
        let Some(label) = cw.labels.get(i) else { break };
        let hot = cw.hot == Part::Button(i);
        let (mut bg, mut fg, mut border) = (pal.button, pal.foreground, pal.border);
        if hot {
            bg = pal.hover;
        }
        if i == 0 {
            // primary
            bg = if hot { pal.primary_hover } else { pal.primary };
            fg = pal.on_primary;
            border = Color(0);
        }
        boxed(dc, rect_of(*b), bg, border, inner);
        text(
            dc,
            label,
            "Segoe UI",
            BUTTON_SIZE_DIP,
            600,
            fg,
            rect_of(*b),
            DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_END_ELLIPSIS,
            sc,
        );
    }
}

// ---- smoke-test hooks ------------------------------------------------------

/// The card windows' handles, oldest first.
pub fn card_windows() -> Vec<isize> {
    ENGINE.keys().into_iter().filter_map(handle_of).collect()
}

static HANDLES: Mutex<Vec<(u64, isize)>> = Mutex::new(Vec::new());

fn lock_handles() -> std::sync::MutexGuard<'static, Vec<(u64, isize)>> {
    HANDLES.lock().unwrap_or_else(|e| e.into_inner())
}

fn handle_of(key: u64) -> Option<isize> {
    lock_handles()
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, h)| *h)
}

/// A window's client size (smoke tests).
pub fn client_size(h: isize) -> (i32, i32) {
    let mut r: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetClientRect(h as HWND, &mut r) };
    (r.right, r.bottom)
}

/// Sends a synthetic mouse message to a card, as a real click arrives.
pub fn post_mouse(h: isize, msg: u32, x: i32, y: i32) {
    let lp = (x as u16 as isize) | ((y as u16 as isize) << 16);
    let wp = if msg == WM_LBUTTONDOWN { 1 } else { 0 };
    unsafe { PostMessageW(h as HWND, msg, wp, lp) };
}

pub const MSG_MOUSEMOVE: u32 = WM_MOUSEMOVE;
pub const MSG_LBUTTONDOWN: u32 = WM_LBUTTONDOWN;
pub const MSG_LBUTTONUP: u32 = WM_LBUTTONUP;

/// The foreground window (smoke tests check focus never moves).
pub fn foreground() -> isize {
    unsafe { GetForegroundWindow() as isize }
}

/// Makes this process per-monitor-v2 DPI aware, as the app's manifest does
/// (Go's wails.exe.manifest). For test programs without that manifest; the
/// app embeds it in app/build.rs instead (architecture.md critic #4).
pub fn set_dpi_aware() {
    use windows_sys::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}
