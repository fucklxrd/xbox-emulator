use core::ffi::c_void;
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::wstr::from_ptr;

pub const LEGACY_SIZE: usize = 0xc8;
pub const LEGACY_REGISTER_STOP: usize = 0xc0;
pub const ERROR_INVALID_DATA: u32 = 0xd;
pub const ERROR_INVALID_PARAMETER: u32 = 0x57;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_SERVICE_NOT_IN_EXE: u32 = 1083;
pub const SERVICE_STOPPED: u32 = 1;
pub const SERVICE_RUNNING: u32 = 4;

pub type StopCallback = unsafe extern "system" fn(*mut c_void, u8);
pub type HandlerEx = unsafe extern "system" fn(u32, u32, *mut c_void, *mut c_void) -> u32;

#[link(name = "kernel32")]
extern "system" {
    fn RegisterWaitForSingleObjectEx(object: *mut c_void, cb: unsafe extern "system" fn(*mut c_void, u8), ctx: *mut c_void, ms: u32, flags: u32) -> *mut c_void;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
}

#[derive(Clone, Debug, Default)]
pub struct Service {
    pub name: String,
    pub handler: usize,
    pub context: usize,
    pub status: [u32; 7],
    pub history: Vec<[u32; 7]>,
    pub stop_callback: usize,
    pub stop_waits: u32,
}

fn table() -> MutexGuard<'static, Vec<Service>> {
    static T: Mutex<Vec<Service>> = Mutex::new(Vec::new());
    T.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn add_service(name: &str) {
    let mut t = table();
    match t.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name)) {
        Some(s) => {
            s.history.clear();
            s.status = [0; 7];
        }
        None => t.push(Service { name: name.to_string(), ..Service::default() }),
    }
}

pub fn service(name: &str) -> Option<Service> {
    table().iter().find(|s| s.name.eq_ignore_ascii_case(name)).cloned()
}

pub unsafe fn control(name: &str, code: u32) -> Option<u32> {
    let s = service(name)?;
    if s.handler == 0 {
        return None;
    }
    let h: HandlerEx = std::mem::transmute(s.handler);
    Some(h(code, 0, core::ptr::null_mut(), s.context as *mut c_void))
}

pub fn is_status_handle(h: usize) -> bool {
    h != 0 && h <= table().len()
}

pub unsafe fn control_by_handle(h: usize, code: u32, kind: u32, data: *mut c_void) -> Option<u32> {
    let s = table().get(h.checked_sub(1)?).cloned()?;
    if s.handler == 0 {
        return None;
    }
    let f: HandlerEx = std::mem::transmute(s.handler);
    Some(f(code, kind, data, s.context as *mut c_void))
}

struct Wait {
    context: usize,
    service: String,
}

unsafe extern "system" fn stop_thunk(ctx: *mut c_void, timed_out: u8) {
    let w = Box::from_raw(ctx as *mut Wait);
    let cb = {
        let mut t = table();
        match t.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&w.service)) {
            Some(s) => {
                let cb = s.stop_callback;
                s.stop_callback = 0;
                cb
            }
            None => 0,
        }
    };
    if cb != 0 {
        let f: StopCallback = std::mem::transmute(cb);
        f(w.context as *mut c_void, timed_out);
    }
    let mut t = table();
    if let Some(s) = t.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&w.service)) {
        s.stop_waits = s.stop_waits.saturating_sub(1);
    }
}

pub unsafe extern "system" fn register_stop_callback(wait: *mut *mut c_void, name: *const u16, object: *mut c_void, callback: *mut c_void, context: *mut c_void, flags: u32) -> u32 {
    if wait.is_null() || name.is_null() || object.is_null() || callback.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let n = from_ptr(name);
    let mut t = table();
    let Some(s) = t.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&n)) else { return ERROR_INVALID_DATA };
    if s.stop_callback != 0 {
        return ERROR_INVALID_DATA;
    }
    let ctx = Box::into_raw(Box::new(Wait { context: context as usize, service: s.name.clone() }));
    let h = RegisterWaitForSingleObjectEx(object, stop_thunk, ctx as *mut c_void, u32::MAX, flags);
    *wait = h;
    if h.is_null() {
        let e = GetLastError();
        drop(Box::from_raw(ctx));
        return e;
    }
    s.stop_waits += 1;
    s.stop_callback = callback as usize;
    0
}

