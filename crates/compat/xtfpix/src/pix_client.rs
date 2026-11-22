use crate::client::*;
use crate::com::*;
use crate::json::*;
use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use xtfcommon::sys::{hr_from_last_error, CloseHandle, GetCurrentThreadId, _wcsicmp};

pub const PIX_CLIENT_VERSION: u32 = 0x55f0_114d;
pub const OP_TRACK_PROCESSES: u32 = 1;
pub const OP_PROCESS_CREATED: u32 = 2;
pub const OP_CAPTURE_NEXT_FRAME: u32 = 4;
pub const OP_CAPTURE_RESULT: u32 = 5;
pub const OP_CONFIGURE_SYSMON: u32 = 6;
pub const OP_GET_SYSMON_CONFIGURATION: u32 = 8;
pub const OP_DO_TIMING_CAPTURE: u32 = 9;
pub const OP_SET_CONTINUOUS_TIMING_CAPTURE: u32 = 0xa;
pub const OP_GET_CONTINUOUS_TIMING_CAPTURE: u32 = 0xb;
pub const OP_TRACK_SYSMON_CONFIGURATION: u32 = 0xd;
pub const OP_FRAME_COUNTER_DATA: u32 = 0xf;
pub const OP_PING: u32 = 0x13;
pub const OP_TRACK_GRAPHICS_CAPTURE_REQUESTS: u32 = 0x1c;
pub const OP_XMEM_STATISTICS: u32 = 0x1d;
pub const OP_VERSION_HANDSHAKE: u32 = 0x1e;
pub const OP_SET_PAUSE_STATE: u32 = 0x1f;
pub const OP_CREATE_EXCLUSIVE_CONNECTION: u32 = 0x25;
pub const OP_RELEASE_EXCLUSIVE_CONNECTION: u32 = 0x26;
pub const OP_GET_FRIENDLY_VERSION: u32 = 0x27;
pub const OP_RETRIEVE_DIAGNOSTICS: u32 = 1;
pub const PING_TIMEOUT_MS: u32 = 2000;
pub const CHANNEL_STATUS_TIMEOUT_MS: u32 = 5000;
pub const STATUS_EXCLUSIVE_UNSUPPORTED: u32 = 0xffff_fffb;

pub const IID_PIX_CLIENT: GUID = GUID::new(0x2ea5ab14, 0xeccf, 0x42ad, [0xa2, 0xb2, 0xb3, 0xeb, 0x4b, 0xb2, 0x37, 0xbd]);
pub const IID_PIX_CLIENT_2: GUID = GUID::new(0xf6ba20e0, 0x0553, 0x4656, [0xbb, 0x19, 0x1b, 0x9a, 0x68, 0xe5, 0x1a, 0x67]);
pub const IID_PIX_CLIENT_3: GUID = GUID::new(0x186f19a8, 0xeb16, 0x4810, [0xa6, 0xd7, 0x50, 0x7a, 0x81, 0xb3, 0x61, 0xda]);
pub const IID_PIX_CLIENT_4: GUID = GUID::new(0xd16924ea, 0xf59d, 0x4154, [0x91, 0x67, 0x5c, 0x64, 0x6c, 0x86, 0x25, 0x37]);
pub const IID_PIX_CLIENT_5: GUID = GUID::new(0x5254b4d1, 0xacda, 0x4c41, [0xbc, 0x01, 0x75, 0x5d, 0x09, 0xd5, 0xb3, 0x44]);
pub const DIAGNOSTIC_INFORMATION_EXTENSION_ID: GUID = GUID::new(0xfd21ac23, 0x3a3a, 0x45ce, [0xa7, 0x85, 0xc9, 0x73, 0x37, 0x2d, 0xd8, 0x81]);

const WAIT_TIMEOUT: u32 = 0x102;
const WAIT_IO_COMPLETION: u32 = 0xc0;
const WAIT_FAILED: u32 = 0xffff_ffff;

