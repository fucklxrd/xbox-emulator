use core::ffi::c_void;
pub use xtfcommon::sys::{WaitForSingleObject, E_INVALIDARG, E_OUTOFMEMORY, E_UNEXPECTED, HANDLE, HRESULT, INFINITE, S_OK};

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl GUID {
    pub const fn new(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        GUID { data1, data2, data3, data4 }
    }
}

pub type BSTR = *mut u16;

pub const E_NOINTERFACE: HRESULT = 0x8000_4002u32 as i32;
pub const E_ACCESSDENIED: HRESULT = 0x8007_0005u32 as i32;
pub const E_PENDING: HRESULT = 0x8000_000Au32 as i32;
pub const E_JSON_NOT_FOUND: HRESULT = 0x8375_0009u32 as i32;
pub const HR_EXCLUSIVE_CONNECTION_HELD: HRESULT = 0x8ABD_0004u32 as i32;

pub const IID_IUNKNOWN: GUID = GUID::new(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
pub const IID_PIX_SERVER: GUID = GUID::new(0x2b25ecdb, 0xb230, 0x46b4, [0xb3, 0x9b, 0xad, 0xb3, 0x7d, 0xec, 0x0b, 0x16]);
pub const IID_EXTENSION_METADATA: GUID = GUID::new(0xe2b6c063, 0x7d82, 0x414a, [0xbe, 0x86, 0x67, 0xed, 0x1b, 0x44, 0x77, 0x97]);
pub const IID_OPERATION_HANDLER: GUID = GUID::new(0x07772e34, 0x6294, 0x4518, [0x9b, 0x11, 0x30, 0x81, 0x5f, 0xc8, 0x63, 0xe0]);
pub const IID_MESSAGE: GUID = GUID::new(0x6965e04a, 0x2829, 0x41a8, [0xb1, 0x94, 0x48, 0x73, 0x15, 0x07, 0x5e, 0x7a]);
pub const PIX_EXTENSION_ID: GUID = GUID::new(0x9bd151a3, 0x47d0, 0x41b9, [0xbe, 0x65, 0xf1, 0x13, 0x74, 0x68, 0xc2, 0x80]);

#[cfg(not(test))]
#[link(name = "xbtp", kind = "raw-dylib")]
extern "C" {
    pub fn XbtpCreateExtensionMetadata(iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    pub fn XbtpCreateMessage(iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    pub fn XbtpCreateJsonObject(out: *mut *mut c_void) -> HRESULT;
    pub fn XbtpCreateJsonArray(out: *mut *mut c_void) -> HRESULT;
    pub fn XbtpGetJsonObjectFromUtf8(utf8: *const u8, out: *mut *mut c_void) -> HRESULT;
    pub fn XbtpOpenSession2(address: *const u16, server_port: u32, options: *const crate::client::OpenSessionOptions, extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
}

#[cfg(not(test))]
#[link(name = "xpal", kind = "raw-dylib")]
extern "system" {
    pub fn XGetSystemType() -> u16;
}

#[cfg(test)]
pub use test_link::*;

#[cfg(test)]
mod test_link {
    use super::{GUID, HRESULT};
    use core::ffi::c_void;

    pub unsafe fn XbtpCreateExtensionMetadata(iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        xbtp::xbtp2_ext::XbtpCreateExtensionMetadata(iid as *const _, out)
    }

    pub unsafe fn XbtpCreateMessage(iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        xbtp::xbtp2_ext::XbtpCreateMessage(iid as *const _, out)
    }

    pub unsafe fn XbtpCreateJsonObject(out: *mut *mut c_void) -> HRESULT {
        xbtp::xbtp2_json::XbtpCreateJsonObject(out)
    }

    pub unsafe fn XbtpCreateJsonArray(out: *mut *mut c_void) -> HRESULT {
        xbtp::xbtp2_json::XbtpCreateJsonArray(out)
    }

    pub unsafe fn XbtpGetJsonObjectFromUtf8(utf8: *const u8, out: *mut *mut c_void) -> HRESULT {
        xbtp::xbtp2_json::XbtpGetJsonObjectFromUtf8(utf8, out)
    }

    pub unsafe fn XbtpOpenSession2(address: *const u16, server_port: u32, options: *const crate::client::OpenSessionOptions, extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        xbtp::api::XbtpOpenSession2(address, server_port, options as *const _, extension as *const _, iid as *const _, out)
    }

    pub unsafe fn XGetSystemType() -> u16 {
        xpal::XGetSystemType()
    }
}

#[link(name = "oleaut32", kind = "raw-dylib")]
extern "system" {
    pub fn SysAllocString(s: *const u16) -> BSTR;
    pub fn SysFreeString(s: BSTR);
}

#[link(name = "kernel32", kind = "raw-dylib")]
extern "system" {
    pub fn GetTickCount64() -> u64;
    pub fn QueryPerformanceFrequency(out: *mut i64) -> i32;
}

pub type QiFn = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
pub type RefFn = unsafe extern "system" fn(*mut c_void) -> u32;

pub unsafe fn slot(p: *mut c_void, i: usize) -> usize {
    *(*(p as *const *const usize)).add(i)
}

pub unsafe fn com_qi(p: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    let f: QiFn = core::mem::transmute(slot(p, 0));
    f(p, iid, out)
}

pub unsafe fn com_add_ref(p: *mut c_void) -> u32 {
    let f: RefFn = core::mem::transmute(slot(p, 1));
    f(p)
}

pub unsafe fn com_release(p: *mut c_void) -> u32 {
    let f: RefFn = core::mem::transmute(slot(p, 2));
    f(p)
}

pub struct ComPtr(pub *mut c_void);

impl ComPtr {
    pub fn null() -> Self {
        ComPtr(core::ptr::null_mut())
    }

    pub fn out(&mut self) -> *mut *mut c_void {
        self.reset();
        &mut self.0
    }

    pub fn reset(&mut self) {
        let p = core::mem::replace(&mut self.0, core::ptr::null_mut());
        if !p.is_null() {
            unsafe { com_release(p) };
        }
    }
}

impl Drop for ComPtr {
    fn drop(&mut self) {
        self.reset();
    }
}

pub struct Bstr(pub BSTR);

impl Bstr {
    pub fn null() -> Self {
        Bstr(core::ptr::null_mut())
    }

    pub fn reset(&mut self) {
        let p = core::mem::replace(&mut self.0, core::ptr::null_mut());
        if !p.is_null() {
            unsafe { SysFreeString(p) };
        }
    }
}

impl Drop for Bstr {
    fn drop(&mut self) {
        self.reset();
    }
}

pub fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}
