use core::ffi::c_void;
use core::ptr::{null, null_mut};
use std::sync::OnceLock;
use xbtp::api::*;
use xbtp::conn::{TcpConnection, TcpConnection_InitializeServer_sp, XbtpConnection};
use xbtp::dispatcher::*;
use xbtp::dispatcher_com::XbtpWrapDispatcher;
use xbtp::msvc::{cpp_new, SharedPtr};
use xbtp::registry::{XbtpExtensionRegistry, XbtpExtensionRegistry_Initialize};
use xbtp::sys::*;

extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
}

const EINVAL: i32 = 0x8007_0057u32 as i32;
const EXISTS: i32 = 0x8007_1392u32 as i32;
const EXT: GUID = GUID::new(0x1234_5678, 0x1111, 0x2222, [1, 2, 3, 4, 5, 6, 7, 8]);

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

struct Console(*mut c_void);
unsafe impl Send for Console {}
unsafe impl Sync for Console {}

fn console() -> Option<&'static Console> {
    static C: OnceLock<Option<Console>> = OnceLock::new();
    C.get_or_init(|| {
        let root = std::path::Path::new(r"D:\xbox-pc-port\games\Terraria\extracted");
        let dir = std::fs::read_dir(root).ok()?.flatten().map(|e| e.path().join(r"EmbeddedXvd\Windows\System32\xbtp.dll")).find(|p| p.is_file())?;
        let h = unsafe { LoadLibraryW(w(dir.to_str()?).as_ptr()) };
        if h.is_null() {
            None
        } else {
            Some(Console(h))
        }
    })
    .as_ref()
}

unsafe fn sym<T: Copy>(c: &Console, name: &str) -> T {
    let z: Vec<u8> = name.bytes().chain(Some(0)).collect();
    let p = GetProcAddress(c.0, z.as_ptr());
    assert!(!p.is_null(), "{name}");
    core::mem::transmute_copy(&p)
}

type OpenSession2 = unsafe extern "C" fn(*const u16, u32, *const XBTP_OPEN_SESSION_OPTIONS, *const GUID, *const GUID, *mut *mut c_void) -> i32;
type OpenSession = unsafe extern "C" fn(*const u16, u32, *const GUID, *const GUID, *mut *mut c_void) -> i32;
type OpenSessionCpp = unsafe extern "C" fn(*const GUID, *const u16, u32, *mut *mut c_void, bool) -> i32;
type OpenSessionInternal = unsafe extern "C" fn(*const GUID, *const u16, u32, *mut *mut c_void, bool, u32) -> i32;
type OpenPipe = unsafe extern "C" fn(*const u16, *const GUID, *const GUID, *mut *mut c_void) -> i32;
type OpenPipeShared = unsafe extern "C" fn(*const u16, *const GUID, *mut [usize; 2]) -> i32;
type Complete = unsafe extern "C" fn(*const GUID, bool, u32, *mut [usize; 2], *mut *mut c_void) -> i32;
type Wrap = unsafe extern "C" fn(*mut [usize; 2], *const GUID, *mut *mut c_void) -> i32;
type Start = unsafe extern "C" fn(u32, i32, *const GUID, *mut *mut c_void) -> i32;
type Init = unsafe extern "C" fn() -> i32;
type DInit = unsafe extern "C" fn(*mut c_void, *mut [usize; 2], *mut [usize; 2], *mut [usize; 2]) -> i32;
type DAddPipe = unsafe extern "C" fn(*mut c_void, *const u16) -> i32;
type RegInit = unsafe extern "C" fn(*mut c_void) -> i32;
type TcpServer = unsafe extern "C" fn(u32, *mut [usize; 2]) -> i32;

