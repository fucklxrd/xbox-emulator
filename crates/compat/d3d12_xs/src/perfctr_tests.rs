use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crate::device_wrap::Guid;
use crate::perfctr::*;

const IUNKNOWN: Guid = Guid { d1: 0, d2: 0, d3: 0, d4: [0xc0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const G1: Guid = Guid { d1: 0x1111_2222, d2: 3, d3: 4, d4: [5, 6, 7, 8, 9, 10, 11, 12] };

pub(crate) static LOCK: Mutex<()> = Mutex::new(());

fn empty_desc() -> [u8; DESC_BYTES] {
    let mut d = [0u8; DESC_BYTES];
    d[0x10..].fill(0xff);
    d
}

fn put(d: &mut [u8; DESC_BYTES], block: usize, slot: usize, sel: u32) {
    let o = DESC_OFFSET[block] as usize + 4 * slot;
    d[o..o + 4].copy_from_slice(&sel.to_le_bytes());
}

#[test]
fn desc_and_result_layout_follow_the_release_ctor() {
    assert_eq!(DESC_OFFSET[32] as usize + 4 * 4, DESC_BYTES);
    for b in 0..BLOCKS - 1 {
        let slots = (DESC_OFFSET[b + 1] - DESC_OFFSET[b]) / 4;
        assert!(slots as u8 >= DURANGO[b].slots && slots as u8 >= SCORPIO[b].slots, "block {b}");
        assert!(slots as u8 == DURANGO[b].slots.max(SCORPIO[b].slots), "block {b}");
        assert_eq!(RESULT_BASE[b] + slots * RESULT_STRIDE[b], RESULT_BASE[b + 1], "block {b}");
    }
    assert_eq!(RESULT_BASE[32] + 4 * RESULT_STRIDE[32], RESULT_BYTES);
    for b in 0..BLOCKS {
        let d = 8 * DURANGO[b].instances as u32 * DURANGO[b].engines as u32;
        let s = 8 * SCORPIO[b].instances as u32 * SCORPIO[b].engines as u32;
        assert_eq!(RESULT_STRIDE[b], d.max(s), "block {b}");
    }
}

#[test]
fn empty_desc_has_no_entries_and_the_ctor_defaults() {
    let s = build(&empty_desc(), 2);
    assert!(s.entries.is_empty());
    assert_eq!(s.sample_bytes, 0);
    assert_eq!((s.sq_ctrl, s.se_mask, s.scorpio), (0xffff, 0xf, false));
    let mut d = empty_desc();
    d[8..12].copy_from_slice(&0x1234_5603u32.to_le_bytes());
    d[0xc..0x10].copy_from_slice(&0x0007_0041u32.to_le_bytes());
    let s = build(&d, 4);
    assert_eq!((s.sq_ctrl, s.se_mask, s.scorpio), (0x41, 0x03, true));
}

#[test]
fn entries_offsets_and_sizes_durango() {
    let mut d = empty_desc();
    put(&mut d, 0, 1, 0);
    put(&mut d, 5, 2, 0x17);
    put(&mut d, 0x15, 3, 5);
    let s = build(&d, 2);
    assert_eq!(s.entries.len(), 3);
    let g = s.entries[0];
    assert_eq!((g.block, g.slot, g.sample_offset, g.result_offset, g.wide), (0, 0, 0, 0x10, false));
    assert_eq!(g.source, PcSource::ConsoleClock { hz: DURANGO_HZ });
    let cb = s.entries[1];
    assert_eq!((cb.slot, cb.instances, cb.engines, cb.sample_offset, cb.result_offset), (0, 2, 2, 8, 0x58 + 2 * 0x40));
    assert_eq!(cb.bytes(), 32);
    assert_eq!(cb.source, PcSource::None);
    let mc = s.entries[2];
    assert_eq!((mc.wide, mc.values(), mc.sample_offset, mc.result_offset), (true, 1, 40, 0x1c58 + 3 * 8));
    assert_eq!(s.sample_bytes, 48);
}

#[test]
fn scorpio_uses_its_own_table() {
    let mut d = empty_desc();
    put(&mut d, 0xc, 4, 9);
    put(&mut d, 9, 7, 1);
    let s = build(&d, 5);
    assert!(s.scorpio);
    assert_eq!(s.entries.len(), 2);
    assert_eq!((s.entries[0].block, s.entries[0].bytes(), s.entries[0].result_offset), (9, 32, 0x3d8 + 7 * 0x20));
    assert_eq!((s.entries[1].block, s.entries[1].bytes(), s.entries[1].result_offset), (0xc, 320, 0x758 + 4 * 0x140));
    assert_eq!(s.sample_bytes, 352);
    let durango = build(&d, 2);
    assert!(durango.entries.is_empty());
}

#[test]
fn sq_slot_after_a_gap_is_dropped_once_one_is_used() {
    let mut d = empty_desc();
    put(&mut d, 0xb, 0, 4);
    put(&mut d, 0xb, 2, 5);
    put(&mut d, 0xb, 3, 6);
    let s = build(&d, 2);
    let sq: Vec<_> = s.entries.iter().map(|e| (e.select, e.slot, e.result_offset)).collect();
    assert_eq!(sq, vec![(4, 0, 0x558), (6, 1, 0x558 + 3 * 0x20)]);
    let mut d = empty_desc();
    put(&mut d, 0xb, 1, 7);
    let s = build(&d, 2);
    assert_eq!(s.entries[0].result_offset, 0x558 + 0x20);
    let mut d = empty_desc();
    put(&mut d, 0xa, 0, 1);
    put(&mut d, 0xa, 2, 2);
    assert_eq!(build(&d, 2).entries.len(), 2);
}

#[test]
fn only_clock_counting_selects_have_a_pc_source() {
    assert_eq!(pc_source_for(2, 0, 0), PcSource::ConsoleClock { hz: DURANGO_HZ });
    assert_eq!(pc_source_for(3, 0, 0x40), PcSource::ConsoleClock { hz: DURANGO_S_HZ });
    assert_eq!(pc_source_for(4, 2, 0x400), PcSource::ConsoleClock { hz: SCORPIO_HZ });
    assert_eq!(pc_source_for(5, 3, 0), PcSource::ConsoleClock { hz: SCORPIO_HZ });
    assert_eq!(pc_source_for(2, 4, 0), PcSource::ConsoleClock { hz: DURANGO_HZ });
    assert_eq!(pc_source_for(2, 0, 2), PcSource::None);
    assert_eq!(pc_source_for(2, 2, 1), PcSource::None);
    for b in [1u8, 5, 6, 7, 8, 9, 0xa, 0xb, 0xc, 0x11, 0x12, 0x13, 0x14, 0x15, 0x20] {
        assert_eq!(pc_source_for(2, b, 0), PcSource::None, "block {b:#x}");
    }
    assert_eq!(pc_source(true, 0, 0), PcSource::ConsoleClock { hz: SCORPIO_HZ });
}

#[test]
fn resolve_fills_ff_and_copies_entries() {
    let mut d = empty_desc();
    put(&mut d, 0, 0, 0);
    put(&mut d, 6, 1, 3);
    let s = build(&d, 2);
    let mut out = [0u8; RESULT_BYTES as usize];
    resolve_into(&s, |e, i| if e.block == 0 { Some(0x1122_3344_5566_7788) } else if i == 2 { Some(9) } else { None }, &mut out);
    assert_eq!(&out[0..8], &[0xf8, 0x1e, 0, 0, 2, 0, 0, 0]);
    assert_eq!(u64::from_le_bytes(out[8..16].try_into().unwrap()), 0x1122_3344_5566_7788);
    let db = 0x158 + 0x40;
    assert_eq!(u64::from_le_bytes(out[db + 16..db + 24].try_into().unwrap()), 9);
    assert!(out[16..db + 16].iter().all(|&b| b == 0xff));
    assert!(out[db + 24..].iter().all(|&b| b == 0xff));
}

#[test]
fn raw_values_come_from_memory_or_the_sampler() {
    let mut d = empty_desc();
    put(&mut d, 0, 0, 0);
    put(&mut d, 5, 0, 1);
    let s = build(&d, 2);
    let mem: Vec<u8> = (0..40u8).collect();
    let cb = &s.entries[1];
    assert_eq!(raw_value(Some(&mem), None, 1, 0, cb, 3), None);
    assert_eq!(raw_value(Some(&mem), None, 1, 0, &s.entries[0], 0), Some(u64::from_le_bytes(mem[0..8].try_into().unwrap())));
    assert_eq!(raw_value(Some(&mem[..4]), None, 1, 0, &s.entries[0], 0), None);
    fn rd(res: usize, off: u64) -> Option<u64> {
        Some(res as u64 * 1000 + off)
    }
    assert_eq!(raw_value(None, Some(rd), 7, 0x100, &s.entries[0], 0), Some(7256));
    assert_eq!(raw_value(Some(&mem), Some(rd), 7, 0x100, &s.entries[0], 0), Some(7256));
    fn miss(_: usize, _: u64) -> Option<u64> {
        None
    }
    assert_eq!(raw_value(Some(&mem), Some(miss), 7, 0x100, &s.entries[0], 0), Some(u64::from_le_bytes(mem[0..8].try_into().unwrap())));
    assert_eq!(raw_value(None, Some(rd), 7, 0x100, cb, 0), None);
    assert_eq!(raw_value(None, None, 7, 0x100, &s.entries[0], 0), None);
}

#[repr(C)]
pub(crate) struct FakeCom {
    vt: *const usize,
    pub(crate) refs: AtomicU32,
}

unsafe extern "system" fn fake_addref(this: *mut FakeCom) -> u32 {
    unsafe { &*this }.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn fake_release(this: *mut FakeCom) -> u32 {
    unsafe { &*this }.refs.fetch_sub(1, Ordering::AcqRel) - 1
}

pub(crate) fn fake() -> Box<FakeCom> {
    let vt: &'static [usize; 3] = Box::leak(Box::new([0, fake_addref as *const () as usize, fake_release as *const () as usize]));
    Box::new(FakeCom { vt: vt.as_ptr(), refs: AtomicU32::new(1) })
}

pub(crate) unsafe fn vs(o: usize, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

fn obj_with(dev: &FakeCom, d: &[u8; DESC_BYTES]) -> usize {
    let mut p = 0usize;
    assert_eq!(unsafe { create_counter_set(2, dev as *const FakeCom as usize, d.as_ptr(), &mut p) }, 0);
    p
}

#[test]
fn counter_set_object_com_semantics() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dev = fake();
    let mut d = empty_desc();
    put(&mut d, 0, 0, 0);
    put(&mut d, 7, 0, 1);
    let o = obj_with(&dev, &d);
    assert_eq!(dev.refs.load(Ordering::Acquire), 2);
    assert!(is_live(o));
    assert_eq!(counter_set_for(o).unwrap().sample_bytes, 8 + 16);
    unsafe {
        let size: unsafe extern "system" fn(usize) -> u64 = core::mem::transmute(vs(o, 8));
        assert_eq!(size(o), 24);
        let qi: unsafe extern "system" fn(usize, *const Guid, *mut usize) -> i32 = core::mem::transmute(vs(o, 0));
        let mut p = 5usize;
        assert_eq!(qi(o, &IUNKNOWN, &mut p), 0x8000_4002u32 as i32);
        assert_eq!(p, 0);
        assert_eq!(qi(o, &IID_RESOURCE, &mut p), 0);
        assert_eq!(p, o);
        assert_eq!(qi(o, &IID_DEVICE, &mut p), 0);
        assert_eq!(dev.refs.load(Ordering::Acquire), 4);
        let get_dev: unsafe extern "system" fn(usize, *const Guid, *mut usize) -> i32 = core::mem::transmute(vs(o, 7));
        let mut dp = 0usize;
        assert_eq!(get_dev(o, &IUNKNOWN, &mut dp), 5);
        assert_eq!(dp, &*dev as *const FakeCom as usize);
        let set_name: unsafe extern "system" fn(usize, *const u16) -> i32 = core::mem::transmute(vs(o, 6));
        assert_eq!(set_name(o, core::ptr::null()), 0);
        let rel: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(vs(o, 2));
        let devrel: unsafe extern "system" fn(*mut FakeCom) -> u32 = fake_release;
        devrel(&*dev as *const FakeCom as *mut FakeCom);
        assert_eq!(rel(o), 2);
        assert_eq!(rel(o), 1);
        assert_eq!(dev.refs.load(Ordering::Acquire), 2);
        assert_eq!(rel(o), 0);
        assert_eq!(dev.refs.load(Ordering::Acquire), 1);
    }
    assert!(!is_live(o));
    assert!(counter_set_for(o).is_none());
}

#[test]
fn private_data_matches_the_console_store() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dev = fake();
    let iface = fake();
    let o = obj_with(&dev, &empty_desc());
    unsafe {
        let get: unsafe extern "system" fn(usize, *const Guid, *mut u32, *mut u8) -> i32 = core::mem::transmute(vs(o, 3));
        let set: unsafe extern "system" fn(usize, *const Guid, u32, *const u8) -> i32 = core::mem::transmute(vs(o, 4));
        let seti: unsafe extern "system" fn(usize, *const Guid, usize) -> i32 = core::mem::transmute(vs(o, 5));
        let mut n = 77u32;
        assert_eq!(get(o, &G1, &mut n, core::ptr::null_mut()), PRIVATE_DATA_NOT_FOUND);
        assert_eq!(n, 0);
        assert_eq!(set(o, &G1, 0, core::ptr::null()), PRIVATE_DATA_NOT_FOUND);
        assert_eq!(set(o, &G1, 6, b"abcdef".as_ptr()), 0);
        let mut buf = [0u8; 4];
        n = 4;
        assert_eq!(get(o, &G1, &mut n, buf.as_mut_ptr()), 0);
        assert_eq!((&buf, n), (b"abcd", 6));
        n = 0;
        assert_eq!(get(o, &G1, &mut n, buf.as_mut_ptr()), 0);
        assert_eq!(n, 6);
        assert_eq!(set(o, &G1, 2, b"zz".as_ptr()), 0);
        n = 8;
        let mut b8 = [0u8; 8];
        assert_eq!(get(o, &G1, &mut n, b8.as_mut_ptr()), 0);
        assert_eq!((n, &b8[..2]), (2, &b"zz"[..]));
        let ip = &*iface as *const FakeCom as usize;
        assert_eq!(seti(o, &G1, ip), 0);
        assert_eq!(iface.refs.load(Ordering::Acquire), 2);
        n = 4;
        assert_eq!(get(o, &G1, &mut n, b8.as_mut_ptr()), 0);
        assert_eq!(n, 8);
        assert_eq!(iface.refs.load(Ordering::Acquire), 3);
        let mut out = 0usize;
        n = 8;
        assert_eq!(get(o, &G1, &mut n, &mut out as *mut usize as *mut u8), 0);
        assert_eq!(out, ip);
        assert_eq!(seti(o, &G1, 0), 0);
        assert_eq!(iface.refs.load(Ordering::Acquire), 3);
        assert_eq!(seti(o, &G1, 0), PRIVATE_DATA_NOT_FOUND);
        assert_eq!(seti(o, &G1, ip), 0);
        let rel: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(vs(o, 2));
        assert_eq!(rel(o), 0);
        assert_eq!(iface.refs.load(Ordering::Acquire), 3);
    }
}

#[repr(C)]
pub(crate) struct FakeRes {
    vt: *const usize,
    pub(crate) data: Vec<u8>,
    mappable: bool,
    pub(crate) maps: AtomicU32,
}

unsafe extern "system" fn res_map(this: *mut FakeRes, _sub: u32, _r: *const [usize; 2], p: *mut *mut u8) -> i32 {
    let r = unsafe { &mut *this };
    if !r.mappable {
        return 0x8007_0057u32 as i32;
    }
    r.maps.fetch_add(1, Ordering::AcqRel);
    unsafe { *p = r.data.as_mut_ptr() };
    0
}

unsafe extern "system" fn res_unmap(this: *mut FakeRes, _sub: u32, _r: *const [usize; 2]) {
    unsafe { &*this }.maps.fetch_sub(1, Ordering::AcqRel);
}

unsafe extern "system" fn res_desc(this: *mut FakeRes, out: *mut u8) -> *mut u8 {
    let r = unsafe { &*this };
    let mut d = [0u8; 0x38];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[8..16].copy_from_slice(&(r.data.len() as u64).to_le_bytes());
    unsafe { core::ptr::copy_nonoverlapping(d.as_ptr(), out, 0x38) };
    out
}

pub(crate) fn fake_res(len: usize, mappable: bool) -> Box<FakeRes> {
    let mut vt = vec![0usize; 11];
    vt[8] = res_map as *const () as usize;
    vt[9] = res_unmap as *const () as usize;
    vt[10] = res_desc as *const () as usize;
    let vt: &'static [usize] = Box::leak(vt.into_boxed_slice());
    Box::new(FakeRes { vt: vt.as_ptr(), data: (0..len).map(|i| i as u8).collect(), mappable, maps: AtomicU32::new(0) })
}

pub(crate) fn sampler(res: usize, off: u64) -> Option<u64> {
    if res != 0 { Some(0xc10c_0000 + off) } else { None }
}

#[test]
fn resolve_reads_mappable_memory_and_falls_back_to_the_sampler() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dev = fake();
    let mut d = empty_desc();
    put(&mut d, 0, 0, 0);
    put(&mut d, 7, 1, 3);
    let o = obj_with(&dev, &d);
    let resolve: unsafe extern "system" fn(usize, usize, u64, *mut u8) -> i32 = unsafe { core::mem::transmute(vs(o, 9)) };
    let r = fake_res(64, true);
    let mut out = vec![0u8; RESULT_BYTES as usize];
    assert_eq!(unsafe { resolve(o, &*r as *const FakeRes as usize, 8, out.as_mut_ptr()) }, 0);
    assert_eq!(r.maps.load(Ordering::Acquire), 0);
    assert_eq!(&out[8..16], &r.data[8..16]);
    assert!(out[16..].iter().all(|&b| b == 0xff));
    let dr = fake_res(64, false);
    out.fill(0);
    assert_eq!(unsafe { resolve(o, &*dr as *const FakeRes as usize, 0x40, out.as_mut_ptr()) }, 0);
    assert!(out[16..].iter().all(|&b| b == 0xff));
    let set = counter_set_for(o).unwrap();
    let mut arr = [0u8; RESULT_BYTES as usize];
    assert!(!unsafe { resolve_resource(&set, &*dr as *const FakeRes as usize, 0x40, &mut arr, Some(sampler)) });
    assert_eq!(u64::from_le_bytes(arr[8..16].try_into().unwrap()), 0xc10c_0040);
    assert!(arr[16..].iter().all(|&b| b == 0xff));
    assert!(!unsafe { resolve_resource(&set, &*dr as *const FakeRes as usize, 0x40, &mut arr, None) });
    assert!(arr[8..].iter().all(|&b| b == 0xff));
    let short = fake_res(12, true);
    assert_eq!(unsafe { resolve(o, &*short as *const FakeRes as usize, 0, out.as_mut_ptr()) }, 0);
    assert_eq!(&out[8..16], &short.data[0..8]);
    assert!(out[16..].iter().all(|&b| b == 0xff));
    let rel: unsafe extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(vs(o, 2)) };
    assert_eq!(unsafe { rel(o) }, 0);
}

