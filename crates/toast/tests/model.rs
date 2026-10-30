//! go-toast's note_test.go and config_test.go.

use std::time::{Duration, Instant};

use toast::config::{class_prefix_for, contrast, mix};
use toast::layout::*;
use toast::note::*;
use toast::sound::*;
use toast::{Config, DARK, Kind, LIGHT, Palette, hex, rgb};

fn t0() -> Instant {
    Instant::now()
}

fn titled(t: &str) -> Note {
    Note {
        title: t.into(),
        ..Note::default()
    }
}

#[test]
fn same_id_replaces_in_place() {
    let t0 = t0();
    let mut s = Stack::default();
    s.post(
        Note {
            id: "save".into(),
            title: "Saving clip…".into(),
            silent: true,
            ..Note::default()
        },
        t0,
    );
    s.post(titled("Other"), t0);
    let n = Note {
        id: "save".into(),
        title: "Clip saved".into(),
        ..Note::default()
    };
    let life = default_lifetime(&n);
    let (key, dropped) = s.post(n, t0 + Duration::from_secs(1));
    assert!(dropped.is_none() && s.cards.len() == 2, "replace stacked");
    let c = &s.cards[0];
    assert!(c.key == key && c.note.title == "Clip saved" && c.rev == 1 && c.born == t0);
    assert_eq!(
        c.until,
        Some(t0 + Duration::from_secs(1) + life),
        "replace restarts the timer"
    );
}

#[test]
fn full_stack_drops_oldest_non_sticky() {
    let t0 = t0();
    let mut s = Stack::default();
    s.post(
        Note {
            sticky: true,
            ..titled("sticky")
        },
        t0,
    );
    for _ in 0..MAX_CARDS - 1 {
        s.post(titled("n"), t0);
    }
    let second = s.cards[1].key;
    let (_, dropped) = s.post(titled("new"), t0);
    assert_eq!(dropped.map(|c| c.key), Some(second));
    assert_eq!(s.cards.len(), MAX_CARDS);
    assert_eq!(s.cards[0].note.title, "sticky");
    assert_eq!(s.cards[MAX_CARDS - 1].note.title, "new", "newest last");
}

#[test]
fn tick_expires_then_fades() {
    let t0 = t0();
    let mut s = Stack::default();
    s.post(titled("x"), t0);
    let life = default_lifetime(&s.cards[0].note);
    assert!(
        s.tick(t0 + life - Duration::from_millis(1), false)
            .is_empty()
    );
    assert!(s.cards[0].closing.is_none(), "expired early");
    let at = t0 + life;
    s.tick(at, false);
    assert!(s.cards[0].closing.is_some());
    assert!(
        s.cards[0].alpha(at + FADE_OUT / 2) < FULL_ALPHA,
        "fading after its lifetime"
    );
    let gone = s.tick(at + FADE_OUT, false);
    assert!(
        gone.len() == 1 && s.cards.is_empty(),
        "removed after the fade"
    );
}

#[test]
fn hover_pauses_and_revives() {
    let t0 = t0();
    let mut s = Stack::default();
    s.post(titled("x"), t0);
    let end = t0 + default_lifetime(&s.cards[0].note);
    s.tick(end, false);
    s.tick(end + FADE_OUT / 2, true);
    assert!(
        s.cards[0].closing.is_none(),
        "hover brings a fading card back"
    );
    let later = end + Duration::from_secs(60);
    assert!(
        s.tick(later, true).is_empty(),
        "nothing expires while hovered"
    );
    assert!(s.cards[0].until.unwrap() - later >= AFTER_HOVER);
}

#[test]
fn sticky_never_expires() {
    let t0 = t0();
    let mut s = Stack::default();
    s.post(
        Note {
            sticky: true,
            expire: Duration::from_secs(1),
            ..titled("Update ready")
        },
        t0,
    );
    assert!(s.tick(t0 + Duration::from_secs(3600), false).is_empty());
    assert!(s.cards[0].closing.is_none());
}

#[test]
fn lifetimes() {
    let act = vec![Action::close("Restart")];
    let cases: Vec<(Note, Duration)> = vec![
        (
            Note {
                kind: Kind::Success,
                ..Note::default()
            },
            Duration::from_secs(3),
        ),
        (
            Note {
                kind: Kind::Error,
                ..Note::default()
            },
            Duration::from_secs(6),
        ),
        (
            Note {
                kind: Kind::Warn,
                ..Note::default()
            },
            Duration::from_secs(6),
        ),
        (
            Note {
                actions: act.clone(),
                ..Note::default()
            },
            Duration::from_secs(10),
        ),
        (
            Note {
                expire: Duration::from_millis(1500),
                actions: act,
                ..Note::default()
            },
            Duration::from_millis(1500),
        ),
        (
            Note {
                sticky: true,
                expire: Duration::from_secs(1),
                ..Note::default()
            },
            Duration::ZERO,
        ),
    ];
    for (n, want) in cases {
        assert_eq!(default_lifetime(&n), want, "{n:?}");
    }
}

