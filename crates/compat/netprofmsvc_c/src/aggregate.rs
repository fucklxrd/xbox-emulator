use crate::tlg::{EnableCallback, Guid, Provider};
use crate::{AGGREGATE_PROVIDER, PROVIDER};
use core::ffi::c_void;
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

#[link(name = "kernel32")]
extern "system" {
    fn CreateThreadpoolTimer(cb: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void), ctx: *mut c_void, env: *mut c_void) -> *mut c_void;
    fn SetThreadpoolTimer(t: *mut c_void, due: *const i64, period: u32, window: u32);
    fn WaitForThreadpoolTimerCallbacks(t: *mut c_void, cancel: i32);
    fn CloseThreadpoolTimer(t: *mut c_void);
}

pub struct Aggregate {
    pub count: AtomicU32,
    pub callback: AtomicUsize,
    pub callback_context: AtomicPtr<c_void>,
    pub provider: &'static Provider,
    pub timer: AtomicPtr<c_void>,
    pub period_ms: AtomicU32,
}

pub struct List(pub Vec<*mut Aggregate>);
unsafe impl Send for List {}

pub static LIST: Mutex<List> = Mutex::new(List(Vec::new()));

pub fn flush_period_ms(object: usize, provider: &Guid) -> u32 {
    let mut h: u32 = 0;
    let key = (object >> 4) as u64;
    for b in provider.bytes().into_iter().chain(key.to_le_bytes()) {
        h = h.wrapping_add(b as u32).wrapping_mul(0x401);
        h ^= h >> 6;
    }
    h = h.wrapping_mul(9);
    h ^= h >> 11;
    h = h.wrapping_mul(0x8001);
    h % 600_000 + 600_000
}

impl Aggregate {
    pub fn create() -> *mut Aggregate {
        let a = Box::into_raw(Box::new(Aggregate {
            count: AtomicU32::new(0),
            callback: AtomicUsize::new(0),
            callback_context: AtomicPtr::new(core::ptr::null_mut()),
            provider: &PROVIDER,
            timer: AtomicPtr::new(core::ptr::null_mut()),
            period_ms: AtomicU32::new(0),
        }));
        let t = unsafe { CreateThreadpoolTimer(timer_callback, a as *mut c_void, core::ptr::null_mut()) };
        if t.is_null() {
            unsafe { destroy(a) };
            return core::ptr::null_mut();
        }
        unsafe { (*a).timer.store(t, Ordering::SeqCst) };
        a
    }

    pub fn entries(&self) -> u32 {
        self.count.load(Ordering::SeqCst)
    }

    pub fn stop_timer(&self) {
        let t = self.timer.load(Ordering::SeqCst);
        if t.is_null() {
            return;
        }
        unsafe {
            SetThreadpoolTimer(t, core::ptr::null(), 0, 0);
            WaitForThreadpoolTimerCallbacks(t, 1);
            CloseThreadpoolTimer(t);
        }
        self.timer.store(core::ptr::null_mut(), Ordering::SeqCst);
    }
}

pub unsafe fn destroy(a: *mut Aggregate) {
    if a.is_null() {
        return;
    }
    (*a).stop_timer();
    drop(Box::from_raw(a));
}

unsafe extern "system" fn timer_callback(_instance: *mut c_void, ctx: *mut c_void, _timer: *mut c_void) {
    let _ = (*(ctx as *const Aggregate)).entries();
}

pub unsafe extern "system" fn aggregate_enable_callback(src: *const Guid, enabled: u32, level: u8, any: u64, all: u64, filter: *const c_void, ctx: *mut c_void) {
    let a = ctx as *const Aggregate;
    let user = (*a).callback.load(Ordering::SeqCst);
    if user != 0 {
        let f: EnableCallback = core::mem::transmute(user);
        f(src, enabled, level, any, all, filter, (*a).callback_context.load(Ordering::SeqCst));
    }
}

pub unsafe extern "system" fn internal_enable_callback(_src: *const Guid, enabled: u32, _level: u8, any: u64, _all: u64, _filter: *const c_void, _ctx: *mut c_void) {
    if enabled != 2 || any != 0x20 {
        return;
    }
    if let Ok(list) = LIST.try_lock() {
        for &a in &list.0 {
            let _ = (*a).entries();
        }
    }
}

pub fn attach() -> i32 {
    let a = Aggregate::create();
    if a.is_null() {
        return PROVIDER.register(None, core::ptr::null_mut());
    }
    let obj = unsafe { &*a };
    obj.callback.store(0, Ordering::SeqCst);
    obj.callback_context.store(core::ptr::null_mut(), Ordering::SeqCst);
    obj.period_ms.store(flush_period_ms(a as usize, &PROVIDER.guid), Ordering::SeqCst);
    let hr = PROVIDER.register(Some(aggregate_enable_callback), a as *mut c_void);
    if hr < 0 {
        unsafe { destroy(a) };
        return hr;
    }
    let mut list = LIST.lock().unwrap_or_else(|e| e.into_inner());
    if list.0.is_empty() {
        AGGREGATE_PROVIDER.register(Some(internal_enable_callback), core::ptr::null_mut());
    }
    if !list.0.iter().any(|&n| core::ptr::eq(unsafe { (*n).provider }, &PROVIDER)) {
        list.0.push(a);
    }
    0
}

pub fn detach() {
    if !PROVIDER.callback_is(aggregate_enable_callback) {
        PROVIDER.unregister();
        return;
    }
    let mut node: *mut Aggregate = core::ptr::null_mut();
    {
        let mut list = LIST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = list.0.iter().position(|&n| core::ptr::eq(unsafe { (*n).provider }, &PROVIDER)) {
            node = list.0.remove(i);
            if list.0.is_empty() {
                AGGREGATE_PROVIDER.unregister();
            }
        }
    }
    if !node.is_null() {
        unsafe { (*node).stop_timer() };
    }
    PROVIDER.unregister();
    unsafe { destroy(node) };
}
