use std::sync::atomic::Ordering;

use crate::device_wrap::Guid;
use crate::perfctr::{PcSource, PRIVATE_DATA_NOT_FOUND};
use crate::perfctr_tests::{fake, fake_res, sampler, vs, FakeCom, FakeRes, LOCK};
use crate::perfctr_xs::*;

const ANACONDA_DT: u32 = 7;
const LOCKHART_DT: u32 = 6;
const ONE_ONLY_IID: Guid = Guid { d1: 0x9d06dffa, d2: 0xd1e5, d3: 0x4d07, d4: [0x83, 0xa8, 0x1b, 0xb1, 0x23, 0xf2, 0xf8, 0x41] };
const XS_ONLY_IID: Guid = Guid { d1: 0x8754318e, d2: 0xd3a9, d3: 0x4541, d4: [0x98, 0xcf, 0x64, 0x5b, 0x50, 0xdc, 0x48, 0x74] };
const IUNKNOWN: Guid = Guid { d1: 0, d2: 0, d3: 0, d4: [0xc0, 0, 0, 0, 0, 0, 0, 0x46] };
const G1: Guid = Guid { d1: 0x1111_2222, d2: 3, d3: 4, d4: [5, 6, 7, 8, 9, 10, 11, 12] };

fn empty() -> [u8; XS_DESC_BYTES] {
    let mut d = [0u8; XS_DESC_BYTES];
    d[0x10..].fill(0xff);
    d
}

fn put(d: &mut [u8; XS_DESC_BYTES], block: usize, slot: usize, sel: u32) {
    let o = XS_DESC_OFFSET[block] as usize + 4 * slot;
    d[o..o + 4].copy_from_slice(&sel.to_le_bytes());
}

fn entry_bytes(t: &XsBlock, block: u8, lockhart: bool) -> u32 {
    8 * (t.instances as u32 * t.engines as u32 - xs_harvested(block, lockhart) as u32) * t.arrays as u32
}

#[test]
fn xs_desc_and_result_layout_follow_ctor_0x5ea70() {
    for b in 0..XS_BLOCKS {
        assert_eq!(ANACONDA[b].slots, LOCKHART[b].slots, "block {b}");
        let end = if b + 1 < XS_BLOCKS { XS_DESC_OFFSET[b + 1] } else { XS_DESC_BYTES as u32 };
        assert_eq!(XS_DESC_OFFSET[b] + 4 * ANACONDA[b].slots as u32, end, "block {b}");
        let next = if b + 1 < XS_BLOCKS { XS_RESULT_BASE[b + 1] } else { XS_RESULT_BYTES };
        assert_eq!(XS_RESULT_BASE[b] + ANACONDA[b].slots as u32 * XS_RESULT_STRIDE[b], next, "block {b}");
        let a = entry_bytes(&ANACONDA[b], b as u8, false);
        let l = entry_bytes(&LOCKHART[b], b as u8, true);
        if b == 0x16 {
            assert_eq!((XS_RESULT_STRIDE[b], a.max(l)), (0x20, 0x10));
        } else {
            assert_eq!(XS_RESULT_STRIDE[b], a.max(l), "block {b}");
        }
    }
}

#[test]
fn xs_ctor_defaults_and_console_table_choice() {
    let s = xs_build(&empty(), ANACONDA_DT);
    assert!(s.entries.is_empty());
    assert_eq!((s.sample_bytes, s.sq_ctrl, s.lockhart), (0, 0xffff, false));
    let mut d = empty();
    d[0xc..0x10].copy_from_slice(&0x0003_0041u32.to_le_bytes());
    let s = xs_build(&d, LOCKHART_DT);
    assert_eq!((s.sq_ctrl, s.lockhart), (0x41, true));
    assert!(lockhart_for(6));
    assert!(!lockhart_for(7) && !lockhart_for(8));
}

#[test]
fn xs_entry_sizes_follow_the_sku_table_and_harvest_rule() {
    let mut d = empty();
    put(&mut d, 0, 1, 3);
    put(&mut d, 0xd, 0, 1);
    put(&mut d, 0x1a, 4, 2);
    put(&mut d, 0x1c, 0, 0);
    let a = xs_build(&d, ANACONDA_DT);
    let sizes: Vec<_> = a.entries.iter().map(|e| (e.block, e.bytes(), e.sample_offset, e.result_offset)).collect();
    assert_eq!(sizes, vec![(0, 64, 0, 0x8 + 0x40), (0xd, 160, 64, 0x668), (0x1a, 416, 224, 0xe58 + 4 * 0x1a0), (0x1c, 416, 640, 0x2378)]);
    assert_eq!(a.sample_bytes, 1056);
    let l = xs_build(&d, LOCKHART_DT);
    let sizes: Vec<_> = l.entries.iter().map(|e| (e.block, e.bytes(), e.sample_offset, e.result_offset)).collect();
    assert_eq!(sizes, vec![(0, 32, 0, 0x8 + 0x40), (0xd, 64, 32, 0x668), (0x1a, 160, 96, 0xe58 + 4 * 0x1a0), (0x1c, 160, 256, 0x2378)]);
    assert_eq!(l.sample_bytes, 416);
}

