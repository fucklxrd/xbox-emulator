use super::*;
use crate::gpu_va::Entry;
use std::cell::RefCell;

thread_local! {
    static CALLS: RefCell<Vec<(usize, usize, Vec<u8>, usize, usize)>> = const { RefCell::new(Vec::new()) };
}

fn take_calls() -> Vec<(usize, usize, Vec<u8>, usize, usize)> {
    CALLS.with(|c| core::mem::take(&mut *c.borrow_mut()))
}

#[repr(C)]
struct FakeObj {
    vt: *const usize,
    fmt: u32,
}

unsafe extern "system" fn fake_get_desc(this: *mut c_void, out: *mut u8) -> *mut u8 {
    let mut d = [0u8; 0x38];
    wr32(&mut d, 0x20, unsafe { (*(this as *const FakeObj)).fmt });
    unsafe { core::ptr::copy_nonoverlapping(d.as_ptr(), out, 0x38) };
    out
}

unsafe extern "system" fn fake_srv(_real: *mut c_void, res: *mut c_void, desc: *const u8, dest: usize) {
    let d = if desc.is_null() { Vec::new() } else { unsafe { core::slice::from_raw_parts(desc, 0x28) }.to_vec() };
    CALLS.with(|c| c.borrow_mut().push((18, res as usize, d, 0, dest)));
}

unsafe extern "system" fn fake_uav(_real: *mut c_void, res: *mut c_void, counter: *mut c_void, desc: *const u8, dest: usize) {
    let d = if desc.is_null() { Vec::new() } else { unsafe { core::slice::from_raw_parts(desc, 0x28) }.to_vec() };
    CALLS.with(|c| c.borrow_mut().push((19, res as usize, d, counter as usize, dest)));
}

unsafe extern "system" fn fake_component(this: *mut c_void, loc: *const c_void, desc: *const u8, state: u32, _clear: *const c_void, _riid: *const Guid, ppv: *mut *mut c_void) -> i32 {
    let d = unsafe { core::slice::from_raw_parts(desc, 0x40) }.to_vec();
    CALLS.with(|c| c.borrow_mut().push((77, loc as usize, d, state as usize, this as usize)));
    unsafe { *ppv = 0x5151 as *mut c_void };
    0x1234
}

unsafe extern "system" fn fake_create_compute(_real: *mut c_void, desc: *const u8, _riid: *const Guid, ppv: *mut *mut c_void) -> i32 {
    let d = unsafe { core::slice::from_raw_parts(desc, 0x38) }.to_vec();
    CALLS.with(|c| c.borrow_mut().push((11, 0, d, 0, 0)));
    unsafe { *ppv = Box::into_raw(Box::new(0u64)) as *mut c_void };
    S_OK
}

fn fake_vtable() -> &'static [usize; 96] {
    let mut v = [0usize; 96];
    v[RES_GET_DESC] = fake_get_desc as usize;
    v[PC_CREATE_COMPUTE_PSO] = fake_create_compute as usize;
    v[PC_CREATE_SRV] = fake_srv as usize;
    v[PC_CREATE_UAV] = fake_uav as usize;
    v[SLOT_COMPONENT_PLACED] = fake_component as usize;
    Box::leak(Box::new(v))
}

fn fake(fmt: u32) -> FakeObj {
    FakeObj { vt: fake_vtable().as_ptr(), fmt }
}

