# toast on macOS: what the Mac must compile and check

`src/darwin.rs` (the AppKit cards) was written on the PC against the objc2
sources in the cargo registry and has never been compiled. Everything else
in the crate (model, layout, sounds, the shared `Engine` the Win32 side
also drives) is platform-neutral and tested on the PC.

## 1. Compile

```sh
cargo build -p irori-toast
cargo clippy -p irori-toast --all-targets -- -D warnings
cargo test -p irori-toast            # model.rs and engine.rs run on any OS
cargo build -p irori-notify --example test_card
```

Expect API-name fixes in `darwin.rs`; the shape is a line-for-line port of
go-toast v0.2.0 `toast_darwin.m` / `toast_darwin.go`. The calls most likely
to need adjusting, each checked only by reading generated sources:

- `define_class!` for `ToastPanel` (super `NSPanel, NSWindow, NSResponder,
  NSObject`) and `ToastView` (super `NSView, NSResponder, NSObject`, ivars
  `ViewIvars`), both `MainThreadOnly`; `msg_send![super(self), updateTrackingAreas]`
  and `msg_send![super(this), initWithFrame: frame]`.
- `msg_send![ToastPanel::alloc(mtm), initWithContentRect:styleMask:backing:defer:]`.
- `NSDictionary::from_slices` for the string attributes, `Retained`-to-`&AnyObject`
  coercions (`f.as_ref()`), and the `NSFontAttributeName` family statics.
- `NSStringNSExtendedStringDrawing::{drawWithRect_options_attributes_context,
  boundingRectWithSize_options_attributes_context}`, `NSStringDrawing::sizeWithAttributes`.
- `NSImage::imageWithSystemSymbolName_accessibilityDescription`,
  `NSImageSymbolConfiguration::configurationWithPointSize_weight`,
  `imageWithSymbolConfiguration`; the icon tint uses `lockFocus` (go-toast
  used `imageWithSize:flipped:drawingHandler:`; either is fine).
- `RcBlock` closures for `NSTimer::timerWithTimeInterval_repeats_block` and
  `NSNotificationCenter::addObserverForName_object_queue_usingBlock`.
- `NSSound::initWithData` + `play` (go-toast used AVAudioPlayer on a serial
  queue; `objc2-av-foundation` is not in Cargo.lock, so NSSound on the main
  queue replaces it; switch if a sound clips or lags).
- `DispatchQueue::main().exec_async` and `after` (dispatch2 0.3.1).
- `darwin::run_for` (behind `toast::run_for`, used by the `test_card`
  example): `NSApplication` with the Accessory policy, `run`, and a timed
  `stop` plus posted `ApplicationDefined` event (go-toast's `Main`).

## 2. Check by hand (on a scratch IRORI_HOME, never the live data dir)

`IRORI_HOME=$(mktemp -d) cargo run -p irori-notify --example test_card 8`
shows the settings' test card. With it up, check what go-toast's
`toast_smoke_darwin_test.go` asserted:

- the card is top-right on the screen with the mouse, inside `visibleFrame`
  (below the menu bar), 340 x 94 pt, dark, blue stripe, `info.circle` icon,
  two-line ellipsized body, Open Board (blue) and Close buttons;
- the frontmost app did not change and Irori did not become active or key
  (`NSWorkspace.frontmostApplication`, `NSApp.isActive`, `keyWindow` nil);
- the panel's level is `NSStatusWindowLevel` (25), it is not key or main, and
  it shows on another Space and over a full-screen app;
- hover brightens the x and pauses expiry; a press dragged off the card is no
  click; Control-click and right click dismiss; a body click with no action
  dismisses; Open Board prints `open board`;
- with `IRORI_TOAST_CAPTURE` unset (the `notify::Notifier` default; the
  example sets `capture_visible`), `screencapture` does not show the card
  (`NSWindowSharingNone`);
- a screen unplugged or a resolution change under a sticky card moves the
  stack (`NSApplicationDidChangeScreenParametersNotification`).

## 3. Port the darwin smoke test

go-toast's `toast_smoke_darwin_test.go` needs an NSApp loop on the main
thread, which `cargo test` doesn't give. Port it as a `harness = false`
test (or an example) that runs `NSApplication` on the main thread, posts
the three smoke cards from another thread, and asserts the list above
through `toast::card_keys()` / `toast::visible()` plus a small test-only
panel-info hook in `darwin.rs` (go-toast's `toast_test_info`).

## 4. In the app

Nothing to call: Tauri runs NSApp on the main thread, and `toast::post`
queues its work on the main dispatch queue. Never start a second NSApp
loop.
