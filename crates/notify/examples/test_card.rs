//! Shows the settings' test card for a look at it:
//!
//!   IRORI_HOME=<scratch dir> cargo run -p irori-notify --example test_card [seconds] [png]
//!
//! With a PNG path it also shows a finished card with that picture as its
//! thumbnail (card #149).
//!
//! It refuses to run without IRORI_HOME (never the live data dir), keeps
//! the card visible to screen capture so it can be screenshotted (unless
//! IRORI_CARD_HIDDEN is set, to check it stays out of capture as in the
//! app), turns the sound off in that scratch home, and exits after the given seconds
//! (default 8). With `kinds` after the seconds it shows one card of each
//! kind instead (info, success, warn, error; card #175).
//! IRORI_CONFETTI=1 turns "Confetti on good news" on, so a success card
//! throws its burst.
//!
//! The cards' look (card #527): IRORI_CARD_LOOK=<file.json> applies that
//! `notify::CardLook` (what the window sends, camelCase) before posting.
//! With `cycle` after the seconds and IRORI_CARD_LOOK_DIR=<dir>, it posts
//! a sticky copy of the test card and then every 2.5 s applies the next
//! `*.json` look in the dir (sorted, wrapping), printing each name, so one
//! run shows cards on screen restyling live. IRORI_CARD_SHOTS=<dir> also
//! saves a picture after each look (`<look>.png`, or `card.png` for the
//! plain test card): the screen's top-right corner on macOS, the cards and
//! what is around them on Windows.

