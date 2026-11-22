use core::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use compat_common::os_host::{self, plm, routes};
use minappmodelx::{DLL_FILE, EXPORTS_TXT};

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(p: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
    fn GetCurrentThreadId() -> u32;
}

type Register = unsafe extern "system" fn(usize, usize, *mut u64) -> u32;
type Unregister = unsafe extern "system" fn(u64) -> u32;

static LOCK: Mutex<()> = Mutex::new(());
static LOG: Mutex<Vec<(u8, usize, u32)>> = Mutex::new(Vec::new());

unsafe extern "system" fn cb(value: u8, ctx: *mut c_void) {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push((value, ctx as usize, GetCurrentThreadId()));
}

fn take() -> Vec<(u8, usize, u32)> {
    std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner()))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn built() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.ancestors()
        .map(|d| d.join(DLL_FILE))
        .filter_map(|p| std::fs::metadata(&p).and_then(|m| m.modified()).ok().map(|t| (t, p)))
        .max()
        .map(|(_, p)| p)
        .unwrap_or_else(|| panic!("{DLL_FILE} not built near {}", exe.display()))
}

fn dll() -> usize {
    static H: OnceLock<usize> = OnceLock::new();
    *H.get_or_init(|| {
        std::env::set_var("TOMBOX_PLM_WATCH", "0");
        os_host::ensure();
        assert!(os_host::installed_here());
        let path = built().display().to_string();
        std::env::set_var("TOMBOX_MINAPPMODELX", &path);
        let h = unsafe { LoadLibraryW(wide(&path).as_ptr()) } as usize;
        assert_ne!(h, 0, "LoadLibraryW({path})");
        h
    })
}

fn by_name(name: &str) -> usize {
    unsafe { GetProcAddress(dll() as *mut c_void, format!("{name}\0").as_ptr()) as usize }
}

fn by_ord(n: u16) -> usize {
    unsafe { GetProcAddress(dll() as *mut c_void, n as usize as *const u8) as usize }
}

fn reg(name: &str) -> Register {
    let p = by_name(name);
    assert_ne!(p, 0, "{name} not exported");
    unsafe { core::mem::transmute(p) }
}

fn unreg(name: &str) -> Unregister {
    let p = by_name(name);
    assert_ne!(p, 0, "{name} not exported");
    unsafe { core::mem::transmute(p) }
}

#[test]
fn every_exports_txt_row_is_exported_by_name_and_ordinal() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let rows = xone_exports::parse(EXPORTS_TXT);
    assert_eq!(rows.len(), 4);
    for r in rows {
        let n = by_name(&r.name);
        assert_ne!(n, 0, "{}", r.name);
        assert_eq!(by_ord(r.ordinal), n, "{} @{}", r.name, r.ordinal);
    }
}

#[test]
fn callback_registered_through_the_dll_fires_on_simulated_suspend_and_resume() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    dll();
    take();
    let me = unsafe { GetCurrentThreadId() };
    let before = plm::registered(plm::Kind::AppState);
    let mut id = 0u64;
    assert_eq!(unsafe { reg("RegisterAppStateChangeNotification")(cb as usize, 0x5eed, &mut id) }, 0);
    assert_ne!(id, 0);
    assert_eq!(plm::registered(plm::Kind::AppState), before + 1, "the DLL registration did not reach the os_host owner's PLM state machine");
    assert_eq!(plm::suspend(), before + 1);
    assert_eq!(plm::resume(), before + 1);
    let log = take();
    assert_eq!(log.iter().map(|e| (e.0, e.1)).collect::<Vec<_>>(), vec![(1, 0x5eed), (0, 0x5eed)]);
    assert!(log.iter().all(|e| e.2 != me));
    assert_eq!(unsafe { unreg("UnregisterAppStateChangeNotification")(id) }, 0);
    assert_eq!(unsafe { unreg("UnregisterAppStateChangeNotification")(id) }, 0x490);
    plm::suspend();
    plm::resume();
    assert!(take().is_empty(), "callback fired after unregister");
}

#[test]
fn constrained_callback_through_the_dll_follows_constrained_changes() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    dll();
    take();
    let mut id = 0u64;
    assert_eq!(unsafe { reg("RegisterAppConstrainedChangeNotification")(cb as usize, 0xc0de, &mut id) }, 0);
    assert_eq!(unsafe { reg("RegisterAppConstrainedChangeNotification")(cb as usize, 0, core::ptr::null_mut()) }, 87);
    assert_eq!(plm::set_constrained(true), 1);
    assert_eq!(plm::set_constrained(false), 1);
    assert_eq!(plm::suspend(), 0);
    assert_eq!(take().iter().map(|e| (e.0, e.1)).collect::<Vec<_>>(), vec![(1, 0xc0de), (0, 0xc0de)]);
    assert_eq!(unsafe { unreg("UnregisterAppStateChangeNotification")(id) }, 0x490);
    assert_eq!(unsafe { unreg("UnregisterAppConstrainedChangeNotification")(id) }, 0);
    assert_eq!(plm::set_constrained(true), 0);
    assert!(take().is_empty());
}

#[test]
fn psm_appnotify_imports_route_to_this_crate_not_the_pc_twin() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    dll();
    let table = os_host::hook_table();
    for build in [19041u32, 22000, 22621] {
        for (set, func) in [
            ("api-ms-win-core-psm-appnotify-l1-1-0.dll", "RegisterAppStateChangeNotification"),
            ("api-ms-win-core-psm-appnotify-l1-1-0.dll", "UnregisterAppStateChangeNotification"),
            ("api-ms-win-core-psm-appnotify-l1-1-1.dll", "RegisterAppConstrainedChangeNotification"),
            ("api-ms-win-core-psm-appnotify-l1-1-1.dll", "UnregisterAppConstrainedChangeNotification"),
        ] {
            assert_eq!(routes::host_of(set, func, build).as_deref(), Some("minappmodelx.dll"));
            let r = routes::lookup(set, func, build).unwrap_or_else(|| panic!("{set}!{func} @{build} has no route"));
            assert_eq!(r.lib, "minappmodelx");
            assert!(!routes::covered(set, func, table), "{set}!{func} is still taken by an os_host IAT hook");
            assert_eq!(unsafe { routes::target(r) }, by_name(func), "{func}");
        }
    }
}
