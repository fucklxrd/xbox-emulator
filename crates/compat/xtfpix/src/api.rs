use crate::client::*;
use crate::com::*;
use crate::json::*;
use core::ffi::c_void;
use core::ptr::null_mut;

pub const MSG_CLEAR_RUNNING_PROCESS_LIST: u32 = 3;
pub const MSG_REQUEST_VIDEO_CLIP: u32 = 0x20;
pub const MSG_PC_TELEMETRY: u32 = 0x24;
pub const MSG_ENABLE_GAME_DVR: u32 = 0x28;
pub const VIDEO_CLIP_REQUEST_TIMEOUT_MS: u32 = 60000;
pub const GAME_DVR_REQUEST_TIMEOUT_MS: u32 = 30000;
pub const TELEMETRY_OPEN_TIMEOUT_MS: u32 = 1000;
pub const GUID_TEXT_CHARS: usize = 0x28;

#[no_mangle]
pub unsafe extern "system" fn XtfPixEnableGameDvr(address: *const u16, enable: i32) -> HRESULT {
    if address.is_null() {
        return E_POINTER;
    }
    let mut raw = Bstr::null();
    let mut port = 0u32;
    let hr = split_address_port(address, XBTP_PORT_SYSTEM, &mut raw, &mut port);
    if hr < 0 {
        return hr;
    }
    let mut session = ComPtr::null();
    let hr = open_vm_session(raw.0, port, &PIX_EXTENSION_ID, session.out());
    if hr < 0 {
        return hr;
    }
    let mut body = ComPtr::null();
    let hr = XbtpCreateJsonObject(body.out());
    if hr < 0 {
        return hr;
    }
    let hr = set_json_i32(body.0, "EnableGameDvr", enable);
    if hr < 0 {
        return hr;
    }
    let mut reply = ComPtr::null();
    let hr = send_json_message_timeout(session.0, body.0, MSG_ENABLE_GAME_DVR, GAME_DVR_REQUEST_TIMEOUT_MS, Some(&mut reply));
    if hr < 0 {
        return hr;
    }
    let mut results = OperationResults::new();
    let hr = results.initialize(reply.0);
    if hr < 0 {
        return hr;
    }
    results.return_code
}

#[no_mangle]
pub unsafe extern "system" fn XtfPixClearRunningProcessList(address: *const u16) -> HRESULT {
    if address.is_null() {
        return E_POINTER;
    }
    let mut raw = Bstr::null();
    let mut port = 0u32;
    let hr = split_address_port(address, XBTP_PORT_TITLE, &mut raw, &mut port);
    if hr < 0 {
        return hr;
    }
    let mut session = ComPtr::null();
    let hr = open_vm_session(raw.0, port, &PIX_EXTENSION_ID, session.out());
    if hr < 0 {
        return hr;
    }
    let mut msg = ComPtr::null();
    let hr = XbtpCreateMessage(&IID_MESSAGE, msg.out());
    if hr < 0 {
        return hr;
    }
    let mut body = ComPtr::null();
    let hr = XbtpCreateJsonObject(body.out());
    if hr < 0 {
        return hr;
    }
    let hr = serialize(body.0, msg.0, SET_JSON_BODY_W_REQUEST);
    if hr < 0 {
        return hr;
    }
    let mut exchange = 0u32;
    let hr = session_send_begin(session.0, MSG_CLEAR_RUNNING_PROCESS_LIST, msg.0, &mut exchange);
    if hr < 0 {
        return hr;
    }
    let mut status = 0u32;
    let mut reply = ComPtr::null();
    let hr = session_receive_end(session.0, DEFAULT_RECEIVE_TIMEOUT_MS, exchange, &mut status, reply.out());
    if hr < 0 {
        return hr;
    }
    if status == 0 {
        S_OK
    } else {
        E_FAIL
    }
}