#[link(name = "kernel32", kind = "raw-dylib")]
extern "system" {
    fn CreateEventW(attr: *mut c_void, manual: i32, initial: i32, name: *const u16) -> HANDLE;
    fn SetEvent(h: HANDLE) -> i32;
    fn WaitForMultipleObjects(n: u32, handles: *const HANDLE, all: i32, ms: u32) -> u32;
    fn WaitForMultipleObjectsEx(n: u32, handles: *const HANDLE, all: i32, ms: u32, alertable: i32) -> u32;
    fn CreateThread(attr: *mut c_void, stack: usize, start: unsafe extern "system" fn(*mut c_void) -> u32, param: *mut c_void, flags: u32, id: *mut u32) -> HANDLE;
    fn GetThreadId(h: HANDLE) -> u32;
    fn LoadLibraryExW(name: *const u16, file: HANDLE, flags: u32) -> HANDLE;
    fn GetProcAddress(module: HANDLE, name: *const u8) -> usize;
    fn GetModuleFileNameW(module: HANDLE, buf: *mut u16, n: u32) -> u32;
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

type LogGetMainLevel = unsafe extern "C" fn() -> i32;
type LogSetMainLevel = unsafe extern "C" fn(u32) -> HRESULT;

static LOG_LOADED: Once = Once::new();
static LOG_GET_MAIN_LEVEL: AtomicUsize = AtomicUsize::new(0);
static LOG_SET_MAIN_LEVEL: AtomicUsize = AtomicUsize::new(0);

unsafe fn load_log_table() {
    let mut path = [0u16; 260];
    let n = GetModuleFileNameW(null_mut(), path.as_mut_ptr(), path.len() as u32) as usize;
    let dir = path[..n.min(path.len())].iter().rposition(|&c| c == b'\\' as u16 || c == b'/' as u16).map(|i| i + 1).unwrap_or(0);
    let mut combined: Vec<u16> = path[..dir].to_vec();
    combined.extend("XtfBaseServices.dll".encode_utf16());
    combined.push(0);
    let mut module = LoadLibraryExW(combined.as_ptr(), null_mut(), 0);
    if module.is_null() {
        module = LoadLibraryExW(w("XtfBaseServices.dll").as_ptr(), null_mut(), 0);
    }
    if module.is_null() {
        return;
    }
    LOG_GET_MAIN_LEVEL.store(GetProcAddress(module, b"XtfLogGetMainLevel\0".as_ptr()), Ordering::Release);
    LOG_SET_MAIN_LEVEL.store(GetProcAddress(module, b"XtfLogSetMainLevel\0".as_ptr()), Ordering::Release);
}

pub fn ensure_log_table() {
    LOG_LOADED.call_once(|| unsafe { load_log_table() });
}

#[cfg(test)]
pub fn set_log_table(get: Option<LogGetMainLevel>, set: Option<LogSetMainLevel>) {
    ensure_log_table();
    LOG_GET_MAIN_LEVEL.store(get.map(|f| f as usize).unwrap_or(0), Ordering::Release);
    LOG_SET_MAIN_LEVEL.store(set.map(|f| f as usize).unwrap_or(0), Ordering::Release);
}

unsafe fn log_get_main_level() -> i32 {
    ensure_log_table();
    match LOG_GET_MAIN_LEVEL.load(Ordering::Acquire) {
        0 => 0,
        p => core::mem::transmute::<usize, LogGetMainLevel>(p)(),
    }
}

unsafe fn log_set_main_level(level: u32) {
    ensure_log_table();
    let p = LOG_SET_MAIN_LEVEL.load(Ordering::Acquire);
    if p != 0 {
        core::mem::transmute::<usize, LogSetMainLevel>(p)(level);
    }
}

unsafe fn client_send_more(session: *mut c_void, exchange: u32, msg: *mut c_void) -> HRESULT {
    let f: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT = core::mem::transmute(slot(session, 4));
    f(session, exchange, msg)
}

unsafe fn client_receive_more(session: *mut c_void, timeout: u32, exchange: u32, out: *mut *mut c_void) -> HRESULT {
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> HRESULT = core::mem::transmute(slot(session, 6));
    f(session, timeout, exchange, out)
}

unsafe fn client_get_event(session: *mut c_void, out: *mut HANDLE) -> HRESULT {
    let f: unsafe extern "system" fn(*mut c_void, *mut HANDLE) -> HRESULT = core::mem::transmute(slot(session, 9));
    f(session, out)
}

unsafe fn msg_get_binary_body(m: *mut c_void, n: *mut u32, p: *mut *mut u8) -> HRESULT {
    let f: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut u8) -> HRESULT = core::mem::transmute(slot(m, 4));
    f(m, n, p)
}

unsafe fn msg_set_binary_body(m: *mut c_void, n: u32, p: *const u8) -> HRESULT {
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8) -> HRESULT = core::mem::transmute(slot(m, 8));
    f(m, n, p)
}

unsafe fn obj_get_array(o: *mut c_void, name: &str, out: *mut *mut c_void) -> HRESULT {
    let f: unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> HRESULT = core::mem::transmute(slot(o, 10));
    f(o, w(name).as_ptr(), out)
}

unsafe fn arr_get_string(a: *mut c_void, i: u32, out: *mut BSTR) -> HRESULT {
    let f: unsafe extern "system" fn(*mut c_void, u32, *mut BSTR) -> HRESULT = core::mem::transmute(slot(a, 8));
    f(a, i, out)
}

unsafe fn arr_get_count(a: *mut c_void, out: *mut u32) -> HRESULT {
    let f: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT = core::mem::transmute(slot(a, 11));
    f(a, out)
}

unsafe fn get_str(body: *mut c_void, name: &str, out: &mut Bstr) -> HRESULT {
    out.reset();
    obj_get_string(body, w(name).as_ptr(), &mut out.0)
}

type Fill<'a> = &'a dyn Fn(*mut c_void) -> HRESULT;

unsafe fn begin(session: *mut c_void, op: u32, fill: Option<Fill>, exchange: &mut u32) -> HRESULT {
    let mut msg = ComPtr::null();
    let hr = XbtpCreateMessage(&IID_MESSAGE, msg.out());
    if hr < 0 {
        return hr;
    }
    if let Some(fill) = fill {
        let mut body = ComPtr::null();
        let hr = XbtpCreateJsonObject(body.out());
        if hr < 0 {
            return hr;
        }
        let hr = fill(body.0);
        if hr < 0 {
            return hr;
        }
        let hr = serialize(body.0, msg.0, SET_JSON_BODY_W_REQUEST);
        if hr < 0 {
            return hr;
        }
    }
    session_send_begin(session, op, msg.0, exchange)
}

unsafe fn round_trip(session: *mut c_void, op: u32, fill: Option<Fill>, timeout: u32, status: &mut u32, reply: &mut ComPtr) -> HRESULT {
    let mut exchange = 0u32;
    let hr = begin(session, op, fill, &mut exchange);
    if hr < 0 {
        return hr;
    }
    session_receive_end(session, timeout, exchange, status, reply.out())
}

unsafe fn request_reply(session: *mut c_void, op: u32, fill: Option<Fill>, body: &mut ComPtr) -> HRESULT {
    let mut status = 0u32;
    let mut reply = ComPtr::null();
    let hr = round_trip(session, op, fill, DEFAULT_RECEIVE_TIMEOUT_MS, &mut status, &mut reply);
    if hr < 0 {
        return hr;
    }
    if status != 0 {
        return E_FAIL;
    }
    deserialize(reply.0, body)
}

unsafe fn get_result(body: *mut c_void, out: &mut i32) -> HRESULT {
    get_bool(body, "Result", out)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    TrackProcesses,
    CaptureNextFrame,
    CaptureCompleted,
    SysMonData,
    SysMonConfigChanged,
    TimingCompleted,
    TimingRange,
    ChannelStatus,
}

struct Fields {
    session: ComPtr,
    callback: ComPtr,
    exchange: u32,
    event: HANDLE,
    extra: u32,
}

