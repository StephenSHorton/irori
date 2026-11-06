//! Windows meters: one PDH query for CPU, disks and GPU engines (the
//! counters Task Manager reads), `GlobalMemoryStatusEx` for memory,
//! `GetIfEntry2` on the up Ethernet and Wi-Fi adapters for network, and D3DKMT for each GPU's name, dedicated
//! memory size and temperature. All of it is in the OS: no driver SDK.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use windows_sys::Wdk::Graphics::Direct3D::{
    D3DKMT_ADAPTER_PERFDATA, D3DKMT_ADAPTERINFO, D3DKMT_ADAPTERREGISTRYINFO, D3DKMT_ADAPTERTYPE,
    D3DKMT_CLOSEADAPTER, D3DKMT_ENUMADAPTERS2, D3DKMT_QUERYADAPTERINFO, D3DKMT_SEGMENTSIZEINFO,
    D3DKMTCloseAdapter, D3DKMTEnumAdapters2, D3DKMTQueryAdapterInfo, KMTQAITYPE_ADAPTERPERFDATA,
    KMTQAITYPE_ADAPTERREGISTRYINFO, KMTQAITYPE_ADAPTERTYPE, KMTQAITYPE_GETSEGMENTSIZE,
    KMTQUERYADAPTERINFOTYPE,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_FRIENDLY_NAME,
    GAA_FLAG_SKIP_MULTICAST, GAA_FLAG_SKIP_UNICAST, GetAdaptersAddresses, GetIfEntry2,
    IP_ADAPTER_ADDRESSES_LH, MIB_IF_ROW2,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::System::Performance::{
    PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
    PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
    PdhGetFormattedCounterArrayW, PdhGetFormattedCounterValue, PdhOpenQueryW,
};
use windows_sys::Win32::System::Power::CallNtPowerInformation;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetActiveProcessorCount;

use super::{Cpu, Disk, Gpu, Mem, Net, Sample, clamp_pct, rate};

/// PDH_FMT_NOCAP100: let `% Processor Utility` go past 100 under boost; we clamp.
const PDH_FMT_NOCAP100: u32 = 0x0000_8000;
/// PDH_CSTATUS_VALID_DATA and PDH_CSTATUS_NEW_DATA.
const PDH_OK: [u32; 2] = [0, 1];
/// POWER_INFORMATION_LEVEL ProcessorInformation.
const PROCESSOR_INFORMATION: i32 = 11;
/// ALL_PROCESSOR_GROUPS.
const ALL_GROUPS: u16 = 0xffff;
/// Below this much dedicated memory a GPU counts as integrated.
const INTEGRATED_BELOW: u64 = 1 << 30;

pub(super) struct Sampler {
    pdh: Option<Pdh>,
    base_mhz: Option<f64>,
    cores: u32,
    adapters: Vec<Adapter>,
    net: NetState,
}

impl Sampler {
    pub(super) fn new() -> Sampler {
        let pdh = Pdh::open();
        // Prime the rate counters so the next read has a delta.
        if let Some(p) = &pdh {
            unsafe { PdhCollectQueryData(p.query) };
        }
        Sampler {
            pdh,
            base_mhz: base_mhz(),
            cores: unsafe { GetActiveProcessorCount(ALL_GROUPS) },
            adapters: adapters(),
            net: NetState::new(),
        }
    }

    pub(super) fn sample(&mut self, out: &mut Sample) {
        out.cpu = Cpu {
            cores: self.cores,
            ..Cpu::default()
        };
        out.mem = mem();
        out.net = self.net.read();

        let mut engine: HashMap<u64, f64> = HashMap::new();
        let mut dedicated: HashMap<u64, u64> = HashMap::new();
        if let Some(p) = &self.pdh {
            if unsafe { PdhCollectQueryData(p.query) } == 0 {
                out.cpu.pct = p.cpu_util.and_then(single).and_then(clamp_pct);
                out.cpu.ghz = match (p.cpu_perf.and_then(single), self.base_mhz) {
                    (Some(perf), Some(mhz)) if perf > 0.0 => {
                        Some((perf / 100.0 * mhz / 1000.0) as f32)
                    }
                    _ => None,
                };
                out.disks = p.disks();
                engine = gpu_engines(p.gpu_engine.map(array).unwrap_or_default());
                for (name, v) in p.gpu_mem.map(array).unwrap_or_default() {
                    if let Some((luid, _)) = parse_luid(&name) {
                        *dedicated.entry(luid).or_default() += v.max(0.0) as u64;
                    }
                }
            }
        }

        out.gpus = self
            .adapters
            .iter()
            .map(|a| Gpu {
                name: a.name.clone(),
                integrated: a.integrated,
                util_pct: engine.get(&a.luid).copied().and_then(clamp_pct).or(
                    // An adapter with no engine instance this tick is idle.
                    self.pdh.as_ref().and(Some(0.0)),
                ),
                temp_c: a.temperature(),
                mem_used_bytes: dedicated.get(&a.luid).copied(),
                mem_total_bytes: (a.dedicated > 0).then_some(a.dedicated),
            })
            .collect();
    }
}