#[test]
fn xs_select_fields_and_sq_gap_rule() {
    assert_eq!(xs_select(0xe, 0x1c5), 0x05);
    assert_eq!(xs_select(0xf, 0x40), 0);
    assert_eq!(xs_select(XS_SQ_BLOCK, 0x1234), 0xf034);
    assert_eq!(xs_select(0, 0xffff), 0x3ff);
    let mut d = empty();
    put(&mut d, 0x17, 0, 4);
    put(&mut d, 0x17, 2, 5);
    put(&mut d, 0x17, 3, 6);
    let s = xs_build(&d, ANACONDA_DT);
    let sq: Vec<_> = s.entries.iter().map(|e| (e.select, e.slot, e.result_offset)).collect();
    assert_eq!(sq, vec![(0xf004, 0, 0xc58), (0xf006, 1, 0xc58 + 3 * 0x10)]);
    let mut d = empty();
    put(&mut d, 0x17, 1, 7);
    assert_eq!(xs_build(&d, ANACONDA_DT).entries[0].result_offset, 0xc58 + 0x10);
    let mut d = empty();
    put(&mut d, 0x16, 0, 1);
    put(&mut d, 0x16, 2, 2);
    assert_eq!(xs_build(&d, ANACONDA_DT).entries.len(), 2);
}

#[test]
fn xs_only_clock_counting_selects_have_a_pc_source() {
    let x = PcSource::ConsoleClock { hz: 1_825_000_000 };
    let s = PcSource::ConsoleClock { hz: xgs_kmd_emu::gpu_frequency_hz(LOCKHART_DT) };
    assert_eq!(xs_pc_source(ANACONDA_DT, 5, 0), x);
    assert_eq!(xs_pc_source(LOCKHART_DT, 3, 0x400), s);
    assert_eq!(xs_pc_source(ANACONDA_DT, 4, 0), x);
    assert_eq!(xs_pc_source(ANACONDA_DT, 0xe, 0), x);
    assert_eq!(xs_pc_source(ANACONDA_DT, 0x10, 0x40), x);
    assert_eq!(xs_pc_source(ANACONDA_DT, 0xe, 2), PcSource::None);
    assert_eq!(xs_pc_source(ANACONDA_DT, 5, 1), PcSource::None);
    for b in [0u8, 1, 2, 6, 7, 8, 9, 0xa, 0xb, 0xc, 0xd, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c] {
        assert_eq!(xs_pc_source(ANACONDA_DT, b, 0), PcSource::None, "block {b:#x}");
    }
}

#[test]
fn xs_resolve_writes_header_ff_fill_and_entry_values() {
    let mut d = empty();
    put(&mut d, 5, 1, 0);
    put(&mut d, 0, 0, 3);
    let s = xs_build(&d, ANACONDA_DT);
    let mut out = [0u8; XS_RESULT_BYTES as usize];
    xs_resolve_into(&s, |e, i| if e.block == 5 { Some(0x1122_3344_5566_7788) } else if i == 7 { Some(9) } else { None }, &mut out);
    assert_eq!(&out[0..8], &[0x18, 0x25, 0, 0, 1, 0, 0, 0]);
    let cpf = 0x1c8 + 8;
    assert_eq!(u64::from_le_bytes(out[cpf..cpf + 8].try_into().unwrap()), 0x1122_3344_5566_7788);
    assert_eq!(u64::from_le_bytes(out[0x8 + 56..0x8 + 64].try_into().unwrap()), 9);
    assert!(out[8..0x8 + 56].iter().all(|&b| b == 0xff));
    assert!(out[0x8 + 64..cpf].iter().all(|&b| b == 0xff));
    assert!(out[cpf + 8..].iter().all(|&b| b == 0xff));
    let cb = s.entries.iter().find(|e| e.block == 0).unwrap();
    let cp = s.entries.iter().find(|e| e.block == 5).unwrap();
    assert_eq!(xs_raw_value(Some(sampler), 1, 0, cb, 0), None);
    assert_eq!(xs_raw_value(None, 1, 0, cp, 0), None);
    assert_eq!(xs_raw_value(Some(sampler), 1, 0x10, cp, 0), Some(0xc10c_0000 + 0x10 + 64));
    let mut arr = [0u8; XS_RESULT_BYTES as usize];
    assert_eq!(xs_resolve_resource(&s, 1, 0x10, &mut arr, Some(sampler)), 1);
    assert_eq!(u64::from_le_bytes(arr[cpf..cpf + 8].try_into().unwrap()), 0xc10c_0000 + 0x10 + 64);
    assert!(arr[8..cpf].iter().all(|&b| b == 0xff) && arr[cpf + 8..].iter().all(|&b| b == 0xff));
    assert_eq!(xs_resolve_resource(&s, 1, 0x10, &mut arr, None), 0);
    assert!(arr[8..].iter().all(|&b| b == 0xff));
}

