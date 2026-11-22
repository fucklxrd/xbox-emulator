use core::ffi::c_void;
use std::sync::OnceLock;

pub const E_FAIL: i32 = 0x8000_4005u32 as i32;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const S_OK: i32 = 0;

extern "system" {
    fn GetLastError() -> u32;
}

fn hresult_from_win32(e: u32) -> i32 {
    if e as i32 <= 0 { e as i32 } else { ((e & 0xffff) | 0x8007_0000) as i32 }
}

pub fn kmd_handle() -> isize {
    static H: OnceLock<isize> = OnceLock::new();
    *H.get_or_init(|| {
        let h = xgs_kmd_emu::open();
        if h != xgs_kmd_emu::INVALID {
            let mut n = 0u32;
            unsafe { xgs_kmd_emu::device_io_control(h, xgs_kmd_emu::IOCTL_HANDSHAKE, core::ptr::null(), 0, core::ptr::null_mut(), 0, &mut n) };
        }
        h
    })
}

fn ioctl(code: u32, inb: *const c_void, inl: u32) -> Result<u32, u32> {
    let h = kmd_handle();
    if h == xgs_kmd_emu::INVALID {
        return Err(xgs_kmd_emu::ERROR_INVALID_HANDLE);
    }
    let mut n = 0u32;
    if unsafe { xgs_kmd_emu::device_io_control(h, code, inb, inl, core::ptr::null_mut(), 0, &mut n) } {
        Ok(n)
    } else {
        Err(unsafe { GetLastError() })
    }
}

pub fn set_driver_level(v: u32) -> i32 {
    if !(5..=10).contains(&v) || kmd_handle() == xgs_kmd_emu::INVALID {
        return E_INVALIDARG;
    }
    match ioctl(xgs_kmd_emu::IOCTL_DRIVER_LEVEL, (&v as *const u32).cast(), 4) {
        Err(e) => hresult_from_win32(e),
        Ok(0) => S_OK,
        Ok(_) => E_FAIL,
    }
}

pub fn driver_level() -> Option<u32> {
    xgs_kmd_emu::driver_level()
}

fn page_pool_request(va: u64, pages: u32, out: *mut u64) -> i32 {
    let mut req = [0u8; 0x18];
    req[0..8].copy_from_slice(&va.to_le_bytes());
    req[8..12].copy_from_slice(&pages.to_le_bytes());
    req[16..24].copy_from_slice(&(out as u64).to_le_bytes());
    match ioctl(xgs_kmd_emu::IOCTL_PAGE_POOL, req.as_ptr().cast(), 0x18) {
        Ok(_) => S_OK,
        Err(_) => E_FAIL,
    }
}

pub fn register_page_pool(va: u64, pages: u32, out: *mut u64) -> i32 {
    if va == 0 || pages == 0 {
        return E_INVALIDARG;
    }
    page_pool_request(va, pages, out)
}

pub fn unregister_page_pool(handle: u64) -> i32 {
    let mut h = handle;
    page_pool_request(0, 0, &mut h)
}

pub fn page_pool(handle: u64) -> Option<(u64, u32)> {
    xgs_kmd_emu::registered_page_pool(handle)
}

#[cfg(test)]
pub fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    static S: std::sync::Mutex<()> = std::sync::Mutex::new(());
    S.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_level_goes_through_kmd_ioctl() {
        let _g = test_serial();
        for v in [0, 4, 11, 0xffff_ffff] {
            assert_eq!(set_driver_level(v), E_INVALIDARG);
        }
        for v in 5..=10 {
            assert_eq!(set_driver_level(v), S_OK);
            assert_eq!(driver_level(), Some(v));
        }
        assert_eq!(set_driver_level(4), E_INVALIDARG);
        assert_eq!(driver_level(), Some(10));
    }

    #[test]
    fn page_pool_handles_come_from_the_kernel() {
        let _g = test_serial();
        let mem = vec![0u8; 0x3_0000];
        let va = ((mem.as_ptr() as u64) + 0xffff) & !0xffff;
        let (mut a, mut b) = (0u64, 0u64);
        assert_eq!(register_page_pool(va, 1, &mut a), S_OK);
        assert_eq!(register_page_pool(va, 1, &mut b), S_OK);
        assert_ne!(a, b);
        assert_eq!(page_pool(a), Some((va, 1)));
        assert_eq!(register_page_pool(0, 1, &mut a), E_INVALIDARG);
        assert_eq!(register_page_pool(va, 0, &mut a), E_INVALIDARG);
        assert_eq!(register_page_pool(va, 1, core::ptr::null_mut()), E_FAIL);
        assert_eq!(unregister_page_pool(a), S_OK);
        assert_eq!(page_pool(a), None);
        assert_eq!(unregister_page_pool(a), E_FAIL);
        assert_eq!(unregister_page_pool(0), E_FAIL);
        assert_eq!(unregister_page_pool(b), S_OK);
        let mut c = 0x55u64;
        assert_eq!(register_page_pool(0x1_0000, 4, &mut c), E_FAIL);
        assert_eq!(c, 0);
        drop(mem);
    }
}
