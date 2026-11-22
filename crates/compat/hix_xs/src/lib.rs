pub mod console;
pub mod hang_stream;
pub mod kmd;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

pub const E_FAIL: i32 = 0x8000_4005u32 as i32;
pub const STREAMING_OBJECT_SIZE: usize = 0x2dc;
pub const BLOB_SLOTS: u32 = 0x40;
pub const BLOB_COUNT: usize = 0x1850;
pub const BLOB_PTRS: usize = 0x1858;
pub const BLOB_SIZES: usize = 0x1a58;
pub const REGION1_SIZE: usize = 0x1b58;

static REGION1: AtomicUsize = AtomicUsize::new(0);
static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static DATA_IDS: Mutex<Option<HashMap<u64, u32>>> = Mutex::new(None);
static STREAMING: Mutex<Vec<[u8; STREAMING_OBJECT_SIZE]>> = Mutex::new(Vec::new());

pub fn region1() -> usize {
    REGION1.load(Ordering::SeqCst)
}

pub unsafe fn set_region1(p: *mut u8) {
    REGION1.store(p as usize, Ordering::SeqCst)
}

pub fn streaming_objects() -> Vec<[u8; STREAMING_OBJECT_SIZE]> {
    STREAMING.lock().unwrap().clone()
}

pub fn next_data_id() -> u32 {
    NEXT_ID.load(Ordering::SeqCst)
}

fn assign_id(out: *mut u32) -> u32 {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    unsafe { *out = id };
    id
}

#[no_mangle]
pub unsafe extern "system" fn SaveDeveloperBlobInfo(ptr: u64, size: u32) -> i32 {
    let r = REGION1.load(Ordering::SeqCst) as *mut u8;
    if r.is_null() {
        return E_FAIL;
    }
    let count = r.add(BLOB_COUNT) as *mut u32;
    let n = count.read_unaligned();
    if n >= BLOB_SLOTS {
        return 1;
    }
    (r.add(BLOB_PTRS + n as usize * 8) as *mut u64).write_unaligned(ptr);
    (r.add(BLOB_SIZES + n as usize * 4) as *mut u32).write_unaligned(size);
    count.write_unaligned(n + 1);
    0
}

#[no_mangle]
pub unsafe extern "system" fn AddStreamingApiObject(obj: *const u8) -> i32 {
    let mut o = [0u8; STREAMING_OBJECT_SIZE];
    core::ptr::copy_nonoverlapping(obj, o.as_mut_ptr(), STREAMING_OBJECT_SIZE);
    STREAMING.lock().unwrap().push(o);
    0
}

#[no_mangle]
pub unsafe extern "system" fn AddData(key: *const u64, _data: *const u8, _size: u32, out: *mut u32) -> i32 {
    if key.is_null() {
        assign_id(out);
        return 0;
    }
    let k = key.read_unaligned();
    let mut g = DATA_IDS.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    if let Some(&id) = m.get(&k) {
        *out = id;
        return 0;
    }
    let id = assign_id(out);
    m.insert(k, id);
    0
}

#[no_mangle]
pub unsafe extern "system" fn InitializeHix(log: usize) -> i32 {
    match console::proc("InitializeHix") {
        Some(f) => {
            let f: unsafe extern "system" fn(usize) -> i32 = core::mem::transmute(f);
            f(log)
        }
        None => E_FAIL,
    }
}

#[no_mangle]
pub unsafe extern "system" fn TerminateHix() -> i32 {
    match console::proc("TerminateHix") {
        Some(f) => {
            let f: unsafe extern "system" fn() -> i32 = core::mem::transmute(f);
            f()
        }
        None => E_FAIL,
    }
}

#[no_mangle]
pub unsafe extern "system" fn CollectHangSummary(ctx: *const u8, report: *mut u8) -> i32 {
    match console::proc("CollectHangSummary") {
        Some(f) => {
            let f: unsafe extern "system" fn(*const u8, *mut u8) -> i32 = core::mem::transmute(f);
            f(ctx, report)
        }
        None => E_FAIL,
    }
}

#[no_mangle]
pub unsafe extern "system" fn WriteHangDump(flags: u32, out: *mut u16, cch: u32, summary: *const u8) -> i32 {
    match console::proc("WriteHangDump") {
        Some(f) => {
            let f: unsafe extern "system" fn(u32, *mut u16, u32, *const u8) -> i32 = core::mem::transmute(f);
            f(flags, out, cch, summary)
        }
        None => E_FAIL,
    }
}

#[no_mangle]
pub unsafe extern "system" fn DebugLive(arg: u32, a2: *const u8, a3: *mut u8) -> i32 {
    match console::proc("DebugLive") {
        Some(f) => {
            let f: unsafe extern "system" fn(u32, *const u8, *mut u8) -> i32 = core::mem::transmute(f);
            f(arg, a2, a3)
        }
        None => E_FAIL,
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_hang;
#[cfg(test)]
mod tests_kmd;
#[cfg(test)]
mod tests_dump;