#[test]
fn clean_and_close() {
    assert!(
        clean(Note {
            title: "  ".into(),
            body: " ".into(),
            ..Note::default()
        })
        .is_none()
    );
    let n = clean(Note {
        body: "Copied".into(),
        actions: vec![Action::close("a"), Action::close("b"), Action::close("c")],
        ..Note::default()
    })
    .unwrap();
    assert!(n.title == "Copied" && n.body.is_empty() && n.actions.len() == MAX_ACTIONS);
    let mut s = Stack::default();
    s.post(
        Note {
            id: "a".into(),
            ..titled("a")
        },
        Instant::now(),
    );
    assert!(!s.close_id(""));
    assert!(s.close_id("a") && s.cards.is_empty());
    assert!(!s.close_id("a"));
}

#[test]
fn alpha_fades_in() {
    let t0 = t0();
    let mut s = Stack::default();
    s.post(titled("x"), t0);
    assert_eq!(s.cards[0].alpha(t0), 0);
    assert_eq!(s.cards[0].alpha(t0 + FADE_IN), FULL_ALPHA);
}

#[test]
fn options_normalized() {
    let o = Options {
        volume: 3.0,
        corner: "middle".into(),
        ..Options::default()
    }
    .normalized();
    assert!(o.volume == 1.0 && o.corner == CORNER_TOP_RIGHT);
    let o = Options {
        volume: f64::NAN,
        corner: CORNER_BOTTOM_LEFT.into(),
        ..Options::default()
    }
    .normalized();
    assert!(o.volume == Options::default().volume && o.corner == CORNER_BOTTOM_LEFT);
    let o = Options {
        volume: -1.0,
        ..Options::default()
    }
    .normalized();
    assert_eq!(o.volume, 0.0);
}

#[test]
fn stack_origins_newest_nearest_corner() {
    let work = Rect {
        l: 0,
        t: 40,
        r: 1000,
        b: 800,
    };
    let heights = [60, 80]; // oldest, newest
    let (w, gap, margin) = (300, 8, 20);
    let p = |x, y| Point { x, y };
    for (corner, old, newest) in [
        (CORNER_TOP_RIGHT, p(680, 40 + 20 + 80 + 8), p(680, 60)),
        (CORNER_TOP_LEFT, p(20, 40 + 20 + 80 + 8), p(20, 60)),
        (
            CORNER_BOTTOM_RIGHT,
            p(680, 800 - 20 - 80 - 8 - 60),
            p(680, 800 - 20 - 80),
        ),
        (
            CORNER_BOTTOM_LEFT,
            p(20, 800 - 20 - 80 - 8 - 60),
            p(20, 800 - 20 - 80),
        ),
    ] {
        let got = stack_origins(work, corner, w, &heights, gap, margin);
        assert_eq!(got, vec![old, newest], "{corner}");
    }
}

#[test]
fn card_heights() {
    assert_eq!(card_height_dip(1, false), 62.0, "Toru's original 62");
    assert_eq!(card_height_dip(0, false), 44.0);
    assert_eq!(
        card_height_dip(5, false),
        card_height_dip(MAX_BODY_LINES, false)
    );
    assert!(card_height_dip(0, true) > card_height_dip(0, false));
    assert_eq!(
        card_height_dip(2, true),
        11.0 + 21.0 + 12.0 + 1.0 + 34.0 + 36.0
    );
}

#[test]
fn hit_parts() {
    let (w, h, sc) = (340, 96, 1.0);
    let cl = close_rect(w, sc);
    let btns = button_rects(w, h, &[40, 30], sc);
    assert!(btns.len() == 2 && btns[0].r <= btns[1].l);
    assert_eq!(btns[1].r, w - BUTTONS_RIGHT_DIP as i32);
    assert_eq!(btns[0].b, h - PAD_BOTTOM_DIP as i32);
    assert_eq!(
        btns[0].r - btns[0].l,
        BUTTON_MIN_DIP as i32,
        "minimum width"
    );
    for (x, y, want) in [
        (cl.l + 2, cl.t + 2, Part::Close),
        (btns[0].l + 1, btns[0].t + 1, Part::Button(0)),
        (btns[1].r - 1, btns[1].b - 1, Part::Button(1)),
        (60, 30, Part::Body),
    ] {
        assert_eq!(hit_part(x, y, cl, &btns), want, "hit({x},{y})");
    }
    assert!(button_rects(w, h, &[], sc).is_empty());
}