const N_OS2: &str = "XbtpOpenSession2";
const N_OS: &str = "XbtpOpenSession";
const N_OSX2: &str = "XbtpOpenSessionEx2";
const N_OSX: &str = "XbtpOpenSessionEx";
const N_CPP: &str = "?XbtpOpenSession@@YAJAEBU_GUID@@PEBGKPEAPEAVXbtpSession@@_N@Z";
const N_INT: &str = "?XbtpOpenSessionInternal@@YAJAEBU_GUID@@PEBGKPEAPEAVXbtpSession@@_NI@Z";
const N_PIPE: &str = "?XbtpOpenPipeSession@@YAJPEBGAEBU_GUID@@1PEAPEAX@Z";
const N_PIPE_SP: &str = "?XbtpOpenPipeSession@@YAJPEBGAEBU_GUID@@PEAV?$shared_ptr@VXbtpSession@@@std@@@Z";
const N_COMPLETE: &str = "?XbtpCompleteSessionSetupInternal@@YAJAEBU_GUID@@_NIV?$shared_ptr@VXbtpConnection@@@std@@PEAPEAVXbtpSession@@@Z";
const N_WRAP: &str = "?XbtpWrapDispatcher@@YAJV?$shared_ptr@VXbtpConnectionDispatcher@@@std@@AEBU_GUID@@PEAPEAX@Z";
const N_DINIT: &str = "?Initialize@XbtpConnectionDispatcher@@QEAAJV?$shared_ptr@VXbtpExtensionRegistry@@@std@@V?$shared_ptr@VXbtpConnection@@@3@1@Z";
const N_ADDPIPE: &str = "?AddPipeEndpoint@XbtpConnectionDispatcher@@QEAAJPEBG@Z";
const N_REGINIT: &str = "?Initialize@XbtpExtensionRegistry@@QEAAJXZ";
const N_TCPSRV: &str = "?InitializeServer@TcpConnection@@SAJKPEAV?$shared_ptr@VTcpConnection@@@std@@@Z";

fn closed_port() -> u32 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let p = l.local_addr().unwrap().port() as u32;
    drop(l);
    p
}

#[test]
fn open_session_argument_errors_match_console() {
    let Some(c) = console() else { return };
    unsafe {
        let ci: Init = sym(c, "XbtpInitialize");
        assert!(ci() >= 0);
        assert!(xbtp::xbtp2_sys::XbtpInitialize() >= 0);
        let c2: OpenSession2 = sym(c, N_OS2);
        let c1: OpenSession = sym(c, N_OS);
        let cx2: OpenSession2 = sym(c, N_OSX2);
        let cx: OpenSession = sym(c, N_OSX);
        let mut a: *mut c_void = null_mut();
        let mut b: *mut c_void = null_mut();
        for port in [2u32, 4, 0x00ff_ffff, 0x0100_0002] {
            assert_eq!(XbtpOpenSession2(null(), port, null(), &EXT, &EXT, &mut a), c2(null(), port, null(), &EXT, &EXT, &mut b), "{port:#x}");
            assert_eq!(XbtpOpenSession(null(), port, &EXT, &EXT, &mut a), c1(null(), port, &EXT, &EXT, &mut b), "{port:#x}");
        }
        for addr in ["+abc", "host+", "+0123456789abcdef0123456789abcde", "a+b+c"] {
            let s = w(addr);
            assert_eq!(XbtpOpenSession(s.as_ptr(), 2, &EXT, &EXT, &mut a), c1(s.as_ptr(), 2, &EXT, &EXT, &mut b), "{addr}");
        }
        assert_eq!(XbtpOpenSessionEx2(null(), 0x1073, null(), &EXT, &EXT, null_mut()), cx2(null(), 0x1073, null(), &EXT, &EXT, null_mut()));
        for size in [0u32, 0x43, 0x45, 0x57, 0x59, 0x1000] {
            let mut o = XBTP_OPEN_SESSION_OPTIONS::default();
            o.size = size;
            assert_eq!(XbtpOpenSessionEx2(null(), 0x1073, &o, &EXT, &EXT, &mut a), cx2(null(), 0x1073, &o, &EXT, &EXT, &mut b), "{size:#x}");
        }
        for port in [0x0200_1073u32, 0xff00_0001, 0x8000_0000] {
            assert_eq!(XbtpOpenSessionEx2(null(), port, null(), &EXT, &EXT, &mut a), cx2(null(), port, null(), &EXT, &EXT, &mut b), "{port:#x}");
            assert_eq!(XbtpOpenSessionEx(null(), port, &EXT, &EXT, &mut a), cx(null(), port, &EXT, &EXT, &mut b), "{port:#x}");
        }
        assert!(a.is_null() && b.is_null());
    }
}