#[test]
fn slot45_builds_the_component_desc_and_forwards_to_slot77() {
    let mut d = [0u8; 0x38];
    wr32(&mut d, 0, 3);
    wr64(&mut d, 0x10, 256);
    wr32(&mut d, 0x30, 1 << 22);
    wr32(&mut d, 0x34, 0x77);
    let c = component_desc(&d);
    assert_eq!(rd32(&c, 0x34), 2);
    assert_eq!(rd32(&c, 0x38), 0);
    assert_eq!(&c[..0x30], &d[..0x30]);
    wr32(&mut d, 0x30, (1 << 22) | (1 << 23));
    assert_eq!(rd32(&component_desc(&d), 0x34), 0);
    wr32(&mut d, 0x30, 0x4);
    let c = component_desc(&d);
    assert_eq!(rd32(&c, 0x34), 0x77);
    assert_eq!(rd64(&c, 0x38), 0);
    let mut obj = fake(0);
    let this = &mut obj as *mut FakeObj as *mut c_void;
    let mut out: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { create_placed_resource_x2(this, 0xabc as *const c_void, d.as_ptr(), 0x40, core::ptr::null(), core::ptr::null(), &mut out) };
    assert_eq!(hr, 0x1234);
    assert_eq!(out as usize, 0x5151);
    let calls = take_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].0, calls[0].1, calls[0].3, calls[0].4), (77, 0xabc, 0x40, this as usize));
    assert_eq!(calls[0].2, component_desc(&d).to_vec());
}

#[test]
fn format_sizes_and_depth_promotion() {
    assert_eq!(dxgi_format_bytes(2), Some(16));
    assert_eq!(dxgi_format_bytes(6), Some(12));
    assert_eq!(dxgi_format_bytes(10), Some(8));
    assert_eq!(dxgi_format_bytes(41), Some(4));
    assert_eq!(dxgi_format_bytes(87), Some(4));
    assert_eq!(dxgi_format_bytes(57), Some(2));
    assert_eq!(dxgi_format_bytes(61), Some(1));
    assert_eq!(dxgi_format_bytes(71), None);
    assert_eq!(promote_depth_view_format(46, 20, false), 21);
    assert_eq!(promote_depth_view_format(44, 19, false), 19);
    assert_eq!(promote_depth_view_format(47, 22, false), 22);
    assert_eq!(promote_depth_view_format(46, 45, false), 46);
    assert_eq!(promote_depth_view_format(46, 20, true), 46);
    assert_eq!(promote_depth_view_format(28, 20, false), 28);
}

#[test]
fn placement_rules() {
    let buf = Entry { start: 0x1000, len: 0x1000, res: 7, buffer: true };
    let tex = Entry { start: 0x8000, len: 0x1000, res: 9, buffer: false };
    assert_eq!(placement_for(true, Some(16), Some((buf, 0x40))), Ok(Placement { res: 7, first_element_delta: 4 }));
    assert!(placement_for(true, Some(16), Some((buf, 0x44))).is_err());
    assert!(placement_for(true, None, Some((buf, 0))).is_err());
    assert!(placement_for(true, Some(4), Some((tex, 0))).is_err());
    assert_eq!(placement_for(false, None, Some((tex, 0))), Ok(Placement { res: 9, first_element_delta: 0 }));
    assert!(placement_for(false, None, Some((tex, 0x100))).is_err());
    assert!(placement_for(false, None, Some((buf, 0))).is_err());
    assert!(placement_for(false, None, None).is_err());
}

fn srv_desc(fmt: u32, dim: u32, first: u64, stride: u32, flags: u32, va: u64) -> [u8; 0x48] {
    let mut d = [0u8; 0x48];
    wr32(&mut d, 0, fmt);
    wr32(&mut d, 4, dim);
    wr32(&mut d, 8, 0x1688);
    wr64(&mut d, 0x10, first);
    wr32(&mut d, 0x18, 8);
    wr32(&mut d, 0x1c, stride);
    wr32(&mut d, 0x20, flags);
    wr64(&mut d, 0x28, va);
    d
}