#[test]
fn sound_mapping() {
    for (k, actionable, want) in [
        (Kind::Success, false, Sound::Success),
        (Kind::Info, false, Sound::Success),
        (Kind::Error, false, Sound::Fail),
        (Kind::Warn, false, Sound::Published),
        (Kind::Success, true, Sound::Published),
        (Kind::Error, true, Sound::Published),
    ] {
        assert_eq!(sound_for(k, actionable), want, "{k:?} {actionable}");
    }
}

#[test]
fn embedded_sounds_are_pcm16() {
    for (name, wav) in [
        ("success", SUCCESS_WAV),
        ("fail", FAIL_WAV),
        ("published", PUBLISHED_WAV),
    ] {
        let (off, n) = pcm_data(wav).unwrap_or_else(|| panic!("{name}: not 16-bit PCM"));
        assert!(n >= 1000 && off + n <= wav.len(), "{name}");
    }
    assert!(pcm_data(b"RIFF\x00\x00\x00\x00WAVE").is_none());
}

#[test]
fn scale_wav_halves_samples() {
    assert_eq!(&scale_wav(SUCCESS_WAV, 1.0)[..], SUCCESS_WAV, "full volume");
    let half = scale_wav(SUCCESS_WAV, 0.5);
    let (off, n) = pcm_data(SUCCESS_WAV).unwrap();
    assert_eq!(half.len(), SUCCESS_WAV.len());
    assert_eq!(&half[..off], &SUCCESS_WAV[..off], "keeps the header");
    let mut i = off;
    while i + 1 < off + n {
        let src = i16::from_le_bytes([SUCCESS_WAV[i], SUCCESS_WAV[i + 1]]) as f64;
        let got = i16::from_le_bytes([half[i], half[i + 1]]) as f64;
        assert!((got - src / 2.0).abs() <= 1.0, "sample at {i}");
        i += 2;
    }
    assert!(
        sound_bytes(Sound::Fail, 0.0).is_none(),
        "volume 0 is silence"
    );
    let a = sound_bytes(Sound::Fail, 0.6).unwrap();
    let b = sound_bytes(Sound::Fail, 0.6).unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&a, &b),
        "same volume reuses the cache"
    );
}

// ---- config_test.go ----

#[test]
fn default_config() {
    let c = Config::default();
    assert!(
        c.app_name == "Toast" && c.palette == DARK && c.corner_radius == 0.0 && c.capture_exclude
    );
    let n = c.normalized();
    assert_eq!(n.class_prefix, "Toast");
    assert_eq!(n.host_class_name(), "ToastHost");
    assert_eq!(n.log_prefix(), "toast: ");
}

#[test]
fn irori_and_toru_names() {
    let c = Config {
        app_name: "Toru".into(),
        ..Config::default()
    }
    .normalized();
    assert_eq!(c.card_class_name(), "ToruToast");
    assert_eq!(c.host_class_name(), "ToruToastHost");
    assert_eq!(c.log_prefix(), "toru: toast: ");
    let c = Config {
        app_name: "Irori".into(),
        ..Config::default()
    }
    .normalized();
    assert_eq!(
        (c.card_class_name(), c.host_class_name()),
        ("IroriToast".into(), "IroriToastHost".into()),
        "inbox.md §4"
    );
    let p = Config {
        app_name: "My App 2!".into(),
        class_prefix: "Custom".into(),
        ..Config::default()
    }
    .normalized();
    assert_eq!(p.class_prefix, "Custom");
    assert_eq!(class_prefix_for("My App 2!"), "MyApp2Toast");
    let p = Config {
        app_name: "  ".into(),
        ..Config::default()
    }
    .normalized();
    assert!(p.app_name == "Toast" && p.class_prefix == "Toast");
}

#[test]
fn config_normalized() {
    for (inp, want) in [(-3.0, 0.0), (f64::NAN, 0.0), (10.0, 10.0), (100.0, 24.0)] {
        let got = Config {
            corner_radius: inp,
            ..Config::default()
        }
        .normalized()
        .corner_radius;
        assert_eq!(got, want, "radius {inp}");
    }
    let zero = Config {
        palette: Palette::default(),
        ..Config::default()
    };
    assert_eq!(zero.normalized().palette, DARK, "zero palette is Dark");
}

