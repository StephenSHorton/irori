//! Shows the settings' test card for a look at it:
//!
//!   IRORI_HOME=<scratch dir> cargo run -p irori-notify --example test_card [seconds] [png]
//!
//! With a PNG path it also shows a finished card with that picture as its
//! thumbnail (card #149).
//!
//! It refuses to run without IRORI_HOME (never the live data dir), keeps
//! the card visible to screen capture so it can be screenshotted, turns
//! the sound off in that scratch home, and exits after the given seconds
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
//! run shows cards on screen restyling live.

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
        capture_visible: true,
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
                std::thread::sleep(std::time::Duration::from_millis(2500));
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

fn read_look(path: &std::path::Path) -> Result<notify::CardLook, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", path.display()))
}
