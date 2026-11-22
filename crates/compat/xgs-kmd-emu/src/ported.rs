use crate::{accessible, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, ERROR_NOACCESS, ERROR_NOT_ENOUGH_MEMORY};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

pub const IOCTL_UNSUPPORTED_1AF: u32 = 0x7900_c1af;
pub const IOCTL_SET_PROCESS_OPTION: u32 = 0x7900_c19c;
pub const IOCTL_CLEAR_FAULT_HOLD: u32 = 0x7900_c227;
pub const IOCTL_TRACKED_KEY: u32 = 0x7900_c22f;
pub const IOCTL_QUERY_PAGE_POOL: u32 = 0x7900_c26f;
pub const IOCTL_LAST_GPU_ERROR: u32 = 0x7900_c27f;
pub const IOCTL_MEMORY_PROFILING: u32 = 0x7900_c1cc;
pub const STATUS_ACCESS_VIOLATION: u32 = 0xc000_0005;
pub const GPU_ERROR_MESSAGE_BYTES: usize = 0x100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessOptions {
    pub option_2: bool,
    pub option_105: bool,
    pub option_10001: bool,
    pub option_e: u32,
    pub fault_hold: bool,
    pub host_posts: HashMap<u32, u32>,
}

impl Default for ProcessOptions {
    fn default() -> Self {
        ProcessOptions { option_2: true, option_105: false, option_10001: false, option_e: 2, fault_hold: false, host_posts: HashMap::new() }
    }
}

struct State {
    options: ProcessOptions,
    tracked: Vec<u64>,
    adapter_baaad911: Option<u32>,
    gpu_error: (u32, String),
    memory_profiling: Option<[u64; 3]>,
}

fn state() -> MutexGuard<'static, State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { options: ProcessOptions::default(), tracked: Vec::new(), adapter_baaad911: None, gpu_error: (0, String::new()), memory_profiling: None })).lock().unwrap_or_else(|e| e.into_inner())
}

static ENTRY: AtomicBool = AtomicBool::new(false);

pub fn create_process_entry() {
    ENTRY.store(true, Ordering::Release);
}

pub fn process_entry_exists() -> bool {
    ENTRY.load(Ordering::Acquire)
}

pub fn process_options() -> ProcessOptions {
    state().options.clone()
}

pub fn adapter_option_baaad911() -> Option<u32> {
    state().adapter_baaad911
}

pub fn tracked_keys() -> Vec<u64> {
    state().tracked.clone()
}

pub fn set_gpu_error(code: u32, message: &str) {
    let mut m = message.to_string();
    while m.len() > GPU_ERROR_MESSAGE_BYTES - 1 {
        m.pop();
    }
    state().gpu_error = (code, m);
}

