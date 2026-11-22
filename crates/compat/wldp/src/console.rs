use core::ffi::c_void;

use super::store::{self, Store};
use super::tlg::{self, EventData, Guid};
use super::{hresult_from_win32, wide, HResult, WldpIsVulnerableDriverBlocklistEligibleToDisable};

pub const DEVELOPER_UNLOCK_SDDL: &str = "O:BAG:BAD:(A;CIID;KA;;;SY)(A;CIID;KA;;;BA)(A;CIID;KR;;;WD)(A;CIID;KR;;;RC)(A;;KR;;;AC)(A;;KR;;;S-1-15-3-1024-1065365936-1281604716-3511738428-1654721687-432734479-3232135806-4053264122-3456934681)";
pub const ERROR_FILE_NOT_FOUND_HR: HResult = 0x8007_0002u32 as i32;
const KEYWORD_PRIVTAG_WDAC: u64 = 0x200_0000;
const KEYWORD_PRIVTAG_BLOCKLIST: u64 = 0x100_0000;

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[link(name = "ntdll")]
extern "system" {
    fn RtlInitUnicodeString(dst: *mut UnicodeString, src: *const u16);
    fn RtlFindUnicodeSubstring(full: *const UnicodeString, search: *const UnicodeString, case_insensitive: u8) -> *const u16;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn QueryFullProcessImageNameW(process: *mut c_void, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    fn GetLastError() -> u32;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl: *const u16, rev: u32, sd: *mut *mut c_void, size: *mut u32) -> i32;
}

pub fn lockdown_mode(options: u32) -> u32 {
    if options & 0x2400_0000 != 0 {
        2
    } else {
        options >> 31
    }
}

pub fn developer_gate(s: &Store) -> u32 {
    (lockdown_mode(s.ci_policy_options()) != 0 || s.state_separation()) as u32
}

pub fn is_blocklist_disabled_in(s: &Store, disabled: &mut i32) -> HResult {
    match s.blocklist_enable() {
        Some(v) => {
            *disabled = (v != 1) as i32;
            0
        }
        None => {
            *disabled = 0;
            ERROR_FILE_NOT_FOUND_HR
        }
    }
}

pub fn change_blocklist_in(s: &Store, state: u32, eligible: i32) -> HResult {
    let (hr, succeeded) = if eligible == 0 {
        (hresult_from_win32(super::ERROR_NOT_SUPPORTED as i32), 0u32)
    } else {
        match s.set_blocklist_enable(state) {
            Ok(()) => (0, 1),
            Err(e) => (store::io_hresult(&e), 0),
        }
    };
    let p = tlg::state();
    let level = tlg::BLOCKLIST_EVENT_DESC[1];
    let kw = u64::from_le_bytes(tlg::BLOCKLIST_EVENT_DESC[3..11].try_into().unwrap());
    if p.enabled(level, kw) {
        let st = state;
        let el = eligible as u32;
        let tag = KEYWORD_PRIVTAG_BLOCKLIST;
        p.write(tlg::BLOCKLIST_EVENT_DESC, tlg::BLOCKLIST_EVENT_META, &[EventData::of(&st), EventData::of(&el), EventData::of(&succeeded), EventData::of(&tag)]);
    }
    store::log("WldpChangeVulnerableDriverBlocklistState", &format!("state={state} eligible={eligible} hr={hr:#x}"));
    hr
}

pub fn disable_developer_mode_in(s: &Store) -> HResult {
    if developer_gate(s) == 0 {
        return 0;
    }
    let r = s.set_developer_unlock(None);
    store::log("WldpDisableDeveloperMode", &format!("delete HKLM\\{} -> {:?}", store::KEY_DEVELOPER_UNLOCK, r.as_ref().err()));
    0
}

pub fn enable_developer_mode_in(s: &Store) -> HResult {
    if developer_gate(s) == 0 {
        return 0;
    }
    let sddl = wide(DEVELOPER_UNLOCK_SDDL);
    let mut sd: *mut c_void = core::ptr::null_mut();
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, core::ptr::null_mut()) } == 0 {
        let e = unsafe { GetLastError() };
        return if e == 0 { 0x8000_FFFFu32 as i32 } else { hresult_from_win32(e as i32) };
    }
    let hr = if s.developer_unlock().is_some() {
        0
    } else {
        match s.set_developer_unlock(Some(DEVELOPER_UNLOCK_SDDL)) {
            Ok(()) => 0,
            Err(e) => store::io_hresult(&e),
        }
    };
    unsafe { LocalFree(sd) };
    store::log("WldpEnableDeveloperMode", &format!("create HKLM\\{} -> {hr:#x}", store::KEY_DEVELOPER_UNLOCK));
    hr
}

pub fn reset_security_watermark_in(s: &Store) -> HResult {
    let hr = s.delete_file(store::DPP_HSTI);
    if hr < 0 {
        return hr;
    }
    let hr = s.delete_file(store::DPP_SB);
    if hr < 0 {
        hr
    } else {
        0
    }
}

pub unsafe fn wcs(p: *const u16) -> Option<Vec<u16>> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    Some(core::slice::from_raw_parts(p, n + 1).to_vec())
}

