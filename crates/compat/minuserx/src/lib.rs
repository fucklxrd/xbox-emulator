#![allow(non_snake_case, clippy::missing_safety_doc)]

use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::Mutex;

pub mod callback;
pub mod queue;

pub use callback::*;

pub const EXPORTS_TXT: &str = include_str!(env!("MUX_EXPORTS"));
pub const DLL_FILE: &str = "minuserx.dll";

pub const S_OK: i32 = 0;
pub const E_NOTIMPL: u32 = 0x8000_4001;
pub const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
pub const E_POINTER: i32 = 0x8000_4003u32 as i32;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_OUTOFMEMORY: i32 = 0x8007_000Eu32 as i32;
pub const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154u32 as i32;

pub type Guid = [u8; 16];

pub const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Guid {
    let a = d1.to_le_bytes();
    let b = d2.to_le_bytes();
    let c = d3.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]]
}

const fn ole(d1: u32) -> Guid {
    guid(d1, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46])
}

pub const CLSID_HOOKS: Guid = guid(0x7fa7a8fd, 0xa259, 0x4a79, [0xbb, 0x7e, 0x09, 0x4c, 0x18, 0xcd, 0x07, 0x23]);
pub const IID_HOOKS: Guid = guid(0x338203c7, 0x6c86, 0x49c4, [0xb5, 0x3e, 0x4c, 0xa4, 0x99, 0x90, 0x56, 0xce]);
pub const IID_CALLBACK: Guid = guid(0x4e4bde9e, 0xb970, 0x41e7, [0xbc, 0x12, 0xae, 0xa6, 0xc6, 0xee, 0x6d, 0xd5]);
pub const IID_IINSPECTABLE: Guid = guid(0xaf86e2e0, 0xb12d, 0x4c6a, [0x9c, 0x5a, 0xd7, 0xaa, 0x65, 0x10, 0x1e, 0x90]);
pub const IID_IAGILEOBJECT: Guid = guid(0x94ea2b94, 0xe9cc, 0x49e0, [0xc0, 0xff, 0xee, 0x64, 0xca, 0x8f, 0x5b, 0x90]);
pub const IID_IUNKNOWN: Guid = ole(0);
pub const IID_IMARSHAL: Guid = ole(3);
pub const IID_IWEAKREFERENCE: Guid = ole(0x37);
pub const IID_IWEAKREFERENCESOURCE: Guid = ole(0x38);
pub const CLSID_INPROC_FREE_MARSHALER: Guid = ole(0x33a);

const CLSCTX_INPROC_SERVER: u32 = 1;
const RPC_C_AUTHN_LEVEL_NONE: u32 = 1;
const RPC_C_IMP_LEVEL_IDENTIFY: u32 = 2;
const EOAC_DISABLE_AAA: u32 = 0x1000;

#[link(name = "kernel32")]
extern "system" {
    fn SetLastError(e: u32);
    fn GetLastError() -> u32;
}

#[link(name = "ole32")]
extern "system" {
    fn CoIncrementMTAUsage(cookie: *mut *mut c_void) -> i32;
    fn CoDecrementMTAUsage(cookie: *mut c_void) -> i32;
    fn CoInitializeSecurity(sd: *mut c_void, n: i32, svc: *mut c_void, r1: *mut c_void, authn: u32, imp: u32, list: *mut c_void, caps: u32, r3: *mut c_void) -> i32;
    fn CoCreateInstance(clsid: *const Guid, outer: *mut c_void, ctx: u32, iid: *const Guid, out: *mut *mut c_void) -> i32;
}

pub fn hr_win32(e: u32) -> i32 {
    if e as i32 <= 0 {
        e as i32
    } else {
        ((e & 0xffff) | 0x8007_0000) as i32
    }
}

unsafe fn notimpl() -> usize {
    SetLastError(E_NOTIMPL);
    0
}

macro_rules! console_notimpl {
    ($($n:ident),*) => {
        $(
            #[no_mangle]
            pub unsafe extern "system" fn $n() -> usize {
                notimpl()
            }
        )*
    };
}

console_notimpl!(
    mux_ord2534,
    mux_ClipCursor,
    mux_GetClipCursor,
    mux_GetCursor,
    mux_GetMessageExtraInfo,
    mux_GetMessagePos,
    mux_GetMessageTime,
    mux_InSendMessage,
    mux_InSendMessageEx,
    mux_RegisterWindowMessageA,
    mux_RegisterWindowMessageW,
    mux_SendMessageCallbackA,
    mux_SendMessageCallbackW,
    mux_SendMessageTimeoutA,
    mux_SendMessageTimeoutW,
    mux_SendNotifyMessageA,
    mux_SendNotifyMessageW,
    mux_SetCursorPos,
    mux_SetMessageExtraInfo,
    mux_WaitMessage
);

