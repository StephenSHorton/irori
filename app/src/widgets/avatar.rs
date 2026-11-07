//! The person's avatar (the React app's lib/avatar.ts, card #393): a small
//! picture data URL they picked, drawn as a face, else their initial on
//! their colour. `Window::person_avatar` gives the URL; it is decoded off
//! the UI thread into [`rune::Pixels`] and kept in the widgets model.

use base64::Engine;

/// The square's side in pixels the window shrinks a picked picture to.
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
        .find_map(|k| a.strip_prefix(&format!("data:image/{k};base64,")))
    else {
        return false;
    };
    !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
}

/// A face's letter with no picture: the name's first, uppercased ("?").
pub fn initials(name: &str) -> String {
    name.trim().chars().next().map_or_else(|| "?".into(), |c| c.to_uppercase().collect())
}

/// A decoded picture: width, height, straight RGBA.
pub type Rgba = (u32, u32, Vec<u8>);

/// Decodes an avatar data URL (slow: off the UI thread). `None` for one
/// [`is_avatar`] refuses, or that doesn't decode.
pub fn decode(url: &str) -> Option<Rgba> {
    if !is_avatar(url) {
        return None;
    }
    let b64 = url.split_once(',')?.1;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let (x, y, side) = crop_square(w, h);
    let sq = image::imageops::crop_imm(&img, x, y, side, side).to_image();
    Some((side, side, sq.into_raw()))
}

/// The person's avatar as the window holds it: the URL it came from and
/// its pixels, ready to draw.
#[derive(Clone, Default)]
pub struct Avatar {
    pub url: String,
    pub pixels: Option<rune::Pixels>,
}

impl Avatar {
    pub fn from_rgba(url: String, rgba: Option<Rgba>) -> Avatar {
        let pixels = rgba.map(|(w, h, px)| {
            let p = rune::Pixels::new();
            p.set(w, h, px);
            p
        });
        Avatar { url, pixels }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_small_png_jpeg_or_webp_data_urls() {
        assert!(is_avatar("data:image/png;base64,iVBORw0KGgo="));
        assert!(is_avatar("data:image/webp;base64,UklGRg=="));
        assert!(!is_avatar(""));
        assert!(!is_avatar("data:image/svg+xml;base64,PHN2Zz4="));
        assert!(!is_avatar("https://example.com/a.png"));
        assert!(!is_avatar(&format!("data:image/png;base64,{}", "A".repeat(40_000))));
    }

    #[test]
    fn the_centred_square() {
        assert_eq!(crop_square(400, 300), (50, 0, 300));
        assert_eq!(crop_square(300, 401), (0, 50, 300));
    }

    #[test]
    fn initials_of_a_name() {
        assert_eq!(initials(" stephen"), "S");
        assert_eq!(initials(""), "?");
    }

    #[test]
    fn decodes_a_png() {
        let mut png = Vec::new();
        {
            let mut e = ::png::Encoder::new(&mut png, 4, 2);
            e.set_color(::png::ColorType::Rgba);
            e.set_depth(::png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&[200u8; 4 * 2 * 4]).unwrap();
        }
        let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png));
        let (w, h, px) = decode(&url).expect("decodes");
        assert_eq!((w, h, px.len()), (2, 2, 16));
        assert!(decode("data:image/png;base64,AAAA").is_none());
    }
}