fn mem() -> Mem {
    let mut st = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    if unsafe { GlobalMemoryStatusEx(&mut st) } == 0 {
        return Mem::default();
    }
    Mem {
        used_bytes: st.ullTotalPhys.saturating_sub(st.ullAvailPhys),
        total_bytes: st.ullTotalPhys,
    }
}

/// The processors' base clock in MHz, which `% Processor Performance` scales.
fn base_mhz() -> Option<f64> {
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct ProcessorPowerInformation {
        number: u32,
        max_mhz: u32,
        current_mhz: u32,
        mhz_limit: u32,
        max_idle_state: u32,
        current_idle_state: u32,
    }
    let n = unsafe { GetActiveProcessorCount(ALL_GROUPS) }.max(1) as usize;
    let mut buf = vec![ProcessorPowerInformation::default(); n];
    let size = std::mem::size_of_val(buf.as_slice()) as u32;
    let st = unsafe {
        CallNtPowerInformation(
            PROCESSOR_INFORMATION,
            std::ptr::null(),
            0,
            buf.as_mut_ptr().cast(),
            size,
        )
    };
    (st == 0 && buf[0].max_mhz > 0).then(|| buf[0].max_mhz as f64)
}

// ---- network ----------------------------------------------------------------

/// IF_OPER_STATUS IfOperStatusUp.
const IF_UP: i32 = 1;
/// IF_TYPE_ETHERNET_CSMACD and IF_TYPE_IEEE80211.
const IF_TYPES: [u32; 2] = [6, 71];
/// How often the list of up adapters is read again.
const RELIST_EVERY: Duration = Duration::from_secs(30);

/// The up Ethernet and Wi-Fi adapters, listed every [`RELIST_EVERY`], and
/// each one's byte counters at the last read. Reading every interface's
/// statistics (`GetIfTable2`, PDH `Network Interface`) walks every filter
/// driver and costs most of a second; reading the few adapters that are up
/// costs a few milliseconds.
struct NetState {
    ifaces: Vec<NET_LUID_LH>,
    listed: Option<Instant>,
    prev: HashMap<u64, (u64, u64)>,
    last: Option<Instant>,
}

impl NetState {
    fn new() -> NetState {
        let mut n = NetState {
            ifaces: Vec::new(),
            listed: None,
            prev: HashMap::new(),
            last: None,
        };
        n.read();
        n
    }

    fn read(&mut self) -> Net {
        if self.listed.is_none_or(|t| t.elapsed() >= RELIST_EVERY) {
            self.ifaces = up_adapters();
            self.listed = Some(Instant::now());
        }
        let now = Instant::now();
        let secs = self.last.map(|t| (now - t).as_secs_f64()).unwrap_or(0.0);
        self.last = Some(now);
        let mut out = Net::default();
        let mut seen = HashMap::new();
        for luid in &self.ifaces {
            let mut row: MIB_IF_ROW2 = unsafe { std::mem::zeroed() };
            row.InterfaceLuid = *luid;
            if unsafe { GetIfEntry2(&mut row) } != 0 {
                continue;
            }
            let key = unsafe { luid.Value };
            let cur = (row.InOctets, row.OutOctets);
            if let Some(&(rx, tx)) = self.prev.get(&key) {
                out.rx_bits_per_s += rate(rx, cur.0, secs) * 8;
                out.tx_bits_per_s += rate(tx, cur.1, secs) * 8;
            }
            seen.insert(key, cur);
        }
        self.prev = seen;
        out
    }
}

