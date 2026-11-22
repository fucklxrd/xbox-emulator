use crate::sys::*;
use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicI32, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub const STATUS_CANCELLED: i64 = 0xC000_0120u32 as i64;
pub const PROCESS_BREAK_ON_TERMINATION: u32 = 0x1D;

pub static SYSTEM_GONE_DARK: AtomicI32 = AtomicI32::new(0);
pub static TCP_BYTES_RECEIVED: AtomicU64 = AtomicU64::new(0);
pub static TCP_BYTES_SENT: AtomicU64 = AtomicU64::new(0);
pub static TLS_EVENT_INDEX: AtomicU32 = AtomicU32::new(0xFFFF_FFFF);

pub const PIPE_TO_TITLE: &str = r"\\.\pipe\XtfLoopback";
pub const PIPE_HOST_TO_TITLE: &str = r"\\.\HEVPIPE\XTF_HOST_TO_TITLE";
pub const PIPE_TO_SYSTEM: &str = r"\\.\SEVPIPE\XTF_TITLE_TO_SYSTEM";
pub const PIPE_SYSTEM_TO_TITLE: &str = r"\\.\SEVPIPE\XTF_SYSTEM_TO_TITLE";
pub const PIPE_TO_HOST: &str = r"\\.\HEVPIPE\XTF_TITLE_TO_HOST";

use crate::link::WSAStartup;

#[no_mangle]
pub unsafe extern "C" fn XbtpSetSystemGoDark(dark: u8) {
    SYSTEM_GONE_DARK.swap(dark as i32, Ordering::SeqCst);
}

#[no_mangle]
pub unsafe extern "C" fn XbtpHasSystemGoneDark() -> i32 {
    (SYSTEM_GONE_DARK.load(Ordering::SeqCst) != 0) as i32
}

#[no_mangle]
pub unsafe extern "C" fn XbtpIsSystemReady() -> i32 {
    (SYSTEM_GONE_DARK.load(Ordering::SeqCst) == 0) as i32
}

#[no_mangle]
pub unsafe extern "C" fn XbtpWaitSystemReady() -> HRESULT {
    if SYSTEM_GONE_DARK.load(Ordering::SeqCst) != 0 {
        E_PENDING
    } else {
        S_OK
    }
}

pub type NtSetInfoFn = unsafe extern "system" fn(HANDLE, u32, *const c_void, u32) -> i32;

unsafe extern "system" fn nt_set_info(h: HANDLE, class: u32, info: *const c_void, len: u32) -> i32 {
    NtSetInformationProcess(h, class, info, len)
}

pub static NT_SET_INFO: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

unsafe fn nt_set_info_fn() -> NtSetInfoFn {
    let p = NT_SET_INFO.load(Ordering::Acquire);
    if p.is_null() {
        nt_set_info
    } else {
        core::mem::transmute::<*mut c_void, NtSetInfoFn>(p)
    }
}

#[no_mangle]
pub unsafe extern "C" fn XbtpMarkToolingCritical(critical: u8) -> HRESULT {
    let value: u32 = critical as u32;
    let st = nt_set_info_fn()(GetCurrentProcess(), PROCESS_BREAK_ON_TERMINATION, &value as *const u32 as *const c_void, 4);
    st | 0x1000_0000
}

#[export_name = "?XbtpOpenXtcSession@@YAJ_KAEBU_GUID@@1PEAPEAX@Z"]
pub unsafe extern "C" fn XbtpOpenXtcSession(_id: u64, _session: *const GUID, _extension: *const GUID, out: *mut *mut c_void) -> HRESULT {
    *out = null_mut();
    E_NOTIMPL
}

unsafe fn pipe_name(src: &str, out: *mut BSTR) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let mut b: BSTR = null_mut();
    if !src.is_empty() {
        b = bstr_from(src);
        if b.is_null() {
            return E_OUTOFMEMORY;
        }
    }
    *out = b;
    S_OK
}

#[export_name = "?XbtpGetPipeNameToHost@@YAJPEAPEAG@Z"]
pub unsafe extern "C" fn XbtpGetPipeNameToHost(out: *mut BSTR) -> HRESULT {
    pipe_name(PIPE_TO_HOST, out)
}

