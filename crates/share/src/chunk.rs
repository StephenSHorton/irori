//! One encoded access unit, and telling a keyframe from the bytes.

use bytes::Bytes;

/// One encoded picture: Annex-B NAL units, with SPS and PPS in band before
/// every IDR.
#[derive(Clone, Debug)]
pub struct Chunk {
    /// The source frame's time, in microseconds from the share's start.
    pub timestamp_us: u64,
    pub data: Bytes,
    /// It decodes on its own (an IDR): a viewer can start here.
    pub keyframe: bool,
}

impl Chunk {
    pub(crate) fn from_encoded(e: moq_video::encode::Encoded) -> Self {
        let keyframe = is_keyframe(&e.payload);
        Self {
            timestamp_us: e.timestamp.as_micros() as u64,
            data: e.payload,
            keyframe,
        }
    }
}

/// Whether an Annex-B H.264 access unit holds an IDR slice (NAL type 5).
/// moq-video 0.1.21's `Encoded` no longer carries a keyframe flag, so read
/// it from the bitstream.
pub fn is_keyframe(annexb: &[u8]) -> bool {
    nal_types(annexb).any(|t| t == 5)
}

/// The NAL unit types in an Annex-B stream, in order.
pub(crate) fn nal_types(annexb: &[u8]) -> impl Iterator<Item = u8> + '_ {
    let mut i = 0;
    std::iter::from_fn(move || {
        // Find the next 00 00 01 start code (a 4-byte 00 00 00 01 ends in one).
        while i + 3 < annexb.len() {
            if annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 1 {
                let header = annexb[i + 3];
                i += 3;
                return Some(header & 0x1f);
            }
            i += 1;
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_idr_after_parameter_sets() {
        // SPS (7), PPS (8), IDR (5), with 4- and 3-byte start codes.
        let au = [
            0, 0, 0, 1, 0x67, 0xaa, 0, 0, 0, 1, 0x68, 0xbb, 0, 0, 1, 0x65, 0x88, 0x84,
        ];
        assert_eq!(nal_types(&au).collect::<Vec<_>>(), vec![7, 8, 5]);
        assert!(is_keyframe(&au));
    }

    #[test]
    fn non_idr_slice_is_not_a_keyframe() {
        let au = [0, 0, 0, 1, 0x09, 0xf0, 0, 0, 0, 1, 0x41, 0x9a, 0x02];
        assert_eq!(nal_types(&au).collect::<Vec<_>>(), vec![9, 1]);
        assert!(!is_keyframe(&au));
        assert!(!is_keyframe(&[]));
    }
}
