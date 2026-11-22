#![allow(non_snake_case)]

pub mod reset;
pub mod fortnite_trace;
pub mod platform;

pub mod exports {
    include!(concat!(env!("OUT_DIR"), "/forwarded.rs"));
}

use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::Once;

type HMODULE = *mut c_void;

extern "system" {
    fn LoadLibraryW(lpLibFileName: *const u16) -> HMODULE;
    fn GetProcAddress(hModule: HMODULE, lpProcName: *const u8) -> *mut c_void;
    fn GetModuleHandleExW(dwFlags: u32, lpModuleName: *const u16, phModule: *mut HMODULE) -> i32;
    fn GetModuleFileNameW(hModule: HMODULE, lpFilename: *mut u16, nSize: u32) -> u32;
}

const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 4;
const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 2;

pub const S_OK: i32 = 0;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const MF_E_SHUTDOWN: i32 = 0xC00D_3E85u32 as i32;
pub const MFASYNC_CALLBACK_QUEUE_MULTITHREADED: u32 = 5;

const SYS_STARTUP: usize = 0;
const SYS_SHUTDOWN: usize = 1;
const SYS_JOIN_WORK_QUEUE: usize = 2;
const N: usize = 3;

static NAMES: [&[u8]; N] = [b"MFStartup\0", b"MFShutdown\0", b"MFJoinWorkQueue\0"];

static PTRS: [AtomicUsize; N] = [const { AtomicUsize::new(0) }; N];
static INIT: Once = Once::new();
static STARTED: AtomicI32 = AtomicI32::new(0);
static PRIVATE_WORKQUEUES: AtomicU32 = AtomicU32::new(0);

#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(key: isize, subkey: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
}

const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
const RRF_RT_REG_DWORD: u32 = 0x10;

pub fn assign_queues_setting() -> Option<u32> {
    let key: Vec<u16> = concat!(r"SOFTWARE\Microsoft\Windows Media Foundation\RT", "\0").encode_utf16().collect();
    let value: Vec<u16> = "AssignQueues\0".encode_utf16().collect();
    let mut v = 0u32;
    let mut len = 4u32;
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, core::ptr::null_mut(), (&mut v as *mut u32).cast(), &mut len) };
    (r == 0).then_some(v)
}
static STARTUP_LOGS: AtomicU32 = AtomicU32::new(0);

type StartupFn = unsafe extern "system" fn(u32, u32) -> i32;
type ShutdownFn = unsafe extern "system" fn() -> i32;
type JoinWorkQueueFn = unsafe extern "system" fn(u32, *mut c_void, *mut u64) -> i32;