/// The LUIDs of the Ethernet and Wi-Fi adapters that are up.
fn up_adapters() -> Vec<NET_LUID_LH> {
    let flags = GAA_FLAG_SKIP_UNICAST
        | GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_DNS_SERVER
        | GAA_FLAG_SKIP_FRIENDLY_NAME;
    let mut size = 16 * 1024u32;
    for _ in 0..3 {
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let first = buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // AF_UNSPEC.
        let st = unsafe { GetAdaptersAddresses(0, flags, std::ptr::null(), first, &mut size) };
        // ERROR_BUFFER_OVERFLOW: size now says how much it needs.
        if st == 111 {
            continue;
        }
        if st != 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut p = first;
        while !p.is_null() {
            let a = unsafe { &*p };
            if a.OperStatus == IF_UP && IF_TYPES.contains(&a.IfType) {
                out.push(a.Luid);
            }
            p = a.Next;
        }
        return out;
    }
    Vec::new()
}

// ---- PDH ----------------------------------------------------------------

struct Pdh {
    query: PDH_HQUERY,
    cpu_util: Option<PDH_HCOUNTER>,
    cpu_perf: Option<PDH_HCOUNTER>,
    disk_idle: Option<PDH_HCOUNTER>,
    disk_read: Option<PDH_HCOUNTER>,
    disk_write: Option<PDH_HCOUNTER>,
    gpu_engine: Option<PDH_HCOUNTER>,
    gpu_mem: Option<PDH_HCOUNTER>,
}

// The handles are only used from the sampler's own thread; Send lets the
// sampler move to that thread once.
unsafe impl Send for Pdh {}

impl Pdh {
    fn open() -> Option<Pdh> {
        let mut query: PDH_HQUERY = std::ptr::null_mut();
        if unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut query) } != 0 {
            return None;
        }
        let add = |path: &str| -> Option<PDH_HCOUNTER> {
            let w = wide(path);
            let mut c: PDH_HCOUNTER = std::ptr::null_mut();
            (unsafe { PdhAddEnglishCounterW(query, w.as_ptr(), 0, &mut c) } == 0).then_some(c)
        };
        Some(Pdh {
            cpu_util: add(r"\Processor Information(_Total)\% Processor Utility"),
            cpu_perf: add(r"\Processor Information(_Total)\% Processor Performance"),
            disk_idle: add(r"\PhysicalDisk(*)\% Idle Time"),
            disk_read: add(r"\PhysicalDisk(*)\Disk Read Bytes/sec"),
            disk_write: add(r"\PhysicalDisk(*)\Disk Write Bytes/sec"),
            gpu_engine: add(r"\GPU Engine(*)\Utilization Percentage"),
            gpu_mem: add(r"\GPU Adapter Memory(*)\Dedicated Usage"),
            query,
        })
    }

    fn disks(&self) -> Vec<Disk> {
        fn entry(by: &mut BTreeMap<String, Disk>, name: String) -> &mut Disk {
            by.entry(name.clone()).or_insert_with(|| Disk {
                name,
                ..Disk::default()
            })
        }
        let mut by: BTreeMap<String, Disk> = BTreeMap::new();
        for (name, v) in self.disk_idle.map(array).unwrap_or_default() {
            entry(&mut by, name).active_pct = clamp_pct(100.0 - v);
        }
        for (name, v) in self.disk_read.map(array).unwrap_or_default() {
            entry(&mut by, name).read_bytes_per_s = v.max(0.0) as u64;
        }
        for (name, v) in self.disk_write.map(array).unwrap_or_default() {
            entry(&mut by, name).write_bytes_per_s = v.max(0.0) as u64;
        }
        by.remove("_Total");
        // "0 C:" sorts before "1 D:"; disks past 9 are rare enough.
        by.into_values().collect()
    }
}

