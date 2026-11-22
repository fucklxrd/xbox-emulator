use crate::com::*;
use crate::json::*;
use core::ffi::c_void;
use core::ptr::null_mut;

pub const XBTP_PORT_SYSTEM: u32 = 0;
pub const XBTP_PORT_TITLE: u32 = 1;
pub const XBTP_PORT_HOST: u32 = 3;
pub const E_POINTER: HRESULT = 0x8000_4003u32 as i32;
pub const E_FAIL: HRESULT = 0x8000_4005u32 as i32;
pub const E_NO_IPADDRESS_IN_CONNSTR: HRESULT = 0x8c11_0407u32 as i32;
pub const E_INVALID_VM_TARGET: HRESULT = 0x8c11_0408u32 as i32;
pub const E_TRANSPORT_TIMEOUT: HRESULT = 0x8c11_0410u32 as i32;
pub const E_COULD_NOT_CONNECT: HRESULT = 0x8c11_0303u32 as i32;
pub const E_RETRY_STOP_A: HRESULT = 0x8c11_040bu32 as i32;
pub const E_RETRY_STOP_B: HRESULT = 0x8c11_040du32 as i32;
pub const E_RETRY_STOP_C: HRESULT = 0x8c11_040fu32 as i32;
pub const DEFAULT_OPEN_SESSION_TIMEOUT_MS: u32 = 30000;
pub const DEFAULT_RECEIVE_TIMEOUT_MS: u32 = 30000;
pub const MAX_ADDRESS_CHARS: usize = 0x200;
pub const SESSION_KEY_CHARS: usize = 0x20;
pub const IID_XBTP_CLIENT_SESSION: GUID = GUID::new(0xcf84f553, 0xcd9b, 0x4608, [0x96, 0x40, 0x28, 0x71, 0x4e, 0x5e, 0xd4, 0x40]);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OpenSessionOptions {
    pub size: u32,
    pub session_key: [u16; SESSION_KEY_CHARS],
    pub skip_ready_check: u32,
    pub allow_unneeded_key: u32,
    pub quiet: u32,
    pub reserved: u32,
    pub timeout_ms: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
extern "system" {
    fn Sleep(ms: u32);
}

#[link(name = "oleaut32", kind = "raw-dylib")]
extern "system" {
    fn SysAllocStringLen(s: *const u16, n: u32) -> BSTR;
}

pub unsafe fn wlen(p: *const u16) -> usize {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    n
}

pub unsafe fn address_length(conn: *const u16) -> Option<usize> {
    if conn.is_null() {
        return None;
    }
    let n = wlen(conn);
    let s = core::slice::from_raw_parts(conn, n);
    let len = match s.iter().position(|&c| c == b'/' as u16) {
        Some(i) => i,
        None if n == 0 => 0,
        None if n < MAX_ADDRESS_CHARS => n,
        None => 0,
    };
    if len != 0 {
        Some(len)
    } else {
        None
    }
}

pub unsafe fn bstr_set(out: &mut Bstr, src: *const u16, count: i64) -> HRESULT {
    let mut n = count;
    let mut b: BSTR = null_mut();
    if n == -1 {
        n = if src.is_null() || *src == 0 { 0 } else { wlen(src) as i64 };
    }
    if n != 0 && !src.is_null() {
        if n < 0 || n > u32::MAX as i64 || (n as u32) > 0x7fff_ffff {
            return E_OUTOFMEMORY;
        }
        b = SysAllocStringLen(src, n as u32);
        if b.is_null() {
            return E_OUTOFMEMORY;
        }
    }
    out.reset();
    out.0 = b;
    S_OK
}

fn eq_nocase(s: &[u16], word: &str) -> bool {
    s.len() == word.len() && s.iter().zip(word.bytes()).all(|(&c, b)| c < 0x80 && (c as u8).to_ascii_lowercase() == b)
}

pub unsafe fn split_console_address(conn: *const u16, default_port: u32, ip: *mut BSTR, key: *mut BSTR, port: *mut u32) -> HRESULT {
    if conn.is_null() || ip.is_null() || port.is_null() {
        return E_INVALIDARG;
    }
    let len = match address_length(conn) {
        Some(l) => l,
        None => return E_NO_IPADDRESS_IN_CONNSTR,
    };
    let mut target = default_port;
    let sep = *conn.add(len);
    if sep == b'\\' as u16 || sep == b'/' as u16 {
        let tail = conn.add(len + 1);
        let t = core::slice::from_raw_parts(tail, wlen(tail));
        target = if eq_nocase(t, "system") {
            XBTP_PORT_SYSTEM
        } else if eq_nocase(t, "title") {
            XBTP_PORT_TITLE
        } else if eq_nocase(t, "host") {
            XBTP_PORT_HOST
        } else {
            return E_INVALID_VM_TARGET;
        };
    }
    *port = target;
    if !key.is_null() {
        *key = null_mut();
        let n = wlen(conn);
        let s = core::slice::from_raw_parts(conn, n);
        if let Some(plus) = s.iter().rposition(|&c| c == b'+' as u16) {
            let key_len = (len as u32).wrapping_sub(plus as u32).wrapping_sub(1);
            let mut k = Bstr::null();
            let hr = bstr_set(&mut k, conn.add(plus + 1), key_len as i64);
            if hr < 0 {
                return hr;
            }
            let mut a = Bstr::null();
            let hr = bstr_set(&mut a, conn, plus as u32 as i64);
            if hr < 0 {
                return hr;
            }
            *key = core::mem::replace(&mut k.0, null_mut());
            *ip = core::mem::replace(&mut a.0, null_mut());
            return S_OK;
        }
    }
    let mut a = Bstr::null();
    let hr = bstr_set(&mut a, conn, len as i64);
    if hr < 0 {
        return hr;
    }
    *ip = core::mem::replace(&mut a.0, null_mut());
    S_OK
}

pub unsafe fn split_address_port(conn: *const u16, default_port: u32, ip: &mut Bstr, port: &mut u32) -> HRESULT {
    ip.reset();
    split_console_address(conn, default_port, &mut ip.0, null_mut(), port)
}

pub unsafe fn get_raw_address(conn: *const u16, raw: &mut Bstr) -> HRESULT {
    if conn.is_null() {
        return E_INVALIDARG;
    }
    let len = match address_length(conn) {
        Some(l) => l,
        None => return E_NO_IPADDRESS_IN_CONNSTR,
    };
    let mut s = Bstr::null();
    let hr = bstr_set(&mut s, conn, len as i64);
    if hr < 0 {
        return hr;
    }
    raw.reset();
    raw.0 = core::mem::replace(&mut s.0, null_mut());
    S_OK
}

unsafe fn tick() -> u64 {
    GetTickCount64()
}

pub unsafe fn open_session_impl(conn: *const u16, port: u32, ext: *const GUID, out: *mut *mut c_void, timeout: Option<u32>) -> HRESULT {
    let start = tick();
    let limit = timeout.unwrap_or(DEFAULT_OPEN_SESSION_TIMEOUT_MS) as u64;
    let mut ip = Bstr::null();
    let mut key = Bstr::null();
    let mut explicit = 0u32;
    let hr = split_console_address(conn, port, &mut ip.0, &mut key.0, &mut explicit);
    if hr < 0 {
        return hr;
    }
    let mut opts = OpenSessionOptions { size: 0x58, session_key: [0; SESSION_KEY_CHARS], skip_ready_check: 0, allow_unneeded_key: 0, quiet: 0, reserved: 0, timeout_ms: limit as u32 };
    if !key.0.is_null() {
        let n = wlen(key.0);
        if n >= SESSION_KEY_CHARS {
            std::process::abort();
        }
        core::ptr::copy_nonoverlapping(key.0, opts.session_key.as_mut_ptr(), n);
    }
    let mut delay: u64 = 250;
    let mut elapsed: u64 = 0;
    let mut last = start;
    loop {
        let hr = XbtpOpenSession2(ip.0, explicit, &opts, ext, &IID_XBTP_CLIENT_SESSION, out);
        if hr != E_COULD_NOT_CONNECT {
            return hr;
        }
        let mut now = tick();
        let spent = now.wrapping_sub(last);
        let d = delay.min(0xdac);
        if spent < d {
            if elapsed + 1000 + d >= limit {
                return hr;
            }
            Sleep((d - spent) as u32);
            now = tick();
        }
        delay += 250;
        elapsed += now.wrapping_sub(last);
        last = now;
        if elapsed + 1000 >= limit {
            return hr;
        }
    }
}

pub unsafe fn open_vm_session(conn: *const u16, port: u32, ext: *const GUID, out: *mut *mut c_void) -> HRESULT {
    open_session_impl(conn, port, ext, out, Some(DEFAULT_OPEN_SESSION_TIMEOUT_MS))
}

pub unsafe fn open_session_with_retry(conn: *const u16, timeout: Option<u32>, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = null_mut();
    let total = timeout.unwrap_or(DEFAULT_OPEN_SESSION_TIMEOUT_MS);
    let attempt = total.min(500);
    let mut last = tick();
    let mut spent: u64 = 0;
    let mut round: u64 = 1;
    let mut session = ComPtr::null();
    let hr = loop {
        let hr = open_session_impl(conn, XBTP_PORT_SYSTEM, &PIX_EXTENSION_ID, session.out(), Some(attempt));
        if timeout.is_none() {
            break hr;
        }
        if hr == E_RETRY_STOP_A || hr == E_RETRY_STOP_C || hr == 0 || hr == E_RETRY_STOP_B {
            break hr;
        }
        let mut now = tick();
        let d = (250 * round).min(0xdac);
        if now.wrapping_sub(last) < d {
            Sleep((d - now.wrapping_sub(last)) as u32);
            now = tick();
        }
        if total != u32::MAX {
            spent += now.wrapping_sub(last);
        }
        last = now;
        round += 1;
        if spent > total as u64 {
            break hr;
        }
    };
    if hr >= 0 {
        *out = core::mem::replace(&mut session.0, null_mut());
    }
    hr
}

type SendBegin = unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut u32) -> HRESULT;
type ReceiveEnd = unsafe extern "system" fn(*mut c_void, u32, u32, *mut u32, *mut *mut c_void) -> HRESULT;
type GetArray = unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> HRESULT;

