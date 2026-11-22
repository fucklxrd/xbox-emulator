use core::ffi::c_void;
use std::ffi::CString;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::layout::LAYOUT_BYTES;

pub static SERIAL: Mutex<()> = Mutex::new(());

const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x0100;
const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0800;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
}

pub struct Oracle {
    pub h: usize,
}

pub fn oracle() -> Option<Oracle> {
    // The reference tests use RVAs from the matching Series X|S console DLL.
    // Supply that legally obtained DLL and its dependencies from a local directory.
    let path = PathBuf::from(std::env::var_os("TOMBOX_XG_XS_ORACLE")?);
    assert!(path.is_absolute(), "TOMBOX_XG_XS_ORACLE must be an absolute DLL path");
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let module = unsafe {
        LoadLibraryExW(
            name.as_ptr(),
            core::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    assert!(!module.is_null(), "could not load {}: {}", path.display(), std::io::Error::last_os_error());
    Some(Oracle { h: module as usize })
}

impl Oracle {
    pub fn f(&self, name: &str) -> usize {
        let name = CString::new(name).unwrap();
        let proc = unsafe { GetProcAddress(self.h as *mut c_void, name.as_ptr().cast()) };
        assert!(!proc.is_null(), "missing export {}", name.to_string_lossy());
        proc as usize
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.h as *mut c_void) };
    }
}

pub type Create = unsafe extern "system" fn(*const u8, *mut *mut c_void) -> i32;

pub struct Obj(pub *mut c_void);

pub unsafe fn vt(object: *mut c_void, index: usize) -> *const c_void {
    *(*(object as *const *const *const c_void)).add(index)
}

impl Obj {
    pub unsafe fn layout(&self) -> Vec<u8> {
        let mut bytes = vec![0; LAYOUT_BYTES];
        let f: unsafe extern "system" fn(*mut c_void, *mut u8) -> i32 = core::mem::transmute(vt(self.0, 2));
        let hr = f(self.0, bytes.as_mut_ptr());
        assert!(hr >= 0, "GetResourceLayout failed: {hr:#010x}");
        bytes
    }

    pub unsafe fn size(&self) -> u64 {
        let f: unsafe extern "system" fn(*mut c_void) -> u64 = core::mem::transmute(vt(self.0, 3));
        f(self.0)
    }

    pub unsafe fn align(&self) -> u64 {
        let f: unsafe extern "system" fn(*mut c_void) -> u64 = core::mem::transmute(vt(self.0, 4));
        f(self.0)
    }

    pub unsafe fn mip_offset(&self, mip: u32) -> u64 {
        let f: unsafe extern "system" fn(*mut c_void, u32, u32) -> u64 = core::mem::transmute(vt(self.0, 5));
        f(self.0, 0, mip)
    }

    pub unsafe fn elem(&self, mip: u32, x: u64, y: u32, z: u32) -> u64 {
        type Offset = unsafe extern "system" fn(*mut c_void, u32, u32, u64, u32, u32, u32, *mut u32) -> u64;
        let f: Offset = core::mem::transmute(vt(self.0, 6));
        let mut bit = 0;
        f(self.0, 0, mip, x, y, z, 0, &mut bit)
    }
}

impl Drop for Obj {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vt(self.0, 1));
                release(self.0);
            }
        }
    }
}

pub fn first_diff(a: &[u8], b: &[u8]) -> Option<(usize, u8, u8)> {
    assert_eq!(a.len(), b.len());
    a.iter().copied().zip(b.iter().copied()).enumerate()
        .find_map(|(index, (a, b))| (a != b).then_some((index, a, b)))
}

pub fn field(offset: usize) -> String {
    format!("byte +{offset:#x}")
}