#[test]
fn cpp_open_session_errors_match_console() {
    let Some(c) = console() else { return };
    unsafe {
        let ci: Init = sym(c, "XbtpInitialize");
        assert!(ci() >= 0);
        let cpp: OpenSessionCpp = sym(c, N_CPP);
        let int: OpenSessionInternal = sym(c, N_INT);
        assert_eq!(XbtpOpenSessionCpp(&EXT, null(), 0x1073, null_mut(), true) as i32, cpp(&EXT, null(), 0x1073, null_mut(), true));
        let mut a = 0x1234usize as *mut xbtp::session::XbtpSession;
        let mut b = 0x1234usize as *mut c_void;
        for port in [0x0200_0001u32, 0xfe00_0000] {
            assert_eq!(XbtpOpenSessionInternal(&EXT, null(), port, &mut a, true, 1000), int(&EXT, null(), port, &mut b, true, 1000), "{port:#x}");
            assert_eq!(a as usize, b as usize);
        }
    }
}

#[test]
fn refused_tcp_connect_matches_console() {
    let Some(c) = console() else { return };
    unsafe {
        let ci: Init = sym(c, "XbtpInitialize");
        assert!(ci() >= 0);
        assert!(xbtp::xbtp2_sys::XbtpInitialize() >= 0);
        let int: OpenSessionInternal = sym(c, N_INT);
        let cx2: OpenSession2 = sym(c, N_OSX2);
        let port = closed_port();
        let host = w("127.0.0.1");
        let mut a: *mut xbtp::session::XbtpSession = null_mut();
        let mut b: *mut c_void = null_mut();
        let ours = XbtpOpenSessionInternal(&EXT, host.as_ptr(), port, &mut a, true, 2000);
        let theirs = int(&EXT, host.as_ptr(), port, &mut b, true, 2000);
        assert_eq!(ours, theirs, "{ours:#x} vs {theirs:#x}");
        assert!(a.is_null() && b.is_null());
        let mut pa: *mut c_void = null_mut();
        let mut pb: *mut c_void = null_mut();
        let mut o = XBTP_OPEN_SESSION_OPTIONS::default();
        o.quiet = 1;
        o.timeout_ms = 2000;
        let ours = XbtpOpenSessionEx2(host.as_ptr(), port, &o, &EXT, &EXT, &mut pa);
        let theirs = cx2(host.as_ptr(), port, &o, &EXT, &EXT, &mut pb);
        assert_eq!(ours, theirs, "{ours:#x} vs {theirs:#x}");
        assert!(pa.is_null() && pb.is_null());
    }
}

#[test]
fn pipe_session_errors_match_console() {
    let Some(c) = console() else { return };
    unsafe {
        let cp: OpenPipe = sym(c, N_PIPE);
        let cs: OpenPipeShared = sym(c, N_PIPE_SP);
        let empty = w("");
        let missing = w(&format!(r"\\.\pipe\tombox-verify-missing-{}", std::process::id()));
        let mut a: *mut c_void = null_mut();
        let mut b: *mut c_void = null_mut();
        assert_eq!(XbtpOpenPipeSession(null(), &EXT, &EXT, &mut a), cp(null(), &EXT, &EXT, &mut b));
        assert_eq!(XbtpOpenPipeSession(empty.as_ptr(), &EXT, &EXT, &mut a), cp(empty.as_ptr(), &EXT, &EXT, &mut b));
        assert_eq!(XbtpOpenPipeSession(missing.as_ptr(), &EXT, &EXT, null_mut()), cp(missing.as_ptr(), &EXT, &EXT, null_mut()));
        let ours = XbtpOpenPipeSession(missing.as_ptr(), &EXT, &EXT, &mut a);
        let theirs = cp(missing.as_ptr(), &EXT, &EXT, &mut b);
        assert_eq!(ours, theirs, "{ours:#x} vs {theirs:#x}");
        assert!(a.is_null() && b.is_null());
        let mut sa = SharedPtr::<xbtp::session::XbtpSession>::default();
        let mut sb = [0usize; 2];
        assert_eq!(XbtpOpenPipeSessionShared(missing.as_ptr(), &EXT, null_mut()), cs(missing.as_ptr(), &EXT, null_mut()));
        let ours = XbtpOpenPipeSessionShared(missing.as_ptr(), &EXT, &mut sa);
        let theirs = cs(missing.as_ptr(), &EXT, &mut sb);
        assert_eq!(ours, theirs, "{ours:#x} vs {theirs:#x}");
        assert!(sa.ptr.is_null() && sb[0] == 0);
    }
}

