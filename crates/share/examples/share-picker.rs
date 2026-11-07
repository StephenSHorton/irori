//! share-picker (Mac, #657): does `SCContentSharingPicker` let an app
//! capture the window the person picks without the persistent Screen
//! Recording grant?
//!
//! It prints whether the process holds the grant, shows the system picker,
//! and when the person picks a window or display, captures it for three
//! seconds with ScreenCaptureKit and prints how many frames came. It never
//! calls `CGRequestScreenCaptureAccess` (moq-video's capture does, before
//! every stream, so the share crate can't test this itself).
//!
//! Run it as its own app so TCC judges this binary, not the terminal:
//! `sh crates/share/grant-test.sh build picker` then
//! `sh crates/share/grant-test.sh run picker`.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("share-picker: the Mac only");
}

#[cfg(target_os = "macos")]
fn main() {
    mac::main();
}

#[cfg(target_os = "macos")]
mod mac {
    use block2::RcBlock;
    use dispatch2::{DispatchQueue, DispatchTime};
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{AnyThread, MainThreadMarker, define_class, msg_send};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_core_media::CMSampleBuffer;
    use objc2_foundation::{NSError, NSObject, NSObjectProtocol};
    use objc2_screen_capture_kit::{
        SCContentFilter, SCContentSharingPicker, SCContentSharingPickerMode,
        SCContentSharingPickerObserver, SCStream, SCStreamConfiguration, SCStreamDelegate,
        SCStreamOutput, SCStreamOutputType,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    fn granted() -> bool {
        objc2_core_graphics::CGPreflightScreenCaptureAccess()
    }

    fn say(s: &str) {
        use std::io::Write;
        println!("{s}");
        let _ = std::io::stdout().flush();
    }

    fn bye(code: i32) -> ! {
        say(&format!("grant at exit: {}", granted()));
        std::process::exit(code)
    }

    pub fn main() {
        let mtm = MainThreadMarker::new().expect("main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        say(&format!(
            "share-picker pid {} grant before picker: {}",
            std::process::id(),
            granted()
        ));

        let observer = Observer::new();
        // SAFETY: the picker is a process singleton; the observer lives to exit.
        unsafe {
            let picker = SCContentSharingPicker::sharedPicker();
            let conf = picker.defaultConfiguration();
            conf.setAllowedPickerModes(
                SCContentSharingPickerMode::SingleWindow
                    | SCContentSharingPickerMode::SingleDisplay,
            );
            picker.setDefaultConfiguration(&conf);
            picker.addObserver(ProtocolObject::from_ref(&*observer));
            picker.setActive(true);
            picker.present();
        }
        say("picker shown: pick a window (or Cancel)");
        std::mem::forget(observer);

        // Give up after two minutes with nobody picking.
        let _ = DispatchQueue::main().after(
            DispatchTime::try_from(Duration::from_secs(120)).expect("time"),
            || {
                say("nobody picked in 120 s");
                bye(3)
            },
        );
        app.run();
    }

    static FRAMES: AtomicU64 = AtomicU64::new(0);

    /// Capture `filter` for three seconds and report.
    fn capture(filter: &SCContentFilter) {
        // SAFETY: plain ScreenCaptureKit calls on objects we own, as moq-video
        // makes them (capture/screencapture.rs).
        unsafe {
            let rect = filter.contentRect();
            say(&format!(
                "picked: style {:?}, {}x{} points, grant now: {}",
                filter.style().0,
                rect.size.width,
                rect.size.height,
                granted()
            ));
            let conf = SCStreamConfiguration::new();
            conf.setWidth(1920);
            conf.setHeight(1080);
            conf.setMinimumFrameInterval(objc2_core_media::CMTime::new(1, 10));
            let output = Output::new();
            let queue = DispatchQueue::new("irori.share-picker", None);
            let stream = SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                filter,
                &conf,
                Some(ProtocolObject::from_ref(&*output)),
            );
            if let Err(e) = stream.addStreamOutput_type_sampleHandlerQueue_error(
                ProtocolObject::from_ref(&*output),
                SCStreamOutputType::Screen,
                Some(&queue),
            ) {
                say(&format!("add output failed: {}", e.localizedDescription()));
                bye(1);
            }
            let started = RcBlock::new(|error: *mut NSError| match error.as_ref() {
                None => say("capture started"),
                Some(e) => {
                    say(&format!(
                        "capture failed to start: {} (code {})",
                        e.localizedDescription(),
                        e.code()
                    ));
                    bye(1);
                }
            });
            stream.startCaptureWithCompletionHandler(Some(&started));
            // Keep the stream running until the process exits (in 3 s).
            std::mem::forget((stream, output, queue));
        }
        let _ = DispatchQueue::main().after(
            DispatchTime::try_from(Duration::from_secs(3)).expect("time"),
            || {
                let n = FRAMES.load(Ordering::SeqCst);
                say(&format!("frames with pictures in 3 s: {n}"));
                say(if n > 0 {
                    "RESULT: captured through the picker"
                } else {
                    "RESULT: no frames through the picker"
                });
                bye(if n > 0 { 0 } else { 1 })
            },
        );
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "IroriSharePickerObserver"]
        struct Observer;

        unsafe impl NSObjectProtocol for Observer {}

        unsafe impl SCContentSharingPickerObserver for Observer {
            #[unsafe(method(contentSharingPicker:didCancelForStream:))]
            unsafe fn did_cancel(
                &self,
                _picker: &SCContentSharingPicker,
                _stream: Option<&SCStream>,
            ) {
                say("picker cancelled");
                bye(2);
            }

            #[unsafe(method(contentSharingPicker:didUpdateWithFilter:forStream:))]
            unsafe fn did_update(
                &self,
                _picker: &SCContentSharingPicker,
                filter: &SCContentFilter,
                _stream: Option<&SCStream>,
            ) {
                say(&format!(
                    "picker answered on the main thread: {}",
                    MainThreadMarker::new().is_some()
                ));
                capture(filter);
            }

            #[unsafe(method(contentSharingPickerStartDidFailWithError:))]
            unsafe fn did_fail(&self, error: &NSError) {
                say(&format!(
                    "picker failed: {} (code {})",
                    error.localizedDescription(),
                    error.code()
                ));
                bye(1);
            }
        }
    );

    impl Observer {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "IroriSharePickerOutput"]
        struct Output;

        unsafe impl NSObjectProtocol for Output {}

        unsafe impl SCStreamDelegate for Output {
            #[unsafe(method(stream:didStopWithError:))]
            unsafe fn did_stop(&self, _stream: &SCStream, error: &NSError) {
                say(&format!(
                    "stream stopped: {} (code {})",
                    error.localizedDescription(),
                    error.code()
                ));
                bye(1);
            }
        }

        unsafe impl SCStreamOutput for Output {
            #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
            unsafe fn did_output(
                &self,
                _stream: &SCStream,
                buffer: &CMSampleBuffer,
                kind: SCStreamOutputType,
            ) {
                // Idle ticks come without a picture; count only real ones.
                if kind.0 == SCStreamOutputType::Screen.0
                    && unsafe { buffer.image_buffer() }.is_some()
                {
                    FRAMES.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    );

    impl Output {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }
}
