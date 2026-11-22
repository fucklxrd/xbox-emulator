mod window;
mod xfp;
#[cfg(test)]
mod supported_tests;

use compat_common::{record, S_OK};
use core::ffi::c_void;
use std::sync::Mutex;

const M: &str = "XFrontPanelDisplay";
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_POINTER: i32 = 0x8000_4003u32 as i32;
pub const PANEL_HEIGHT: u32 = 0x40;
pub const PANEL_WIDTH: u32 = 0x100;
pub const PANEL_BYTES: usize = (PANEL_WIDTH * PANEL_HEIGHT) as usize;
pub const DXGI_FORMAT_R8_UNORM: u32 = 0x3d;
pub const LIGHT_MASK: u32 = 0x1f;
pub const BUTTON_MASK: u32 = 0x3ff;

pub struct Panel {
    pub pixels: Vec<u8>,
    pub lights: u32,
    pub buttons: u32,
    pub presents: u64,
}

pub static PANEL: Mutex<Panel> = Mutex::new(Panel { pixels: Vec::new(), lights: 0, buttons: 0, presents: 0 });

pub fn panel() -> std::sync::MutexGuard<'static, Panel> {
    let mut g = PANEL.lock().unwrap_or_else(|e| e.into_inner());
    if g.pixels.len() != PANEL_BYTES {
        g.pixels = vec![0; PANEL_BYTES];
    }
    g
}

fn window_enabled() -> bool {
    !cfg!(test)
        && std::env::var("TOMBOX_FRONTPANEL")
            .ok()
            .or_else(|| compat_common::cfg("TOMBOX_FRONTPANEL"))
            .map_or(true, |v| v != "0" && !v.eq_ignore_ascii_case("off"))
}

#[no_mangle]
pub extern "system" fn XFrontPanelIsSupported() -> bool {
    let hr = xfp::try_connect();
    if hr < 0 {
        record(M, "XFrontPanelIsSupported", &format!("XFP_GC XfpTryConnect failed {hr:#x}: no front panel"));
    }
    hr >= 0
}

#[no_mangle]
pub extern "system" fn XFrontPanelGetScreenDimensions(height: *mut u32, width: *mut u32) -> i32 {
    if height.is_null() || width.is_null() {
        return E_INVALIDARG;
    }
    unsafe {
        *height = PANEL_HEIGHT;
        *width = PANEL_WIDTH;
    }
    S_OK
}

#[no_mangle]
pub extern "system" fn XFrontPanelGetScreenPixelFormat(format: *mut u32) -> i32 {
    if format.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *format = DXGI_FORMAT_R8_UNORM };
    S_OK
}

#[no_mangle]
pub extern "system" fn XFrontPanelPresentBuffer(size: u32, buffer: *const c_void) -> i32 {
    if size as usize != PANEL_BYTES {
        return E_INVALIDARG;
    }
    if buffer.is_null() {
        return E_POINTER;
    }
    if xfp::try_connect() < 0 {
        return xfp::try_connect();
    }
    let hr = xfp::set_display_buffer(buffer as *const u8, 8);
    if hr < 0 {
        return hr;
    }
    let src = unsafe { core::slice::from_raw_parts(buffer as *const u8, PANEL_BYTES) };
    let n = {
        let mut p = panel();
        p.pixels.copy_from_slice(src);
        p.presents += 1;
        p.presents
    };
    if n == 1 || n % 600 == 0 {
        let lit = src.iter().filter(|&&v| v != 0).count();
        record(M, "XFrontPanelPresentBuffer", &format!("frame {n}: {lit} lit pixels of {PANEL_BYTES}"));
    }
    if window_enabled() {
        window::show();
        window::invalidate();
    }
    hr
}

