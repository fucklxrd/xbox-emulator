mod common;

use common::*;

type Get = unsafe extern "system" fn(*const u16, *mut [u32; 2]) -> i32;

fn get() -> Get {
    let h = load_built("kernelbase.dll", "tbx_kernelbase.dll");
    assert_eq!(check_forwarders(h, &[]), 0);
    unsafe { core::mem::transmute(proc(h, "GetPackageInstallTime")) }
}

#[test]
fn export_table_matches_exports_txt() {
    check_table("kernelbase.dll");
}

#[test]
fn family_xref_round_trips_through_get_package_install_time() {
    let get = get();
    let h = load_built("kernelbase.dll", "tbx_kernelbase.dll");
    type Add = unsafe extern "system" fn(*const core::ffi::c_void, *const u16, *const u16, u32, u64) -> i32;
    type Remove = unsafe extern "system" fn(*const core::ffi::c_void, *const u16) -> i32;
    let add: Add = unsafe { core::mem::transmute(proc(h, "AddPackageToFamilyXref")) };
    let remove: Remove = unsafe { core::mem::transmute(proc(h, "RemovePackageFromFamilyXref")) };
    let full = wide(&format!("TomBox.XrefDll{}_2.0.0.0_x64__8wekyb3d8bbwe", std::process::id()));
    let mut ft = [0u32; 2];
    unsafe {
        assert_eq!(add(core::ptr::null(), core::ptr::null(), core::ptr::null(), 0, 0), 0x8007_0057u32 as i32);
        assert_eq!(add(core::ptr::null(), full.as_ptr(), core::ptr::null(), 0, 0x0123_4567_89ab_cdef), 0);
        assert_eq!(get(full.as_ptr(), &mut ft), 0);
        assert_eq!(ft, [0x89ab_cdef, 0x0123_4567]);
        assert_eq!(remove(core::ptr::null(), full.as_ptr()), 0);
        assert_eq!(get(full.as_ptr(), &mut ft), 0x490);
        assert_eq!(remove(core::ptr::null(), full.as_ptr()), 0);
    }
}

#[test]
fn get_package_install_time_console_errors() {
    let f = get();
    let mut ft = [7u32, 7];
    unsafe {
        assert_eq!(f(core::ptr::null(), &mut ft), 0x57);
        assert_eq!(f(wide("A_1.0.0.0_x64__8wekyb3d8bbwe").as_ptr(), core::ptr::null_mut()), 0x57);
        assert_eq!(f(wide("TomBox.Unregistered_1.0.0.0_x64__8wekyb3d8bbwe").as_ptr(), &mut ft), 0x490);
        #[link(name = "kernel32")]
        extern "system" {
            fn PackageFamilyNameFromFullName(full: *const u16, len: *mut u32, family: *mut u16) -> i32;
        }
        let bad = wide("not a package name");
        let mut b = [0u16; 65];
        let mut n = 65u32;
        let pc = PackageFamilyNameFromFullName(bad.as_ptr(), &mut n, b.as_mut_ptr());
        assert_ne!(pc, 0);
        assert_eq!(f(bad.as_ptr(), &mut ft), pc);
    }
    assert_eq!(ft, [7, 7]);
}
