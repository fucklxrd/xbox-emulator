use std::ffi::c_void;

#[link(name = "winhttp")]
extern "system" {
    fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> *mut c_void;
    fn WinHttpConnect(s: *mut c_void, server: *const u16, port: u16, r: u32) -> *mut c_void;
    fn WinHttpOpenRequest(c: *mut c_void, verb: *const u16, obj: *const u16, ver: *const u16, refr: *const u16, acc: *const *const u16, flags: u32) -> *mut c_void;
    fn WinHttpSetOption(h: *mut c_void, opt: u32, buf: *mut c_void, len: u32) -> i32;
    fn WinHttpAddRequestHeaders(h: *mut c_void, hdr: *const u16, len: u32, flags: u32) -> i32;
    fn WinHttpSendRequest(h: *mut c_void, hdr: *const u16, hl: u32, opt: *mut c_void, ol: u32, total: u32, ctx: usize) -> i32;
    fn WinHttpReceiveResponse(h: *mut c_void, r: *mut c_void) -> i32;
    fn WinHttpWebSocketCompleteUpgrade(h: *mut c_void, ctx: usize) -> *mut c_void;
    fn WinHttpWebSocketSend(h: *mut c_void, ty: u32, buf: *mut c_void, len: u32) -> u32;
    fn WinHttpWebSocketReceive(h: *mut c_void, buf: *mut c_void, len: u32, read: *mut u32, ty: *mut u32) -> u32;
    fn WinHttpWebSocketClose(h: *mut c_void, status: u16, reason: *mut c_void, len: u32) -> u32;
    fn WinHttpCloseHandle(h: *mut c_void) -> i32;
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[test]
#[cfg_attr(not(tombox_backend_data), ignore = "needs crates/compat/compat-common/backend_data")]
fn wss_xmpp_through_guard_reaches_builtin_server() {
    std::env::set_var("TOMBOX_NET_BACKEND", "builtin");
    std::env::set_var("TOMBOX_NET_TITLE", "fortnite");
    compat_common::net_guard::ensure();
    unsafe {
        let s = WinHttpOpen(w("tombox-test").as_ptr(), 1, std::ptr::null(), std::ptr::null(), 0);
        assert!(!s.is_null());
        let c = WinHttpConnect(s, w("xmpp-service-prod.ol.epicgames.com").as_ptr(), 443, 0);
        assert!(!c.is_null(), "connect err {}", std::io::Error::last_os_error());
        let r = WinHttpOpenRequest(c, w("GET").as_ptr(), w("/").as_ptr(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0x0080_0000);
        assert!(!r.is_null());
        assert_ne!(WinHttpSetOption(r, 114, std::ptr::null_mut(), 0), 0);
        let h = w("Sec-WebSocket-Protocol: xmpp");
        assert_ne!(WinHttpAddRequestHeaders(r, h.as_ptr(), u32::MAX, 0x2000_0000), 0);
        assert_ne!(WinHttpSendRequest(r, std::ptr::null(), 0, std::ptr::null_mut(), 0, 0, 0), 0, "send err {}", std::io::Error::last_os_error());
        assert_ne!(WinHttpReceiveResponse(r, std::ptr::null_mut()), 0, "recv err {}", std::io::Error::last_os_error());
        let ws = WinHttpWebSocketCompleteUpgrade(r, 0);
        assert!(!ws.is_null(), "upgrade err {}", std::io::Error::last_os_error());
        WinHttpCloseHandle(r);
        let mut open = b"<open xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\" to=\"prod.ol.epicgames.com\" version=\"1.0\"/>".to_vec();
        assert_eq!(WinHttpWebSocketSend(ws, 2, open.as_mut_ptr() as *mut c_void, open.len() as u32), 0);
        let mut got = Vec::new();
        for _ in 0..2 {
            let mut buf = vec![0u8; 4096];
            let (mut n, mut ty) = (0u32, 0u32);
            assert_eq!(WinHttpWebSocketReceive(ws, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n, &mut ty), 0);
            got.push(String::from_utf8_lossy(&buf[..n as usize]).into_owned());
        }
        assert!(got[0].starts_with("<open "), "{got:?}");
        assert!(got[1].contains("PLAIN"), "{got:?}");
        WinHttpWebSocketClose(ws, 1000, std::ptr::null_mut(), 0);
        WinHttpCloseHandle(ws);
        WinHttpCloseHandle(c);
        WinHttpCloseHandle(s);
    }
}