#[test]
fn placed_srv_targets_the_resource_at_the_address() {
    let mut r = fake(2);
    let res = &mut r as *mut FakeObj as *mut c_void;
    let other = Entry { start: 0x10_0000, len: 0x1000, res: 0x4242, buffer: true };
    let look = move |va: u64| (va >= other.start && va < other.start + other.len).then(|| (other, va - other.start));
    let d = srv_desc(2, 1, 1, 0, 0, NO_ADDRESS);
    assert_eq!(unsafe { plan_placed_view(res, d.as_ptr(), false, &look) }, ViewPlan::Desc(res as usize, d[..0x28].to_vec(), 0, NO_ADDRESS));
    let d = srv_desc(2, 1, 1, 0, 0, 0x10_0020);
    let ViewPlan::Desc(t, pc, _, _) = (unsafe { plan_placed_view(res, d.as_ptr(), false, &look) }) else { panic!() };
    assert_eq!(t, 0x4242);
    assert_eq!(rd64(&pc, 0x10), 3);
    let d = srv_desc(0, 1, 0, 0, 1, 0x10_0008);
    let ViewPlan::Desc(_, pc, _, _) = (unsafe { plan_placed_view(res, d.as_ptr(), false, &look) }) else { panic!() };
    assert_eq!(rd64(&pc, 0x10), 2);
    let d = srv_desc(2, 1, 0, 0, 0, 0x10_0004);
    assert_eq!(unsafe { plan_placed_view(res, d.as_ptr(), false, &look) }, ViewPlan::Null);
    let d = srv_desc(2, 1, 0, 0, 0, 0x99_0000);
    assert_eq!(unsafe { plan_placed_view(res, d.as_ptr(), false, &look) }, ViewPlan::Null);
    assert_eq!(unsafe { plan_placed_view(core::ptr::null_mut(), d.as_ptr(), false, &look) }, ViewPlan::Null);
    assert_eq!(unsafe { plan_placed_view(res, core::ptr::null(), false, &look) }, ViewPlan::Default(res as usize));
}

#[test]
fn placed_srv_promotes_depth_view_formats_like_the_console() {
    let mut r = fake(20);
    let res = &mut r as *mut FakeObj as *mut c_void;
    let d = srv_desc(46, 4, 0, 0, 0, NO_ADDRESS);
    let ViewPlan::Desc(_, pc, _, _) = (unsafe { plan_placed_view(res, d.as_ptr(), false, &|_| None) }) else { panic!() };
    assert_eq!(rd32(&pc, 0), 21);
}

#[test]
fn placed_uav_resolves_the_counter_address() {
    let mut r = fake(42);
    let res = &mut r as *mut FakeObj as *mut c_void;
    let buf = Entry { start: 0x20_0000, len: 0x2000, res: 0x77, buffer: true };
    let look = move |va: u64| (va >= buf.start && va < buf.start + buf.len).then(|| (buf, va - buf.start));
    let mut d = [0u8; 0x40];
    wr32(&mut d, 0, 42);
    wr32(&mut d, 4, 1);
    wr64(&mut d, 8, 0);
    wr32(&mut d, 0x10, 16);
    wr64(&mut d, 0x28, 0x20_0040);
    wr64(&mut d, 0x30, 0x20_1000);
    let ViewPlan::Desc(t, pc, c, _) = (unsafe { plan_placed_view(res, d.as_ptr(), true, &look) }) else { panic!() };
    assert_eq!((t, c), (0x77, 0x77));
    assert_eq!(rd64(&pc, 8), 0x10);
    assert_eq!(rd64(&pc, 0x18), 0x1000);
    wr64(&mut d, 0x30, 0x55_0000);
    assert_eq!(unsafe { plan_placed_view(res, d.as_ptr(), true, &look) }, ViewPlan::Null);
    wr64(&mut d, 0x30, NO_ADDRESS);
    wr64(&mut d, 0x28, NO_ADDRESS);
    let ViewPlan::Desc(t, _, c, _) = (unsafe { plan_placed_view(res, d.as_ptr(), true, &look) }) else { panic!() };
    assert_eq!((t, c), (res as usize, 0));
}

