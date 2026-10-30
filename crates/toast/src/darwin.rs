//! The macOS cards (go-toast toast_darwin.go, toast_darwin.m), on objc2.
//!
//! NOT COMPILED ON THE PC: this file is written against objc2 0.6.5,
//! objc2-app-kit 0.3.2, objc2-foundation 0.3.2, block2 0.6.2 and dispatch2
//! 0.3.1 as read from their sources, and is built and checked on the Mac
//! (crates/toast/MAC.md lists exactly what to compile and check).
//!
//! Each card is a borderless, non-activating `NSPanel` that can never
//! become key or main, shown with `orderFrontRegardless`, at the status
//! window level on every Space (full-screen ones too), and that doesn't
//! hide when the app is in the background. So, as on Windows, a card never
//! takes focus and clicking one doesn't activate the app.
//!
//! AppKit only works on the main thread, so that is the window thread
//! here: other threads change the model under the engine's lock and queue a
//! sync on the main dispatch queue ([`wake`]); the main thread runs the
//! engine's sync, the 30 ms tick timer and the mouse handlers. The main
//! queue runs because Tauri runs NSApp on the main thread; this crate never
//! starts an event loop of its own (go-toast's `Main` is not ported).
//!
//! Geometry is in points, so the engine's scale is always 1: AppKit draws
//! at the screen's backing scale by itself. The engine's coordinates are
//! top-down from the primary screen's top-left; AppKit's are bottom-up.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSBackingStoreType, NSBezierPath, NSColor,
    NSCompositingOperation, NSCursor, NSEvent, NSEventModifierFlags, NSFont, NSFontAttributeName,
    NSFontWeightRegular, NSFontWeightSemibold, NSForegroundColorAttributeName, NSImage,
    NSImageSymbolConfiguration, NSLineBreakMode, NSMutableParagraphStyle, NSPanel,
    NSParagraphStyleAttributeName, NSRectFill, NSRectFillUsingOperation, NSResponder, NSScreen,
    NSSound, NSStatusWindowLevel, NSStringDrawing, NSStringDrawingOptions,
    NSStringNSExtendedStringDrawing, NSTextAlignment, NSTrackingArea, NSTrackingAreaOptions,
    NSView, NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowSharingType,
    NSWindowStyleMask,
};
use objc2_foundation::{
    NSData, NSDictionary, NSNotificationCenter, NSOperationQueue, NSPoint, NSRect, NSRunLoop,
    NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};

use crate::ENGINE;
use crate::config::{Color, Config, Kind};
use crate::engine::{Look, Mouse, Surface, TICK_MS};
use crate::layout::{
    BODY_GAP_DIP, BODY_SIZE_DIP, BUTTON_SIZE_DIP, ICON_LEFT_DIP, ICON_SIZE_DIP, PAD_TOP_DIP, Part,
    Rect, STRIPE_DIP, TEXT_LEFT_DIP, TEXT_RIGHT_DIP, TITLE_DIP, TITLE_RIGHT_DIP, TITLE_SIZE_DIP,
};

/// The platform draws cards.
pub const DRAWS: bool = true;

/// A post has happened (options changes before it don't touch AppKit).
static STARTED: AtomicBool = AtomicBool::new(false);
/// A sync is queued on the main thread.
static QUEUED: AtomicBool = AtomicBool::new(false);

/// Queues a sync on the main thread, unless one is queued already.
pub fn wake() {
    STARTED.store(true, Ordering::Release);
    if QUEUED.swap(true, Ordering::AcqRel) {
        return;
    }
    DispatchQueue::main().exec_async(|| {
        QUEUED.store(false, Ordering::Release); // before syncing: a change during it queues another
        sync();
    });
}

pub fn wake_if_started() {
    if STARTED.load(Ordering::Acquire) {
        wake();
    }
}

/// Plays a WAV without waiting. NSSound copies nothing it doesn't need;
/// the players are held until they finish (at most 8).
pub fn play(b: Arc<[u8]>) {
    DispatchQueue::main().exec_async(move || {
        let data = NSData::with_bytes(&b);
        let Some(s) = NSSound::initWithData(NSSound::alloc(), &data) else {
            return;
        };
        PLAYING.with(|p| {
            let mut p = p.borrow_mut();
            p.retain(|s| s.isPlaying());
            if p.len() >= 8 {
                p[0].stop();
                p.remove(0);
            }
            s.play();
            p.push(s);
        });
    });
}

