use core::ffi::c_void;
use core::ptr::null_mut;
use xbtp::api::*;
use xbtp::conn::{TcpConnection, TcpConnection_InitializeServer_sp, XbtpConnection};
use xbtp::dispatcher::*;
use xbtp::dispatcher_com::*;
use xbtp::handshake::E_SESSION_KEY_MISMATCH;
use xbtp::link_conn::link_initialize_server;
use xbtp::msvc::{cpp_delete, cpp_new, SharedPtr};
use xbtp::registry::{XbtpExtensionRegistry, XbtpExtensionRegistry_Initialize, XbtpExtensionRegistry_dtor};
use xbtp::session_impl::{SessionImplVtbl, IID_XBTP_CLIENT_SESSION};
use xbtp::sys::*;
use xbtp::xbtp2_ext::{com_release, new_message, Message, MetadataVtbl, XbtpCreateExtensionMetadata, IID_XBTP_EXTENSION_METADATA};

const EXT: GUID = GUID::new(0x7e57d15c, 0x3333, 0x4444, [9, 8, 7, 6, 5, 4, 3, 2]);
const LINK_PORT: u32 = 0x10a5;

unsafe fn json_text(m: *mut c_void) -> String {
    let m = m as *mut Message;
    let mut n = 0u32;
    let mut p: *mut u8 = null_mut();
    assert_eq!(((*(*m).vtbl).get_json_body)(m, &mut n, &mut p), S_OK);
    if p.is_null() {
        return String::new();
    }
    String::from_utf8_lossy(core::slice::from_raw_parts(p, n as usize)).trim_end_matches('\0').to_string()
}

unsafe fn set_reply(m: *mut c_void, text: &str) {
    let w = wide(text);
    assert_eq!(xbtp::xbtp2_ext::reply_set_json_w(m, w.as_ptr()), S_OK);
}

unsafe fn set_json(m: *mut c_void, text: &str) {
    let m = m as *mut Message;
    let w = wide(text);
    assert_eq!(((*(*m).vtbl).set_json_body_w)(m, w.as_ptr()), S_OK);
}

unsafe extern "system" fn echo(_s: usize, received: usize, reply: usize) -> HRESULT {
    let t = json_text(received as *mut c_void);
    set_reply(reply as *mut c_void, &t.replace("ping", "pong"));
    S_OK
}

unsafe fn drop_registry(p: *mut c_void) {
    XbtpExtensionRegistry_dtor(p as *mut XbtpExtensionRegistry);
    cpp_delete(p);
}

unsafe fn drop_dispatcher(p: *mut c_void) {
    XbtpConnectionDispatcher_dtor(p as *mut XbtpConnectionDispatcher);
    cpp_delete(p);
}

struct Raw(*mut XbtpConnectionDispatcher);
unsafe impl Send for Raw {}

unsafe fn open(key: &str) -> (HRESULT, *mut c_void) {
    let mut o = XBTP_OPEN_SESSION_OPTIONS::default();
    let k: Vec<u16> = key.encode_utf16().collect();
    o.session_key[..k.len()].copy_from_slice(&k);
    o.quiet = 1;
    o.timeout_ms = 5000;
    let host = wide("localhost");
    let mut s: *mut c_void = null_mut();
    let hr = XbtpOpenSessionEx2(host.as_ptr(), LINK_PORT | 0x0100_0000, &o, &EXT, &IID_XBTP_CLIENT_SESSION, &mut s);
    (hr, s)
}

#[test]
fn ex2_over_in_process_link_with_session_key() {
    unsafe {
        assert!(xbtp::xbtp2_sys::XbtpInitialize() >= 0);
        let r = cpp_new(8) as *mut XbtpExtensionRegistry;
        (*r).m_impl = null_mut();
        assert_eq!(XbtpExtensionRegistry_Initialize(r), S_OK);
        let mut reg = SharedPtr::new_with(r, drop_registry).unwrap();
        let mut tcp = SharedPtr::<TcpConnection>::default();
        assert_eq!(TcpConnection_InitializeServer_sp(47911, &mut tcp), S_OK);
        let mut t: SharedPtr<XbtpConnection> = tcp.cast();
        let mut link = SharedPtr::<XbtpConnection>::default();
        assert_eq!(link_initialize_server(LINK_PORT, &mut link), S_OK);
        assert!(xbtp::link_dev::listening(LINK_PORT));
        let d = cpp_new(8) as *mut XbtpConnectionDispatcher;
        (*d).m_impl = core::ptr::null();
        assert_eq!(XbtpConnectionDispatcher_Initialize(d, &mut reg, &mut t, &mut link), S_OK);
        let mut disp = SharedPtr::new_with(d, drop_dispatcher).unwrap();
        let mut arg = disp.clone_ref();
        let mut out: *mut c_void = null_mut();
        assert_eq!(XbtpWrapDispatcher(&mut arg, &IID_XBTP_DISPATCHER, &mut out), S_OK);
        let com = out as *mut DispatcherCom;
        let mut md: *mut c_void = null_mut();
        assert_eq!(XbtpCreateExtensionMetadata(&IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
        let mv = &**(md as *const *const MetadataVtbl);
        assert_eq!((mv.add_function)(md as *mut _, 7, Some(echo)), S_OK);
        assert_eq!(((*(*com).vtbl).add_extension)(com, &EXT, md), S_OK);
        com_release(md);
        let key = wide("linkkey");
        assert_eq!(((*(*com).vtbl).set_session_key)(com, key.as_ptr()), S_OK);
        let raw = Raw(d);
        let run = std::thread::spawn(move || {
            let raw = raw;
            unsafe { XbtpConnectionDispatcher_Run(raw.0) }
        });
        let (hr, s) = open("linkkey");
        assert_eq!(hr, S_OK);
        let v = &**(s as *const *const SessionImplVtbl);
        let mut addr: BSTR = null_mut();
        let name = wide("Address");
        assert_eq!((v.get_property)(s as *mut _, name.as_ptr(), &mut addr), S_OK);
        assert_eq!(from_wide(addr), "xbtplink");
        SysFreeString(addr);
        let m = new_message() as *mut c_void;
        set_json(m, "{\"via\":\"ping\"}");
        let mut ex = 0i32;
        assert_eq!((v.send_begin_message)(s as *mut _, 7, m, &mut ex), S_OK);
        com_release(m);
        let mut status = 1i32;
        let mut reply: *mut c_void = null_mut();
        assert_eq!((v.receive_end_message)(s as *mut _, 5000, ex, &mut status, &mut reply), S_OK);
        assert_eq!(status, 0);
        assert_eq!(json_text(reply), "{\"via\":\"pong\"}");
        com_release(reply);
        com_release(s);
        let (hr, s) = open("nope");
        assert_eq!(hr, E_SESSION_KEY_MISMATCH);
        com_release(s);
        assert_eq!(((*(*com).vtbl).shutdown)(com), S_OK);
        assert_eq!(run.join().unwrap(), S_OK);
        ((*(*com).vtbl).release)(com);
        disp.release();
    }
}