fn main() {
    let Some(home) = std::env::var_os("IRORI_HOME") else {
        eprintln!("set IRORI_HOME to a scratch dir");
        std::process::exit(2);
    };
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    #[cfg(windows)]
    toast::win32::set_dpi_aware();
    let n = notify::Notifier::new(notify::Config {
        dir: Some(home.into()),
        capture_visible: std::env::var_os("IRORI_CARD_HIDDEN").is_none(),
        ..notify::Config::default()
    });
    let (_, err) = n.set_options(notify::Options {
        sound: false,
        confetti: std::env::var("IRORI_CONFETTI").as_deref() == Ok("1"),
        ..n.options()
    });
    if let Err(e) = err {
        eprintln!("settings: {e}");
    }
    if let Some(path) = std::env::var_os("IRORI_CARD_LOOK") {
        match read_look(std::path::Path::new(&path)) {
            Ok(l) => {
                if let Err(e) = n.set_look(l) {
                    eprintln!("look: {e}");
                }
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
    n.set_opener(|t| println!("open {} {}", t.tab, t.ref_));
    if std::env::args().nth(2).as_deref() == Some("cycle") {
        let Some(dir) = std::env::var_os("IRORI_CARD_LOOK_DIR") else {
            eprintln!("cycle: set IRORI_CARD_LOOK_DIR to a dir of CardLook JSON files");
            std::process::exit(2);
        };
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "json"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        if files.is_empty() {
            eprintln!(
                "cycle: no *.json in {}",
                std::path::Path::new(&dir).display()
            );
            std::process::exit(1);
        }
        // Sticky, so it stays up while the looks change under it.
        n.post(notify::Note {
            id: "test-cycle".into(),
            title: "Irori notifications are on".into(),
            body: "Cards like this show what needs you: held messages, blocked cards, and agents asking you to choose.".into(),
            sticky: true,
            actions: vec![
                notify::Action::close("Open Board"),
                notify::Action::close("Close"),
            ],
            ..notify::Note::default()
        });
        let n = n.clone();
        std::thread::spawn(move || {
            for f in files.iter().cycle() {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                let name = f.file_name().unwrap_or_default().to_string_lossy();
                match read_look(f) {
                    Ok(l) => {
                        println!("look {name}");
                        if let Err(e) = n.set_look(l) {
                            eprintln!("{name}: {e}");
                        }
                    }
                    Err(e) => eprintln!("{e}"),
                }
                std::thread::sleep(std::time::Duration::from_millis(1000));
                shot(name.trim_end_matches(".json"));
            }
        });
    } else if std::env::args().nth(2).as_deref() == Some("kinds") {
        use notify::{Level, Note};
        for (i, (level, title)) in [
            (Level::Error, "Error: the build failed"),
            (Level::Warn, "Warn: a card is blocked on you"),
            (Level::Success, "Success: a card is done"),
            (Level::Info, "Info: a message Irori held"),
        ]
        .into_iter()
        .enumerate()
        {
            let shown = n.post(Note {
                id: format!("kinds-{i}"),
                title: title.into(),
                body: "One card of each kind, to compare their stripes and icons.".into(),
                level,
                sticky: true,
                ..Note::default()
            });
            println!("{title}: shown {shown}");
        }
    } else if let Err(e) = n.post_test() {
        eprintln!("{e}");
        std::process::exit(1);
    } else {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            shot("card");
        });
    }
    if let Some(path) = std::env::args()
        .nth(2)
        .filter(|a| a != "kinds" && a != "cycle")
    {
        let Some(img) = notify::thumbnail(std::path::Path::new(&path)) else {
            eprintln!("{path}: not a PNG this can read");
            std::process::exit(1);
        };
        n.post(notify::Note {
            id: "test-thumb".into(),
            title: "ember finished “Card thumbnails” (#149)".into(),
            body: "Toast cards show the proof's first screenshot · 1 screenshot".into(),
            image: Some(img),
            level: notify::Level::Success,
            actions: vec![
                notify::Action::close("Open"),
                notify::Action::close("Mark Done"),
            ],
            ..notify::Note::default()
        });
    }
    toast::run_for(std::time::Duration::from_secs(secs));
}

/// IRORI_CARD_SHOTS=<dir>: a picture of the cards. On macOS the top-right
/// corner of the main display, taken from inside this process: a process
/// may always capture its own windows, so this works where the terminal
/// has no Screen Recording permission (other apps' windows then show as
/// the desktop, which is what the glass blurs). On Windows the cards plus a
/// margin, wherever they landed. Elsewhere it does nothing.
fn shot(name: &str) {
    let Some(dir) = std::env::var_os("IRORI_CARD_SHOTS") else {
        return;
    };
    let path = std::path::Path::new(&dir).join(format!("{name}.png"));
    #[cfg(target_os = "macos")]
    match mac_shot::grab(&path, 460.0, 220.0) {
        Ok(()) => println!("shot {}", path.display()),
        Err(e) => eprintln!("shot {name}: {e}"),
    }
    #[cfg(windows)]
    match win_shot(&path) {
        Ok(()) => println!("shot {}", path.display()),
        Err(e) => eprintln!("shot {name}: {e}"),
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = path;
}

#[cfg(windows)]
fn win_shot(path: &std::path::Path) -> Result<(), String> {
    let (w, h, rgba) = toast::win32::capture_cards(40).ok_or("no card on screen")?;
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut out = enc.write_header().map_err(|e| e.to_string())?;
    out.write_image_data(&rgba).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
mod mac_shot {
    use std::ffi::c_void;
    use std::os::unix::ffi::OsStrExt;

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
        fn CGMainDisplayID() -> u32;
        fn CGDisplayBounds(d: u32) -> CGRect;
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

    /// The w x h pt top-right corner of the main display, as a PNG.
    pub fn grab(path: &std::path::Path, w: f64, h: f64) -> Result<(), String> {
        unsafe {
            let b = CGDisplayBounds(CGMainDisplayID());
            let r = CGRect {
                x: b.x + b.w - w,
                y: b.y,
                w,
                h,
            };
            // kCGWindowListOptionOnScreenOnly, kCGNullWindowID, default.
            let img = CGWindowListCreateImage(r, 1, 0, 0);
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
            CFRelease(ty);
            CFRelease(url);
            CGImageRelease(img);
            if ok {
                Ok(())
            } else {
                Err("can't write the PNG".into())
            }
        }
    }
}

fn read_look(path: &std::path::Path) -> Result<notify::CardLook, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", path.display()))
}
