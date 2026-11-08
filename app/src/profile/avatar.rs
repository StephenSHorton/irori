//! The person's avatar (lib/avatar.ts, card #393): a picture they pick,
//! shrunk to a small square so it fits on their member.profile in every
//! space they share (coord::person_avatar::MAX_LEN caps it at 32 KB), and
//! Face (profile/face.tsx): the picture, else the initial on their colour.

use base64::Engine;
use std::sync::{Arc, LazyLock};

use rune::{Color, El, ImageCache, Pixels};
use rune_kit::Avatar;

use crate::ui::Frame;

/// The square's side in pixels: sharp at the largest face (64) on a 2x
/// screen.
pub const AVATAR_SIZE: u32 = 128;
/// coord::person_avatar::MAX_LEN.
pub const MAX_AVATAR: usize = 32 * 1024;

/// The centred square of a w×h picture: its left, top and side.
pub fn crop_square(w: u32, h: u32) -> (u32, u32, u32) {
    let side = w.min(h);
    ((w - side) / 2, (h - side) / 2, side)
}

/// coord::person_avatar::valid for a non-empty one: a PNG, JPEG or WebP
/// data URL of plain base64, within the cap.
pub fn is_avatar(a: &str) -> bool {
    if a.is_empty() || a.len() > MAX_AVATAR {
        return false;
    }
    let Some(rest) = ["png", "jpeg", "webp"]
        .iter()
        .find_map(|t| a.strip_prefix(&format!("data:image/{t};base64,")))
    else {
        return false;
    };
    !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
}

/// A face's letter with no picture: the name's first, uppercased.
pub fn initials(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

/// The kinds of picture the picker offers (React's accept list).
pub const PICTURE_TYPES: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "heic", "heif"];

/// A picked file's bytes. HEIC (an iPhone's photos) has no decoder here,
/// so on the Mac the system's `sips` turns it into a PNG first.
pub fn read_picture(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ext != "heic" && ext != "heif" {
        return std::fs::read(path).map_err(|e| e.to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::env::temp_dir().join(format!("irori-picture-{}.png", std::process::id()));
        let ok = std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "png"])
            .arg(path)
            .arg("--out")
            .arg(&out)
            .output()
            .is_ok_and(|o| o.status.success());
        let read = if ok { std::fs::read(&out).ok() } else { None };
        let _ = std::fs::remove_file(&out);
        if let Some(b) = read {
            return Ok(b);
        }
    }
    Err("Irori can't read HEIC pictures here. Save it as a PNG or JPEG and pick that.".into())
}