#[test]
fn view_emission_calls_the_pc_view_creators() {
    let mut dev = fake(0);
    let real = &mut dev as *mut FakeObj as *mut c_void;
    take_calls();
    unsafe {
        emit_srv(real, ViewPlan::Null, 0x100);
        emit_srv(real, ViewPlan::Default(0x9), 0x200);
        emit_srv(real, ViewPlan::Desc(0xa, vec![1u8; 0x28], 0, 0), 0x300);
        emit_uav(real, ViewPlan::Null, 0x400);
        emit_uav(real, ViewPlan::Desc(0xb, vec![2u8; 0x28], 0xc, 0), 0x500);
    }
    let c = take_calls();
    assert_eq!((c[0].0, c[0].1, c[0].2.clone(), c[0].4), (18, 0, NULL_SRV.to_vec(), 0x100));
    assert_eq!((c[1].1, c[1].2.len(), c[1].4), (9, 0, 0x200));
    assert_eq!((c[2].1, c[2].2.clone()), (0xa, vec![1u8; 0x28]));
    assert_eq!((c[3].0, c[3].1, c[3].2.clone()), (19, 0, NULL_UAV.to_vec()));
    assert_eq!((c[4].1, c[4].3, c[4].4), (0xb, 0xc, 0x500));
}

#[derive(Default)]
struct Host {
    file: bool,
    cache: Option<Result<usize, i32>>,
    destroyed: Vec<usize>,
    pokes: Vec<u64>,
    hangs: Vec<u32>,
}

impl DriverHost for Host {
    fn file_exists(&mut self, path: &str) -> bool {
        assert_eq!(path, "g:\\D3D_DisableShaderCache");
        self.file
    }
    fn shader_cache_create(&mut self) -> Result<usize, i32> {
        self.cache.unwrap_or(Err(E_FAIL))
    }
    fn shader_cache_destroy(&mut self, cache: usize) {
        self.destroyed.push(cache);
    }
    fn poke(&mut self, addr: u64) -> bool {
        self.pokes.push(addr);
        true
    }
    fn hang_analysis(&mut self, mode: u32) {
        self.hangs.push(mode);
    }
}

#[test]
fn slot49_driver_options_match_the_release_switch() {
    let mut s = DriverState::default();
    let mut h = Host::default();
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 3, 1), S_OK);
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 9, 1), S_OK);
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 10, 1), S_OK);
    assert!(s.kernel.is_empty());
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 4, 1), E_NOTIMPL);
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 15, 7), S_OK);
    assert_eq!(s.globals.get(&15), Some(&1));
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 18, 0), S_OK);
    assert_eq!(s.globals.get(&18), Some(&0));
    assert!(s.kernel.is_empty());
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 17, 5), S_OK);
    assert_eq!((s.globals.get(&17), s.kernel.get(&17)), (Some(&1), Some(&5)));
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 0x16, 0), S_OK);
    assert_eq!(s.globals.get(&0x16), Some(&1));
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 0x102, 0x33), S_OK);
    assert_eq!((s.globals.get(&0x102), s.kernel.get(&0x102)), (Some(&0x33), Some(&0x33)));
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 2, 9), S_OK);
    assert_eq!(s.kernel.get(&2), Some(&9));
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 0x104, 3), S_OK);
    assert_eq!(h.pokes, vec![0x3000]);
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 0x104, 0), S_OK);
    assert_eq!(h.pokes.len(), 1);
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 0xbe7a, 0), S_OK);
    assert_eq!(s.dev_flags.get(&1), Some(&DEV_FLAG_BE7A));
    assert!(!s.kernel.contains_key(&0xbe7a));
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 0xdeaddead, 0), S_OK);
    assert_eq!(s.dev_flags.get(&1), Some(&(DEV_FLAG_BE7A | DEV_FLAG_DEADDEAD)));
    assert!(s.kernel.contains_key(&0xdeaddead));
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 0xbaaad411, 0), S_OK);
    assert_eq!(h.hangs, vec![2]);
}

#[test]
fn slot49_shader_cache_toggle() {
    let mut s = DriverState::default();
    let mut h = Host { file: true, ..Default::default() };
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 16, 1), E_INVALIDARG);
    h.file = false;
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 16, 1), E_FAIL);
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 16, 0), S_OK);
    assert!(h.destroyed.is_empty());
    h.cache = Some(Ok(0x99));
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 16, 1), S_OK);
    h.file = true;
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 16, 1), S_OK);
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 16, 0), S_OK);
    assert_eq!(h.destroyed, vec![0x99]);
    h.file = false;
    h.cache = Some(Err(E_OUTOFMEMORY));
    assert_eq!(set_driver_option(&mut s, &mut h, 1, 16, 1), E_OUTOFMEMORY);
    assert!(s.kernel.is_empty());
}