#[test]
fn color() {
    assert!(toast::Color(0).is_zero() && !rgb(0, 0, 0).is_zero());
    assert_eq!(hex("#4cabfd"), rgb(0x4c, 0xab, 0xfd));
    assert_eq!(hex("4CABFD"), hex("#4cabfd"));
    assert_eq!(
        hex("#4cabfd").colorref(),
        0x00fd_ab4c,
        "COLORREF is 0x00BBGGRR"
    );
    assert_eq!(hex("#0f172a").to_string(), "#0f172a");
    assert_eq!(toast::Color(0).to_string(), "unset");
    for bad in ["", "#fff", "#gggggg", "#1234567", "+12345"] {
        assert!(
            std::panic::catch_unwind(|| hex(bad)).is_err(),
            "hex({bad:?})"
        );
    }
}

#[test]
fn dark_is_toru() {
    let d = DARK.resolved();
    for (name, got, want) in [
        ("card", d.background, rgb(0x11, 0x12, 0x13)),
        ("border", d.border, rgb(0x2c, 0x2e, 0x31)),
        ("title", d.foreground, rgb(0xfa, 0xfa, 0xfa)),
        ("body", d.muted, rgb(0x9e, 0xa1, 0xa7)),
        ("faint", d.faint, rgb(0x5c, 0x5f, 0x66)),
        ("hot", d.hover, rgb(0x26, 0x29, 0x2e)),
        ("button", d.button, rgb(0x20, 0x22, 0x25)),
        ("primary", d.primary, rgb(0x4c, 0xab, 0xfd)),
        ("primaryHot", d.primary_hover, rgb(0x6f, 0xbc, 0xff)),
        ("onPrimary", d.on_primary, rgb(0x06, 0x0a, 0x13)),
        ("info", d.accent.for_kind(Kind::Info), rgb(0x4c, 0xab, 0xfd)),
        (
            "success",
            d.accent.for_kind(Kind::Success),
            rgb(0x4c, 0xab, 0xfd),
        ),
        ("warn", d.accent.for_kind(Kind::Warn), rgb(0xf5, 0xae, 0x39)),
        (
            "error",
            d.accent.for_kind(Kind::Error),
            rgb(0xea, 0x3c, 0x3f),
        ),
    ] {
        assert_eq!(got, want, "{name}");
    }
    assert_eq!(d, DARK, "Dark needs no derivation");
}

#[test]
fn light_preset() {
    let l = LIGHT.resolved();
    assert_eq!(l.background, hex("#ffffff"));
    assert_eq!(l.foreground, hex("#0f172a"));
    assert_eq!(l.muted, hex("#64748b"));
    assert_eq!(l.border, rgb(203, 213, 225));
    assert_eq!(l.accent.info, hex("#2563eb"));
    assert_eq!(l.accent.success, hex("#15803d"));
    assert_eq!(l.accent.warn, hex("#c2620a"));
    assert_eq!(l.accent.error, hex("#ef4444"));
    assert_eq!(l.primary, hex("#2563eb"));
    assert_eq!(l.on_primary, hex("#ffffff"));
    for (name, c) in [
        ("faint", l.faint),
        ("hover", l.hover),
        ("button", l.button),
        ("primaryHover", l.primary_hover),
    ] {
        assert!(!c.is_zero() && c != l.background, "{name} not derived: {c}");
    }
    assert!(contrast(l.hover, l.background) < contrast(l.foreground, l.background));
    assert!(contrast(l.on_primary, l.primary) >= 4.5);
    // The derivations by hand: 8% and 5% of the way to the foreground.
    assert_eq!(l.hover, mix(l.background, l.foreground, 0.08));
    assert_eq!(l.faint, mix(l.muted, l.background, 0.4));
}

#[test]
fn partial_palette_fills_from_dark() {
    let p = Palette {
        background: rgb(0, 0, 0),
        accent: toast::Accents {
            error: hex("#ff0000"),
            ..Default::default()
        },
        ..Palette::default()
    }
    .resolved();
    assert!(p.background == rgb(0, 0, 0) && p.foreground == DARK.foreground);
    assert!(p.accent.error == hex("#ff0000") && p.accent.warn == DARK.accent.warn);
    assert!(p.primary == DARK.accent.info && !p.on_primary.is_zero());
}
