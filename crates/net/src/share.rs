//! Screen share on a call connection (#658): the wire.
//!
//! A share rides the `irori/call/1` connections the huddle already has
//! (crate::call), one per pair of devices:
//!
//! - **Control** goes on the call's first bi stream, the `call` stream,
//!   which carried nothing past its open messages before. Each message is
//!   one length-prefixed JSON frame ([`ShareMsg`]): the sharer says a share
//!   is on ([`ShareMsg::Start`], sent again when its picture changes size)
//!   or off; a viewer says it is watching or not, and asks for a keyframe.
//!   A message whose `t` this build doesn't know is skipped.
//! - **Video** goes on uni streams, one per keyframe group (GOP): a stream
//!   opens with the label [`SHARE_LABEL`] and a [`GopHeader`], then carries
//!   access units ([`Au`]) in order, the first a keyframe. A lost packet
//!   stalls only its own GOP; a viewer that sees a newer GOP drops the older
//!   one (`stop`), and a sender that falls behind resets its GOP and starts
//!   the next at a keyframe. Video streams sit below the control stream in
//!   priority ([`VIDEO_PRIORITY`]); voice datagrams go ahead of all stream
//!   data in every packet anyway (noq writes DATAGRAM frames before STREAM
//!   frames), and the share's bitrate cap is what keeps the path free for
//!   them (#653: 2.5 Mbit/s held voice clean, 8 Mbit/s from the Mac didn't).
//! - The encoder emits **Annex-B** H.264 with SPS/PPS in band on every
//!   keyframe (crates/share). The viewer turns it into what WebCodecs takes
//!   ([`Avc`]): AVCC access units and an `avcC` description with the codec
//!   string from the SPS, and frames it for the window ([`view_config`],
//!   [`view_chunk`]).

use bytes::Bytes;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use irori_meshseam::{MAX_LABEL, read_frame, write_frame};
use serde::{Deserialize, Serialize};

/// The label a share's video stream opens with.
pub const SHARE_LABEL: &str = "share";
/// The control messages' version.
pub const SHARE_V: u32 = 1;
/// The largest control message.
pub const MSG_MAX: usize = 16 * 1024;
/// The largest access unit: a Mac keyframe runs 0.6-1 MB (#657).
pub const AU_MAX: usize = 8 * 1024 * 1024;
/// Video streams go after the control stream (priority 0).
pub const VIDEO_PRIORITY: i32 = -1;
/// The access unit's flag: it is a keyframe (an IDR, decodable alone).
pub const FLAG_KEY: u8 = 1;

/// What is being shared, as the sharer tells the call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareInfo {
    pub v: u32,
    /// 32 lower-case hex characters, new for each share.
    pub share_id: String,
    /// When the share started, the sharer's Unix ms: the newer of two
    /// shares wins (sharing yours replaces theirs).
    pub started_ms: i64,
    /// `window` or `display`.
    pub kind: String,
    /// The window's title, or the display's name.
    pub title: String,
    /// The app that owns the window ("" for a display).
    pub app: String,
    /// The picture's size, pixels.
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// The encoder's target bitrate now, kbit/s.
    pub kbps: u32,
}

/// One control message on the call stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum ShareMsg {
    /// Sharer to every peer: this share is on (again when it changes).
    Start(ShareInfo),
    /// Sharer to every peer: the share ended.
    #[serde(rename_all = "camelCase")]
    Stop { share_id: String },
    /// Viewer to sharer: send me the video (on) or stop (off).
    #[serde(rename_all = "camelCase")]
    Watch { share_id: String, on: bool },
    /// Viewer to sharer: I need a keyframe (I joined, or lost my place).
    #[serde(rename_all = "camelCase")]
    Keyframe { share_id: String },
    /// A message a newer build sends that this one doesn't know.
    #[serde(other)]
    Unknown,
}

/// Writes one control message.
pub async fn write_msg(s: &mut SendStream, m: &ShareMsg) -> Result<(), String> {
    let b = serde_json::to_vec(m).map_err(|e| e.to_string())?;
    write_frame(s, &b, MSG_MAX).await.map_err(|e| e.to_string())
}

