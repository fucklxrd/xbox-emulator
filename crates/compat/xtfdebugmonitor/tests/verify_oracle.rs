use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::OnceLock;
use xtfdebugmonitor::client::*;
use xtfdebugmonitor::DEBUG_MONITOR_EXTENSION_ID;
use xtfmsg::com::IID_XBTP_EXTENSION_METADATA;
use xtfmsg::sys::*;

extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
}

const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 8;
const CONSOLE_DLLS: [&str; 2] = [r"D:\xbox-pc-port\games\XDefiant\extracted\EmbeddedXvd\Windows\System32\xtfdebugmonitor.dll", r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\xtfdebugmonitor.dll"];

type Factory = unsafe extern "system" fn(*const u16, *const GUID, *mut *mut c_void) -> HRESULT;
type Entry = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
type Start = unsafe extern "system" fn(*mut c_void, u32, usize, *mut c_void) -> HRESULT;
type Qi = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type Count = unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT;

struct Console {
    factory: Factory,
    entry: Entry,
}

unsafe impl Send for Console {}
unsafe impl Sync for Console {}

fn consoles() -> &'static Vec<(&'static str, Console)> {
    static C: OnceLock<Vec<(&'static str, Console)>> = OnceLock::new();
    C.get_or_init(|| {
        let mut v = Vec::new();
        for p in CONSOLE_DLLS {
            if !std::path::Path::new(p).is_file() {
                continue;
            }
            let h = unsafe { LoadLibraryExW(wide(p).as_ptr(), null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
            if h.is_null() {
                continue;
            }
            unsafe {
                let f = GetProcAddress(h, b"XtfCreateDebugMonitorClient\0".as_ptr());
                let e = GetProcAddress(h, 4usize as *const u8);
                assert!(!f.is_null() && !e.is_null(), "{p}: exports");
                v.push((p, Console { factory: core::mem::transmute::<*mut c_void, Factory>(f), entry: core::mem::transmute::<*mut c_void, Entry>(e) }));
            }
            break;
        }
        v
    })
}

unsafe fn slot<T: Copy>(o: *mut c_void, i: usize) -> T {
    let p = *(*(o as *const *const usize)).add(i);
    core::mem::transmute_copy(&p)
}

unsafe fn create(f: Factory, address: Option<&str>, iid: &GUID) -> (HRESULT, *mut c_void) {
    let a = address.map(wide);
    let mut out = 0x5555usize as *mut c_void;
    let hr = f(a.as_ref().map(|v| v.as_ptr()).unwrap_or(core::ptr::null()), iid, &mut out);
    (hr, out)
}

const BAD: GUID = GUID::new(1, 2, 3, [4; 8]);

#[test]
fn factory_matches_console_dll() {
    let Some((path, c)) = consoles().first() else {
        eprintln!("console xtfdebugmonitor.dll does not load on this PC; oracle skipped");
        return;
    };
    unsafe {
        for (addr, iid) in [(Some(""), IID_XTF_DEBUG_MONITOR_CLIENT), (Some("127.0.0.1/bogus"), IID_XTF_DEBUG_MONITOR_CLIENT), (None, IID_XTF_DEBUG_MONITOR_CLIENT), (Some(&"a".repeat(0x200) as &str), IID_XTF_DEBUG_MONITOR_CLIENT), (Some("127.0.0.1"), BAD)] {
            let (want, wout) = create(c.factory, addr, &iid);
            let (got, gout) = create(XtfCreateDebugMonitorClient, addr, &iid);
            assert_eq!(got as u32, want as u32, "{path}: {addr:?}");
            assert_eq!(wout, null_mut(), "{addr:?}");
            assert_eq!(gout, null_mut(), "{addr:?}");
        }
        for addr in ["127.0.0.1", "127.0.0.1/system", "127.0.0.1/HOST", "10.0.0.1+key/title", &"a".repeat(0x1ff)] {
            let (want, console) = create(c.factory, Some(addr), &IID_XTF_DEBUG_MONITOR_CLIENT);
            let (got, ours) = create(XtfCreateDebugMonitorClient, Some(addr), &IID_XTF_DEBUG_MONITOR_CLIENT);
            assert_eq!((got, want), (S_OK, S_OK), "{addr}");
            for iid in [IID_IUNKNOWN, IID_XTF_DEBUG_MONITOR_CLIENT_BASE, IID_XTF_DEBUG_MONITOR_CLIENT, BAD] {
                let mut a = 0x77usize as *mut c_void;
                let mut b = 0x77usize as *mut c_void;
                let hw = slot::<Qi>(console, 0)(console, &iid, &mut a);
                let hg = slot::<Qi>(ours, 0)(ours, &iid, &mut b);
                assert_eq!(hg, hw, "{addr} QI");
                assert_eq!(a == console, b == ours, "{addr} QI identity");
                assert_eq!(a == 0x77usize as *mut c_void, b == 0x77usize as *mut c_void, "{addr} QI out untouched");
            }
            assert_eq!(slot::<Start>(ours, 3)(ours, 7, 0, null_mut()), slot::<Start>(console, 3)(console, 7, 0, null_mut()), "{addr} Start null callback");
            let r = slot::<unsafe extern "system" fn(*mut c_void) -> u32>(ours, 2);
            while r(ours) > 0 {}
        }
    }
}

#[test]
fn server_entry_point_matches_console_dll() {
    let Some((path, c)) = consoles().first() else {
        eprintln!("console xtfdebugmonitor.dll does not load on this PC; oracle skipped");
        return;
    };
    unsafe {
        let mut out = 0x1234usize as *mut c_void;
        assert_eq!((c.entry)(&BAD, &IID_XBTP_EXTENSION_METADATA, &mut out), E_UNEXPECTED, "{path}");
        assert_eq!(out, 0x1234usize as *mut c_void);
        assert_eq!(xtfdebugmonitor::XtfOrdinal_4(&BAD, &IID_XBTP_EXTENSION_METADATA, &mut out), E_UNEXPECTED);
        assert_eq!((c.entry)(&DEBUG_MONITOR_EXTENSION_ID, &BAD, &mut out), E_UNEXPECTED);
        assert_eq!(xtfdebugmonitor::XtfOrdinal_4(&DEBUG_MONITOR_EXTENSION_ID, &BAD, &mut out), E_UNEXPECTED);
        assert_eq!(out, 0x1234usize as *mut c_void);
        let mut md: *mut c_void = null_mut();
        assert_eq!((c.entry)(&DEBUG_MONITOR_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
        assert!(!md.is_null());
        let mut n = 0u32;
        assert_eq!(slot::<Count>(md, 3)(md, &mut n), S_OK);
        assert_eq!(n as usize, xtfdebugmonitor::HANDLERS.len());
        assert_eq!(xtfdebugmonitor::HANDLERS.iter().map(|h| h.0).collect::<Vec<_>>(), [5, 3, 2, 4]);
    }
}
