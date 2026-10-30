//! Go `hash/fnv` `New32a` (coord.md §7.16): FNV-1a, 32 bits. The names
//! list, device colours and agent colours index fixed lists with it, so the
//! value is part of their format.

const OFFSET32: u32 = 0x811c_9dc5;
const PRIME32: u32 = 0x0100_0193;

/// `fnv.New32a()` fed `data`, then `Sum32()`.
pub fn fnv32a(data: &[u8]) -> u32 {
    let mut h = OFFSET32;
    for &b in data {
        h ^= u32::from(b);
        h = h.wrapping_mul(PRIME32);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::fnv32a;

    #[test]
    fn go_vectors() {
        // coord.md §3.13 (Go 1.26).
        assert_eq!(fnv32a(b"s1"), 139_573_449);
        assert_eq!(
            fnv32a(b"0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0"),
            3_936_500_947
        );
        assert_eq!(fnv32a(b"pc"), 1_313_756_516);
        assert_eq!(fnv32a(b"mac"), 3_970_106_408);
        assert_eq!(fnv32a(b""), 0x811c_9dc5);
    }
}