#[export_name = "?XbtpGetPipeNameToSystem@@YAJPEAPEAG@Z"]
pub unsafe extern "C" fn XbtpGetPipeNameToSystem(out: *mut BSTR) -> HRESULT {
    pipe_name(PIPE_TO_SYSTEM, out)
}

#[export_name = "?XbtpGetPipeNameToTitle@@YAJPEAPEAG@Z"]
pub unsafe extern "C" fn XbtpGetPipeNameToTitle(out: *mut BSTR) -> HRESULT {
    pipe_name(PIPE_TO_TITLE, out)
}

#[no_mangle]
pub unsafe extern "C" fn XbtpGetTcpNetworkTrafficStatistics(received: *mut u64, sent: *mut u64) -> HRESULT {
    if received.is_null() {
        return E_POINTER;
    }
    if sent.is_null() {
        return E_POINTER;
    }
    *received = TCP_BYTES_RECEIVED.load(Ordering::SeqCst);
    *sent = TCP_BYTES_SENT.load(Ordering::SeqCst);
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn XbtpGetCurrentTlsEventData(out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = null_mut();
    let v = TlsGetValue(TLS_EVENT_INDEX.load(Ordering::SeqCst));
    if v.is_null() {
        return E_OUTOFMEMORY;
    }
    *out = v;
    S_OK
}

#[repr(C)]
pub struct ChannelRequest {
    pub process: unsafe fn(*mut ChannelRequest) -> HRESULT,
    pub status: i64,
    pub event: HANDLE,
    pub next: *mut ChannelRequest,
    pub context: *mut c_void,
}

struct ChannelSystem {
    running: bool,
    head: *mut ChannelRequest,
    wake: HANDLE,
}

unsafe impl Send for ChannelSystem {}

static CHANNELS: Mutex<ChannelSystem> = Mutex::new(ChannelSystem { running: false, head: null_mut(), wake: null_mut() });
static CHANNEL_THREAD: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

fn channel_worker() {
    loop {
        let wake = {
            let g = CHANNELS.lock().unwrap();
            if !g.running {
                return;
            }
            g.wake
        };
        unsafe { WaitForSingleObject(wake, 0xFFFF_FFFF) };
        let mut g = CHANNELS.lock().unwrap();
        while !g.head.is_null() {
            let r = g.head;
            unsafe {
                g.head = (*r).next;
                let hr = ((*r).process)(r);
                if hr < 0 && hr != E_PENDING {
                    SetEvent((*r).event);
                }
            }
        }
        if !g.running {
            return;
        }
    }
}

pub unsafe fn channel_initialize_system() -> HRESULT {
    let wake = CreateEventW(null_mut(), 0, 0, core::ptr::null());
    {
        let mut g = CHANNELS.lock().unwrap();
        g.running = true;
        g.wake = wake;
    }
    if wake.is_null() {
        channel_shutdown_system();
        return E_OUTOFMEMORY;
    }
    match std::thread::Builder::new().spawn(channel_worker) {
        Ok(h) => {
            *CHANNEL_THREAD.lock().unwrap() = Some(h);
            S_OK
        }
        Err(_) => {
            channel_shutdown_system();
            E_OUTOFMEMORY
        }
    }
}

pub unsafe fn channel_shutdown_system() {
    let wake = {
        let mut g = CHANNELS.lock().unwrap();
        g.running = false;
        g.wake
    };
    if !wake.is_null() {
        SetEvent(wake);
    }
    if let Some(h) = CHANNEL_THREAD.lock().unwrap().take() {
        let _ = h.join();
    }
    let mut g = CHANNELS.lock().unwrap();
    while !g.head.is_null() {
        let r = g.head;
        g.head = (*r).next;
        (*r).status = STATUS_CANCELLED;
        SetEvent((*r).event);
    }
    if !g.wake.is_null() {
        CloseHandle(g.wake);
    }
    g.wake = null_mut();
}

pub unsafe fn channel_submit(r: *mut ChannelRequest) {
    let wake = {
        let mut g = CHANNELS.lock().unwrap();
        if g.running {
            (*r).next = g.head;
            g.head = r;
        } else {
            (*r).status = STATUS_CANCELLED;
            SetEvent((*r).event);
        }
        g.wake
    };
    if !wake.is_null() {
        SetEvent(wake);
    }
}

pub unsafe fn channel_cancel_request<F: FnOnce()>(r: *mut ChannelRequest, issued: F) {
    let mut g = CHANNELS.lock().unwrap_or_else(|e| e.into_inner());
    let mut prev: *mut *mut ChannelRequest = &mut g.head;
    while !(*prev).is_null() {
        if *prev == r {
            *prev = (*r).next;
            (*r).next = null_mut();
            (*r).status = STATUS_CANCELLED;
            SetEvent((*r).event);
            return;
        }
        prev = &mut (**prev).next;
    }
    issued();
    drop(g);
}

static TLS_ONCE: OnceLock<HRESULT> = OnceLock::new();
static INIT: OnceLock<HRESULT> = OnceLock::new();

unsafe fn initialize_tls() -> HRESULT {
    *TLS_ONCE.get_or_init(|| {
        let i = TlsAlloc();
        TLS_EVENT_INDEX.store(i, Ordering::SeqCst);
        if i == 0xFFFF_FFFF {
            hr_last_error()
        } else {
            S_OK
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn XbtpInitialize() -> HRESULT {
    *INIT.get_or_init(|| {
        let mut data = [0u8; 0x198];
        let mut hr = hr_from_win32(WSAStartup(0x202, data.as_mut_ptr()) as u32);
        if hr >= 0 {
            hr = initialize_tls();
        }
        if hr >= 0 {
            hr = channel_initialize_system();
        }
        hr
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    static DARK_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn set_system_go_dark_swaps_flag() {
        let _g = DARK_LOCK.lock().unwrap();
        unsafe {
            XbtpSetSystemGoDark(7);
            assert_eq!(SYSTEM_GONE_DARK.load(Ordering::SeqCst), 7);
            XbtpSetSystemGoDark(0);
            assert_eq!(SYSTEM_GONE_DARK.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn has_system_gone_dark_reads_flag() {
        let _g = DARK_LOCK.lock().unwrap();
        unsafe {
            XbtpSetSystemGoDark(1);
            assert_eq!(XbtpHasSystemGoneDark(), 1);
            XbtpSetSystemGoDark(0);
            assert_eq!(XbtpHasSystemGoneDark(), 0);
        }
    }

    #[test]
    fn is_system_ready_is_inverse() {
        let _g = DARK_LOCK.lock().unwrap();
        unsafe {
            XbtpSetSystemGoDark(1);
            assert_eq!(XbtpIsSystemReady(), 0);
            XbtpSetSystemGoDark(0);
            assert_eq!(XbtpIsSystemReady(), 1);
        }
    }

    #[test]
    fn wait_system_ready_returns_pending_when_dark() {
        let _g = DARK_LOCK.lock().unwrap();
        unsafe {
            XbtpSetSystemGoDark(1);
            assert_eq!(XbtpWaitSystemReady(), E_PENDING);
            XbtpSetSystemGoDark(0);
            assert_eq!(XbtpWaitSystemReady(), S_OK);
        }
    }

    static SEEN: Mutex<(u32, u32, u32)> = Mutex::new((0, 0, 0));

    unsafe extern "system" fn fake_nt(_h: HANDLE, class: u32, info: *const c_void, len: u32) -> i32 {
        *SEEN.lock().unwrap() = (class, *(info as *const u32), len);
        0xC000_0022u32 as i32
    }

    #[test]
    fn mark_tooling_critical_sets_break_on_termination() {
        unsafe {
            NT_SET_INFO.store(fake_nt as *mut c_void, Ordering::Release);
            let hr = XbtpMarkToolingCritical(1);
            NT_SET_INFO.store(null_mut(), Ordering::Release);
            assert_eq!(*SEEN.lock().unwrap(), (0x1D, 1, 4));
            assert_eq!(hr as u32, 0xD000_0022);
        }
    }

    #[test]
    fn open_xtc_session_is_not_implemented_on_console() {
        unsafe {
            let mut out: *mut c_void = 1 as *mut c_void;
            let g = GUID::default();
            assert_eq!(XbtpOpenXtcSession(0, &g, &g, &mut out), E_NOTIMPL);
            assert!(out.is_null());
        }
    }

    unsafe fn check_pipe(f: unsafe extern "C" fn(*mut BSTR) -> HRESULT, want: &str) {
        assert_eq!(f(null_mut()), E_INVALIDARG);
        let mut b: BSTR = null_mut();
        assert_eq!(f(&mut b), S_OK);
        assert_eq!(from_wide(b), want);
        SysFreeString(b);
    }

    #[test]
    fn pipe_name_to_host() {
        unsafe { check_pipe(XbtpGetPipeNameToHost, r"\\.\HEVPIPE\XTF_TITLE_TO_HOST") }
    }

    #[test]
    fn pipe_name_to_system() {
        unsafe { check_pipe(XbtpGetPipeNameToSystem, r"\\.\SEVPIPE\XTF_TITLE_TO_SYSTEM") }
    }

    #[test]
    fn pipe_name_to_title() {
        unsafe { check_pipe(XbtpGetPipeNameToTitle, r"\\.\pipe\XtfLoopback") }
    }

    #[test]
    fn tcp_traffic_statistics() {
        unsafe {
            let mut r = 0u64;
            let mut s = 0u64;
            assert_eq!(XbtpGetTcpNetworkTrafficStatistics(null_mut(), &mut s), E_POINTER);
            assert_eq!(XbtpGetTcpNetworkTrafficStatistics(&mut r, null_mut()), E_POINTER);
            TCP_BYTES_RECEIVED.fetch_add(10, Ordering::SeqCst);
            TCP_BYTES_SENT.fetch_add(20, Ordering::SeqCst);
            assert_eq!(XbtpGetTcpNetworkTrafficStatistics(&mut r, &mut s), S_OK);
            assert!(r >= 10 && s >= 20);
        }
    }

    #[test]
    fn current_tls_event_data() {
        unsafe {
            assert_eq!(XbtpGetCurrentTlsEventData(null_mut()), E_INVALIDARG);
            assert!(XbtpInitialize() >= 0);
            let mut out: *mut c_void = 1 as *mut c_void;
            let idx = TLS_EVENT_INDEX.load(Ordering::SeqCst);
            std::thread::spawn(move || {
                let mut o: *mut c_void = 1 as *mut c_void;
                assert_eq!(XbtpGetCurrentTlsEventData(&mut o), E_OUTOFMEMORY);
                assert!(o.is_null());
                let mut v = 5u32;
                TlsSetValue(idx, &mut v as *mut u32 as *mut c_void);
                assert_eq!(XbtpGetCurrentTlsEventData(&mut o), S_OK);
                assert_eq!(o, &mut v as *mut u32 as *mut c_void);
            })
            .join()
            .unwrap();
            let _ = &mut out;
        }
    }

    static PROCESSED: AtomicU32 = AtomicU32::new(0);

    unsafe fn fail_req(r: *mut ChannelRequest) -> HRESULT {
        PROCESSED.fetch_add(1, Ordering::SeqCst);
        (*r).status = 0x55;
        E_FAIL
    }

    #[test]
    fn initialize_starts_channel_system() {
        unsafe {
            assert_eq!(XbtpInitialize(), S_OK);
            assert_eq!(XbtpInitialize(), S_OK);
            assert_ne!(TLS_EVENT_INDEX.load(Ordering::SeqCst), 0xFFFF_FFFF);
            let ev = CreateEventW(null_mut(), 1, 0, core::ptr::null());
            let mut r = ChannelRequest { process: fail_req, status: 0, event: ev, next: null_mut(), context: null_mut() };
            channel_submit(&mut r);
            assert_eq!(WaitForSingleObject(ev, 5000), 0);
            assert_eq!(r.status, 0x55);
            assert!(PROCESSED.load(Ordering::SeqCst) >= 1);
            CloseHandle(ev);
        }
    }
}
