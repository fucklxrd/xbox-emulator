use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::atomic::{AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use minuserx::*;

#[link(name = "kernel32")]
extern "system" {
    fn SetLastError(e: u32);
    fn GetLastError() -> u32;
}

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn check_notimpl(f: unsafe extern "system" fn() -> usize) {
    unsafe {
        SetLastError(0);
        assert_eq!(f(), 0);
        assert_eq!(GetLastError(), E_NOTIMPL);
    }
}

macro_rules! notimpl_tests {
    ($($t:ident => $f:ident),*) => {
        $(
            #[test]
            fn $t() {
                check_notimpl($f);
            }
        )*
    };
}

notimpl_tests!(
    ord2534_fails_like_console => mux_ord2534,
    clip_cursor_fails_like_console => mux_ClipCursor,
    get_clip_cursor_fails_like_console => mux_GetClipCursor,
    get_cursor_fails_like_console => mux_GetCursor,
    get_message_extra_info_fails_like_console => mux_GetMessageExtraInfo,
    get_message_pos_fails_like_console => mux_GetMessagePos,
    get_message_time_fails_like_console => mux_GetMessageTime,
    in_send_message_fails_like_console => mux_InSendMessage,
    in_send_message_ex_fails_like_console => mux_InSendMessageEx,
    register_window_message_a_fails_like_console => mux_RegisterWindowMessageA,
    register_window_message_w_fails_like_console => mux_RegisterWindowMessageW,
    send_message_callback_a_fails_like_console => mux_SendMessageCallbackA,
    send_message_callback_w_fails_like_console => mux_SendMessageCallbackW,
    send_message_timeout_a_fails_like_console => mux_SendMessageTimeoutA,
    send_message_timeout_w_fails_like_console => mux_SendMessageTimeoutW,
    send_notify_message_a_fails_like_console => mux_SendNotifyMessageA,
    send_notify_message_w_fails_like_console => mux_SendNotifyMessageW,
    set_cursor_pos_fails_like_console => mux_SetCursorPos,
    set_message_extra_info_fails_like_console => mux_SetMessageExtraInfo,
    wait_message_fails_like_console => mux_WaitMessage
);

#[repr(C)]
struct FakeHooks {
    vtbl: *const [usize; 6],
    refs: AtomicU32,
    attached: AtomicUsize,
    attach_calls: AtomicU32,
    refresh_calls: AtomicU32,
    refresh_hr: AtomicI32,
}

unsafe extern "system" fn fh_qi(_t: *mut FakeHooks, _i: *const Guid, o: *mut *mut c_void) -> i32 {
    *o = null_mut();
    E_NOINTERFACE
}

unsafe extern "system" fn fh_add_ref(t: *mut FakeHooks) -> u32 {
    (*t).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn fh_release(t: *mut FakeHooks) -> u32 {
    (*t).refs.fetch_sub(1, Ordering::SeqCst) - 1
}

unsafe extern "system" fn fh_attach(t: *mut FakeHooks, cb: *mut c_void) -> i32 {
    (*t).attach_calls.fetch_add(1, Ordering::SeqCst);
    let old = (*t).attached.swap(cb as usize, Ordering::SeqCst);
    if old != 0 {
        release(old as *mut c_void);
    }
    add_ref(cb);
    S_OK
}

unsafe extern "system" fn fh_refresh(t: *mut FakeHooks) -> i32 {
    (*t).refresh_calls.fetch_add(1, Ordering::SeqCst);
    (*t).refresh_hr.load(Ordering::SeqCst)
}

unsafe extern "system" fn fh_custom(_t: *mut FakeHooks) -> i32 {
    E_INVALIDARG
}

fn fake() -> &'static FakeHooks {
    let v: &'static [usize; 6] = Box::leak(Box::new([
        fh_qi as *const () as usize,
        fh_add_ref as *const () as usize,
        fh_release as *const () as usize,
        fh_attach as *const () as usize,
        fh_refresh as *const () as usize,
        fh_custom as *const () as usize,
    ]));
    Box::leak(Box::new(FakeHooks {
        vtbl: v,
        refs: AtomicU32::new(1),
        attached: AtomicUsize::new(0),
        attach_calls: AtomicU32::new(0),
        refresh_calls: AtomicU32::new(0),
        refresh_hr: AtomicI32::new(S_OK),
    }))
}

