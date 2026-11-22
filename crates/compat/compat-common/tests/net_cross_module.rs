use compat_common::gdk;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::Duration;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(m: *mut c_void, name: *const u8) -> *mut c_void;
}

#[path = "support/built_dlls.rs"]
mod built_dlls;

fn xcurl_dll() -> std::path::PathBuf {
    built_dlls::ensure(&[("xcurl", "xcurl.dll")]).join("xcurl.dll")
}

#[link(name = "winhttp")]
extern "system" {
    fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> *mut c_void;
    fn WinHttpConnect(s: *mut c_void, server: *const u16, port: u16, r: u32) -> *mut c_void;
    fn WinHttpOpenRequest(c: *mut c_void, verb: *const u16, obj: *const u16, ver: *const u16, refr: *const u16, acc: *const *const u16, flags: u32) -> *mut c_void;
    fn WinHttpSetOption(h: *mut c_void, opt: u32, buf: *mut c_void, len: u32) -> i32;
    fn WinHttpSetStatusCallback(h: *mut c_void, cb: usize, flags: u32, r: usize) -> usize;
    fn WinHttpSendRequest(h: *mut c_void, hdr: *const u16, hl: u32, opt: *mut c_void, ol: u32, total: u32, ctx: usize) -> i32;
    fn WinHttpReceiveResponse(h: *mut c_void, r: *mut c_void) -> i32;
    fn WinHttpQueryHeaders(h: *mut c_void, level: u32, name: *const u16, buf: *mut c_void, len: *mut u32, idx: *mut u32) -> i32;
    fn WinHttpCloseHandle(h: *mut c_void) -> i32;
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[derive(Debug)]
enum Note {
    Verified(i32),
    SendDone,
    Headers,
    Error(usize, u32),
}

static TX: Mutex<Option<Sender<Note>>> = Mutex::new(None);
static VOBJ: AtomicUsize = AtomicUsize::new(0);
static VFN: AtomicUsize = AtomicUsize::new(0);

fn note(n: Note) {
    if let Some(t) = TX.lock().unwrap().as_ref() {
        let _ = t.send(n);
    }
}

type Verify = unsafe extern "system" fn(*mut c_void, *mut c_void, *const u8) -> i32;

unsafe extern "system" fn cb(h: *mut c_void, _ctx: usize, status: u32, info: *mut c_void, _len: u32) {
    match status {
        0x0000_0010 => {
            let f: Verify = core::mem::transmute(VFN.load(Ordering::SeqCst));
            let hr = f(VOBJ.load(Ordering::SeqCst) as *mut c_void, h, core::ptr::null());
            note(Note::Verified(hr));
            if hr < 0 {
                WinHttpCloseHandle(h);
            }
        }
        0x0040_0000 => note(Note::SendDone),
        0x0002_0000 => note(Note::Headers),
        0x0020_0000 => {
            let r = &*(info as *const [usize; 2]);
            note(Note::Error(r[0], r[1] as u32));
        }
        _ => {}
    }
}

#[test]
#[cfg_attr(not(tombox_backend_data), ignore = "needs crates/compat/compat-common/backend_data")]
fn verify_in_a_second_module_copy_accepts_the_backend() {
    std::env::set_var("TOMBOX_NET_BACKEND", "builtin");
    std::env::set_var("TOMBOX_NET_TITLE", "fortnite");
    std::env::remove_var("TOMBOX_NET_LOCAL_PORT");
    std::env::remove_var("TOMBOX_NET_GUARD_OWNER");
    unsafe {
        let dll = xcurl_dll();
        let wide = w(&dll.to_string_lossy());
        let m = LoadLibraryW(wide.as_ptr());
        assert!(!m.is_null(), "LoadLibrary {}", dll.display());
        let init = GetProcAddress(m, b"curl_global_init\0".as_ptr());
        assert!(!init.is_null());
        let init: unsafe extern "C" fn(i32) -> i32 = core::mem::transmute(init);
        assert_eq!(init(3), 0);
        let easy = GetProcAddress(m, b"curl_easy_init\0".as_ptr());
        let easy: unsafe extern "C" fn() -> *mut c_void = core::mem::transmute(easy);
        assert!(!easy().is_null());
        let owner = std::env::var("TOMBOX_NET_GUARD_OWNER").unwrap_or_default();
        assert_eq!(owner, std::process::id().to_string(), "xcurl.dll must own net_guard");
        let port: u16 = std::env::var(compat_common::net_guard::INPROC_PORT_ENV).expect("backend port published").parse().unwrap();
        assert!(port != 0);
        compat_common::net_guard::ensure();
        assert!(compat_common::net_guard::url_is_in_process(&format!("http://127.0.0.1:{port}/x"), port));
        assert!(!compat_common::net_guard::url_is_in_process(&format!("https://127.0.0.1:{port}/x"), port));
        assert!(!compat_common::net_guard::url_is_in_process("http://127.0.0.1:1/x", port));
        let mut obj: *mut c_void = core::ptr::null_mut();
        assert_eq!(gdk::query_api_impl(&gdk::net::CLSID, &gdk::net::IID_V2, &mut obj), 0);
        let vt = *(obj as *const *const usize);
        VOBJ.store(obj as usize, Ordering::SeqCst);
        VFN.store(*vt.add(14), Ordering::SeqCst);
        let (tx, rx) = channel();
        *TX.lock().unwrap() = Some(tx);
        let s = WinHttpOpen(w("tombox-test").as_ptr(), 1, std::ptr::null(), std::ptr::null(), 0x1000_0000);
        assert!(!s.is_null());
        let c = WinHttpConnect(s, w("account-public-service-prod.ol.epicgames.com").as_ptr(), 0, 0);
        assert!(!c.is_null());
        let r = WinHttpOpenRequest(c, w("POST").as_ptr(), w("/account/api/oauth/token").as_ptr(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0x0080_0000);
        assert!(!r.is_null());
        assert_ne!(WinHttpSetOption(r, 47, std::ptr::null_mut(), 0), 0, "WINHTTP_NO_CLIENT_CERT_CONTEXT on a redirected request");
        WinHttpSetStatusCallback(r, cb as usize, 0xffff_ffff, 0);
        let hdr = w("Content-Type: application/x-www-form-urlencoded\r\nAuthorization: basic ZWM2ODRiOGM2ODdmNDc5ZmFkZWEzY2IyYWQ4M2Y1YzY6ZTFmMzFjMjExZjI4NDEzMTg2MjYyZDM3YTEzZmM4NGQ=");
        let mut body = b"grant_type=exchange_code&exchange_code=tombox".to_vec();
        assert_ne!(WinHttpSendRequest(r, hdr.as_ptr(), u32::MAX, body.as_mut_ptr() as *mut c_void, body.len() as u32, body.len() as u32, 0), 0);
        let mut verified = None;
        loop {
            match rx.recv_timeout(Duration::from_secs(10)).expect("winhttp callback") {
                Note::Verified(hr) => verified = Some(hr),
                Note::SendDone => {
                    assert_ne!(WinHttpReceiveResponse(r, std::ptr::null_mut()), 0);
                }
                Note::Headers => break,
                Note::Error(api, err) => panic!("REQUEST_ERROR api={api} err={err} verified={verified:?}"),
            }
        }
        assert_eq!(verified, Some(0), "XNetworkingVerifyServerCertificate must accept the in-process backend");
        let mut status = 0u32;
        let mut n = 4u32;
        assert_ne!(WinHttpQueryHeaders(r, 19 | 0x2000_0000, std::ptr::null(), &mut status as *mut u32 as *mut c_void, &mut n, std::ptr::null_mut()), 0);
        assert!(status > 0 && status < 500, "status {status}");
        WinHttpSetStatusCallback(r, 0, 0xffff_ffff, 0);
        WinHttpCloseHandle(r);
        WinHttpCloseHandle(c);
        WinHttpCloseHandle(s);
    }
}