thread_local! {
    static PANELS: RefCell<HashMap<u64, Retained<ToastPanel>>> = RefCell::new(HashMap::new());
    static TICKER: RefCell<Option<Retained<NSTimer>>> = const { RefCell::new(None) };
    static SETUP: Cell<bool> = const { Cell::new(false) };
    static PLAYING: RefCell<Vec<Retained<NSSound>>> = const { RefCell::new(Vec::new()) };
}

fn sync() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    setup();
    ENGINE.sync(&mut MacSurface { mtm }, Instant::now());
}

/// Once: watch for screen changes (re-place the stack).
fn setup() {
    if SETUP.with(|s| s.replace(true)) {
        return;
    }
    let block = RcBlock::new(|_n: std::ptr::NonNull<objc2_foundation::NSNotification>| {
        ENGINE.rehome();
        sync();
    });
    unsafe {
        let obs = NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSApplicationDidChangeScreenParametersNotification),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &block,
        );
        std::mem::forget(obs); // for the life of the process
    }
}

/// The primary screen's height: AppKit's global coordinates are bottom-up
/// from its bottom-left corner.
fn primary_height(mtm: MainThreadMarker) -> f64 {
    let ss = NSScreen::screens(mtm);
    ss.firstObject()
        .map(|s| s.frame().size.height)
        .unwrap_or(0.0)
}

fn top_down(mtm: MainThreadMarker, r: NSRect) -> Rect {
    let h = primary_height(mtm);
    Rect {
        l: r.origin.x.round() as i32,
        t: (h - (r.origin.y + r.size.height)).round() as i32,
        r: (r.origin.x + r.size.width).round() as i32,
        b: (h - r.origin.y).round() as i32,
    }
}

fn rgb(c: Color) -> Retained<NSColor> {
    let (r, g, b) = c.rgb();
    NSColor::colorWithSRGBRed_green_blue_alpha(
        r as f64 / 255.0,
        g as f64 / 255.0,
        b as f64 / 255.0,
        1.0,
    )
}

fn font(size: f64, bold: bool) -> Retained<NSFont> {
    let w = unsafe {
        if bold {
            NSFontWeightSemibold
        } else {
            NSFontWeightRegular
        }
    };
    NSFont::systemFontOfSize_weight(size, w)
}

fn line_height(f: &NSFont) -> f64 {
    (f.ascender() - f.descender() + f.leading()).ceil()
}

fn attrs(
    f: &NSFont,
    c: Option<&NSColor>,
    ps: Option<&NSMutableParagraphStyle>,
) -> Retained<NSDictionary<objc2_foundation::NSString, AnyObject>> {
    let mut keys: Vec<&objc2_foundation::NSString> = vec![unsafe { NSFontAttributeName }];
    let mut vals: Vec<&AnyObject> = vec![f.as_ref()];
    if let Some(c) = c {
        keys.push(unsafe { NSForegroundColorAttributeName });
        vals.push(c.as_ref());
    }
    if let Some(ps) = ps {
        keys.push(unsafe { NSParagraphStyleAttributeName });
        vals.push(ps.as_ref());
    }
    NSDictionary::from_slices(&keys, &vals)
}

/// The engine's panels on the main thread.
struct MacSurface {
    mtm: MainThreadMarker,
}

fn panel(key: u64) -> Option<Retained<ToastPanel>> {
    PANELS.with(|p| p.borrow().get(&key).cloned())
}

impl Surface for MacSurface {
    fn work_area(&mut self, active: bool, old: Rect) -> (Rect, f64) {
        let mtm = self.mtm;
        let screens = NSScreen::screens(mtm);
        let mut scr = None;
        if !active {
            // The screen the stack is on, if it is still there.
            let h = primary_height(mtm);
            let c = NSPoint::new(
                (old.l + old.r) as f64 / 2.0,
                h - (old.t + old.b) as f64 / 2.0,
            );
            scr = screens.iter().find(|s| ns_point_in_rect(c, s.frame()));
        }
        if scr.is_none() {
            // The screen with the mouse.
            let m = NSEvent::mouseLocation();
            scr = screens
                .iter()
                .find(|s| ns_point_in_rect(m, s.frame()))
                .or_else(|| NSScreen::mainScreen(mtm))
                .or_else(|| screens.firstObject());
        }
        match scr {
            // visibleFrame leaves out the menu bar and the Dock.
            Some(s) => (top_down(mtm, s.visibleFrame()), 1.0),
            None => (
                Rect {
                    l: 0,
                    t: 0,
                    r: 1280,
                    b: 800,
                },
                1.0,
            ),
        }
    }

