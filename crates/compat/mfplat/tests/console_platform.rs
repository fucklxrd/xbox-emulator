#[path = "../../compat-common/tests/support/built_dlls.rs"]
mod built_dlls;

use core::ffi::c_void;
use std::path::{Path, PathBuf};

type H = *mut c_void;

extern "system" {
    fn LoadLibraryW(name: *const u16) -> H;
    fn GetProcAddress(h: H, name: *const u8) -> *mut c_void;
    fn GetModuleHandleW(name: *const u16) -> H;
    fn GetModuleHandleExW(flags: u32, addr: *const u16, out: *mut H) -> i32;
}

const MF_VERSION: u32 = 0x0002_0070;
const MF_E_BAD_STARTUP_VERSION: i32 = 0xC00D_36E3u32 as i32;
const CASE: &str = "TOMBOX_TEST_MFPLAT_CASE";

fn consoles() -> Vec<String> {
    console_image::dump_dirs(Path::new(console_image::GAMES)).into_iter().map(|d| d.join("mfplat.dll")).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()).collect()
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn wp(p: &Path) -> Vec<u16> {
    w(&p.to_string_lossy())
}

fn built_mfplat(dir: &Path) -> PathBuf {
    let mut c: Vec<PathBuf> = [dir.join("mfplat.dll"), dir.join("deps").join("mfplat.dll")].into_iter().filter(|p| p.is_file()).collect();
    c.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    c.pop().expect("built mfplat.dll")
}

fn module_of(addr: usize) -> usize {
    let mut h: H = core::ptr::null_mut();
    if unsafe { GetModuleHandleExW(6, addr as *const u16, &mut h) } == 0 {
        0
    } else {
        h as usize
    }
}

fn get<T: Copy>(m: H, name: &str) -> T {
    let c = format!("{name}\0");
    let p = unsafe { GetProcAddress(m, c.as_ptr()) };
    assert!(!p.is_null(), "{name}");
    unsafe { core::mem::transmute_copy(&p) }
}

unsafe fn mfplat_iat_targets(module: usize) -> Vec<(String, usize)> {
    let base = module as *const u8;
    let rd = |o: usize| (base.add(o) as *const u32).read_unaligned() as usize;
    let cstr = |o: usize| {
        let mut v = Vec::new();
        while *base.add(o + v.len()) != 0 {
            v.push(*base.add(o + v.len()));
        }
        String::from_utf8_lossy(&v).into_owned()
    };
    let pe = rd(0x3c);
    let mut out = Vec::new();
    for (dir, step, name_at, int_at, iat_at) in [(pe + 24 + 112 + 8, 20usize, 12usize, 0usize, 16usize), (pe + 24 + 112 + 13 * 8, 32, 4, 16, 12)] {
        let mut d = rd(dir);
        if d == 0 {
            continue;
        }
        while rd(d + name_at) != 0 {
            if cstr(rd(d + name_at)).eq_ignore_ascii_case("mfplat.dll") {
                let (int, iat) = (rd(d + int_at), rd(d + iat_at));
                let mut i = 0;
                loop {
                    let t = (base.add(int + i * 8) as *const u64).read_unaligned();
                    if t == 0 {
                        break;
                    }
                    let n = if t >> 63 == 1 { format!("#{}", t & 0xffff) } else { cstr(t as usize + 2) };
                    let v = (base.add(iat + i * 8) as *const usize).read_unaligned();
                    if module_of(v) != module {
                        out.push((n, v));
                    }
                    i += 1;
                }
            }
            d += step;
        }
    }
    out
}

struct Oracle {
    startup: unsafe extern "system" fn(u32, u32) -> i32,
    shutdown: unsafe extern "system" fn() -> i32,
    platform: unsafe extern "system" fn() -> *mut u8,
}

#[test]
fn mfgetplatform_matches_every_console_dump_and_the_default_pick() {
    let mut cases = consoles();
    if cases.is_empty() {
        eprintln!("no console mfplat.dll in the title dumps, oracle skipped");
        return;
    }
    cases.push("auto".into());
    let exe = std::env::current_exe().unwrap();
    for c in cases {
        let out = std::process::Command::new(&exe).args(["--exact", "inner_oracle", "--nocapture", "--test-threads=1"]).env(CASE, &c).output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && text.contains("ORACLE OK"), "case {c}:\n{text}");
    }
}