#[test]
fn scarlett_returns_not_implemented_without_the_retail_counter_file() {
    if retail_counters_enabled() {
        return;
    }
    let dev = fake();
    let mut p = 0x55usize;
    let d = empty_desc();
    for t in [0u32, 1, 6, 7, 8] {
        assert_eq!(unsafe { create_counter_set(t, &*dev as *const FakeCom as usize, d.as_ptr(), &mut p) }, 0x8000_4001u32 as i32);
    }
    assert_eq!(p, 0x55);
    assert_eq!(dev.refs.load(Ordering::Acquire), 1);
}

#[test]
fn null_ppv_keeps_the_object_like_the_console() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dev = fake();
    let d = empty_desc();
    assert_eq!(unsafe { create_counter_set(4, &*dev as *const FakeCom as usize, d.as_ptr(), core::ptr::null_mut()) }, 0);
    assert_eq!(dev.refs.load(Ordering::Acquire), 2);
}

#[test]
fn device_slot_82_is_installed() {
    let mut v = vec![0usize; 100];
    install_device(&mut v);
    assert_ne!(v[DEVICE_SLOT], 0);
    assert_eq!(v.iter().filter(|&&x| x != 0).count(), 1);
}

unsafe fn hw_buffer(dev: usize, heap: u32, size: u64) -> usize {
    let h: [u32; 5] = [heap, 0, 0, 1, 1];
    let mut d = [0u8; 56];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    let state: u32 = if heap == 2 { 0xac3 } else { 0 };
    let f: unsafe extern "system" fn(usize, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut usize) -> i32 = unsafe { core::mem::transmute(vs(dev, 27)) };
    let mut r = 0usize;
    let hr = unsafe { f(dev, h.as_ptr(), 0, d.as_ptr(), state, core::ptr::null(), &IID_RESOURCE, &mut r) };
    assert!(hr >= 0 && r != 0, "CreateCommittedResource hr={hr:#010x}");
    r
}