#[link(name = "user32")]
extern "system" {
    fn IsWindowUnicode(h: *mut c_void) -> i32;
    fn GetWindowLongPtrA(h: *mut c_void, i: i32) -> isize;
    fn GetWindowLongPtrW(h: *mut c_void, i: i32) -> isize;
    fn SetWindowLongPtrA(h: *mut c_void, i: i32, v: isize) -> isize;
    fn SetWindowLongPtrW(h: *mut c_void, i: i32, v: isize) -> isize;
}

unsafe fn window_long_get(h: *mut c_void, i: i32) -> isize {
    if IsWindowUnicode(h) != 0 {
        GetWindowLongPtrW(h, i)
    } else {
        GetWindowLongPtrA(h, i)
    }
}

unsafe fn window_long_set(h: *mut c_void, i: i32, v: isize) -> isize {
    if IsWindowUnicode(h) != 0 {
        SetWindowLongPtrW(h, i, v)
    } else {
        SetWindowLongPtrA(h, i, v)
    }
}

#[no_mangle]
pub unsafe extern "system" fn mux_GetWindowLongPtrA(h: *mut c_void, i: i32) -> isize {
    window_long_get(h, i)
}

#[no_mangle]
pub unsafe extern "system" fn mux_GetWindowLongPtrW(h: *mut c_void, i: i32) -> isize {
    window_long_get(h, i)
}

#[no_mangle]
pub unsafe extern "system" fn mux_SetWindowLongPtrA(h: *mut c_void, i: i32, v: isize) -> isize {
    window_long_set(h, i, v)
}

#[no_mangle]
pub unsafe extern "system" fn mux_SetWindowLongPtrW(h: *mut c_void, i: i32, v: isize) -> isize {
    window_long_set(h, i, v)
}

