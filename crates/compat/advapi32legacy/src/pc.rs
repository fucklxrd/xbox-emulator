use core::ffi::c_void;
use std::sync::OnceLock;

use crate::{Handle, NtStatus};

pub const STATUS_DLL_NOT_FOUND: NtStatus = 0xC000_0135u32 as i32;
pub const STATUS_ENTRYPOINT_NOT_FOUND: NtStatus = 0xC000_0139u32 as i32;

pub const STAGED_NAME: &str = "advapi32_sys.dll";

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(m: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetSystemDirectoryW(buf: *mut u16, n: u32) -> u32;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

pub type RegisterEventSourceWFn = unsafe extern "system" fn(*const u16, *const u16) -> Handle;
pub type DeregisterEventSourceFn = unsafe extern "system" fn(Handle) -> i32;
pub type ReportEventWFn = unsafe extern "system" fn(Handle, u16, u16, u32, *mut c_void, u16, u32, *const *const u16, *const c_void) -> i32;

pub struct Pc {
    pub module: usize,
    pub register: Option<RegisterEventSourceWFn>,
    pub deregister: Option<DeregisterEventSourceFn>,
    pub report: Option<ReportEventWFn>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn system32_advapi32() -> Option<Vec<u16>> {
    let mut b = vec![0u16; 520];
    let n = unsafe { GetSystemDirectoryW(b.as_mut_ptr(), b.len() as u32) } as usize;
    if n == 0 || n >= b.len() {
        return None;
    }
    b.truncate(n);
    b.extend("\\advapi32.dll\0".encode_utf16());
    Some(b)
}

unsafe fn module() -> *mut c_void {
    let staged = GetModuleHandleW(wide(STAGED_NAME).as_ptr());
    if !staged.is_null() {
        return staged;
    }
    match system32_advapi32() {
        Some(p) => LoadLibraryExW(p.as_ptr(), core::ptr::null_mut(), 0),
        None => core::ptr::null_mut(),
    }
}

unsafe fn proc<T>(m: *mut c_void, name: &[u8]) -> Option<T> {
    let p = GetProcAddress(m, name.as_ptr());
    (!p.is_null()).then(|| core::mem::transmute_copy(&p))
}

pub fn pc() -> &'static Pc {
    static PC: OnceLock<Pc> = OnceLock::new();
    PC.get_or_init(|| unsafe {
        let saved = GetLastError();
        let pc = bind();
        SetLastError(saved);
        pc
    })
}

unsafe fn bind() -> Pc {
    let m = module();
    if m.is_null() {
        return Pc { module: 0, register: None, deregister: None, report: None };
    }
    Pc {
        module: m as usize,
        register: proc(m, b"RegisterEventSourceW\0"),
        deregister: proc(m, b"DeregisterEventSource\0"),
        report: proc(m, b"ReportEventW\0"),
    }
}

pub fn missing_status() -> NtStatus {
    if pc().module == 0 {
        STATUS_DLL_NOT_FOUND
    } else {
        STATUS_ENTRYPOINT_NOT_FOUND
    }
}
