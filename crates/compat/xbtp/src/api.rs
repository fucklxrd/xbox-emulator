use crate::adapter::*;
use crate::base::log_error_args;
use crate::conn::{connection_delete, NpConnection_InitializeClient_sp, TcpConnection_InitializeClient_sp, XbtpChannel, XbtpConnection, NpConnection, TcpConnection};
use crate::link_conn::link_initialize_client;
use crate::msvc::SharedPtr;
use crate::session::*;
use crate::session_impl::wrap_session;
use crate::sys::*;
use core::ffi::c_void;
use core::ptr::null_mut;

pub const PORT_KIND_MASK: u32 = 0xff00_0000;
pub const PORT_KIND_TCP: u32 = 0;
pub const PORT_KIND_LINK: u32 = 0x0100_0000;
pub const DEFAULT_HOST: &str = "localhost";

pub fn get_server_port(server_port: u32) -> Result<u32, HRESULT> {
    let kind = server_port & PORT_KIND_MASK;
    let port = match server_port & 0x00ff_ffff {
        0 => 0x1073,
        1 => 0x107d,
        3 => 0x1069,
        _ => return Err(E_INVALIDARG),
    };
    Ok(port | kind)
}

unsafe fn drop_adapter(p: *mut c_void) {
    adapter_delete(p as *mut XbtpClientAdapter);
}

unsafe fn drop_session(p: *mut c_void) {
    session_delete(p as *mut XbtpSession);
}

#[export_name = "?XbtpCompleteSessionSetupInternal@@YAJAEBU_GUID@@_NIV?$shared_ptr@VXbtpConnection@@@std@@PEAPEAVXbtpSession@@@Z"]
pub unsafe extern "C" fn XbtpCompleteSessionSetupInternal(extension: *const GUID, quiet: bool, timeout: u32, connection: *mut SharedPtr<XbtpConnection>, out: *mut *mut XbtpSession) -> HRESULT {
    let conn = SharedPtr::take(connection);
    if out.is_null() {
        return E_INVALIDARG;
    }
    let session = new_session();
    *out = session;
    let hr = 'run: {
        if session.is_null() {
            break 'run E_OUTOFMEMORY;
        }
        let a = new_adapter();
        if a.is_null() {
            break 'run E_OUTOFMEMORY;
        }
        let adapter = match SharedPtr::new_with(a, drop_adapter) {
            Some(sp) => sp,
            None => {
                adapter_delete(a);
                break 'run E_OUTOFMEMORY;
            }
        };
        (*a).quiet = quiet as u8;
        let mut c = conn.clone_ref();
        let hr = XbtpClientAdapter_Initialize(a, &mut c);
        if hr < 0 {
            if !quiet {
                log_error_args(b"Failed 0x%X to initialize client adapter\0", &[hr as u32 as usize]);
            }
            break 'run hr;
        }
        let hr = XbtpClientAdapter_ConnectTimeout(a, timeout);
        if hr < 0 {
            if !quiet {
                log_error_args(b"Failed 0x%X to negotiate connect\0", &[hr as u32 as usize]);
            }
            break 'run hr;
        }
        let hr = XbtpClientAdapter_BeginSessionTimeout(a, extension, timeout);
        if hr < 0 {
            if !quiet {
                let g = guid_args(&*extension);
                let mut args = vec![hr as u32 as usize];
                args.extend_from_slice(&g);
                log_error_args(b"Failed 0x%X to negotiate session for extension {%08X-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X}\0", &args);
            }
            break 'run hr;
        }
        let mut ad = adapter.clone_ref();
        let mut ch: SharedPtr<XbtpChannel> = (*a).channel.clone_ref();
        XbtpSession_Initialize(session, &(*a).session_id, &mut ad, &mut ch)
    };
    if hr < 0 && !(*out).is_null() {
        session_delete(*out);
        *out = null_mut();
    }
    drop(conn);
    hr
}