#[test]
#[ignore = "needs a D3D12 device: TOMBOX_EXEC_HW=1 cargo test -p d3d12_xs --lib hw_perfctr -- --ignored"]
fn hw_perfctr_resolve_reads_a_real_upload_buffer() {
    if std::env::var_os("TOMBOX_EXEC_HW").is_none() {
        return;
    }
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut dev: *mut c_void = core::ptr::null_mut();
    let create: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(crate::host_proc(b"D3D12CreateDevice\0")) };
    assert!(create(core::ptr::null_mut(), 0xb000, &IID_DEVICE, &mut dev) >= 0);
    let dev = dev as usize;
    unsafe {
        let up = hw_buffer(dev, 2, 4096);
        let map: unsafe extern "system" fn(usize, u32, *const c_void, *mut *mut u8) -> i32 = core::mem::transmute(vs(up, 8));
        let mut p: *mut u8 = core::ptr::null_mut();
        assert!(map(up, 0, core::ptr::null(), &mut p) >= 0);
        for i in 0..4096 {
            *p.add(i) = (i * 7) as u8;
        }
        let mut d = empty_desc();
        put(&mut d, 2, 0, 0);
        put(&mut d, 12, 1, 4);
        let mut o = 0usize;
        assert_eq!(create_counter_set(2, dev, d.as_ptr(), &mut o), 0);
        let resolve: unsafe extern "system" fn(usize, usize, u64, *mut u8) -> i32 = core::mem::transmute(vs(o, 9));
        let mut out = vec![0u8; RESULT_BYTES as usize];
        assert_eq!(resolve(o, up, 0x100, out.as_mut_ptr()), 0);
        let src = core::slice::from_raw_parts(p.add(0x100), 8 + 96);
        assert_eq!(&out[0x28..0x30], &src[0..8]);
        assert!(out[0x30..].iter().all(|&b| b == 0xff));
        assert_ne!(&src[8..16], &[0xff; 8]);
        let def = hw_buffer(dev, 1, 4096);
        let set = counter_set_for(o).unwrap();
        let mut arr = [0u8; RESULT_BYTES as usize];
        assert!(!resolve_resource(&set, def, 0x10, &mut arr, Some(sampler)));
        assert_eq!(u64::from_le_bytes(arr[0x28..0x30].try_into().unwrap()), 0xc10c_0010);
        assert!(arr[0x30..].iter().all(|&b| b == 0xff));
        for x in [o, def, up, dev] {
            let rel: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(vs(x, 2));
            rel(x);
        }
        assert!(!is_live(o));
    }
}
