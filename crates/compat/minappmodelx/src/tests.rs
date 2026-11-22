use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());
static LOG: Mutex<Vec<(u8, usize, u32)>> = Mutex::new(Vec::new());

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThreadId() -> u32;
}

unsafe extern "system" fn cb(value: u8, ctx: *mut core::ffi::c_void) {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push((value, ctx as usize, GetCurrentThreadId()));
}

fn take() -> Vec<(u8, usize, u32)> {
    std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner()))
}

fn setup() -> std::sync::MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("TOMBOX_PLM_WATCH", "0");
    os_host::ensure();
    take();
    g
}

#[test]
fn exports_table_matches_the_console_ordinals() {
    let rows = xone_exports::parse(EXPORTS_TXT);
    let got: Vec<(u16, &str, &str)> = rows.iter().map(|r| (r.ordinal, r.name.as_str(), r.status.as_str())).collect();
    assert_eq!(
        got,
        vec![
            (1, "RegisterAppConstrainedChangeNotification", "implemented"),
            (2, "RegisterAppStateChangeNotification", "implemented"),
            (3, "UnregisterAppConstrainedChangeNotification", "implemented"),
            (4, "UnregisterAppStateChangeNotification", "implemented"),
        ]
    );
    let games = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../games");
    let mut checked = 0;
    for title in ["Fortnite", "XDefiant"] {
        let p = games.join(title).join("extracted/EmbeddedXvd/Windows/System32").join(DLL_FILE);
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let mut console = xone_exports::pe_exports(&bytes).expect("console export table");
        console.sort();
        let mine: Vec<(u16, String)> = rows.iter().map(|r| (r.ordinal, r.name.clone())).collect();
        assert_eq!(console, mine, "{}", p.display());
        checked += 1;
    }
    eprintln!("console images compared: {checked}");
}

#[test]
fn registration_ids_are_shared_across_kinds_and_null_out_is_rejected() {
    let _g = setup();
    let (mut a, mut b) = (0u64, 0u64);
    unsafe {
        assert_eq!(mam_RegisterAppStateChangeNotification(cb as usize, 1, &mut a), 0);
        assert_eq!(mam_RegisterAppConstrainedChangeNotification(cb as usize, 2, &mut b), 0);
        assert!(a >= 1);
        assert_eq!(b, a + 1);
        assert_eq!(mam_RegisterAppStateChangeNotification(cb as usize, 3, core::ptr::null_mut()), 87);
        assert_eq!(mam_UnregisterAppStateChangeNotification(b), 0x490);
        assert_eq!(mam_UnregisterAppConstrainedChangeNotification(a), 0x490);
        assert_eq!(mam_UnregisterAppStateChangeNotification(a), 0);
        assert_eq!(mam_UnregisterAppConstrainedChangeNotification(b), 0);
        assert_eq!(mam_UnregisterAppStateChangeNotification(a), 0x490);
    }
    assert_eq!(plm::registered(plm::Kind::AppState), 0);
    assert_eq!(plm::registered(plm::Kind::Constrained), 0);
}

#[test]
fn suspend_resume_and_constrained_reach_registered_callbacks() {
    let _g = setup();
    let me = unsafe { GetCurrentThreadId() };
    let (mut s, mut c) = (0u64, 0u64);
    unsafe {
        assert_eq!(mam_RegisterAppStateChangeNotification(cb as usize, 0xa11, &mut s), 0);
        assert_eq!(mam_RegisterAppConstrainedChangeNotification(cb as usize, 0xc22, &mut c), 0);
    }
    assert_eq!(plm::suspend(), 1);
    assert_eq!(plm::resume(), 1);
    assert_eq!(plm::set_constrained(true), 1);
    assert_eq!(plm::set_constrained(false), 1);
    let log = take();
    let seen: Vec<(u8, usize)> = log.iter().map(|e| (e.0, e.1)).collect();
    assert_eq!(seen, vec![(1, 0xa11), (0, 0xa11), (1, 0xc22), (0, 0xc22)]);
    assert!(log.iter().all(|e| e.2 != me), "callbacks must run on the notification thread, not the signaller");
    unsafe {
        assert_eq!(mam_UnregisterAppStateChangeNotification(s), 0);
        assert_eq!(mam_UnregisterAppConstrainedChangeNotification(c), 0);
    }
    assert_eq!(plm::suspend(), 0);
    assert_eq!(plm::set_constrained(true), 0);
    assert!(take().is_empty());
}

static PAR: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn slow(_value: u8, _ctx: *mut core::ffi::c_void) {
    let n = PAR.fetch_add(1, Ordering::SeqCst) + 1;
    PEAK.fetch_max(n, Ordering::SeqCst);
    std::thread::sleep(std::time::Duration::from_millis(150));
    PAR.fetch_sub(1, Ordering::SeqCst);
}

#[test]
fn several_registrations_are_delivered_in_parallel_and_waited_for() {
    let _g = setup();
    PEAK.store(0, Ordering::SeqCst);
    let mut ids = [0u64; 3];
    for id in ids.iter_mut() {
        assert_eq!(unsafe { mam_RegisterAppStateChangeNotification(slow as usize, 0, id) }, 0);
    }
    assert_eq!(plm::suspend(), 3);
    assert_eq!(PAR.load(Ordering::SeqCst), 0);
    assert!(PEAK.load(Ordering::SeqCst) >= 2, "fan-out did not overlap callbacks");
    for id in ids {
        assert_eq!(unsafe { mam_UnregisterAppStateChangeNotification(id) }, 0);
    }
}
