//! Shows the settings' test card for a look at it:
//!
//!   IRORI_HOME=<scratch dir> cargo run -p irori-notify --example test_card [seconds]
//!
//! It refuses to run without IRORI_HOME (never the live data dir), keeps
//! the card visible to screen capture so it can be screenshotted, turns
//! the sound off in that scratch home, and exits after the given seconds
//! (default 8).

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
        ..n.options()
    });
    if let Err(e) = err {
        eprintln!("settings: {e}");
    }
    n.set_opener(|t| println!("open {} {}", t.tab, t.ref_));
    if let Err(e) = n.post_test() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    toast::run_for(std::time::Duration::from_secs(secs));
}