/// Reads one control message; `None` when the stream ended. A message that
/// doesn't parse is [`ShareMsg::Unknown`], not an error, so an older build
/// keeps reading past what a newer one sends.
pub async fn read_msg(r: &mut RecvStream) -> Result<Option<ShareMsg>, String> {
    let Some(b) = read_frame(r, MSG_MAX, false)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    Ok(Some(
        serde_json::from_slice(&b).unwrap_or(ShareMsg::Unknown),
    ))
}

/// What a video stream opens with, after its label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GopHeader {
    pub share_id: String,
    /// Counts up through the share: a higher one is newer.
    pub gop: u64,
}

/// One access unit on a video stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Au {
    /// The frame's time from the share's start, µs (the sharer's clock).
    pub ts_us: u64,
    pub key: bool,
    /// Annex-B NAL units.
    pub data: Bytes,
}

/// Opens a GOP's stream on `conn`, below the control stream in priority,
/// and writes its label and header.
pub async fn open_gop(conn: &Connection, h: &GopHeader) -> Result<SendStream, String> {
    let mut s = conn.open_uni().await.map_err(|e| e.to_string())?;
    // Only fails on a stream already closed.
    let _ = s.set_priority(VIDEO_PRIORITY);
    write_frame(&mut s, SHARE_LABEL.as_bytes(), MAX_LABEL)
        .await
        .map_err(|e| e.to_string())?;
    let b = serde_json::to_vec(h).map_err(|e| e.to_string())?;
    write_frame(&mut s, &b, MSG_MAX)
        .await
        .map_err(|e| e.to_string())?;
    Ok(s)
}

/// Writes one access unit: `ts_us` (u64 BE), flags (u8), then the bytes,
/// as one length-prefixed frame.
pub async fn write_au(s: &mut SendStream, au: &Au) -> Result<(), String> {
    let mut b = Vec::with_capacity(9 + au.data.len());
    b.extend_from_slice(&au.ts_us.to_be_bytes());
    b.push(if au.key { FLAG_KEY } else { 0 });
    b.extend_from_slice(&au.data);
    write_frame(s, &b, AU_MAX + 9)
        .await
        .map_err(|e| e.to_string())
}

/// Reads a video stream's label and header. `Err` for a uni stream that
/// isn't a share's.
pub async fn read_gop(r: &mut RecvStream) -> Result<GopHeader, String> {
    let label = read_frame(r, MAX_LABEL, false)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("no label")?;
    if label != SHARE_LABEL.as_bytes() {
        return Err("not a share stream".into());
    }
    let h = read_frame(r, MSG_MAX, false)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("no header")?;
    serde_json::from_slice(&h).map_err(|e| e.to_string())
}

/// Reads the next access unit; `None` when the GOP ended.
pub async fn read_au(r: &mut RecvStream) -> Result<Option<Au>, String> {
    let Some(b) = read_frame(r, AU_MAX + 9, false)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    if b.len() < 9 {
        return Err("short access unit".into());
    }
    let ts_us = u64::from_be_bytes(b[..8].try_into().expect("8 bytes"));
    let key = b[8] & FLAG_KEY != 0;
    let mut b = Bytes::from(b);
    Ok(Some(Au {
        ts_us,
        key,
        data: b.split_off(9),
    }))
}

// ---- Annex-B to WebCodecs ---------------------------------------------------

/// The NAL units of an Annex-B access unit, without their start codes (and
/// without the zero bytes that pad before the next one).
pub fn nal_units(annexb: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= annexb.len() {
        if annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let mut e = if k + 1 < starts.len() {
            starts[k + 1] - 3
        } else {
            annexb.len()
        };
        while e > s && annexb[e - 1] == 0 {
            e -= 1;
        }
        if e > s {
            out.push(&annexb[s..e]);
        }
    }
    out
}

const NAL_IDR: u8 = 5;
const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;
const NAL_AUD: u8 = 9;

fn nal_type(n: &[u8]) -> u8 {
    n[0] & 0x1f
}

