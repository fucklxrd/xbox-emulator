mod console_x;
mod esram;
mod iids;
#[cfg(test)]
mod iid_console_tests;
mod xgs_kmd;
mod pixie;
mod microcode;
mod driver_call;
pub mod rs_strip;
#[cfg(test)]
mod rs_strip_tests;

use compat_common::{record, E_FAIL, S_OK};
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

pub use esram::D3DMapEsramMemory;

const M: &str = "d3d12_x";
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const S_FALSE: i32 = 1;
const D3D_FEATURE_LEVEL_12_0: u32 = 0xc000;
const DEBUG_FLAG_MASK: u32 = 0x81f0_0001;

type HMODULE = *mut c_void;
extern "system" {
    fn LoadLibraryW(p: *const u16) -> HMODULE;
    fn GetProcAddress(h: HMODULE, n: *const u8) -> *mut c_void;
    fn GetModuleHandleExW(flags: u32, addr: *const c_void, out: *mut HMODULE) -> i32;
    fn GetModuleFileNameW(h: HMODULE, buf: *mut u16, n: u32) -> u32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn cached(cell: &AtomicUsize, open: impl FnOnce() -> usize) -> HMODULE {
    let v = cell.load(Ordering::Acquire);
    if v != 0 {
        return if v == 1 { core::ptr::null_mut() } else { v as HMODULE };
    }
    let h = open();
    cell.store(if h == 0 { 1 } else { h }, Ordering::Release);
    h as HMODULE
}

fn own_dir() -> Option<String> {
    let mut h: HMODULE = core::ptr::null_mut();
    let ok = unsafe { GetModuleHandleExW(0x4 | 0x2, own_dir as *const c_void, &mut h) };
    if ok == 0 {
        return None;
    }
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(h, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    let path = String::from_utf16_lossy(&buf[..n]);
    path.rfind('\\').map(|i| path[..i].to_string())
}

fn host_d3d12() -> HMODULE {
    static CELL: AtomicUsize = AtomicUsize::new(0);
    cached(&CELL, || unsafe { LoadLibraryW(wide(r"C:\Windows\System32\d3d12.dll").as_ptr()) as usize })
}

fn host_d3dcompiler() -> HMODULE {
    static CELL: AtomicUsize = AtomicUsize::new(0);
    cached(&CELL, || unsafe { LoadLibraryW(wide(r"C:\Windows\System32\d3dcompiler_47.dll").as_ptr()) as usize })
}

fn scarlett_runtime() -> HMODULE {
    static CELL: AtomicUsize = AtomicUsize::new(0);
    cached(&CELL, || {
        let sibling = own_dir().map(|d| format!("{d}\\d3d12_xs.dll"));
        let mut h = sibling.map_or(0, |p| unsafe { LoadLibraryW(wide(&p).as_ptr()) as usize });
        if h == 0 {
            h = unsafe { LoadLibraryW(wide("d3d12_xs.dll").as_ptr()) as usize };
        }
        record(M, "init", &format!("emulator d3d12_xs.dll = {h:#x}"));
        h
    })
}

fn proc_in(h: HMODULE, name: &[u8]) -> usize {
    if h.is_null() {
        return 0;
    }
    unsafe { GetProcAddress(h, name.as_ptr()) as usize }
}

macro_rules! forward {
    ($module:expr, $name:literal, fn($($a:ident: $t:ty),*) -> i32) => {{
        let p = proc_in($module, concat!($name, "\0").as_bytes());
        if p == 0 {
            record(M, $name, "host export missing -> E_FAIL");
            return E_FAIL;
        }
        let f: unsafe extern "system" fn($($t),*) -> i32 = unsafe { core::mem::transmute(p) };
        unsafe { f($($a),*) }
    }};
}

#[no_mangle]
pub extern "system" fn D3D12XboxCreateDevice(
    adapter: *mut c_void,
    params: *const c_void,
    riid: *const c_void,
    pp_device: *mut *mut c_void,
) -> i32 {
    let _ = xgs_kmd_emu::api();
    if params.is_null() {
        record(M, "D3D12XboxCreateDevice", "null parameters -> E_INVALIDARG");
        return E_INVALIDARG;
    }
    let mut local: *mut c_void = core::ptr::null_mut();
    let caller_out = pp_device;
    let pp_device: *mut *mut c_void = if caller_out.is_null() { &mut local } else { caller_out };
    unsafe { *pp_device = core::ptr::null_mut() };
    let hr = create_device_into(adapter, params, riid, pp_device);
    if caller_out.is_null() && !local.is_null() {
        unsafe {
            let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(local as *const *const usize)).add(2));
            rel(local);
        }
    }
    hr
}

