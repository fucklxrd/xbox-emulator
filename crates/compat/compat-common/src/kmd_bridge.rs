use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const VAR: &str = "TOMBOX_XGSKMD_OWNER";
pub const VERSION: u32 = 1;

#[repr(C)]
pub struct KmdApi {
    pub version: u32,
    pub open: unsafe extern "C" fn() -> isize,
    pub is_device: unsafe extern "C" fn(isize) -> i32,
    pub close: unsafe extern "C" fn(isize) -> i32,
    pub ioctl: unsafe extern "C" fn(isize, u32, *const c_void, u32, *mut c_void, u32, *mut u32) -> i32,
    pub query: unsafe extern "C" fn(u32, u64, *mut u64) -> i32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn CreateMutexW(sa: *mut c_void, owner: i32, name: *const u16) -> isize;
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
    fn ReleaseMutex(h: isize) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn SetLastError(e: u32);
    fn GetLastError() -> u32;
}

static OWNER: AtomicUsize = AtomicUsize::new(0);

fn parse(v: &str, pid: u32) -> Option<usize> {
    let (p, a) = v.split_once(':')?;
    if u32::from_str_radix(p, 16).ok()? != pid {
        return None;
    }
    usize::from_str_radix(a, 16).ok().filter(|&a| a != 0)
}

fn from_env() -> Option<&'static KmdApi> {
    let pid = unsafe { GetCurrentProcessId() };
    let a = parse(&std::env::var(VAR).ok()?, pid)?;
    let api = unsafe { &*(a as *const KmdApi) };
    (api.version == VERSION).then_some(api)
}

pub fn owner() -> Option<&'static KmdApi> {
    let a = OWNER.load(Ordering::Acquire);
    if a != 0 {
        return Some(unsafe { &*(a as *const KmdApi) });
    }
    let api = from_env()?;
    OWNER.store(api as *const KmdApi as usize, Ordering::Release);
    Some(api)
}


pub fn publish(api: &'static KmdApi) -> &'static KmdApi {
    if let Some(o) = owner() {
        return o;
    }
    let pid = unsafe { GetCurrentProcessId() };
    let name: Vec<u16> = format!("Local\\tombox-xgskmd-{pid:x}").encode_utf16().chain(Some(0)).collect();
    let m = unsafe { CreateMutexW(core::ptr::null_mut(), 0, name.as_ptr()) };
    if m != 0 {
        unsafe { WaitForSingleObject(m, 0xffff_ffff) };
    }
    let chosen = match from_env() {
        Some(o) => o,
        None => {
            std::env::set_var(VAR, format!("{pid:x}:{:x}", api as *const KmdApi as usize));
            api
        }
    };
    if m != 0 {
        unsafe {
            ReleaseMutex(m);
            CloseHandle(m);
        }
    }
    OWNER.store(chosen as *const KmdApi as usize, Ordering::Release);
    chosen
}

pub fn is_kmd_path(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l == "\\\\.\\xgskmd10" || l == "\\\\.\\xgskmd"
}

unsafe fn wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while n < 64 && *p.add(n) != 0 {
        n += 1;
    }
    if n == 64 {
        return String::new();
    }
    String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
}

static ORIG_CREATE: AtomicUsize = AtomicUsize::new(0);
static ORIG_IOCTL: AtomicUsize = AtomicUsize::new(0);
static ORIG_CLOSE: AtomicUsize = AtomicUsize::new(0);

pub unsafe extern "system" fn create_file_w(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: isize) -> isize {
    if is_kmd_path(&wide(name)) {
        if let Some(api) = owner() {
            return (api.open)();
        }
    }
    let f = ORIG_CREATE.load(Ordering::Acquire);
    if f == 0 {
        SetLastError(127);
        return -1;
    }
    let f: unsafe extern "system" fn(*const u16, u32, u32, *mut c_void, u32, u32, isize) -> isize = core::mem::transmute(f);
    f(name, access, share, sa, disp, flags, tmpl)
}

pub unsafe extern "system" fn device_io_control(h: isize, code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32, ret: *mut u32, ov: *mut c_void) -> i32 {
    if let Some(api) = owner() {
        if (api.is_device)(h) != 0 {
            let mut n = 0u32;
            let ok = (api.ioctl)(h, code, inb, inl, outb, outl, &mut n);
            let e = GetLastError();
            if !ret.is_null() {
                *ret = n;
            }
            SetLastError(e);
            return ok;
        }
    }
    let f = ORIG_IOCTL.load(Ordering::Acquire);
    if f == 0 {
        SetLastError(127);
        return 0;
    }
    let f: unsafe extern "system" fn(isize, u32, *const c_void, u32, *mut c_void, u32, *mut u32, *mut c_void) -> i32 = core::mem::transmute(f);
    f(h, code, inb, inl, outb, outl, ret, ov)
}

pub unsafe extern "system" fn close_handle(h: isize) -> i32 {
    if let Some(api) = owner() {
        if (api.is_device)(h) != 0 {
            return (api.close)(h);
        }
    }
    let f = ORIG_CLOSE.load(Ordering::Acquire);
    if f == 0 {
        return CloseHandle(h);
    }
    let f: unsafe extern "system" fn(isize) -> i32 = core::mem::transmute(f);
    f(h)
}

pub fn targets() -> Vec<(&'static str, usize, &'static AtomicUsize)> {
    if crate::gdk::setting("console_kmd_route").as_deref() != Some("1") {
        return Vec::new();
    }
    vec![
        ("CreateFileW", create_file_w as usize, &ORIG_CREATE),
        ("DeviceIoControl", device_io_control as usize, &ORIG_IOCTL),
        ("CloseHandle", close_handle as usize, &ORIG_CLOSE),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kmd_paths() {
        assert!(is_kmd_path("\\\\.\\XgsKmd10"));
        assert!(is_kmd_path("\\\\.\\xgskmd"));
        assert!(!is_kmd_path("\\\\.\\XgsKmd11"));
        assert!(!is_kmd_path("C:\\XgsKmd10"));
    }

    #[test]
    fn title_route_is_opt_in() {
        std::env::remove_var("TOMBOX_CONSOLE_KMD_ROUTE");
        if crate::cfg("console_kmd_route").is_none() {
            assert!(targets().is_empty());
        }
        std::env::set_var("TOMBOX_CONSOLE_KMD_ROUTE", "1");
        let names: Vec<&str> = targets().iter().map(|t| t.0).collect();
        assert_eq!(names, vec!["CreateFileW", "DeviceIoControl", "CloseHandle"]);
        std::env::remove_var("TOMBOX_CONSOLE_KMD_ROUTE");
    }

    #[test]
    fn owner_value_is_per_process() {
        assert_eq!(parse("1a:ff00", 0x1a), Some(0xff00));
        assert_eq!(parse("1b:ff00", 0x1a), None);
        assert_eq!(parse("1a:0", 0x1a), None);
        assert_eq!(parse("junk", 0x1a), None);
    }
}