fn ensure() {
    INIT.call_once(|| {
        let mut hself: HMODULE = core::ptr::null_mut();
        let addr = ensure as *const c_void as *const u16;
        let ok = unsafe {
            GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, addr, &mut hself)
        };
        if ok == 0 {
            compat_common::record("mfplat", "ensure", "GetModuleHandleExW failed");
            return;
        }
        let mut buf = [0u16; 32768];
        let len = unsafe { GetModuleFileNameW(hself, buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if len == 0 || len >= buf.len() {
            compat_common::record("mfplat", "ensure", "GetModuleFileNameW failed");
            return;
        }
        let mut path: Vec<u16> = buf[..len].to_vec();
        let sep = path.iter().rposition(|&c| c == b'\\' as u16 || c == b'/' as u16);
        path.truncate(sep.map(|i| i + 1).unwrap_or(0));
        path.extend("mfplat_sys.dll\0".encode_utf16());
        let real = unsafe { LoadLibraryW(path.as_ptr()) };
        if real.is_null() {
            compat_common::record("mfplat", "ensure", "LoadLibraryW(mfplat_sys.dll) failed: is mfplat_sys.dll present next to mfplat.dll?");
            return;
        }
        let mut resolved = 0;
        for i in 0..N {
            let p = unsafe { GetProcAddress(real, NAMES[i].as_ptr()) } as usize;
            PTRS[i].store(p, Ordering::Relaxed);
            if p != 0 {
                resolved += 1;
            }
        }
        compat_common::record(
            "mfplat",
            "ensure",
            &format!(
                "loaded mfplat_sys.dll; {} exports are linker forwarders to mfplat_sys ({}), {resolved}/{N} native helpers resolved",
                exports::FORWARDED.len() + exports::FORWARDED_ALIASES.len(),
                exports::FORWARD_SOURCE
            ),
        );
    });
}

fn sys(idx: usize) -> usize {
    ensure();
    PTRS[idx].load(Ordering::Relaxed)
}

pub fn platform_started() -> bool {
    STARTED.load(Ordering::SeqCst) > 0
}

#[no_mangle]
pub unsafe extern "system" fn MFStartup(version: u32, flags: u32) -> i32 {
    let p = sys(SYS_STARTUP);
    if p == 0 {
        compat_common::record("mfplat", "MFStartup", "mfplat_sys!MFStartup unresolved -> E_FAIL");
        return compat_common::E_FAIL;
    }
    let f: StartupFn = unsafe { core::mem::transmute(p) };
    let held = platform::gate();
    let hr = unsafe { f(version, flags) };
    if hr >= 0 && assign_queues_setting().unwrap_or(0) != 0 {
        PRIVATE_WORKQUEUES.store(1, Ordering::SeqCst);
    }
    let n = if hr >= 0 { STARTED.fetch_add(1, Ordering::SeqCst) + 1 } else { STARTED.load(Ordering::SeqCst) };
    if hr >= 0 {
        platform::on_startup(&held, version, flags);
    }
    drop(held);
    if STARTUP_LOGS.fetch_add(1, Ordering::Relaxed) < 16 {
        compat_common::record("mfplat", "MFStartup", &format!("version={version:#x} flags={flags:#x} -> hr={:#010x} startups={n}", hr as u32));
    }
    hr
}

#[no_mangle]
pub unsafe extern "system" fn MFShutdown() -> i32 {
    let p = sys(SYS_SHUTDOWN);
    if p == 0 {
        compat_common::record("mfplat", "MFShutdown", "mfplat_sys!MFShutdown unresolved -> E_FAIL");
        return compat_common::E_FAIL;
    }
    let f: ShutdownFn = unsafe { core::mem::transmute(p) };
    let held = platform::gate();
    let hr = unsafe { f() };
    let prev = if hr >= 0 { STARTED.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| Some((v - 1).max(0))).unwrap_or(0) } else { STARTED.load(Ordering::SeqCst) };
    if hr >= 0 && prev > 0 {
        platform::on_shutdown(&held);
    }
    drop(held);
    let n = if hr >= 0 { (prev - 1).max(0) } else { prev };
    if STARTUP_LOGS.fetch_add(1, Ordering::Relaxed) < 16 {
        compat_common::record("mfplat", "MFShutdown", &format!("-> hr={:#010x} startups={n}", hr as u32));
    }
    hr
}

#[no_mangle]
pub unsafe extern "system" fn MFResetDXGIDeviceManagerX(device_manager: *mut c_void, device: *mut c_void, reset_token: u32) -> i32 {
    ensure();
    let out = unsafe { reset::reset_device_manager(device_manager, device, reset_token) };
    compat_common::record(
        "mfplat",
        "MFResetDXGIDeviceManagerX",
        &format!(
            "manager={:#x} xbox_device={:#x} -> real ID3D12Device {:#x} via {} token={reset_token} ResetDevice hr={:#010x}",
            device_manager as usize, device as usize, out.real_device, out.via, out.hr as u32
        ),
    );
    out.hr
}

#[no_mangle]
pub extern "system" fn MFGetPlatform() -> *mut c_void {
    platform::platform()
}

#[no_mangle]
pub extern "system" fn LFGetGlobalPool() -> *mut c_void {
    core::ptr::null_mut()
}

fn block_state() -> i32 {
    if platform_started() {
        S_OK
    } else {
        MF_E_SHUTDOWN
    }
}

#[no_mangle]
pub extern "system" fn MFBlockThread() -> i32 {
    block_state()
}

#[no_mangle]
pub extern "system" fn MFUnblockThread() -> i32 {
    block_state()
}

#[no_mangle]
pub unsafe extern "system" fn MFGetPrivateWorkqueues(out: *mut u32) -> i32 {
    if !platform_started() {
        return MF_E_SHUTDOWN;
    }
    if out.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *out = PRIVATE_WORKQUEUES.load(Ordering::SeqCst) };
    S_OK
}

#[no_mangle]
pub unsafe extern "system" fn MFJoinIoPort(handle: *mut c_void) -> i32 {
    let p = sys(SYS_JOIN_WORK_QUEUE);
    if p == 0 {
        compat_common::record("mfplat", "MFJoinIoPort", "mfplat_sys!MFJoinWorkQueue unresolved -> E_FAIL");
        return compat_common::E_FAIL;
    }
    let f: JoinWorkQueueFn = unsafe { core::mem::transmute(p) };
    let mut cookie = 0u64;
    let hr = unsafe { f(MFASYNC_CALLBACK_QUEUE_MULTITHREADED, handle, &mut cookie) };
    compat_common::record("mfplat", "MFJoinIoPort", &format!("handle={:#x} -> MFJoinWorkQueue(MULTITHREADED) hr={:#010x}", handle as usize, hr as u32));
    hr
}