struct ThreadState {
    thread: HANDLE,
    stop: HANDLE,
}

pub struct Listener {
    kind: Kind,
    timeout: u32,
    token: *const AtomicU64,
    counter: AtomicU32,
    fields: Mutex<Fields>,
    thread: Mutex<ThreadState>,
}

unsafe impl Send for Listener {}
unsafe impl Sync for Listener {}

type CbPath = unsafe extern "system" fn(*mut c_void, *const u16, u32) -> HRESULT;
type CbFrame = unsafe extern "system" fn(*mut c_void, u32, u32, u32) -> HRESULT;
type CbCompleted = unsafe extern "system" fn(*mut c_void, u32, i32, *const u16) -> HRESULT;
type CbData = unsafe extern "system" fn(*mut c_void, u32, *const u8) -> HRESULT;
type CbDword = unsafe extern "system" fn(*mut c_void, u32) -> HRESULT;
type CbTiming = unsafe extern "system" fn(*mut c_void, i32, *const u16) -> HRESULT;
type CbRange = unsafe extern "system" fn(*mut c_void, u64, u64) -> HRESULT;

impl Listener {
    fn new(kind: Kind, timeout: u32) -> Listener {
        Listener {
            kind,
            timeout,
            token: core::ptr::null(),
            counter: AtomicU32::new(0),
            fields: Mutex::new(Fields { session: ComPtr::null(), callback: ComPtr::null(), exchange: 0, event: null_mut(), extra: 0 }),
            thread: Mutex::new(ThreadState { thread: null_mut(), stop: null_mut() }),
        }
    }

    unsafe fn stop(&self) {
        let t = lock(&self.thread);
        if t.thread.is_null() || WaitForSingleObject(t.thread, 0) != WAIT_TIMEOUT {
            return;
        }
        let h = t.thread;
        let other = GetThreadId(h) != GetCurrentThreadId();
        SetEvent(t.stop);
        drop(t);
        if other {
            loop {
                let r = WaitForMultipleObjectsEx(1, &h, 0, INFINITE, 1);
                if r == WAIT_IO_COMPLETION {
                    continue;
                }
                if r == WAIT_FAILED && hr_from_last_error() >= 0 {
                    continue;
                }
                break;
            }
        }
    }

    unsafe fn release_all(&self) {
        let mut f = lock(&self.fields);
        f.session.reset();
        f.exchange = 0;
        f.callback.reset();
    }

    unsafe fn clear(&self) {
        self.stop();
        self.release_all();
    }

    unsafe fn start_thread(&self) -> HRESULT {
        let mut t = lock(&self.thread);
        if !t.thread.is_null() && WaitForSingleObject(t.thread, 0) == WAIT_TIMEOUT {
            return E_UNEXPECTED;
        }
        if !t.stop.is_null() {
            CloseHandle(t.stop);
        }
        t.stop = CreateEventW(null_mut(), 1, 0, core::ptr::null());
        if t.stop.is_null() {
            return hr_from_last_error();
        }
        if !t.thread.is_null() {
            CloseHandle(t.thread);
        }
        t.thread = CreateThread(null_mut(), 0, listener_main, self as *const Listener as *mut c_void, 0, null_mut());
        if t.thread.is_null() {
            E_OUTOFMEMORY
        } else {
            S_OK
        }
    }

    unsafe fn start(&self, session: *mut c_void, callback: *mut c_void, exchange: u32) -> HRESULT {
        let busy = !lock(&self.fields).session.0.is_null();
        if busy {
            self.clear();
        }
        let event = {
            let mut f = lock(&self.fields);
            f.session.reset();
            if !session.is_null() {
                com_add_ref(session);
            }
            f.session.0 = session;
            f.callback.reset();
            if !callback.is_null() {
                com_add_ref(callback);
            }
            f.callback.0 = callback;
            f.exchange = exchange;
            let mut ev: HANDLE = null_mut();
            let hr = client_get_event(session, &mut ev);
            if hr < 0 {
                return hr;
            }
            f.event = ev;
            ev
        };
        let _ = event;
        self.start_thread()
    }

    unsafe fn start_channel(&self, session: *mut c_void, callback: *mut c_void, data: u32) -> HRESULT {
        lock(&self.fields).extra = data;
        self.start(session, callback, 0)
    }

    unsafe fn teardown(&self) {
        self.stop();
        {
            let mut f = lock(&self.fields);
            f.callback.reset();
            f.session.reset();
        }
        let mut t = lock(&self.thread);
        if !t.thread.is_null() {
            CloseHandle(t.thread);
            t.thread = null_mut();
        }
        if !t.stop.is_null() {
            CloseHandle(t.stop);
            t.stop = null_mut();
        }
    }

