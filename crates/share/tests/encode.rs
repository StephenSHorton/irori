//! The hardware encoder at the card's target size: keyframes come when asked
//! for and not otherwise, and the bitrate can change mid-share. Runs on the
//! Mac (VideoToolbox) and the PC (Media Foundation); no capture, so no
//! Screen Recording grant.

use share::{Encoder, Settings, synthetic_frame};
use std::time::Duration;

#[test]
fn keyframes_on_request_at_1080p() {
    let settings = Settings {
        // Long enough that only our requests make keyframes in 40 frames.
        keyframe_every: Duration::from_secs(60),
        bitrate_kbps: Some(1500),
        ..Settings::default()
    };
    let mut enc = Encoder::new(&settings).expect("a hardware H.264 encoder");
    assert!(
        ["videotoolbox", "mediafoundation"].contains(&enc.name()),
        "backend {}",
        enc.name()
    );

    let ask_at = [15u64, 30];
    let mut chunks = Vec::new();
    for n in 0..40u64 {
        if ask_at.contains(&n) {
            enc.request_keyframe().unwrap();
        }
        if n == 20 {
            enc.set_bitrate_kbps(800).unwrap();
            assert_eq!(enc.bitrate_kbps(), 800);
        }
        let at = Duration::from_millis(n * 100);
        let frame = synthetic_frame(settings.width, settings.height, n, at).unwrap();
        chunks.extend(enc.encode(&frame).unwrap());
    }
    chunks.extend(enc.flush().unwrap());

    assert_eq!(chunks.len(), 40, "one chunk per frame");
    let keys: Vec<u64> = chunks
        .iter()
        .filter(|c| c.keyframe)
        .map(|c| c.timestamp_us / 100_000)
        .collect();
    assert_eq!(keys, vec![0, 15, 30], "keyframes at start and on request only");
    // Every keyframe carries its parameter sets, so a viewer can start there.
    for c in chunks.iter().filter(|c| c.keyframe) {
        let types: Vec<u8> = nal_types(&c.data);
        assert!(types.contains(&7) && types.contains(&8), "SPS/PPS in {types:?}");
    }
}

fn nal_types(au: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 < au.len() {
        if au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 1 {
            out.push(au[i + 3] & 0x1f);
            i += 3;
        } else {
            i += 1;
        }
    }
    out
}
