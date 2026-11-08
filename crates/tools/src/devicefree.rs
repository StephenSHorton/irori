//! The `device_free` tool (device-free plan §2): what this device can still
//! give, after margins, and who holds its heavy and check slots.

use std::path::Path;

use crate::args::Ty;
use crate::format::{MAX_FIELD_RUNES, in_, one_line, trim};
use crate::{Caller, Inner, decode, err_text, untrusted};

/// What one read of this device found; each part is `None` when it can't be
/// read here (the PC, until its card lands).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceFree {
    /// The folder the disk figure is for, as the backend resolved it.
    pub dir: String,
    /// Free and total bytes of the volume.
    pub disk: Option<(u64, u64)>,
    /// Available and total memory, bytes.
    pub mem: Option<(u64, u64)>,
    /// CPU busy 0 to 100 over about a second, and logical cores.
    pub cpu: Option<(f32, u32)>,
}

const GB: u64 = 1_000_000_000;

/// What to keep free of a disk of `total` bytes: 5%, 10 GB to 50 GB.
pub(crate) fn disk_keep(total: u64) -> u64 {
    (total / 20).clamp(10 * GB, 50 * GB)
}

/// What to keep free of `total` bytes of memory: 10%, 2 GB to 16 GB.
pub(crate) fn mem_keep(total: u64) -> u64 {
    (total / 10).clamp(2 * GB, 16 * GB)
}

/// Bytes as "1.9 TB", "92 GB", "640 MB".
pub(crate) fn size(b: u64) -> String {
    let f = b as f64;
    if b >= 1_000_000_000_000 {
        format!("{:.1} TB", f / 1e12)
    } else if b >= 10 * GB {
        format!("{:.0} GB", f / 1e9)
    } else if b >= GB {
        format!("{:.1} GB", f / 1e9)
    } else {
        format!("{:.0} MB", f / 1e6)
    }
}

const ARGS: &[(&str, Ty)] = &[("path", Ty::Str)];

pub(crate) fn device_free(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let given = trim(a.str("path"));
    let cwd = s.cwd_of(c);
    // A relative path joins to the caller's folder, never the Core's.
    let path = if given.is_empty() {
        cwd
    } else if Path::new(given).is_absolute() || cwd.is_empty() {
        if !Path::new(given).is_absolute() {
            return err_text("give an absolute path: your working folder isn't known to resolve it.");
        }
        given.to_string()
    } else {
        Path::new(&cwd).join(given).to_string_lossy().into_owned()
    };
    let Some(d) = s.b.device_free(&path) else {
        return err_text("this device's free room can't be read here.");
    };
    // Read outside any lock; from here only formatting.
    let mut b = format!("On this device, now. {}\n", untrusted());
    match d.disk {
        Some((free, total)) => {
            let keep = disk_keep(total);
            b += &format!(
                "Disk ({}): {} free of {}. Up to {} can be used (keeping {}).\n",
                one_line(&d.dir, 200),
                size(free),
                size(total),
                size(free.saturating_sub(keep)),
                size(keep)
            );
        }
        None => b += "Disk: not available here.\n",
    }
    match d.mem {
        Some((avail, total)) => {
            let keep = mem_keep(total);
            b += &format!(
                "Memory: {} available of {}. Up to {} can be used (keeping {}).\n",
                size(avail),
                size(total),
                size(avail.saturating_sub(keep)),
                size(keep)
            );
        }
        None => b += "Memory: not available here.\n",
    }
    match d.cpu {
        Some((pct, cores)) => b += &format!("CPU: {pct:.0}% busy over 1 s, {cores} cores.\n"),
        None => b += "CPU: not available here.\n",
    }
    b += &slots(s);
    b += "These numbers don't count what the jobs holding slots will still use.";
    b
}

/// The heavy and check slot lines, and the call to take one.
fn slots(s: &Inner) -> String {
    let dev = s.b.device_id();
    let family = crate::heavy_family(&dev);
    let n = s.roster();
    let now = s.now();
    let ls = s.b.coord().leases();
    let open = s.b.coord().list_tasks(&coord::TaskFilter {
        open: true,
        ..coord::TaskFilter::default()
    });
    // A holder as the plan words it: an agent by name and held card, a
    // script run as "script <run> (no card)".
    let holder = |h: &str| -> String {
        if coord::is_run_holder(h) {
            let run = h.split_once('/').map_or(h, |(_, r)| &r[coord::RUN_HOLDER.len()..]);
            return format!("script {} (no card)", one_line(run, MAX_FIELD_RUNES));
        }
        let name = n.who(h);
        let cards: Vec<String> = open
            .iter()
            .filter(|t| t.holders.iter().any(|x| x == h) && !t.short.is_empty())
            .map(|t| format!("#{}", one_line(&t.short, 20)))
            .collect();
        if cards.is_empty() {
            format!("{name} (no card)")
        } else {
            format!("{name} on {}", cards.join(", "))
        }
    };
    let mut out = String::new();
    if let Some(fam) = &family {
        let mut parts = Vec::new();
        for i in 1..=crate::HEAVY_SLOTS {
            let name = format!("{fam}/slot-{i}");
            match ls.iter().find(|l| l.resource == name) {
                Some(l) => parts.push(format!(
                    "slot-{i} held by {}, expires {}",
                    holder(&l.holder),
                    in_(&now, &l.until)
                )),
                None => parts.push(format!("slot-{i} free")),
            }
        }
        out += &format!("Heavy slots ({}): {}.\n", crate::HEAVY_SLOTS, parts.join("; "));
        // When every slot is taken, name the one to free soonest.
        let held: Vec<_> = (1..=crate::HEAVY_SLOTS)
            .filter_map(|i| {
                let name = format!("{fam}/slot-{i}");
                ls.iter().find(|l| l.resource == name)
            })
            .collect();
        if held.len() == crate::HEAVY_SLOTS
            && let Some(soon) = held.iter().min_by_key(|l| l.until.unix_nano())
        {
            out += &format!(
                "All taken; {} frees first. Message its holder, or after committing and pushing raise board_block on=\"lease {}\".\n",
                one_line(&soon.resource, MAX_FIELD_RUNES),
                one_line(&soon.resource, MAX_FIELD_RUNES)
            );
        }
        out += &format!(
            "Take one: lease resource={fam} ttl_minutes=<your estimate, at most 240>. Lease again to renew; release when done.\n"
        );
    } else {
        out += "Heavy slots: not available here (this device has no id).\n";
    }
    let check: Vec<String> = ls
        .iter()
        .filter(|l| l.resource.starts_with("check/") && l.holder.starts_with(&format!("{dev}/")))
        .map(|l| {
            format!(
                "{} held by {}, expires {}",
                one_line(&l.resource, MAX_FIELD_RUNES),
                holder(&l.holder),
                in_(&now, &l.until)
            )
        })
        .collect();
    if check.is_empty() {
        out += "Check slots: none held.\n";
    } else {
        out += &format!("Check slots: {}.\n", check.join("; "));
    }
    out
}