/// The decoder setup a keyframe's parameter sets make: what WebCodecs'
/// `VideoDecoder.configure` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Avc {
    /// `avc1.PPCCLL` from the SPS: profile, constraint flags, level.
    pub codec: String,
    /// The `avcC` box body (ISO 14496-15): WebCodecs' `description`, which
    /// tells it the chunks are length-prefixed, not Annex-B.
    pub description: Vec<u8>,
}

impl Avc {
    /// From the first SPS and PPS in an access unit; `None` when it has
    /// either missing (only keyframes carry them).
    pub fn from_annexb(annexb: &[u8]) -> Option<Avc> {
        let nals = nal_units(annexb);
        let sps = nals.iter().find(|n| nal_type(n) == NAL_SPS)?;
        let pps = nals.iter().find(|n| nal_type(n) == NAL_PPS)?;
        if sps.len() < 4 || sps.len() > 0xffff || pps.len() > 0xffff {
            return None;
        }
        let (profile, compat, level) = (sps[1], sps[2], sps[3]);
        let mut d = Vec::with_capacity(11 + sps.len() + pps.len());
        d.extend_from_slice(&[1, profile, compat, level, 0xff, 0xe1]);
        d.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        d.extend_from_slice(sps);
        d.push(1);
        d.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        d.extend_from_slice(pps);
        // High profiles (100 and up) carry chroma format and bit depth;
        // 4:2:0 at 8 bits, no SPS extensions.
        if profile >= 100 {
            d.extend_from_slice(&[0xfc | 1, 0xf8, 0xf8, 0]);
        }
        Some(Avc {
            codec: format!("avc1.{profile:02x}{compat:02x}{level:02x}"),
            description: d,
        })
    }
}

/// An Annex-B access unit as AVCC: each NAL unit with a 4-byte big-endian
/// length, parameter sets and delimiters left out (they are in the
/// description).
pub fn to_avcc(annexb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(annexb.len() + 16);
    for n in nal_units(annexb) {
        if matches!(nal_type(n), NAL_SPS | NAL_PPS | NAL_AUD) {
            continue;
        }
        out.extend_from_slice(&(n.len() as u32).to_be_bytes());
        out.extend_from_slice(n);
    }
    out
}

/// Whether an Annex-B access unit holds an IDR slice.
pub fn has_idr(annexb: &[u8]) -> bool {
    nal_units(annexb).iter().any(|n| nal_type(n) == NAL_IDR)
}

// ---- To the window ----------------------------------------------------------
//
// One binary message per item on the window's channel, so a config always
// arrives before the chunks it is for:
//   config: [1] [codec length u16 BE] [codec, UTF-8] [avcC description]
//   chunk:  [2] [flags u8] [ts_us u64 BE] [AVCC access unit]

/// The window message kind of a decoder config.
pub const VIEW_CONFIG: u8 = 1;
/// The window message kind of an access unit.
pub const VIEW_CHUNK: u8 = 2;

/// A decoder config for the window.
pub fn view_config(a: &Avc) -> Vec<u8> {
    let c = a.codec.as_bytes();
    let mut b = Vec::with_capacity(3 + c.len() + a.description.len());
    b.push(VIEW_CONFIG);
    b.extend_from_slice(&(c.len() as u16).to_be_bytes());
    b.extend_from_slice(c);
    b.extend_from_slice(&a.description);
    b
}

