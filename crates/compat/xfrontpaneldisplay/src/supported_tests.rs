use super::*;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn no_module() -> std::sync::MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    xfp::set_module(Some("tombox_no_such_xfp_gc.dll"));
    g
}

#[test]
fn led_word_matches_console_bit_layout() {
    assert_eq!(xfp::lights_to_led(0x1), 0x8000);
    assert_eq!(xfp::lights_to_led(0x2), 0x2000);
    assert_eq!(xfp::lights_to_led(0x4), 0x0002);
    assert_eq!(xfp::lights_to_led(0x8), 0x0800);
    assert_eq!(xfp::lights_to_led(0x10), 0x0200);
    assert_eq!(xfp::lights_to_led(0xffff_ffe0), 0);
    assert_eq!(xfp::led_to_lights(0xaa02), 0x1f);
    assert_eq!(xfp::led_to_lights(0x55fd), 0);
    for l in 0..0x20 {
        assert_eq!(xfp::led_to_lights(xfp::lights_to_led(l)), l);
    }
}

#[test]
fn no_xfp_module_reports_connect_failure_everywhere() {
    let _g = no_module();
    assert!(!XFrontPanelIsSupported());
    assert_eq!(XFrontPanelGetButtonStates(core::ptr::null_mut()), xfp::E_MOD_NOT_FOUND);
    let mut b = 0x55u32;
    assert_eq!(XFrontPanelGetButtonStates(&mut b), xfp::E_MOD_NOT_FOUND);
    assert_eq!(b, 0x55);
    assert_eq!(XFrontPanelGetLightStates(core::ptr::null_mut()), E_INVALIDARG);
    let mut v = 7u32;
    assert_eq!(XFrontPanelGetLightStates(&mut v), xfp::E_MOD_NOT_FOUND);
    assert_eq!(v, 0);
    let before = panel().lights;
    assert_eq!(XFrontPanelSetLightStates(0x1f), xfp::E_MOD_NOT_FOUND);
    assert_eq!(panel().lights, before);
    let buf = vec![9u8; PANEL_BYTES];
    assert_eq!(XFrontPanelPresentBuffer(0x3fff, buf.as_ptr() as *const c_void), E_INVALIDARG);
    assert_eq!(XFrontPanelPresentBuffer(0x4000, core::ptr::null()), E_POINTER);
    let presents = panel().presents;
    assert_eq!(XFrontPanelPresentBuffer(0x4000, buf.as_ptr() as *const c_void), xfp::E_MOD_NOT_FOUND);
    assert_eq!(panel().presents, presents);
    xfp::set_module(None);
}

#[test]
fn real_xfp_gc_without_panel_pipe_returns_pipe_open_failure() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(dll) = xone_exports::built_dll(env!("CARGO_MANIFEST_DIR"), "xfp_gc.dll") else {
        eprintln!("xfp_gc.dll not built; skipped");
        return;
    };
    std::env::set_var("TOMBOX_XFP_PIPE", format!(r"\\.\pipe\tombox-no-xfp-{}", std::process::id()));
    xfp::set_module(Some(dll.to_str().unwrap()));
    let not_found = 0x8007_0002u32 as i32;
    assert!(!XFrontPanelIsSupported());
    let mut b = 0u32;
    assert_eq!(XFrontPanelGetButtonStates(&mut b), not_found);
    let mut v = 3u32;
    assert_eq!(XFrontPanelGetLightStates(&mut v), not_found);
    assert_eq!(v, 0);
    assert_eq!(XFrontPanelSetLightStates(0x3), not_found);
    let buf = vec![0u8; PANEL_BYTES];
    assert_eq!(XFrontPanelPresentBuffer(0x4000, buf.as_ptr() as *const c_void), not_found);
    xfp::set_module(None);
}
