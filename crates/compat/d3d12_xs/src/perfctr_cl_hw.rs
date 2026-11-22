use core::ffi::c_void;
use std::time::{Duration, Instant};

use crate::device_wrap::Guid;
use crate::perfctr::{CounterEntry, CounterSet, PcSource};
use crate::perfctr_cl::{active_set, begin_counters_x, end_counters_x, read_sample, read_value, sample_counters_x};

const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_QUEUE: Guid = Guid { d1: 0x0ec8_70a6, d2: 0x5d7e, d3: 0x4c22, d4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed] };
const IID_ALLOCATOR: Guid = Guid { d1: 0x6102_dee4, d2: 0xaf59, d3: 0x4b09, d4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24] };
const IID_LIST: Guid = Guid { d1: 0x5b16_0d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
const IID_FENCE: Guid = Guid { d1: 0x0a75_3dcf, d2: 0xc4d8, d3: 0x4b91, d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76] };
const IID_FACTORY4: Guid = Guid { d1: 0x1bc6_ea02, d2: 0xef36, d3: 0x464f, d4: [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a] };
const IID_ADAPTER: Guid = Guid { d1: 0x2411_e7e1, d2: 0x12ac, d3: 0x4ccf, d4: [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0] };

const ST_COMMON: u32 = 0;
const ST_COPY_DEST: u32 = 0x400;
const ST_GENERIC_READ: u32 = 0xac3;
const HEAP_DEFAULT: u32 = 1;
const HEAP_UPLOAD: u32 = 2;
const HEAP_READBACK: u32 = 3;
const FLAG_UAV: u32 = 4;
const X_CLOSE: usize = 9;
const X_COPY_BUFFER_REGION: usize = 15;
const HZ: u64 = 853_000_000;