#[test]
fn slot58_sends_kernel_option_10001() {
    assert_eq!(unsafe { kernel_option_10001_x(core::ptr::null_mut(), 0x5a) }, S_OK);
    assert_eq!(kernel_option(KERNEL_OPTION_FIXED_ID), Some(0x5a));
    assert_eq!(unsafe { set_driver_option_x(core::ptr::null_mut(), 0x101, 0x11) }, S_OK);
    assert_eq!((driver_option(0x101), kernel_option(0x101)), (Some(0x11), Some(0x11)));
}

#[test]
fn slot51_gpu_crash_reports_once_per_device() {
    let mut s = DriverState::default();
    let mut h = Host::default();
    assert_eq!(gpu_crash(&mut s, &mut h, 5, 1), CrashAction::HangAnalysisOnly);
    assert_eq!(h.hangs, vec![2]);
    assert!(s.kernel.is_empty());
    assert_eq!(gpu_crash(&mut s, &mut h, 5, 0), CrashAction::Fatal);
    assert_eq!(s.kernel.get(&0xbaaad911), Some(&2));
    assert_eq!(h.hangs, vec![2, 2]);
    assert_eq!(gpu_crash(&mut s, &mut h, 5, 0), CrashAction::Ignored);
    assert_eq!(gpu_crash(&mut s, &mut h, 6, 2), CrashAction::Fatal);
    unsafe { gpu_crash_x(0x77 as *mut c_void, 1) };
}

fn rep<const N: usize>(p: [i8; N]) -> [i8; 32] {
    core::array::from_fn(|i| p[i % N])
}

#[test]
fn slot59_sample_patterns_match_the_console_driver() {
    let cases: [(u32, u32, [i8; 32], [u8; 16], [u32; 2]); 6] = [
        (1, 1, rep([0, 0, -8, -8]), [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1], [0x21, 0x15240]),
        (2, 0xffff_fffe, [0; 32], [0; 16], [0x81, 0x15249]),
        (3, 2, rep([-6, -6, 6, 6, -2, 2, 2, -2]), [0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3], [0xb2, 0x15489]),
        (4, 0xffff_ffff, rep([-2, -6, 6, -2, -6, 2, 2, 6]), [0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3], [0x132, 0x15492]),
        (8, 0xffff_ffff, rep([1, -3, -1, 3, 5, 1, -3, -5, -5, 5, -7, -1, 3, 7, 7, -7]), [0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 6, 7], [0x1c3, 0x156db]),
        (8, 4, [7, 6, -7, -8, -5, 5, 1, -5, 3, 7, -3, -7, -1, 1, 5, -1, 4, 2, -8, -6, -2, 3, 2, -3, 0, 4, -4, -2, -6, 0, 6, -4], [7, 13, 12, 6, 2, 15, 9, 11, 0, 4, 10, 14, 3, 1, 5, 8], [0x1c4, 0x1591b]),
    ];
    for (count, q, pos, cen, regs) in cases {
        let p = sample_pattern(count, q).unwrap();
        assert_eq!(p.positions, [pos; 4], "count {count} q {q:#x}");
        assert_eq!(p.centroid, cen);
        assert_eq!(p.regs, regs);
    }
    for (c, q) in [(16, 0), (16, 0xffff_ffff), (8, 5), (1, 0xffff_fffd), (0, 5)] {
        assert_eq!(sample_pattern(c, q), Err(E_INVALIDARG));
    }
    let mut pos = [[0i8; 32]; 4];
    let mut cen = [0u8; 16];
    let mut regs = [0u32; 2];
    let desc = [4u32, 0xffff_ffff];
    assert_eq!(unsafe { get_sample_pattern_x(core::ptr::null_mut(), &desc, &mut pos, &mut cen, &mut regs) }, S_OK);
    assert_eq!(regs, [0x132, 0x15492]);
    assert_eq!(pos[3][..8], [-2, -6, 6, -2, -6, 2, 2, 6]);
    assert_eq!(unsafe { get_sample_pattern_x(core::ptr::null_mut(), &desc, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()) }, S_OK);
    assert_eq!(unsafe { get_sample_pattern_x(core::ptr::null_mut(), core::ptr::null(), &mut pos, &mut cen, &mut regs) }, E_INVALIDARG);
    let bad = [16u32, 0];
    regs = [7, 7];
    assert_eq!(unsafe { get_sample_pattern_x(core::ptr::null_mut(), &bad, &mut pos, &mut cen, &mut regs) }, E_INVALIDARG);
    assert_eq!(regs, [7, 7]);
}