    fn body_lines(&mut self, body: &str, width: i32, _scale: f64) -> usize {
        let s = NSString::from_str(body);
        if s.length() == 0 {
            return 0;
        }
        let f = font(BODY_SIZE_DIP, false);
        let r = unsafe {
            s.boundingRectWithSize_options_attributes_context(
                NSSize::new(width as f64, f64::MAX),
                NSStringDrawingOptions::UsesLineFragmentOrigin,
                Some(&attrs(&f, None, None)),
                None,
            )
        };
        if r.size.height > line_height(&f) * 1.5 {
            2
        } else {
            1
        }
    }

    fn label_width(&mut self, label: &str, _scale: f64) -> Option<i32> {
        let f = font(BUTTON_SIZE_DIP, true);
        let size =
            unsafe { NSString::from_str(label).sizeWithAttributes(Some(&attrs(&f, None, None))) };
        Some(size.width.ceil() as i32)
    }

    fn create(&mut self, key: u64, cfg: &Config, w: i32, h: i32) -> bool {
        let mtm = self.mtm;
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w as f64, h as f64));
        let p: Retained<ToastPanel> = unsafe {
            msg_send![
                ToastPanel::alloc(mtm),
                initWithContentRect: rect,
                styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        };
        p.setTitle(&NSString::from_str(&cfg.app_name));
        unsafe { p.setReleasedWhenClosed(false) };
        p.setFloatingPanel(true);
        p.setBecomesKeyOnlyIfNeeded(true);
        // NSPanel's default would hide the cards whenever the app isn't frontmost.
        p.setHidesOnDeactivate(false);
        p.setLevel(NSStatusWindowLevel);
        p.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        p.setOpaque(false);
        p.setBackgroundColor(Some(&NSColor::clearColor()));
        p.setHasShadow(true);
        p.setMovable(false);
        p.setAnimationBehavior(NSWindowAnimationBehavior::None);
        p.setAcceptsMouseMovedEvents(true);
        p.setAlphaValue(0.0); // fades in from nothing
        if cfg.capture_exclude {
            p.setSharingType(NSWindowSharingType::None);
        }
        let v = ToastView::new(mtm, key, rect);
        v.setWantsLayer(true);
        p.setContentView(Some(&v));
        PANELS.with(|m| m.borrow_mut().insert(key, p));
        true
    }

    fn draw(&mut self, look: &Look) {
        let Some(p) = panel(look.key) else { return };
        let Some(v) = p.contentView() else { return };
        let v: &ToastView = unsafe { &*(Retained::as_ptr(&v) as *const ToastView) };
        *v.ivars().look.borrow_mut() = Some(look.clone());
        if let Some(layer) = v.layer() {
            layer.setCornerRadius(look.radius as f64);
            layer.setMasksToBounds(look.radius > 0);
        }
        v.setNeedsDisplay(true);
        if look.hovered {
            let c = if look.hand_cursor() {
                NSCursor::pointingHandCursor()
            } else {
                NSCursor::arrowCursor()
            };
            c.set();
        }
    }

    fn place(&mut self, key: u64, x: i32, y: i32, w: i32, h: i32, raise: bool, _placed: bool) {
        let Some(p) = panel(key) else { return };
        let f = NSRect::new(
            NSPoint::new(x as f64, primary_height(self.mtm) - y as f64 - h as f64),
            NSSize::new(w as f64, h as f64),
        );
        let cur = p.frame();
        if f != cur {
            let resized = f.size != cur.size;
            p.setFrame_display(f, true);
            if resized {
                p.invalidateShadow();
            }
        }
        if raise && p.isVisible() {
            p.orderFrontRegardless();
        }
    }

    fn alpha(&mut self, key: u64, a: u8) {
        if let Some(p) = panel(key) {
            p.setAlphaValue(a as f64 / 255.0);
        }
    }

    fn show(&mut self, key: u64) {
        if let Some(p) = panel(key) {
            p.orderFrontRegardless(); // shows it without activating the app or making it key
            p.invalidateShadow();
        }
    }

    fn destroy(&mut self, key: u64) {
        // orderOut, not close: an app that quits after its last window
        // closes mustn't count a card.
        if let Some(p) = PANELS.with(|m| m.borrow_mut().remove(&key)) {
            p.orderOut(None);
        }
    }

    fn timer(&mut self, on: bool) {
        TICKER.with(|t| {
            let mut t = t.borrow_mut();
            if on && t.is_none() {
                let block = RcBlock::new(|_t: std::ptr::NonNull<NSTimer>| {
                    ENGINE.tick(Instant::now());
                    sync();
                });
                let timer = unsafe {
                    NSTimer::timerWithTimeInterval_repeats_block(
                        TICK_MS as f64 / 1000.0,
                        true,
                        &block,
                    )
                };
                // Common modes: the cards keep counting down while the app
                // tracks a menu or a window resize.
                unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
                *t = Some(timer);
            } else if !on && let Some(timer) = t.take() {
                timer.invalidate();
            }
        });
    }
}