impl Drop for Pdh {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

/// One counter's value, or `None` if it has no valid data this tick.
fn single(c: PDH_HCOUNTER) -> Option<f64> {
    let mut v = PDH_FMT_COUNTERVALUE::default();
    let st = unsafe {
        PdhGetFormattedCounterValue(
            c,
            PDH_FMT_DOUBLE | PDH_FMT_NOCAP100,
            std::ptr::null_mut(),
            &mut v,
        )
    };
    (st == 0 && PDH_OK.contains(&v.CStatus)).then(|| unsafe { v.Anonymous.doubleValue })
}

/// A wildcard counter's instances and values; instances with no valid data
/// this tick (one that just appeared) are left out.
fn array(c: PDH_HCOUNTER) -> Vec<(String, f64)> {
    let fmt = PDH_FMT_DOUBLE | PDH_FMT_NOCAP100;
    // Instances come and go between the size call and the read; retry once.
    for _ in 0..3 {
        let (mut bytes, mut count) = (0u32, 0u32);
        let st = unsafe {
            PdhGetFormattedCounterArrayW(c, fmt, &mut bytes, &mut count, std::ptr::null_mut())
        };
        if st != PDH_MORE_DATA || bytes == 0 {
            return Vec::new();
        }
        // u64 words keep the items aligned.
        let mut buf = vec![0u64; (bytes as usize).div_ceil(8)];
        let items = buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        let st = unsafe { PdhGetFormattedCounterArrayW(c, fmt, &mut bytes, &mut count, items) };
        if st == PDH_MORE_DATA {
            continue;
        }
        if st != 0 {
            return Vec::new();
        }
        let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
        return items
            .iter()
            .filter(|it| PDH_OK.contains(&it.FmtValue.CStatus))
            .map(|it| unsafe { (from_wide_ptr(it.szName), it.FmtValue.Anonymous.doubleValue) })
            .collect();
    }
    Vec::new()
}

/// Task Manager's GPU utilization per adapter: each engine's use summed over
/// processes, then the busiest engine.
fn gpu_engines(items: Vec<(String, f64)>) -> HashMap<u64, f64> {
    let mut per_engine: HashMap<(u64, String), f64> = HashMap::new();
    for (name, v) in items {
        let Some((luid, rest)) = parse_luid(&name) else {
            continue;
        };
        // rest: "_phys_0_eng_3_engtype_3D"; the engine is phys + eng.
        let engine = rest.split("_engtype_").next().unwrap_or(rest).to_string();
        *per_engine.entry((luid, engine)).or_default() += v.max(0.0);
    }
    let mut out: HashMap<u64, f64> = HashMap::new();
    for ((luid, _), v) in per_engine {
        let e = out.entry(luid).or_default();
        *e = e.max(v);
    }
    out
}

/// Finds `luid_0xHHHHHHHH_0xLLLLLLLL` in a GPU counter instance and gives the
/// LUID as one number (high << 32 | low) and the text after it.
fn parse_luid(name: &str) -> Option<(u64, &str)> {
    let at = name.find("luid_0x")? + "luid_0x".len();
    let rest = &name[at..];
    let (high, rest) = rest.split_once("_0x")?;
    let low_len = rest
        .find(|c: char| !c.is_ascii_hexdigit())
        .unwrap_or(rest.len());
    let (low, rest) = rest.split_at(low_len);
    let high = u32::from_str_radix(high, 16).ok()?;
    let low = u32::from_str_radix(low, 16).ok()?;
    Some((((high as u64) << 32) | low as u64, rest))
}

// ---- GPUs via D3DKMT ------------------------------------------------------

struct Adapter {
    handle: u32,
    luid: u64,
    name: String,
    integrated: bool,
    dedicated: u64,
}

impl Adapter {
    /// The GPU's temperature, where the driver reports one (WDDM 2.4+).
    fn temperature(&self) -> Option<f32> {
        let mut perf = D3DKMT_ADAPTER_PERFDATA::default();
        query(self.handle, KMTQAITYPE_ADAPTERPERFDATA, &mut perf)?;
        // Deci-degrees Celsius; 0 means the driver doesn't report it.
        (perf.Temperature > 0).then(|| perf.Temperature as f32 / 10.0)
    }
}

impl Drop for Adapter {
    fn drop(&mut self) {
        let close = D3DKMT_CLOSEADAPTER {
            hAdapter: self.handle,
        };
        unsafe { D3DKMTCloseAdapter(&close) };
    }
}

/// The hardware GPUs (software renderers left out), most dedicated memory first.
fn adapters() -> Vec<Adapter> {
    let mut e = D3DKMT_ENUMADAPTERS2::default();
    if unsafe { D3DKMTEnumAdapters2(&mut e) } != 0 || e.NumAdapters == 0 {
        return Vec::new();
    }
    let mut infos = vec![D3DKMT_ADAPTERINFO::default(); e.NumAdapters as usize];
    e.pAdapters = infos.as_mut_ptr();
    if unsafe { D3DKMTEnumAdapters2(&mut e) } != 0 {
        return Vec::new();
    }
    infos.truncate(e.NumAdapters as usize);
    let mut out = Vec::new();
    for info in infos {
        let luid =
            ((info.AdapterLuid.HighPart as u32 as u64) << 32) | info.AdapterLuid.LowPart as u64;
        let mut a = Adapter {
            handle: info.hAdapter,
            luid,
            name: String::new(),
            integrated: false,
            dedicated: 0,
        };
        let mut kind = D3DKMT_ADAPTERTYPE::default();
        if query(a.handle, KMTQAITYPE_ADAPTERTYPE, &mut kind).is_some() {
            let bits = unsafe { kind.Anonymous.Value };
            let render = bits & 1 != 0;
            let software = bits & (1 << 2) != 0;
            if !render || software {
                continue; // dropped: closes the handle
            }
            if bits & (1 << 5) != 0 {
                a.integrated = true; // HybridIntegrated
            }
        }
        let mut reg = D3DKMT_ADAPTERREGISTRYINFO::default();
        if query(a.handle, KMTQAITYPE_ADAPTERREGISTRYINFO, &mut reg).is_some() {
            a.name = from_wide(&reg.AdapterString);
        }
        let mut seg = D3DKMT_SEGMENTSIZEINFO::default();
        if query(a.handle, KMTQAITYPE_GETSEGMENTSIZE, &mut seg).is_some() {
            a.dedicated = seg.DedicatedVideoMemorySize;
        }
        a.integrated |= a.dedicated < INTEGRATED_BELOW;
        if a.name.is_empty() {
            a.name = "GPU".into();
        }
        out.push(a);
    }
    out.sort_by(|a, b| b.dedicated.cmp(&a.dedicated));
    out
}

fn query<T>(handle: u32, kind: KMTQUERYADAPTERINFOTYPE, out: &mut T) -> Option<()> {
    let mut q = D3DKMT_QUERYADAPTERINFO {
        hAdapter: handle,
        Type: kind,
        pPrivateDriverData: (out as *mut T).cast(),
        PrivateDriverDataSize: std::mem::size_of::<T>() as u32,
    };
    (unsafe { D3DKMTQueryAdapterInfo(&mut q) } == 0).then_some(())
}

// ---- strings ----------------------------------------------------------------

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

unsafe fn from_wide_ptr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gpu_instances() {
        let (luid, rest) =
            parse_luid("pid_2780_luid_0x00000000_0x00016342_phys_0_eng_0_engtype_3D").unwrap();
        assert_eq!(luid, 0x16342);
        assert_eq!(rest, "_phys_0_eng_0_engtype_3D");
        let (luid, rest) = parse_luid("luid_0x00000001_0x0000ABCD_phys_0").unwrap();
        assert_eq!(luid, (1 << 32) | 0xABCD);
        assert_eq!(rest, "_phys_0");
        assert!(parse_luid("_Total").is_none());
    }

    #[test]
    fn busiest_engine_wins() {
        let m = gpu_engines(vec![
            (
                "pid_1_luid_0x00000000_0x00000010_phys_0_eng_0_engtype_3D".into(),
                20.0,
            ),
            (
                "pid_2_luid_0x00000000_0x00000010_phys_0_eng_0_engtype_3D".into(),
                15.0,
            ),
            (
                "pid_1_luid_0x00000000_0x00000010_phys_0_eng_3_engtype_Copy".into(),
                30.0,
            ),
            (
                "pid_1_luid_0x00000000_0x00000020_phys_0_eng_0_engtype_3D".into(),
                5.0,
            ),
        ]);
        assert_eq!(m[&0x10], 35.0);
        assert_eq!(m[&0x20], 5.0);
    }
}
