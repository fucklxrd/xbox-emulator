use core::ffi::c_void;
use std::sync::Mutex;

extern "system" {
    fn GetModuleHandleW(n: *const u16) -> *mut c_void;
    fn LoadLibraryW(n: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

pub const E_MOD_NOT_FOUND: i32 = 0x8007_007eu32 as i32;
pub const E_PROC_NOT_FOUND: i32 = 0x8007_007fu32 as i32;
pub const MODULE: &str = "xfp_gc.dll";

static MODULE_OVERRIDE: Mutex<Option<String>> = Mutex::new(None);

pub fn set_module(path: Option<&str>) {
    *MODULE_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = path.map(str::to_string);
}

fn module_name() -> String {
    MODULE_OVERRIDE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| MODULE.to_string())
}

fn proc(name: &[u8]) -> Result<*mut c_void, i32> {
    let n: Vec<u16> = module_name().encode_utf16().chain(Some(0)).collect();
    unsafe {
        let mut h = GetModuleHandleW(n.as_ptr());
        if h.is_null() {
            h = LoadLibraryW(n.as_ptr());
        }
        if h.is_null() {
            return Err(E_MOD_NOT_FOUND);
        }
        let p = GetProcAddress(h, name.as_ptr());
        if p.is_null() {
            return Err(E_PROC_NOT_FOUND);
        }
        Ok(p)
    }
}

pub fn try_connect() -> i32 {
    match proc(b"XfpTryConnect\0") {
        Ok(p) => unsafe { core::mem::transmute::<*mut c_void, extern "system" fn() -> i32>(p)() },
        Err(hr) => hr,
    }
}

pub fn get_key_state(keys: *mut u32) -> i32 {
    match proc(b"XfpGetKeyState\0") {
        Ok(p) => unsafe { core::mem::transmute::<*mut c_void, unsafe extern "system" fn(*mut u32) -> i32>(p)(keys) },
        Err(hr) => hr,
    }
}

pub fn get_led_state(led: *mut u16) -> i32 {
    match proc(b"XfpGetLEDState\0") {
        Ok(p) => unsafe { core::mem::transmute::<*mut c_void, unsafe extern "system" fn(*mut u16) -> i32>(p)(led) },
        Err(hr) => hr,
    }
}

pub fn set_led_state(led: u16) -> i32 {
    match proc(b"XfpSetLEDState\0") {
        Ok(p) => unsafe { core::mem::transmute::<*mut c_void, extern "system" fn(u16) -> i32>(p)(led) },
        Err(hr) => hr,
    }
}

pub fn set_display_buffer(buffer: *const u8, format: u8) -> i32 {
    match proc(b"XfpSetDisplayBuffer\0") {
        Ok(p) => unsafe { core::mem::transmute::<*mut c_void, unsafe extern "system" fn(*const u8, u8) -> i32>(p)(buffer, format) },
        Err(hr) => hr,
    }
}

pub const LED_BITS: [(u32, u16); 5] = [(0x1, 0x8000), (0x2, 0x2000), (0x4, 0x0002), (0x8, 0x0800), (0x10, 0x0200)];

pub fn lights_to_led(lights: u32) -> u16 {
    LED_BITS.iter().filter(|(l, _)| lights & l != 0).fold(0, |a, (_, b)| a | b)
}

pub fn led_to_lights(led: u16) -> u32 {
    LED_BITS.iter().filter(|(_, b)| led & b != 0).fold(0, |a, (l, _)| a | l)
}