#[test]
fn slot63_resource_limits_match_the_console_encoder() {
    let cases: [(bool, [u32; 6], [u32; 5]); 14] = [
        (false, [0x0, 0x0, 0x0, 0x0, 0x0, 0x0], [0x1000000, 0xffff, 0xffff, 0x0, 0x0]),
        (false, [0xffff, 0x0, 0x0, 0x0, 0x0, 0x0], [0x1000000, 0x1, 0x1, 0x0, 0x0]),
        (false, [0xff, 0x40, 0x3, 0x10, 0x2, 0x1], [0x1023020, 0xff00, 0xff00, 0x0, 0x0]),
        (false, [0x1234, 0x10000, 0x63, 0x10000, 0x63, 0x4], [0x7fcf0f0, 0xedcb, 0xedcb, 0x0, 0x0]),
        (false, [0x0, 0x1f, 0xf, 0x7, 0x8, 0x2], [0x741f010, 0xffff, 0xffff, 0x0, 0x0]),
        (false, [0xfffe, 0x1, 0x1, 0x1, 0x1, 0x3], [0x811010, 0x1, 0x1, 0x0, 0x0]),
        (false, [0x5555, 0x640, 0x0, 0x1e0, 0x0, 0x0], [0x13c00f0, 0xaaaa, 0xaaaa, 0x0, 0x0]),
        (true, [0x0, 0x0, 0x0, 0x0, 0x0, 0x0], [0x1000000, 0xffff, 0xffff, 0xffff, 0xffff]),
        (true, [0xffff, 0x0, 0x0, 0x0, 0x0, 0x0], [0x1000000, 0x1, 0x1, 0x1, 0x1]),
        (true, [0xff, 0x40, 0x3, 0x10, 0x2, 0x1], [0x1013010, 0xff00, 0xff00, 0xff00, 0xff00]),
        (true, [0x1234, 0x10000, 0x63, 0x10000, 0x63, 0x4], [0x7e4f190, 0xedcb, 0xedcb, 0xedcb, 0xedcb]),
        (true, [0x0, 0x1f, 0xf, 0x7, 0x8, 0x2], [0x741f010, 0xffff, 0xffff, 0xffff, 0xffff]),
        (true, [0xfffe, 0x1, 0x1, 0x1, 0x1, 0x3], [0x811010, 0x1, 0x1, 0x1, 0x1]),
        (true, [0x5555, 0x640, 0x0, 0x1e0, 0x0, 0x0], [0x11e0190, 0xaaaa, 0xaaaa, 0xaaaa, 0xaaaa]),
    ];
    for (sc, p, want) in cases {
        assert_eq!(compute_resource_limits(&p, sc), want, "scarlett={sc} {p:x?}");
    }
}