    unsafe fn handle(&self, msg: *mut c_void, cb: *mut c_void, done: &mut bool) {
        if self.kind == Kind::SysMonData {
            let mut n = 0u32;
            let mut p: *mut u8 = null_mut();
            if msg_get_binary_body(msg, &mut n, &mut p) >= 0 {
                let f: CbData = core::mem::transmute(slot(cb, 3));
                f(cb, n, p);
            }
            *done = false;
            return;
        }
        if self.kind == Kind::ChannelStatus {
            *done = false;
            return;
        }
        let mut body = ComPtr::null();
        let ok = deserialize(msg, &mut body) >= 0;
        let b = body.0;
        match self.kind {
            Kind::TrackProcesses => {
                let mut pid = 0u32;
                let mut path = Bstr::null();
                let mut created = 0i32;
                if ok && get_dword(b, "ProcessId", &mut pid) >= 0 && get_str(b, "ProcessPath", &mut path) >= 0 && get_bool(b, "ProcessCreated", &mut created) >= 0 {
                    let f: CbPath = core::mem::transmute(slot(cb, if created != 0 { 3 } else { 4 }));
                    f(cb, path.0, pid);
                }
                *done = false;
            }
            Kind::CaptureNextFrame => {
                let (mut pid, mut flags, mut latency, mut frames) = (0u32, 0u32, 0u32, 0u32);
                if ok && get_dword(b, "ProcessId", &mut pid) >= 0 && get_dword(b, "CaptureFlags", &mut flags) >= 0 && get_dword(b, "CommandListLatencyFrameCount", &mut latency) >= 0 && get_dword(b, "CaptureFrameCount", &mut frames) >= 0 {
                    let f: CbFrame = core::mem::transmute(slot(cb, 3));
                    f(cb, flags, latency, frames);
                }
                *done = false;
            }
            Kind::CaptureCompleted => {
                let mut pid = 0u32;
                let mut result = 0i32;
                let mut path = Bstr::null();
                if ok && get_dword(b, "ProcessId", &mut pid) >= 0 && get_result(b, &mut result) >= 0 && get_str(b, "FilePath", &mut path) >= 0 {
                    let f: CbCompleted = core::mem::transmute(slot(cb, 3));
                    f(cb, pid, result, path.0);
                }
                *done = true;
            }
            Kind::SysMonConfigChanged => {
                let mut flags = 0u32;
                if ok && get_dword(b, "SysMonFlags", &mut flags) >= 0 {
                    let f: CbDword = core::mem::transmute(slot(cb, 3));
                    f(cb, flags);
                }
                *done = false;
            }
            Kind::TimingCompleted => {
                let mut result = 0i32;
                let mut path = Bstr::null();
                if ok && get_result(b, &mut result) >= 0 && get_str(b, "FilePath", &mut path) >= 0 {
                    let f: CbTiming = core::mem::transmute(slot(cb, 3));
                    f(cb, result, path.0);
                    self.counter.fetch_add(1, Ordering::AcqRel);
                }
                *done = self.counter.load(Ordering::Acquire) >= 2;
            }
            Kind::TimingRange => {
                let (mut start, mut end) = (0u64, 0u64);
                if ok && get_u64(b, "StartTime", &mut start) >= 0 && get_u64(b, "EndTime", &mut end) >= 0 {
                    let f: CbRange = core::mem::transmute(slot(cb, 3));
                    f(cb, start, end);
                }
                *done = false;
            }
            Kind::SysMonData | Kind::ChannelStatus => {}
        }
    }

    unsafe fn on_timeout(&self, session: *mut c_void, cb: *mut c_void, extra: u32, done: &mut bool) {
        if self.kind != Kind::ChannelStatus {
            return;
        }
        if ping_server(session, (*self.token).load(Ordering::Acquire)) < 0 {
            let f: CbDword = core::mem::transmute(slot(cb, 3));
            f(cb, extra);
            *done = true;
        }
    }
}

unsafe fn ping_server(session: *mut c_void, token: u64) -> HRESULT {
    let mut status = 0u32;
    let mut reply = ComPtr::null();
    let hr = round_trip(session, OP_PING, Some(&|b| set_u64(b, "ConnectionToken", token)), PING_TIMEOUT_MS, &mut status, &mut reply);
    if hr < 0 {
        return hr;
    }
    if status != 0 {
        return E_FAIL;
    }
    S_OK
}

