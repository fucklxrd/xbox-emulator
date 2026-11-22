use core::ffi::c_void;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(p: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
}

type Register = unsafe extern "system" fn(usize, usize, *mut u64) -> u32;
type Unregister = unsafe extern "system" fn(u64) -> u32;
type Dump = unsafe extern "system" fn(usize, usize);

static LOG: Mutex<Vec<(u8, usize)>> = Mutex::new(Vec::new());

unsafe extern "system" fn cb(value: u8, ctx: *mut c_void) {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push((value, ctx as usize));
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn built(file: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.ancestors().map(|d| d.join(file)).find(|p| p.is_file()).unwrap_or_else(|| panic!("{file} not built"))
}

fn load(file: &str) -> *mut c_void {
    let p = built(file);
    let h = unsafe { LoadLibraryW(wide(p.to_str().unwrap()).as_ptr()) };
    assert!(!h.is_null(), "{}", p.display());
    h
}

fn sym(h: *mut c_void, n: &str) -> usize {
    let p = unsafe { GetProcAddress(h, format!("{n}\0").as_ptr()) } as usize;
    assert_ne!(p, 0, "{n}");
    p
}

fn command(dir: &PathBuf, cmd: &str) {
    let p = dir.join(format!("{}.cmd", unsafe { GetCurrentProcessId() }));
    std::fs::write(&p, format!("{cmd}\n")).unwrap();
    let t = Instant::now();
    while p.exists() && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!p.exists(), "owner PLM watcher never consumed {cmd}");
    std::thread::sleep(Duration::from_millis(300));
}

fn take() -> Vec<(u8, usize)> {
    std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner()))
}

#[test]
fn registration_through_a_non_owner_minappmodelx_reaches_the_owner_watcher() {
    let dir = std::env::temp_dir().join(format!("tombox_plm_verify_{}", unsafe { GetCurrentProcessId() }));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TOMBOX_PLM_DIR", &dir);
    std::env::remove_var("TOMBOX_PLM_WATCH");
    std::env::remove_var("TOMBOX_OSHOST_OWNER");
    let xm = load("xmem.dll");
    let dump: Dump = unsafe { core::mem::transmute(sym(xm, "XMemDumpMemoryStats")) };
    unsafe { dump(1, 1) };
    let owner_signal = std::env::var("TOMBOX_OSHOST_SIGNAL").expect("xmem did not become the os_host owner");
    let mam = load("minappmodelx.dll");
    let reg: Register = unsafe { core::mem::transmute(sym(mam, "RegisterAppStateChangeNotification")) };
    let unreg: Unregister = unsafe { core::mem::transmute(sym(mam, "UnregisterAppStateChangeNotification")) };
    let regc: Register = unsafe { core::mem::transmute(sym(mam, "RegisterAppConstrainedChangeNotification")) };
    let unregc: Unregister = unsafe { core::mem::transmute(sym(mam, "UnregisterAppConstrainedChangeNotification")) };
    assert_eq!(std::env::var("TOMBOX_OSHOST_SIGNAL").unwrap(), owner_signal);
    let (mut a, mut c) = (0u64, 0u64);
    unsafe {
        assert_eq!(reg(cb as usize, 0xa5, &mut a), 0);
        assert_eq!(regc(cb as usize, 0xc5, &mut c), 0);
    }
    assert_eq!(c, a + 1);
    assert_eq!(std::env::var("TOMBOX_OSHOST_SIGNAL").unwrap(), owner_signal, "minappmodelx took ownership away from the first compat DLL");
    command(&dir, "suspend");
    command(&dir, "resume");
    command(&dir, "constrain");
    command(&dir, "unconstrain");
    assert_eq!(take(), vec![(1, 0xa5), (0, 0xa5), (1, 0xc5), (0, 0xc5)]);
    unsafe {
        assert_eq!(unregc(a), 0x490);
        assert_eq!(unreg(a), 0);
        assert_eq!(unregc(c), 0);
        assert_eq!(unreg(a), 0x490);
    }
    command(&dir, "suspend");
    command(&dir, "constrain");
    assert!(take().is_empty(), "callback after unregister");
    let _ = std::fs::remove_dir_all(&dir);
}