pub fn gpu_error() -> (u32, String) {
    state().gpu_error.clone()
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn put_u64(addr: u64, align: u64, v: u64) -> Result<(), u32> {
    if addr & align != 0 || !accessible(addr, 8, true) {
        return Err(ERROR_NOACCESS);
    }
    unsafe { core::ptr::write_unaligned(addr as usize as *mut u64, v) };
    Ok(())
}

fn put_u32(addr: u64, v: u32) -> Result<(), u32> {
    if addr & 3 != 0 || !accessible(addr, 4, true) {
        return Err(ERROR_NOACCESS);
    }
    unsafe { core::ptr::write_unaligned(addr as usize as *mut u32, v) };
    Ok(())
}

fn set_process_option(input: &[u8]) -> Result<u32, u32> {
    if input.len() < 8 {
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    create_process_entry();
    let key = u32_at(input, 0);
    let value = u32_at(input, 4);
    let mut s = state();
    match key {
        2 => s.options.option_2 = value != 0,
        0xe => s.options.option_e = if value == 1 { 2 } else { 0 },
        0x105 => s.options.option_105 = value != 0,
        0x10001 => s.options.option_10001 = value != 0,
        0x10002 | 0x10005 => {}
        0xbaaa_d911 => s.adapter_baaad911 = Some(value),
        0xdead_dead => s.options.fault_hold = true,
        _ => {
            s.options.host_posts.insert(key, value);
        }
    }
    Ok(0)
}

fn clear_fault_hold() -> Result<u32, u32> {
    if !process_entry_exists() {
        return Err(ERROR_NOT_ENOUGH_MEMORY);
    }
    state().options.fault_hold = false;
    Ok(0)
}

fn tracked_key(input: &[u8]) -> Result<u32, u32> {
    let key = u64_at(input, 0);
    let op = u64_at(input, 8);
    let out = u64_at(input, 16);
    if !process_entry_exists() {
        return Err(ERROR_NOT_ENOUGH_MEMORY);
    }
    if op != 0 {
        if out == 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        if key != 0 {
            state().tracked.push(key);
        }
        put_u64(out, 3, key)?;
        return if key == 0 { Err(ERROR_INVALID_PARAMETER) } else { Ok(0) };
    }
    if key == 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    {
        let mut s = state();
        match s.tracked.iter().position(|&k| k == key) {
            Some(i) => {
                s.tracked.remove(i);
            }
            None => return Err(ERROR_INVALID_PARAMETER),
        }
    }
    if out != 0 {
        put_u64(out, 3, 0)?;
    }
    Ok(0)
}

fn query_page_pool(input: &[u8]) -> Result<u32, u32> {
    let handle = u64_at(input, 0);
    let out_va = u64_at(input, 8);
    let out_pages = u64_at(input, 16);
    if handle == 0 || out_va == 0 || out_pages == 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if !process_entry_exists() {
        return Err(ERROR_NOT_ENOUGH_MEMORY);
    }
    let Some((va, pages)) = crate::page_pool(handle) else { return Err(ERROR_INVALID_PARAMETER) };
    put_u64(out_va, 7, va)?;
    put_u32(out_pages, pages)?;
    Ok(0)
}

fn last_gpu_error(input: &mut [u8]) -> Result<u32, u32> {
    let buf = u64_at(input, 0);
    let len = u32_at(input, 8);
    let (code, msg) = gpu_error();
    let mut status = code;
    if code != 0 && len != 0 {
        if !accessible(buf, len as u64, true) {
            status = STATUS_ACCESS_VIOLATION;
        } else {
            let dst = unsafe { core::slice::from_raw_parts_mut(buf as usize as *mut u8, len as usize) };
            if msg.len() < dst.len() {
                dst[..msg.len()].copy_from_slice(msg.as_bytes());
                dst[msg.len()] = 0;
            } else {
                dst[0] = 0;
            }
        }
    }
    if !accessible(input.as_ptr() as u64 + 12, 4, true) {
        return Err(ERROR_NOACCESS);
    }
    input[12..16].copy_from_slice(&status.to_le_bytes());
    Ok(0)
}

fn memory_profiling_ioctl(input: &[u8]) -> Result<u32, u32> {
    if input.len() < 0x18 {
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    create_process_entry();
    state().memory_profiling = Some([u64_at(input, 0), u64_at(input, 8), u64_at(input, 16)]);
    Ok(0)
}

pub fn memory_profiling() -> Option<[u64; 3]> {
    state().memory_profiling
}

pub fn dispatch(code: u32, input: &mut [u8], output: &mut [u8]) -> Option<Result<u32, u32>> {
    let _ = output;
    let need = neither_input_len(code).unwrap_or(0);
    if input.len() < need {
        return Some(Err(ERROR_INVALID_PARAMETER));
    }
    Some(match code {
        IOCTL_UNSUPPORTED_1AF => Err(ERROR_NOT_ENOUGH_MEMORY),
        IOCTL_SET_PROCESS_OPTION => set_process_option(input),
        IOCTL_CLEAR_FAULT_HOLD => clear_fault_hold(),
        IOCTL_TRACKED_KEY => tracked_key(input),
        IOCTL_QUERY_PAGE_POOL => query_page_pool(input),
        IOCTL_LAST_GPU_ERROR => last_gpu_error(input),
        IOCTL_MEMORY_PROFILING => memory_profiling_ioctl(input),
        _ => return None,
    })
}

pub fn neither_input_len(code: u32) -> Option<usize> {
    match code {
        IOCTL_UNSUPPORTED_1AF | IOCTL_TRACKED_KEY | IOCTL_QUERY_PAGE_POOL => Some(0x18),
        IOCTL_LAST_GPU_ERROR => Some(0x10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{device_io_control, open, close, test_serial, IOCTL_PAGE_POOL};

    extern "system" {
        fn GetLastError() -> u32;
    }

    fn req3(a: u64, b: u64, c: u64) -> [u8; 0x18] {
        let mut r = [0u8; 0x18];
        r[0..8].copy_from_slice(&a.to_le_bytes());
        r[8..16].copy_from_slice(&b.to_le_bytes());
        r[16..24].copy_from_slice(&c.to_le_bytes());
        r
    }

    fn ioctl(h: isize, code: u32, inb: &mut [u8]) -> Result<(), u32> {
        let mut ret = 0u32;
        if unsafe { device_io_control(h, code, inb.as_mut_ptr().cast(), inb.len() as u32, core::ptr::null_mut(), 0, &mut ret) } {
            assert_eq!(ret, 0);
            Ok(())
        } else {
            Err(unsafe { GetLastError() })
        }
    }

    #[test]
    fn c1af_always_fails_no_memory_after_probe() {
        let _g = test_serial();
        let h = open();
        assert_eq!(ioctl(h, IOCTL_UNSUPPORTED_1AF, &mut [0u8; 0x18]), Err(ERROR_NOT_ENOUGH_MEMORY));
        let mut ret = 0u32;
        assert!(!unsafe { device_io_control(h, IOCTL_UNSUPPORTED_1AF, core::ptr::null(), 0, core::ptr::null_mut(), 0, &mut ret) });
        assert_eq!(unsafe { GetLastError() }, ERROR_INVALID_PARAMETER);
        assert!(close(h));
    }

    #[test]
    fn c19c_sets_process_options_like_kernel() {
        let _g = test_serial();
        let mut out = [0u8; 0];
        assert_eq!(dispatch(IOCTL_SET_PROCESS_OPTION, &mut [0u8; 7], &mut out), Some(Err(ERROR_INSUFFICIENT_BUFFER)));
        let set = |k: u32, v: u32| {
            let mut b = [0u8; 8];
            b[0..4].copy_from_slice(&k.to_le_bytes());
            b[4..8].copy_from_slice(&v.to_le_bytes());
            dispatch(IOCTL_SET_PROCESS_OPTION, &mut b, &mut [])
        };
        assert_eq!(set(2, 0), Some(Ok(0)));
        assert!(!process_options().option_2);
        assert_eq!(set(2, 5), Some(Ok(0)));
        assert!(process_options().option_2);
        assert_eq!(set(0xe, 1), Some(Ok(0)));
        assert_eq!(process_options().option_e, 2);
        assert_eq!(set(0xe, 3), Some(Ok(0)));
        assert_eq!(process_options().option_e, 0);
        assert_eq!(set(0x105, 1), Some(Ok(0)));
        assert!(process_options().option_105);
        assert_eq!(set(0x10001, 1), Some(Ok(0)));
        assert!(process_options().option_10001);
        assert_eq!(set(0x10005, 9), Some(Ok(0)));
        assert_eq!(set(0xbaaa_d911, 0x1234), Some(Ok(0)));
        assert_eq!(adapter_option_baaad911(), Some(0x1234));
        assert_eq!(set(0x77, 3), Some(Ok(0)));
        assert_eq!(process_options().host_posts.get(&0x77), Some(&3));
        assert_eq!(set(0xdead_dead, 0), Some(Ok(0)));
        assert!(process_options().fault_hold);
        assert!(process_entry_exists());
    }

    #[test]
    fn c227_clears_fault_hold() {
        let _g = test_serial();
        let mut b = [0u8; 8];
        b[0..4].copy_from_slice(&0xdead_deadu32.to_le_bytes());
        assert_eq!(dispatch(IOCTL_SET_PROCESS_OPTION, &mut b, &mut []), Some(Ok(0)));
        assert!(process_options().fault_hold);
        assert_eq!(dispatch(IOCTL_CLEAR_FAULT_HOLD, &mut [], &mut []), Some(Ok(0)));
        assert!(!process_options().fault_hold);
        assert_eq!(dispatch(IOCTL_CLEAR_FAULT_HOLD, &mut [], &mut []), Some(Ok(0)));
    }

    #[test]
    fn c22f_tracks_keys_as_multiset() {
        let _g = test_serial();
        create_process_entry();
        let h = open();
        let mut out = 0x55u64;
        let o = &mut out as *mut u64 as u64;
        assert_eq!(ioctl(h, IOCTL_TRACKED_KEY, &mut req3(0xabc0, 1, 0)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(ioctl(h, IOCTL_TRACKED_KEY, &mut req3(0xabc0, 1, o)), Ok(()));
        assert_eq!(out, 0xabc0);
        assert_eq!(ioctl(h, IOCTL_TRACKED_KEY, &mut req3(0xabc0, 1, o)), Ok(()));
        assert_eq!(tracked_keys().iter().filter(|&&k| k == 0xabc0).count(), 2);
        assert_eq!(ioctl(h, IOCTL_TRACKED_KEY, &mut req3(0, 1, o)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(out, 0);
        assert_eq!(ioctl(h, IOCTL_TRACKED_KEY, &mut req3(0xabc0, 1, o + 1)), Err(ERROR_NOACCESS));
        assert_eq!(tracked_keys().iter().filter(|&&k| k == 0xabc0).count(), 3);
        out = 0x99;
        assert_eq!(ioctl(h, IOCTL_TRACKED_KEY, &mut req3(0xabc0, 0, o)), Ok(()));
        assert_eq!(out, 0);
        assert_eq!(ioctl(h, IOCTL_TRACKED_KEY, &mut req3(0xabc0, 0, 0)), Ok(()));
        assert_eq!(ioctl(h, IOCTL_TRACKED_KEY, &mut req3(0xabc0, 0, 0)), Ok(()));
        assert_eq!(ioctl(h, IOCTL_TRACKED_KEY, &mut req3(0xabc0, 0, 0)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(ioctl(h, IOCTL_TRACKED_KEY, &mut req3(0, 0, o)), Err(ERROR_INVALID_PARAMETER));
        assert!(close(h));
    }

    #[test]
    fn c26f_queries_registered_page_pool() {
        let _g = test_serial();
        create_process_entry();
        let h = open();
        let mem = vec![0u8; 0x3_0000];
        let va = ((mem.as_ptr() as u64) + 0xffff) & !0xffff;
        let mut handle = 0u64;
        let mut reg = [0u8; 0x18];
        reg[0..8].copy_from_slice(&va.to_le_bytes());
        reg[8..12].copy_from_slice(&2u32.to_le_bytes());
        reg[16..24].copy_from_slice(&(&mut handle as *mut u64 as u64).to_le_bytes());
        assert_eq!(ioctl(h, IOCTL_PAGE_POOL, &mut reg), Ok(()));
        let mut qva = 0u64;
        let mut qpages = [0u32; 2];
        let pv = &mut qva as *mut u64 as u64;
        let pp = qpages.as_mut_ptr() as u64;
        assert_eq!(ioctl(h, IOCTL_QUERY_PAGE_POOL, &mut req3(handle, pv, pp)), Ok(()));
        assert_eq!((qva, qpages[0]), (va, 2));
        assert_eq!(ioctl(h, IOCTL_QUERY_PAGE_POOL, &mut req3(handle, 0, pp)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(ioctl(h, IOCTL_QUERY_PAGE_POOL, &mut req3(handle + 0x1000, pv, pp)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(ioctl(h, IOCTL_QUERY_PAGE_POOL, &mut req3(handle, pv + 4, pp)), Err(ERROR_NOACCESS));
        assert_eq!(ioctl(h, IOCTL_QUERY_PAGE_POOL, &mut req3(handle, pv, pp + 2)), Err(ERROR_NOACCESS));
        let mut un = req3(0, 0, &mut handle as *mut u64 as u64);
        assert_eq!(ioctl(h, IOCTL_PAGE_POOL, &mut un), Ok(()));
        assert_eq!(ioctl(h, IOCTL_QUERY_PAGE_POOL, &mut req3(handle, pv, pp)), Err(ERROR_INVALID_PARAMETER));
        assert!(close(h));
        drop(mem);
    }

    #[test]
    fn c1cc_stores_memory_profiling_config() {
        let _g = test_serial();
        assert_eq!(dispatch(IOCTL_MEMORY_PROFILING, &mut [0u8; 0x17], &mut []), Some(Err(ERROR_INSUFFICIENT_BUFFER)));
        let mut b = req3(0x11, 0x22, 0x33);
        assert_eq!(dispatch(IOCTL_MEMORY_PROFILING, &mut b, &mut []), Some(Ok(0)));
        assert_eq!(memory_profiling(), Some([0x11, 0x22, 0x33]));
        assert!(process_entry_exists());
    }

    #[test]
    fn c27f_reports_last_gpu_error() {
        let _g = test_serial();
        let h = open();
        let mut buf = [0xccu8; 32];
        let mut req = [0u8; 0x10];
        req[0..8].copy_from_slice(&(buf.as_mut_ptr() as u64).to_le_bytes());
        req[8..12].copy_from_slice(&32u32.to_le_bytes());
        req[12..16].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        set_gpu_error(0, "");
        assert_eq!(ioctl(h, IOCTL_LAST_GPU_ERROR, &mut req), Ok(()));
        assert_eq!(u32::from_le_bytes(req[12..16].try_into().unwrap()), 0);
        assert_eq!(buf, [0xcc; 32]);
        set_gpu_error(0x8007_0005, "gpu hang ring 3");
        assert_eq!(ioctl(h, IOCTL_LAST_GPU_ERROR, &mut req), Ok(()));
        assert_eq!(u32::from_le_bytes(req[12..16].try_into().unwrap()), 0x8007_0005);
        assert_eq!(&buf[..16], b"gpu hang ring 3\0");
        req[8..12].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(ioctl(h, IOCTL_LAST_GPU_ERROR, &mut req), Ok(()));
        assert_eq!(buf[0], 0);
        req[0..8].copy_from_slice(&0x10u64.to_le_bytes());
        assert_eq!(ioctl(h, IOCTL_LAST_GPU_ERROR, &mut req), Ok(()));
        assert_eq!(u32::from_le_bytes(req[12..16].try_into().unwrap()), STATUS_ACCESS_VIOLATION);
        set_gpu_error(0, "");
        assert!(close(h));
    }
}
