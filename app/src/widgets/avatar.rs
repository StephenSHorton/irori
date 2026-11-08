//! The person's avatar (the React app's lib/avatar.ts, card #393): a small
//! picture data URL they picked, drawn as a face, else their initial on
//! their colour. `Window::person_avatar` gives the URL; it is decoded off
//! the UI thread into [`rune::Pixels`] and kept in the widgets model.

/// A face's letter with no picture: the name's first, uppercased ("?").
pub fn initials(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map_or_else(|| "?".into(), |c| c.to_uppercase().collect())
}

/// Decodes an avatar data URL through Rune (slow: off the UI thread).
/// `None` for one that isn't an avatar, or doesn't decode.
pub fn decode(url: &str) -> Option<rune::Pixels> {
    crate::profile::avatar::decode(url)
}

/// The person's avatar as the window holds it: the URL it came from and
/// its pixels, ready to draw.
#[derive(Clone, Default)]
pub struct Avatar {
    pub url: String,
    pub pixels: Option<rune::Pixels>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_of_a_name() {
        assert_eq!(initials(" stephen"), "S");
        assert_eq!(initials(""), "?");
    }

    #[test]
    fn decodes_a_png() {
        use base64::Engine;
        let mut png = Vec::new();
        {
            let mut e = ::png::Encoder::new(&mut png, 4, 2);
            e.set_color(::png::ColorType::Rgba);
            e.set_depth(::png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&[200u8; 4 * 2 * 4]).unwrap();
        }
        let url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        // Rune keeps the whole picture; the face crops it to its square.
        assert_eq!(decode(&url).expect("decodes").size(), (4, 2));
        assert!(decode("data:image/png;base64,AAAA").is_none());
    }
}
