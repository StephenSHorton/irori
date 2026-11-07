//! This computer's meters, as the strip and its panel read them (card
//! #599): the React app's lib/meters.ts, its pure helpers. The readings
//! themselves are crates/window's ([`Window::meters_history`], 60 samples,
//! two minutes): that history is the ring, so the TS `pushRing` and
//! `mergeHistory` (a page keeping its own copy) have no Rust twin.
//!
//! [`Window::meters_history`]: irori_window::Window::meters_history

use irori_window::meters::{MetersCpu, MetersDisk, MetersGpu, MetersMem, MetersNet, MetersSample};

/// How many readings a graph spans: two minutes at one per 2 s.
pub const RING: usize = irori_window::meters::KEEP;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// A whole percent ("23%"), or "—" with no reading.
pub fn fmt_pct(pct: Option<f64>) -> String {
    match pct {
        Some(p) if p.is_finite() => format!("{}%", p.clamp(0.0, 100.0).round() as i64),
        _ => "—".into(),
    }
}

/// How full memory is, 0 to 100.
pub fn mem_pct(m: &MetersMem) -> Option<f64> {
    (m.total_bytes > 0).then(|| m.used_bytes as f64 / m.total_bytes as f64 * 100.0)
}

/// Bytes as gigabytes with one decimal, Task Manager's way (1024-based).
pub fn fmt_gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / GIB)
}

/// "26.2/61.6 GB (43%)".
pub fn fmt_memory(m: &MetersMem) -> String {
    if m.total_bytes == 0 {
        return "—".into();
    }
    format!(
        "{}/{} GB ({})",
        fmt_gb(m.used_bytes),
        fmt_gb(m.total_bytes),
        fmt_pct(mem_pct(m))
    )
}

/// A GPU's dedicated memory, "2.4/16.0 GB"; None when it doesn't say (an
/// integrated one shares the system's).
pub fn fmt_gpu_memory(g: &MetersGpu) -> Option<String> {
    let total = g.mem_total_bytes.filter(|t| *t > 0)?;
    Some(format!(
        "{}/{} GB",
        fmt_gb(g.mem_used_bytes.unwrap_or(0)),
        fmt_gb(total)
    ))
}

/// "23% 5.33 GHz" (no speed when it isn't read).
pub fn fmt_cpu(c: &MetersCpu) -> String {
    let pct = fmt_pct(c.pct.map(f64::from));
    match c.ghz {
        Some(g) if g > 0.0 => format!("{pct} {g:.2} GHz"),
        _ => pct,
    }
}

/// "21% (30 °C)": its use, and its temperature when read.
pub fn fmt_gpu(g: &MetersGpu) -> String {
    let pct = fmt_pct(g.util_pct.map(f64::from));
    match g.temp_c {
        Some(t) => format!("{pct} ({} °C)", t.round() as i64),
        None => pct,
    }
}

/// A number with about three digits: "280", "72.0", "8.0", "0".
fn num3(n: f64) -> String {
    if n == 0.0 {
        "0".into()
    } else if n >= 100.0 {
        format!("{n:.0}")
    } else {
        format!("{n:.1}")
    }
}

const BIT_UNITS: [&str; 3] = ["Kbps", "Mbps", "Gbps"];

/// One unit for a set of bit rates, by the largest: Kbps at the least, as
/// Task Manager shows an idle link.
fn bit_unit(max: f64) -> (f64, &'static str) {
    let mut div = 1000.0;
    let mut i = 0;
    while i < BIT_UNITS.len() - 1 && max >= div * 1000.0 {
        div *= 1000.0;
        i += 1;
    }
    (div, BIT_UNITS[i])
}

/// One bit rate: "72.0 Kbps", "1.2 Mbps".
pub fn fmt_bits(bits: f64) -> String {
    let (div, unit) = bit_unit(bits);
    format!("{} {unit}", num3(bits / div))
}

/// Sent and received in one unit: "S: 280 R: 72.0 Kbps".
pub fn fmt_net(n: &MetersNet) -> String {
    let (tx, rx) = (n.tx_bits_per_s as f64, n.rx_bits_per_s as f64);
    let (div, unit) = bit_unit(tx.max(rx));
    format!("S: {} R: {} {unit}", num3(tx / div), num3(rx / div))
}

const BYTE_UNITS: [&str; 4] = ["B/s", "KB/s", "MB/s", "GB/s"];

