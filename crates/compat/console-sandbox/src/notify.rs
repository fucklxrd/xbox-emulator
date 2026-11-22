use core::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

use crate::host::{self, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, SERVICE_RUNNING, SERVICE_STOPPED};

pub const SERVICE_NOTIFY_STATUS_CHANGE: u32 = 2;
pub const ERROR_SUCCESS: u32 = 0;
pub const RPCSS: &str = "RpcSs";

#[link(name = "kernel32")]
extern "system" {
    fn QueueUserAPC(f: usize, thread: usize, data: usize) -> u32;
    fn GetCurrentThread() -> usize;
    fn GetCurrentProcess() -> usize;
    fn DuplicateHandle(sp: usize, sh: usize, tp: usize, th: *mut usize, access: u32, inherit: i32, opts: u32) -> i32;
    fn CloseHandle(h: usize) -> i32;
    fn SetLastError(e: u32);
    fn GetLastError() -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn MultiByteToWideChar(cp: u32, flags: u32, s: *const u8, n: i32, w: *mut u16, cch: i32) -> i32;
}

struct Pending {
    handle: usize,
    service: String,
    mask: u32,
    buffer: usize,
    thread: usize,
}

fn pending() -> MutexGuard<'static, Vec<Pending>> {
    static P: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
    P.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn builtin_running(name: &str) -> bool {
    name.eq_ignore_ascii_case(RPCSS)
}

pub fn status_of(name: &str) -> [u32; 7] {
    if let Some(s) = host::service(name).filter(|s| s.status[1] != 0) {
        return s.status;
    }
    if builtin_running(name) {
        return [0x20, SERVICE_RUNNING, 0, 0, 0, 0, 0];
    }
    [0x10, SERVICE_STOPPED, 0, 1077, 0, 0, 0]
}

pub fn pid_of(st: &[u32; 7]) -> u32 {
    if st[1] == SERVICE_STOPPED {
        0
    } else {
        std::process::id()
    }
}

unsafe fn ansi(p: *const u8) -> Option<Vec<u16>> {
    if p.is_null() {
        return Some(vec![0]);
    }
    let n = MultiByteToWideChar(0, 0, p, -1, core::ptr::null_mut(), 0);
    if n <= 0 {
        return None;
    }
    let mut w = vec![0u16; n as usize];
    MultiByteToWideChar(0, 0, p, -1, w.as_mut_ptr(), n);
    Some(w)
}

pub unsafe extern "system" fn open_sc_manager_a(machine: *const u8, db: *const u8, access: u32) -> usize {
    let (Some(m), Some(d)) = (ansi(machine), ansi(db)) else {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    };
    host::open_sc_manager_w(if machine.is_null() { core::ptr::null() } else { m.as_ptr() }, if db.is_null() { core::ptr::null() } else { d.as_ptr() }, access)
}

pub unsafe extern "system" fn open_service_a(scm: usize, name: *const u8, access: u32) -> usize {
    let Some(n) = ansi(name) else {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    };
    host::open_service_w(scm, if name.is_null() { core::ptr::null() } else { n.as_ptr() }, access)
}

fn bit(state: u32) -> u32 {
    if (1..=7).contains(&state) {
        1 << (state - 1)
    } else {
        0
    }
}

unsafe fn fire(buffer: usize, thread: usize, st: &[u32; 7], triggered: u32) -> bool {
    let b = buffer as *mut u8;
    (b.add(0x18) as *mut u32).write_unaligned(ERROR_SUCCESS);
    let words: [u32; 9] = [st[0], st[1], st[2], st[3], st[4], st[5], st[6], pid_of(st), 0];
    core::ptr::copy_nonoverlapping(words.as_ptr() as *const u8, b.add(0x1c), 36);
    (b.add(0x40) as *mut u32).write_unaligned(triggered);
    (b.add(0x48) as *mut usize).write_unaligned(0);
    let cb = (b.add(8) as *const usize).read_unaligned();
    QueueUserAPC(cb, thread, buffer) != 0
}

pub unsafe extern "system" fn notify_service_status_change(service: usize, mask: u32, buffer: *mut c_void) -> u32 {
    let Some(name) = host::service_handle_name(service) else { return ERROR_INVALID_HANDLE };
    if buffer.is_null() || mask == 0 || mask & !0x7f != 0 {
        return ERROR_INVALID_PARAMETER;
    }
    let b = buffer as *const u8;
    if (b as *const u32).read_unaligned() != SERVICE_NOTIFY_STATUS_CHANGE || (b.add(8) as *const usize).read_unaligned() == 0 {
        return ERROR_INVALID_PARAMETER;
    }
    let mut thread = 0usize;
    if DuplicateHandle(GetCurrentProcess(), GetCurrentThread(), GetCurrentProcess(), &mut thread, 0, 0, 2) == 0 {
        return GetLastError();
    }
    let st = status_of(&name);
    if bit(st[1]) & mask != 0 {
        let ok = fire(buffer as usize, thread, &st, bit(st[1]));
        CloseHandle(thread);
        return if ok { ERROR_SUCCESS } else { GetLastError() };
    }
    pending().push(Pending { handle: service, service: name, mask, buffer: buffer as usize, thread });
    ERROR_SUCCESS
}

pub fn deliver(name: &str) {
    let st = status_of(name);
    let b = bit(st[1]);
    let fired: Vec<Pending> = {
        let mut p = pending();
        let (hit, keep): (Vec<Pending>, Vec<Pending>) = p.drain(..).partition(|x| x.service.eq_ignore_ascii_case(name) && x.mask & b != 0);
        *p = keep;
        hit
    };
    for f in fired {
        unsafe {
            fire(f.buffer, f.thread, &st, b);
            CloseHandle(f.thread);
        }
    }
}

pub fn cancel(handle: usize) {
    let mut p = pending();
    p.retain(|x| {
        if x.handle == handle {
            unsafe { CloseHandle(x.thread) };
            false
        } else {
            true
        }
    });
}
