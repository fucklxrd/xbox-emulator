use compat_common::gdk::{self, capture, package, store, system, user, xasync, Guid};
use core::ffi::c_void;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const E_ABORT: i32 = 0x8000_4004u32 as i32;

#[repr(C)]
struct Block {
    queue: usize,
    context: usize,
    callback: usize,
    internal: [u8; 32],
}

fn env() {
    std::env::set_var("TOMBOX_GDK_XASYNC", "emulated");
    std::env::set_var("TOMBOX_CAPTURE_HISTORY", "0");
    std::env::set_var("TOMBOX_GDK_ROOT", std::env::temp_dir().join(format!("tombox_gdk_vfix_{}", std::process::id())));
    assert!(gdk::rt::active());
}

unsafe fn method(clsid: Guid, iid: Guid, slot: usize) -> (*mut c_void, usize) {
    let mut obj: *mut c_void = core::ptr::null_mut();
    assert_eq!(gdk::query_api_impl(&clsid, &iid, &mut obj), 0);
    (obj, *(*(obj as *const *const usize)).add(slot))
}

type Cb = unsafe extern "system" fn(*mut c_void, bool);
type TermCb = unsafe extern "system" fn(*mut c_void);

struct Tq {
    o: *mut c_void,
    create: unsafe extern "system" fn(*mut c_void, u32, u32, *mut usize) -> i32,
    composite: unsafe extern "system" fn(*mut c_void, usize, usize, *mut usize) -> i32,
    port: unsafe extern "system" fn(*mut c_void, usize, u32, *mut usize) -> i32,
    dispatch: unsafe extern "system" fn(*mut c_void, usize, u32, u32) -> bool,
    close: unsafe extern "system" fn(*mut c_void, usize),
    submit: unsafe extern "system" fn(*mut c_void, usize, u32, *mut c_void, Cb) -> i32,
    delayed: unsafe extern "system" fn(*mut c_void, usize, u32, u32, *mut c_void, Cb) -> i32,
    terminate: unsafe extern "system" fn(*mut c_void, usize, bool, *mut c_void, TermCb) -> i32,
}

unsafe fn tq() -> Tq {
    env();
    let s = |n| method(xasync::XASYNC, xasync::XASYNC, n);
    Tq {
        o: s(12).0,
        create: core::mem::transmute(s(12).1),
        composite: core::mem::transmute(s(13).1),
        port: core::mem::transmute(s(14).1),
        dispatch: core::mem::transmute(s(16).1),
        close: core::mem::transmute(s(17).1),
        submit: core::mem::transmute(s(18).1),
        delayed: core::mem::transmute(s(19).1),
        terminate: core::mem::transmute(s(22).1),
    }
}

static LOG: Mutex<Vec<(usize, &'static str)>> = Mutex::new(Vec::new());
static SERIAL: Mutex<()> = Mutex::new(());

fn push(tag: usize, what: &'static str) {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push((tag, what));
}

fn events(tag: usize) -> Vec<&'static str> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|e| e.0 == tag).map(|e| e.1).collect()
}

unsafe extern "system" fn record(ctx: *mut c_void, canceled: bool) {
    push(ctx as usize, if canceled { "canceled" } else { "ran" });
}

unsafe extern "system" fn slow(ctx: *mut c_void, canceled: bool) {
    std::thread::sleep(Duration::from_millis(300));
    push(ctx as usize, if canceled { "slow canceled" } else { "slow ran" });
}

unsafe extern "system" fn delayed_cb(ctx: *mut c_void, canceled: bool) {
    push(ctx as usize, if canceled { "delayed canceled" } else { "delayed ran" });
}

unsafe extern "system" fn terminated(ctx: *mut c_void) {
    push(ctx as usize, "terminated");
}

#[test]
fn diagnostic_clip_takes_start_time_by_value() {
    env();
    unsafe {
        let (o, f) = method(capture::CAPTURE, capture::CAPTURE_IID, 4);
        let f: unsafe extern "system" fn(*mut c_void, i64, u32, *const u8, *mut u8) -> i32 = core::mem::transmute(f);
        let mut out = vec![0u8; capture::CLIP_RESULT_SIZE];
        let now_s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        assert_eq!(f(o, now_s + 100_000_000, 1000, core::ptr::null(), out.as_mut_ptr()), gdk::E_INVALIDARG);
        assert_eq!(f(o, now_s, 0, core::ptr::null(), out.as_mut_ptr()), gdk::E_INVALIDARG);
        assert_eq!(f(o, now_s, 1000, core::ptr::null(), core::ptr::null_mut()), gdk::E_INVALIDARG);
    }
}