fn result_bool(r: Result<bool, u32>) -> i32 {
    match r {
        Ok(b) => {
            unsafe { SetLastError(0) };
            b as i32
        }
        Err(e) => {
            unsafe { SetLastError(e) };
            0
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn mux_PostMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> i32 {
    let hr = queue::post(hwnd, msg, wparam, lparam);
    queue::last_error_from(hr);
    (hr >= 0) as i32
}

#[no_mangle]
pub unsafe extern "system" fn mux_PostThreadMessageW(tid: u32, msg: u32, wparam: usize, lparam: isize) -> i32 {
    let hr = queue::post_thread(tid, msg, wparam, lparam);
    queue::last_error_from(hr);
    (hr >= 0) as i32
}

#[no_mangle]
pub unsafe extern "system" fn mux_PeekMessageW(m: *mut queue::Msg, hwnd: *mut c_void, lo: u32, hi: u32, flags: u32) -> i32 {
    result_bool(queue::peek(m, hwnd, lo, hi, flags))
}

#[no_mangle]
pub unsafe extern "system" fn mux_GetMessageW(m: *mut queue::Msg, hwnd: *mut c_void, lo: u32, hi: u32) -> i32 {
    match queue::get(m, hwnd, lo, hi) {
        Ok(b) => {
            SetLastError(0);
            b as i32
        }
        Err(e) => {
            SetLastError(e);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn mux_DispatchMessageW(m: *const queue::Msg) -> isize {
    match queue::dispatch(m) {
        Ok(r) => {
            SetLastError(0);
            r
        }
        Err(e) => {
            SetLastError(e);
            0
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn mux_PostMessageA(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> i32 {
    mux_PostMessageW(hwnd, msg, wparam, lparam)
}

#[no_mangle]
pub unsafe extern "system" fn mux_PostThreadMessageA(tid: u32, msg: u32, wparam: usize, lparam: isize) -> i32 {
    mux_PostThreadMessageW(tid, msg, wparam, lparam)
}

#[no_mangle]
pub unsafe extern "system" fn mux_PeekMessageA(m: *mut queue::Msg, hwnd: *mut c_void, lo: u32, hi: u32, flags: u32) -> i32 {
    mux_PeekMessageW(m, hwnd, lo, hi, flags)
}

#[no_mangle]
pub unsafe extern "system" fn mux_GetMessageA(m: *mut queue::Msg, hwnd: *mut c_void, lo: u32, hi: u32) -> i32 {
    mux_GetMessageW(m, hwnd, lo, hi)
}

#[no_mangle]
pub unsafe extern "system" fn mux_DispatchMessageA(m: *const queue::Msg) -> isize {
    mux_DispatchMessageW(m)
}

pub unsafe fn vcall(o: *mut c_void, slot: usize) -> usize {
    *(*(o as *const *const usize)).add(slot)
}

pub unsafe fn query(o: *mut c_void, iid: &Guid, out: *mut *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vcall(o, 0));
    f(o, iid, out)
}

pub unsafe fn add_ref(o: *mut c_void) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vcall(o, 1));
    f(o)
}

pub unsafe fn release(o: *mut c_void) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vcall(o, 2));
    f(o)
}

pub(crate) unsafe fn hooks_refresh(o: *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(vcall(o, 4));
    f(o)
}

unsafe fn hooks_attach(o: *mut c_void, cb: *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 = core::mem::transmute(vcall(o, 3));
    f(o, cb)
}

pub struct Hooks {
    pub hooks: usize,
    pub init_done: bool,
    pub mta_cookie: usize,
    pub clsid: Guid,
}

pub static HOOKS: Mutex<Hooks> = Mutex::new(Hooks { hooks: 0, init_done: false, mta_cookie: 0, clsid: CLSID_HOOKS });

fn hooks_lock() -> std::sync::MutexGuard<'static, Hooks> {
    HOOKS.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe fn attach(hooks: *mut c_void) -> i32 {
    let cb = new_callback(true);
    hooks_attach(hooks, callback_iface(cb));
    release(cb as *mut c_void);
    S_OK
}

unsafe fn create_default(st: &mut Hooks) -> i32 {
    if st.mta_cookie != 0 {
        let e = GetLastError();
        CoDecrementMTAUsage(st.mta_cookie as *mut c_void);
        SetLastError(e);
    }
    st.mta_cookie = 0;
    let mut cookie = null_mut();
    let hr = CoIncrementMTAUsage(&mut cookie);
    if hr < 0 {
        return hr;
    }
    st.mta_cookie = cookie as usize;
    CoInitializeSecurity(null_mut(), -1, null_mut(), null_mut(), RPC_C_AUTHN_LEVEL_NONE, RPC_C_IMP_LEVEL_IDENTIFY, null_mut(), EOAC_DISABLE_AAA, null_mut());
    if st.hooks != 0 {
        let old = st.hooks as *mut c_void;
        st.hooks = 0;
        release(old);
    }
    let mut out = null_mut();
    let hr = CoCreateInstance(&st.clsid, null_mut(), CLSCTX_INPROC_SERVER, &IID_HOOKS, &mut out);
    if hr == REGDB_E_CLASSNOTREG {
        return S_OK;
    }
    if hr < 0 {
        return hr;
    }
    st.hooks = out as usize;
    if out.is_null() {
        return S_OK;
    }
    attach(out);
    S_OK
}

pub(crate) unsafe fn get_hooks(out: *mut *mut c_void) -> i32 {
    let mut st = hooks_lock();
    if st.hooks == 0 && !st.init_done {
        let hr = create_default(&mut st);
        if hr < 0 {
            return hr;
        }
        st.init_done = true;
    }
    let h = st.hooks as *mut c_void;
    if !h.is_null() {
        add_ref(h);
    }
    *out = h;
    S_OK
}

#[no_mangle]
pub unsafe extern "system" fn mux_MinUserXP_GetHooks(out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    get_hooks(out)
}

#[no_mangle]
pub unsafe extern "system" fn mux_MinUserXP_SetHooks(hooks: *mut c_void) -> i32 {
    let mut st = hooks_lock();
    if st.hooks as *mut c_void != hooks {
        if !hooks.is_null() {
            add_ref(hooks);
        }
        let old = st.hooks as *mut c_void;
        st.hooks = hooks as usize;
        if !old.is_null() {
            release(old);
        }
    }
    let h = st.hooks as *mut c_void;
    if h.is_null() {
        return S_OK;
    }
    drop(st);
    attach(h)
}

pub fn reset_for_tests() {
    let mut st = hooks_lock();
    if st.hooks != 0 {
        unsafe { release(st.hooks as *mut c_void) };
    }
    st.hooks = 0;
    st.init_done = false;
    st.clsid = CLSID_HOOKS;
}
