//! The window engine, driven headless: what go-toast's smoke tests check
//! on a real desktop (stacking, replace in place, click actions, fades,
//! hover) against a surface that records what it is told.

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use toast::engine::{Engine, Look, Mouse, Surface};
use toast::layout::{CARD_MARGIN_DIP, CARD_WIDTH_DIP, Part, Rect, px};
use toast::note::{FADE_IN, FADE_OUT, FULL_ALPHA};
use toast::{Action, Config, DARK, Kind, LIGHT, Note, Options, Scheme, hex};

#[derive(Debug, Clone, Default)]
struct Win {
    look: Option<Look>,
    pos: (i32, i32, i32, i32),
    alpha: u8,
    shown: bool,
    raised: u32,
    capture_excluded: bool,
}

#[derive(Default)]
struct Rec {
    work: Rect,
    scale: f64,
    wins: BTreeMap<u64, Win>,
    destroyed: Vec<u64>,
    timer: bool,
    work_calls: Vec<bool>,
    bursts: Vec<(Rect, Rect)>,
    os_dark: bool,
    creates: u32,
}

impl Rec {
    fn new(scale: f64) -> Rec {
        Rec {
            work: Rect {
                l: 0,
                t: 0,
                r: 1920,
                b: 1040,
            },
            scale,
            ..Rec::default()
        }
    }
}

impl Surface for Rec {
    fn work_area(&mut self, active: bool, _old: Rect) -> (Rect, f64) {
        self.work_calls.push(active);
        (self.work, self.scale)
    }
    fn body_lines(&mut self, body: &str, width: i32, scale: f64) -> usize {
        // About 7 dip a character.
        let per = (width as f64 / (7.0 * scale)).max(1.0) as usize;
        body.chars().count().div_ceil(per)
    }
    fn label_width(&mut self, label: &str, scale: f64) -> Option<i32> {
        Some((label.chars().count() as f64 * 7.0 * scale) as i32)
    }
    fn create(&mut self, key: u64, cfg: &Config, _w: i32, _h: i32) -> bool {
        self.creates += 1;
        self.wins.insert(
            key,
            Win {
                capture_excluded: cfg.capture_exclude,
                ..Win::default()
            },
        );
        true
    }
    fn draw(&mut self, look: &Look) {
        self.wins.get_mut(&look.key).unwrap().look = Some(look.clone());
    }
    fn place(&mut self, key: u64, x: i32, y: i32, w: i32, h: i32, raise: bool, _placed: bool) {
        let win = self.wins.get_mut(&key).unwrap();
        win.pos = (x, y, w, h);
        if raise {
            win.raised += 1;
        }
    }
    fn alpha(&mut self, key: u64, a: u8) {
        self.wins.get_mut(&key).unwrap().alpha = a;
    }
    fn show(&mut self, key: u64) {
        self.wins.get_mut(&key).unwrap().shown = true;
    }
    fn destroy(&mut self, key: u64) {
        self.wins.remove(&key);
        self.destroyed.push(key);
    }
    fn timer(&mut self, on: bool) {
        self.timer = on;
    }
    fn confetti(&mut self, card: Rect, work: Rect) {
        self.bursts.push((card, work));
    }
    fn os_dark(&mut self) -> bool {
        self.os_dark
    }
}

fn note(title: &str) -> Note {
    Note {
        title: title.into(),
        ..Note::default()
    }
}

fn engine() -> Engine {
    let e = Engine::new();
    e.configure(Config {
        app_name: "Irori".into(),
        ..Config::default()
    });
    e
}

