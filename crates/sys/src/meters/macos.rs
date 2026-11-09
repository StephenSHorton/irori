//! macOS meters: Mach host statistics for CPU and memory, the routing
//! sysctl's 64-bit interface counters for network, and IOKit's registry for
//! disks (`IOBlockStorageDriver` Statistics, by BSD name) and the GPU (`IOAccelerator`
//! PerformanceStatistics). None of it needs root. IOKit and CoreFoundation
//! are declared by hand, as `awake.rs` does.

// libc marks the Mach calls deprecated in favour of the mach2 crate; they
// are the same symbols, and this crate keeps to libc.
#![allow(deprecated)]

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::time::Instant;

use super::{Cpu, Disk, Gpu, Mem, Net, Sample, clamp_pct, rate};

pub(super) struct Sampler {
    cpu_prev: Option<(u64, u64)>,
    cores: u32,
    net_prev: HashMap<u16, (u64, u64)>,
    disk_prev: HashMap<String, DiskRaw>,
    last: Option<Instant>,
}

impl Sampler {
    pub(super) fn new() -> Sampler {
        let mut s = Sampler {
            cpu_prev: None,
            cores: 0,
            net_prev: HashMap::new(),
            disk_prev: HashMap::new(),
            last: None,
        };
        // Prime the deltas.
        let mut out = Sample::default();
        s.sample(&mut out);
        s
    }

    pub(super) fn sample(&mut self, out: &mut Sample) {
        let now = Instant::now();
        let secs = self.last.map(|t| (now - t).as_secs_f64()).unwrap_or(0.0);
        self.last = Some(now);

        out.cpu = self.cpu();
        out.mem = mem();
        out.net = self.net(secs);
        out.disks = self.disks(secs);
        out.gpus = gpus();
    }

    /// CPU busy % from the summed tick counters of every processor.
    fn cpu(&mut self) -> Cpu {
        let Some((busy, total, cores)) = cpu_ticks() else {
            return Cpu::default();
        };
        self.cores = cores;
        let pct = match self.cpu_prev {
            Some((b0, t0)) if total > t0 => {
                clamp_pct((busy.saturating_sub(b0)) as f64 * 100.0 / (total - t0) as f64)
            }
            _ => None,
        };
        self.cpu_prev = Some((busy, total));
        Cpu {
            pct,
            ghz: None,
            cores: self.cores,
        }
    }