pub fn legacy_globals() -> *const u8 {
    static G: OnceLock<usize> = OnceLock::new();
    *G.get_or_init(|| {
        let b = Box::into_raw(Box::new([0u8; LEGACY_SIZE])) as *mut u8;
        unsafe { (b.add(LEGACY_REGISTER_STOP) as *mut usize).write_unaligned(register_stop_callback as *const () as usize) };
        b as usize
    }) as *const u8
}

pub unsafe extern "system" fn register_service_ctrl_handler_ex_w(name: *const u16, handler: *mut c_void, context: *mut c_void) -> usize {
    let n = from_ptr(name);
    let mut t = table();
    match t.iter().position(|s| s.name.eq_ignore_ascii_case(&n)) {
        Some(i) => {
            t[i].handler = handler as usize;
            t[i].context = context as usize;
            i + 1
        }
        None => {
            SetLastError(ERROR_SERVICE_NOT_IN_EXE);
            0
        }
    }
}

pub unsafe extern "system" fn set_service_status(h: usize, status: *const u32) -> i32 {
    let mut t = table();
    if h == 0 || h > t.len() || status.is_null() {
        SetLastError(if status.is_null() { ERROR_INVALID_PARAMETER } else { ERROR_INVALID_HANDLE });
        return 0;
    }
    let mut st = [0u32; 7];
    st.copy_from_slice(std::slice::from_raw_parts(status, 7));
    t[h - 1].status = st;
    t[h - 1].history.push(st);
    let name = t[h - 1].name.clone();
    drop(t);
    crate::notify::deliver(&name);
    1
}

pub const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_INVALID_LEVEL: u32 = 124;
pub const ERROR_INVALID_NAME: u32 = 123;
const SCM_HANDLE: usize = 0x5C4D_0000;
const SVC_HANDLE_BASE: usize = 0x5C4E_0000;

fn handles() -> MutexGuard<'static, Vec<String>> {
    static H: Mutex<Vec<String>> = Mutex::new(Vec::new());
    H.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn services_key(name: &str) -> String {
    format!("\\Registry\\Machine\\SYSTEM\\CurrentControlSet\\Services\\{name}")
}

fn known(name: &str) -> bool {
    service(name).is_some() || crate::notify::builtin_running(name) || crate::registry::get(&services_key(name), "Type").is_some()
}

pub fn service_handle_name(h: usize) -> Option<String> {
    handle_name(h)
}

fn handle_name(h: usize) -> Option<String> {
    if h < SVC_HANDLE_BASE + 4 || (h - SVC_HANDLE_BASE) % 4 != 0 {
        return None;
    }
    handles().get((h - SVC_HANDLE_BASE) / 4 - 1).cloned().filter(|n| !n.is_empty())
}

pub unsafe extern "system" fn open_sc_manager_w(machine: *const u16, _db: *const u16, _access: u32) -> usize {
    let m = from_ptr(machine);
    if !m.is_empty() && !m.trim_start_matches('\\').eq_ignore_ascii_case(&crate::system::hostname(0).unwrap_or_default()) {
        SetLastError(ERROR_INVALID_NAME);
        return 0;
    }
    SCM_HANDLE
}

pub unsafe extern "system" fn open_service_w(scm: usize, name: *const u16, _access: u32) -> usize {
    if scm != SCM_HANDLE {
        SetLastError(ERROR_INVALID_HANDLE);
        return 0;
    }
    let n = from_ptr(name);
    if n.is_empty() || n.len() > 256 || n.contains('\\') || n.contains('/') {
        SetLastError(ERROR_INVALID_NAME);
        return 0;
    }
    if !known(&n) {
        SetLastError(ERROR_SERVICE_DOES_NOT_EXIST);
        return 0;
    }
    let mut h = handles();
    h.push(n);
    SVC_HANDLE_BASE + h.len() * 4
}

