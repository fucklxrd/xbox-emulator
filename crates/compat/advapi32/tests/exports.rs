mod common;

use common::*;
use core::ffi::c_void;

fn dlls() -> (*mut c_void, *mut c_void) {
    let legacy = load_built("advapi32legacy.dll", "advapi32legacy.dll");
    (load_built("advapi32.dll", "tbx_advapi32.dll"), legacy)
}

#[test]
fn export_table_matches_exports_txt() {
    check_table("advapi32.dll");
}

#[test]
fn every_export_forwards_to_sechost_or_the_emulator_advapi32legacy() {
    let (h, legacy) = dlls();
    let n = check_forwarders(h, &[("advapi32legacy.dll", legacy)]);
    assert_eq!(n, rows().iter().filter(|e| e.status != "missing").count());
}

#[test]
fn elf_exports_reach_the_advapi32legacy_implementation() {
    let (h, _) = dlls();
    unsafe {
        let flush: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(proc(h, "ElfFlushEventLog"));
        assert_eq!(flush(core::ptr::null_mut()), 0xC000_0022u32 as i32);
        let dereg: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(proc(h, "ElfDeregisterEventSource"));
        assert_eq!(dereg(core::ptr::null_mut()), 0xC000_0008u32 as i32);
    }
}

#[test]
fn lsa_password_less_forwarders_reach_the_advapi32legacy_implementation() {
    let root = std::env::temp_dir().join(format!("tombox-a32-pwdless-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::env::set_var("TOMBOX_GDK_ROOT", &root);
    let value = root.join("hklm").join("SOFTWARE").join("Microsoft").join("Windows NT").join("CurrentVersion").join("PasswordLess").join("Device").join("DevicePasswordLessBuildVersion.dword");
    let (h, legacy) = dlls();
    unsafe {
        assert_eq!(proc(h, "LsaEnablePasswordLessCurrentUser"), proc(legacy, "LsaEnablePasswordLessCurrentUser"));
        assert_eq!(proc(h, "LsaDisablePasswordLessCurrentUser"), proc(legacy, "LsaDisablePasswordLessCurrentUser"));
        let enable: unsafe extern "system" fn() -> i32 = core::mem::transmute(proc(h, "LsaEnablePasswordLessCurrentUser"));
        let disable: unsafe extern "system" fn() -> i32 = core::mem::transmute(proc(h, "LsaDisablePasswordLessCurrentUser"));
        assert_eq!(enable(), 0);
        assert_eq!(std::fs::read_to_string(&value).unwrap().trim(), "2");
        assert_eq!(disable(), 0);
        assert_eq!(std::fs::read_to_string(&value).unwrap().trim(), "0");
    }
    let _ = std::fs::remove_dir_all(&root);
}