extern "system" {
    fn LoadLibraryW(p: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

unsafe fn vs(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

macro_rules! call {
    ($o:expr, $i:expr, ($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o = $o as *mut c_void;
        let f: unsafe extern "system" fn(*mut c_void $(, $t)*) -> $r = core::mem::transmute(vs(o, $i));
        f(o $(, $a)*)
    }};
}

unsafe fn warp_adapter() -> *mut c_void {
    let name: Vec<u16> = "dxgi.dll\0".encode_utf16().collect();
    let h = unsafe { LoadLibraryW(name.as_ptr()) };
    let p = unsafe { GetProcAddress(h, b"CreateDXGIFactory2\0".as_ptr()) };
    assert!(!p.is_null());
    let create: unsafe extern "system" fn(u32, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(p) };
    let mut f: *mut c_void = core::ptr::null_mut();
    assert!(unsafe { create(0, &IID_FACTORY4, &mut f) } >= 0);
    let mut a: *mut c_void = core::ptr::null_mut();
    assert!(call!(f, 27, (*const Guid, *mut *mut c_void) -> i32, &IID_ADAPTER, &mut a) >= 0);
    a
}

unsafe fn device() -> (usize, usize) {
    crate::enable_experimental_shader_models();
    let warp = std::env::var("TOMBOX_HW_ADAPTER").map(|v| v.eq_ignore_ascii_case("warp")).unwrap_or(false);
    let adapter = if warp { unsafe { warp_adapter() } } else { core::ptr::null_mut() };
    let p = crate::host_proc(b"D3D12CreateDevice\0");
    let create: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(p) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    let hr = create(adapter, 0xb000, &IID_DEVICE, &mut dev);
    assert!(hr >= 0 && !dev.is_null(), "D3D12CreateDevice hr={hr:#010x}");
    let desc = [0u32; 4];
    let mut q: *mut c_void = core::ptr::null_mut();
    assert!(unsafe { call!(dev, 8, (*const u32, *const Guid, *mut *mut c_void) -> i32, desc.as_ptr(), &IID_QUEUE, &mut q) } >= 0);
    (dev as usize, q as usize)
}

unsafe fn buffer(dev: usize, heap: u32, size: u64, flags: u32, state: u32) -> (usize, *mut u8) {
    let h: [u32; 5] = [heap, 0, 0, 1, 1];
    let mut d = [0u8; 56];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    d[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
    let mut r: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { call!(dev, 27, (*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32,
        h.as_ptr(), 0, d.as_ptr(), state, core::ptr::null(), &IID_RESOURCE, &mut r) };
    assert!(hr >= 0, "CreateCommittedResource hr={hr:#010x}");
    let va = unsafe { call!(r, 11, () -> u64) };
    crate::gpu_va::insert(crate::gpu_va::Entry { start: va, len: size, res: r as usize, buffer: true });
    crate::gpu_va::set_state(r as usize, state);
    let mut cpu: *mut u8 = core::ptr::null_mut();
    if heap != HEAP_DEFAULT {
        assert!(unsafe { call!(r, 8, (u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut cpu) } >= 0);
    }
    (r as usize, cpu)
}

unsafe fn list(dev: usize) -> *mut c_void {
    let mut a: *mut c_void = core::ptr::null_mut();
    assert!(unsafe { call!(dev, 9, (u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut a) } >= 0);
    let mut l: *mut c_void = core::ptr::null_mut();
    assert!(unsafe { call!(dev, 12, (u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, a, core::ptr::null_mut(), &IID_LIST, &mut l) } >= 0);
    crate::cmdlist_wrap::unwrap(crate::cmdlist_wrap::wrap(l))
}

unsafe fn run(dev: usize, q: usize, l: *mut c_void) {
    assert!(unsafe { call!(l, X_CLOSE, () -> i32) } >= 0);
    let mut f: *mut c_void = core::ptr::null_mut();
    assert!(unsafe { call!(dev, 36, (u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut f) } >= 0);
    let ls = [l];
    unsafe {
        call!(q, 10, (u32, *const *mut c_void) -> (), 1, ls.as_ptr());
        call!(q, 14, (*mut c_void, u64) -> i32, f, 1);
    }
    let t = Instant::now();
    while unsafe { call!(f, 8, () -> u64) } < 1 {
        assert!(t.elapsed() < Duration::from_secs(5), "GPU work did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn entry(block: u8, sample_offset: u32, source: PcSource) -> CounterEntry {
    CounterEntry { block, slot: 0, select: 0, instances: 1, engines: 1, wide: false, sample_offset, result_offset: sample_offset, source }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_perf_counter_clock_samples_are_measured_on_the_gpu_timeline() {
    unsafe {
        let (dev, q) = device();
        let set = CounterSet {
            scorpio: false,
            sq_ctrl: 0xffff,
            se_mask: 0xf,
            entries: vec![entry(0, 0, PcSource::ConsoleClock { hz: HZ }), entry(0xb, 8, PcSource::None), entry(2, 16, PcSource::ConsoleClock { hz: HZ })],
            sample_bytes: 24,
        };
        let set_obj = crate::perfctr::create(dev, set) as *mut c_void;
        let (title, _) = buffer(dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_COMMON);
        let (up, up_cpu) = buffer(dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        let (rb, rb_cpu) = buffer(dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        core::ptr::write_bytes(up_cpu, 0xab, 256);
        let l0 = list(dev);
        call!(l0, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), title, 0, up, 0, 256);
        run(dev, q, l0);

        let la = list(dev);
        begin_counters_x(la, set_obj);
        assert_eq!(active_set(dev), Some(set_obj as usize));
        sample_counters_x(la, title as *mut c_void, 0);
        run(dev, q, la);
        let wall = Instant::now();
        std::thread::sleep(Duration::from_millis(60));

        let lb = list(dev);
        sample_counters_x(lb, title as *mut c_void, 32);
        sample_counters_x(lb, title as *mut c_void, 4);
        call!(lb, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb, 0, title, 0, 256);
        end_counters_x(lb);
        assert_eq!(active_set(dev), None);
        sample_counters_x(lb, title as *mut c_void, 64);
        run(dev, q, lb);
        let elapsed = wall.elapsed().as_secs_f64();

        let u64_at = |o: usize| core::ptr::read_volatile(rb_cpu.add(o) as *const u64);
        let (a0, a16, b32, b48) = (u64_at(0), u64_at(16), u64_at(32), u64_at(48));
        assert_eq!(a0, a16, "two clock counters sampled at the same point differ");
        assert_eq!(b32, b48);
        assert_eq!(u64_at(8), 0xabab_abab_abab_abab, "a counter with no PC source was written");
        assert_eq!(u64_at(40), 0xabab_abab_abab_abab, "a counter with no PC source was written");
        assert!((64..256).step_by(8).all(|o| u64_at(o) == 0xabab_abab_abab_abab), "a sample after EndCountersX or with a misaligned offset was recorded");
        assert!(a0 < HZ / 2, "first sample {a0} clocks after Begin is implausibly late");
        let secs = (b32 - a0) as f64 / HZ as f64;
        assert!(secs >= 0.055 && secs <= elapsed + 0.05, "sample interval {secs:.4}s vs wall {elapsed:.4}s");
        assert_eq!(read_value(title, 0), Some(a0));
        assert_eq!(read_value(title, 16), Some(a16));
        assert_eq!(read_value(title, 32), Some(b32));
        assert_eq!(read_value(title, 8), None);
        assert_eq!(read_sample(title, 0, 24), None);
        assert_eq!(read_sample(title, 16, 8), Some(a16.to_le_bytes().to_vec()));
        println!("perf counters: first sample {a0} clocks, interval {secs:.4}s over wall {elapsed:.4}s");
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_perf_counter_readback_returns_only_executed_samples() {
    unsafe {
        let (dev, q) = device();
        let set = CounterSet { scorpio: false, sq_ctrl: 0xffff, se_mask: 0xf, entries: vec![entry(0, 0, PcSource::ConsoleClock { hz: HZ })], sample_bytes: 8 };
        let set_obj = crate::perfctr::create(dev, set) as *mut c_void;
        let (title, _) = buffer(dev, HEAP_DEFAULT, 64, FLAG_UAV, ST_COMMON);
        let (rb, rb_cpu) = buffer(dev, HEAP_READBACK, 64, 0, ST_COPY_DEST);
        let la = list(dev);
        begin_counters_x(la, set_obj);
        sample_counters_x(la, title as *mut c_void, 0);
        run(dev, q, la);
        let a = read_value(title, 0).expect("executed sample not readable");
        std::thread::sleep(Duration::from_millis(20));
        let lb = list(dev);
        sample_counters_x(lb, title as *mut c_void, 0);
        call!(lb, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb, 0, title, 0, 64);
        end_counters_x(lb);
        assert_eq!(read_value(title, 0), Some(a), "a recorded but unexecuted sample was returned");
        run(dev, q, lb);
        let b = read_value(title, 0).expect("second sample not readable");
        assert!(b > a + HZ / 100, "second sample {b} not later than first {a}");
        assert_eq!(core::ptr::read_volatile(rb_cpu as *const u64), b);
    }
}
#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_perf_counter_scarlett_set_samples_clock_entries() {
    unsafe {
        let (dev, q) = device();
        let xe = |block: u8, off: u32, source: PcSource| crate::perfctr_xs::XsEntry { block, slot: 0, select: 0, instances: 1, engines: 1, arrays: 1, harvested: 0, sample_offset: off, result_offset: off, source };
        let set = crate::perfctr_xs::XsCounterSet { lockhart: false, sq_ctrl: 0xffff, entries: vec![xe(0xb, 0, PcSource::None), xe(0xe, 8, PcSource::ConsoleClock { hz: HZ })], sample_bytes: 16 };
        let set_obj = crate::perfctr_xs::xs_create(dev, set) as *mut c_void;
        let (title, _) = buffer(dev, HEAP_DEFAULT, 64, FLAG_UAV, ST_COMMON);
        let (rb, rb_cpu) = buffer(dev, HEAP_READBACK, 64, 0, ST_COPY_DEST);
        let l = list(dev);
        begin_counters_x(l, set_obj);
        assert_eq!(active_set(dev), Some(set_obj as usize));
        sample_counters_x(l, title as *mut c_void, 0);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb, 0, title, 0, 64);
        end_counters_x(l);
        run(dev, q, l);
        let v = core::ptr::read_volatile(rb_cpu.add(8) as *const u64);
        assert!(v < HZ / 2, "clock sample {v} implausible");
        assert_eq!(read_value(title, 8), Some(v));
        assert_eq!(read_value(title, 0), None);
    }
}