//! Each of the person's devices wears a colour (card #22;
//! I:internal/devcolor, coord.md §2.7, §3.13): a default nobody else has, or
//! the person's replicated pick (a `device.color` row).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use gocompat::fnv::fnv32a;
use gocompat::time::GoTime;

use crate::gojson::decode;

/// The replicated row a pick writes: `{deviceId, color, at}`.
pub const KIND: &str = store::kinds::DEVICE_COLOR;

/// The colours a device can wear, by name. Append only: a default is its
/// device id's hash over this list.
pub const PALETTE: &[&str] = &["teal", "orange", "pink", "lime", "cyan", "indigo", "sand"];

/// Whether `c` is a palette colour.
pub fn valid(c: &str) -> bool {
    PALETTE.contains(&c)
}

/// One device to colour: its id and when it joined the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub at: GoTime,
}

/// Colours devices: a picked colour wins; the rest, in join order (then
/// id), take the colour their id hashes to or the next one along nobody
/// holds, falling back to the hash colour when all are used.
pub fn assign(devices: &[Device], chosen: &HashMap<String, String>) -> BTreeMap<String, String> {
    let mut ds = devices.to_vec();
    ds.sort_by(|a, b| crate::gotime::cmp(&a.at, &b.at).then_with(|| a.id.cmp(&b.id)));
    let mut out = BTreeMap::new();
    let mut used: HashSet<&str> = HashSet::new();
    for d in &ds {
        if let Some(c) = chosen.get(&d.id)
            && valid(c)
        {
            out.insert(d.id.clone(), c.clone());
            used.insert(PALETTE.iter().find(|p| **p == c).copied().unwrap_or(""));
        }
    }
    for d in &ds {
        if out.contains_key(&d.id) {
            continue;
        }
        let start = (fnv32a(d.id.as_bytes()) % PALETTE.len() as u32) as usize;
        let mut c = PALETTE[start];
        for i in 0..PALETTE.len() {
            let p = PALETTE[(start + i) % PALETTE.len()];
            if !used.contains(p) {
                c = p;
                break;
            }
        }
        out.insert(d.id.clone(), c.to_string());
        used.insert(c);
    }
    out
}

#[derive(Debug, Clone)]
struct Pick {
    color: String,
    hlc: u64,
    device: String,
    seq: i64,
}

fn newer(a: &Pick, b: &Pick) -> bool {
    if a.hlc != b.hlc {
        return a.hlc > b.hlc;
    }
    if a.device != b.device {
        return a.device > b.device;
    }
    a.seq > b.seq
}

crate::go_struct! {
    #[derive(Debug, Clone, Default)]
    struct Row {
        device_id: String = "deviceId",
        color: String = "color",
    }
}

/// Folds `device.color` rows: the person's pick per device. Safe for
/// concurrent use.
#[derive(Debug, Default)]
pub struct Picks {
    m: Mutex<HashMap<String, Pick>>,
}

impl Picks {
    /// An empty fold.
    pub fn new() -> Picks {
        Picks::default()
    }

    /// Folds one row; reports whether the pick changed. Only the person
    /// picks (actor `user`).
    pub fn apply(&self, ev: &store::Event) -> bool {
        if ev.kind != KIND || ev.actor != "user" {
            return false;
        }
        let Some(r) = decode::<Row>(&ev.payload) else {
            return false;
        };
        if r.device_id.is_empty() || (!r.color.is_empty() && !valid(&r.color)) {
            return false;
        }
        let n = Pick {
            color: r.color,
            hlc: ev.hlc.0,
            device: ev.device_id.clone(),
            seq: ev.seq,
        };
        let mut m = self.m.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cur) = m.get(&r.device_id)
            && !newer(&n, cur)
        {
            return false;
        }
        m.insert(r.device_id, n);
        true
    }

    /// The person's pick per device (`""` picks are left out).
    pub fn chosen(&self) -> HashMap<String, String> {
        let m = self.m.lock().unwrap_or_else(|e| e.into_inner());
        m.iter()
            .filter(|(_, p)| !p.color.is_empty())
            .map(|(id, p)| (id.clone(), p.color.clone()))
            .collect()
    }
}
