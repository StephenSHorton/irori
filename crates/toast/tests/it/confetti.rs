//! The confetti simulation and its software fill.

use std::time::Duration;

use toast::confetti::{Burst, DURATION, PIECES, rasterize};
use toast::layout::Rect;

const AREA: Rect = Rect {
    l: 0,
    t: 0,
    r: 1920,
    b: 1080,
};
// A top-right card.
const CARD: Rect = Rect {
    l: 1540,
    t: 20,
    r: 1900,
    b: 110,
};

#[test]
fn a_burst_flies_from_the_card_toward_the_screen_and_ends() {
    let b = Burst::new(CARD, AREA, 1.0, 7);
    let start = b.at(Duration::ZERO);
    assert_eq!(start.len(), PIECES);
    for p in &start {
        assert!(p.x >= 1540.0 && p.x <= 1900.0 && p.y >= 20.0 && p.y <= 110.0);
        assert_eq!(p.alpha, 1.0);
    }
    let mid = b.at(Duration::from_millis(900));
    let mean_x = mid.iter().map(|p| p.x).sum::<f64>() / mid.len() as f64;
    let mean_y = mid.iter().map(|p| p.y).sum::<f64>() / mid.len() as f64;
    assert!(
        mean_x < 1400.0,
        "pieces head left, into the screen: {mean_x}"
    );
    assert!(mean_y > 110.0, "and down: {mean_y}");
    // Some reach well across the screen.
    assert!(mid.iter().any(|p| p.x < 960.0));
    let late = b.at(DURATION - Duration::from_millis(100));
    assert!(late.iter().all(|p| p.alpha > 0.0 && p.alpha < 1.0));
    assert!(b.at(DURATION).is_empty());
}

#[test]
fn a_burst_is_deterministic_per_seed() {
    let t = Duration::from_millis(500);
    assert_eq!(
        Burst::new(CARD, AREA, 1.0, 3).at(t),
        Burst::new(CARD, AREA, 1.0, 3).at(t)
    );
    assert_ne!(
        Burst::new(CARD, AREA, 1.0, 3).at(t),
        Burst::new(CARD, AREA, 1.0, 4).at(t)
    );
}

#[test]
fn a_bottom_left_card_throws_right_and_up() {
    let card = Rect {
        l: 20,
        t: 970,
        r: 380,
        b: 1060,
    };
    let b = Burst::new(card, AREA, 1.0, 1);
    let early = b.at(Duration::from_millis(300));
    let mean_x = early.iter().map(|p| p.x).sum::<f64>() / early.len() as f64;
    let mean_y = early.iter().map(|p| p.y).sum::<f64>() / early.len() as f64;
    assert!(mean_x > 300.0 && mean_y < 1000.0, "{mean_x},{mean_y}");
}

#[test]
fn rasterize_fills_pieces_premultiplied_and_clips() {
    let b = Burst::new(CARD, AREA, 2.0, 9);
    let (w, h) = (1920, 1080);
    let mut buf = vec![0xdead_beef_u32; w * h];
    rasterize(&b.at(Duration::ZERO), w, h, &mut buf);
    let lit: Vec<u32> = buf.iter().copied().filter(|&p| p != 0).collect();
    assert!(
        lit.len() > PIECES * 10,
        "pieces cover pixels: {}",
        lit.len()
    );
    assert!(lit.iter().all(|p| p >> 24 == 255), "opaque at the start");
    // Off-screen pieces (a burst over a tiny area) don't panic.
    let mut small = vec![0u32; 10 * 10];
    rasterize(&b.at(Duration::from_millis(700)), 10, 10, &mut small);
    // Fading pieces are premultiplied: no channel above alpha.
    let fading = b.at(DURATION - Duration::from_millis(100));
    rasterize(&fading, w, h, &mut buf);
    for p in buf.iter().copied().filter(|&p| p != 0) {
        let a = p >> 24;
        assert!(a < 255 && (p >> 16 & 255) <= a && (p >> 8 & 255) <= a && (p & 255) <= a);
    }
}