fn create_device_into(adapter: *mut c_void, params: *const c_void, riid: *const c_void, pp_device: *mut *mut c_void) -> i32 {
    let xone = unsafe { core::slice::from_raw_parts(params as *const u8, XONE_PARAMS_BYTES) };
    let xs_params = scarlett_params(xone);
    DEBUG_FLAGS.store(xs_params[1], Ordering::Relaxed);
    let xs = proc_in(scarlett_runtime(), b"D3D12XboxCreateDevice\0");
    if xs != 0 {
        let f: unsafe extern "system" fn(*mut c_void, *const c_void, *const c_void, *mut *mut c_void) -> i32 =
            unsafe { core::mem::transmute(xs) };
        let hr = unsafe { f(adapter, xs_params.as_ptr() as *const c_void, riid, pp_device) };
        record(M, "D3D12XboxCreateDevice", &format!("Xbox One params version {:#x} flags {:#x} -> Scarlett version {:#x}; d3d12_xs device {:p} hr={hr:#010x}", xone_params_version(xone), xs_params[1], xs_params[0], unsafe { *pp_device }));
        return hr;
    }
    let hr = host_create_device(core::ptr::null_mut(), D3D_FEATURE_LEVEL_12_0, riid, pp_device);
    record(M, "D3D12XboxCreateDevice", &format!("d3d12_xs.dll unavailable; unwrapped host device hr={hr:#010x}"));
    hr
}

#[no_mangle]
pub extern "system" fn D3D12CreateDevice(adapter: *mut c_void, level: u32, riid: *const c_void, pp: *mut *mut c_void) -> i32 {
    let params = default_xone_params(D3D12XboxGetProcessDebugFlags());
    record(M, "D3D12CreateDevice", &format!("feature level {level:#x} ignored as on console; D3D12XboxCreateDevice with version {:#x} flags {:#x}", params[0], params[1]));
    D3D12XboxCreateDevice(adapter, params.as_ptr() as *const c_void, riid, pp)
}

fn host_create_device(adapter: *mut c_void, level: u32, riid: *const c_void, pp: *mut *mut c_void) -> i32 {
    forward!(host_d3d12(), "D3D12CreateDevice", fn(adapter: *mut c_void, level: u32, riid: *const c_void, pp: *mut *mut c_void) -> i32)
}

pub const XONE_PARAMS_VERSION: u32 = 0x1000a;
pub const XONE_PARAMS_BYTES: usize = 0x28;
pub const XS_PARAMS_VERSION: u32 = 0x20004;
pub const XS_PARAMS_DWORDS: usize = 0x48 / 4;

pub fn default_xone_params(flags: u32) -> [u32; XONE_PARAMS_BYTES / 4] {
    let mut p = [0u32; XONE_PARAMS_BYTES / 4];
    p[0] = XONE_PARAMS_VERSION;
    p[1] = flags;
    p[2] = u32::MAX;
    p[6] = u32::MAX;
    p[7] = u32::MAX;
    p
}

fn xone_params_version(xone: &[u8]) -> u32 {
    u32::from_le_bytes(xone[0..4].try_into().unwrap())
}

pub fn scarlett_params(xone: &[u8]) -> [u32; XS_PARAMS_DWORDS] {
    let mut p = [0u32; XS_PARAMS_DWORDS];
    for (i, c) in xone.chunks_exact(4).take(XONE_PARAMS_BYTES / 4).enumerate() {
        p[i] = u32::from_le_bytes(c.try_into().unwrap());
    }
    p[0] = XS_PARAMS_VERSION;
    p
}

#[no_mangle]
pub extern "system" fn D3D12GetDebugInterface(riid: *const c_void, pp: *mut *mut c_void) -> i32 {
    forward!(host_d3d12(), "D3D12GetDebugInterface", fn(riid: *const c_void, pp: *mut *mut c_void) -> i32)
}

#[no_mangle]
pub extern "system" fn D3D12SerializeRootSignature(desc: *const c_void, version: u32, blob: *mut *mut c_void, err: *mut *mut c_void) -> i32 {
    let p = proc_in(host_d3d12(), b"D3D12SerializeRootSignature\0");
    if p == 0 {
        return E_FAIL;
    }
    unsafe { rs_strip::serialize(core::mem::transmute(p), desc, version, blob, err) }
}

#[no_mangle]
pub extern "system" fn D3D12SerializeVersionedRootSignature(desc: *const c_void, blob: *mut *mut c_void, err: *mut *mut c_void) -> i32 {
    let p = proc_in(host_d3d12(), b"D3D12SerializeVersionedRootSignature\0");
    if p == 0 {
        return E_FAIL;
    }
    unsafe { rs_strip::serialize_versioned(core::mem::transmute(p), desc, blob, err) }
}