fn data_url(kind: &str, bytes: &[u8]) -> String {
    format!(
        "data:image/{kind};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// A picked picture as the avatar: its centred square at AVATAR_SIZE. One
/// with see-through parts keeps them as a PNG (or lossless WebP) when that
/// fits; otherwise JPEG, lowering the quality until it fits, any
/// see-through parts on white.
pub fn shrink_avatar(bytes: &[u8]) -> Result<String, String> {
    let img = image::load_from_memory(bytes)
        .map_err(|_| "That file isn't a picture Irori can read.".to_string())?;
    let (x, y, side) = crop_square(img.width(), img.height());
    let sq = img
        .crop_imm(x, y, side, side)
        .resize_exact(
            AVATAR_SIZE,
            AVATAR_SIZE,
            image::imageops::FilterType::CatmullRom,
        )
        .to_rgba8();
    if sq.pixels().any(|p| p[3] < 255) {
        for (fmt, kind) in [
            (image::ImageFormat::Png, "png"),
            (image::ImageFormat::WebP, "webp"),
        ] {
            let mut out = Vec::new();
            if sq
                .write_to(&mut std::io::Cursor::new(&mut out), fmt)
                .is_ok()
            {
                let url = data_url(kind, &out);
                if is_avatar(&url) {
                    return Ok(url);
                }
            }
        }
    }
    let flat = image::RgbImage::from_fn(AVATAR_SIZE, AVATAR_SIZE, |x, y| {
        let p = sq.get_pixel(x, y);
        let a = u16::from(p[3]);
        image::Rgb([0, 1, 2].map(|i| ((u16::from(p[i]) * a + 255 * (255 - a)) / 255) as u8))
    });
    for q in [90u8, 80, 65, 50] {
        let mut out = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q);
        if flat.write_with_encoder(enc).is_err() {
            continue;
        }
        let url = data_url("jpeg", &out);
        if is_avatar(&url) {
            return Ok(url);
        }
    }
    Err("That picture is still too big after shrinking it.".into())
}

/// Avatars decode on Rune's image workers; [`decode`] waits for its own
/// decode off the UI thread, so the cache needs no wake.
static IMAGES: LazyLock<ImageCache> = LazyLock::new(|| ImageCache::new(Arc::new(|| {})));

/// An avatar's picture, decoded by Rune (any raster it reads: Irori only
/// saves PNG, JPEG and WebP, the React app WebP first), for [`face`]; None
/// when it isn't one (the face falls back to the initial). It waits for
/// the decode: call it off the UI thread.
pub fn decode(avatar: &str) -> Option<Pixels> {
    if !is_avatar(avatar) {
        return None;
    }
    let b64 = avatar.split_once(',')?.1;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    // load_raster refuses SVG by content, which `load` would draw.
    let img = IMAGES.load_raster(bytes);
    img.wait().ok()?;
    Some(img.pixels().clone())
}

/// A person's face (kit Avatar): their picture, else their initial on
/// their colour (grey with none). `size` is its side in points.
pub fn face(
    f: &Frame,
    key: String,
    name: &str,
    color: Option<Color>,
    pic: Option<&Pixels>,
    size: f32,
) -> El {
    let k = f.k;
    let (fill, ink) = match color {
        Some(c) => (c, k.c.bg_base),
        None => (k.c.layer_03, k.c.text_muted),
    };
    let letters = Letters {
        text: initials(name),
        size: (size * 0.42).round().max(9.0),
        weight: 600.0,
        ink,
    };
    kit_face(name, size, fill, letters, pic, None)
        .help(name)
        .build(k)
        .key(key)
}

/// The letters a face shows without a picture, at main's size for it.
pub struct Letters {
    pub text: String,
    pub size: f32,
    pub weight: f32,
    pub ink: Color,
}

/// A kit Avatar `side` points across, named `name`, drawn as main's
/// faces are: with a picture, only the picture (no colour or letters
/// behind it, so a transparent one shows the page, as face.tsx); without
/// one, `letters` on `fill`. `status` is its corner dot.
pub fn kit_face(
    name: &str,
    side: f32,
    fill: Color,
    letters: Letters,
    pic: Option<&Pixels>,
    status: Option<Color>,
) -> Avatar {
    let mut a = Avatar::new(name)
        .size(side)
        .fill(fill)
        .initials(letters.text)
        .letter_size(letters.size)
        .letter_weight(letters.weight)
        .ink(letters.ink);
    if let Some(p) = pic {
        a = a.image(p);
    }
    match status {
        Some(c) => a.status(c),
        None => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_is_the_centred_square() {
        assert_eq!(crop_square(400, 300), (50, 0, 300));
        assert_eq!(crop_square(300, 401), (0, 50, 300));
        assert_eq!(crop_square(64, 64), (0, 0, 64));
    }

    #[test]
    fn only_small_image_data_urls_count() {
        assert!(is_avatar("data:image/png;base64,iVBORw0KGgo="));
        assert!(is_avatar("data:image/webp;base64,UklGR="));
        assert!(!is_avatar(""));
        assert!(!is_avatar("data:image/gif;base64,R0lG"));
        assert!(!is_avatar("data:image/png;base64,<script>"));
        assert!(!is_avatar("https://example.com/me.png"));
        let big = format!("data:image/png;base64,{}", "A".repeat(MAX_AVATAR));
        assert!(!is_avatar(&big));
    }

    #[test]
    fn initials_are_the_first_letter() {
        assert_eq!(initials(" stephen"), "S");
        assert_eq!(initials(""), "?");
        assert_eq!(initials("ärla"), "Ä");
    }

    #[test]
    fn a_picture_shrinks_to_a_small_square_that_reads_back() {
        let img = image::RgbImage::from_fn(300, 200, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let a = shrink_avatar(&png).unwrap();
        assert!(a.starts_with("data:image/jpeg;base64,"));
        assert!(is_avatar(&a));
        assert_eq!(decode(&a).unwrap().size(), (AVATAR_SIZE, AVATAR_SIZE));
        assert!(shrink_avatar(b"not a picture").is_err());
    }

    fn encoded(img: image::DynamicImage, fmt: image::ImageFormat) -> Vec<u8> {
        let mut b = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut b), fmt)
            .unwrap();
        b
    }

    #[test]
    fn a_see_through_picture_keeps_its_transparency() {
        // A dot on nothing: the corners stay clear.
        let img = image::RgbaImage::from_fn(200, 200, |x, y| {
            let (dx, dy) = (x as i32 - 100, y as i32 - 100);
            if dx * dx + dy * dy < 60 * 60 {
                image::Rgba([200, 40, 40, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        let a = shrink_avatar(&encoded(
            image::DynamicImage::ImageRgba8(img),
            image::ImageFormat::Png,
        ))
        .unwrap();
        assert!(a.starts_with("data:image/png;base64,"), "{}", &a[..30]);
        let png = base64::engine::general_purpose::STANDARD
            .decode(a.split_once(',').unwrap().1)
            .unwrap();
        let px = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(px.get_pixel(0, 0)[3], 0, "the corner is still clear");
        assert_eq!(decode(&a).unwrap().size(), (AVATAR_SIZE, AVATAR_SIZE));
    }

    #[test]
    fn a_gif_shrinks_too() {
        let img = image::RgbaImage::from_fn(40, 30, |x, _| image::Rgba([x as u8 * 6, 90, 30, 255]));
        let gif = encoded(
            image::DynamicImage::ImageRgba8(img),
            image::ImageFormat::Gif,
        );
        assert!(
            shrink_avatar(&gif)
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
    }

    #[test]
    fn a_webp_avatar_from_the_react_app_draws() {
        let img = image::RgbaImage::from_fn(16, 12, |x, y| {
            image::Rgba([x as u8 * 10, y as u8 * 10, 99, 255])
        });
        let mut webp = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut webp),
                image::ImageFormat::WebP,
            )
            .unwrap();
        let url = format!(
            "data:image/webp;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&webp)
        );
        // Rune decodes WebP itself.
        assert_eq!(decode(&url).expect("webp decodes").size(), (16, 12));
        assert!(decode("data:image/png;base64,AAAA").is_none());
        // SVG labelled as PNG: Rune would rasterize it, so it never gets there.
        let svg = base64::engine::general_purpose::STANDARD.encode(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4"/></svg>"#,
        );
        assert!(decode(&format!("data:image/png;base64,{svg}")).is_none());
        // And a picked WebP shrinks like any other picture.
        assert!(shrink_avatar(&webp).is_ok());
    }
}
