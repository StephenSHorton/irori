//! The cards' sounds (go-toast sound.go): the s&box editor notification
//! sounds (Facepunch, MIT; sounds/NOTICE.md). All three are 16-bit PCM at
//! 44.1 kHz; success is stereo, and fail and success carry a LIST chunk
//! before their data.

use std::sync::{Arc, Mutex};

use crate::config::Kind;

pub static SUCCESS_WAV: &[u8] = include_bytes!("../sounds/success.wav");
pub static FAIL_WAV: &[u8] = include_bytes!("../sounds/fail.wav");
pub static PUBLISHED_WAV: &[u8] = include_bytes!("../sounds/published.wav");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Success,
    Fail,
    Published,
}

impl Sound {
    pub fn wav(self) -> &'static [u8] {
        match self {
            Sound::Fail => FAIL_WAV,
            Sound::Published => PUBLISHED_WAV,
            Sound::Success => SUCCESS_WAV,
        }
    }

    fn slot(self) -> usize {
        match self {
            Sound::Success => 0,
            Sound::Fail => 1,
            Sound::Published => 2,
        }
    }
}

/// A card's sound: an actionable card (it has buttons) or a warning gets
/// "published", an error "fail", anything else "success".
pub fn sound_for(k: Kind, actionable: bool) -> Sound {
    if actionable || k == Kind::Warn {
        Sound::Published
    } else if k == Kind::Error {
        Sound::Fail
    } else {
        Sound::Success
    }
}

/// The sample bytes `wav[off..off + n]` of a 16-bit PCM WAV, found by
/// walking its RIFF chunks; `None` for anything else.
pub fn pcm_data(wav: &[u8]) -> Option<(usize, usize)> {
    if wav.len() < 12 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let mut pcm16 = false;
    let mut p = 12usize;
    while p + 8 <= wav.len() {
        let id = &wav[p..p + 4];
        let size = u32::from_le_bytes([wav[p + 4], wav[p + 5], wav[p + 6], wav[p + 7]]) as usize;
        let body = p + 8;
        if body + size > wav.len() {
            return None;
        }
        match id {
            b"fmt " if size >= 16 => {
                let format = u16::from_le_bytes([wav[body], wav[body + 1]]);
                let bits = u16::from_le_bytes([wav[body + 14], wav[body + 15]]);
                pcm16 = format == 1 && bits == 16;
            }
            b"data" => return pcm16.then_some((body, size)),
            _ => {}
        }
        p = body + size + (size & 1); // chunks are word-aligned
    }
    None
}

/// A copy of wav with its samples scaled by vol. At full volume, or for a
/// file it can't parse, wav itself.
pub fn scale_wav(wav: &'static [u8], vol: f64) -> Arc<[u8]> {
    let Some((off, n)) = pcm_data(wav) else {
        return Arc::from(wav);
    };
    if vol >= 1.0 {
        return Arc::from(wav);
    }
    let mut out = wav.to_vec();
    let mut i = off;
    while i + 1 < off + n {
        let v = i16::from_le_bytes([out[i], out[i + 1]]) as f64 * vol;
        out[i..i + 2].copy_from_slice(&(v.round() as i16).to_le_bytes());
        i += 2;
    }
    Arc::from(out)
}

struct Cache {
    vol: f64,
    sounds: [Option<Arc<[u8]>>; 3],
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    vol: -1.0,
    sounds: [None, None, None],
});

/// s scaled to vol (a cached copy), or `None` for silence. The volume is
/// rounded to 0.01, so a dragged slider rebuilds per percent. The OS keeps
/// its own reference to what it plays (the Arc the player holds).
pub fn sound_bytes(s: Sound, vol: f64) -> Option<Arc<[u8]>> {
    let vol = (vol * 100.0).round() / 100.0;
    if vol <= 0.0 || vol.is_nan() {
        return None;
    }
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if c.vol != vol {
        c.vol = vol;
        c.sounds = [None, None, None];
    }
    let b = c.sounds[s.slot()].get_or_insert_with(|| scale_wav(s.wav(), vol));
    Some(b.clone())
}
