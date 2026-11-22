use super::*;
use std::sync::{Mutex, Once};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn init() -> std::sync::MutexGuard<'static, ()> {
    static I: Once = Once::new();
    I.call_once(|| {
        let d = std::env::temp_dir().join(format!("tombox-sandbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::env::set_var("TOMBOX_GDK_ROOT", &d);
    });
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn w(s: &str) -> Vec<u16> {
    wstr::wide(s)
}

#[test]
fn paths_stay_inside_the_sandbox() {
    let _g = init();
    let (c, h) = fs::resolve(r"c:\windows\..\..\..\..\x\y.txt").unwrap();
    assert_eq!(c, r"C:\x\y.txt");
    assert!(h.starts_with(fs::fs_root()));
    assert_eq!(fs::resolve(r"boot\a.bsc").unwrap().0, r"C:\Windows\System32\boot\a.bsc");
    assert!(fs::resolve(r"\\server\share\f").is_none());
    assert!(fs::resolve(r"\\.\PhysicalDrive0").is_none());
}

#[test]
fn reg_get_value_reports_sizes_types_and_missing_values() {
    let _g = init();
    let key = r"\Registry\Machine\SOFTWARE\TomBoxSandboxTest";
    registry::set_string(key, "Name", "abc");
    registry::set(key, "Num", registry::REG_DWORD, &7u32.to_le_bytes());
    let mut h = 0usize;
    unsafe {
        assert_eq!(registry::reg_open_key_ex_w(registry::HKEY_LOCAL_MACHINE, w(r"SOFTWARE\TomBoxSandboxTest").as_ptr(), 0, 0x20019, &mut h), 0);
        let mut size = 0u32;
        let mut kind = 0u32;
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Name").as_ptr(), 2, &mut kind, core::ptr::null_mut(), &mut size), 0);
        assert_eq!((kind, size), (registry::REG_SZ, 8));
        let mut small = [0u8; 4];
        size = 4;
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Name").as_ptr(), 2, &mut kind, small.as_mut_ptr(), &mut size), registry::ERROR_MORE_DATA);
        assert_eq!(size, 8);
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Num").as_ptr(), 2, &mut kind, core::ptr::null_mut(), &mut size), registry::ERROR_UNSUPPORTED_TYPE);
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Gone").as_ptr(), 0xffff, &mut kind, core::ptr::null_mut(), &mut size), registry::ERROR_FILE_NOT_FOUND);
        let mut name = [0u16; 16];
        let mut nlen = 16u32;
        assert_eq!(registry::reg_enum_value_w(h, 1, name.as_mut_ptr(), &mut nlen, core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), core::ptr::null_mut()), 0);
        assert_eq!(String::from_utf16_lossy(&name[..nlen as usize]), "Num");
        nlen = 16;
        assert_eq!(registry::reg_enum_value_w(h, 2, name.as_mut_ptr(), &mut nlen, core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), core::ptr::null_mut()), registry::ERROR_NO_MORE_ITEMS);
        assert_eq!(registry::reg_close_key(h), 0);
        assert_eq!(registry::reg_close_key(h), registry::ERROR_INVALID_HANDLE);
    }
}

#[test]
fn scm_queries_answer_from_the_in_process_host() {
    let _g = init();
    host::add_service("TomBoxSvc");
    unsafe {
        let scm = host::open_sc_manager_w(core::ptr::null(), core::ptr::null(), 1);
        assert_ne!(scm, 0);
        assert_eq!(host::open_service_w(scm, w("NoSuchSvc").as_ptr(), 4), 0);
        let s = host::open_service_w(scm, w("TomBoxSvc").as_ptr(), 4);
        assert_ne!(s, 0);
        let mut buf = [0u32; 9];
        let mut need = 0u32;
        assert_eq!(host::query_service_status_ex(s, 0, buf.as_mut_ptr() as *mut u8, 8, &mut need), 0);
        assert_eq!(need, 36);
        assert_eq!(host::query_service_status_ex(s, 0, buf.as_mut_ptr() as *mut u8, 36, &mut need), 1);
        assert_eq!(buf[1], host::SERVICE_STOPPED);
        assert_eq!(host::close_service_handle(s), 1);
        assert_eq!(host::close_service_handle(s), 0);
    }
}