#[no_mangle]
pub extern "system" fn D3D12CreateRootSignatureDeserializer(data: *const c_void, size: usize, riid: *const c_void, pp: *mut *mut c_void) -> i32 {
    let p = proc_in(host_d3d12(), b"D3D12CreateRootSignatureDeserializer\0");
    if p == 0 {
        return E_FAIL;
    }
    unsafe { rs_strip::deserializer(core::mem::transmute(p), false, data, size, riid, pp) }
}

#[no_mangle]
pub extern "system" fn D3D12CreateVersionedRootSignatureDeserializer(data: *const c_void, size: usize, riid: *const c_void, pp: *mut *mut c_void) -> i32 {
    let p = proc_in(host_d3d12(), b"D3D12CreateVersionedRootSignatureDeserializer\0");
    if p == 0 {
        return E_FAIL;
    }
    unsafe { rs_strip::deserializer(core::mem::transmute(p), true, data, size, riid, pp) }
}

#[no_mangle]
pub extern "system" fn D3D10CreateBlob(size: usize, pp: *mut *mut c_void) -> i32 {
    forward!(host_d3dcompiler(), "D3DCreateBlob", fn(size: usize, pp: *mut *mut c_void) -> i32)
}

static DEBUG_FLAGS: AtomicU32 = AtomicU32::new(0);

#[no_mangle]
pub extern "system" fn D3D12XboxSetProcessDebugFlags(flags: u32) -> i32 {
    DEBUG_FLAGS.store(flags & DEBUG_FLAG_MASK, Ordering::Relaxed);
    record(M, "D3D12XboxSetProcessDebugFlags", &format!("{flags:#x} -> stored {:#x}", flags & DEBUG_FLAG_MASK));
    S_OK
}

#[no_mangle]
pub extern "system" fn D3D12XboxGetProcessDebugFlags() -> u32 {
    DEBUG_FLAGS.load(Ordering::Relaxed)
}


#[cfg(test)]
mod tests {
    use super::*;

    const EXPORTS: &str = include_str!("../exports.txt");

    #[test]
    fn export_table_matches_reference_dll() {
        xone_exports::check_crate(env!("CARGO_MANIFEST_DIR"), "d3d12_x.dll", EXPORTS);
    }

    #[test]
    fn built_dll_has_reference_ordinals() {
        xone_exports::check_built(env!("CARGO_MANIFEST_DIR"), "d3d12_x.dll", EXPORTS);
    }

    #[test]
    fn debug_flags_round_trip_with_real_mask() {
        assert_eq!(D3D12XboxSetProcessDebugFlags(0xffff_ffff), S_OK);
        assert_eq!(D3D12XboxGetProcessDebugFlags(), 0x81f0_0001);
        assert_eq!(default_xone_params(D3D12XboxGetProcessDebugFlags())[1], 0x81f0_0001);
        D3D12XboxSetProcessDebugFlags(0);
        assert_eq!(D3D12XboxGetProcessDebugFlags(), 0);
    }

    #[test]
    fn create_device_rejects_null_parameters() {
        let mut p: *mut c_void = core::ptr::null_mut();
        assert_eq!(D3D12XboxCreateDevice(core::ptr::null_mut(), core::ptr::null(), core::ptr::null(), &mut p), E_INVALIDARG);
    }

    #[test]
    fn create_device_null_out_returns_the_creation_result_like_console() {
        let params = default_xone_params(0);
        let iid: [u32; 4] = [0x1898_19f1, 0x4b57_1db6, 0x2118_54be, 0xf785_9b33];
        let hr_null = D3D12XboxCreateDevice(core::ptr::null_mut(), params.as_ptr() as *const c_void, iid.as_ptr() as *const c_void, core::ptr::null_mut());
        let mut p: *mut c_void = core::ptr::null_mut();
        let hr = D3D12XboxCreateDevice(core::ptr::null_mut(), params.as_ptr() as *const c_void, iid.as_ptr() as *const c_void, &mut p);
        assert_ne!(hr_null, S_FALSE);
        assert_eq!(hr_null, hr);
        if !p.is_null() {
            unsafe {
                let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(p as *const *const usize)).add(2));
                rel(p);
            }
        }
    }

    #[test]
    fn blob_forwards_to_host_compiler() {
        let mut b: *mut c_void = core::ptr::null_mut();
        assert_eq!(D3D10CreateBlob(64, &mut b), S_OK);
        assert!(!b.is_null());
        unsafe {
            let vt = *(b as *const *const usize);
            let size: unsafe extern "system" fn(*mut c_void) -> usize = core::mem::transmute(*vt.add(4));
            assert_eq!(size(b), 64);
            let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
            rel(b);
        }
    }
}