#[test]
fn complete_session_setup_with_empty_connection_matches_console() {
    let Some(c) = console() else { return };
    unsafe {
        let cc: Complete = sym(c, N_COMPLETE);
        let mut ea = SharedPtr::<XbtpConnection>::default();
        let mut eb = [0usize; 2];
        let mut a = 0x1234usize as *mut xbtp::session::XbtpSession;
        let mut b = 0x1234usize as *mut c_void;
        let ours = XbtpCompleteSessionSetupInternal(&EXT, true, 1000, &mut ea, &mut a);
        let theirs = cc(&EXT, true, 1000, &mut eb, &mut b);
        assert_eq!(ours, theirs, "{ours:#x} vs {theirs:#x}");
        assert_eq!(a as usize, b as usize);
    }
}

#[test]
fn start_and_wrap_dispatcher_errors_match_console() {
    let Some(c) = console() else { return };
    unsafe {
        let cs: Start = sym(c, "XbtpStartDispatcher");
        let cx: Start = sym(c, "XbtpStartDispatcherEx");
        let cw: Wrap = sym(c, N_WRAP);
        let mut a: *mut c_void = null_mut();
        let mut b: *mut c_void = null_mut();
        for port in [2u32, 4, 0x0100_0005] {
            assert_eq!(xbtp::start::XbtpStartDispatcher(port, 0, &EXT, &mut a), cs(port, 0, &EXT, &mut b), "{port:#x}");
        }
        assert_eq!(xbtp::start::XbtpStartDispatcherEx(closed_port(), 0, &EXT, null_mut()), cx(closed_port(), 0, &EXT, null_mut()));
        let mut ea = SharedPtr::<XbtpConnectionDispatcher>::default();
        let mut eb = [0usize; 2];
        assert_eq!(XbtpWrapDispatcher(&mut ea, &EXT, &mut a), cw(&mut eb, &EXT, &mut b));
        assert!(a.is_null() && b.is_null());
    }
}

static mut CONSOLE_REG_DTOR: usize = 0;

unsafe fn drop_console_registry(p: *mut c_void) {
    let f: unsafe extern "C" fn(*mut c_void) = core::mem::transmute(CONSOLE_REG_DTOR);
    f(p);
}

unsafe fn drop_registry(p: *mut c_void) {
    xbtp::registry::XbtpExtensionRegistry_dtor(p as *mut XbtpExtensionRegistry);
}

#[test]
fn dispatcher_initialize_null_args_match_console() {
    let Some(c) = console() else { return };
    unsafe {
        let di: DInit = sym(c, N_DINIT);
        let cd = cpp_new(8) as *mut usize;
        *cd = 0;
        let mut r = [0usize; 2];
        let mut t = [0usize; 2];
        let mut l = [0usize; 2];
        let theirs = di(cd as *mut c_void, &mut r, &mut t, &mut l);
        let od = cpp_new(8) as *mut XbtpConnectionDispatcher;
        (*od).m_impl = null();
        let mut rr = SharedPtr::<XbtpExtensionRegistry>::default();
        let mut tt = SharedPtr::<XbtpConnection>::default();
        let mut ll = SharedPtr::<XbtpConnection>::default();
        let ours = XbtpConnectionDispatcher_Initialize(od, &mut rr, &mut tt, &mut ll);
        assert_eq!(ours, theirs);
        assert_eq!(ours, EINVAL);
        assert_eq!(*cd, 0);
        assert!((*od).m_impl.is_null());
    }
}

