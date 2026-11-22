use core::ffi::c_void;
use XAudio2_9::ffi::*;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
}

#[link(name = "ole32")]
extern "system" {
    fn CoTaskMemFree(p: *mut c_void);
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn load_gcxaudioext() {
    let exe = std::env::current_exe().unwrap();
    let in_build_dir = |build: std::path::PathBuf| {
        std::fs::read_dir(build.join("gcxaudioext")).ok()?.flatten().map(|e| e.path().join("out").join("gcxaudioext.dll")).filter(|p| p.is_file()).max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
    };
    let dll = exe
        .ancestors()
        .find_map(|d| [d.join("gcxaudioext.dll"), d.join("deps").join("gcxaudioext.dll"), d.join("debug").join("gcxaudioext.dll")].into_iter().find(|p| p.is_file()).or_else(|| in_build_dir(d.join("build"))))
        .unwrap_or_else(|| panic!("gcxaudioext.dll (an XAudio2_9 dev-dependency) is not in the target dir above {}", exe.display()));
    assert!(!unsafe { LoadLibraryW(wide(dll.to_str().unwrap()).as_ptr()) }.is_null());
}

unsafe fn slot(o: *mut c_void, i: usize) -> usize {
    *(*(o as *const *const usize)).add(i)
}

#[test]
fn create_fx_routes_console_platform_clsids_to_gcxaudioext() {
    load_gcxaudioext();
    unsafe {
        let mut fx: *mut c_void = core::ptr::null_mut();
        assert_eq!(XAudio2_9::CreateFX(&CLSID_CONSOLE_LOUDNESS_METER, &mut fx, core::ptr::null(), 0), 0);
        assert!(!fx.is_null());
        let get: unsafe extern "system" fn(*mut c_void, *mut *mut u8) -> i32 = core::mem::transmute(slot(fx, 3));
        let mut reg: *mut u8 = core::ptr::null_mut();
        assert_eq!(get(fx, &mut reg), 0);
        let clsid = core::ptr::read_unaligned(reg as *const GUID);
        assert_eq!(clsid, CLSID_CONSOLE_LOUDNESS_METER);
        CoTaskMemFree(reg as *mut c_void);
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(slot(fx, 2));
        assert_eq!(release(fx), 0);
        let unknown = GUID { data1: 0x1234_5678, data2: 1, data3: 2, data4: [3; 8] };
        let mut none = 0x55usize as *mut c_void;
        assert_eq!(XAudio2_9::CreateFX(&unknown, &mut none, core::ptr::null(), 0), 0x8007_0057u32 as i32);
        assert!(none.is_null());
        assert!(XAudio2_9::CONSOLE_XAPOFX_CLSIDS.iter().all(|c| *c != CLSID_CONSOLE_LOUDNESS_METER));
    }
}
