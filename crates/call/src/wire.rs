//! A voice datagram: an 8-byte header and one Opus packet (20 ms).
//!
//! ```text
//! 0      1      2..4        4..8
//! v=1    flags  seq (u16)   ts (u32, 48 kHz samples)    opus packet...
//! ```
//!
//! Each peer has its own connection, so the sender is the connection's
//! device and the header names no speaker. `seq` counts the datagrams a
//! sender sent (a gap is a loss); `ts` says where the frame sits in the
//! sender's audio, so silence the sender didn't send (DTX) is a gap in `ts`
//! but not in `seq`. Big-endian throughout.

/// This build's datagram version. A datagram of another version is dropped.
pub const VERSION: u8 = 1;
/// Header bytes before the Opus packet.
pub const HEADER: usize = 8;
/// Samples in one frame: 20 ms at 48 kHz.
pub const FRAME: u32 = 960;

/// The sender is in DTX: this packet is Opus's silence frame and nothing
/// more comes until it speaks again. The receiver plays comfort noise.
pub const FLAG_DTX: u8 = 1;
/// The sender's mic is muted: the receiver shows it.
pub const FLAG_MUTED: u8 = 2;
/// A measurement click starts in this frame (call-dev's mouth-to-ear probe).
pub const FLAG_PROBE: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub flags: u8,
    pub seq: u16,
    pub ts: u32,
}

/// One datagram: the header, then the packet.
pub fn encode(h: Header, packet: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + packet.len());
    out.push(VERSION);
    out.push(h.flags);
    out.extend_from_slice(&h.seq.to_be_bytes());
    out.extend_from_slice(&h.ts.to_be_bytes());
    out.extend_from_slice(packet);
    out
}

/// The header and packet of a datagram, or None for a short one or another
/// version.
pub fn parse(d: &[u8]) -> Option<(Header, &[u8])> {
    if d.len() < HEADER || d[0] != VERSION {
        return None;
    }
    let h = Header {
        flags: d[1],
        seq: u16::from_be_bytes([d[2], d[3]]),
        ts: u32::from_be_bytes([d[4], d[5], d[6], d[7]]),
    };
    Some((h, &d[HEADER..]))
}

/// `a - b` for wrapping sequence numbers, as a signed distance.
pub fn seq_diff(a: u16, b: u16) -> i32 {
    a.wrapping_sub(b) as i16 as i32
}

/// `a - b` for wrapping 48 kHz timestamps, as a signed distance.
pub fn ts_diff(a: u32, b: u32) -> i64 {
    a.wrapping_sub(b) as i32 as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let h = Header {
            flags: FLAG_DTX | FLAG_MUTED,
            seq: 65535,
            ts: 0xdead_beef,
        };
        let d = encode(h, &[1, 2, 3]);
        assert_eq!(d.len(), HEADER + 3);
        let (g, p) = parse(&d).unwrap();
        assert_eq!(g, h);
        assert_eq!(p, &[1, 2, 3]);
    }

    #[test]
    fn refuses_short_and_other_versions() {
        assert!(parse(&[1, 0, 0, 0, 0, 0, 0]).is_none());
        let mut d = encode(
            Header {
                flags: 0,
                seq: 1,
                ts: 2,
            },
            &[9],
        );
        d[0] = 2;
        assert!(parse(&d).is_none());
    }

    #[test]
    fn wrapping_distances() {
        assert_eq!(seq_diff(1, 65535), 2);
        assert_eq!(seq_diff(65535, 1), -2);
        assert_eq!(ts_diff(100, u32::MAX - 859), 960);
        assert_eq!(ts_diff(u32::MAX - 859, 100), -960);
    }
}