#[test]
fn add_pipe_endpoint_duplicate_path_matches_console() {
    let Some(c) = console() else { return };
    unsafe {
        let ci: Init = sym(c, "XbtpInitialize");
        assert!(ci() >= 0);
        assert!(xbtp::xbtp2_sys::XbtpInitialize() >= 0);
        CONSOLE_REG_DTOR = GetProcAddress(c.0, b"??1XbtpExtensionRegistry@@QEAA@XZ\0".as_ptr()) as usize;
        assert_ne!(CONSOLE_REG_DTOR, 0);
        let reg_init: RegInit = sym(c, N_REGINIT);
        let tcp_srv: TcpServer = sym(c, N_TCPSRV);
        let di: DInit = sym(c, N_DINIT);
        let add: DAddPipe = sym(c, N_ADDPIPE);
        let creg = cpp_new(8) as *mut usize;
        *creg = 0;
        assert_eq!(reg_init(creg as *mut c_void), 0);
        let csp = SharedPtr::new_with(creg as *mut XbtpExtensionRegistry, drop_console_registry).unwrap();
        let mut ctcp = [0usize; 2];
        assert_eq!(tcp_srv(closed_port(), &mut ctcp), 0);
        let ctcp_sp: SharedPtr<XbtpConnection> = core::mem::transmute(ctcp);
        let cd = cpp_new(8) as *mut usize;
        *cd = 0;
        let mut r: [usize; 2] = core::mem::transmute(csp.clone_ref());
        let mut t: [usize; 2] = core::mem::transmute(ctcp_sp.clone_ref());
        let mut l: [usize; 2] = core::mem::transmute(ctcp_sp.clone_ref());
        assert_eq!(di(cd as *mut c_void, &mut r, &mut t, &mut l), 0);
        let cpath = w(&format!(r"\\.\pipe\tombox-verify-dup-c-{}", std::process::id()));
        let c1 = add(cd as *mut c_void, cpath.as_ptr());
        let c2 = add(cd as *mut c_void, cpath.as_ptr());
        let cup = w(&format!(r"\\.\PIPE\TOMBOX-VERIFY-DUP-C-{}", std::process::id()));
        let c3 = add(cd as *mut c_void, cup.as_ptr());
        let oreg = cpp_new(8) as *mut XbtpExtensionRegistry;
        (*oreg).m_impl = null_mut();
        assert_eq!(XbtpExtensionRegistry_Initialize(oreg), 0);
        let osp = SharedPtr::new_with(oreg, drop_registry).unwrap();
        let mut otcp = SharedPtr::<TcpConnection>::default();
        assert_eq!(TcpConnection_InitializeServer_sp(closed_port(), &mut otcp), 0);
        let otcp: SharedPtr<XbtpConnection> = otcp.cast();
        let od = cpp_new(8) as *mut XbtpConnectionDispatcher;
        (*od).m_impl = null();
        let mut rr = osp.clone_ref();
        let mut tt = otcp.clone_ref();
        let mut ll = otcp.clone_ref();
        assert_eq!(XbtpConnectionDispatcher_Initialize(od, &mut rr, &mut tt, &mut ll), 0);
        let opath = w(&format!(r"\\.\pipe\tombox-verify-dup-o-{}", std::process::id()));
        let o1 = XbtpConnectionDispatcher_AddPipeEndpoint(od, opath.as_ptr());
        let o2 = XbtpConnectionDispatcher_AddPipeEndpoint(od, opath.as_ptr());
        let oup = w(&format!(r"\\.\PIPE\TOMBOX-VERIFY-DUP-O-{}", std::process::id()));
        let o3 = XbtpConnectionDispatcher_AddPipeEndpoint(od, oup.as_ptr());
        assert_eq!(c1, 0);
        assert_eq!(c2, EXISTS, "console second add {c2:#x}");
        assert_eq!(o1, c1);
        assert_eq!(o2, c2, "ours second add {o2:#x}, console {c2:#x}");
        println!("dup c1={c1:#x} c2={c2:#x} c3={c3:#x} o1={o1:#x} o2={o2:#x} o3={o3:#x}");
        assert_eq!(o3, c3, "case-variant add: ours {o3:#x}, console {c3:#x}");
        core::mem::forget(csp);
        core::mem::forget(ctcp_sp);
        core::mem::forget(osp);
        core::mem::forget(otcp);
    }
}