pub unsafe fn session_send_begin(session: *mut c_void, op: u32, msg: *mut c_void, exchange: &mut u32) -> HRESULT {
    let f: SendBegin = core::mem::transmute(slot(session, 3));
    f(session, op, msg, exchange)
}

pub unsafe fn session_receive_end(session: *mut c_void, timeout: u32, exchange: u32, status: &mut u32, reply: *mut *mut c_void) -> HRESULT {
    let f: ReceiveEnd = core::mem::transmute(slot(session, 7));
    f(session, timeout, exchange, status, reply)
}

pub unsafe fn send_json_message_timeout(session: *mut c_void, body: *mut c_void, op: u32, timeout: u32, reply: Option<&mut ComPtr>) -> HRESULT {
    if session.is_null() {
        return E_INVALIDARG;
    }
    let map = |hr: HRESULT| if hr == E_PENDING { E_TRANSPORT_TIMEOUT } else { hr };
    let mut req = ComPtr::null();
    let hr = XbtpCreateMessage(&IID_MESSAGE, req.out());
    if hr < 0 {
        return map(hr);
    }
    if !body.is_null() {
        let hr = serialize(body, req.0, SET_JSON_BODY_W_REQUEST);
        if hr < 0 {
            return map(hr);
        }
    }
    let mut exchange = 0u32;
    let hr = session_send_begin(session, op, req.0, &mut exchange);
    if hr < 0 {
        return map(hr);
    }
    let mut status = 0u32;
    let mut resp = ComPtr::null();
    let hr = session_receive_end(session, timeout, exchange, &mut status, resp.out());
    if hr < 0 {
        return map(hr);
    }
    if status != 0 {
        return map(((status & 0xff) | 0x8c11_0100) as i32);
    }
    if let Some(out) = reply {
        let mut b = ComPtr::null();
        let hr = deserialize(resp.0, &mut b);
        if hr < 0 {
            return map(hr);
        }
        out.reset();
        out.0 = core::mem::replace(&mut b.0, null_mut());
    }
    S_OK
}

