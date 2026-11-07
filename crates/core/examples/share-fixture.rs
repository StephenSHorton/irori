//! share-fixture (#658): encode a picture on this machine's hardware H.264
//! encoder and write what the window's share channel would carry, for the
//! browser preview's share stage (frontend/public/preview/share-fixture.bin).
//!
//! ```sh
//! ffmpeg -i picture.png -vf scale=W:H -pix_fmt yuv420p -f rawvideo pic.yuv
//! cargo run --release -p irori-core --example share-fixture -- pic.yuv W H out.bin [frames]
//! ```
//!
//! The file is the channel's messages (irori_net::share::view_config,
//! view_chunk), each after its length as a u32 BE: a config, then a
//! keyframe and `frames - 1` more access units at 10 fps. A raw file with
//! several pictures in a row is played in order, then the last repeats.

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main() {
    use irori_net::share::{Avc, has_idr, to_avcc, view_chunk, view_config};
    use std::time::Duration;

    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 4 {
        eprintln!("usage: share-fixture <in.yuv> <width> <height> <out.bin> [frames]");
        std::process::exit(2);
    }
    let (w, h): (u32, u32) = (a[1].parse().unwrap(), a[2].parse().unwrap());
    let frames: u64 = a.get(4).map_or(30, |v| v.parse().unwrap());
    let raw = std::fs::read(&a[0]).expect("read yuv");
    let size = (w * h * 3 / 2) as usize;
    let pics: Vec<&[u8]> = raw.chunks_exact(size).collect();
    assert!(!pics.is_empty(), "the yuv file is smaller than one picture");
    let settings = share::Settings {
        width: w,
        height: h,
        fps: 10,
        bitrate_kbps: Some(2500),
        ..share::Settings::default()
    };
    let mut enc = share::Encoder::new(&settings).expect("encoder");
    let mut out = Vec::new();
    let mut put = |b: Vec<u8>| {
        out.extend_from_slice(&(b.len() as u32).to_be_bytes());
        out.extend_from_slice(&b);
    };
    let mut config: Option<Avc> = None;
    let mut chunks = Vec::new();
    for n in 0..frames {
        let pic = pics[(n as usize).min(pics.len() - 1)].to_vec();
        let f = share::i420_frame(w, h, pic, Duration::from_millis(n * 100)).unwrap();
        chunks.extend(enc.encode(&f).unwrap());
    }
    chunks.extend(enc.flush().unwrap());
    for c in chunks {
        let key = has_idr(&c.data);
        if key
            && let Some(a) = Avc::from_annexb(&c.data)
            && config.as_ref() != Some(&a)
        {
            put(view_config(&a));
            config = Some(a);
        }
        if config.is_some() {
            put(view_chunk(key, c.timestamp_us, &to_avcc(&c.data)));
        }
    }
    std::fs::write(&a[3], &out).expect("write");
    println!(
        "{} bytes, codec {}",
        out.len(),
        config.map(|c| c.codec).unwrap_or_default()
    );
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {
    eprintln!("share-fixture needs the Mac's or the PC's hardware encoder");
}