/// A disk's rate: "0 KB/s", "312 KB/s", "1.2 MB/s" (1024-based).
pub fn fmt_byte_rate(bytes: u64) -> String {
    let mut n = bytes as f64 / 1024.0;
    let mut i = 1;
    while i < BYTE_UNITS.len() - 1 && n >= 1024.0 {
        n /= 1024.0;
        i += 1;
    }
    format!("{} {}", num3(n), BYTE_UNITS[i])
}

/// "Disk 0 (C:)" for the host's "0 C:"; another name stays.
pub fn disk_name(name: &str) -> String {
    let t = name.trim();
    if let Some((num, rest)) = t.split_once(char::is_whitespace)
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
        && !rest.trim().is_empty()
    {
        return format!("Disk {num} ({})", rest.trim());
    }
    name.to_string()
}

/// A device in one reading, by name, so a graph follows the device even
/// when the list changes order.
pub fn gpu_by_name<'a>(s: &'a MetersSample, name: &str) -> Option<&'a MetersGpu> {
    s.gpus.iter().find(|g| g.name == name)
}

pub fn disk_by_name<'a>(s: &'a MetersSample, name: &str) -> Option<&'a MetersDisk> {
    s.disks.iter().find(|d| d.name == name)
}

/// The average of the readings there are; -1 with none.
fn average(values: impl Iterator<Item = Option<f64>>) -> f64 {
    let (mut sum, mut n) = (0.0, 0);
    for v in values.flatten() {
        if v.is_nan() {
            continue;
        }
        sum += v;
        n += 1;
    }
    if n > 0 { sum / n as f64 } else { -1.0 }
}

/// The GPU the strip shows, by name: the discrete one busiest on average
/// over the window (so the pick holds still), else the first. None with
/// no GPU.
pub fn pick_gpu(samples: &[MetersSample]) -> Option<String> {
    let last = samples.last()?;
    let first = last.gpus.first()?;
    let mut best: Option<(&str, f64)> = None;
    for g in last.gpus.iter().filter(|g| !g.integrated) {
        let avg = average(
            samples
                .iter()
                .map(|s| gpu_by_name(s, &g.name).and_then(|x| x.util_pct).map(f64::from)),
        );
        if best.is_none_or(|(_, b)| avg > b) {
            best = Some((&g.name, avg));
        }
    }
    Some(best.map_or(first.name.as_str(), |(n, _)| n).to_string())
}

/// The disk the strip shows, by name: the system disk (C:), else the
/// first. None with no disk.
pub fn pick_disk(samples: &[MetersSample]) -> Option<String> {
    let disks = &samples.last()?.disks;
    let is_c = |n: &str| {
        let u = n.to_ascii_uppercase();
        u.starts_with("C:") || u.contains(" C:") || u.contains("\tC:")
    };
    disks
        .iter()
        .find(|d| is_c(&d.name))
        .or(disks.first())
        .map(|d| d.name.clone())
}