#[test]
fn slot63_derives_a_new_compute_pipeline_from_the_base() {
    let mut patches = vec![0u8; 2 * 0x1c];
    for (i, v) in [0xffu32, 0x40, 3, 0x10, 2, 1].iter().enumerate() {
        wr32(&mut patches, 0x1c + 4 + 4 * i, *v);
    }
    assert_eq!(last_patch(2, patches.as_ptr()), Some([0xff, 0x40, 3, 0x10, 2, 1]));
    assert_eq!(last_patch(0, patches.as_ptr()), None);
    let mut dev = fake(0);
    let real = &mut dev as *mut FakeObj as *mut c_void;
    let mut out: *mut c_void = 1 as *mut c_void;
    assert_eq!(unsafe { derive_compute_on(real, 0xdead0 as *mut c_void, 2, patches.as_ptr(), &mut out) }, E_INVALIDARG);
    assert!(out.is_null());
    assert_eq!(unsafe { derive_compute_on(real, 0xdead0 as *mut c_void, 2, patches.as_ptr(), core::ptr::null_mut()) }, E_INVALIDARG);
    let code = std::sync::Arc::new(vec![0x44u8, 0x58, 0x42, 0x43]);
    let mut desc = vec![0u8; crate::pso_serial::COMPUTE_DESC_SIZE.max(0x38)];
    wr64(&mut desc, 0, 0x777);
    wr64(&mut desc, 8, code.as_ptr() as u64);
    wr64(&mut desc, 16, code.len() as u64);
    let base = 0xbeef0usize;
    crate::device_x::register_compute(base as *mut c_void, &desc, code.clone());
    take_calls();
    assert_eq!(unsafe { derive_compute_on(real, base as *mut c_void, 2, patches.as_ptr(), &mut out) }, S_OK);
    assert!(!out.is_null());
    let calls = take_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(rd64(&calls[0].2, 0), 0x777);
    assert_eq!(rd64(&calls[0].2, 8), code.as_ptr() as u64);
    assert_eq!(derived_limits(out as usize), Some(compute_resource_limits(&[0xff, 0x40, 3, 0x10, 2, 1], SCARLETT)));
    assert_eq!(crate::device_x::compute_code(out as usize).map(|c| c.len()), Some(4));
}

#[test]
fn install_fills_the_dev_a_slots() {
    let mut v = vec![0usize; 90];
    install(&mut v);
    for s in [45, 46, 47, 49, 51, 58, 59, 63] {
        assert_ne!(v[s], 0, "slot {s}");
    }
    assert_eq!(v[44], 0);
    assert_eq!(v[77], 0);
    assert_eq!(v[63], create_derived_compute_pso_x as usize);
}

fn tex_res(dim: u32, fmt: u32, samples: u32) -> [u8; 0x38] {
    let mut r = [0u8; 0x38];
    wr32(&mut r, 0, dim);
    wr32(&mut r, 0x20, fmt);
    wr32(&mut r, 0x24, samples);
    r
}

#[test]
fn unknown_texture_view_format_comes_from_the_resource_typed() {
    let mut pc = vec![0u8; 0x28];
    wr32(&mut pc, 4, 4);
    assert!(fit_texture_view(&mut pc, &tex_res(3, 60, 1), false));
    assert_eq!((rd32(&pc, 0), rd32(&pc, 4)), (61, 4), "R8_TYPELESS resource gives an R8_UNORM view");
    let mut pc = vec![0u8; 0x28];
    wr32(&mut pc, 4, 4);
    assert!(fit_texture_view(&mut pc, &tex_res(3, 26, 1), false));
    assert_eq!(rd32(&pc, 0), 26);
    let mut keep = vec![0u8; 0x28];
    wr32(&mut keep, 0, 28);
    wr32(&mut keep, 4, 4);
    assert!(!fit_texture_view(&mut keep, &tex_res(3, 27, 1), false), "a typed view format is kept");
    assert_eq!(rd32(&keep, 0), 28);
    let mut buf = vec![0u8; 0x28];
    wr32(&mut buf, 4, 1);
    assert!(!fit_texture_view(&mut buf, &tex_res(1, 0, 1), false), "buffer views keep UNKNOWN for structured data");
    assert_eq!(rd32(&buf, 0), 0);
}

