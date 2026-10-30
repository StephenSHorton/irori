//! The friendly name Irori gives a Claude session nobody named (card #21;
//! I:internal/names, coord.md §3.13): a short word from a fixed list,
//! distinct from every live agent's name.

use gocompat::fnv::fnv32a;

/// The list names come from. Append only: a session's first choice is its
/// session id's hash over this list, so reordering would move names on
/// restart (`tundra` before `tulip` is as Go wrote it).
pub const WORDS: &[&str] = &[
    "acorn", "alder", "amber", "anchor", "aspen", "aster", "badger", "basil", "beacon", "birch",
    "bison", "bramble", "brook", "cedar", "cinder", "clover", "comet", "coral", "cove", "crane",
    "cricket", "cypress", "dahlia", "delta", "dune", "eagle", "ember", "fennel", "fern", "finch",
    "fjord", "flint", "forest", "fox", "garnet", "gecko", "ginger", "glacier", "grove", "harbor",
    "hazel", "heron", "hickory", "holly", "ibis", "indigo", "iris", "ivy", "jade", "jasper",
    "juniper", "kestrel", "kite", "koala", "lagoon", "lark", "laurel", "lemur", "lichen", "lily",
    "linden", "lotus", "lynx", "magpie", "maple", "marble", "marten", "meadow", "mesa", "mink",
    "minnow", "moss", "nectar", "newt", "nimbus", "oak", "ocelot", "olive", "onyx", "orca",
    "osprey", "otter", "owl", "panda", "pebble", "pelican", "pepper", "pine", "plover", "poppy",
    "prairie", "puffin", "quail", "quartz", "raven", "reed", "ridge", "robin", "rowan", "saffron",
    "sage", "salmon", "sequoia", "shale", "sparrow", "spruce", "stork", "summit", "swift", "tansy",
    "tern", "thistle", "thyme", "tiger", "topaz", "tundra", "tulip", "umber", "valley", "vireo",
    "walnut", "willow", "wren", "yarrow", "yew", "zephyr", "zinnia",
];

/// The name for `seed` (a session id): the word its hash lands on, else the
/// next word along that `taken` refuses, wrapping round; when every word is
/// taken, the same probe with `2`, then `3`, ... appended. `taken` is asked
/// with the lowercase name.
pub fn pick(seed: &str, taken: Option<&dyn Fn(&str) -> bool>) -> String {
    let n_words = WORDS.len();
    let start = (fnv32a(seed.as_bytes()) % n_words as u32) as usize;
    let mut n = 1u64;
    loop {
        for i in 0..n_words {
            let mut name = WORDS[(start + i) % n_words].to_string();
            if n > 1 {
                name.push_str(&n.to_string());
            }
            match taken {
                Some(t) if t(&name) => {}
                _ => return name,
            }
        }
        n += 1;
    }
}

/// The form names are compared in: `strings.ToLower(strings.TrimSpace(name))`.
pub fn fold(name: &str) -> String {
    gocompat::text::to_lower_str(trim_space(name))
}

/// `strings.TrimSpace`.
pub(crate) fn trim_space(s: &str) -> &str {
    s.trim_matches(gocompat::text::is_space)
}
