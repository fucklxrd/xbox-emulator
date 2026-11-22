use std::ffi::c_void;
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[link(name = "winhttp")]
extern "system" {
    fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> *mut c_void;
    fn WinHttpConnect(s: *mut c_void, server: *const u16, port: u16, r: u32) -> *mut c_void;
    fn WinHttpOpenRequest(c: *mut c_void, verb: *const u16, obj: *const u16, ver: *const u16, refr: *const u16, acc: *const *const u16, flags: u32) -> *mut c_void;
    fn WinHttpSetStatusCallback(h: *mut c_void, cb: usize, flags: u32, r: usize) -> usize;
    fn WinHttpSendRequest(h: *mut c_void, hdr: *const u16, hl: u32, opt: *mut c_void, ol: u32, total: u32, ctx: usize) -> i32;
    fn WinHttpReceiveResponse(h: *mut c_void, r: *mut c_void) -> i32;
    fn WinHttpQueryDataAvailable(h: *mut c_void, n: *mut u32) -> i32;
    fn WinHttpReadData(h: *mut c_void, buf: *mut c_void, len: u32, read: *mut u32) -> i32;
    fn WinHttpCloseHandle(h: *mut c_void) -> i32;
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

enum Note {
    Headers(Instant),
    Done(usize, usize, Instant),
    Error(usize, u32),
}

static TX: Mutex<Option<Sender<Note>>> = Mutex::new(None);
static BUF: Mutex<(Vec<u8>, usize, usize)> = Mutex::new((Vec::new(), 0, 0));

fn note(n: Note) {
    if let Some(t) = TX.lock().unwrap().as_ref() {
        let _ = t.send(n);
    }
}

unsafe extern "system" fn cb(h: *mut c_void, _ctx: usize, status: u32, info: *mut c_void, len: u32) {
    match status {
        0x0040_0000 => {
            WinHttpReceiveResponse(h, std::ptr::null_mut());
        }
        0x0002_0000 => {
            note(Note::Headers(Instant::now()));
            WinHttpQueryDataAvailable(h, std::ptr::null_mut());
        }
        0x0004_0000 => {
            let n = *(info as *const u32);
            if n == 0 {
                let g = BUF.lock().unwrap();
                note(Note::Done(g.1, g.2, Instant::now()));
                return;
            }
            let p = {
                let mut g = BUF.lock().unwrap();
                g.0.resize(8192, 0);
                g.0.as_mut_ptr()
            };
            WinHttpReadData(h, p as *mut c_void, 8192, std::ptr::null_mut());
        }
        0x0008_0000 => {
            {
                let mut g = BUF.lock().unwrap();
                g.1 += len as usize;
                g.2 += 1;
            }
            if len == 0 {
                let g = BUF.lock().unwrap();
                note(Note::Done(g.1, g.2, Instant::now()));
                return;
            }
            WinHttpQueryDataAvailable(h, std::ptr::null_mut());
        }
        0x0020_0000 => {
            let r = &*(info as *const [usize; 2]);
            note(Note::Error(r[0], r[1] as u32));
        }
        _ => {}
    }
}

unsafe fn fetch(profile: &str) -> (usize, usize, Duration, Duration) {
    {
        let mut g = BUF.lock().unwrap();
        g.1 = 0;
        g.2 = 0;
    }
    let (tx, rx) = channel();
    *TX.lock().unwrap() = Some(tx);
    {
        let s = WinHttpOpen(w("tombox-test").as_ptr(), 1, std::ptr::null(), std::ptr::null(), 0x1000_0000);
        let c = WinHttpConnect(s, w("fortnite-public-service-prod11.ol.epicgames.com").as_ptr(), 0, 0);
        let r = WinHttpOpenRequest(c, w("POST").as_ptr(), w(&format!("/fortnite/api/game/v2/profile/XboxEmulator/client/QueryProfile?profileId={profile}&rvn=-1")).as_ptr(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0x0080_0000);
        assert!(!r.is_null());
        WinHttpSetStatusCallback(r, cb as usize, 0xffff_ffff, 0);
        let hdr = w("Content-Type: application/json\r\nAuthorization: bearer tombox");
        let mut body = b"{}".to_vec();
        let t0 = Instant::now();
        assert_ne!(WinHttpSendRequest(r, hdr.as_ptr(), u32::MAX, body.as_mut_ptr() as *mut c_void, 2, 2, 0), 0);
        let mut th = None;
        let out = loop {
            match rx.recv_timeout(Duration::from_secs(60)).expect("winhttp callback") {
                Note::Headers(t) => th = Some(t),
                Note::Done(bytes, reads, t) => break (bytes, reads, th.unwrap() - t0, t - t0),
                Note::Error(api, err) => panic!("REQUEST_ERROR api={api} err={err}"),
            }
        };
        WinHttpSetStatusCallback(r, 0, 0xffff_ffff, 0);
        WinHttpCloseHandle(r);
        WinHttpCloseHandle(c);
        WinHttpCloseHandle(s);
        out
    }
}

#[test]
#[cfg_attr(not(tombox_backend_data), ignore = "needs crates/compat/compat-common/backend_data")]
fn large_profile_downloads_quickly_in_small_async_reads() {
    std::env::set_var("TOMBOX_NET_BACKEND", "builtin");
    std::env::set_var("TOMBOX_NET_TITLE", "fortnite");
    compat_common::net_guard::ensure();
    unsafe {
        for profile in ["common_core", "athena", "athena"] {
            let (bytes, reads, head, total) = fetch(profile);
            eprintln!("{profile} profile: {bytes} bytes in {reads} reads; headers after {head:?}, body done after {total:?}");
            assert!(bytes > 0);
            assert!(reads * 8192 >= bytes);
        }
        let mut warm = Vec::new();
        for _ in 0..5 {
            let (bytes, _, head, total) = fetch("athena");
            assert!(bytes < 2 * 1024 * 1024, "athena profile is {bytes} bytes; every 8 KB read costs the title one HTTP-thread tick");
            assert!(total < Duration::from_secs(1), "a warm athena QueryProfile stalled: {total:?} (headers {head:?})");
            warm.push((head, total));
        }
        let head = warm.iter().map(|w| w.0).min().unwrap();
        let total = warm.iter().map(|w| w.1).min().unwrap();
        eprintln!("warm athena best of {}: headers {head:?}, body {total:?}; all {warm:?}", warm.len());
        assert!(head < Duration::from_millis(60) && total < Duration::from_millis(100), "best warm athena QueryProfile took {total:?} (headers {head:?}); all {warm:?}");
    }
}
