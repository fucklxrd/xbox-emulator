use core::ffi::c_void;
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicUsize, Ordering};

pub const INVALID: isize = -1;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_FAIL: i32 = 0x8000_4005u32 as i32;
pub const STATUS_UNSUCCESSFUL: i32 = 0xc000_0001u32 as i32;
pub const KMD_PATH: &str = "\\\\.\\XgsKmd";
pub const IOCTL_PROBE: u32 = 0x7900_0000;
pub const IOCTL_HANDSHAKE: u32 = 0x7900_c190;
pub const IOCTL_RELEASE: u32 = 0x7900_c194;

extern "system" {
    fn GetLastError() -> u32;
    fn GetModuleHandleW(n: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

type DirectCall = unsafe extern "system" fn(u32, *const c_void, u32, *mut c_void, *mut u32) -> i32;

static HANDLE: AtomicIsize = AtomicIsize::new(INVALID);
static DIRECT: AtomicBool = AtomicBool::new(false);
static LOCK: AtomicI32 = AtomicI32::new(0);
static DIRECT_FN: AtomicUsize = AtomicUsize::new(0);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

pub fn hresult_from_win32(e: u32) -> i32 {
    if (e as i32) <= 0 {
        e as i32
    } else {
        ((e & 0xffff) | 0x8007_0000) as i32
    }
}

fn last_error_hr() -> i32 {
    hresult_from_win32(unsafe { GetLastError() })
}

fn direct_fn() -> Option<DirectCall> {
    let mut v = DIRECT_FN.load(Ordering::Acquire);
    if v == 0 {
        let m = unsafe { GetModuleHandleW(wide("ntdll.dll").as_ptr()) };
        let f = if m.is_null() { null_mut() } else { unsafe { GetProcAddress(m, b"NtDirectGraphicsCall\0".as_ptr()) } };
        v = if f.is_null() { 1 } else { f as usize };
        DIRECT_FN.store(v, Ordering::Release);
    }
    if v == 1 {
        None
    } else {
        Some(unsafe { core::mem::transmute::<usize, DirectCall>(v) })
    }
}

unsafe fn direct(code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: *mut u32) -> Option<i32> {
    direct_fn().map(|f| f(code, inb, inl, outb, outl))
}

fn lock() {
    loop {
        if LOCK.fetch_add(1, Ordering::AcqRel) + 1 == 1 {
            return;
        }
        LOCK.fetch_sub(1, Ordering::AcqRel);
        core::hint::spin_loop();
    }
}

fn unlock() {
    LOCK.fetch_sub(1, Ordering::AcqRel);
}

pub fn cached() -> isize {
    HANDLE.load(Ordering::Acquire)
}

#[cfg(test)]
pub fn direct_enabled() -> bool {
    DIRECT.load(Ordering::Acquire)
}

pub unsafe fn create() -> isize {
    xgs_kmd_emu::open()
}

pub unsafe fn close_handle(h: isize) {
    xgs_kmd_emu::close(h);
}

pub unsafe fn ioctl(h: isize, code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32, ret: &mut u32) -> bool {
    xgs_kmd_emu::device_io_control(h, code, inb, inl, outb, outl, ret)
}

pub fn last_error() -> i32 {
    last_error_hr()
}

pub unsafe fn open(out: *mut isize, handshake: bool) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let h = HANDLE.load(Ordering::Acquire);
    if h != INVALID {
        *out = h;
        return 0;
    }
    lock();
    let mut hr = 0;
    let h = HANDLE.load(Ordering::Acquire);
    if h != INVALID {
        *out = h;
    } else {
        DIRECT.store(direct(IOCTL_PROBE, null(), 0, null_mut(), null_mut()) == Some(0), Ordering::Release);
        let f = create();
        if f == INVALID {
            hr = last_error_hr();
        } else {
            let mut keep = true;
            if handshake {
                let mut ret = 0u32;
                let st = if !ioctl(f, IOCTL_HANDSHAKE, null(), 0, null_mut(), 0, &mut ret) {
                    last_error_hr()
                } else if ret != 0 {
                    E_FAIL
                } else {
                    0
                };
                if st < 0 {
                    close_handle(f);
                    hr = st;
                    keep = false;
                }
            }
            if keep {
                HANDLE.store(f, Ordering::Release);
                *out = f;
            }
        }
    }
    unlock();
    hr
}

pub unsafe fn release(h: isize) -> i32 {
    lock();
    if HANDLE.load(Ordering::Acquire) != INVALID {
        HANDLE.store(INVALID, Ordering::Release);
        if h != INVALID {
            let mut ret = 0u32;
            if !ioctl(h, IOCTL_RELEASE, null(), 0, null_mut(), 0, &mut ret) {
                GetLastError();
            }
            close_handle(h);
        }
    }
    unlock();
    0
}

unsafe fn device_status(code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32) -> i32 {
    let mut ret = 0u32;
    let hr = if ioctl(HANDLE.load(Ordering::Acquire), code, inb, inl, outb, outl, &mut ret) { 0 } else { last_error_hr() };
    if hr != 0 {
        STATUS_UNSUCCESSFUL
    } else {
        0
    }
}

unsafe fn direct_status(code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32) -> Option<i32> {
    if !DIRECT.load(Ordering::Acquire) {
        return None;
    }
    let mut size = outl;
    let sp = if outb.is_null() { null_mut() } else { &mut size as *mut u32 };
    direct(code, inb, inl, outb, sp)
}

pub unsafe fn call_cached(code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32) -> i32 {
    if let Some(st) = direct_status(code, inb, inl, outb, outl) {
        return st;
    }
    device_status(code, inb, inl, outb, outl)
}

pub unsafe fn call_device(code: u32, inb: *const c_void, inl: u32) -> i32 {
    device_status(code, inb, inl, null_mut(), 0)
}

pub unsafe fn call_opened(code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32) -> i32 {
    if let Some(st) = direct_status(code, inb, inl, outb, outl) {
        return st;
    }
    let mut h = INVALID;
    if open(&mut h, false) < 0 {
        return STATUS_UNSUCCESSFUL;
    }
    device_status(code, inb, inl, outb, outl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hresult_mapping_matches_console() {
        assert_eq!(hresult_from_win32(0), 0);
        assert_eq!(hresult_from_win32(2), 0x8007_0002u32 as i32);
        assert_eq!(hresult_from_win32(0x8000_4005), 0x8000_4005u32 as i32);
        assert_eq!(hresult_from_win32(0x1_0005), 0x8007_0005u32 as i32);
    }

    #[test]
    fn open_uses_the_emulated_device() {
        let _g = test_serial();
        assert_eq!(unsafe { open(null_mut(), false) }, E_INVALIDARG);
        let mut h = INVALID;
        assert_eq!(unsafe { open(&mut h, true) }, 0);
        assert!(xgs_kmd_emu::is_device(h));
        assert_eq!(cached(), h);
        assert!(!direct_enabled());
        let mut out = [0u8; 0x10];
        let mut ret = 0u32;
        assert!(unsafe { ioctl(h, 0x7900_c1fc, null(), 0, out.as_mut_ptr().cast(), 0x10, &mut ret) });
        assert_eq!(ret, 0x10);
        assert_eq!(out, xgs_kmd_emu::gpu_reply(xgs_kmd_emu::device_type()));
        let buf = [0u8; 0x40];
        assert_eq!(unsafe { call_cached(0x7900_c20b, buf.as_ptr().cast(), 0x20, null_mut(), 0) }, STATUS_UNSUCCESSFUL);
        assert_eq!(unsafe { call_opened(0x7900_c21b, buf.as_ptr().cast(), 0x18, null_mut(), 0) }, STATUS_UNSUCCESSFUL);
        assert_eq!(unsafe { call_device(0x7900_c273, buf.as_ptr().cast(), 0x18) }, STATUS_UNSUCCESSFUL);
        assert_eq!(unsafe { release(h) }, 0);
        assert_eq!(cached(), INVALID);
        assert!(!xgs_kmd_emu::is_device(h));
        assert_eq!(unsafe { release(INVALID) }, 0);
    }
}

#[cfg(test)]
pub fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}