#[test]
fn hostname_size_contract_matches_get_computer_name_ex() {
    let _g = init();
    registry::set_string(system::COMPUTER_NAME, "ComputerName", "SANDBOXPC");
    let mut b = [0u16; 4];
    let mut n = 4u32;
    unsafe {
        assert_eq!(system::get_computer_name_ex_w(4, b.as_mut_ptr(), &mut n), 0);
        assert_eq!(n, 10);
        let mut big = [0u16; 16];
        n = 16;
        assert_eq!(system::get_computer_name_ex_w(4, big.as_mut_ptr(), &mut n), 1);
        assert_eq!(String::from_utf16_lossy(&big[..n as usize]), "SANDBOXPC");
        assert_eq!(system::get_computer_name_ex_w(9, big.as_mut_ptr(), &mut n), 0);
    }
}
#[test]
fn import_walk_includes_delay_loads() {
    let _g = init();
    let m = console_image::load_path(std::path::Path::new(r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\netprofmsvc_c.dll"));
    if m == 0 {
        return;
    }
    let imps = unsafe { imports(m) };
    assert!(imps.iter().any(|(d, n)| d.eq_ignore_ascii_case("NSI.dll") && n == "NsiSetAllParameters"), "delay import listed");
}
#[test]
fn reg_query_enum_and_delete_stay_in_the_emulated_hive() {
    let _g = init();
    let key = r"\Registry\Machine\SOFTWARE\TomBoxRegW";
    registry::set(key, "Bin", registry::REG_BINARY, &[1, 2, 3]);
    registry::create_key(&format!(r"{key}\Beta"));
    registry::create_key(&format!(r"{key}\Alpha"));
    let mut h = 0usize;
    unsafe {
        assert_eq!(registry::reg_open_key_ex_w(registry::HKEY_LOCAL_MACHINE, w(r"SOFTWARE\TomBoxRegW").as_ptr(), 0, 0x20019, &mut h), 0);
        let (mut kind, mut size) = (0u32, 0u32);
        assert_eq!(regw::reg_query_value_ex_w(h, w("bin").as_ptr(), core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), &mut size), 0);
        assert_eq!((kind, size), (registry::REG_BINARY, 3));
        let mut buf = [0u8; 2];
        size = 2;
        assert_eq!(regw::reg_query_value_ex_w(h, w("Bin").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), buf.as_mut_ptr(), &mut size), registry::ERROR_MORE_DATA);
        assert_eq!(size, 3);
        let mut big = [0u8; 8];
        size = 8;
        assert_eq!(regw::reg_query_value_ex_w(h, w("Bin").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), big.as_mut_ptr(), &mut size), 0);
        assert_eq!(&big[..3], &[1, 2, 3]);
        assert_eq!(regw::reg_query_value_ex_w(h, w("Nope").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), &mut size), registry::ERROR_FILE_NOT_FOUND);
        assert_eq!(regw::reg_query_value_ex_w(h, w("Bin").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), big.as_mut_ptr(), core::ptr::null_mut()), registry::ERROR_INVALID_PARAMETER);
        let mut name = [0u16; 16];
        let mut n = 16u32;
        assert_eq!(regw::reg_enum_key_ex_w(h, 0, name.as_mut_ptr(), &mut n, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()), 0);
        assert_eq!(String::from_utf16_lossy(&name[..n as usize]), "Alpha");
        let mut small = [0u16; 3];
        n = 3;
        assert_eq!(regw::reg_enum_key_ex_w(h, 1, small.as_mut_ptr(), &mut n, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()), registry::ERROR_MORE_DATA);
        assert_eq!(regw::reg_enum_key_w(h, 1, name.as_mut_ptr(), 16), 0);
        assert_eq!(String::from_utf16_lossy(&name[..4]), "Beta");
        assert_eq!(regw::reg_enum_key_w(h, 2, name.as_mut_ptr(), 16), registry::ERROR_NO_MORE_ITEMS);
        assert_eq!(regw::reg_delete_value_w(h, w("Bin").as_ptr()), 0);
        assert_eq!(regw::reg_delete_value_w(h, w("Bin").as_ptr()), registry::ERROR_FILE_NOT_FOUND);
        assert!(registry::get(key, "Bin").is_none());
        assert_eq!(regw::reg_delete_key_w(registry::HKEY_LOCAL_MACHINE, w(r"SOFTWARE\TomBoxRegW").as_ptr()), regw::ERROR_ACCESS_DENIED);
        assert_eq!(regw::reg_delete_key_w(h, w("Alpha").as_ptr()), 0);
        assert!(!registry::key_exists(&format!(r"{key}\Alpha")));
        assert_eq!(regw::reg_delete_key_w(h, w("Alpha").as_ptr()), registry::ERROR_FILE_NOT_FOUND);
        assert_eq!(regw::reg_enum_key_w(h, 0, name.as_mut_ptr(), 16), 0);
        assert_eq!(String::from_utf16_lossy(&name[..4]), "Beta");
        registry::reg_close_key(h);
    }
}