pub unsafe extern "system" fn close_service_handle(h: usize) -> i32 {
    if h == SCM_HANDLE {
        return 1;
    }
    if handle_name(h).is_some() {
        crate::notify::cancel(h);
        handles()[(h - SVC_HANDLE_BASE) / 4 - 1] = String::new();
        return 1;
    }
    SetLastError(ERROR_INVALID_HANDLE);
    0
}

pub unsafe extern "system" fn query_service_status_ex(h: usize, level: u32, buf: *mut u8, size: u32, needed: *mut u32) -> i32 {
    let Some(n) = handle_name(h) else {
        SetLastError(ERROR_INVALID_HANDLE);
        return 0;
    };
    if level != 0 {
        SetLastError(ERROR_INVALID_LEVEL);
        return 0;
    }
    if !needed.is_null() {
        *needed = 36;
    }
    if buf.is_null() || size < 36 {
        SetLastError(ERROR_INSUFFICIENT_BUFFER);
        return 0;
    }
    let st = crate::notify::status_of(&n);
    let words: [u32; 9] = [st[0], st[1], st[2], st[3], st[4], st[5], st[6], crate::notify::pid_of(&st), 0];
    core::ptr::copy_nonoverlapping(words.as_ptr() as *const u8, buf, 36);
    1
}

fn reg_dword(key: &str, name: &str, default: u32) -> u32 {
    crate::registry::get(key, name).filter(|v| v.data.len() >= 4).map(|v| u32::from_le_bytes(v.data[..4].try_into().unwrap())).unwrap_or(default)
}

fn reg_wide(key: &str, name: &str, multi: bool) -> Vec<u16> {
    let mut w: Vec<u16> = crate::registry::get(key, name).map(|v| v.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()).unwrap_or_default();
    while w.last() == Some(&0) {
        w.pop();
    }
    w.push(0);
    if multi {
        w.push(0);
    }
    w
}

pub unsafe extern "system" fn query_service_config_w(h: usize, buf: *mut u8, size: u32, needed: *mut u32) -> i32 {
    let Some(n) = handle_name(h) else {
        SetLastError(ERROR_INVALID_HANDLE);
        return 0;
    };
    let k = services_key(&n);
    let strings = [reg_wide(&k, "ImagePath", false), reg_wide(&k, "Group", false), reg_wide(&k, "DependOnService", true), reg_wide(&k, "ObjectName", false), {
        let d = reg_wide(&k, "DisplayName", false);
        if d.len() > 1 {
            d
        } else {
            n.encode_utf16().chain(Some(0)).collect()
        }
    }];
    let head = 0x40usize;
    let total = head + strings.iter().map(|s| s.len() * 2).sum::<usize>();
    if !needed.is_null() {
        *needed = total as u32;
    }
    if buf.is_null() || (size as usize) < total {
        SetLastError(ERROR_INSUFFICIENT_BUFFER);
        return 0;
    }
    core::ptr::write_bytes(buf, 0, total);
    let w32 = |off: usize, v: u32| (buf.add(off) as *mut u32).write_unaligned(v);
    w32(0, reg_dword(&k, "Type", 0x20));
    w32(4, reg_dword(&k, "Start", 3));
    w32(8, reg_dword(&k, "ErrorControl", 1));
    w32(0x20, reg_dword(&k, "Tag", 0));
    let mut off = head;
    for (slot, s) in [0x10usize, 0x18, 0x28, 0x30, 0x38].iter().zip(strings.iter()) {
        let p = buf.add(off);
        core::ptr::copy_nonoverlapping(s.as_ptr() as *const u8, p, s.len() * 2);
        (buf.add(*slot) as *mut usize).write_unaligned(p as usize);
        off += s.len() * 2;
    }
    1
}