    /// Bits/s over the up Ethernet-type interfaces (Wi-Fi is one too on
    /// macOS), from NET_RT_IFLIST2's 64-bit counters.
    fn net(&mut self, secs: f64) -> Net {
        let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
        let mut len: libc::size_t = 0;
        // SAFETY: a size query.
        if unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                6,
                std::ptr::null_mut(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return Net::default();
        }
        let mut buf = vec![0u8; len];
        // SAFETY: buf holds len bytes.
        if unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                6,
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return Net::default();
        }
        buf.truncate(len);
        let mut out = Net::default();
        let mut seen = HashMap::new();
        let mut at = 0usize;
        const IFT_ETHER: u8 = 0x06;
        while at + std::mem::size_of::<libc::if_msghdr>() <= buf.len() {
            // SAFETY: at least a header's worth of bytes remain; unaligned read.
            let hdr: libc::if_msghdr =
                unsafe { std::ptr::read_unaligned(buf.as_ptr().add(at).cast()) };
            let msglen = hdr.ifm_msglen as usize;
            if msglen == 0 {
                break;
            }
            if hdr.ifm_type as libc::c_int == libc::RTM_IFINFO2
                && at + std::mem::size_of::<libc::if_msghdr2>() <= buf.len()
            {
                // SAFETY: the message is an if_msghdr2 and fits.
                let m: libc::if_msghdr2 =
                    unsafe { std::ptr::read_unaligned(buf.as_ptr().add(at).cast()) };
                let data = m.ifm_data;
                let up = m.ifm_flags & libc::IFF_UP != 0;
                if up && data.ifi_type == IFT_ETHER {
                    let cur = (data.ifi_ibytes, data.ifi_obytes);
                    if let Some(&(rx, tx)) = self.net_prev.get(&m.ifm_index) {
                        out.rx_bits_per_s += rate(rx, cur.0, secs) * 8;
                        out.tx_bits_per_s += rate(tx, cur.1, secs) * 8;
                    }
                    seen.insert(m.ifm_index, cur);
                }
            }
            at += msglen;
        }
        self.net_prev = seen;
        out
    }

    /// Each disk's throughput, and its active time as the share of wall
    /// time spent in reads and writes (approximate: two overlapping
    /// requests count twice; clamped to 100). Disks are named by their
    /// whole media's BSD name ("disk0", as `diskutil` and `iostat` say);
    /// a driver with no media (an empty card reader) and mounted disk
    /// images (simulator runtimes, .dmg files) are left out.
    fn disks(&mut self, secs: f64) -> Vec<Disk> {
        let mut out = Vec::new();
        let mut seen = HashMap::new();
        for entry in services(c"IOBlockStorageDriver") {
            if entry.is_disk_image() {
                continue;
            }
            let Some(name) = entry.search_string("BSD Name", K_IO_REGISTRY_ITERATE_RECURSIVELY)
            else {
                continue;
            };
            let Some(stats) = entry.dict_property("Statistics") else {
                continue;
            };
            let raw = DiskRaw {
                read_bytes: stats.u64("Bytes (Read)").unwrap_or(0),
                write_bytes: stats.u64("Bytes (Write)").unwrap_or(0),
                busy_ns: stats.u64("Total Time (Read)").unwrap_or(0)
                    + stats.u64("Total Time (Write)").unwrap_or(0),
            };
            let mut d = Disk {
                name: name.clone(),
                ..Disk::default()
            };
            if let Some(p) = self.disk_prev.get(&name)
                && secs > 0.0
            {
                d.read_bytes_per_s = rate(p.read_bytes, raw.read_bytes, secs);
                d.write_bytes_per_s = rate(p.write_bytes, raw.write_bytes, secs);
                let busy = raw.busy_ns.saturating_sub(p.busy_ns) as f64;
                d.active_pct = clamp_pct(busy / (secs * 1e9) * 100.0);
            }
            seen.insert(name, raw);
            out.push(d);
        }
        self.disk_prev = seen;
        // disk0 first, then by number (disk2 before disk10).
        out.sort_by_key(|d| {
            let n = d
                .name
                .trim_start_matches("disk")
                .parse::<u32>()
                .unwrap_or(u32::MAX);
            (n, d.name.clone())
        });
        out
    }
}

#[derive(Clone, Copy)]
struct DiskRaw {
    read_bytes: u64,
    write_bytes: u64,
    busy_ns: u64,
}

/// Summed busy and total ticks over every processor, and the processor count.
fn cpu_ticks() -> Option<(u64, u64, u32)> {
    let mut count: libc::natural_t = 0;
    let mut info: libc::processor_info_array_t = std::ptr::null_mut();
    let mut info_cnt: libc::mach_msg_type_number_t = 0;
    // SAFETY: out pointers are valid; the array is freed below.
    let kr = unsafe {
        libc::host_processor_info(
            libc::mach_host_self(),
            libc::PROCESSOR_CPU_LOAD_INFO,
            &mut count,
            &mut info,
            &mut info_cnt,
        )
    };
    if kr != libc::KERN_SUCCESS || info.is_null() {
        return None;
    }
    let states = libc::CPU_STATE_MAX as usize;
    // SAFETY: the kernel returned info_cnt integers.
    let ticks = unsafe { std::slice::from_raw_parts(info, info_cnt as usize) };
    let (mut busy, mut total) = (0u64, 0u64);
    for cpu in ticks.chunks_exact(states) {
        let t = |i: libc::c_int| cpu[i as usize] as u32 as u64;
        let idle = t(libc::CPU_STATE_IDLE);
        let used = t(libc::CPU_STATE_USER) + t(libc::CPU_STATE_SYSTEM) + t(libc::CPU_STATE_NICE);
        busy += used;
        total += used + idle;
    }
    // SAFETY: the array host_processor_info allocated in our task.
    unsafe {
        libc::vm_deallocate(
            libc::mach_task_self(),
            info as libc::vm_address_t,
            info_cnt as libc::vm_size_t * std::mem::size_of::<libc::integer_t>(),
        );
    }
    Some((busy, total, count))
}