#[no_mangle]
pub unsafe extern "system" fn XtfPixRequestVideoClip(address: *const u16, start_tick: u64, end_tick: u64, start_out: *mut u64, end_out: *mut u64, file_path: *mut BSTR) -> HRESULT {
    if address.is_null() || start_out.is_null() || end_out.is_null() || file_path.is_null() {
        return E_POINTER;
    }
    let mut raw = Bstr::null();
    let mut port = 0u32;
    let hr = split_address_port(address, XBTP_PORT_SYSTEM, &mut raw, &mut port);
    if hr < 0 {
        return hr;
    }
    let mut session = ComPtr::null();
    let hr = open_vm_session(raw.0, port, &PIX_EXTENSION_ID, session.out());
    if hr < 0 {
        return hr;
    }
    let mut body = ComPtr::null();
    let hr = XbtpCreateJsonObject(body.out());
    if hr < 0 {
        return hr;
    }
    let hr = set_u64(body.0, "StartTime", start_tick);
    if hr < 0 {
        return hr;
    }
    let hr = set_u64(body.0, "EndTime", end_tick);
    if hr < 0 {
        return hr;
    }
    let mut reply = ComPtr::null();
    let hr = send_json_message_timeout(session.0, body.0, MSG_REQUEST_VIDEO_CLIP, VIDEO_CLIP_REQUEST_TIMEOUT_MS, Some(&mut reply));
    if hr < 0 {
        return hr;
    }
    let mut results = OperationResults::new();
    let hr = results.initialize(reply.0);
    if hr < 0 {
        return hr;
    }
    if results.return_code < 0 {
        return results.return_code;
    }
    let hr = get_u64(reply.0, "StartTime", &mut *start_out);
    if hr < 0 {
        return hr;
    }
    let hr = get_u64(reply.0, "EndTime", &mut *end_out);
    if hr < 0 {
        return hr;
    }
    let mut path = Bstr::null();
    let hr = obj_get_string(reply.0, w("FilePath").as_ptr(), &mut path.0);
    if hr < 0 {
        return hr;
    }
    *file_path = core::mem::replace(&mut path.0, null_mut());
    hr
}

pub struct Cursor {
    pub cur: *mut u8,
    pub end: *mut u8,
}

impl Cursor {
    pub fn remaining(&self) -> usize {
        (self.end as usize).wrapping_sub(self.cur as usize)
    }

    pub fn take(&mut self, n: usize) -> Result<*mut u8, HRESULT> {
        if self.remaining() < n {
            return Err(E_OUTOFMEMORY);
        }
        let p = self.cur;
        if p.is_null() {
            return Err(E_OUTOFMEMORY);
        }
        self.cur = unsafe { p.add(n) };
        Ok(p)
    }

    pub unsafe fn string(&mut self) -> Result<*const u16, HRESULT> {
        let start = self.cur;
        if self.remaining() < 2 {
            return Err(E_OUTOFMEMORY);
        }
        let mut p = start;
        loop {
            if *p == 0 && *p.add(1) == 0 {
                self.cur = p.add(2);
                break;
            }
            p = p.add(2);
            self.cur = p;
            if (self.end as usize).wrapping_sub(p as usize) < 2 {
                *p.sub(1) = 0;
                *p.sub(2) = 0;
                self.cur = self.end;
                break;
            }
        }
        if start.is_null() {
            return Err(E_OUTOFMEMORY);
        }
        Ok(start as *const u16)
    }
}

pub unsafe fn guid_text(p: *const u8) -> Vec<u16> {
    let g = core::ptr::read_unaligned(p as *const GUID);
    let s = format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1, g.data2, g.data3, g.data4[0], g.data4[1], g.data4[2], g.data4[3], g.data4[4], g.data4[5], g.data4[6], g.data4[7]
    );
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.resize(GUID_TEXT_CHARS, 0);
    v
}

unsafe fn rd_u32(p: *const u8) -> u32 {
    core::ptr::read_unaligned(p as *const u32)
}

unsafe fn rd_u64(p: *const u8) -> u64 {
    core::ptr::read_unaligned(p as *const u64)
}

