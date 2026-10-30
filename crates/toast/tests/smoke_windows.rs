//! go-toast's toast_smoke_windows_test.go: real cards on the desktop, a
//! click on one, and focus never moving. Opt-in (IRORI_TOAST_SMOKE=1): it
//! draws on the screen. Sounds are off, so it plays nothing.
#![cfg(windows)]

use std::sync::mpsc;
use std::time::Duration;

use toast::layout::{CARD_WIDTH_DIP, button_rects, px};
use toast::win32::{
    MSG_LBUTTONDOWN, MSG_LBUTTONUP, MSG_MOUSEMOVE, card_windows, client_size, foreground,
    post_mouse, set_dpi_aware,
};
use toast::{Action, Config, Kind, Note, Options};

fn windows_soon(want: usize) -> Vec<isize> {
    let mut got = vec![];
    for _ in 0..40 {
        got = card_windows();
        if got.len() == want {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    got
}

#[test]
fn toast_smoke() {
    if std::env::var("IRORI_TOAST_SMOKE").as_deref() != Ok("1") {
        eprintln!("skipped: set IRORI_TOAST_SMOKE=1");
        return;
    }
    set_dpi_aware();
    toast::configure(Config {
        app_name: "Irori smoke".into(),
        ..Config::default()
    });
    toast::set_options(Options {
        sound: false,
        ..Options::default()
    });
    let before = foreground();
    let focus = || assert_eq!(foreground(), before, "a card stole focus");

    toast::post(Note {
        id: "smoke-save".into(),
        title: "Saving clip…".into(),
        body: "Test · Last 1 min".into(),
        silent: true,
        ..Note::default()
    });
    std::thread::sleep(Duration::from_millis(700));
    toast::show("Clip saved", "Test · Last 1 min", true);
    toast::post(Note {
        id: "smoke-save".into(),
        title: "Copied to clipboard".into(),
        kind: Kind::Success,
        ..Note::default()
    });
    assert_eq!(windows_soon(2).len(), 2, "same id replaces");

    let (tx, rx) = mpsc::channel();
    toast::post(Note {
        title: "Irori v9.9.9 is ready".into(),
        body: "Restart now, or it installs when you quit.".into(),
        sticky: true,
        actions: vec![
            Action::new("Restart", move || tx.send("restart").unwrap()),
            Action::close("Later"),
        ],
        ..Note::default()
    });
    let cards = windows_soon(3);
    assert_eq!(cards.len(), 3);
    std::thread::sleep(Duration::from_millis(400));
    focus();

    // Click the primary button, as the person would. Real labels are at
    // least as wide as these, which only pushes the first button left, so
    // a point just inside this one's left edge is still on it.
    let newest = *cards.last().unwrap();
    let (w, h) = client_size(newest);
    let sc = w as f64 / CARD_WIDTH_DIP;
    let b = button_rects(w, h, &[px(30.0, sc), px(30.0, sc)], sc)[0];
    post_mouse(newest, MSG_MOUSEMOVE, b.l + 4, b.t + 4);
    post_mouse(newest, MSG_LBUTTONDOWN, b.l + 4, b.t + 4);
    post_mouse(newest, MSG_LBUTTONUP, b.l + 4, b.t + 4);
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok("restart"));
    focus();
    assert_eq!(windows_soon(2).len(), 2, "the clicked card went");

    toast::close("smoke-save");
    for _ in 0..70 {
        if !toast::visible() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        focus();
    }
    assert!(!toast::visible(), "cards should have faded out");
}