/// CPU busy over `window`, from two tick reads that far apart. No shared state.
pub(super) fn cpu_now(window: std::time::Duration) -> Option<Cpu> {
    let (b0, t0, _) = cpu_ticks()?;
    std::thread::sleep(window);
    let (b1, t1, cores) = cpu_ticks()?;
    let pct = if t1 > t0 {
        clamp_pct(b1.saturating_sub(b0) as f64 * 100.0 / (t1 - t0) as f64)
    } else {
        None
    };
    Some(Cpu {
        pct,
        ghz: None,
        cores,
    })
}

/// Memory now, or `None` when the read failed.
pub(super) fn mem_now() -> Option<Mem> {
    mem_read()
}

/// Memory used as Activity Monitor counts it: app memory (internal pages
/// less purgeable) plus wired plus compressed.
fn mem() -> Mem {
    mem_read().unwrap_or_else(|| Mem {
        used_bytes: 0,
        total_bytes: sysctl_u64(c"hw.memsize").unwrap_or(0),
    })
}

fn mem_read() -> Option<Mem> {
    let total = sysctl_u64(c"hw.memsize").unwrap_or(0);
    let page = sysctl_u64(c"hw.pagesize").unwrap_or(16384);
    // SAFETY: a zeroed plain-data struct.
    let mut st: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: st is a vm_statistics64 and count its size in integers.
    let kr = unsafe {
        libc::host_statistics64(
            libc::mach_host_self(),
            libc::HOST_VM_INFO64,
            (&mut st as *mut libc::vm_statistics64).cast(),
            &mut count,
        )
    };
    if kr != libc::KERN_SUCCESS || total == 0 {
        return None;
    }
    let internal = st.internal_page_count as u64;
    let purgeable = st.purgeable_count as u64;
    let wired = st.wire_count as u64;
    let compressed = st.compressor_page_count as u64;
    let used = (internal.saturating_sub(purgeable) + wired + compressed) * page;
    Some(Mem {
        used_bytes: used.min(total),
        total_bytes: total,
    })
}

/// The GPU's busy % and the memory it holds (Apple Silicon shares memory,
/// so there is no total of its own).
fn gpus() -> Vec<Gpu> {
    let mut out = Vec::new();
    for entry in services(c"IOAccelerator") {
        let Some(perf) = entry.dict_property("PerformanceStatistics") else {
            continue;
        };
        out.push(Gpu {
            name: entry
                .string_property("model")
                .unwrap_or_else(|| "GPU".into()),
            integrated: true,
            util_pct: perf
                .u64("Device Utilization %")
                .and_then(|v| clamp_pct(v as f64)),
            temp_c: None,
            mem_used_bytes: perf.u64("In use system memory"),
            mem_total_bytes: None,
        });
    }
    out
}

fn sysctl_u64(name: &CStr) -> Option<u64> {
    let mut v: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: v is 8 writable bytes; both these names are 64-bit.
    let r = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut v as *mut u64).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (r == 0).then_some(v)
}