#[export_name = "?XbtpOpenSessionInternal@@YAJAEBU_GUID@@PEBGKPEAPEAVXbtpSession@@_NI@Z"]
pub unsafe extern "C" fn XbtpOpenSessionInternal(extension: *const GUID, address: *const u16, port: u32, out: *mut *mut XbtpSession, quiet: bool, timeout: u32) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let host = wide(DEFAULT_HOST);
    let addr = if address.is_null() { host.as_ptr() } else { address };
    let kind = port & PORT_KIND_MASK;
    let number = port & 0x00ff_ffff;
    let hr = if kind == PORT_KIND_LINK {
        let mut c = SharedPtr::<XbtpConnection>::default();
        let hr = link_initialize_client(addr, number, &mut c);
        if hr < 0 {
            hr
        } else {
            XbtpCompleteSessionSetupInternal(extension, quiet, timeout, &mut c, out)
        }
    } else if kind == PORT_KIND_TCP {
        let mut t = SharedPtr::<TcpConnection>::default();
        let hr = TcpConnection_InitializeClient_sp(addr, number, &mut t);
        if hr < 0 {
            hr
        } else {
            let mut c: SharedPtr<XbtpConnection> = t.cast();
            XbtpCompleteSessionSetupInternal(extension, quiet, timeout, &mut c, out)
        }
    } else {
        return E_INVALIDARG;
    };
    if hr < 0 && !quiet {
        log_error_args(b"Failed 0x%X to initialize the client to %hs :: %d\0", &[hr as u32 as usize, addr as usize, port as usize]);
    }
    hr
}

#[export_name = "?XbtpOpenSession@@YAJAEBU_GUID@@PEBGKPEAPEAVXbtpSession@@_N@Z"]
pub unsafe extern "C" fn XbtpOpenSessionCpp(extension: *const GUID, address: *const u16, port: u32, out: *mut *mut XbtpSession, quiet: bool) -> HRESULT {
    XbtpOpenSessionInternal(extension, address, port, out, quiet, DEFAULT_TIMEOUT_MS)
}

#[export_name = "?XbtpOpenPipeSession@@YAJPEBGAEBU_GUID@@PEAV?$shared_ptr@VXbtpSession@@@std@@@Z"]
pub unsafe extern "C" fn XbtpOpenPipeSessionShared(path: *const u16, extension: *const GUID, out: *mut SharedPtr<XbtpSession>) -> HRESULT {
    if path.is_null() || *path == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    let mut np = SharedPtr::<NpConnection>::default();
    let hr = NpConnection_InitializeClient_sp(path, &mut np);
    if hr < 0 {
        log_error_args(b"Failed 0x%X to initialize the client to %ws\0", &[hr as u32 as usize, path as usize]);
        return hr;
    }
    let mut c: SharedPtr<XbtpConnection> = np.cast();
    let mut raw: *mut XbtpSession = null_mut();
    let hr = XbtpCompleteSessionSetupInternal(extension, false, DEFAULT_TIMEOUT_MS, &mut c, &mut raw);
    if hr < 0 {
        return hr;
    }
    match SharedPtr::new_with(raw, drop_session) {
        Some(sp) => {
            sp.assign_into(out);
            hr
        }
        None => {
            session_delete(raw);
            E_OUTOFMEMORY
        }
    }
}

#[export_name = "?XbtpOpenPipeSession@@YAJPEBGAEBU_GUID@@1PEAPEAX@Z"]
pub unsafe extern "C" fn XbtpOpenPipeSession(path: *const u16, extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if path.is_null() || *path == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    let mut session = SharedPtr::<XbtpSession>::default();
    let hr = XbtpOpenPipeSessionShared(path, extension, &mut session);
    if hr < 0 {
        return hr;
    }
    wrap_session(session, iid, out)
}

pub unsafe fn delete_connection(c: *mut XbtpConnection) {
    connection_delete(c);
}

pub const OPEN_OPTIONS_SIZE: u32 = 0x58;
pub const OPEN_OPTIONS_SIZE_V1: u32 = 0x44;
pub const SESSION_KEY_CHARS: usize = 0x20;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct XBTP_OPEN_SESSION_OPTIONS {
    pub size: u32,
    pub session_key: [u16; SESSION_KEY_CHARS],
    pub skip_ready_check: u32,
    pub allow_unneeded_key: u32,
    pub quiet: u32,
    pub reserved: u32,
    pub timeout_ms: u32,
}

