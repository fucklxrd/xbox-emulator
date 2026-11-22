use crate::base::{log_error_args, log_info_args};
use crate::sys::*;
use crate::xbtp2_ext::{com_release, Message};
use crate::xbtp2_json::{JsonObject, ObjVtbl, XbtpCreateJsonObject, XbtpGetJsonObjectFromUtf8};
use crate::xbtp2_ext::new_message;
use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicU32, Ordering};

pub const HANDSHAKE_OPERATION: u32 = 0x8000_0000;
pub const TITLE_STATUS_OPERATION: u32 = 0x8000_0001;
pub const SESSION_KEY_ATTRIBUTE: &str = "SessionKey";
pub const SESSION_KEY_RESULT_ATTRIBUTE: &str = "Result";
pub const SYSTEM_READY_STATE_ATTRIBUTE: &str = "SS";
pub const SYSTEM_IS_READY_VALUE: &str = "r";
pub const SYSTEM_NOT_READY_VALUE: &str = "nr";
pub const SYSTEM_UPDATE_VALUE: &str = "suv2";
pub const SYSTEM_NOT_INITIALIZED_VALUE: &str = "sni";
pub const TITLE_STATUS_PROPERTY: &str = "status";
pub const TITLE_STATUS_UNKNOWN: &str = "unknown";
pub const KEY_RESULT_MISMATCH: f64 = 1.0;
pub const KEY_RESULT_MATCH: f64 = 2.0;
pub const KEY_RESULT_NOT_NEEDED: f64 = 3.0;
pub const E_SYSTEM_NOT_READY: HRESULT = 0x8c11_040eu32 as i32;
pub const E_SESSION_KEY_REQUIRED: HRESULT = 0x8c11_040fu32 as i32;
pub const E_SESSION_KEY_MISMATCH: HRESULT = 0x8c11_040bu32 as i32;
pub const E_SYSTEM_UPDATING: HRESULT = 0x8c11_0411u32 as i32;
pub const E_SYSTEM_NOT_BOOTED: HRESULT = 0x8c11_0412u32 as i32;
pub const OLDER_CONSOLE_STATUS: i32 = -5;
pub const HANDSHAKE_TIMEOUT_MS: u32 = 30_000;

pub static SYSTEM_NOT_READY: AtomicU32 = AtomicU32::new(0);

pub struct Json(pub *mut JsonObject);

impl Json {
    pub unsafe fn new() -> Result<Json, HRESULT> {
        let mut p: *mut c_void = null_mut();
        let hr = XbtpCreateJsonObject(&mut p);
        if hr < 0 {
            return Err(hr);
        }
        Ok(Json(p as *mut JsonObject))
    }

    unsafe fn vt(&self) -> &ObjVtbl {
        &**(self.0 as *const *const ObjVtbl)
    }

    pub unsafe fn set_number(&self, name: &str, v: f64) -> HRESULT {
        let n = wide(name);
        (self.vt().set_number)(self.0, n.as_ptr(), v)
    }

    pub unsafe fn get_number(&self, name: &str, v: &mut f64) -> HRESULT {
        let n = wide(name);
        (self.vt().get_number)(self.0, n.as_ptr(), v)
    }

    pub unsafe fn set_string(&self, name: &str, v: *const u16) -> HRESULT {
        let n = wide(name);
        (self.vt().set_string)(self.0, n.as_ptr(), v)
    }

    pub unsafe fn get_string(&self, name: &str) -> Result<Bstr, HRESULT> {
        let n = wide(name);
        let mut b: BSTR = null_mut();
        let hr = (self.vt().get_string)(self.0, n.as_ptr(), &mut b);
        if hr < 0 {
            if !b.is_null() {
                SysFreeString(b);
            }
            return Err(hr);
        }
        Ok(Bstr(b))
    }

    pub unsafe fn set_object(&self, name: &str, v: &Json) -> HRESULT {
        let n = wide(name);
        (self.vt().set_object)(self.0, n.as_ptr(), v.0 as *mut c_void)
    }

    pub unsafe fn set_array(&self, name: &str, v: *mut c_void) -> HRESULT {
        let n = wide(name);
        (self.vt().set_array)(self.0, n.as_ptr(), v)
    }

