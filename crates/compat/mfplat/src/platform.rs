use core::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use console_image::ConsoleDll;

pub static CONSOLE: ConsoleDll = ConsoleDll::new("mfplat.dll", "TOMBOX_MFPLAT_CONSOLE", "mfplat_console.dll", "MFGetPlatform", &[]);

static SIDE_STARTS: AtomicI32 = AtomicI32::new(0);
static GATE: Mutex<()> = Mutex::new(());

pub fn gate() -> std::sync::MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}
static MODULE: OnceLock<usize> = OnceLock::new();

extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetModuleHandleExW(flags: u32, name: *const u16, out: *mut *mut c_void) -> i32;
    fn GetModuleFileNameW(h: *mut c_void, buf: *mut u16, n: u32) -> u32;
}

type StartupFn = unsafe extern "system" fn(u32, u32) -> i32;
type ShutdownFn = unsafe extern "system" fn() -> i32;
type GetPlatformFn = unsafe extern "system" fn() -> *mut c_void;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn own_dir() -> Option<std::path::PathBuf> {
    let mut h: *mut c_void = core::ptr::null_mut();
    let ok = unsafe { GetModuleHandleExW(6, own_dir as *const () as *const u16, &mut h) };
    if ok == 0 {
        return None;
    }
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(h, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0).then(|| std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n]))).and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

fn ensure_dep(name: &str) -> bool {
    if !unsafe { GetModuleHandleW(wide(name).as_ptr()) }.is_null() {
        return true;
    }
    let Some(dir) = own_dir() else { return false };
    let p = dir.join(name);
    if !p.is_file() {
        return false;
    }
    !unsafe { LoadLibraryW(wide(&p.to_string_lossy()).as_ptr()) }.is_null()
}

pub fn module() -> usize {
    *MODULE.get_or_init(|| {
        if !ensure_dep("xmem.dll") {
            compat_common::record("mfplat", "MFGetPlatform", "the console mfplat.dll imports XMemAlloc/XMemFree; no xmem.dll is loaded or staged beside mfplat.dll, so the console platform object cannot be started");
            return 0;
        }
        ensure_dep("MFLNMHelp.dll");
        let m = CONSOLE.module(own_dir as *const () as usize);
        if m != 0 {
            let hits = unsafe { console_sandbox::attach(m) };
            compat_common::record("mfplat", "MFGetPlatform", &format!("console mfplat.dll platform sidecar at {m:#x}, {} OS imports routed to console-sandbox", hits.len()));
        }
        m
    })
}

fn proc<T: Copy>(m: usize, name: &str) -> Option<T> {
    console_image::symbol(m, name).map(|p| unsafe { core::mem::transmute_copy(&p) })
}

pub fn on_startup(_held: &std::sync::MutexGuard<'static, ()>, version: u32, flags: u32) {
    let m = module();
    let Some(f) = proc::<StartupFn>(m, "MFStartup") else { return };
    let hr = unsafe { f(version, flags) };
    if hr >= 0 {
        SIDE_STARTS.fetch_add(1, Ordering::SeqCst);
    } else {
        compat_common::record("mfplat", "MFGetPlatform", &format!("console platform MFStartup({version:#x}, {flags:#x}) failed hr={:#010x}; MFGetPlatform keeps returning NULL", hr as u32));
    }
}

pub fn on_shutdown(_held: &std::sync::MutexGuard<'static, ()>) {
    if SIDE_STARTS.load(Ordering::SeqCst) <= 0 {
        return;
    }
    let Some(f) = proc::<ShutdownFn>(module(), "MFShutdown") else { return };
    unsafe { f() };
    SIDE_STARTS.fetch_sub(1, Ordering::SeqCst);
}

pub fn platform() -> *mut c_void {
    let m = match MODULE.get() {
        Some(&m) => m,
        None => return core::ptr::null_mut(),
    };
    match proc::<GetPlatformFn>(m, "MFGetPlatform") {
        Some(f) => unsafe { f() },
        None => core::ptr::null_mut(),
    }
}

pub fn sidecar_starts() -> i32 {
    SIDE_STARTS.load(Ordering::SeqCst)
}