#[test]
fn submit_after_terminate_returns_e_abort() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let t = tq();
        let mut q = 0usize;
        assert_eq!((t.create)(t.o, 1, 1, &mut q), 0);
        assert_eq!((t.terminate)(t.o, q, true, 0x3001 as *mut c_void, terminated), 0);
        assert_eq!(events(0x3001), ["terminated"]);
        assert_eq!((t.submit)(t.o, q, 0, 0x3002 as *mut c_void, record), E_ABORT);
        assert_eq!((t.submit)(t.o, q, 1, 0x3002 as *mut c_void, record), E_ABORT);
        assert_eq!((t.delayed)(t.o, q, 0, 50, 0x3002 as *mut c_void, record), E_ABORT);
        assert_eq!((t.delayed)(t.o, q, 1, 0, 0x3002 as *mut c_void, record), E_ABORT);
        std::thread::sleep(Duration::from_millis(150));
        assert!(events(0x3002).is_empty(), "{:?}", events(0x3002));
        (t.close)(t.o, q);
    }
}

#[test]
fn waiting_terminate_cancels_delayed_and_drains_thread_pool_before_callback() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let t = tq();
        let mut q = 0usize;
        assert_eq!((t.create)(t.o, 1, 1, &mut q), 0);
        let tag = 0x4001usize;
        assert_eq!((t.submit)(t.o, q, 0, tag as *mut c_void, slow), 0);
        assert_eq!((t.delayed)(t.o, q, 0, 1500, tag as *mut c_void, delayed_cb), 0);
        assert_eq!((t.delayed)(t.o, q, 1, 1500, tag as *mut c_void, delayed_cb), 0);
        std::thread::sleep(Duration::from_millis(50));
        let t0 = Instant::now();
        assert_eq!((t.terminate)(t.o, q, true, tag as *mut c_void, terminated), 0);
        assert!(t0.elapsed() >= Duration::from_millis(150), "terminate(wait) returned before the in-flight callback finished");
        let e = events(tag);
        assert_eq!(e.last(), Some(&"terminated"), "{e:?}");
        assert_eq!(e.iter().filter(|s| **s == "delayed canceled").count(), 2, "{e:?}");
        assert!(e.contains(&"slow ran") || e.contains(&"slow canceled"), "{e:?}");
        std::thread::sleep(Duration::from_millis(1800));
        assert_eq!(events(tag), e, "a callback fired after the terminate callback");
        (t.close)(t.o, q);
    }
}

#[test]
fn async_terminate_callback_follows_in_flight_thread_pool_work() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let t = tq();
        let mut q = 0usize;
        assert_eq!((t.create)(t.o, 1, 1, &mut q), 0);
        let tag = 0x5001usize;
        assert_eq!((t.submit)(t.o, q, 0, tag as *mut c_void, slow), 0);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!((t.terminate)(t.o, q, false, tag as *mut c_void, terminated), 0);
        let t0 = Instant::now();
        while !events(tag).contains(&"terminated") && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let e = events(tag);
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(e[0].starts_with("slow"), "{e:?}");
        assert_eq!(e[1], "terminated");
        (t.close)(t.o, q);
    }
}

#[test]
fn composite_terminate_cancels_only_its_own_pending_callbacks() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let t = tq();
        let mut q = 0usize;
        assert_eq!((t.create)(t.o, 0, 0, &mut q), 0);
        let (mut w, mut c) = (0usize, 0usize);
        assert_eq!((t.port)(t.o, q, 0, &mut w), 0);
        assert_eq!((t.port)(t.o, q, 1, &mut c), 0);
        let mut comp = 0usize;
        assert_eq!((t.composite)(t.o, w, c, &mut comp), 0);
        assert_eq!((t.submit)(t.o, comp, 0, 0x6001 as *mut c_void, record), 0);
        assert_eq!((t.delayed)(t.o, comp, 0, 5000, 0x6001 as *mut c_void, record), 0);
        assert_eq!((t.submit)(t.o, q, 0, 0x6002 as *mut c_void, record), 0);
        assert_eq!((t.terminate)(t.o, comp, true, 0x6003 as *mut c_void, terminated), 0);
        assert_eq!(events(0x6001), ["canceled", "canceled"]);
        assert_eq!(events(0x6003), ["terminated"]);
        assert!(events(0x6002).is_empty());
        assert_eq!((t.submit)(t.o, comp, 0, 0x6001 as *mut c_void, record), E_ABORT);
        assert!((t.dispatch)(t.o, q, 0, 0));
        assert_eq!(events(0x6002), ["ran"]);
        (t.close)(t.o, comp);
        (t.close)(t.o, q);
    }
}

