use super::console::*;
use super::store::{self, Store};
use super::tlg::{self, Guid};
use super::*;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

const TM_WLDP: &str = r"D:\xbox-pc-port\games\Trackmania\extracted\Ubisoft.TrackmaniaNext_1.6.2.0_neutral__ngz4m417e0mpw\EmbeddedXvd\Windows\System32\wldp.dll";

pub fn env_lock() -> MutexGuard<'static, ()> {
    static M: Mutex<()> = Mutex::new(());
    M.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn temp_root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tombox_wldp_test_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn fresh(tag: &str) -> (Store, PathBuf) {
    let d = temp_root(tag);
    (Store::at(&d), d)
}

#[test]
fn store_defaults_are_retail_console() {
    let (s, d) = fresh("defaults");
    assert!(d.join("state.txt").is_file());
    assert_eq!(s.ci_policy_options(), store::RETAIL_CI_POLICY_OPTIONS);
    assert_eq!(lockdown_mode(s.ci_policy_options()), 2);
    assert!(!s.state_separation());
    assert!(!s.manufacturing_mode());
    assert_eq!(s.blocklist_enable(), None);
    assert_eq!(s.developer_unlock(), None);
    assert!(!s.file_exists(store::DPP_HSTI));
    assert!(!s.file_exists(store::DPP_SB));
    assert!(s.console_path("%SystemDrive%\\DPP").is_dir());
    assert!(s.console_path(store::CI_DATA).is_dir());
    assert_eq!(s.console_path(store::DPP_SB), d.join("SystemDrive").join("DPP").join("WatermarkSBTestComplete.bin"));
    assert_eq!(s.console_path("%SystemRoot%\\System32\\CodeIntegrity\\Data\\x.bin"), d.join("SystemDrive").join("Windows").join("System32").join("CodeIntegrity").join("Data").join("x.bin"));
    s.set("manufacturing_mode", Some("1")).unwrap();
    assert!(Store::at(&d).manufacturing_mode());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn lockdown_mode_matches_console_query() {
    assert_eq!(lockdown_mode(0), 0);
    assert_eq!(lockdown_mode(0x8000_0000), 1);
    assert_eq!(lockdown_mode(0x0400_0000), 2);
    assert_eq!(lockdown_mode(0x2000_0000 | 0x8000_0000), 2);
    assert_eq!(lockdown_mode(0x1234), 0);
}

#[test]
fn wldp_change_vulnerable_driver_blocklist_state() {
    let (s, d) = fresh("blocklist");
    let mut dis = 9;
    assert_eq!(is_blocklist_disabled_in(&s, &mut dis), ERROR_FILE_NOT_FOUND_HR);
    assert_eq!(dis, 0);
    assert_eq!(change_blocklist_in(&s, 0, 0), 0x8007_0032u32 as i32);
    assert_eq!(s.blocklist_enable(), None);
    assert_eq!(change_blocklist_in(&s, 0, 1), 0);
    assert_eq!(s.blocklist_enable(), Some(0));
    assert_eq!(is_blocklist_disabled_in(&s, &mut dis), 0);
    assert_eq!(dis, 1);
    tlg::state().enable(1, 0, u64::MAX, 0);
    assert_eq!(change_blocklist_in(&s, 1, 1), 0);
    tlg::state().enable(0, 0, 0, 0);
    assert_eq!(s.blocklist_enable(), Some(1));
    assert_eq!(is_blocklist_disabled_in(&s, &mut dis), 0);
    assert_eq!(dis, 0);
    assert_eq!(Store::at(&d).blocklist_enable(), Some(1));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn wldp_change_vulnerable_driver_blocklist_state_export() {
    let _g = env_lock();
    let d = temp_root("export19");
    std::env::set_var("TOMBOX_GDK_ROOT", &d);
    let mut e = 0;
    unsafe { WldpIsVulnerableDriverBlocklistEligibleToDisable(&mut e) };
    let hr = unsafe { WldpChangeVulnerableDriverBlocklistState(0) };
    let s = Store::open();
    if e == 0 {
        assert_eq!(hr, 0x8007_0032u32 as i32);
        assert_eq!(s.blocklist_enable(), None);
    } else {
        assert_eq!(hr, 0);
        assert_eq!(s.blocklist_enable(), Some(0));
        let mut dis = 0;
        assert_eq!(unsafe { WldpIsVulnerableDriverBlocklistDisabled(&mut dis) }, 0);
        assert_eq!(dis, 1);
    }
    std::env::remove_var("TOMBOX_GDK_ROOT");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn wldp_enable_and_disable_developer_mode() {
    let (s, d) = fresh("devmode");
    assert_eq!(developer_gate(&s), 1);
    assert_eq!(disable_developer_mode_in(&s), 0);
    assert_eq!(enable_developer_mode_in(&s), 0);
    assert_eq!(s.developer_unlock().as_deref(), Some(DEVELOPER_UNLOCK_SDDL));
    assert_eq!(enable_developer_mode_in(&s), 0);
    assert_eq!(disable_developer_mode_in(&s), 0);
    assert_eq!(s.developer_unlock(), None);
    assert_eq!(disable_developer_mode_in(&s), 0);
    s.set("ci_policy_options", Some("0")).unwrap();
    assert_eq!(developer_gate(&s), 0);
    assert_eq!(enable_developer_mode_in(&s), 0);
    assert_eq!(s.developer_unlock(), None);
    s.set("state_separation", Some("1")).unwrap();
    assert_eq!(developer_gate(&s), 1);
    assert_eq!(enable_developer_mode_in(&s), 0);
    assert!(s.developer_unlock().is_some());
    s.set("state_separation", Some("0")).unwrap();
    assert_eq!(disable_developer_mode_in(&s), 0);
    assert!(s.developer_unlock().is_some());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn wldp_developer_mode_exports() {
    let _g = env_lock();
    let d = temp_root("export2324");
    std::env::set_var("TOMBOX_GDK_ROOT", &d);
    unsafe {
        assert_eq!(WldpEnableDeveloperMode(), 0);
        assert!(Store::open().developer_unlock().is_some());
        assert_eq!(WldpDisableDeveloperMode(), 0);
        assert!(Store::open().developer_unlock().is_none());
    }
    std::env::remove_var("TOMBOX_GDK_ROOT");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn wldp_reset_security_watermark_state() {
    let (s, d) = fresh("reset");
    assert_eq!(reset_security_watermark_in(&s), 0);
    assert_eq!(s.write_file(store::DPP_HSTI, &[1u8; 32]), 0);
    assert_eq!(s.write_file(store::DPP_SB, &[2u8; 32]), 0);
    let mut b = [0u8; 32];
    assert_eq!(s.read_file(store::DPP_SB, &mut b), 0);
    assert_eq!(b, [2u8; 32]);
    let mut big = [0u8; 33];
    assert_eq!(s.read_file(store::DPP_SB, &mut big), 1);
    assert_eq!(reset_security_watermark_in(&s), 0);
    assert!(!s.file_exists(store::DPP_HSTI));
    assert!(!s.file_exists(store::DPP_SB));
    assert_eq!(s.read_file(store::DPP_SB, &mut b), ERROR_FILE_NOT_FOUND_HR);
    std::fs::remove_dir_all(s.console_path("%SystemDrive%\\DPP")).unwrap();
    assert_eq!(reset_security_watermark_in(&s), 0x8007_0003u32 as i32);
    let _ = std::fs::remove_dir_all(&d);
    let _g = env_lock();
    let d = temp_root("export40");
    std::env::set_var("TOMBOX_GDK_ROOT", &d);
    let s = Store::open();
    assert_eq!(s.write_file(store::DPP_HSTI, &[3u8; 32]), 0);
    assert_eq!(unsafe { WldpResetSecurityWatermarkState() }, 0);
    assert!(!s.file_exists(store::DPP_HSTI));
    std::env::remove_var("TOMBOX_GDK_ROOT");
    let _ = std::fs::remove_dir_all(&d);
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn tracelogging_metadata_matches_console_binary() {
    assert_eq!(tlg::ACTIVITY_META.len(), 71);
    assert_eq!(u16::from_le_bytes([tlg::ACTIVITY_META[0], tlg::ACTIVITY_META[1]]), 71);
    assert!(contains(tlg::ACTIVITY_META, b"Microsoft.Windows.Security.CodeIntegrity.Activity\0"));
    assert!(contains(tlg::STATE_META, b"Microsoft.Windows.Security.CodeIntegrity.State\0"));
    assert_eq!(tlg::Guid::from_le(tlg::ACTIVITY_GUID), Guid { data1: 0x28dcc28b, data2: 0x3e31, data3: 0x527b, data4: [0xef, 0xd6, 0xb4, 0xcc, 0x4d, 0x73, 0xd1, 0x57] });
    assert_eq!(&tlg::WDAC_EVENT_DESC[..3], &[0x0b, 5, 0]);
    assert_eq!(u64::from_le_bytes(tlg::WDAC_EVENT_DESC[3..11].try_into().unwrap()), 0x4000_0000_0000);
    assert!(contains(tlg::WDAC_EVENT_META, b"BlockDialogShown\0PartA_PrivTags\0\x0aFileName\0\x01CurrentProcess\0\x01hResult\0\x14DialogGuid\0\x0fUserActionsTaken\0\x01"));
    assert!(contains(tlg::BLOCKLIST_EVENT_META, b"VulnerableDriverBlocklistStateChanged\0State\0\x07"));
    if let Ok(dll) = std::fs::read(TM_WLDP) {
        for (g, m) in [(tlg::ACTIVITY_GUID, tlg::ACTIVITY_META), (tlg::STATE_GUID, tlg::STATE_META)] {
            let mut blob = vec![4u8];
            blob.extend_from_slice(g);
            blob.extend_from_slice(m);
            assert!(contains(&dll, &blob));
        }
        for (dsc, m) in [(tlg::WDAC_EVENT_DESC, tlg::WDAC_EVENT_META), (tlg::BLOCKLIST_EVENT_DESC, tlg::BLOCKLIST_EVENT_META)] {
            let mut blob = vec![6u8];
            blob.extend_from_slice(dsc);
            blob.extend_from_slice(m);
            assert!(contains(&dll, &blob));
        }
    }
}

#[test]
fn wldp_tracelogging_wdac_block_dialog_shown() {
    let p = tlg::activity();
    assert_eq!(p.register_status.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_ne!(p.handle.load(std::sync::atomic::Ordering::SeqCst), 0);
    let path = wide("C:\\Users\\someone\\AppData\\blocked.dll");
    let actions = wide("Dismissed");
    let g = Guid { data1: 1, data2: 2, data3: 3, data4: [4; 8] };
    unsafe {
        p.enable(0, 0, 0, 0);
        assert_eq!(wdac_block_dialog_shown(path.as_ptr(), 0x8007_10d1, &g, actions.as_ptr()), None);
        p.enable(1, 4, u64::MAX, 0);
        assert_eq!(wdac_block_dialog_shown(path.as_ptr(), 0x8007_10d1, &g, actions.as_ptr()), None);
        p.enable(1, 5, 0x8000_0000_0000, 0);
        assert_eq!(wdac_block_dialog_shown(path.as_ptr(), 0x8007_10d1, &g, actions.as_ptr()), None);
        p.enable(1, 5, 0x4000_0000_0000, 0);
        assert_eq!(wdac_block_dialog_shown(path.as_ptr(), 0x8007_10d1, &g, actions.as_ptr()), Some(0));
        p.enable(1, 0, u64::MAX, 0);
        assert_eq!(wdac_block_dialog_shown(core::ptr::null(), 0, &g, core::ptr::null()), Some(0));
        WldpTraceLoggingWDACBlockDialogShown(path.as_ptr(), 0, &g, actions.as_ptr());
        p.enable(0, 0, 0, 0);
    }
}

#[test]
fn userprofile_substitution_matches_console() {
    unsafe {
        let s = |x: &str| {
            let w = wide(x);
            let (hr, out) = userprofile_path(w.as_ptr());
            assert_eq!(hr, 0x1000_0000);
            out.map(|v| String::from_utf16(&v[..v.len() - 1]).unwrap())
        };
        assert_eq!(s("C:\\Users\\bob\\Desktop\\a.exe").as_deref(), Some("%USERPROFILE%\\Desktop\\a.exe"));
        assert_eq!(s("d:\\users\\Bob\\x").as_deref(), Some("%USERPROFILE%\\x"));
        assert_eq!(s("C:\\Users\\bob").as_deref(), None);
        assert_eq!(s("C:\\Windows\\System32\\a.dll").as_deref(), None);
        let (hr, out) = userprofile_path(core::ptr::null());
        assert_eq!((hr, out), (0x1000_0000, None));
    }
}

fn tm_system32() -> PathBuf {
    PathBuf::from(TM_WLDP).parent().unwrap().to_path_buf()
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect()
}

#[test]
fn wldp_send_smart_app_control_block_toast() {
    assert_eq!(console_activate_xml_document(CO_E_NOTINITIALIZED), CO_E_NOTINITIALIZED);
    assert_eq!(console_activate_xml_document(0), REGDB_E_CLASSNOTREG);
    assert_eq!(send_smart_app_control_block_toast_in(0), REGDB_E_CLASSNOTREG);
    assert_eq!(send_smart_app_control_block_toast_in(CO_E_NOTINITIALIZED), CO_E_NOTINITIALIZED);
    let dir = tm_system32();
    let Ok(software) = std::fs::read(dir.join("config").join("SOFTWARE")) else { return };
    let class = XML_DOCUMENT_CLASS;
    assert!(!software.windows(class.len()).any(|w| w == class.as_bytes()));
    let wc = utf16(class);
    assert!(!software.windows(wc.len()).any(|w| w == wc.as_slice()));
    assert!(software.windows(18).any(|w| w == b"ActivatableClassId"));
    assert!(!dir.join("msxml6.dll").exists());
    let dll = std::fs::read(TM_WLDP).unwrap();
    assert!(dll.windows(wc.len()).any(|w| w == wc.as_slice()));
}

#[test]
fn wldp_send_smart_app_control_block_toast_export() {
    let f = || unsafe { WldpSendSmartAppControlBlockToast(wide("C:\\x.exe").as_ptr(), core::ptr::null()) };
    let uninit = std::thread::spawn(move || (console_apartment_status(), f())).join().unwrap();
    if uninit.0 == CO_E_NOTINITIALIZED {
        assert_eq!(uninit.1, CO_E_NOTINITIALIZED);
    } else {
        assert_eq!(uninit.1, REGDB_E_CLASSNOTREG);
    }
    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(r: *mut core::ffi::c_void, flags: u32) -> i32;
        fn CoUninitialize();
    }
    for flags in [0u32, 2] {
        let r = std::thread::spawn(move || unsafe {
            let ok = CoInitializeEx(core::ptr::null_mut(), flags) >= 0;
            let hr = f();
            if ok {
                CoUninitialize();
            }
            hr
        })
        .join()
        .unwrap();
        assert_eq!(r, REGDB_E_CLASSNOTREG);
    }
}
