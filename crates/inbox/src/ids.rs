//! ULID-style ids (I:internal/inbox/ids.go): 48 bits of milliseconds then
//! 80 random bits, 26 Crockford base32 characters. Within one millisecond
//! (or an earlier one) the random part counts up, so ids from one generator
//! strictly increase. One generator serves msg_, dlv_ and wake_.

use std::sync::Mutex;

use gocompat::time::GoTime;

/// Crockford's base32 (no I, L, O, U), so ids sort in mint order.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Default)]
struct Last {
    ms: u64,
    rand: [u8; 10],
}

/// The id generator (Go `idGen`).
#[derive(Default)]
pub struct IdGen {
    last: Mutex<Last>,
}

impl IdGen {
    pub fn new() -> IdGen {
        IdGen::default()
    }

    /// The next id at `now` (26 characters, no prefix).
    pub fn next(&self, now: &GoTime) -> String {
        let mut g = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let ms = now.unix_milli_value().max(0) as u64;
        let ms = if ms <= g.ms {
            // Same (or an earlier) millisecond: keep the time part, count up.
            for i in (0..g.rand.len()).rev() {
                g.rand[i] = g.rand[i].wrapping_add(1);
                if g.rand[i] != 0 {
                    break;
                }
            }
            g.ms
        } else {
            g.ms = ms;
            if let Err(e) = getrandom::fill(&mut g.rand) {
                panic!("inbox: crypto/rand: {e}");
            }
            g.rand[0] &= 0x7f; // leave room to count up without wrapping
            ms
        };
        let mut raw = [0u8; 16];
        for (i, b) in raw.iter_mut().take(6).enumerate() {
            *b = (ms >> (8 * (5 - i))) as u8;
        }
        raw[6..].copy_from_slice(&g.rand);
        encode32(&raw)
    }
}

/// 128 bits as 26 base32 characters (a 130-bit number with two leading
/// zero bits).
fn encode32(raw: &[u8; 16]) -> String {
    let mut out = String::with_capacity(26);
    let mut acc: u64 = 0;
    let mut bits = 2;
    for &b in raw {
        acc = (acc << 8) | u64::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(CROCKFORD[((acc >> bits) & 31) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_sort_and_differ() {
        let g = IdGen::new();
        // 2026-01-01T00:00:00Z
        let at = GoTime::unix(1_767_225_600, 0);
        let mut prev = String::new();
        let mut seen = HashSet::new();
        for i in 0..5000i64 {
            let id = g.next(&GoTime::unix_milli(at.unix_milli_value() + i / 1000));
            assert!(
                id.len() == 26 && id > prev && !seen.contains(&id),
                "id {i} {id} after {prev}"
            );
            seen.insert(id.clone());
            prev = id;
        }
        // A clock that steps back still yields increasing ids.
        let id = g.next(&at);
        assert!(id > prev, "{id} <= {prev}");
        let g2 = IdGen::new();
        let a = g2.next(&at);
        let b = g2.next(&GoTime::unix_milli(at.unix_milli_value() + 1));
        assert_ne!(a[..10], b[..10], "time part didn't move");
    }

    /// encode32 against Go's (testdata/golden/inbox/ids.json).
    #[test]
    fn encode_matches_go() {
        let p = std::path::PathBuf::from(
            std::env::var_os("CARGO_MANIFEST_DIR")
                .unwrap_or_else(|| env!("CARGO_MANIFEST_DIR").into()),
        )
        .join("../../testdata/golden/inbox/ids.json");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
        let cases = v.as_array().unwrap();
        assert_eq!(cases.len(), 4);
        for c in cases {
            let raw = c["raw"].as_str().unwrap();
            let mut b = [0u8; 16];
            for (i, x) in b.iter_mut().enumerate() {
                *x = u8::from_str_radix(&raw[2 * i..2 * i + 2], 16).unwrap();
            }
            assert_eq!(encode32(&b), c["id"].as_str().unwrap(), "{raw}");
        }
    }

    #[test]
    fn encode_layout() {
        // All-ones: the first char carries 3 bits (7), the rest 31 (Z).
        let s = encode32(&[0xff; 16]);
        assert_eq!(s, format!("7{}", "Z".repeat(25)));
        assert_eq!(encode32(&[0; 16]), "0".repeat(26));
    }
}