// ---- IOKit and CoreFoundation -------------------------------------------------

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFAllocatorRef = *const c_void;
type CFIndex = isize;
type CFTypeID = usize;
type IoObject = libc::mach_port_t;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_CF_NUMBER_SINT64_TYPE: CFIndex = 4;
/// kIORegistryIterateRecursively / kIORegistryIterateParents.
const K_IO_REGISTRY_ITERATE_RECURSIVELY: u32 = 1;
const K_IO_REGISTRY_ITERATE_PARENTS: u32 = 2;
/// kIOServicePlane.
const IO_SERVICE_PLANE: &CStr = c"IOService";
/// kIOMainPortDefault.
const IO_MAIN_PORT_DEFAULT: libc::mach_port_t = 0;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithBytes(
        alloc: CFAllocatorRef,
        bytes: *const u8,
        num_bytes: CFIndex,
        encoding: u32,
        is_external: u8,
    ) -> CFStringRef;
    fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: CFIndex, encoding: u32) -> u8;
    fn CFDictionaryGetValue(d: CFDictionaryRef, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(n: CFTypeRef, kind: CFIndex, out: *mut c_void) -> u8;
    fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    fn CFNumberGetTypeID() -> CFTypeID;
    fn CFDictionaryGetTypeID() -> CFTypeID;
    fn CFStringGetTypeID() -> CFTypeID;
    fn CFDataGetTypeID() -> CFTypeID;
    fn CFDataGetLength(d: CFTypeRef) -> CFIndex;
    fn CFDataGetBytePtr(d: CFTypeRef) -> *const u8;
    fn CFRelease(cf: CFTypeRef);
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFDictionaryRef;
    fn IOServiceGetMatchingServices(
        main_port: libc::mach_port_t,
        matching: CFDictionaryRef,
        existing: *mut IoObject,
    ) -> libc::kern_return_t;
    fn IOIteratorNext(iter: IoObject) -> IoObject;
    fn IOObjectRelease(obj: IoObject) -> libc::kern_return_t;
    fn IORegistryEntryCreateCFProperty(
        entry: IoObject,
        key: CFStringRef,
        alloc: CFAllocatorRef,
        options: u32,
    ) -> CFTypeRef;
    fn IORegistryEntrySearchCFProperty(
        entry: IoObject,
        plane: *const c_char,
        key: CFStringRef,
        alloc: CFAllocatorRef,
        options: u32,
    ) -> CFTypeRef;
}

/// A CF object we own one reference to.
struct Cf(CFTypeRef);

impl Drop for Cf {
    fn drop(&mut self) {
        // SAFETY: a reference this wrapper owns.
        unsafe { CFRelease(self.0) };
    }
}

impl Cf {
    fn string(s: &str) -> Option<Cf> {
        // SAFETY: bytes and length come from a live &str.
        let r = unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                s.as_ptr(),
                s.len() as CFIndex,
                K_CF_STRING_ENCODING_UTF8,
                0,
            )
        };
        (!r.is_null()).then(|| Cf(r))
    }

    /// A number in this dictionary, as u64.
    fn u64(&self, key: &str) -> Option<u64> {
        let k = Cf::string(key)?;
        // SAFETY: self is a CFDictionary (checked when made); the value is borrowed.
        let v = unsafe { CFDictionaryGetValue(self.0, k.0) };
        if v.is_null() || unsafe { CFGetTypeID(v) } != unsafe { CFNumberGetTypeID() } {
            return None;
        }
        let mut n: i64 = 0;
        // SAFETY: v is a CFNumber; n is 8 writable bytes.
        let ok =
            unsafe { CFNumberGetValue(v, K_CF_NUMBER_SINT64_TYPE, (&mut n as *mut i64).cast()) };
        (ok != 0 && n >= 0).then_some(n as u64)
    }
}

/// A registry entry we hold.
struct Entry(IoObject);

impl Drop for Entry {
    fn drop(&mut self) {
        // SAFETY: an object IOIteratorNext gave us.
        unsafe { IOObjectRelease(self.0) };
    }
}