fn ns_point_in_rect(p: NSPoint, r: NSRect) -> bool {
    p.x >= r.origin.x
        && p.x < r.origin.x + r.size.width
        && p.y >= r.origin.y
        && p.y < r.origin.y + r.size.height
}

// ---- the panel and its view ----

define_class!(
    /// Never key or main, so it can't take focus from anything.
    #[unsafe(super(NSPanel, objc2_app_kit::NSWindow, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "IroriToastPanel"]
    struct ToastPanel;

    impl ToastPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            false
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            false
        }
    }
);

struct ViewIvars {
    key: u64,
    look: RefCell<Option<Look>>,
    /// Control-click is the Mac's right click.
    ctrl_down: Cell<bool>,
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "IroriToastView"]
    #[ivars = ViewIvars]
    struct ToastView;

    impl ToastView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _e: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            unsafe {
                let _: () = msg_send![super(self), updateTrackingAreas];
            }
            if self.ivars().tracking.borrow().is_some() {
                return;
            }
            // ActiveAlways: the app that posted the card is usually not the
            // active one.
            let opts = NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::InVisibleRect;
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    NSRect::ZERO,
                    opts,
                    Some(self),
                    None,
                )
            };
            self.addTrackingArea(&area);
            *self.ivars().tracking.borrow_mut() = Some(area);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, e: &NSEvent) {
            self.send(Mouse::Move, e);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, e: &NSEvent) {
            self.send(Mouse::Move, e);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, e: &NSEvent) {
            self.send(Mouse::Leave, e);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            let ctrl = e.modifierFlags().contains(NSEventModifierFlags::Control);
            self.ivars().ctrl_down.set(ctrl);
            if !ctrl {
                self.send(Mouse::Down, e);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, e: &NSEvent) {
            if self.ivars().ctrl_down.replace(false) {
                self.send(Mouse::RightUp, e);
                return;
            }
            self.send(Mouse::Up, e);
        }

        /// No context menu.
        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, _e: &NSEvent) {}

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, e: &NSEvent) {
            self.send(Mouse::RightUp, e);
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            if let Some(l) = self.ivars().look.borrow().as_ref() {
                draw_card(self.bounds(), l);
            }
        }
    }
);

impl ToastView {
    fn new(mtm: MainThreadMarker, key: u64, frame: NSRect) -> Retained<ToastView> {
        let this = ToastView::alloc(mtm).set_ivars(ViewIvars {
            key,
            look: RefCell::new(None),
            ctrl_down: Cell::new(false),
            tracking: RefCell::new(None),
        });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn send(&self, ev: Mouse, e: &NSEvent) {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        if ENGINE.mouse(&mut MacSurface { mtm }, self.ivars().key, ev, x, y) {
            wake(); // queued: the clicked panel isn't removed inside its own event
        }
    }
}

fn symbol_for(kind: Kind) -> &'static str {
    match kind {
        Kind::Success => "checkmark",
        Kind::Warn => "exclamationmark.triangle",
        Kind::Error => "xmark.circle",
        Kind::Info => "info.circle",
    }
}

