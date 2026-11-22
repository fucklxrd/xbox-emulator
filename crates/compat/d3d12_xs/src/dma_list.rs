use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use compat_common::record;

use crate::device_wrap::Guid;

pub const IID_XBOX_DMA_COMMAND_LIST: Guid =
    Guid { d1: 0x30bc_77b9, d2: 0x8379, d3: 0x4b25, d4: [0x95, 0xde, 0xb8, 0x58, 0x8c, 0x42, 0x27, 0x5d] };
pub const IID_GRAPHICS_COMMAND_LIST: Guid =
    Guid { d1: 0x5b16_0d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] };

const N: usize = 32;
const REAL_OFF: usize = 0x38;
const SHIM_SIZE: usize = 0x400;
const PC_RESOURCE_BARRIER: usize = 26;
const STATE_COPY_DEST: u32 = 0x400;
const STATE_COPY_SOURCE: u32 = 0x800;

static TARGET: [AtomicUsize; N] = [const { AtomicUsize::new(0) }; N];
static THUNKS: [AtomicUsize; N] = [const { AtomicUsize::new(0) }; N];
static REAL_BARRIER: AtomicUsize = AtomicUsize::new(0);
static EXE_BASE: AtomicUsize = AtomicUsize::new(0);
static BUILT: std::sync::Once = std::sync::Once::new();

pub fn map_slot(i: usize) -> Option<usize> {
    match i {
        0..=10 => Some(i),
        11 => Some(15),
        12 => Some(16),
        13 => Some(17),
        15 => Some(PC_RESOURCE_BARRIER),
        _ => None,
    }
}

pub fn copy_legal_barriers(src: &[u8]) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(src.len());
    let mut dropped = 0;
    for b in src.chunks_exact(32) {
        let ty = u32::from_le_bytes(b[0..4].try_into().unwrap());
        if ty != 0 {
            dropped += 1;
            continue;
        }
        let mask = STATE_COPY_DEST | STATE_COPY_SOURCE;
        let before = u32::from_le_bytes(b[20..24].try_into().unwrap()) & mask;
        let after = u32::from_le_bytes(b[24..28].try_into().unwrap()) & mask;
        if before == after {
            dropped += 1;
            continue;
        }
        let mut r = [0u8; 32];
        r.copy_from_slice(b);
        r[20..24].copy_from_slice(&before.to_le_bytes());
        r[24..28].copy_from_slice(&after.to_le_bytes());
        out.extend_from_slice(&r);
    }
    (out, dropped)
}

unsafe extern "system" fn resource_barrier_dma(this: *mut c_void, n: u32, barriers: *const u8) {
    if n == 0 || barriers.is_null() {
        return;
    }
    let src = core::slice::from_raw_parts(barriers, n as usize * 32);
    crate::gpu_va::note_barriers(n, barriers);
    let (fixed, dropped) = copy_legal_barriers(src);
    static LOGS: AtomicUsize = AtomicUsize::new(0);
    if dropped != 0 && LOGS.fetch_add(1, Ordering::Relaxed) < 4 {
        record("d3d12_xs", "dma.barrier", &format!("n={n} kept={} dropped={dropped}", fixed.len() / 32));
    }
    if fixed.is_empty() {
        return;
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8) =
        core::mem::transmute(REAL_BARRIER.load(Ordering::Relaxed));
    f(this, (fixed.len() / 32) as u32, fixed.as_ptr());
}

pub const DMA_SLOT_FILL: usize = 23;
pub const DMA_SLOT_TIMESTAMP: usize = 24;

unsafe extern "system" fn timestamp_dma(this: *mut c_void, addr: u64, a: u32, b: u32, c: u32) {
    crate::timestamp::write(this, true, addr, a, b, c);
}

static REAL_RESET: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn reset_dma(this: *mut c_void, alloc: *mut c_void, pso: *mut c_void) -> i32 {
    crate::ts99::forget_list(this);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 = core::mem::transmute(REAL_RESET.load(Ordering::Relaxed));
    f(this, alloc, pso)
}

unsafe extern "system" fn fill_dma(this: *mut c_void, va: u64, size: u32, value: u32, flags: u32) {
    crate::fill::fill(this, true, va, size, value, flags);
}

unsafe extern "system" fn probe_impl(_this: *mut c_void, a: u64, caller: usize, slot: u64) {
    static PER: [AtomicUsize; N] = [const { AtomicUsize::new(0) }; N];
    if PER[(slot as usize).min(N - 1)].fetch_add(1, Ordering::Relaxed) < 4 {
        let base = EXE_BASE.load(Ordering::Relaxed);
        record("d3d12_xs", "dma.unmapped", &format!("slot={slot} arg1={a:#x} caller={:#x}", caller.wrapping_sub(base)));
    }
}

macro_rules! dma_probes {
    ($(($name:ident, $slot:literal)),* $(,)?) => {
        $(
            #[unsafe(naked)]
            unsafe extern "system" fn $name() {
                core::arch::naked_asm!(
                    "mov r8, [rsp]",
                    "mov r9d, {s}",
                    "jmp {i}",
                    s = const $slot, i = sym probe_impl,
                );
            }
        )*
        fn probe_for(slot: usize) -> usize {
            match slot {
                $( $slot => $name as usize, )*
                _ => 0,
            }
        }
    };
}
dma_probes!(
    (q14,14),(q16,16),(q17,17),(q18,18),(q19,19),(q20,20),(q21,21),(q22,22),
    (q23,23),(q24,24),(q25,25),(q26,26),(q27,27),(q28,28),(q29,29),(q30,30),(q31,31),
);