#[test]
fn cards_stack_replace_and_place_in_the_corner() {
    let e = engine();
    let mut s = Rec::new(1.5);
    let t0 = Instant::now();
    e.post(
        Note {
            id: "save".into(),
            title: "Saving clip…".into(),
            body: "Test · Last 1 min".into(),
            silent: true,
            ..Note::default()
        },
        t0,
    );
    e.sync(&mut s, t0);
    assert_eq!(
        s.work_calls,
        vec![true],
        "a new stack goes to the active screen"
    );
    e.post(note("Clip saved"), t0);
    e.post(
        Note {
            id: "save".into(),
            title: "Copied to clipboard".into(),
            kind: Kind::Success,
            ..Note::default()
        },
        t0,
    );
    e.sync(&mut s, t0);
    assert_eq!(
        s.work_calls,
        vec![true, false],
        "a stack that is up rehomes"
    );
    let keys = e.keys();
    assert_eq!(keys.len(), 2, "same id replaces in place");
    let first = s.wins[&keys[0]].look.clone().unwrap();
    assert_eq!(first.title, "Copied to clipboard");
    assert_eq!(first.lines, 0, "the replace dropped the body");
    assert_eq!(first.h, px(44.0, 1.5));
    // Newest nearest the top-right corner, both at the right margin.
    let w = px(CARD_WIDTH_DIP, 1.5);
    let m = px(CARD_MARGIN_DIP, 1.5);
    let newest = s.wins[&keys[1]].pos;
    let oldest = s.wins[&keys[0]].pos;
    assert_eq!((newest.0, newest.1, newest.2), (1920 - m - w, m, w));
    assert_eq!(oldest.1, m + newest.3 + px(8.0, 1.5));
    assert!(s.wins.values().all(|w| w.shown && w.capture_excluded));
    assert!(s.timer, "timed cards tick");
}

#[test]
fn fades_in_expires_and_destroys() {
    let e = engine();
    let mut s = Rec::new(1.0);
    let t0 = Instant::now();
    e.post(note("x"), t0);
    e.sync(&mut s, t0);
    let k = e.keys()[0];
    assert_eq!(s.wins[&k].alpha, 0, "fades in from nothing");
    e.sync(&mut s, t0 + FADE_IN);
    assert_eq!(s.wins[&k].alpha, FULL_ALPHA);
    let end = t0 + Duration::from_secs(3);
    e.tick(end);
    e.sync(&mut s, end + FADE_OUT / 2);
    assert!(s.wins[&k].alpha < FULL_ALPHA, "fading");
    e.tick(end + FADE_OUT);
    e.sync(&mut s, end + FADE_OUT);
    assert!(s.wins.is_empty() && s.destroyed == vec![k]);
    assert!(!s.timer, "the timer stops with the last card");
    assert!(!e.visible());
}

#[test]
fn sticky_cards_need_no_timer() {
    let e = engine();
    let mut s = Rec::new(1.0);
    let t0 = Instant::now();
    e.post(
        Note {
            sticky: true,
            ..note("held")
        },
        t0,
    );
    e.sync(&mut s, t0);
    assert!(s.timer, "fading in");
    e.sync(&mut s, t0 + FADE_IN);
    assert!(!s.timer, "only sticky cards: no tick");
}

#[test]
fn buttons_and_clicks_run_once_on_their_own_thread() {
    let e = engine();
    let mut s = Rec::new(1.0);
    let t0 = Instant::now();
    let (tx, rx) = mpsc::channel::<String>();
    let (t1, t2, t3) = (tx.clone(), tx.clone(), tx.clone());
    e.post(
        Note {
            title: "Irori v9.9.9 is ready".into(),
            body: "Restart now.".into(),
            sticky: true,
            actions: vec![
                Action::new("Restart", move || t1.send("restart".into()).unwrap()),
                Action::close("Later"),
            ],
            on_click: Some(std::sync::Arc::new(move || t2.send("body".into()).unwrap())),
            on_dismiss: Some(std::sync::Arc::new(move || {
                t3.send("dismiss".into()).unwrap()
            })),
            ..Note::default()
        },
        t0,
    );
    e.sync(&mut s, t0);
    let k = e.keys()[0];
    let look = s.wins[&k].look.clone().unwrap();
    assert_eq!(look.labels, vec!["Restart", "Later"]);
    assert_eq!(look.buttons.len(), 2);
    assert_eq!(look.lines, 1);
    let b = look.buttons[0];
    // Hover, press and release on the primary button.
    assert!(!e.mouse(&mut s, k, Mouse::Move, b.l + 4, b.t + 4));
    let hot = s.wins[&k].look.clone().unwrap();
    assert!(hot.hovered && hot.hot == Part::Button(0) && hot.hand_cursor());
    assert!(!e.mouse(&mut s, k, Mouse::Down, b.l + 4, b.t + 4));
    assert!(
        e.mouse(&mut s, k, Mouse::Up, b.l + 4, b.t + 4),
        "a click asks for a sync"
    );
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "restart");
    e.sync(&mut s, t0);
    assert!(s.wins.is_empty(), "the clicked card went");
    assert!(
        rx.recv_timeout(Duration::from_millis(50)).is_err(),
        "only one outcome"
    );
}