fn fill_box(r: NSRect, bg: Option<&NSColor>, border: Option<&NSColor>, radius: f64) {
    if let Some(bg) = bg {
        bg.setFill();
        if radius > 0.0 {
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r, radius, radius).fill();
        } else {
            NSRectFill(r);
        }
    }
    if let Some(border) = border {
        let inset = NSRect::new(
            NSPoint::new(r.origin.x + 0.5, r.origin.y + 0.5),
            NSSize::new(r.size.width - 1.0, r.size.height - 1.0),
        );
        let rr = if radius > 0.5 { radius - 0.5 } else { 0.0 };
        let p = if rr > 0.0 {
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(inset, rr, rr)
        } else {
            NSBezierPath::bezierPathWithRect(inset)
        };
        p.setLineWidth(1.0);
        border.setStroke();
        p.stroke();
    }
}

/// s on one line in r, vertically centred, ellipsized at the end.
fn draw_line(s: &str, f: &NSFont, c: &NSColor, r: NSRect, align: NSTextAlignment) {
    let ps = NSMutableParagraphStyle::new();
    ps.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    ps.setAlignment(align);
    let lh = line_height(f);
    let line = NSRect::new(
        NSPoint::new(
            r.origin.x,
            r.origin.y + ((r.size.height - lh) / 2.0).floor(),
        ),
        NSSize::new(r.size.width, lh),
    );
    unsafe {
        NSString::from_str(s).drawWithRect_options_attributes_context(
            line,
            NSStringDrawingOptions::UsesLineFragmentOrigin
                | NSStringDrawingOptions::TruncatesLastVisibleLine,
            Some(&attrs(f, Some(c), Some(&ps))),
            None,
        );
    }
}

fn ns_rect(r: Rect) -> NSRect {
    NSRect::new(
        NSPoint::new(r.l as f64, r.t as f64),
        NSSize::new((r.r - r.l) as f64, (r.b - r.t) as f64),
    )
}

