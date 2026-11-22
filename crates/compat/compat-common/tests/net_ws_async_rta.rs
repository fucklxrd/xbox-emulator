use std::ffi::c_void;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[link(name = "winhttp")]
extern "system" {
    fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> *mut c_void;
    fn WinHttpConnect(s: *mut c_void, server: *const u16, port: u16, r: u32) -> *mut c_void;
    fn WinHttpOpenRequest(c: *mut c_void, verb: *const u16, obj: *const u16, ver: *const u16, refr: *const u16, acc: *const *const u16, flags: u32) -> *mut c_void;
    fn WinHttpSetOption(h: *mut c_void, opt: u32, buf: *mut c_void, len: u32) -> i32;
    fn WinHttpSetStatusCallback(h: *mut c_void, cb: usize, flags: u32, reserved: usize) -> usize;
    fn WinHttpAddRequestHeaders(h: *mut c_void, hdr: *const u16, len: u32, flags: u32) -> i32;
    fn WinHttpSendRequest(h: *mut c_void, hdr: *const u16, hl: u32, opt: *mut c_void, ol: u32, total: u32, ctx: usize) -> i32;
    fn WinHttpReceiveResponse(h: *mut c_void, r: *mut c_void) -> i32;
    fn WinHttpWebSocketCompleteUpgrade(h: *mut c_void, ctx: usize) -> *mut c_void;
    fn WinHttpWebSocketSend(h: *mut c_void, ty: u32, buf: *mut c_void, len: u32) -> u32;
    fn WinHttpWebSocketReceive(h: *mut c_void, buf: *mut c_void, len: u32, read: *mut u32, ty: *mut u32) -> u32;
    fn WinHttpWebSocketClose(h: *mut c_void, status: u16, reason: *mut c_void, len: u32) -> u32;
    fn WinHttpCloseHandle(h: *mut c_void) -> i32;
}

const HANDLE_CREATED: u32 = 0x0000_0400;
const SENDREQUEST_COMPLETE: u32 = 0x0040_0000;
const HEADERS_AVAILABLE: u32 = 0x0002_0000;
const READ_COMPLETE: u32 = 0x0008_0000;
const WRITE_COMPLETE: u32 = 0x0010_0000;
const REQUEST_ERROR: u32 = 0x0020_0000;
const ALL: u32 = 0xffff_ffff;

static EVENTS: Mutex<Vec<(usize, u32, usize, u32)>> = Mutex::new(Vec::new());

unsafe extern "system" fn cb(h: *mut c_void, _ctx: usize, status: u32, info: *mut c_void, len: u32) {
    let first = if info.is_null() || len < 4 { 0 } else if len >= 8 { core::ptr::read_unaligned(info as *const usize) } else { core::ptr::read_unaligned(info as *const u32) as usize };
    EVENTS.lock().unwrap().push((h as usize, status, first, len));
}

fn wait(h: usize, status: u32) -> (usize, u32) {
    let t = Instant::now();
    loop {
        {
            let ev = EVENTS.lock().unwrap();
            if let Some(e) = ev.iter().find(|e| e.0 == h && e.1 == status) {
                return (e.2, e.3);
            }
            if let Some(e) = ev.iter().find(|e| e.0 == h && e.1 == REQUEST_ERROR) {
                panic!("request error on {h:#x}: {e:?}");
            }
        }
        assert!(t.elapsed() < Duration::from_secs(10), "timed out waiting for {status:#x} on {h:#x}: {:?}", EVENTS.lock().unwrap());
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[test]
#[cfg_attr(not(tombox_backend_data), ignore = "needs crates/compat/compat-common/backend_data")]
fn async_rta_websocket_handle_gets_every_callback_and_acks_subscribe() {
    std::env::set_var("TOMBOX_NET_BACKEND", "builtin");
    std::env::set_var("TOMBOX_NET_TITLE", "fortnite");
    compat_common::net_guard::ensure();
    unsafe {
        let s = WinHttpOpen(w("tombox-test").as_ptr(), 1, std::ptr::null(), std::ptr::null(), 0x1000_0000);
        assert!(!s.is_null());
        let c = WinHttpConnect(s, w("rta.xboxlive.com").as_ptr(), 443, 0);
        assert!(!c.is_null(), "connect err {}", std::io::Error::last_os_error());
        let r = WinHttpOpenRequest(c, w("GET").as_ptr(), w("/connect").as_ptr(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0x0080_0000);
        assert!(!r.is_null());
        assert_ne!(WinHttpSetStatusCallback(r, cb as *const () as usize, ALL, 0), usize::MAX);
        assert_ne!(WinHttpSetOption(r, 114, std::ptr::null_mut(), 0), 0);
        let h = w("Sec-WebSocket-Protocol: rta.xboxlive.com.V2");
        assert_ne!(WinHttpAddRequestHeaders(r, h.as_ptr(), u32::MAX, 0x2000_0000), 0);
        assert_ne!(WinHttpSendRequest(r, std::ptr::null(), 0, std::ptr::null_mut(), 0, 0, 0), 0, "send err {}", std::io::Error::last_os_error());
        wait(r as usize, SENDREQUEST_COMPLETE);
        assert_ne!(WinHttpReceiveResponse(r, std::ptr::null_mut()), 0, "recv err {}", std::io::Error::last_os_error());
        wait(r as usize, HEADERS_AVAILABLE);
        let ws = WinHttpWebSocketCompleteUpgrade(r, 0);
        assert!(!ws.is_null(), "upgrade err {}", std::io::Error::last_os_error());
        let (created, _) = wait(ws as usize, HANDLE_CREATED);
        assert_eq!(created, ws as usize);
        WinHttpCloseHandle(r);
        let mut sub = b"[1,1,\"https://sessiondirectory.xboxlive.com/connections/\"]\0".to_vec();
        assert_eq!(WinHttpWebSocketSend(ws, 2, sub.as_mut_ptr() as *mut c_void, sub.len() as u32), 0);
        wait(ws as usize, WRITE_COMPLETE);
        let mut buf = vec![0u8; 4096];
        assert_eq!(WinHttpWebSocketReceive(ws, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, std::ptr::null_mut(), std::ptr::null_mut()), 0);
        let (read, _) = wait(ws as usize, READ_COMPLETE);
        let n = read & 0xffff_ffff;
        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text:?}"));
        assert_eq!(v[0], 1);
        assert_eq!(v[1], 1);
        assert_eq!(v[2], 0);
        assert!(v[4]["ConnectionId"].as_str().map_or(false, |s| s.len() == 36), "{text}");
        WinHttpWebSocketClose(ws, 1000, std::ptr::null_mut(), 0);
        WinHttpCloseHandle(ws);
        WinHttpCloseHandle(c);
        WinHttpCloseHandle(s);
    }
}