#[test]
fn press_dragged_off_is_no_click_and_right_click_dismisses() {
    let e = engine();
    let mut s = Rec::new(1.0);
    let t0 = Instant::now();
    let (tx, rx) = mpsc::channel::<&str>();
    let t1 = tx.clone();
    e.post(
        Note {
            sticky: true,
            on_dismiss: Some(std::sync::Arc::new(move || t1.send("dismiss").unwrap())),
            ..note("x")
        },
        t0,
    );
    e.sync(&mut s, t0);
    let k = e.keys()[0];
    e.mouse(&mut s, k, Mouse::Down, 50, 20);
    assert!(
        !e.mouse(&mut s, k, Mouse::Up, 50, 400),
        "released off the card"
    );
    // A body click with no on_click is a dismiss.
    e.mouse(&mut s, k, Mouse::Down, 50, 20);
    assert!(e.mouse(&mut s, k, Mouse::Up, 50, 20));
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "dismiss");

    let t2 = tx.clone();
    e.post(
        Note {
            sticky: true,
            on_dismiss: Some(std::sync::Arc::new(move || t2.send("right").unwrap())),
            ..note("y")
        },
        t0,
    );
    e.sync(&mut s, t0);
    let k = e.keys()[0];
    assert!(e.mouse(&mut s, k, Mouse::RightUp, 5, 5));
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "right");
}

#[test]
fn hover_pauses_expiry_and_leave_restarts_it() {
    let e = engine();
    let mut s = Rec::new(1.0);
    let t0 = Instant::now();
    e.post(note("x"), t0);
    e.sync(&mut s, t0);
    let k = e.keys()[0];
    e.mouse(&mut s, k, Mouse::Move, 30, 20);
    let late = t0 + Duration::from_secs(60);
    e.tick(late);
    e.sync(&mut s, late);
    assert!(s.wins.contains_key(&k), "nothing expires while hovered");
    e.mouse(&mut s, k, Mouse::Leave, 0, 0);
    assert!(!s.wins[&k].look.clone().unwrap().hovered);
    e.tick(late + Duration::from_millis(1400));
    e.sync(&mut s, late + Duration::from_millis(1400));
    assert_eq!(
        s.wins[&k].alpha, FULL_ALPHA,
        "at least 1.5 s after the cursor leaves"
    );
    let gone = late + Duration::from_millis(1500);
    e.tick(gone);
    e.tick(gone + FADE_OUT);
    e.sync(&mut s, gone + FADE_OUT);
    assert!(s.wins.is_empty());
}

#[test]
fn close_and_options_off_clear_without_callbacks() {
    let e = engine();
    let mut s = Rec::new(1.0);
    let t0 = Instant::now();
    let (tx, rx) = mpsc::channel::<&str>();
    let t1 = tx.clone();
    e.post(
        Note {
            id: "a".into(),
            sticky: true,
            on_dismiss: Some(std::sync::Arc::new(move || t1.send("ran").unwrap())),
            ..note("a")
        },
        t0,
    );
    e.post(note("b"), t0);
    e.sync(&mut s, t0);
    assert!(e.close("a") && !e.close("a"));
    e.sync(&mut s, t0);
    assert_eq!(e.keys().len(), 1);
    assert!(e.set_options(Options {
        enabled: false,
        ..Options::default()
    }));
    e.sync(&mut s, t0);
    assert!(s.wins.is_empty());
    assert!(
        rx.recv_timeout(Duration::from_millis(50)).is_err(),
        "no callback ran"
    );
    let p = e.post(note("off"), t0);
    assert!(!p.shown && p.sound.is_none(), "off makes a post a no-op");
}