fn draw_card(all: NSRect, l: &Look) {
    let k = l.palette;
    let (w, h) = (all.size.width, all.size.height);
    let accent = rgb(k.accent.for_kind(l.kind));
    let radius = l.radius as f64;

    // The layer's corner mask rounds the card; the stripe sits under the
    // rounded border so the curve stays clean.
    rgb(k.background).setFill();
    NSRectFill(all);
    accent.setFill();
    NSRectFill(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(STRIPE_DIP, h),
    ));
    fill_box(all, None, Some(&rgb(k.border)), radius);

    let top = PAD_TOP_DIP;
    if let Some(sym) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol_for(l.kind)),
        None,
    ) {
        let cfg = NSImageSymbolConfiguration::configurationWithPointSize_weight(
            ICON_SIZE_DIP * 0.9,
            unsafe { objc2_app_kit::NSFontWeightMedium },
        );
        if let Some(sym) = sym.imageWithSymbolConfiguration(&cfg) {
            let s = sym.size();
            let r = NSRect::new(
                NSPoint::new(
                    ICON_LEFT_DIP + ((ICON_SIZE_DIP - s.width) / 2.0).floor(),
                    top + (TITLE_DIP - s.height) / 2.0,
                ),
                s,
            );
            // Tint the template image: draw it, then fill over it with
            // source-atop in the accent (go-toast's drawingHandler).
            let (glyph, tint) = (sym.clone(), accent.clone());
            let handler = RcBlock::new(move |rect: NSRect| {
                glyph.drawInRect(rect);
                tint.set();
                NSRectFillUsingOperation(rect, NSCompositingOperation::SourceAtop);
                Bool::YES
            });
            let tinted = NSImage::imageWithSize_flipped_drawingHandler(s, false, &handler);
            unsafe {
                tinted.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
                    r,
                    NSRect::ZERO,
                    NSCompositingOperation::SourceOver,
                    1.0,
                    true,
                    None,
                )
            };
        }
    }

    draw_line(
        &l.title,
        &font(TITLE_SIZE_DIP, true),
        &rgb(k.foreground),
        NSRect::new(
            NSPoint::new(TEXT_LEFT_DIP, top),
            NSSize::new(w - TITLE_RIGHT_DIP - TEXT_LEFT_DIP, TITLE_DIP),
        ),
        NSTextAlignment::Left,
    );

    if l.lines > 0 && !l.body.is_empty() {
        let f = font(BODY_SIZE_DIP, false);
        let ps = NSMutableParagraphStyle::new();
        ps.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        let bt = top + TITLE_DIP + BODY_GAP_DIP;
        let r = NSRect::new(
            NSPoint::new(TEXT_LEFT_DIP, bt),
            NSSize::new(
                w - TEXT_RIGHT_DIP - TEXT_LEFT_DIP,
                l.lines as f64 * line_height(&f) + 1.0,
            ),
        );
        unsafe {
            NSString::from_str(&l.body).drawWithRect_options_attributes_context(
                r,
                NSStringDrawingOptions::UsesLineFragmentOrigin
                    | NSStringDrawingOptions::TruncatesLastVisibleLine,
                Some(&attrs(&f, Some(&rgb(k.muted)), Some(&ps))),
                None,
            );
        }
    }

    // Inner pieces (the x's hover fill, buttons) get a smaller radius.
    let inner = radius * 0.6;
    let close = ns_rect(l.close);
    let mut xc = rgb(k.faint);
    if l.hot == Part::Close {
        fill_box(close, Some(&rgb(k.hover)), None, inner);
        xc = rgb(k.foreground);
    } else if l.hovered {
        xc = rgb(k.muted);
    }
    let (cx, cy) = (
        close.origin.x + close.size.width / 2.0,
        close.origin.y + close.size.height / 2.0,
    );
    let a = 4.0;
    let x = NSBezierPath::bezierPath();
    x.moveToPoint(NSPoint::new(cx - a, cy - a));
    x.lineToPoint(NSPoint::new(cx + a, cy + a));
    x.moveToPoint(NSPoint::new(cx - a, cy + a));
    x.lineToPoint(NSPoint::new(cx + a, cy - a));
    x.setLineWidth(1.2);
    x.setLineCapStyle(objc2_app_kit::NSLineCapStyle::Round);
    xc.setStroke();
    x.stroke();

    for (i, b) in l.buttons.iter().enumerate().take(2) {
        let Some(label) = l.labels.get(i) else { break };
        let hot = l.hot == Part::Button(i);
        let (bg, fg, border) = if i == 0 {
            // primary
            (
                rgb(if hot { k.primary_hover } else { k.primary }),
                rgb(k.on_primary),
                None,
            )
        } else {
            (
                rgb(if hot { k.hover } else { k.button }),
                rgb(k.foreground),
                Some(rgb(k.border)),
            )
        };
        let r = ns_rect(*b);
        fill_box(r, Some(&bg), border.as_deref(), inner);
        let inset = NSRect::new(
            NSPoint::new(r.origin.x + 4.0, r.origin.y),
            NSSize::new(r.size.width - 8.0, r.size.height),
        );
        draw_line(
            label,
            &font(BUTTON_SIZE_DIP, true),
            &fg,
            inset,
            NSTextAlignment::Center,
        );
    }
}

/// Runs NSApp on the main thread for d, then stops it: for test programs
/// that have no Tauri (go-toast's `Main`, cut down). The app never calls it.
pub fn run_for(d: std::time::Duration) {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    let Some(mtm) = MainThreadMarker::new() else {
        std::thread::sleep(d);
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // An unbundled binary starts Prohibited, which may not show windows.
    // Accessory shows them without a Dock icon or a menu bar.
    if app.activationPolicy() == NSApplicationActivationPolicy::Prohibited {
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
    let when = dispatch2::DispatchTime::try_from(d).unwrap_or(dispatch2::DispatchTime::NOW);
    let _ = DispatchQueue::main().after(when, move || {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        app.stop(None);
        // stop takes effect after the next event; post one so it does now.
        let e = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            objc2_app_kit::NSEventType::ApplicationDefined,
            NSPoint::new(0.0, 0.0),
            NSEventModifierFlags::empty(),
            0.0,
            0,
            None,
            0,
            0,
            0,
        );
        if let Some(e) = e {
            app.postEvent_atStart(&e, true);
        }
    });
    app.run();
}