unsafe extern "system" fn listener_main(param: *mut c_void) -> u32 {
    let l = &*(param as *const Listener);
    let stop = lock(&l.thread).stop;
    let (session, cb, exchange, event, extra) = {
        let f = lock(&l.fields);
        (f.session.0, f.callback.0, f.exchange, f.event, f.extra)
    };
    let handles = [stop, event];
    let mut done = false;
    while !done {
        match WaitForMultipleObjects(2, handles.as_ptr(), 0, l.timeout) {
            0 => done = true,
            1 => {
                let mut m = ComPtr::null();
                if client_receive_more(session, 0, exchange, m.out()) >= 0 {
                    l.handle(m.0, cb, &mut done);
                }
            }
            WAIT_TIMEOUT => l.on_timeout(session, cb, extra, &mut done),
            _ => done = true,
        }
    }
    0
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct PixInfo {
    pub server_version: u32,
    pub system_type: i32,
    pub cpu_frequency: u64,
    pub reserved: [u64; 2],
}

struct Stream {
    session: ComPtr,
    exchange: u32,
}

#[repr(C)]
pub struct PixClient {
    vtbl: *const PixClientVtbl,
    refs: AtomicU32,
    address: Bstr,
    port: u32,
    info: PixInfo,
    connection_token: AtomicU64,
    frame: Mutex<Stream>,
    xmem: Mutex<Stream>,
    capture_next_frame: Listener,
    channel_status: Listener,
    timing_range: Listener,
    timing_completed: Listener,
    sysmon_config: Listener,
    sysmon_data: Listener,
    capture_completed: Listener,
    track_processes: Listener,
}

type This = *mut PixClient;
type Cb = *mut c_void;

#[repr(C)]
pub struct PixClientVtbl {
    pub query_interface: unsafe extern "system" fn(This, *const GUID, *mut *mut c_void) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(This) -> u32,
    pub release: unsafe extern "system" fn(This) -> u32,
    pub track_instrumented_processes: unsafe extern "system" fn(This, Cb) -> HRESULT,
    pub capture_next_frame: unsafe extern "system" fn(This, u32, u32, u32, u32, Cb) -> HRESULT,
    pub configure_sysmon: unsafe extern "system" fn(This, u32, Cb) -> HRESULT,
    pub get_sysmon_configuration: unsafe extern "system" fn(This, *mut u32) -> HRESULT,
    pub track_sysmon_configuration: unsafe extern "system" fn(This, Cb) -> HRESULT,
    pub do_timing_capture: unsafe extern "system" fn(This, u64, Cb) -> HRESULT,
    pub start_continuous_timing_capture: unsafe extern "system" fn(This, Cb) -> HRESULT,
    pub stop_continuous_timing_capture: unsafe extern "system" fn(This) -> HRESULT,
    pub get_continuous_timing_capture: unsafe extern "system" fn(This, u64, u64, Cb) -> HRESULT,
    pub report_frame_counter_data: unsafe extern "system" fn(This, u32, *const u8) -> HRESULT,
    pub track_channel_status: unsafe extern "system" fn(This, Cb, u32, i32) -> HRESULT,
    pub announce_title_process: unsafe extern "system" fn(This, u32, *const u16) -> HRESULT,
    pub track_graphics_capture_requests: unsafe extern "system" fn(This, Cb) -> HRESULT,
    pub report_graphics_capture_result: unsafe extern "system" fn(This, u32, i32, *const u16) -> HRESULT,
    pub report_xmem_statistics: unsafe extern "system" fn(This, u32, *const u8) -> HRESULT,
    pub get_info: unsafe extern "system" fn(This, *mut PixInfo) -> HRESULT,
    pub set_paused_state: unsafe extern "system" fn(This, i32) -> HRESULT,
    pub get_console_id: unsafe extern "system" fn(This, *mut BSTR) -> HRESULT,
    pub do_timing_capture_ex: unsafe extern "system" fn(This, u64, u32, Cb) -> HRESULT,
    pub start_continuous_timing_capture_ex: unsafe extern "system" fn(This, u32, Cb) -> HRESULT,
    pub get_client_version: unsafe extern "system" fn(This, *mut u32) -> HRESULT,
    pub get_friendly_version: unsafe extern "system" fn(This, *mut BSTR, *mut BSTR, *mut BSTR, *mut u32) -> HRESULT,
    pub track_channel_status_with_fixed_token: unsafe extern "system" fn(This, Cb, u32, u64, i32) -> HRESULT,
}

pub static PIX_CLIENT_VTBL: PixClientVtbl = PixClientVtbl {
    query_interface: c_qi,
    add_ref: c_add_ref,
    release: c_release,
    track_instrumented_processes,
    capture_next_frame,
    configure_sysmon,
    get_sysmon_configuration,
    track_sysmon_configuration,
    do_timing_capture,
    start_continuous_timing_capture,
    stop_continuous_timing_capture,
    get_continuous_timing_capture,
    report_frame_counter_data,
    track_channel_status,
    announce_title_process,
    track_graphics_capture_requests,
    report_graphics_capture_result,
    report_xmem_statistics,
    get_info,
    set_paused_state,
    get_console_id,
    do_timing_capture_ex,
    start_continuous_timing_capture_ex,
    get_client_version,
    get_friendly_version,
    track_channel_status_with_fixed_token,
};

const CLIENT_IIDS: [GUID; 6] = [IID_PIX_CLIENT, IID_IUNKNOWN, IID_PIX_CLIENT_2, IID_PIX_CLIENT_3, IID_PIX_CLIENT_4, IID_PIX_CLIENT_5];

unsafe extern "system" fn c_qi(this: This, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if CLIENT_IIDS.iter().any(|g| *g == *iid) {
        *out = this as *mut c_void;
        c_add_ref(this);
        S_OK
    } else {
        E_NOINTERFACE
    }
}

unsafe extern "system" fn c_add_ref(this: This) -> u32 {
    (*this).refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn c_release(this: This) -> u32 {
    let n = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        destroy(this);
    }
    n
}

unsafe fn destroy(this: This) {
    track_channel_status_with_fixed_token(this, null_mut(), 0, 0, 0);
    let c = &*this;
    lock(&c.xmem).session.reset();
    lock(&c.frame).session.reset();
    for l in [&c.capture_next_frame, &c.channel_status, &c.timing_range, &c.timing_completed, &c.sysmon_config, &c.sysmon_data, &c.capture_completed, &c.track_processes] {
        l.teardown();
    }
    drop(Box::from_raw(this));
}

unsafe fn open(c: &PixClient, out: &mut ComPtr) -> HRESULT {
    open_vm_session(c.address.0, c.port, &PIX_EXTENSION_ID, out.out())
}

unsafe fn stream(c: &PixClient, op: u32, fill: Option<Fill>, listener: &Listener, cb: Cb, result: Option<&mut i32>) -> HRESULT {
    let mut session = ComPtr::null();
    let hr = open(c, &mut session);
    if hr < 0 {
        return hr;
    }
    let mut exchange = 0u32;
    let hr = begin(session.0, op, fill, &mut exchange);
    if hr < 0 {
        return hr;
    }
    let mut m = ComPtr::null();
    let hr = client_receive_more(session.0, DEFAULT_RECEIVE_TIMEOUT_MS, exchange, m.out());
    if hr < 0 {
        return hr;
    }
    let mut body = ComPtr::null();
    let hr = deserialize(m.0, &mut body);
    if hr < 0 {
        return hr;
    }
    let mut r = 0i32;
    let hr = get_result(body.0, &mut r);
    if hr < 0 {
        return hr;
    }
    if r < 0 {
        return r;
    }
    if let Some(out) = result {
        *out = r;
        if cb.is_null() {
            listener.clear();
            return r;
        }
    }
    listener.start(session.0, cb, exchange)
}

unsafe extern "system" fn track_instrumented_processes(this: This, cb: Cb) -> HRESULT {
    let c = &*this;
    if cb.is_null() {
        c.track_processes.clear();
        return S_OK;
    }
    stream(c, OP_TRACK_PROCESSES, Some(&|b| set_int(b, "TrackingEnabled", 1)), &c.track_processes, cb, None)
}

unsafe extern "system" fn capture_next_frame(this: This, pid: u32, flags: u32, latency: u32, frames: u32, cb: Cb) -> HRESULT {
    let c = &*this;
    let fill = |b: *mut c_void| {
        let hr = set_dword(b, "ProcessId", pid);
        if hr < 0 {
            return hr;
        }
        let hr = set_dword(b, "CaptureFlags", flags);
        if hr < 0 {
            return hr;
        }
        let hr = set_dword(b, "CommandListLatencyFrameCount", latency);
        if hr < 0 {
            return hr;
        }
        set_dword(b, "CaptureFrameCount", frames)
    };
    stream(c, OP_CAPTURE_NEXT_FRAME, Some(&fill), &c.capture_completed, cb, None)
}

unsafe extern "system" fn configure_sysmon(this: This, flags: u32, cb: Cb) -> HRESULT {
    let c = &*this;
    if (flags == 0) != cb.is_null() {
        return E_INVALIDARG;
    }
    let mut r = 0i32;
    stream(c, OP_CONFIGURE_SYSMON, Some(&|b| set_dword(b, "SysMonFlags", flags)), &c.sysmon_data, cb, Some(&mut r))
}

unsafe extern "system" fn get_sysmon_configuration(this: This, out: *mut u32) -> HRESULT {
    let c = &*this;
    *out = 0;
    let mut session = ComPtr::null();
    let hr = open(c, &mut session);
    if hr < 0 {
        return hr;
    }
    let mut body = ComPtr::null();
    let hr = request_reply(session.0, OP_GET_SYSMON_CONFIGURATION, Some(&|_| S_OK), &mut body);
    if hr < 0 {
        return hr;
    }
    get_dword(body.0, "SysMonFlags", &mut *out)
}

unsafe extern "system" fn track_sysmon_configuration(this: This, cb: Cb) -> HRESULT {
    let c = &*this;
    if cb.is_null() {
        c.sysmon_config.clear();
        return S_OK;
    }
    stream(c, OP_TRACK_SYSMON_CONFIGURATION, Some(&|b| set_int(b, "TrackingEnabled", 1)), &c.sysmon_config, cb, None)
}

unsafe extern "system" fn do_timing_capture(this: This, duration: u64, cb: Cb) -> HRESULT {
    let c = &*this;
    stream(c, OP_DO_TIMING_CAPTURE, Some(&|b| set_u64(b, "Duration", duration)), &c.timing_completed, cb, None)
}

unsafe extern "system" fn start_continuous_timing_capture(this: This, cb: Cb) -> HRESULT {
    let c = &*this;
    stream(c, OP_SET_CONTINUOUS_TIMING_CAPTURE, Some(&|b| set_int(b, "CaptureEnabled", 1)), &c.timing_range, cb, None)
}

unsafe extern "system" fn stop_continuous_timing_capture(this: This) -> HRESULT {
    let c = &*this;
    let mut session = ComPtr::null();
    let hr = open(c, &mut session);
    if hr < 0 {
        return hr;
    }
    let mut body = ComPtr::null();
    let hr = request_reply(session.0, OP_SET_CONTINUOUS_TIMING_CAPTURE, Some(&|b| set_int(b, "CaptureEnabled", 0)), &mut body);
    if hr < 0 {
        return hr;
    }
    let mut r = 0i32;
    let hr = get_result(body.0, &mut r);
    if hr < 0 {
        return hr;
    }
    if r < 0 {
        return r;
    }
    c.timing_range.clear();
    r
}

unsafe extern "system" fn get_continuous_timing_capture(this: This, start: u64, end: u64, cb: Cb) -> HRESULT {
    let c = &*this;
    let fill = |b: *mut c_void| {
        let hr = set_u64(b, "StartTime", start);
        if hr < 0 {
            return hr;
        }
        set_u64(b, "EndTime", end)
    };
    stream(c, OP_GET_CONTINUOUS_TIMING_CAPTURE, Some(&fill), &c.timing_completed, cb, None)
}

unsafe fn report_binary(c: &PixClient, st: &Mutex<Stream>, op: u32, n: u32, p: *const u8) -> HRESULT {
    let mut msg = ComPtr::null();
    let hr = XbtpCreateMessage(&IID_MESSAGE, msg.out());
    if hr < 0 {
        return hr;
    }
    let hr = msg_set_binary_body(msg.0, n, p);
    if hr < 0 {
        return hr;
    }
    let mut s = lock(st);
    if s.session.0.is_null() {
        let hr = open(c, &mut s.session);
        if hr < 0 {
            return hr;
        }
        let session = s.session.0;
        session_send_begin(session, op, msg.0, &mut s.exchange)
    } else {
        client_send_more(s.session.0, s.exchange, msg.0)
    }
}

unsafe extern "system" fn report_frame_counter_data(this: This, n: u32, p: *const u8) -> HRESULT {
    let c = &*this;
    report_binary(c, &c.frame, OP_FRAME_COUNTER_DATA, n, p)
}

unsafe extern "system" fn track_channel_status(this: This, cb: Cb, data: u32, force: i32) -> HRESULT {
    track_channel_status_with_fixed_token(this, cb, data, 0, force)
}

unsafe extern "system" fn announce_title_process(this: This, pid: u32, path: *const u16) -> HRESULT {
    let c = &*this;
    let mut session = ComPtr::null();
    let hr = open(c, &mut session);
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
    let hr = set_dword(body.0, "ProcessId", pid);
    if hr < 0 {
        return hr;
    }
    let hr = set_string(body.0, "ProcessPath", path);
    if hr < 0 {
        return hr;
    }
    let hr = serialize(body.0, msg.0, SET_JSON_BODY_W_REQUEST);
    if hr < 0 {
        return hr;
    }
    let mut reply = ComPtr::null();
    send_json_message(session.0, body.0, OP_PROCESS_CREATED, Some(&mut reply))
}

unsafe extern "system" fn track_graphics_capture_requests(this: This, cb: Cb) -> HRESULT {
    let c = &*this;
    let mut r = 0i32;
    stream(c, OP_TRACK_GRAPHICS_CAPTURE_REQUESTS, None, &c.capture_next_frame, cb, Some(&mut r))
}

unsafe extern "system" fn report_graphics_capture_result(this: This, pid: u32, result: i32, path: *const u16) -> HRESULT {
    let c = &*this;
    let mut session = ComPtr::null();
    let hr = open(c, &mut session);
    if hr < 0 {
        return hr;
    }
    let fill = |b: *mut c_void| {
        let hr = set_dword(b, "ProcessId", pid);
        if hr < 0 {
            return hr;
        }
        let hr = set_int(b, "Result", result);
        if hr < 0 {
            return hr;
        }
        set_string(b, "FilePath", path)
    };
    let mut body = ComPtr::null();
    let hr = request_reply(session.0, OP_CAPTURE_RESULT, Some(&fill), &mut body);
    if hr < 0 {
        return hr;
    }
    let mut r = 0i32;
    let hr = get_result(body.0, &mut r);
    if hr < 0 {
        return hr;
    }
    r
}

unsafe extern "system" fn report_xmem_statistics(this: This, n: u32, p: *const u8) -> HRESULT {
    let c = &*this;
    report_binary(c, &c.xmem, OP_XMEM_STATISTICS, n, p)
}

unsafe extern "system" fn get_info(this: This, out: *mut PixInfo) -> HRESULT {
    *out = (*this).info;
    S_OK
}

unsafe extern "system" fn set_paused_state(this: This, paused: i32) -> HRESULT {
    let c = &*this;
    let mut session = ComPtr::null();
    let hr = open(c, &mut session);
    if hr < 0 {
        return hr;
    }
    let mut status = 0u32;
    let mut reply = ComPtr::null();
    let hr = round_trip(session.0, OP_SET_PAUSE_STATE, Some(&|b| set_int(b, "PauseState", paused)), DEFAULT_RECEIVE_TIMEOUT_MS, &mut status, &mut reply);
    if hr < 0 {
        return hr;
    }
    if status == 0 {
        S_OK
    } else {
        E_FAIL
    }
}

unsafe extern "system" fn get_console_id(this: This, out: *mut BSTR) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let c = &*this;
    let mut session = ComPtr::null();
    let hr = open_vm_session(c.address.0, XBTP_PORT_HOST, &DIAGNOSTIC_INFORMATION_EXTENSION_ID, session.out());
    if hr < 0 {
        return hr;
    }
    let mut reply = ComPtr::null();
    let hr = send_json_message(session.0, null_mut(), OP_RETRIEVE_DIAGNOSTICS, Some(&mut reply));
    if hr < 0 {
        return hr;
    }
    let mut props = ComPtr::null();
    let hr = obj_get_object(reply.0, w("Properties").as_ptr(), props.out());
    if hr < 0 {
        return hr;
    }
    let mut names = ComPtr::null();
    let hr = obj_get_array(props.0, "Names", names.out());
    if hr < 0 {
        return hr;
    }
    let mut values = ComPtr::null();
    let hr = obj_get_array(props.0, "Values", values.out());
    if hr < 0 {
        return hr;
    }
    let mut count = 0u32;
    let mut hr = arr_get_count(names.0, &mut count);
    if hr < 0 {
        return hr;
    }
    let key = w("ConsoleId");
    for i in 0..count {
        let mut name = Bstr::null();
        hr = arr_get_string(names.0, i, &mut name.0);
        if hr < 0 {
            break;
        }
        if _wcsicmp(name.0, key.as_ptr()) == 0 {
            hr = arr_get_string(values.0, i, out);
            break;
        }
    }
    hr
}