/// Network, both ways, bits a second.
pub fn net_total(s: &MetersSample) -> f64 {
    (s.net.rx_bits_per_s + s.net.tx_bits_per_s) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(name: &str, integrated: bool, util: Option<f32>) -> MetersGpu {
        MetersGpu {
            name: name.into(),
            integrated,
            util_pct: util,
            ..MetersGpu::default()
        }
    }

    fn at(gpus: Vec<MetersGpu>, disks: &[(&str, f32)]) -> MetersSample {
        MetersSample {
            gpus,
            disks: disks
                .iter()
                .map(|(n, p)| MetersDisk {
                    name: n.to_string(),
                    active_pct: Some(*p),
                    ..MetersDisk::default()
                })
                .collect(),
            ..MetersSample::default()
        }
    }

    #[test]
    fn percent() {
        assert_eq!(fmt_pct(Some(23.4)), "23%");
        assert_eq!(fmt_pct(None), "—");
        assert_eq!(fmt_pct(Some(104.0)), "100%");
    }

    #[test]
    fn cpu_with_its_speed() {
        let c = MetersCpu {
            pct: Some(23.2),
            ghz: Some(5.33),
            cores: 16,
        };
        assert_eq!(fmt_cpu(&c), "23% 5.33 GHz");
        let none = MetersCpu {
            pct: None,
            ghz: None,
            cores: 16,
        };
        assert_eq!(fmt_cpu(&none), "—");
    }

    #[test]
    fn memory_used_of_total() {
        let m = MetersMem {
            used_bytes: (26.2 * GIB) as u64,
            total_bytes: (61.6 * GIB) as u64,
        };
        assert_eq!(fmt_memory(&m), "26.2/61.6 GB (43%)");
        assert_eq!(fmt_memory(&MetersMem::default()), "—");
    }

    #[test]
    fn network_in_one_unit_by_the_larger() {
        let n = |tx: u64, rx: u64| MetersNet {
            tx_bits_per_s: tx,
            rx_bits_per_s: rx,
        };
        assert_eq!(fmt_net(&n(280_000, 72_000)), "S: 280 R: 72.0 Kbps");
        assert_eq!(fmt_net(&n(0, 0)), "S: 0 R: 0 Kbps");
        assert_eq!(fmt_net(&n(400_000, 12_500_000)), "S: 0.4 R: 12.5 Mbps");
        assert_eq!(fmt_bits(1.2e9), "1.2 Gbps");
        assert_eq!(fmt_bits(500.0), "0.5 Kbps");
        assert_eq!(fmt_bits(72e3), "72.0 Kbps");
    }

    #[test]
    fn disk_rates_and_names() {
        assert_eq!(fmt_byte_rate(0), "0 KB/s");
        assert_eq!(fmt_byte_rate(312 * 1024), "312 KB/s");
        assert_eq!(fmt_byte_rate(1024 * 1024 * 3 / 2), "1.5 MB/s");
        assert_eq!(disk_name("0 C:"), "Disk 0 (C:)");
        assert_eq!(disk_name("disk0"), "disk0");
    }

    #[test]
    fn gpu_lines() {
        let g = MetersGpu {
            name: "NVIDIA GeForce RTX 5080".into(),
            integrated: false,
            util_pct: Some(21.0),
            temp_c: Some(30.4),
            mem_used_bytes: Some((2.4 * GIB) as u64),
            mem_total_bytes: Some((16.0 * GIB) as u64),
        };
        assert_eq!(fmt_gpu(&g), "21% (30 °C)");
        assert_eq!(fmt_gpu_memory(&g).as_deref(), Some("2.4/16.0 GB"));
        let bare = MetersGpu {
            mem_used_bytes: None,
            mem_total_bytes: None,
            ..g.clone()
        };
        assert_eq!(fmt_gpu_memory(&bare), None);
        let idle = MetersGpu {
            util_pct: None,
            temp_c: None,
            ..g
        };
        assert_eq!(fmt_gpu(&idle), "—");
    }

    #[test]
    fn the_discrete_gpu_busiest_over_the_window_else_the_first() {
        let a = |u| gpu("A", false, Some(u));
        let b = |u| gpu("B", false, Some(u));
        let igpu = || gpu("iGPU", true, Some(90.0));
        // B is busier on average though A is busier now.
        let mut window = vec![
            at(vec![igpu(), a(5.0), b(60.0)], &[]),
            at(vec![igpu(), a(5.0), b(60.0)], &[]),
            at(vec![igpu(), a(40.0), b(30.0)], &[]),
        ];
        assert_eq!(pick_gpu(&window).as_deref(), Some("B"));
        // The list changing order doesn't change the pick.
        window.push(at(vec![b(50.0), igpu(), a(10.0)], &[]));
        assert_eq!(pick_gpu(&window).as_deref(), Some("B"));
        let two_integrated = [at(vec![igpu(), gpu("i2", true, Some(10.0))], &[])];
        assert_eq!(pick_gpu(&two_integrated).as_deref(), Some("iGPU"));
        assert_eq!(pick_gpu(&[at(vec![], &[])]), None);
        assert_eq!(pick_gpu(&[]), None);
    }

    #[test]
    fn the_system_disk_else_the_first() {
        let disks = [at(vec![], &[("1 D:", 90.0), ("0 C:", 2.0)])];
        assert_eq!(pick_disk(&disks).as_deref(), Some("0 C:"));
        let no_c = [at(vec![], &[("1 D:", 9.0), ("2 E:", 50.0)])];
        assert_eq!(pick_disk(&no_c).as_deref(), Some("1 D:"));
        assert_eq!(pick_disk(&[at(vec![], &[])]), None);
        assert_eq!(pick_disk(&[]), None);
    }

    #[test]
    fn a_device_is_found_by_name() {
        let s = at(vec![gpu("X", false, Some(7.0))], &[("0 C:", 3.0)]);
        assert_eq!(gpu_by_name(&s, "X").and_then(|g| g.util_pct), Some(7.0));
        assert!(gpu_by_name(&s, "Y").is_none());
        assert_eq!(disk_by_name(&s, "0 C:").and_then(|d| d.active_pct), Some(3.0));
    }
}