static NOTIFIED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe extern "system" fn on_notify(buffer: *mut core::ffi::c_void) {
    let b = buffer as *const u8;
    let flag = (b.add(0x10) as *const *mut u8).read_unaligned();
    *flag = 1;
    NOTIFIED.store(((b.add(0x40) as *const u32).read_unaligned() as usize) << 8 | (b.add(0x20) as *const u32).read_unaligned() as usize, std::sync::atomic::Ordering::SeqCst);
}

#[link(name = "kernel32")]
extern "system" {
    fn SleepEx(ms: u32, alertable: i32) -> u32;
}

unsafe extern "system" fn no_handler(_c: u32, _t: u32, _d: *mut core::ffi::c_void, _x: *mut core::ffi::c_void) -> u32 {
    0
}

#[test]
fn scm_a_and_status_notifications_follow_win32() {
    let _g = init();
    unsafe {
        let scm = notify::open_sc_manager_a(core::ptr::null(), core::ptr::null(), 4);
        assert_ne!(scm, 0);
        assert_eq!(notify::open_service_a(scm, b"No/Such\0".as_ptr(), 5), 0);
        let rpcss = notify::open_service_a(scm, b"RpcSs\0".as_ptr(), 5);
        assert_ne!(rpcss, 0, "RpcSs is the live in-process RPC runtime");
        let mut flag = 0u8;
        let mut buf = [0u8; 0x50];
        (buf.as_mut_ptr() as *mut u32).write_unaligned(2);
        (buf.as_mut_ptr().add(8) as *mut usize).write_unaligned(on_notify as *const () as usize);
        (buf.as_mut_ptr().add(0x10) as *mut usize).write_unaligned(&mut flag as *mut u8 as usize);
        assert_eq!(notify::notify_service_status_change(rpcss, 0, buf.as_mut_ptr().cast()), host::ERROR_INVALID_PARAMETER);
        assert_eq!(notify::notify_service_status_change(0x1234, 8, buf.as_mut_ptr().cast()), host::ERROR_INVALID_HANDLE);
        assert_eq!(notify::notify_service_status_change(rpcss, 8, buf.as_mut_ptr().cast()), 0);
        assert_eq!(flag, 0, "the callback is an APC, not a direct call");
        assert_eq!(SleepEx(5000, 1), 0xc0, "WAIT_IO_COMPLETION");
        assert_eq!(flag, 1);
        assert_eq!(NOTIFIED.load(std::sync::atomic::Ordering::SeqCst), (8 << 8) | host::SERVICE_RUNNING as usize);
        assert_eq!(&buf[0x18..0x1c], &[0, 0, 0, 0]);

        host::add_service("TomBoxNotifySvc");
        let svc = notify::open_service_a(scm, b"TomBoxNotifySvc\0".as_ptr(), 5);
        assert_ne!(svc, 0);
        flag = 0;
        assert_eq!(notify::notify_service_status_change(svc, 8, buf.as_mut_ptr().cast()), 0);
        assert_eq!(SleepEx(0, 1), 0, "stopped service: nothing delivered yet");
        let name = w("TomBoxNotifySvc");
        let st = host::register_service_ctrl_handler_ex_w(name.as_ptr(), no_handler as *mut core::ffi::c_void, core::ptr::null_mut());
        assert_ne!(st, 0);
        let start_pending = [0x10u32, 2, 0, 0, 0, 0, 0];
        assert_eq!(host::set_service_status(st, start_pending.as_ptr()), 1);
        assert_eq!(SleepEx(0, 1), 0, "START_PENDING is not in the RUNNING mask");
        let running = [0x10u32, 4, 1, 0, 0, 0, 0];
        assert_eq!(host::set_service_status(st, running.as_ptr()), 1);
        assert_eq!(SleepEx(5000, 1), 0xc0);
        assert_eq!(flag, 1);
        flag = 0;
        assert_eq!(notify::notify_service_status_change(svc, 1, buf.as_mut_ptr().cast()), 0);
        assert_eq!(host::close_service_handle(svc), 1);
        let stopped = [0x10u32, 1, 0, 0, 0, 0, 0];
        assert_eq!(host::set_service_status(st, stopped.as_ptr()), 1);
        assert_eq!(SleepEx(0, 1), 0, "closing the handle cancels its notification");
        assert_eq!(flag, 0);
        host::close_service_handle(rpcss);
        host::close_service_handle(scm);
    }
}