pub unsafe fn send_json_message(session: *mut c_void, body: *mut c_void, op: u32, reply: Option<&mut ComPtr>) -> HRESULT {
    let hr = send_json_message_timeout(session, body, op, DEFAULT_RECEIVE_TIMEOUT_MS, reply);
    if hr < 0 {
        return hr;
    }
    S_OK
}

pub struct OperationResults {
    pub checkpoint: u32,
    pub return_code: i32,
    pub errors: ComPtr,
}

impl OperationResults {
    pub fn new() -> Self {
        OperationResults { checkpoint: 0, return_code: 0, errors: ComPtr::null() }
    }

    pub unsafe fn initialize(&mut self, reply: *mut c_void) -> HRESULT {
        if reply.is_null() {
            return E_INVALIDARG;
        }
        let mut err = ComPtr::null();
        if obj_get_object(reply, w("XtfOperationError").as_ptr(), err.out()) < 0 {
            return S_OK;
        }
        let mut d = 0.0f64;
        let hr = obj_get_number(err.0, w("ReturnCode").as_ptr(), &mut d);
        if hr < 0 {
            return hr;
        }
        self.return_code = cvtt32(d);
        let hr = obj_get_number(err.0, w("Checkpoint").as_ptr(), &mut d);
        if hr < 0 {
            return hr;
        }
        self.checkpoint = cvtt64(d) as u32;
        let f: GetArray = core::mem::transmute(slot(err.0, 10));
        f(err.0, w("Errors").as_ptr(), self.errors.out())
    }
}

impl Default for OperationResults {
    fn default() -> Self {
        Self::new()
    }
}

pub unsafe fn set_json_i32(body: *mut c_void, name: &str, v: i32) -> HRESULT {
    set_int(body, name, v)
}

pub unsafe fn set_json_u32(body: *mut c_void, name: &str, v: u32) -> HRESULT {
    set_dword(body, name, v)
}

pub unsafe fn set_json_f64(body: *mut c_void, name: &str, v: f64) -> HRESULT {
    if body.is_null() {
        return E_INVALIDARG;
    }
    obj_set_number(body, w(name).as_ptr(), v)
}

pub unsafe fn set_json_str(body: *mut c_void, name: &str, v: *const u16) -> HRESULT {
    obj_set_string(body, w(name).as_ptr(), v)
}
