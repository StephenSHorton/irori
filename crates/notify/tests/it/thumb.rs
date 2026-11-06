//! Card thumbnails (card #149): PNG decode, shrink, and the local-only
//! resolver.

use std::sync::Arc;

use notify::thumb::{THUMB_MAX_H, THUMB_MAX_W, fit, thumbnail_from_png};
use notify::{FileFn, attachment_thumbnail, thumbnail};

fn png(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut e = png::Encoder::new(&mut out, w, h);
    e.set_color(color);
    e.set_depth(png::BitDepth::Eight);
    let mut wr = e.write_header().unwrap();
    wr.write_image_data(data).unwrap();
    wr.finish().unwrap();
    out
}

#[test]
fn a_small_rgba_png_comes_back_as_is() {
    let px = [10, 20, 30, 255, 40, 50, 60, 128];
    let img = thumbnail_from_png(&png(2, 1, png::ColorType::Rgba, &px)).unwrap();
    assert_eq!((img.w, img.h), (2, 1));
    assert_eq!(&img.rgba[..], &px);
}

#[test]
fn rgb_and_gray_become_rgba() {
    let img = thumbnail_from_png(&png(1, 1, png::ColorType::Rgb, &[1, 2, 3])).unwrap();
    assert_eq!(&img.rgba[..], &[1, 2, 3, 255]);
    let img = thumbnail_from_png(&png(1, 1, png::ColorType::Grayscale, &[9])).unwrap();
    assert_eq!(&img.rgba[..], &[9, 9, 9, 255]);
    let img = thumbnail_from_png(&png(1, 1, png::ColorType::GrayscaleAlpha, &[9, 7])).unwrap();
    assert_eq!(&img.rgba[..], &[9, 9, 9, 7]);
}

#[test]
fn a_screenshot_is_shrunk_to_the_slot_keeping_its_aspect() {
    let (w, h) = (1920, 1080);
    let data: Vec<u8> = (0..w * h).flat_map(|_| [200u8, 100, 50]).collect();
    let img = thumbnail_from_png(&png(w, h, png::ColorType::Rgb, &data)).unwrap();
    assert!(img.w <= THUMB_MAX_W && img.h <= THUMB_MAX_H);
    assert_eq!((img.w, img.h), fit(w, h, THUMB_MAX_W, THUMB_MAX_H));
    let ratio = img.w as f64 / img.h as f64;
    assert!((ratio - 16.0 / 9.0).abs() < 0.02, "{ratio}");
    // A flat colour stays that colour through the box filter.
    assert_eq!(&img.rgba[..4], &[200, 100, 50, 255]);
}

#[test]
fn not_a_png_or_not_here_gives_none() {
    assert!(thumbnail_from_png(b"not a png").is_none());
    assert!(thumbnail_from_png(&[]).is_none());
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.png");
    assert!(thumbnail(&missing).is_none());
    let junk = dir.path().join("junk");
    std::fs::write(&junk, b"\x89PNG but not").unwrap();
    assert!(thumbnail(&junk).is_none());
}

#[test]
fn the_resolver_decides_what_is_local() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("shot");
    std::fs::write(&p, png(1, 1, png::ColorType::Rgb, &[1, 2, 3])).unwrap();
    let here = p.clone();
    let file: FileFn = Arc::new(move |id: &str| (id == "aa").then(|| here.clone()));
    assert!(attachment_thumbnail(Some(&file), "aa").is_some());
    assert!(
        attachment_thumbnail(Some(&file), "bb").is_none(),
        "not on this device"
    );
    assert!(attachment_thumbnail(None, "aa").is_none(), "no resolver");
}