#[test]
fn multisampled_resources_get_ms_view_dimensions() {
    let mut pc = vec![0u8; 0x28];
    wr32(&mut pc, 0, 61);
    wr32(&mut pc, 4, 4);
    wr32(&mut pc, 0x14, 1);
    assert!(fit_texture_view(&mut pc, &tex_res(3, 61, 2), false));
    assert_eq!(rd32(&pc, 4), 6);
    assert!(pc[0x10..0x28].iter().all(|b| *b == 0));
    let mut arr = vec![0u8; 0x28];
    wr32(&mut arr, 0, 61);
    wr32(&mut arr, 4, 5);
    wr32(&mut arr, 0x14, 1);
    wr32(&mut arr, 0x18, 2);
    wr32(&mut arr, 0x1c, 3);
    assert!(fit_texture_view(&mut arr, &tex_res(3, 61, 2), false));
    assert_eq!((rd32(&arr, 4), rd32(&arr, 0x10), rd32(&arr, 0x14)), (7, 2, 3), "array slices move to the MS array fields");
    let mut uav = vec![0u8; 0x28];
    wr32(&mut uav, 0, 61);
    wr32(&mut uav, 4, 4);
    assert!(!fit_texture_view(&mut uav, &tex_res(3, 61, 2), true));
}

#[test]
fn fitted_view_of_a_2x_r8_target_keeps_the_device_alive() {
    unsafe {
        let iid_dev = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
        let iid_res = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
        let iid_dh = Guid { d1: 0x8efb_471d, d2: 0x616c, d3: 0x4f49, d4: [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51] };
        let p = crate::host_proc(b"D3D12CreateDevice\0");
        let mk: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(p);
        let mut dev: *mut c_void = core::ptr::null_mut();
        assert!(mk(core::ptr::null_mut(), 0xb000, &iid_dev, &mut dev) >= 0 && !dev.is_null());
        let heap: [u32; 5] = [1, 0, 0, 1, 1];
        let mut rd = tex_res(3, 61, 2);
        rd[0x10..0x18].copy_from_slice(&1920u64.to_le_bytes());
        wr32(&mut rd, 0x18, 1080);
        rd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        rd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        wr32(&mut rd, 0x30, 1);
        let committed: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vfn(dev, 27));
        let mut r: *mut c_void = core::ptr::null_mut();
        assert!(committed(dev, heap.as_ptr(), 0, rd.as_ptr(), 4, core::ptr::null(), &iid_res, &mut r) >= 0 && !r.is_null());
        let hd = [0u32, 1, 0, 0];
        let mk_heap: unsafe extern "system" fn(*mut c_void, *const u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vfn(dev, 14));
        let mut dh: *mut c_void = core::ptr::null_mut();
        assert!(mk_heap(dev, hd.as_ptr(), &iid_dh, &mut dh) >= 0 && !dh.is_null());
        let cpu_start: unsafe extern "system" fn(*mut c_void, *mut usize) -> *mut usize = core::mem::transmute(vfn(dh, 9));
        let mut h = 0usize;
        cpu_start(dh, &mut h);
        let mut xdesc = vec![0u8; 0x48];
        wr32(&mut xdesc, 4, 4);
        wr32(&mut xdesc, 8, 0x1688);
        wr32(&mut xdesc, 0x14, 1);
        xdesc[0x28..0x30].copy_from_slice(&NO_ADDRESS.to_le_bytes());
        wr32(&mut xdesc, 0x30, 0x2f);
        wr32(&mut xdesc, 0x34, 4);
        let plan = plan_placed_view(r, xdesc.as_ptr(), false, &|_| None);
        match &plan {
            ViewPlan::Desc(_, d, _, _) => assert_eq!((rd32(d, 0), rd32(d, 4)), (61, 6), "the Trackmania view becomes an R8_UNORM TEXTURE2DMS view"),
            p => panic!("unexpected plan {p:?}"),
        }
        emit_srv(dev, plan, h);
        let removed: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(vfn(dev, 37));
        assert_eq!(removed(dev), 0);
        for o in [dh, r, dev] {
            let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vfn(o, 2));
            rel(o);
        }
    }
}