/// An access unit for the window, already AVCC.
pub fn view_chunk(key: bool, ts_us: u64, avcc: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(10 + avcc.len());
    b.push(VIEW_CHUNK);
    b.push(if key { FLAG_KEY } else { 0 });
    b.extend_from_slice(&ts_us.to_be_bytes());
    b.extend_from_slice(avcc);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    // An SPS for High (100), level 4.0, a PPS, and an IDR slice.
    const SPS: &[u8] = &[0x67, 0x64, 0x00, 0x28, 0xac, 0xd9];
    const PPS: &[u8] = &[0x68, 0xeb, 0xe3, 0xcb];
    const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00, 0x33];

    fn key_au() -> Vec<u8> {
        let mut v = vec![0, 0, 0, 1, 0x09, 0xf0];
        v.extend_from_slice(&[0, 0, 0, 1]);
        v.extend_from_slice(SPS);
        v.extend_from_slice(&[0, 0, 0, 1]);
        v.extend_from_slice(PPS);
        v.extend_from_slice(&[0, 0, 1]);
        v.extend_from_slice(IDR);
        v
    }

    #[test]
    fn splits_nal_units_on_3_and_4_byte_start_codes() {
        let au = key_au();
        let n = nal_units(&au);
        assert_eq!(n, vec![&[0x09, 0xf0][..], SPS, PPS, IDR]);
        assert!(has_idr(&au));
        assert!(nal_units(&[]).is_empty());
        assert!(nal_units(&[0, 0, 1]).is_empty());
    }

    #[test]
    fn avc_config_from_a_keyframe() {
        let a = Avc::from_annexb(&key_au()).expect("config");
        assert_eq!(a.codec, "avc1.640028");
        let d = &a.description;
        assert_eq!(&d[..6], &[1, 0x64, 0x00, 0x28, 0xff, 0xe1]);
        assert_eq!(u16::from_be_bytes([d[6], d[7]]) as usize, SPS.len());
        assert_eq!(&d[8..8 + SPS.len()], SPS);
        let p = 8 + SPS.len();
        assert_eq!(d[p], 1);
        assert_eq!(u16::from_be_bytes([d[p + 1], d[p + 2]]) as usize, PPS.len());
        assert_eq!(&d[p + 3..p + 3 + PPS.len()], PPS);
        // High's trailer: 4:2:0, 8-bit, no extensions.
        assert_eq!(&d[p + 3 + PPS.len()..], &[0xfd, 0xf8, 0xf8, 0]);
        // A P-frame has no parameter sets.
        assert_eq!(Avc::from_annexb(&[0, 0, 0, 1, 0x41, 0x9a]), None);
    }

    #[test]
    fn avcc_drops_parameter_sets_and_prefixes_lengths() {
        let v = to_avcc(&key_au());
        let mut want = (IDR.len() as u32).to_be_bytes().to_vec();
        want.extend_from_slice(IDR);
        assert_eq!(v, want);
    }

    #[test]
    fn view_messages_carry_their_kind_first() {
        let a = Avc::from_annexb(&key_au()).unwrap();
        let c = view_config(&a);
        assert_eq!(c[0], VIEW_CONFIG);
        assert_eq!(u16::from_be_bytes([c[1], c[2]]), 11);
        assert_eq!(&c[3..14], b"avc1.640028");
        assert_eq!(&c[14..], &a.description[..]);
        let k = view_chunk(true, 0x0102, &[9, 9]);
        assert_eq!(k, vec![VIEW_CHUNK, FLAG_KEY, 0, 0, 0, 0, 0, 0, 1, 2, 9, 9]);
    }

    #[test]
    fn msgs_use_wire_names_and_skip_unknown() {
        let m = ShareMsg::Keyframe {
            share_id: "ab".into(),
        };
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            serde_json::json!({"t": "keyframe", "shareId": "ab"})
        );
        let w = ShareMsg::Watch {
            share_id: "ab".into(),
            on: true,
        };
        assert_eq!(
            serde_json::to_value(&w).unwrap(),
            serde_json::json!({"t": "watch", "shareId": "ab", "on": true})
        );
        let s: ShareMsg = serde_json::from_value(serde_json::json!({
            "t": "start", "v": 1, "shareId": "ab", "startedMs": 5, "kind": "window",
            "title": "T", "app": "A", "width": 2, "height": 2, "fps": 10, "kbps": 900
        }))
        .unwrap();
        assert!(matches!(s, ShareMsg::Start(ShareInfo { width: 2, .. })));
        let u: ShareMsg =
            serde_json::from_value(serde_json::json!({"t": "pointer", "x": 1})).unwrap();
        assert_eq!(u, ShareMsg::Unknown);
    }
}