unsafe fn build(real_vt: *const usize) {
    extern "system" {
        fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, protect: u32) -> *mut c_void;
        fn GetModuleHandleW(n: *const u16) -> *mut c_void;
    }
    EXE_BASE.store(GetModuleHandleW(core::ptr::null()) as usize, Ordering::Relaxed);
    REAL_BARRIER.store(*real_vt.add(PC_RESOURCE_BARRIER), Ordering::Relaxed);
    for i in 0..N {
        let t = match map_slot(i) {
            Some(PC_RESOURCE_BARRIER) => resource_barrier_dma as usize,
            Some(pc) => *real_vt.add(pc),
            None => probe_for(i),
        };
        TARGET[i].store(t, Ordering::Relaxed);
    }
    TARGET[12].store(crate::video_probe::install(*real_vt.add(16)), Ordering::Relaxed);
    TARGET[DMA_SLOT_FILL].store(fill_dma as usize, Ordering::Relaxed);
    TARGET[DMA_SLOT_TIMESTAMP].store(timestamp_dma as usize, Ordering::Relaxed);
    REAL_RESET.store(*real_vt.add(10), Ordering::Relaxed);
    TARGET[10].store(reset_dma as usize, Ordering::Relaxed);
    let code = VirtualAlloc(core::ptr::null_mut(), N * 16, 0x3000, 0x40) as *mut u8;
    assert!(!code.is_null(), "d3d12_xs: cannot allocate DMA list thunks");
    for i in 0..N {
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&[0x48, 0x8B, 0x49, REAL_OFF as u8]);
        b[4] = 0x48;
        b[5] = 0xB8;
        b[6..14].copy_from_slice(&(TARGET.as_ptr().add(i) as u64).to_le_bytes());
        b[14] = 0xFF;
        b[15] = 0x20;
        core::ptr::copy_nonoverlapping(b.as_ptr(), code.add(i * 16), 16);
        THUNKS[i].store(code.add(i * 16) as usize, Ordering::Relaxed);
    }
    record("d3d12_xs", "dma.vtable", &format!("built Xbox DMA list vtable from real {real_vt:p}"));
}

pub fn wrap(real_list: *mut c_void) -> *mut c_void {
    if real_list.is_null() {
        return real_list;
    }
    unsafe {
        let real_vt = *(real_list as *const *const usize);
        BUILT.call_once(|| build(real_vt));
        crate::ts99::forget_list(real_list);
        let layout = std::alloc::Layout::from_size_align(SHIM_SIZE, 16).unwrap();
        let p = std::alloc::alloc_zeroed(layout);
        assert!(!p.is_null(), "d3d12_xs: cannot allocate DMA list shim");
        *(p as *mut usize) = THUNKS.as_ptr() as usize;
        *(p.add(REAL_OFF) as *mut usize) = real_list as usize;
        record("d3d12_xs", "dma.shim", &format!("real {real_list:p} wrapped as DMA shim {p:p}"));
        p as *mut c_void
    }
}

pub fn unwrap(p: *mut c_void) -> Option<*mut c_void> {
    if p.is_null() {
        return None;
    }
    unsafe {
        if *(p as *const usize) == THUNKS.as_ptr() as usize {
            Some(*((p as *const u8).add(REAL_OFF) as *const usize) as *mut c_void)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn barrier(ty: u32, before: u32, after: u32) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0..4].copy_from_slice(&ty.to_le_bytes());
        b[8..16].copy_from_slice(&0x1234u64.to_le_bytes());
        b[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        b[20..24].copy_from_slice(&before.to_le_bytes());
        b[24..28].copy_from_slice(&after.to_le_bytes());
        b
    }

    #[test]
    fn dma_slot_map() {
        for i in 0..=10 { assert_eq!(map_slot(i), Some(i)); }
        assert_eq!(map_slot(11), Some(15));
        assert_eq!(map_slot(12), Some(16));
        assert_eq!(map_slot(13), Some(17));
        assert_eq!(map_slot(15), Some(26));
        for i in [14, 16, 23, 24, 31] { assert_eq!(map_slot(i), None); }
        for i in 0..N {
            assert!(map_slot(i).is_some() || probe_for(i) != 0, "slot {i} has no target");
        }
    }

    #[test]
    fn dma_barriers_fortnite_video_upload() {
        let mut src = Vec::new();
        src.extend_from_slice(&barrier(0, 0, 0x800));
        src.extend_from_slice(&barrier(0, 0x880, 0x400));
        src.extend_from_slice(&barrier(0, 0x880, 0x400));
        let (out, dropped) = copy_legal_barriers(&src);
        assert_eq!(dropped, 0);
        assert_eq!(out.len(), 96);
        let st = |k: usize, o: usize| u32::from_le_bytes(out[k * 32 + o..k * 32 + o + 4].try_into().unwrap());
        assert_eq!((st(0, 20), st(0, 24)), (0, 0x800));
        assert_eq!((st(1, 20), st(1, 24)), (0x800, 0x400));
        assert_eq!(&out[40..48], &0x1234u64.to_le_bytes());
        let mut back = Vec::new();
        back.extend_from_slice(&barrier(0, 0x800, 0));
        back.extend_from_slice(&barrier(0, 0x400, 0x880));
        back.extend_from_slice(&barrier(2, 0, 0));
        back.extend_from_slice(&barrier(0, 0x80, 0x40));
        let (out, dropped) = copy_legal_barriers(&back);
        assert_eq!(dropped, 2);
        assert_eq!(out.len(), 64);
    }
}