unsafe extern "system" fn do_timing_capture_ex(this: This, duration: u64, flags: u32, cb: Cb) -> HRESULT {
    let c = &*this;
    let fill = |b: *mut c_void| {
        let hr = set_u64(b, "Duration", duration);
        if hr < 0 {
            return hr;
        }
        set_dword(b, "CaptureFlags", flags)
    };
    stream(c, OP_DO_TIMING_CAPTURE, Some(&fill), &c.timing_completed, cb, None)
}

unsafe extern "system" fn start_continuous_timing_capture_ex(this: This, flags: u32, cb: Cb) -> HRESULT {
    let c = &*this;
    let fill = |b: *mut c_void| {
        let hr = set_int(b, "CaptureEnabled", 1);
        if hr < 0 {
            return hr;
        }
        set_dword(b, "CaptureFlags", flags)
    };
    stream(c, OP_SET_CONTINUOUS_TIMING_CAPTURE, Some(&fill), &c.timing_range, cb, None)
}

unsafe extern "system" fn get_client_version(_this: This, out: *mut u32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = PIX_CLIENT_VERSION;
    S_OK
}

unsafe extern "system" fn get_friendly_version(this: This, fv: *mut BSTR, fcv: *mut BSTR, branch: *mut BSTR, xdk: *mut u32) -> HRESULT {
    if fv.is_null() || fcv.is_null() {
        return E_POINTER;
    }
    let c = &*this;
    let mut session = ComPtr::null();
    let hr = open(c, &mut session);
    if hr < 0 {
        return hr;
    }
    let mut body = ComPtr::null();
    let hr = request_reply(session.0, OP_GET_FRIENDLY_VERSION, Some(&|_| S_OK), &mut body);
    if hr < 0 {
        return hr;
    }
    let mut a = Bstr::null();
    let hr = get_str(body.0, "FriendlyVersionProperty", &mut a);
    if hr < 0 {
        return hr;
    }
    let mut b = Bstr::null();
    let hr = get_str(body.0, "FriendlyCompactVersionProperty", &mut b);
    if hr < 0 {
        return hr;
    }
    let mut d = 0.0f64;
    let hr = obj_get_number(body.0, w("XDKEditionProperty").as_ptr(), &mut d);
    if hr < 0 {
        return hr;
    }
    let mut br = Bstr::null();
    let hr = get_str(body.0, "BuildBranchProperty", &mut br);
    if hr < 0 {
        return hr;
    }
    *fv = core::mem::replace(&mut a.0, null_mut());
    *fcv = core::mem::replace(&mut b.0, null_mut());
    *branch = core::mem::replace(&mut br.0, null_mut());
    *xdk = cvtt64(d) as u32;
    hr
}