    pub unsafe fn get_object(&self, name: &str) -> Result<Json, HRESULT> {
        let n = wide(name);
        let mut p: *mut c_void = null_mut();
        let hr = (self.vt().get_object)(self.0, n.as_ptr(), &mut p);
        if hr < 0 {
            return Err(hr);
        }
        Ok(Json(p as *mut JsonObject))
    }

    pub unsafe fn get_array(&self, name: &str) -> Result<*mut c_void, HRESULT> {
        let n = wide(name);
        let mut p: *mut c_void = null_mut();
        let hr = (self.vt().get_array)(self.0, n.as_ptr(), &mut p);
        if hr < 0 {
            return Err(hr);
        }
        Ok(p)
    }

    pub unsafe fn stringify(&self) -> Result<Bstr, HRESULT> {
        let mut b: BSTR = null_mut();
        let hr = (self.vt().serialize)(self.0, &mut b);
        if hr < 0 {
            return Err(hr);
        }
        Ok(Bstr(b))
    }
}

impl Drop for Json {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { com_release(self.0 as *mut c_void) };
        }
    }
}

pub struct Bstr(pub BSTR);

impl Bstr {
    pub unsafe fn text(&self) -> String {
        if self.0.is_null() {
            String::new()
        } else {
            from_wide(self.0)
        }
    }
}

impl Drop for Bstr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { SysFreeString(self.0) };
        }
    }
}

pub unsafe fn deserialize_json_message(message: *mut c_void) -> Result<Json, HRESULT> {
    let m = message as *mut Message;
    let mut count = 0u32;
    let mut buf: *mut u8 = null_mut();
    let hr = ((*(*m).vtbl).get_json_body)(m, &mut count, &mut buf);
    if hr < 0 {
        return Err(hr);
    }
    let mut p: *mut c_void = null_mut();
    let hr = XbtpGetJsonObjectFromUtf8(buf, &mut p);
    if hr < 0 {
        return Err(hr);
    }
    Ok(Json(p as *mut JsonObject))
}

pub unsafe fn serialize_json_message(body: &Json, message: *mut c_void) -> HRESULT {
    let text = match body.stringify() {
        Ok(t) => t,
        Err(hr) => return hr,
    };
    let m = message as *mut Message;
    ((*(*m).vtbl).set_json_body_w)(m, text.0)
}

pub unsafe fn serialize_json_reply(body: &Json, reply: *mut c_void) -> HRESULT {
    let text = match body.stringify() {
        Ok(t) => t,
        Err(hr) => return hr,
    };
    crate::xbtp2_ext::reply_set_json_w(reply, text.0)
}

pub unsafe fn server_handshake(expected_key: Option<&[u16]>, received: *mut c_void, reply: *mut c_void) -> HRESULT {
    let req = match deserialize_json_message(received) {
        Ok(j) => j,
        Err(hr) => return hr,
    };
    let client_key = match req.get_string(SESSION_KEY_ATTRIBUTE) {
        Ok(k) => k,
        Err(hr) => return hr,
    };
    let result = match expected_key {
        Some(key) => {
            let mut e = key.to_vec();
            e.push(0);
            log_info_args(b"Validating '%ws' == '%ws'\0", &[e.as_ptr() as usize, client_key.0 as usize]);
            let given: Vec<u16> = if client_key.0.is_null() { Vec::new() } else { core::slice::from_raw_parts(client_key.0, wcslen(client_key.0)).to_vec() };
            if given.as_slice() != key {
                KEY_RESULT_MISMATCH
            } else {
                KEY_RESULT_MATCH
            }
        }
        None => KEY_RESULT_NOT_NEEDED,
    };
    let body = match Json::new() {
        Ok(j) => j,
        Err(hr) => return hr,
    };
    let hr = body.set_number(SESSION_KEY_RESULT_ATTRIBUTE, result);
    if hr < 0 {
        return hr;
    }
    let state = if SYSTEM_NOT_READY.load(Ordering::SeqCst) == 0 { SYSTEM_IS_READY_VALUE } else { SYSTEM_NOT_READY_VALUE };
    let v = wide(state);
    let hr = body.set_string(SYSTEM_READY_STATE_ATTRIBUTE, v.as_ptr());
    if hr < 0 {
        return hr;
    }
    serialize_json_reply(&body, reply)
}

pub unsafe fn title_status(reply: *mut c_void) -> HRESULT {
    let body = match Json::new() {
        Ok(j) => j,
        Err(hr) => return hr,
    };
    let v = wide(TITLE_STATUS_UNKNOWN);
    let hr = body.set_string(TITLE_STATUS_PROPERTY, v.as_ptr());
    if hr < 0 {
        return hr;
    }
    serialize_json_reply(&body, reply)
}