#[test]
fn sounds_follow_kind_buttons_and_settings() {
    use toast::sound::Sound;
    let e = engine();
    let t0 = Instant::now();
    let p = e.post(
        Note {
            kind: Kind::Error,
            ..note("x")
        },
        t0,
    );
    assert_eq!(p.sound, Some((Sound::Fail, 0.6)));
    let p = e.post(
        Note {
            actions: vec![Action::close("Ok")],
            ..note("y")
        },
        t0,
    );
    assert_eq!(p.sound.map(|s| s.0), Some(Sound::Published));
    let p = e.post(
        Note {
            silent: true,
            ..note("z")
        },
        t0,
    );
    assert!(p.shown && p.sound.is_none());
    e.set_options(Options {
        sound: false,
        ..Options::default()
    });
    assert!(e.post(note("w"), t0).sound.is_none());
}

#[test]
fn a_sixth_card_drops_the_oldest_non_sticky_window() {
    let e = engine();
    let mut s = Rec::new(1.0);
    let t0 = Instant::now();
    e.post(
        Note {
            sticky: true,
            ..note("sticky")
        },
        t0,
    );
    for i in 0..5 {
        e.post(note(&format!("n{i}")), t0);
    }
    e.sync(&mut s, t0);
    let titles: Vec<String> = e
        .keys()
        .iter()
        .map(|k| s.wins[k].look.clone().unwrap().title)
        .collect();
    assert_eq!(titles, vec!["sticky", "n1", "n2", "n3", "n4"]);
}

#[test]
fn two_line_bodies_grow_the_card() {
    let e = engine();
    let mut s = Rec::new(1.0);
    let t0 = Instant::now();
    e.post(
        Note {
            body: "word ".repeat(40),
            actions: vec![Action::close("Open")],
            ..note("long")
        },
        t0,
    );
    e.sync(&mut s, t0);
    let l = s.wins[&e.keys()[0]].look.clone().unwrap();
    assert_eq!(l.lines, 2, "clamped to two lines");
    assert_eq!(l.h, 11 + 21 + 12 + 1 + 34 + 36);
}

#[test]
fn confetti_bursts_from_placed_success_cards_only_when_on() {
    let e = engine();
    let mut s = Rec::new(1.0);
    let t0 = Instant::now();
    let ok = |title: &str| Note {
        title: title.into(),
        kind: Kind::Success,
        ..Note::default()
    };

    // Off by default: no burst.
    e.post(ok("done"), t0);
    e.sync(&mut s, t0);
    assert!(s.bursts.is_empty());

    e.set_options(Options {
        confetti: true,
        ..Options::default()
    });
    e.post(note("info"), t0);
    e.post(
        Note {
            kind: Kind::Error,
            ..note("bad")
        },
        t0,
    );
    e.sync(&mut s, t0);
    assert!(s.bursts.is_empty(), "only success cards throw confetti");

    e.post(ok("finished"), t0);
    e.sync(&mut s, t0);
    assert_eq!(s.bursts.len(), 1);
    let key = *e.keys().last().unwrap();
    let (x, y, w, h) = s.wins[&key].pos;
    let (card, work) = s.bursts[0];
    assert_eq!(
        card,
        Rect {
            l: x,
            t: y,
            r: x + w,
            b: y + h
        }
    );
    assert_eq!(work, s.work);

    // Once per post: later syncs don't repeat it.
    e.sync(&mut s, t0 + Duration::from_millis(30));
    assert_eq!(s.bursts.len(), 1);
}

#[test]
fn a_thumbnail_goes_under_the_body_and_grows_the_card() {
    use toast::Image;
    use toast::layout::{TEXT_LEFT_DIP, thumb_size_dip};
    let e = engine();
    let mut s = Rec::new(2.0);
    let t0 = Instant::now();
    let plain = Note {
        id: "plain".into(),
        title: "Finished".into(),
        body: "Ready for you".into(),
        ..Note::default()
    };
    let img = Image::new(800, 400, vec![0u8; 800 * 400 * 4]).unwrap();
    e.post(plain.clone(), t0);
    e.post(
        Note {
            id: "pic".into(),
            image: Some(img.clone()),
            ..plain
        },
        t0,
    );
    e.sync(&mut s, t0);
    let looks: Vec<Look> = s.wins.values().map(|w| w.look.clone().unwrap()).collect();
    let (a, b) = (&looks[0], &looks[1]);
    assert!(a.image.is_none());
    let (got, r) = b.image.clone().unwrap();
    assert_eq!(got, img, "the same pixels, not a copy");
    let (tw, th) = thumb_size_dip(800, 400);
    assert_eq!((r.r - r.l, r.b - r.t), (px(tw, 2.0), px(th, 2.0)));
    assert_eq!(r.l, px(TEXT_LEFT_DIP, 2.0));
    assert_eq!(b.lines, a.lines);
    assert!(r.t > px(11.0 + 21.0, 2.0), "under the title and body");
    assert!(r.b < b.h, "inside the card");
    assert_eq!(b.h - a.h, px(8.0 + th, 2.0), "taller by gap + thumbnail");
}