unsafe extern "system" fn track_channel_status_with_fixed_token(this: This, cb: Cb, data: u32, token: u64, force: i32) -> HRESULT {
    let c = &*this;
    if cb.is_null() {
        let mut hr = S_OK;
        let held = c.connection_token.load(Ordering::Acquire);
        if c.port == XBTP_PORT_HOST && held != 0 {
            let mut session = ComPtr::null();
            hr = open(c, &mut session);
            if hr < 0 {
                return hr;
            }
            let mut exchange = 0u32;
            hr = begin(session.0, OP_RELEASE_EXCLUSIVE_CONNECTION, Some(&|b| set_u64(b, "ConnectionToken", held)), &mut exchange);
            if hr < 0 {
                return hr;
            }
        }
        c.channel_status.clear();
        return hr;
    }
    let mut session = ComPtr::null();
    let hr = open(c, &mut session);
    if hr < 0 {
        return hr;
    }
    if c.port == XBTP_PORT_HOST {
        let fill = |b: *mut c_void| {
            let hr = set_int(b, "ForceConnect", force);
            if hr < 0 {
                return hr;
            }
            set_u64(b, "ConnectionToken", token)
        };
        let mut status = 0u32;
        let mut reply = ComPtr::null();
        let hr = round_trip(session.0, OP_CREATE_EXCLUSIVE_CONNECTION, Some(&fill), DEFAULT_RECEIVE_TIMEOUT_MS, &mut status, &mut reply);
        if hr < 0 {
            return hr;
        }
        if status != STATUS_EXCLUSIVE_UNSUPPORTED {
            if status != 0 {
                return ((status & 0xff) | 0x8c11_0100) as i32;
            }
            let mut body = ComPtr::null();
            let hr = deserialize(reply.0, &mut body);
            if hr < 0 {
                return hr;
            }
            let mut r = 0i32;
            let hr = get_result(body.0, &mut r);
            if hr < 0 {
                return hr;
            }
            let mut granted = 0u64;
            let hr = get_u64(body.0, "ConnectionToken", &mut granted);
            if hr < 0 {
                return hr;
            }
            if r < 0 {
                return r;
            }
            c.connection_token.store(token, Ordering::Release);
        }
    }
    c.channel_status.start_channel(session.0, cb, data)
}