impl Default for XBTP_OPEN_SESSION_OPTIONS {
    fn default() -> Self {
        XBTP_OPEN_SESSION_OPTIONS { size: OPEN_OPTIONS_SIZE, session_key: [0; SESSION_KEY_CHARS], skip_ready_check: 0, allow_unneeded_key: 0, quiet: 0, reserved: 0, timeout_ms: 0 }
    }
}

#[no_mangle]
pub unsafe extern "C" fn XbtpOpenSessionEx2(address: *const u16, port: u32, options: *const XBTP_OPEN_SESSION_OPTIONS, extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let mut key: *const u16 = core::ptr::null();
    let mut hs = crate::handshake::HandshakeOptions { skip_ready_check: false, allow_unneeded_key: false };
    let mut quiet = false;
    let mut timeout = DEFAULT_TIMEOUT_MS;
    if !options.is_null() {
        let size = (*options).size;
        if size != OPEN_OPTIONS_SIZE && size != OPEN_OPTIONS_SIZE_V1 {
            return E_INVALIDARG;
        }
        key = (*options).session_key.as_ptr();
        if size == OPEN_OPTIONS_SIZE {
            hs.skip_ready_check = (*options).skip_ready_check != 0;
            hs.allow_unneeded_key = (*options).allow_unneeded_key != 0;
            quiet = (*options).quiet != 0;
            if (*options).timeout_ms != 0 {
                timeout = (*options).timeout_ms;
            }
        }
    }
    let hr = crate::xbtp2_sys::XbtpInitialize();
    if hr < 0 {
        return hr;
    }
    let mut raw: *mut XbtpSession = null_mut();
    let hr = XbtpOpenSessionInternal(extension, address, port, &mut raw, quiet, timeout);
    if hr < 0 {
        return hr;
    }
    let session = match SharedPtr::new_with(raw, drop_session) {
        Some(s) => s,
        None => {
            session_delete(raw);
            return E_OUTOFMEMORY;
        }
    };
    let imp = crate::session_impl::new_session_impl(session) as *mut c_void;
    let hr = crate::xbtp2_ext::com_qi(imp, iid, out);
    if hr < 0 {
        crate::xbtp2_ext::com_release(imp);
        return hr;
    }
    let hr = crate::handshake::client_handshake(imp, key, &hs);
    crate::xbtp2_ext::com_release(imp);
    hr
}

#[no_mangle]
pub unsafe extern "C" fn XbtpOpenSessionEx(address: *const u16, port: u32, extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    XbtpOpenSessionEx2(address, port, core::ptr::null(), extension, iid, out)
}

#[no_mangle]
pub unsafe extern "C" fn XbtpOpenSession2(address: *const u16, server_port: u32, options: *const XBTP_OPEN_SESSION_OPTIONS, extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    let port = match get_server_port(server_port) {
        Ok(p) => p,
        Err(hr) => return hr,
    };
    XbtpOpenSessionEx2(address, port, options, extension, iid, out)
}

#[no_mangle]
pub unsafe extern "C" fn XbtpOpenSession(address: *const u16, server_port: u32, extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    let mut opts = XBTP_OPEN_SESSION_OPTIONS::default();
    let mut prefix: BSTR = null_mut();
    let mut addr = address;
    if !address.is_null() {
        let n = wcslen(address);
        let s = core::slice::from_raw_parts(address, n);
        if let Some(i) = s.iter().position(|&c| c == b'+' as u16) {
            let tail = &s[i + 1..];
            if tail.len() >= SESSION_KEY_CHARS {
                return 0x8007_0057u32 as i32;
            }
            opts.session_key[..tail.len()].copy_from_slice(tail);
            if i != 0 {
                prefix = SysAllocStringLen(address, i as u32);
                if prefix.is_null() {
                    return E_OUTOFMEMORY;
                }
            }
            addr = prefix;
        }
    }
    let hr = XbtpOpenSession2(addr, server_port, &opts, extension, iid, out);
    if !prefix.is_null() {
        SysFreeString(prefix);
    }
    hr
}
