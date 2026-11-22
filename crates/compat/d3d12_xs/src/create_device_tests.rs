use core::ffi::c_void;

use crate::device_wrap::Guid;

const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };

#[test]
fn xbox_create_device_null_params_is_invalidarg() {
    let mut dev = 0x1234usize as *mut c_void;
    let hr = crate::D3D12XboxCreateDevice(core::ptr::null_mut(), core::ptr::null(), &IID_DEVICE as *const Guid as *const c_void, &mut dev);
    assert_eq!(hr, crate::E_INVALIDARG);
    assert_eq!(dev as usize, 0x1234);
}

#[test]
fn create_device_builds_console_default_params() {
    let p = crate::default_create_params(0x0010_0001);
    assert_eq!(core::mem::size_of_val(&p), 0x48);
    assert_eq!(p[0], 0x20004);
    assert_eq!(p[1], 0x0010_0001);
    assert_eq!(p[2], u32::MAX);
    assert_eq!(p[6], u32::MAX);
    assert_eq!(p[7], u32::MAX);
    for i in [3, 4, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17] {
        assert_eq!(p[i], 0, "dword {i}");
    }
}

#[test]
fn create_device_returns_wrapped_device() {
    let mut dev: *mut c_void = core::ptr::null_mut();
    let hr = crate::D3D12CreateDevice(core::ptr::null_mut(), 0xb000, &IID_DEVICE as *const Guid as *const c_void, &mut dev);
    if hr < 0 {
        eprintln!("no D3D12 device on this machine (hr={hr:#010x}); wrap check skipped");
        assert!(dev.is_null());
        return;
    }
    assert!(!dev.is_null());
    assert!(crate::device_wrap::is_wrapper(dev), "D3D12CreateDevice returned an unwrapped device");
    unsafe {
        let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(dev as *const *const usize)).add(2));
        rel(dev);
    }
}