#[no_mangle]
pub unsafe extern "system" fn XtfPixPCLogTelemetryEvent(address: *const u16, buffer: *mut u8, size: i32) -> HRESULT {
    if address.is_null() || *address == 0 {
        return E_INVALIDARG;
    }
    let mut raw = Bstr::null();
    let hr = get_raw_address(address, &mut raw);
    if hr < 0 {
        return hr;
    }
    let mut session = ComPtr::null();
    let hr = open_session_with_retry(raw.0, Some(TELEMETRY_OPEN_TIMEOUT_MS), session.out());
    if hr < 0 {
        return hr;
    }
    let mut body = ComPtr::null();
    let hr = XbtpCreateJsonObject(body.out());
    if hr < 0 {
        return hr;
    }
    let hr = fill_telemetry(body.0, buffer, size);
    if hr < 0 {
        return hr;
    }
    let mut reply = ComPtr::null();
    send_json_message(session.0, body.0, MSG_PC_TELEMETRY, Some(&mut reply))
}

macro_rules! step {
    ($e:expr) => {{
        let hr = $e;
        if hr < 0 {
            return Err(hr);
        }
        hr
    }};
}

pub unsafe fn fill_telemetry(body: *mut c_void, buffer: *mut u8, size: i32) -> HRESULT {
    let mut c = Cursor { cur: buffer, end: (buffer as usize).wrapping_add(size as isize as usize) as *mut u8 };
    let r = (|| -> Result<HRESULT, HRESULT> {
        let kind_p = c.take(4)?;
        let time_p = c.take(8)?;
        let session_guid = guid_text(c.take(16)?);
        let os_version = c.string()?;
        let recovery = c.take(4)?;
        let xdk = c.take(4)?;
        let file_type = c.string()?;
        let file_xdk = c.take(4)?;
        let file_guid = guid_text(c.take(16)?);
        let file_size = c.take(8)?;
        let kind = rd_u32(kind_p);
        step!(set_json_u32(body, "EventType", kind));
        step!(set_u64(body, "EventTime", rd_u64(time_p)));
        step!(set_json_str(body, "SessionGUID", session_guid.as_ptr()));
        step!(set_json_str(body, "OSVersion", os_version));
        step!(set_json_u32(body, "RecoveryVersion", rd_u32(recovery)));
        step!(set_json_u32(body, "XdkVersion", rd_u32(xdk)));
        step!(set_json_str(body, "CaptureFileType", file_type));
        step!(set_json_u32(body, "CaptureFileXdkVersion", rd_u32(file_xdk)));
        step!(set_json_str(body, "CaptureFileGUID", file_guid.as_ptr()));
        let mut hr = step!(set_u64(body, "CaptureFileSize", rd_u64(file_size)));
        match kind {
            0 => {
                let context = c.string()?;
                let name = c.string()?;
                let message = c.string()?;
                let code = c.take(4)?;
                let stack = c.string()?;
                step!(set_json_str(body, "ErrorContext", context));
                step!(set_json_str(body, "ErrorName", name));
                step!(set_json_str(body, "ErrorMessage", message));
                step!(set_json_u32(body, "ErrorCode", rd_u32(code)));
                hr = step!(set_json_str(body, "StackTrace", stack));
            }
            1 => {
                let name = c.string()?;
                if c.remaining() < 8 || c.cur.is_null() {
                    return Err(E_OUTOFMEMORY);
                }
                let value = core::ptr::read_unaligned(c.cur as *const f64);
                step!(set_json_str(body, "CounterName", name));
                hr = step!(set_json_f64(body, "CounterValue", value));
            }
            2 => {
                let action = c.string()?;
                let device = guid_text(c.take(16)?);
                step!(set_json_str(body, "Action", action));
                hr = step!(set_json_str(body, "DeviceConnectionGUID", device.as_ptr()));
            }
            3 => {
                let name = c.string()?;
                if c.remaining() < 8 || c.cur.is_null() {
                    return Err(E_OUTOFMEMORY);
                }
                let value = rd_u64(c.cur);
                step!(set_json_str(body, "CountName", name));
                hr = step!(set_u64(body, "CountValue", value));
            }
            _ => {}
        }
        Ok(hr)
    })();
    match r {
        Ok(h) => h,
        Err(e) => e,
    }
}