#[test]
fn inner_oracle() {
    let Ok(case) = std::env::var(CASE) else { return };
    assert!(unsafe { GetModuleHandleW(w("mfplat.dll").as_ptr()) }.is_null());
    let profile = built_dlls::ensure(&[("xmem", "xmem.dll"), ("mfplat", "mfplat.dll")]);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("mfplat_platform_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(built_mfplat(&profile), dir.join("mfplat.dll")).unwrap();
    std::fs::copy(profile.join("xmem.dll"), dir.join("xmem.dll")).unwrap();
    let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    std::fs::copy(Path::new(&sysroot).join("System32").join("mfplat.dll"), dir.join("mfplat_sys.dll")).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    if case == "auto" {
        std::env::remove_var("TOMBOX_MFPLAT_CONSOLE");
    } else {
        std::env::set_var("TOMBOX_MFPLAT_CONSOLE", &case);
    }
    let proxy = unsafe { LoadLibraryW(wp(&dir.join("mfplat.dll")).as_ptr()) };
    assert!(!proxy.is_null());
    let startup: unsafe extern "system" fn(u32, u32) -> i32 = get(proxy, "MFStartup");
    let shutdown: unsafe extern "system" fn() -> i32 = get(proxy, "MFShutdown");
    let platform: unsafe extern "system" fn() -> *mut u8 = get(proxy, "MFGetPlatform");

    assert!(unsafe { platform() }.is_null(), "NULL before the first MFStartup, as the console with a zero start count");
    assert_eq!(unsafe { startup(0x0003_0070, 0) }, MF_E_BAD_STARTUP_VERSION);
    assert!(unsafe { platform() }.is_null(), "a rejected MFStartup does not start the platform");

    assert_eq!(unsafe { startup(MF_VERSION, 0) }, 0);
    let rw = unsafe { LoadLibraryW(w("mfreadwrite.dll").as_ptr()) };
    assert!(!rw.is_null());
    let p = unsafe { platform() };
    assert!(!p.is_null(), "started: MFGetPlatform returns the platform object");
    let side = module_of(p as usize);
    assert_ne!(side, 0);
    assert_ne!(side, proxy as usize, "the object lives in the console mfplat image, not in the proxy");
    let oracle = Oracle { startup: get(side as H, "MFStartup"), shutdown: get(side as H, "MFShutdown"), platform: get(side as H, "MFGetPlatform") };
    assert_eq!(unsafe { (oracle.platform)() }, p, "the console export itself returns the same object");
    let started = |p: *mut u8| unsafe { (p.add(0x4300) as *const i32).read_volatile() };
    assert_eq!(started(p), 1, "console start count at CMFPlatform+0x4300");
    let side_path = console_sandbox::modules::module_file(side).unwrap();
    if case != "auto" {
        assert!(side_path.to_string_lossy().eq_ignore_ascii_case(&case), "{} != {case}", side_path.display());
    }
    let body = get::<usize>(side as H, "MFGetPlatform");
    let code = unsafe { core::slice::from_raw_parts(body as *const u8, 16) };
    let rel = |at: usize, end: usize| (body as isize + end as isize + i32::from_le_bytes(code[at..at + 4].try_into().unwrap()) as isize) as usize;
    let (count, target) = match (&code[0..2], &code[6..9], &code[9..12]) {
        ([0x83, 0x3d], _, [0x48, 0x8d, 0x05]) => (rel(2, 7), rel(12, 16)),
        ([0x8b, 0x05], [0x48, 0x8d, 0x0d], _) => (rel(2, 6), rel(9, 13)),
        _ => panic!("unrecognised console MFGetPlatform body {code:02x?}"),
    };
    assert_eq!(p as usize, target, "the returned pointer is the static the console code loads (rva {:#x})", p as usize - side);
    assert_eq!(count, p as usize + 0x4300, "the start count it tests is CMFPlatform+0x4300");
    assert_eq!(unsafe { GetModuleHandleW(w("mfplat.dll").as_ptr()) }, proxy, "base name mfplat.dll still resolves to the proxy with the sidecar loaded");
    let kb = unsafe { LoadLibraryW(w("kernelbase.dll").as_ptr()) };
    let resolve: unsafe extern "system" fn(H, *const u8, u32) -> i32 = get(kb, "ResolveDelayLoadsFromDll");
    assert_eq!(unsafe { resolve(rw, b"MFPlat.DLL\0".as_ptr(), 0) }, 0, "ResolveDelayLoadsFromDll(mfreadwrite, MFPlat.DLL)");
    let sys = unsafe { GetModuleHandleW(wp(&dir.join("mfplat_sys.dll")).as_ptr()) } as usize;
    let bound = unsafe { mfplat_iat_targets(rw as usize) };
    assert!(!bound.is_empty(), "mfreadwrite imports mfplat");
    for (name, t) in &bound {
        let m = module_of(*t);
        assert!(m != side && m != 0, "mfreadwrite!{name} bound to {m:#x}: the sidecar {side:#x} must never serve the title's media path (proxy {:#x}, mfplat_sys {sys:#x})", proxy as usize);
    }
    let routed = unsafe { console_sandbox::unrouted(side) };
    for bad in ["RegQueryValueExW", "RegEnumKeyExW", "RegDeleteValueW", "RegDeleteKeyW", "RegEnumKeyW", "RegSetValueExW", "RegCreateKeyExW", "RegOpenKeyExW", "CreateFileW", "DeleteFileW"] {
        assert!(!routed.iter().any(|u| u.ends_with(&format!("!{bad}"))), "{bad} reaches the PC: {routed:?}");
    }
    assert_eq!(unsafe { (p.add(0x42b4) as *const u32).read_volatile() } & 1, 0, "the shutdown bit (+0x42b4 & 1) the console checks is clear while started");

    assert_eq!(unsafe { startup(MF_VERSION, 0) }, 0);
    assert_eq!(unsafe { platform() }, p);
    assert_eq!(started(p), 2, "nested MFStartup bumps the console count");
    assert_eq!(unsafe { shutdown() }, 0);
    assert_eq!(unsafe { platform() }, p, "still started after one of two shutdowns");
    assert_eq!(started(p), 1);
    assert_eq!(unsafe { shutdown() }, 0);
    assert!(unsafe { platform() }.is_null(), "NULL after the last MFShutdown");
    assert!(unsafe { (oracle.platform)() }.is_null());

    assert_eq!(unsafe { (oracle.startup)(MF_VERSION, 0) }, 0);
    assert_eq!(unsafe { platform() }, p, "MFGetPlatform reports the console object's own state");
    assert_eq!(unsafe { (oracle.shutdown)() }, 0);
    assert!(unsafe { platform() }.is_null());

    let log = std::fs::read_to_string(dir.join("tombox_compat.log")).unwrap_or_default();
    assert!(log.contains("platform sidecar"), "{log}");
    println!("ORACLE OK {}", side_path.display());
}
