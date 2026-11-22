use console_sandbox::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Once};

static LOCK: Mutex<()> = Mutex::new(());

fn init() -> std::sync::MutexGuard<'static, ()> {
    static I: Once = Once::new();
    I.call_once(|| {
        let d = std::env::temp_dir().join(format!("tombox-verify-sandbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::env::set_var("TOMBOX_GDK_ROOT", &d);
    });
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn w(s: &str) -> Vec<u16> {
    wstr::wide(s)
}

#[link(name = "kernel32")]
extern "system" {
    fn SleepEx(ms: u32, alertable: i32) -> u32;
    fn GetLastError() -> u32;
    fn GetCurrentThreadId() -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(h: usize, sub: *const u16, opt: u32, sam: u32, out: *mut usize) -> u32;
    fn RegCloseKey(h: usize) -> u32;
}

static FIRED_ON: AtomicUsize = AtomicUsize::new(0);
static FIRED_STATE: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn on_notify(buffer: *mut core::ffi::c_void) {
    let b = buffer as *const u8;
    FIRED_ON.store(GetCurrentThreadId() as usize, Ordering::SeqCst);
    FIRED_STATE.store((b.add(0x20) as *const u32).read_unaligned() as usize, Ordering::SeqCst);
}

unsafe extern "system" fn no_handler(_c: u32, _t: u32, _d: *mut core::ffi::c_void, _x: *mut core::ffi::c_void) -> u32 {
    0
}

fn notify_buffer() -> Box<[u8; 0x50]> {
    let mut b = Box::new([0u8; 0x50]);
    unsafe {
        (b.as_mut_ptr() as *mut u32).write_unaligned(2);
        (b.as_mut_ptr().add(8) as *mut usize).write_unaligned(on_notify as *const () as usize);
    }
    b
}

#[test]
fn delete_pending_mask_is_accepted_on_a_service_handle() {
    let _g = init();
    unsafe {
        host::add_service("TomBoxVerifyDelPend");
        let scm = notify::open_sc_manager_a(core::ptr::null(), core::ptr::null(), 4);
        let svc = notify::open_service_a(scm, b"TomBoxVerifyDelPend\0".as_ptr(), 5);
        assert_ne!(svc, 0);
        let mut b = notify_buffer();
        assert_eq!(notify::notify_service_status_change(svc, 0x200, b.as_mut_ptr().cast()), 0, "SERVICE_NOTIFY_DELETE_PENDING is a valid service-handle mask");
        assert_eq!(notify::notify_service_status_change(svc, 0x201 | 0x08, b.as_mut_ptr().cast()), 0, "mixed state and DELETE_PENDING mask");
        assert_eq!(notify::notify_service_status_change(svc, 0x400, b.as_mut_ptr().cast()), host::ERROR_INVALID_PARAMETER);
        host::close_service_handle(svc);
    }
}

#[test]
fn notification_apc_targets_the_registering_thread() {
    let _g = init();
    unsafe {
        host::add_service("TomBoxVerifyThread");
        let scm = notify::open_sc_manager_a(core::ptr::null(), core::ptr::null(), 4);
        let svc = notify::open_service_a(scm, b"TomBoxVerifyThread\0".as_ptr(), 5);
        assert_ne!(svc, 0);
        let (tx, rx) = std::sync::mpsc::channel::<u32>();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let t = std::thread::spawn(move || {
            let mut b = notify_buffer();
            let r = notify::notify_service_status_change(svc, 8, b.as_mut_ptr().cast());
            tx.send(r).unwrap();
            tx.send(GetCurrentThreadId()).unwrap();
            go_rx.recv().unwrap();
            let woke = SleepEx(5000, 1);
            (woke, b)
        });
        assert_eq!(rx.recv().unwrap(), 0);
        let waiter = rx.recv().unwrap() as usize;
        let name = w("TomBoxVerifyThread");
        let st = host::register_service_ctrl_handler_ex_w(name.as_ptr(), no_handler as *mut core::ffi::c_void, core::ptr::null_mut());
        assert_ne!(st, 0);
        let running = [0x10u32, 4, 1, 0, 0, 0, 0];
        assert_eq!(host::set_service_status(st, running.as_ptr()), 1);
        assert_eq!(SleepEx(0, 1), 0, "the APC must not land on the thread that called SetServiceStatus");
        go_tx.send(()).unwrap();
        let (woke, b) = t.join().unwrap();
        assert_eq!(woke, 0xc0, "WAIT_IO_COMPLETION on the registering thread");
        assert_eq!(FIRED_ON.load(Ordering::SeqCst), waiter);
        assert_eq!(FIRED_STATE.load(Ordering::SeqCst), host::SERVICE_RUNNING as usize);
        assert_eq!((b.as_ptr().add(0x40) as *const u32).read_unaligned(), 8, "dwNotificationTriggered");
        assert_eq!((b.as_ptr().add(0x18) as *const u32).read_unaligned(), 0, "dwNotificationStatus");
        host::close_service_handle(svc);
    }
}

#[test]
fn closed_service_handles_are_dead() {
    let _g = init();
    unsafe {
        let scm = notify::open_sc_manager_a(core::ptr::null(), core::ptr::null(), 4);
        let svc = notify::open_service_a(scm, b"RpcSs\0".as_ptr(), 5);
        assert_ne!(svc, 0);
        assert_eq!(host::close_service_handle(svc), 1);
        assert_eq!(host::close_service_handle(svc), 0);
        assert_eq!(GetLastError(), host::ERROR_INVALID_HANDLE);
        let mut b = notify_buffer();
        assert_eq!(notify::notify_service_status_change(svc, 8, b.as_mut_ptr().cast()), host::ERROR_INVALID_HANDLE);
        assert_eq!(notify::notify_service_status_change(svc + 1, 8, b.as_mut_ptr().cast()), host::ERROR_INVALID_HANDLE);
        assert_eq!(notify::notify_service_status_change(0, 8, b.as_mut_ptr().cast()), host::ERROR_INVALID_HANDLE);
        let mut bad = notify_buffer();
        (bad.as_mut_ptr() as *mut u32).write_unaligned(1);
        let live = notify::open_service_a(scm, b"RpcSs\0".as_ptr(), 5);
        assert_eq!(notify::notify_service_status_change(live, 8, bad.as_mut_ptr().cast()), host::ERROR_INVALID_PARAMETER);
        assert_eq!(notify::notify_service_status_change(live, 8, core::ptr::null_mut()), host::ERROR_INVALID_PARAMETER);
        assert_eq!(SleepEx(0, 1), 0, "rejected registrations queue nothing");
        host::close_service_handle(live);
    }
}

#[test]
fn query_value_size_contract() {
    let _g = init();
    unsafe {
        let mut h = 0usize;
        let sub = w("SOFTWARE\\TomBoxVerify\\Query");
        assert_eq!(registry::reg_create_key_ex_w(registry::HKEY_LOCAL_MACHINE, sub.as_ptr(), 0, core::ptr::null(), 0, 0, core::ptr::null(), &mut h, core::ptr::null_mut()), 0);
        let name = w("Dw");
        let v = 0x11223344u32.to_le_bytes();
        assert_eq!(registry::reg_set_value_ex_w(h, name.as_ptr(), 0, registry::REG_DWORD, v.as_ptr(), 4), 0);
        let mut kind = 0xffu32;
        let mut size = 3u32;
        let mut out = [0u8; 8];
        assert_eq!(regw::reg_query_value_ex_w(h, name.as_ptr(), core::ptr::null_mut(), &mut kind, out.as_mut_ptr(), &mut size), registry::ERROR_MORE_DATA);
        assert_eq!((kind, size), (registry::REG_DWORD, 4));
        assert_eq!(out, [0u8; 8], "ERROR_MORE_DATA writes no data");
        size = 8;
        assert_eq!(regw::reg_query_value_ex_w(h, name.as_ptr(), core::ptr::null_mut(), &mut kind, out.as_mut_ptr(), &mut size), 0);
        assert_eq!((size, &out[..4], &out[4..]), (4, &v[..], &[0u8; 4][..]));
        assert_eq!(regw::reg_query_value_ex_w(h, name.as_ptr(), core::ptr::null_mut(), &mut kind, out.as_mut_ptr(), core::ptr::null_mut()), registry::ERROR_INVALID_PARAMETER);
        assert_eq!(regw::reg_query_value_ex_w(h, name.as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()), 0, "existence probe");
        let missing = w("Nope");
        size = 8;
        assert_eq!(regw::reg_query_value_ex_w(h, missing.as_ptr(), core::ptr::null_mut(), &mut kind, out.as_mut_ptr(), &mut size), registry::ERROR_FILE_NOT_FOUND);
        assert_eq!(regw::reg_query_value_ex_w(0x1357, name.as_ptr(), core::ptr::null_mut(), &mut kind, out.as_mut_ptr(), &mut size), registry::ERROR_INVALID_HANDLE);
        registry::reg_close_key(h);
    }
}

#[test]
fn enum_and_delete_key_contract() {
    let _g = init();
    unsafe {
        let mut parent = 0usize;
        let sub = w("SOFTWARE\\TomBoxVerify\\Enum");
        assert_eq!(registry::reg_create_key_ex_w(registry::HKEY_LOCAL_MACHINE, sub.as_ptr(), 0, core::ptr::null(), 0, 0, core::ptr::null(), &mut parent, core::ptr::null_mut()), 0);
        let mut child = 0usize;
        let leaf = w("LeafKey");
        assert_eq!(registry::reg_create_key_ex_w(parent, leaf.as_ptr(), 0, core::ptr::null(), 0, 0, core::ptr::null(), &mut child, core::ptr::null_mut()), 0);
        let mut grand = 0usize;
        let g = w("Grand");
        assert_eq!(registry::reg_create_key_ex_w(child, g.as_ptr(), 0, core::ptr::null(), 0, 0, core::ptr::null(), &mut grand, core::ptr::null_mut()), 0);
        registry::reg_close_key(grand);

        let mut name = [0xffffu16; 16];
        let mut len = 7u32;
        assert_eq!(regw::reg_enum_key_ex_w(parent, 0, name.as_mut_ptr(), &mut len, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()), registry::ERROR_MORE_DATA, "7 chars needs 8 with the terminator");
        len = 8;
        let mut ft = 0u64;
        assert_eq!(regw::reg_enum_key_ex_w(parent, 0, name.as_mut_ptr(), &mut len, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), &mut ft), 0);
        assert_eq!((len, String::from_utf16_lossy(&name[..7]), name[7]), (7, "LeafKey".to_string(), 0));
        assert!(ft > 116_444_736_000_000_000, "FILETIME after 1970");
        len = 16;
        assert_eq!(regw::reg_enum_key_ex_w(parent, 1, name.as_mut_ptr(), &mut len, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()), registry::ERROR_NO_MORE_ITEMS);
        assert_eq!(regw::reg_enum_key_w(parent, 0, name.as_mut_ptr(), 7), registry::ERROR_MORE_DATA);
        assert_eq!(regw::reg_enum_key_w(parent, 0, name.as_mut_ptr(), 8), 0);

        assert_eq!(regw::reg_delete_key_w(parent, leaf.as_ptr()), regw::ERROR_ACCESS_DENIED, "RegDeleteKeyW refuses keys with subkeys");
        assert_eq!(regw::reg_delete_key_w(child, g.as_ptr()), 0);
        assert_eq!(regw::reg_delete_key_w(child, g.as_ptr()), registry::ERROR_FILE_NOT_FOUND);
        assert_eq!(regw::reg_delete_key_w(parent, leaf.as_ptr()), 0);
        len = 16;
        assert_eq!(regw::reg_enum_key_ex_w(parent, 0, name.as_mut_ptr(), &mut len, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()), registry::ERROR_NO_MORE_ITEMS);
        assert_eq!(regw::reg_delete_key_w(parent, core::ptr::null()), registry::ERROR_INVALID_PARAMETER);

        let dw = w("X");
        let one = 1u32.to_le_bytes();
        assert_eq!(registry::reg_set_value_ex_w(child, dw.as_ptr(), 0, registry::REG_DWORD, one.as_ptr(), 4), 1018, "ERROR_KEY_DELETED on a handle to a deleted key");
        let mut again = 0usize;
        let sub2 = w("Again");
        assert_eq!(registry::reg_create_key_ex_w(child, sub2.as_ptr(), 0, core::ptr::null(), 0, 0, core::ptr::null(), &mut again, core::ptr::null_mut()), 1018, "a deleted key is not resurrected through a stale handle");
        let mut probe = 0usize;
        let full = w("SOFTWARE\\TomBoxVerify\\Enum\\LeafKey");
        assert_eq!(registry::reg_open_key_ex_w(registry::HKEY_LOCAL_MACHINE, full.as_ptr(), 0, 0, &mut probe), registry::ERROR_FILE_NOT_FOUND);
        registry::reg_close_key(child);
        registry::reg_close_key(parent);
    }
}

#[test]
fn delete_value_and_persistence_stay_off_the_host() {
    let _g = init();
    unsafe {
        let mut h = 0usize;
        let sub = w("Software\\TomBoxVerifyHostProbe");
        assert_eq!(registry::reg_create_key_ex_w(registry::HKEY_CURRENT_USER, sub.as_ptr(), 0, core::ptr::null(), 0, 0, core::ptr::null(), &mut h, core::ptr::null_mut()), 0);
        let name = w("Probe");
        let data = w("value");
        assert_eq!(registry::reg_set_value_ex_w(h, name.as_ptr(), 0, registry::REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32), 0);
        let mut host_key = 0usize;
        let r = RegOpenKeyExW(registry::HKEY_CURRENT_USER, sub.as_ptr(), 0, 0x20019, &mut host_key);
        if r == 0 {
            RegCloseKey(host_key);
        }
        assert_eq!(r, 2, "the PC registry never sees the emulated key");
        let file = std::path::PathBuf::from(std::env::var("TOMBOX_GDK_ROOT").unwrap()).join("console").join("registry.txt");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("TomBoxVerifyHostProbe"), "persisted in the emulated hive file {}", file.display());
        let upper = w("PROBE");
        assert_eq!(regw::reg_delete_value_w(h, upper.as_ptr()), 0, "value names are case-insensitive");
        assert_eq!(regw::reg_delete_value_w(h, name.as_ptr()), registry::ERROR_FILE_NOT_FOUND);
        assert!(!std::fs::read_to_string(&file).unwrap().contains("V\tProbe\t"), "delete is persisted");
        registry::reg_close_key(h);
    }
}