fn ptr(f: &FakeHooks) -> *mut c_void {
    f as *const FakeHooks as *mut c_void
}

#[test]
fn get_hooks_without_registered_class_returns_s_ok_and_null() {
    let _g = serial();
    reset_for_tests();
    let mut out = 1usize as *mut c_void;
    assert_eq!(unsafe { mux_MinUserXP_GetHooks(&mut out) }, S_OK);
    assert!(out.is_null());
    assert!(HOOKS.lock().unwrap().init_done);
    let mut again = 1usize as *mut c_void;
    assert_eq!(unsafe { mux_MinUserXP_GetHooks(&mut again) }, S_OK);
    assert!(again.is_null());
    assert_eq!(unsafe { mux_MinUserXP_GetHooks(null_mut()) }, E_INVALIDARG);
}

#[test]
fn get_hooks_returns_the_set_object_add_refed() {
    let _g = serial();
    reset_for_tests();
    let f = fake();
    assert_eq!(unsafe { mux_MinUserXP_SetHooks(ptr(f)) }, S_OK);
    assert_eq!(f.refs.load(Ordering::SeqCst), 2);
    let mut out = null_mut();
    assert_eq!(unsafe { mux_MinUserXP_GetHooks(&mut out) }, S_OK);
    assert_eq!(out, ptr(f));
    assert_eq!(f.refs.load(Ordering::SeqCst), 3);
    unsafe { release(out) };
    reset_for_tests();
    assert_eq!(f.refs.load(Ordering::SeqCst), 1);
}

#[test]
fn set_hooks_swaps_refs_and_attaches_a_callback() {
    let _g = serial();
    reset_for_tests();
    let a = fake();
    let b = fake();
    assert_eq!(unsafe { mux_MinUserXP_SetHooks(ptr(a)) }, S_OK);
    assert_eq!(a.attach_calls.load(Ordering::SeqCst), 1);
    let cb = a.attached.load(Ordering::SeqCst) as *mut c_void;
    assert!(!cb.is_null());
    assert_eq!(unsafe { mux_MinUserXP_SetHooks(ptr(a)) }, S_OK);
    assert_eq!(a.refs.load(Ordering::SeqCst), 2);
    assert_eq!(a.attach_calls.load(Ordering::SeqCst), 2);
    assert_eq!(unsafe { mux_MinUserXP_SetHooks(ptr(b)) }, S_OK);
    assert_eq!(a.refs.load(Ordering::SeqCst), 1);
    assert_eq!(b.refs.load(Ordering::SeqCst), 2);
    assert_eq!(unsafe { mux_MinUserXP_SetHooks(null_mut()) }, S_OK);
    assert_eq!(b.refs.load(Ordering::SeqCst), 1);
    let mut out = 1usize as *mut c_void;
    assert_eq!(unsafe { mux_MinUserXP_GetHooks(&mut out) }, S_OK);
    assert!(out.is_null());
    reset_for_tests();
}

#[test]
fn exports_txt_covers_the_console_ordinal_table() {
    let rows: Vec<_> = EXPORTS_TXT.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(rows.len(), 74);
    let mut ords: Vec<u32> = rows.iter().map(|l| l.split_whitespace().next().unwrap().parse().unwrap()).collect();
    ords.sort_unstable();
    assert_eq!(ords, (2534..=2607).collect::<Vec<_>>());
    let st = |s: &str| rows.iter().filter(|l| l.split_whitespace().nth(2) == Some(s)).count();
    assert_eq!(st("forwarded"), 38);
    assert_eq!(st("implemented"), 36);
}
