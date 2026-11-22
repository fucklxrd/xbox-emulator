use core::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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

static LOCK: Mutex<()> = Mutex::new(());
static UNREG: AtomicUsize = AtomicUsize::new(0);
static SELF_ID: AtomicU64 = AtomicU64::new(0);
static SELF_RC: AtomicUsize = AtomicUsize::new(usize::MAX);
static DONE: AtomicUsize = AtomicUsize::new(0);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn mam() -> *mut c_void {
    static H: Mutex<usize> = Mutex::new(0);
    let mut g = H.lock().unwrap();
    if *g == 0 {
        let dir = std::env::temp_dir().join(format!("tombox_plm_inflight_{}", unsafe { GetCurrentProcessId() }));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("TOMBOX_PLM_DIR", &dir);
        let exe = std::env::current_exe().unwrap();
        let p: PathBuf = exe.ancestors().map(|d| d.join("minappmodelx.dll")).find(|p| p.is_file()).expect("minappmodelx.dll");
        *g = unsafe { LoadLibraryW(wide(p.to_str().unwrap()).as_ptr()) } as usize;
        assert_ne!(*g, 0);
    }
    *g as *mut c_void
}

fn f<T>(n: &str) -> T {
    let p = unsafe { GetProcAddress(mam(), format!("{n}\0").as_ptr()) } as usize;
    assert_ne!(p, 0);
    unsafe { core::mem::transmute_copy(&p) }
}

fn command(cmd: &str) {
    let dir = std::env::temp_dir().join(format!("tombox_plm_inflight_{}", unsafe { GetCurrentProcessId() }));
    let p = dir.join(format!("{}.cmd", unsafe { GetCurrentProcessId() }));
    std::fs::write(&p, format!("{cmd}\n")).unwrap();
    let t = Instant::now();
    while p.exists() && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!p.exists());
}

unsafe extern "system" fn self_unregister(_v: u8, _c: *mut c_void) {
    let u: Unregister = core::mem::transmute(UNREG.load(Ordering::SeqCst));
    SELF_RC.store(u(SELF_ID.load(Ordering::SeqCst)) as usize, Ordering::SeqCst);
    DONE.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "system" fn slow(_v: u8, _c: *mut c_void) {
    std::thread::sleep(Duration::from_millis(400));
    DONE.fetch_add(1, Ordering::SeqCst);
}

fn wait_done(n: usize) {
    let t = Instant::now();
    while DONE.load(Ordering::SeqCst) < n && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn unregister_from_inside_its_own_callback_does_not_deadlock() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let reg: Register = f("RegisterAppStateChangeNotification");
    let unreg: Unregister = f("UnregisterAppStateChangeNotification");
    UNREG.store(unreg as usize, Ordering::SeqCst);
    DONE.store(0, Ordering::SeqCst);
    let mut id = 0u64;
    assert_eq!(unsafe { reg(self_unregister as usize, 0, &mut id) }, 0);
    SELF_ID.store(id, Ordering::SeqCst);
    command("suspend");
    wait_done(1);
    assert_eq!(DONE.load(Ordering::SeqCst), 1, "self-unregistering callback deadlocked or never ran");
    assert_eq!(SELF_RC.load(Ordering::SeqCst), 0);
    assert_eq!(unsafe { unreg(id) }, 0x490);
}

#[test]
fn unregister_waits_for_an_in_flight_callback() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let reg: Register = f("RegisterAppConstrainedChangeNotification");
    let unreg: Unregister = f("UnregisterAppConstrainedChangeNotification");
    DONE.store(0, Ordering::SeqCst);
    let mut id = 0u64;
    assert_eq!(unsafe { reg(slow as usize, 0, &mut id) }, 0);
    let dir = std::env::temp_dir().join(format!("tombox_plm_inflight_{}", unsafe { GetCurrentProcessId() }));
    let p = dir.join(format!("{}.cmd", unsafe { GetCurrentProcessId() }));
    std::fs::write(&p, "constrain\n").unwrap();
    let t = Instant::now();
    while p.exists() && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(DONE.load(Ordering::SeqCst), 0);
    assert_eq!(unsafe { unreg(id) }, 0);
    assert_eq!(DONE.load(Ordering::SeqCst), 1, "unregister returned while the callback was still running");
    command("unconstrain");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(DONE.load(Ordering::SeqCst), 1);
}
