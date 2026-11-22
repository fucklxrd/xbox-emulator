#[path = "../../compat-common/tests/support/built_dlls.rs"]
mod built_dlls;

use core::ffi::c_void;
use std::path::{Path, PathBuf};

type H = *mut c_void;

extern "system" {
    fn LoadLibraryW(name: *const u16) -> H;
    fn GetProcAddress(h: H, name: *const u8) -> *mut c_void;
    fn GetModuleHandleW(name: *const u16) -> H;
}

const MF_VERSION: u32 = 0x0002_0070;
const CASE: &str = "TOMBOX_TEST_MFPLAT_EDGE";

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn get<T: Copy>(m: H, name: &str) -> T {
    let c = format!("{name}\0");
    let p = unsafe { GetProcAddress(m, c.as_ptr()) };
    assert!(!p.is_null(), "{name}");
    unsafe { core::mem::transmute_copy(&p) }
}

fn built_mfplat(dir: &Path) -> PathBuf {
    let mut c: Vec<PathBuf> = [dir.join("mfplat.dll"), dir.join("deps").join("mfplat.dll")].into_iter().filter(|p| p.is_file()).collect();
    c.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    c.pop().expect("built mfplat.dll")
}

struct Api {
    startup: unsafe extern "system" fn(u32, u32) -> i32,
    shutdown: unsafe extern "system" fn() -> i32,
    platform: unsafe extern "system" fn() -> *mut u8,
    dir: PathBuf,
}

fn stage(with_xmem: bool) -> Api {
    assert!(unsafe { GetModuleHandleW(w("mfplat.dll").as_ptr()) }.is_null());
    let profile = built_dlls::ensure(&[("xmem", "xmem.dll"), ("mfplat", "mfplat.dll")]);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("mfplat_edge_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(built_mfplat(&profile), dir.join("mfplat.dll")).unwrap();
    if with_xmem {
        std::fs::copy(profile.join("xmem.dll"), dir.join("xmem.dll")).unwrap();
    }
    let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    std::fs::copy(Path::new(&sysroot).join("System32").join("mfplat.dll"), dir.join("mfplat_sys.dll")).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    std::env::remove_var("TOMBOX_MFPLAT_CONSOLE");
    let proxy = unsafe { LoadLibraryW(w(&dir.join("mfplat.dll").to_string_lossy()).as_ptr()) };
    assert!(!proxy.is_null());
    Api { startup: get(proxy, "MFStartup"), shutdown: get(proxy, "MFShutdown"), platform: get(proxy, "MFGetPlatform"), dir }
}

fn count(p: *mut u8) -> i32 {
    unsafe { (p.add(0x4300) as *const i32).read_volatile() }
}

fn run(case: &str) {
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(&exe).args(["--exact", "inner_edge", "--nocapture", "--test-threads=1"]).env(CASE, case).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && text.contains("EDGE OK"), "case {case}:\n{text}");
}

#[test]
fn unbalanced_shutdowns_do_not_drift_the_console_count() {
    run("unbalanced");
}

#[test]
fn concurrent_startup_shutdown_pairs_keep_the_platform_alive() {
    run("threads");
}

#[test]
fn missing_xmem_reports_null_without_failing_startup() {
    run("noxmem");
}

#[test]
fn inner_edge() {
    let Ok(case) = std::env::var(CASE) else { return };
    match case.as_str() {
        "unbalanced" => {
            let a = stage(true);
            assert_eq!(unsafe { (a.shutdown)() }, 0, "MFShutdown with no MFStartup");
            assert!(unsafe { (a.platform)() }.is_null());
            assert_eq!(unsafe { (a.startup)(MF_VERSION, 0) }, 0);
            let p = unsafe { (a.platform)() };
            assert!(!p.is_null());
            assert_eq!(count(p), 1);
            for _ in 0..3 {
                assert_eq!(unsafe { (a.shutdown)() }, 0);
                assert!(unsafe { (a.platform)() }.is_null());
                assert_eq!(count(p), 0);
            }
            for round in 0..5 {
                assert_eq!(unsafe { (a.startup)(MF_VERSION, 0) }, 0, "restart {round}");
                assert_eq!(unsafe { (a.platform)() }, p, "same static object after restart {round}");
                assert_eq!(count(p), 1, "count after restart {round}");
                assert_eq!(unsafe { (a.shutdown)() }, 0);
                assert!(unsafe { (a.platform)() }.is_null());
            }
            assert_eq!(unsafe { (a.startup)(0x0001_0070, 0) }, 0, "older MF_VERSION accepted by both");
            assert_eq!(unsafe { (a.platform)() }, p);
            assert_eq!(unsafe { (a.shutdown)() }, 0);
            assert!(unsafe { (a.platform)() }.is_null());
        }
        "threads" => {
            let a = stage(true);
            assert_eq!(unsafe { (a.startup)(MF_VERSION, 0) }, 0);
            let p = unsafe { (a.platform)() } as usize;
            assert_ne!(p, 0);
            let (st, sd, pl) = (a.startup as usize, a.shutdown as usize, a.platform as usize);
            let hs: Vec<_> = (0..8)
                .map(|_| {
                    std::thread::spawn(move || {
                        let st: unsafe extern "system" fn(u32, u32) -> i32 = unsafe { core::mem::transmute(st) };
                        let sd: unsafe extern "system" fn() -> i32 = unsafe { core::mem::transmute(sd) };
                        let pl: unsafe extern "system" fn() -> *mut u8 = unsafe { core::mem::transmute(pl) };
                        for _ in 0..40 {
                            assert_eq!(unsafe { st(MF_VERSION, 0) }, 0);
                            assert_eq!(unsafe { pl() } as usize, p);
                            assert_eq!(unsafe { sd() }, 0);
                            assert_eq!(unsafe { pl() } as usize, p, "the main thread's startup keeps it alive");
                        }
                    })
                })
                .collect();
            for h in hs {
                h.join().unwrap();
            }
            assert_eq!(count(p as *mut u8), 1, "console count back to the one outstanding startup");
            assert_eq!(unsafe { (a.shutdown)() }, 0);
            assert!(unsafe { (a.platform)() }.is_null());
            assert_eq!(count(p as *mut u8), 0);
        }
        "noxmem" => {
            let a = stage(false);
            assert!(unsafe { GetModuleHandleW(w("xmem.dll").as_ptr()) }.is_null());
            assert_eq!(unsafe { (a.startup)(MF_VERSION, 0) }, 0, "the PC media path still starts");
            assert!(unsafe { (a.platform)() }.is_null(), "no console platform without xmem");
            assert_eq!(unsafe { (a.shutdown)() }, 0);
            assert!(unsafe { (a.platform)() }.is_null());
            let log = std::fs::read_to_string(a.dir.join("tombox_compat.log")).unwrap_or_default();
            assert!(log.contains("xmem.dll"), "{log}");
        }
        _ => panic!("{case}"),
    }
    println!("EDGE OK {case}");
}