unsafe fn version_handshake(c: &mut PixClient) -> HRESULT {
    let mut session = ComPtr::null();
    let hr = open(c, &mut session);
    if hr < 0 {
        return hr;
    }
    let mut body = ComPtr::null();
    let hr = request_reply(session.0, OP_VERSION_HANDSHAKE, Some(&|b| set_dword(b, "Version", PIX_CLIENT_VERSION)), &mut body);
    if hr < 0 {
        return hr;
    }
    let mut version = 0u32;
    let hr = get_dword(body.0, "Version", &mut version);
    if hr < 0 {
        return hr;
    }
    let mut result = 0i32;
    let hr = get_result(body.0, &mut result);
    if hr < 0 {
        return hr;
    }
    let mut freq = 0u64;
    get_u64(body.0, "CpuFrequency", &mut freq);
    let mut system_type = 0i32;
    get_bool(body.0, "SystemType", &mut system_type);
    if result < 0 {
        return result;
    }
    c.info.server_version = version;
    c.info.system_type = system_type as i16 as i32;
    c.info.cpu_frequency = freq;
    result
}

fn is_local(address: *const u16) -> bool {
    if address.is_null() {
        return false;
    }
    let s = unsafe { core::slice::from_raw_parts(address, wlen(address)) };
    s.iter().copied().eq("local".encode_utf16())
}

#[no_mangle]
pub unsafe extern "system" fn XtfCreatePixClient(address: *const u16, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    *ppv = null_mut();
    let localhost = w("localhost");
    let addr = if is_local(address) { localhost.as_ptr() } else { address };
    let p = Box::into_raw(Box::new(PixClient {
        vtbl: &PIX_CLIENT_VTBL,
        refs: AtomicU32::new(1),
        address: Bstr::null(),
        port: 0,
        info: PixInfo::default(),
        connection_token: AtomicU64::new(0),
        frame: Mutex::new(Stream { session: ComPtr::null(), exchange: 0 }),
        xmem: Mutex::new(Stream { session: ComPtr::null(), exchange: 0 }),
        capture_next_frame: Listener::new(Kind::CaptureNextFrame, INFINITE),
        channel_status: Listener::new(Kind::ChannelStatus, CHANNEL_STATUS_TIMEOUT_MS),
        timing_range: Listener::new(Kind::TimingRange, INFINITE),
        timing_completed: Listener::new(Kind::TimingCompleted, INFINITE),
        sysmon_config: Listener::new(Kind::SysMonConfigChanged, INFINITE),
        sysmon_data: Listener::new(Kind::SysMonData, INFINITE),
        capture_completed: Listener::new(Kind::CaptureCompleted, INFINITE),
        track_processes: Listener::new(Kind::TrackProcesses, INFINITE),
    }));
    let c = &mut *p;
    let token = &c.connection_token as *const AtomicU64;
    for l in [&mut c.capture_next_frame, &mut c.channel_status, &mut c.timing_range, &mut c.timing_completed, &mut c.sysmon_config, &mut c.sysmon_data, &mut c.capture_completed, &mut c.track_processes] {
        l.token = token;
    }
    let mut port = 0u32;
    let hr = split_address_port(addr, XBTP_PORT_TITLE, &mut c.address, &mut port);
    c.port = port;
    if hr < 0 {
        c_release(p);
        return hr;
    }
    let hr = version_handshake(c);
    if hr < 0 {
        c_release(p);
        return hr;
    }
    let hr = c_qi(p, riid, ppv);
    c_release(p);
    hr
}

#[no_mangle]
pub unsafe extern "system" fn XtfCreatePixClientQuiet(address: *const u16, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    let old = log_get_main_level();
    log_set_main_level(0);
    let hr = XtfCreatePixClient(address, riid, ppv);
    log_set_main_level(old as u32);
    hr
}

#[cfg(test)]
pub unsafe fn client_port(p: *mut c_void) -> u32 {
    (*(p as This)).port
}

#[cfg(test)]
pub unsafe fn client_token(p: *mut c_void) -> u64 {
    (*(p as This)).connection_token.load(Ordering::Acquire)
}