pub struct HandshakeOptions {
    pub skip_ready_check: bool,
    pub allow_unneeded_key: bool,
}

pub const IGNORE_STATE_VARIABLE: &str = "XTF_IGNORE_STATE";

fn ignore_state_override() -> bool {
    match std::env::var(IGNORE_STATE_VARIABLE) {
        Ok(v) => {
            let t = v.trim_start();
            let end = t.char_indices().find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')))).map(|(i, _)| i).unwrap_or(t.len());
            t[..end].parse::<i64>().map(|n| n != 0).unwrap_or(false)
        }
        Err(_) => false,
    }
}

fn ready_state_error(text: &str) -> HRESULT {
    let t = text;
    if ignore_state_override() {
        return S_OK;
    }
    if t == SYSTEM_NOT_READY_VALUE {
        crate::base::log_warning("System not ready - failing connection");
        return E_SYSTEM_NOT_READY;
    }
    if t.eq_ignore_ascii_case(SYSTEM_UPDATE_VALUE) {
        crate::base::log_warning("System update in progress - failing connection");
        return E_SYSTEM_UPDATING;
    }
    if t.eq_ignore_ascii_case(SYSTEM_NOT_INITIALIZED_VALUE) {
        crate::base::log_warning("System never finished booting - failing connection");
        return E_SYSTEM_NOT_BOOTED;
    }
    if t == SYSTEM_IS_READY_VALUE {
        return S_OK;
    }
    crate::base::log_warning("System not ready for unknown reason - failing connection");
    E_SYSTEM_NOT_READY
}

pub unsafe fn client_handshake(session: *mut c_void, key: *const u16, options: &HandshakeOptions) -> HRESULT {
    let req = match Json::new() {
        Ok(j) => j,
        Err(hr) => return hr,
    };
    let empty = [0u16];
    let k = if key.is_null() { empty.as_ptr() } else { key };
    let hr = req.set_string(SESSION_KEY_ATTRIBUTE, k);
    if hr < 0 {
        return hr;
    }
    let msg = new_message() as *mut c_void;
    let hr = serialize_json_message(&req, msg);
    if hr < 0 {
        com_release(msg);
        return hr;
    }
    let vt = &**(session as *const *const crate::session_impl::SessionImplVtbl);
    let mut exchange = 0i32;
    let hr = (vt.send_begin_message)(session as *mut _, HANDSHAKE_OPERATION, msg, &mut exchange);
    com_release(msg);
    if hr < 0 {
        return hr;
    }
    let mut status = 0i32;
    let mut reply: *mut c_void = null_mut();
    let hr = (vt.receive_end_message)(session as *mut _, HANDSHAKE_TIMEOUT_MS, exchange, &mut status, &mut reply);
    if hr < 0 {
        return hr;
    }
    let hr = 'run: {
        if status == OLDER_CONSOLE_STATUS {
            log_info_args(b"SessionHandshake : older console - no handshake\0", &[]);
            break 'run S_OK;
        }
        if status != 0 {
            break 'run crate::session::status_error(status);
        }
        let body = match deserialize_json_message(reply) {
            Ok(j) => j,
            Err(hr) => break 'run hr,
        };
        if !options.skip_ready_check {
            if let Ok(s) = body.get_string(SYSTEM_READY_STATE_ATTRIBUTE) {
                let hr = ready_state_error(&s.text());
                if hr < 0 {
                    break 'run hr;
                }
            }
        }
        let mut result = 0f64;
        let hr = body.get_number(SESSION_KEY_RESULT_ATTRIBUTE, &mut result);
        if hr < 0 {
            break 'run hr;
        }
        let mut hr = S_OK;
        if result == KEY_RESULT_MATCH {
        } else if result == KEY_RESULT_NOT_NEEDED {
            if !options.allow_unneeded_key && *k != 0 {
                hr = E_SESSION_KEY_REQUIRED;
            }
        } else {
            hr = E_SESSION_KEY_MISMATCH;
        }
        if hr < 0 {
            log_error_args(b"Failed 0x%X session link handshake with key %ws\0", &[hr as u32 as usize, k as usize]);
        }
        hr
    };
    if !reply.is_null() {
        com_release(reply);
    }
    hr
}
