use core::ffi::c_void;
use std::sync::OnceLock;

use console_image::ConsoleDll;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

pub type Hstring = *mut c_void;

pub const S_OK: i32 = 0;
pub const S_FALSE: i32 = 1;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
pub const CLASS_E_CLASSNOTAVAILABLE: i32 = 0x8004_0111u32 as i32;

pub const CLSID_SIPC_PROXY_CONTROL: Guid = Guid { data1: 0xF5715449, data2: 0x10F6, data3: 0x4225, data4: [0xA7, 0xC6, 0xD5, 0x84, 0xBD, 0x3C, 0x7F, 0x5B] };
pub const CLSID_SECOND: Guid = Guid { data1: 0x7FA7A8FD, data2: 0xA259, data3: 0x4A79, data4: [0xBB, 0x7E, 0x09, 0x4C, 0x18, 0xCD, 0x07, 0x23] };
pub const COM_CLASSES: [Guid; 2] = [CLSID_SIPC_PROXY_CONTROL, CLSID_SECOND];
pub const ACTIVATION_ARG_NAME: &str = "activatibleClassId";

pub static CONSOLE: ConsoleDll = ConsoleDll::new("gamecoreclient.dll", "TOMBOX_GAMECORECLIENT_CONSOLE", "gamecoreclient_console.dll", "DllGetClassObject", &[]);
static ATTACHED: OnceLock<Vec<String>> = OnceLock::new();

fn own() -> usize {
    DllCanUnloadNow as *const () as usize
}

pub fn console_module() -> usize {
    let m = CONSOLE.module(own());
    if m != 0 {
        ATTACHED.get_or_init(|| unsafe { console_sandbox::attach(m) });
    }
    m
}

pub fn sandbox_hits() -> Vec<String> {
    ATTACHED.get().cloned().unwrap_or_default()
}

fn loaded_module() -> usize {
    match (CONSOLE.handle.get(), ATTACHED.get()) {
        (Some(&m), Some(_)) => m,
        _ => 0,
    }
}

type ClassObjectFn = unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32;
type CanUnloadFn = unsafe extern "system" fn() -> i32;

#[link(name = "runtimeobject")]
extern "system" {
    fn WindowsIsStringEmpty(s: Hstring) -> i32;
    fn WindowsStringHasEmbeddedNull(s: Hstring, has: *mut i32) -> i32;
    fn RoOriginateError(hr: i32, message: Hstring) -> i32;
    fn RoOriginateErrorW(hr: i32, cch: u32, message: *const u16) -> i32;
}

fn invalid_class_id() -> i32 {
    let name: Vec<u16> = ACTIVATION_ARG_NAME.encode_utf16().chain(Some(0)).collect();
    unsafe { RoOriginateErrorW(E_INVALIDARG, (name.len() - 1) as u32, name.as_ptr()) };
    E_INVALIDARG
}

#[no_mangle]
pub unsafe extern "system" fn DllGetActivationFactory(id: Hstring, factory: *mut *mut c_void) -> i32 {
    *factory = core::ptr::null_mut();
    if WindowsIsStringEmpty(id) != 0 {
        return invalid_class_id();
    }
    let mut has_null = 0;
    if WindowsStringHasEmbeddedNull(id, &mut has_null) < 0 || has_null == 1 {
        return invalid_class_id();
    }

    RoOriginateError(CLASS_E_CLASSNOTAVAILABLE, id);
    CLASS_E_CLASSNOTAVAILABLE
}

#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const Guid, iid: *const Guid, ppv: *mut *mut c_void) -> i32 {
    match console_image::symbol(console_module(), "DllGetClassObject") {
        Some(p) => {
            let f: ClassObjectFn = core::mem::transmute(p);
            f(clsid, iid, ppv)
        }
        None => {
            if !ppv.is_null() {
                *ppv = core::ptr::null_mut();
            }
            CONSOLE.missing("DllGetClassObject", CLASS_E_CLASSNOTAVAILABLE as u32 as usize);
            CLASS_E_CLASSNOTAVAILABLE
        }
    }
}

#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> i32 {
    match console_image::symbol(loaded_module(), "DllCanUnloadNow") {
        Some(p) => unsafe {
            let f: CanUnloadFn = core::mem::transmute(p);
            f()
        },
        None => S_OK,
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
