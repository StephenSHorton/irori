//! A moving test picture, for timing the encoder without capturing anything
//! (no Screen Recording grant needed).

use crate::{Frame, Result, timestamp};
use std::time::Duration;

/// Frame `n` of a test pattern at `width` x `height`: text-like stripes that
/// scroll, and a block that moves across, so every frame has new detail the
/// way a scrolling document or a terminal does. Harder to encode than most
/// real shares, which are mostly still.
pub fn synthetic_frame(width: u32, height: u32, n: u64, at: Duration) -> Result<Frame> {
    let (w, h) = (width as usize, height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut data = vec![0u8; w * h + 2 * cw * ch];
    let scroll = (n as usize * 7) % h;
    let (luma, chroma) = data.split_at_mut(w * h);
    for y in 0..h {
        let row = &mut luma[y * w..(y + 1) * w];
        let line = (y + scroll) % h;
        // "Lines of text": dark runs on light rows, every 18 px.
        let text = line % 18 < 12;
        for (x, px) in row.iter_mut().enumerate() {
            let word = ((x / 9) * 31 + line / 18 * 17) % 11 < 7;
            *px = if text && word && !(x + line).is_multiple_of(3) {
                40
            } else {
                230
            };
        }
    }
    // A moving 200 x 120 block.
    let bx = (n as usize * 23) % w.saturating_sub(200).max(1);
    let by = (n as usize * 11) % h.saturating_sub(120).max(1);
    for y in by..(by + 120).min(h) {
        for x in bx..(bx + 200).min(w) {
            luma[y * w + x] = ((x + y + n as usize) % 256) as u8;
        }
    }
    chroma.fill(128);
    let i420 = moq_video::I420::new(moq_video::Size::new(width, height), data)?;
    Ok(Frame::new(moq_video::Surface::I420(i420), timestamp(at)?))
}