pub unsafe fn userprofile_path(path: *const u16) -> (HResult, Option<Vec<u16>>) {
    let users = wide("\\Users\\");
    let bslash = wide("\\");
    let us = UnicodeString { length: 0xe, maximum_length: 0x10, buffer: users.as_ptr() };
    let bs = UnicodeString { length: 2, maximum_length: 4, buffer: bslash.as_ptr() };
    let mut s = UnicodeString { length: 0, maximum_length: 0, buffer: core::ptr::null() };
    RtlInitUnicodeString(&mut s, path);
    let found = RtlFindUnicodeSubstring(&s, &us, 1);
    if found.is_null() {
        return (0x1000_0000, None);
    }
    let q = found.add(us.length as usize / 2);
    let consumed = (q.offset_from(s.buffer) as u16).wrapping_mul(2);
    let rest_len = s.length.wrapping_sub(consumed);
    let rest = UnicodeString { length: rest_len, maximum_length: rest_len, buffer: q };
    let r = RtlFindUnicodeSubstring(&rest, &bs, 1);
    if r.is_null() {
        return (0x1000_0000, None);
    }
    let tail_bytes = s.length.wrapping_sub((r.offset_from(s.buffer) as u16).wrapping_mul(2));
    let mut out: Vec<u16> = "%USERPROFILE%".encode_utf16().collect();
    out.extend_from_slice(core::slice::from_raw_parts(r, tail_bytes as usize / 2));
    out.push(0);
    (0x1000_0000, Some(out))
}

pub unsafe fn wdac_block_dialog_shown(path: *const u16, hresult: u32, guid: *const Guid, actions: *const u16) -> Option<u32> {
    let (hr, conv) = userprofile_path(path);
    if hr < 0 {
        return None;
    }
    let file_name = conv.or_else(|| wcs(path));
    let mut buf = [0u16; 0x104];
    let mut size = 0x104u32;
    let image = if QueryFullProcessImageNameW(GetCurrentProcess(), 0, buf.as_mut_ptr(), &mut size) != 0 {
        let (hr, conv) = userprofile_path(buf.as_ptr());
        if hr < 0 {
            return None;
        }
        conv.or_else(|| wcs(buf.as_ptr()))
    } else {
        Some(vec![0u16])
    };
    let p = tlg::activity();
    let level = tlg::WDAC_EVENT_DESC[1];
    let kw = u64::from_le_bytes(tlg::WDAC_EVENT_DESC[3..11].try_into().unwrap());
    if !p.enabled(level, kw) {
        return None;
    }
    let tag = KEYWORD_PRIVTAG_WDAC;
    let actions_w = wcs(actions);
    let fields = [
        EventData::of(&tag),
        EventData::wstr(file_name.as_deref()),
        EventData::wstr(image.as_deref()),
        EventData::of(&hresult),
        EventData { ptr: guid as u64, size: 16, reserved: 0 },
        EventData::wstr(actions_w.as_deref()),
    ];
    Some(p.write(tlg::WDAC_EVENT_DESC, tlg::WDAC_EVENT_META, &fields))
}

#[no_mangle]
pub unsafe extern "system" fn WldpChangeVulnerableDriverBlocklistState(state: u32) -> HResult {
    let mut eligible = 0i32;
    WldpIsVulnerableDriverBlocklistEligibleToDisable(&mut eligible);
    change_blocklist_in(&Store::open(), state, eligible)
}

#[no_mangle]
pub unsafe extern "system" fn WldpDisableDeveloperMode() -> HResult {
    disable_developer_mode_in(&Store::open())
}

#[no_mangle]
pub unsafe extern "system" fn WldpEnableDeveloperMode() -> HResult {
    enable_developer_mode_in(&Store::open())
}

#[no_mangle]
pub unsafe extern "system" fn WldpResetSecurityWatermarkState() -> HResult {
    reset_security_watermark_in(&Store::open())
}

#[no_mangle]
pub unsafe extern "system" fn WldpTraceLoggingWDACBlockDialogShown(path: *const u16, hresult: u32, guid: *const Guid, actions: *const u16) {
    wdac_block_dialog_shown(path, hresult, guid, actions);
}

pub const CO_E_NOTINITIALIZED: HResult = 0x8004_01F0u32 as i32;
pub const REGDB_E_CLASSNOTREG: HResult = 0x8004_0154u32 as i32;
pub const XML_DOCUMENT_CLASS: &str = "Windows.Data.Xml.Dom.XmlDocument";

#[link(name = "ole32")]
extern "system" {
    fn CoGetApartmentType(apt: *mut i32, qualifier: *mut i32) -> HResult;
}

pub fn console_apartment_status() -> HResult {
    let (mut a, mut q) = (0i32, 0i32);
    let hr = unsafe { CoGetApartmentType(&mut a, &mut q) };
    if hr == CO_E_NOTINITIALIZED {
        CO_E_NOTINITIALIZED
    } else {
        0
    }
}

pub fn console_activate_xml_document(apartment: HResult) -> HResult {
    if apartment < 0 {
        apartment
    } else {
        REGDB_E_CLASSNOTREG
    }
}

pub fn send_smart_app_control_block_toast_in(apartment: HResult) -> HResult {
    let hr = console_activate_xml_document(apartment);
    store::log("WldpSendSmartAppControlBlockToast", &format!("RoActivateInstance({XML_DOCUMENT_CLASS}) -> {hr:#x} (toasthelper.cpp line 0x195/0x1c5)"));
    hr
}

#[no_mangle]
pub unsafe extern "system" fn WldpSendSmartAppControlBlockToast(_file: *const u16, _arg2: *const c_void) -> HResult {
    send_smart_app_control_block_toast_in(console_apartment_status())
}
