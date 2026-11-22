use core::ffi::c_void;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
}

type Ws = unsafe extern "system" fn(u32, *mut u8) -> i32;

fn ws() -> Ws {
    let exe = std::env::current_exe().unwrap();
    let p = exe.ancestors().skip(1).flat_map(|a| [a.join("xmem.dll"), a.join("debug").join("xmem.dll")]).find(|p| p.is_file()).expect("build xmem first");
    let w: Vec<u16> = p.to_str().unwrap().encode_utf16().chain(Some(0)).collect();
    let h = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!h.is_null());
    unsafe { core::mem::transmute(GetProcAddress(h, b"XMemGetWorkingSetStatistics\0".as_ptr())) }
}

fn d(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn near(a: u32, b: u32) {
    assert!(a.abs_diff(b) <= 0x100_0000, "{a:#x} vs {b:#x}: live pool bytes drifted too far");
}

#[test]
fn v2_copies_internal_pool_and_commit_limit_like_console_2a50() {
    let f = ws();
    let mut int = [0u8; 0x78];
    let mut v2 = [0xccu8; 0x50];
    unsafe {
        assert_eq!(f(0x2000_0009, int.as_mut_ptr()), 1);
        assert_eq!(f(0x1000_0009, v2.as_mut_ptr()), 1);
    }
    assert_ne!(d(&int, 0x48), 0);
    near(d(&v2, 0x48), d(&int, 0x48));
    assert_eq!(d(&v2, 0x4c), d(&int, 0x4c));
}

#[test]
fn legacy_puts_internal_pool_bytes_at_0x24_like_console_2b44() {
    let f = ws();
    let mut int = [0u8; 0x78];
    let mut old = [0xccu8; 0x48];
    unsafe {
        assert_eq!(f(0x2000_0009, int.as_mut_ptr()), 1);
        assert_eq!(f(0x0000_0009, old.as_mut_ptr()), 1);
    }
    assert_ne!(d(&int, 0x48), 0);
    near(d(&old, 0x24), d(&int, 0x48));
}
