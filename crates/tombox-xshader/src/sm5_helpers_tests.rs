use std::ffi::c_void;

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

type D3DCompile = unsafe extern "system" fn(*const u8, usize, *const u8, *const c_void, *const c_void, *const u8, *const u8, u32, u32, *mut *mut c_void, *mut *mut c_void) -> i32;

unsafe fn release(o: *mut c_void) {
    if !o.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(o as *const *const usize)).add(2));
        f(o);
    }
}

#[test]
fn common_helpers_have_no_64bit_integers_for_sm5() {
    assert!(!crate::hlsl::CUBE_HELPERS.contains("uint64_t"), "the always-emitted helper prelude must stay valid ps_5_1 HLSL");
}

#[test]
fn common_helpers_compile_with_fxc_ps_5_1() {
    let h = unsafe { LoadLibraryA(b"d3dcompiler_47.dll\0".as_ptr()) };
    if h.is_null() {
        eprintln!("d3dcompiler_47.dll missing; FXC check skipped");
        return;
    }
    let f: D3DCompile = unsafe { core::mem::transmute(GetProcAddress(h, b"D3DCompile\0".as_ptr())) };
    let src = format!("{}\nfloat4 main(float4 p : SV_Position) : SV_Target {{ return float4((float)tb_nrm(p.x, 65535u), (float)tb_nrm(p.y, 32767u), 0, 0); }}\n", crate::hlsl::CUBE_HELPERS);
    let (mut code, mut err): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
    let hr = unsafe { f(src.as_ptr(), src.len(), core::ptr::null(), core::ptr::null(), core::ptr::null(), b"main\0".as_ptr(), b"ps_5_1\0".as_ptr(), 0, 0, &mut code, &mut err) };
    let msg = if err.is_null() {
        String::new()
    } else {
        let p: unsafe extern "system" fn(*mut c_void) -> *const i8 = unsafe { core::mem::transmute(*(*(err as *const *const usize)).add(3)) };
        unsafe { std::ffi::CStr::from_ptr(p(err)) }.to_string_lossy().into_owned()
    };
    unsafe {
        release(code);
        release(err);
    }
    assert!(hr >= 0, "hr={hr:#010x} {msg}");
}