impl Entry {
    fn property(&self, key: &str) -> Option<Cf> {
        let k = Cf::string(key)?;
        // SAFETY: a live entry and key; the result is ours to release.
        let v = unsafe { IORegistryEntryCreateCFProperty(self.0, k.0, std::ptr::null(), 0) };
        (!v.is_null()).then(|| Cf(v))
    }

    fn dict_property(&self, key: &str) -> Option<Cf> {
        let v = self.property(key)?;
        // SAFETY: a live CF object.
        (unsafe { CFGetTypeID(v.0) } == unsafe { CFDictionaryGetTypeID() }).then_some(v)
    }

    /// A string property; IOKit gives some ("model") as data holding a C string.
    fn string_property(&self, key: &str) -> Option<String> {
        cf_string(&self.property(key)?)
    }

    /// A property found on this entry or, per `options`, its children or
    /// parents in the service plane.
    fn search(&self, key: &str, options: u32) -> Option<Cf> {
        let k = Cf::string(key)?;
        // SAFETY: a live entry, plane name and key; the result is ours to release.
        let v = unsafe {
            IORegistryEntrySearchCFProperty(
                self.0,
                IO_SERVICE_PLANE.as_ptr(),
                k.0,
                std::ptr::null(),
                options,
            )
        };
        (!v.is_null()).then(|| Cf(v))
    }

    fn search_string(&self, key: &str, options: u32) -> Option<String> {
        cf_string(&self.search(key, options)?)
    }

    /// Whether this block storage driver sits on a mounted disk image: its
    /// device's interconnect is "Virtual Interface" (both the user-space
    /// and in-kernel disk image devices say so).
    fn is_disk_image(&self) -> bool {
        let Some(pc) = self.search(
            "Protocol Characteristics",
            K_IO_REGISTRY_ITERATE_PARENTS | K_IO_REGISTRY_ITERATE_RECURSIVELY,
        ) else {
            return false;
        };
        // SAFETY: a live CF object.
        if unsafe { CFGetTypeID(pc.0) } != unsafe { CFDictionaryGetTypeID() } {
            return false;
        }
        let Some(k) = Cf::string("Physical Interconnect") else {
            return false;
        };
        // SAFETY: pc is a CFDictionary; the value is borrowed, not released.
        let v = unsafe { CFDictionaryGetValue(pc.0, k.0) };
        if v.is_null() {
            return false;
        }
        cf_string_ref(v).as_deref() == Some("Virtual Interface")
    }
}

/// A CFString, or CFData holding a C string, as a String.
fn cf_string(v: &Cf) -> Option<String> {
    cf_string_ref(v.0)
}

fn cf_string_ref(v: CFTypeRef) -> Option<String> {
    // SAFETY: a live CF object, typed before use.
    unsafe {
        let id = CFGetTypeID(v);
        if id == CFStringGetTypeID() {
            let mut buf = [0 as c_char; 256];
            if CFStringGetCString(v, buf.as_mut_ptr(), 256, K_CF_STRING_ENCODING_UTF8) != 0 {
                return Some(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned());
            }
        } else if id == CFDataGetTypeID() {
            let n = CFDataGetLength(v).max(0) as usize;
            let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(v), n);
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(n);
            let s = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// The registry entries of an IOKit class.
fn services(class: &CStr) -> Vec<Entry> {
    // SAFETY: a C string; the dictionary is consumed by the next call.
    let matching = unsafe { IOServiceMatching(class.as_ptr()) };
    if matching.is_null() {
        return Vec::new();
    }
    let mut iter: IoObject = 0;
    // SAFETY: matching is consumed; iter is a valid out pointer.
    if unsafe { IOServiceGetMatchingServices(IO_MAIN_PORT_DEFAULT, matching, &mut iter) } != 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    loop {
        // SAFETY: a live iterator.
        let e = unsafe { IOIteratorNext(iter) };
        if e == 0 {
            break;
        }
        out.push(Entry(e));
    }
    // SAFETY: the iterator we got above.
    unsafe { IOObjectRelease(iter) };
    out
}