fn devp(d: &FakeCom) -> usize {
    d as *const FakeCom as usize
}

#[test]
fn xs_default_is_retail_not_implemented() {
    let dev = fake();
    let mut p = 0x55usize;
    let d = empty();
    assert_eq!(unsafe { create_counter_set_xs(false, ANACONDA_DT, devp(&dev), d.as_ptr(), &mut p) }, 0x8000_4001u32 as i32);
    assert_eq!(unsafe { create_counter_set_xs(false, LOCKHART_DT, devp(&dev), core::ptr::null(), &mut p) }, 0x8000_4001u32 as i32);
    assert_eq!(p, 0x55);
    assert_eq!(dev.refs.load(Ordering::Acquire), 1);
    assert_eq!(unsafe { create_counter_set_xs(true, ANACONDA_DT, devp(&dev), core::ptr::null(), &mut p) }, 0x8007_0057u32 as i32);
    assert_eq!(p, 0x55);
}

#[test]
fn xs_counter_set_object_matches_the_scarlett_vtable() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dev = fake();
    let mut d = empty();
    put(&mut d, 3, 0, 0);
    put(&mut d, 0, 0, 1);
    let mut o = 0usize;
    assert_eq!(unsafe { create_counter_set_xs(true, ANACONDA_DT, devp(&dev), d.as_ptr(), &mut o) }, 0);
    assert_ne!(o, 0);
    assert_eq!(dev.refs.load(Ordering::Acquire), 2);
    assert_eq!(xs_counter_set_for(o).unwrap().sample_bytes, 64 + 8);
    assert!(crate::perfctr::counter_set_for(o).is_none());
    unsafe {
        let size: unsafe extern "system" fn(usize) -> u64 = core::mem::transmute(vs(o, 8));
        assert_eq!(size(o), 72);
        let qi: unsafe extern "system" fn(usize, *const Guid, *mut usize) -> i32 = core::mem::transmute(vs(o, 0));
        let mut p = 5usize;
        assert_eq!(qi(o, &ONE_ONLY_IID, &mut p), 0x8000_4002u32 as i32);
        assert_eq!(p, 0);
        assert_eq!(qi(o, &IUNKNOWN, &mut p), 0x8000_4002u32 as i32);
        assert_eq!(qi(o, &XS_ONLY_IID, &mut p), 0);
        assert_eq!(p, o);
        assert_eq!(dev.refs.load(Ordering::Acquire), 3);
        let get: unsafe extern "system" fn(usize, *const Guid, *mut u32, *mut u8) -> i32 = core::mem::transmute(vs(o, 3));
        let set: unsafe extern "system" fn(usize, *const Guid, u32, *const u8) -> i32 = core::mem::transmute(vs(o, 4));
        let mut n = 9u32;
        assert_eq!(get(o, &WKPDID_D3D_DEBUG_OBJECT_NAME_W, &mut n, core::ptr::null_mut()), E_FAIL);
        assert_eq!(n, 0);
        assert_eq!(get(o, &WKPDID_D3D_DEBUG_OBJECT_NAME_W, core::ptr::null_mut(), core::ptr::null_mut()), E_FAIL);
        assert_eq!(set(o, &WKPDID_D3D_DEBUG_OBJECT_NAME_W, 4, b"n\0a\0".as_ptr()), 0);
        n = 9;
        assert_eq!(get(o, &WKPDID_D3D_DEBUG_OBJECT_NAME_W, &mut n, core::ptr::null_mut()), E_FAIL);
        assert_eq!(get(o, &G1, &mut n, core::ptr::null_mut()), PRIVATE_DATA_NOT_FOUND);
        assert_eq!(set(o, &G1, 3, b"abc".as_ptr()), 0);
        let mut buf = [0u8; 3];
        n = 3;
        assert_eq!(get(o, &G1, &mut n, buf.as_mut_ptr()), 0);
        assert_eq!((&buf, n), (b"abc", 3));
        let resolve: unsafe extern "system" fn(usize, usize, u64, *mut u8) -> i32 = core::mem::transmute(vs(o, 9));
        let r: Box<FakeRes> = fake_res(128, true);
        let mut out = vec![0u8; XS_RESULT_BYTES as usize];
        assert_eq!(resolve(o, &*r as *const FakeRes as usize, 8, out.as_mut_ptr()), 0);
        assert_eq!(&out[0..8], &[0x18, 0x25, 0, 0, 1, 0, 0, 0]);
        assert!(out[8..].iter().all(|&b| b == 0xff), "bytes no sampler wrote must not be resolved");
        assert_eq!(r.maps.load(Ordering::Acquire), 0);
        let rel: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(vs(o, 2));
        assert_eq!(rel(o), 1);
        assert_eq!(rel(o), 0);
    }
    assert!(xs_counter_set_for(o).is_none());
    assert_eq!(dev.refs.load(Ordering::Acquire), 1);
}