#[no_mangle]
pub extern "system" fn XFrontPanelGetLightStates(lights: *mut u32) -> i32 {
    if lights.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *lights = 0 };
    if xfp::try_connect() < 0 {
        return xfp::try_connect();
    }
    let mut led = 0u16;
    if xfp::get_led_state(&mut led) < 0 {
        return xfp::get_led_state(&mut led);
    }
    let v = xfp::led_to_lights(led);
    unsafe { *lights = v };
    panel().lights = v;
    S_OK
}

#[no_mangle]
pub extern "system" fn XFrontPanelSetLightStates(lights: u32) -> i32 {
    let led = xfp::lights_to_led(lights);
    if xfp::try_connect() < 0 {
        return xfp::try_connect();
    }
    let hr = xfp::set_led_state(led);
    if hr < 0 {
        return hr;
    }
    let old = {
        let mut p = panel();
        let old = p.lights;
        p.lights = lights & LIGHT_MASK;
        old
    };
    if old != lights & LIGHT_MASK {
        record(M, "XFrontPanelSetLightStates", &format!("{old:#04x} -> {:#04x}", lights & LIGHT_MASK));
        if window_enabled() {
            window::invalidate();
        }
    }
    hr
}

#[no_mangle]
pub extern "system" fn XFrontPanelGetButtonStates(buttons: *mut u32) -> i32 {
    if xfp::try_connect() < 0 {
        return xfp::try_connect();
    }
    let hr = xfp::get_key_state(buttons);
    if hr >= 0 && !buttons.is_null() {
        panel().buttons = unsafe { *buttons } & BUTTON_MASK;
    }
    hr
}

pub fn set_buttons(b: u32) {
    let old = {
        let mut p = panel();
        let old = p.buttons;
        p.buttons = b & BUTTON_MASK;
        old
    };
    if old != b & BUTTON_MASK {
        record(M, "buttons", &format!("{old:#05x} -> {:#05x}", b & BUTTON_MASK));
    }
}

#[no_mangle]
pub extern "system" fn XFrontPanelSetDisplayName(name: *const u16) -> i32 {
    if name.is_null() {
        return E_INVALIDARG;
    }
    let mut n = 0;
    while n < 256 && unsafe { *name.add(n) } != 0 {
        n += 1;
    }
    let s = String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(name, n) });
    record(M, "XFrontPanelSetDisplayName", &s);
    if window_enabled() {
        window::set_title(&s);
    }
    S_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORTS: &str = include_str!("../exports.txt");

    #[test]
    fn export_table_matches_reference_dll() {
        xone_exports::check_crate(env!("CARGO_MANIFEST_DIR"), "xfrontpaneldisplay.dll", EXPORTS);
    }

    #[test]
    fn built_dll_has_reference_ordinals() {
        xone_exports::check_built(env!("CARGO_MANIFEST_DIR"), "xfrontpaneldisplay.dll", EXPORTS);
    }

    #[test]
    fn panel_geometry_and_format_match_console() {
        let (mut h, mut w) = (0u32, 0u32);
        assert_eq!(XFrontPanelGetScreenDimensions(&mut h, &mut w), S_OK);
        assert_eq!((h, w), (64, 256));
        assert_eq!(XFrontPanelGetScreenDimensions(core::ptr::null_mut(), &mut w), E_INVALIDARG);
        let mut f = 0u32;
        assert_eq!(XFrontPanelGetScreenPixelFormat(&mut f), S_OK);
        assert_eq!(f, 0x3d);
        assert_eq!(XFrontPanelGetScreenPixelFormat(core::ptr::null_mut()), E_INVALIDARG);
    }

    #[test]
    fn display_name_requires_pointer() {
        assert_eq!(XFrontPanelSetDisplayName(core::ptr::null()), E_INVALIDARG);
        let n: Vec<u16> = "Terraria".encode_utf16().chain(std::iter::once(0)).collect();
        assert_eq!(XFrontPanelSetDisplayName(n.as_ptr()), S_OK);
    }

    #[test]
    fn key_map_covers_all_buttons() {
        let all = window::KEYS.iter().fold(0, |a, k| a | k.1);
        assert_eq!(all, BUTTON_MASK);
    }
}