#[test]
fn a_thumbnail_sits_above_the_buttons() {
    use toast::Image;
    let e = engine();
    let mut s = Rec::new(1.0);
    let t0 = Instant::now();
    e.post(
        Note {
            title: "Finished".into(),
            body: "Ready".into(),
            image: Image::new(100, 100, vec![255u8; 100 * 100 * 4]),
            actions: vec![Action::close("Open"), Action::close("Later")],
            ..Note::default()
        },
        t0,
    );
    e.sync(&mut s, t0);
    let l = s.wins.values().next().unwrap().look.clone().unwrap();
    let (_, r) = l.image.unwrap();
    assert!(r.b <= l.buttons[0].t, "{r:?} over {:?}", l.buttons[0]);
}

// ---- card #527: the cards follow the app's look ----

fn themed(light: toast::Palette) -> Config {
    Config {
        app_name: "Irori".into(),
        light,
        scheme: Scheme::Light,
        corner_radius: 14.0,
        glass: 0.62,
        ..Config::default()
    }
}

#[test]
fn configure_restyles_cards_on_screen() {
    let e = engine();
    let mut s = Rec::new(1.0);
    let t0 = Instant::now();
    e.post(note("Themed"), t0);
    e.sync(&mut s, t0);
    let key = *s.wins.keys().next().unwrap();
    let first = s.wins[&key].look.clone().unwrap();
    assert_eq!(first.palette, DARK.resolved());
    assert!(first.dark && first.glass.is_none() && first.radius == 0);

    let mut light = LIGHT;
    light.background = hex("#f1f2f6");
    light.tint = hex("#fcfcff");
    e.configure(themed(light));
    e.sync(&mut s, t0);
    assert_eq!(s.creates, 1, "restyled in place, not remade");
    let l = s.wins[&key].look.clone().unwrap();
    assert!(!l.dark, "light now");
    assert_eq!(l.palette.background, hex("#f1f2f6"));
    assert_eq!(l.palette.tint, hex("#fcfcff"));
    assert_eq!(l.radius, 14);
    assert_eq!(l.glass, Some(0.62));
    assert_eq!((l.title.as_str(), l.w, l.h), ("Themed", first.w, first.h));

    // The same config again changes nothing.
    s.wins.get_mut(&key).unwrap().look = None;
    assert!(!e.configure(themed(light)), "unchanged");
    e.sync(&mut s, t0);
    assert!(
        s.wins[&key].look.is_none(),
        "no redraw for an unchanged look"
    );
}

#[test]
fn system_scheme_follows_the_os() {
    let e = engine();
    e.configure(Config {
        app_name: "Irori".into(),
        scheme: Scheme::System,
        ..Config::default()
    });
    let mut s = Rec::new(1.0);
    s.os_dark = false;
    let t0 = Instant::now();
    e.post(note("Follows"), t0);
    e.sync(&mut s, t0);
    let key = *s.wins.keys().next().unwrap();
    assert!(!s.wins[&key].look.as_ref().unwrap().dark);
    assert_eq!(
        s.wins[&key].look.as_ref().unwrap().palette,
        LIGHT.resolved()
    );
    s.os_dark = true; // the OS flips; the platform syncs
    e.sync(&mut s, t0);
    let l = s.wins[&key].look.clone().unwrap();
    assert!(l.dark);
    assert_eq!(l.palette, DARK.resolved());
    assert_eq!(s.creates, 1);
}