#[test]
fn xversion_packs_major_into_the_low_word() {
    assert_eq!(package::xversion([1, 2, 3, 4]), 0x0004_0003_0002_0001);
    let b = system::analytics_bytes();
    let os = u64::from_le_bytes(b[0..8].try_into().unwrap());
    assert_eq!(os & 0xffff, 10);
    assert_eq!((os >> 32) & 0xffff, 25398);
    assert_eq!(&b[0..2], &10u16.to_le_bytes());
    assert_eq!(&b[4..6], &25398u16.to_le_bytes());
}

#[test]
fn user_lookups_return_game_user_codes() {
    env();
    unsafe {
        let (o, by_local) = method(user::CLSID, user::IID_V1, 10);
        let by_local: unsafe extern "system" fn(*mut c_void, u64, *mut usize) -> i32 = core::mem::transmute(by_local);
        let (_, by_id) = method(user::CLSID, user::IID_V1, 12);
        let by_id: unsafe extern "system" fn(*mut c_void, u64, *mut usize) -> i32 = core::mem::transmute(by_id);
        let mut h = 0usize;
        assert_eq!(by_local(o, 0xdead_beef_0000_0001, &mut h), user::E_GAMEUSER_USER_NOT_FOUND);
        assert_eq!(by_id(o, 0xdead_beef_0000_0002, &mut h), user::E_GAMEUSER_USER_NOT_FOUND);
        assert_eq!(user::E_GAMEUSER_USER_NOT_FOUND, 0x8924_5104u32 as i32);
        assert_eq!(user::E_GAMEUSER_DEFERRAL_NOT_AVAILABLE, 0x8924_5103u32 as i32);
        let (o2, defer) = method(user::CLSID, user::IID_V2, 35);
        let defer: unsafe extern "system" fn(*mut c_void, *mut usize) -> i32 = core::mem::transmute(defer);
        let mut d = 7usize;
        assert_eq!(defer(o2, &mut d), user::E_GAMEUSER_DEFERRAL_NOT_AVAILABLE);
        assert_eq!(d, 0);
    }
}

#[test]
fn store_codes_are_the_documented_e_gamestore_values() {
    assert_eq!(store::E_GAMESTORE_LICENSE_ACTION_NOT_APPLICABLE_TO_PRODUCT, 0x8924_5300u32 as i32);
    assert_eq!(store::E_GAMESTORE_INSUFFICIENT_QUANTITY, 0x8924_5303u32 as i32);
    assert_eq!(store::E_GAMESTORE_ALREADY_PURCHASED, 0x8924_5304u32 as i32);
}

#[test]
fn consumable_fulfillment_over_balance_reports_insufficient_quantity() {
    env();
    unsafe {
        let (o, fulfill) = method(store::CLSID, store::IID, 35);
        let fulfill: unsafe extern "system" fn(*mut c_void, usize, *const u8, u32, *const u8, *mut Block) -> i32 = core::mem::transmute(fulfill);
        let (xa, status) = method(xasync::XASYNC, xasync::XASYNC, 3);
        let status: unsafe extern "system" fn(*mut c_void, *mut Block, bool) -> i32 = core::mem::transmute(status);
        let id = b"9TOMBOXNOBALANCE\0";
        let mut b = Box::new(Block { queue: 0, context: 0, callback: 0, internal: [0; 32] });
        assert_eq!(fulfill(o, 1, id.as_ptr(), 5, core::ptr::null(), &mut *b), 0);
        assert_eq!(status(xa, &mut *b, true), store::E_GAMESTORE_INSUFFICIENT_QUANTITY);
    }
}
