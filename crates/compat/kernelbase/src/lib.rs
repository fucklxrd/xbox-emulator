use core::ffi::c_void;
use core::ptr::null_mut;

pub mod repository;

use repository::Lookup;

pub type Hkey = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn PackageFamilyNameFromFullName(full: *const u16, len: *mut u32, family: *mut u16) -> i32;
    fn PackageIdFromFullName(full: *const u16, flags: u32, len: *mut u32, buf: *mut u8) -> i32;
    fn PackageFamilyNameFromId(id: *const u8, len: *mut u32, family: *mut u16) -> i32;
    fn GetLastError() -> u32;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenCurrentUser(sam: u32, out: *mut Hkey) -> i32;
    fn RegOpenKeyExW(key: Hkey, sub: *const u16, opts: u32, sam: u32, out: *mut Hkey) -> i32;
    fn RegQueryValueExW(key: Hkey, name: *const u16, res: *mut u32, ty: *mut u32, data: *mut u8, len: *mut u32) -> i32;
    fn RegCloseKey(key: Hkey) -> i32;
    fn GetTokenInformation(token: *mut c_void, class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(s: *const u16, rev: u32, sd: *mut *mut c_void, len: *mut u32) -> i32;
    fn GetSecurityDescriptorDacl(sd: *mut c_void, present: *mut i32, dacl: *mut *mut c_void, defaulted: *mut i32) -> i32;
    fn FreeSid(sid: *mut c_void) -> *mut c_void;
}

#[link(name = "userenv")]
extern "system" {
    fn DeriveAppContainerSidFromAppContainerName(name: *const u16, sid: *mut *mut c_void) -> i32;
}

#[repr(C)]
struct UnicodeString {
    len: u16,
    max: u16,
    buf: *mut u16,
}

#[link(name = "ntdll")]
extern "system" {
    fn RtlConvertSidToUnicodeString(out: *mut UnicodeString, sid: *const c_void, alloc: u8) -> i32;
    fn RtlFreeUnicodeString(s: *mut UnicodeString);
}

pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const HKEY_USERS: Hkey = 0x8000_0003usize as Hkey;
pub const KEY_ALL_ACCESS: u32 = 0xf003f;
pub const ALL_APP_PACKAGES: &str = "S-1-15-2-1";
pub const CURRENT_THREAD_EFFECTIVE_TOKEN: *mut c_void = -6isize as *mut c_void;

pub fn hresult_from_win32(e: u32) -> i32 {
    if e as i32 <= 0 {
        e as i32
    } else {
        ((e & 0xffff) | 0x8007_0000) as i32
    }
}

unsafe fn wstr(p: *const u16) -> String {
    String::from_utf16_lossy(core::slice::from_raw_parts(p, wlen(p)))
}

fn wz(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn family_of(full: *const u16) -> Result<String, i32> {
    let mut id = [0u8; 0xf0];
    let mut len = id.len() as u32;
    let r = PackageIdFromFullName(full, 0, &mut len, id.as_mut_ptr());
    if r != 0 {
        return Err(hresult_from_win32(r as u32));
    }
    let mut family = [0u16; 0x41];
    let mut flen = family.len() as u32;
    let r = PackageFamilyNameFromId(id.as_ptr(), &mut flen, family.as_mut_ptr());
    if r != 0 {
        return Err(hresult_from_win32(r as u32));
    }
    Ok(wstr(family.as_ptr()))
}

pub unsafe fn sid_string(sid: *const c_void) -> Result<String, i32> {
    let mut u = UnicodeString { len: 0, max: 0, buf: null_mut() };
    let st = RtlConvertSidToUnicodeString(&mut u, sid, 1);
    if st < 0 {
        return Err(st);
    }
    let s = String::from_utf16_lossy(core::slice::from_raw_parts(u.buf, (u.len / 2) as usize));
    RtlFreeUnicodeString(&mut u);
    Ok(s)
}

pub unsafe fn effective_user() -> Result<String, i32> {
    let mut buf = [0u64; 64];
    let mut ret = 0u32;
    if GetTokenInformation(CURRENT_THREAD_EFFECTIVE_TOKEN, 1, buf.as_mut_ptr() as *mut c_void, 512, &mut ret) == 0 {
        return Err(hresult_from_win32(GetLastError()));
    }
    sid_string(buf[0] as *const c_void)
}

unsafe fn resolve_user(user: *const c_void) -> Result<String, i32> {
    let s = if user.is_null() { effective_user()? } else { sid_string(user)? };
    let sub = wz(&s);
    let mut k: Hkey = null_mut();
    let r = RegOpenKeyExW(HKEY_USERS, sub.as_ptr(), 0, KEY_ALL_ACCESS, &mut k);
    if r != 0 {
        return Err(hresult_from_win32(r as u32));
    }
    RegCloseKey(k);
    Ok(s)
}

pub fn capability_form(mut s: Vec<u16>) -> Vec<u16> {
    match s.len() {
        n if n > 7 => s[7] = b'3' as u16,
        7 => s.push(b'3' as u16),
        _ => {}
    }
    s
}

unsafe fn family_dacl(package_sid: *const u16, flags: u32, family: &str) -> Result<String, i32> {
    let base: Vec<u16> = if flags == 2 {
        ALL_APP_PACKAGES.encode_utf16().collect()
    } else if !package_sid.is_null() {
        core::slice::from_raw_parts(package_sid, wlen(package_sid)).to_vec()
    } else {
        let mut sid: *mut c_void = null_mut();
        let hr = DeriveAppContainerSidFromAppContainerName(wz(family).as_ptr(), &mut sid);
        if hr < 0 {
            return Err(hr);
        }
        let s = sid_string(sid);
        FreeSid(sid);
        s?.encode_utf16().collect()
    };
    let cap = String::from_utf16_lossy(&capability_form(base));
    let sddl = format!("D:AR(A;OICI;KR;;;{ALL_APP_PACKAGES})(A;OICI;KR;;;{cap})");
    let mut sd: *mut c_void = null_mut();
    if ConvertStringSecurityDescriptorToSecurityDescriptorW(wz(&sddl).as_ptr(), 1, &mut sd, null_mut()) == 0 {
        return Err(hresult_from_win32(GetLastError()));
    }
    let (mut present, mut defaulted) = (0i32, 0i32);
    let mut dacl: *mut c_void = null_mut();
    let ok = GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted);
    let err = GetLastError();
    LocalFree(sd);
    if ok == 0 {
        return Err(hresult_from_win32(err));
    }
    Ok(sddl)
}

#[no_mangle]
pub unsafe extern "system" fn AddPackageToFamilyXref(user: *const c_void, full: *const u16, package_sid: *const u16, flags: u32, install_time: u64) -> i32 {
    if full.is_null() {
        return E_INVALIDARG;
    }
    let family = match family_of(full) {
        Ok(f) => f,
        Err(hr) => return hr,
    };
    let sid = match resolve_user(user) {
        Ok(s) => s,
        Err(hr) => return hr,
    };
    let sddl = match family_dacl(package_sid, flags, &family) {
        Ok(s) => s,
        Err(hr) => return hr,
    };
    repository::store().add(&sid, &family, &wstr(full), &sddl, flags, install_time);
    0
}

#[no_mangle]
pub unsafe extern "system" fn RemovePackageFromFamilyXref(user: *const c_void, full: *const u16) -> i32 {
    if full.is_null() {
        return E_INVALIDARG;
    }
    let family = match family_of(full) {
        Ok(f) => f,
        Err(hr) => return hr,
    };
    let sid = match resolve_user(user) {
        Ok(s) => s,
        Err(hr) => return hr,
    };
    repository::store().remove(&sid, &family, &wstr(full));
    0
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct FileTime {
    pub low: u32,
    pub high: u32,
}

pub const ERROR_FILE_NOT_FOUND: i32 = 2;
pub const ERROR_INVALID_PARAMETER: i32 = 0x57;
pub const ERROR_NOT_FOUND: i32 = 0x490;
pub const ERROR_REGISTRY_CORRUPT_TYPE: i32 = 0x3d55;
pub const KEY_READ: u32 = 0x20019;
pub const REG_QWORD: u32 = 0xb;
pub const PATH_CAP: usize = 0x128;
pub const FAMILIES: &str = "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\Repository\\Families";

unsafe fn wlen(p: *const u16) -> usize {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    n
}

pub unsafe fn install_time_under(root: Hkey, prefix: &str, full: *const u16, out: *mut FileTime) -> i32 {
    install_time_with(root, prefix, full, out, None)
}

pub unsafe fn install_time_with(root: Hkey, prefix: &str, full: *const u16, out: *mut FileTime, overlay_user: Option<&str>) -> i32 {
    if full.is_null() || out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let mut family = [0u16; 65];
    let mut flen = family.len() as u32;
    let r = PackageFamilyNameFromFullName(full, &mut flen, family.as_mut_ptr());
    if r != 0 {
        return r;
    }
    let mut path: Vec<u16> = prefix.encode_utf16().collect();
    path.push(b'\\' as u16);
    path.extend_from_slice(&family[..wlen(family.as_ptr())]);
    path.push(b'\\' as u16);
    path.extend_from_slice(core::slice::from_raw_parts(full, wlen(full)));
    if path.len() >= PATH_CAP {
        return ERROR_INVALID_PARAMETER;
    }
    if let Some(user) = overlay_user {
        let fam = String::from_utf16_lossy(&family[..wlen(family.as_ptr())]);
        match repository::store().lookup(user, &fam, &wstr(full)) {
            Lookup::Present(t) => {
                (*out).low = t as u32;
                (*out).high = (t >> 32) as u32;
                return 0;
            }
            Lookup::Removed => return ERROR_NOT_FOUND,
            Lookup::Untouched => {}
        }
    }
    path.push(0);
    let mut key: Hkey = null_mut();
    let r = RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ, &mut key);
    if r != 0 {
        return if r == ERROR_FILE_NOT_FOUND { ERROR_NOT_FOUND } else { r };
    }
    let name: Vec<u16> = "InstallTime".encode_utf16().chain(Some(0)).collect();
    let mut ty = 0u32;
    let mut data = 0u64;
    let mut len = 8u32;
    let r = RegQueryValueExW(key, name.as_ptr(), null_mut(), &mut ty, &mut data as *mut u64 as *mut u8, &mut len);
    RegCloseKey(key);
    if r != 0 {
        return r;
    }
    if ty != REG_QWORD {
        return ERROR_REGISTRY_CORRUPT_TYPE;
    }
    (*out).low = data as u32;
    (*out).high = (data >> 32) as u32;
    0
}

#[no_mangle]
pub unsafe extern "system" fn GetPackageInstallTime(full: *const u16, out: *mut FileTime) -> i32 {
    if full.is_null() || out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let mut user: Hkey = null_mut();
    let r = RegOpenCurrentUser(KEY_READ, &mut user);
    if r != 0 {
        return r;
    }
    let me = effective_user().ok();
    let r = install_time_with(user, FAMILIES, full, out, me.as_deref());
    RegCloseKey(user);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ptr::null;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(key: Hkey, sub: *const u16, r: u32, class: *const u16, opts: u32, sam: u32, sa: *const c_void, out: *mut Hkey, disp: *mut u32) -> i32;
        fn RegSetValueExW(key: Hkey, name: *const u16, r: u32, ty: u32, data: *const u8, len: u32) -> i32;
        fn RegDeleteTreeW(key: Hkey, sub: *const u16) -> i32;
    }

    const HKCU: Hkey = 0x8000_0001usize as Hkey;
    const FULL: &str = "TomBox.Test_1.0.0.0_x64__8wekyb3d8bbwe";
    const FAMILY: &str = "TomBox.Test_8wekyb3d8bbwe";

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn seed(prefix: &str, ty: u32, data: &[u8]) {
        unsafe {
            let mut k: Hkey = null_mut();
            let sub = w(&format!("{prefix}\\{FAMILY}\\{FULL}"));
            assert_eq!(RegCreateKeyExW(HKCU, sub.as_ptr(), 0, null(), 0, 0xf003f, null(), &mut k, null_mut()), 0);
            assert_eq!(RegSetValueExW(k, w("InstallTime").as_ptr(), 0, ty, data.as_ptr(), data.len() as u32), 0);
            RegCloseKey(k);
        }
    }

    fn wipe(prefix: &str) {
        unsafe { RegDeleteTreeW(HKCU, w(prefix).as_ptr()) };
    }

    #[test]
    fn reads_qword_from_families_key() {
        let p = format!("Software\\TomBoxTest-{}-a", std::process::id());
        seed(&p, REG_QWORD, &0x01d9_1234_5678_9abcu64.to_le_bytes());
        let mut ft = FileTime::default();
        let r = unsafe { install_time_under(HKCU, &p, w(FULL).as_ptr(), &mut ft) };
        wipe(&p);
        assert_eq!(r, 0);
        assert_eq!(ft, FileTime { low: 0x5678_9abc, high: 0x01d9_1234 });
    }

    #[test]
    fn wrong_type_is_corrupt_type() {
        let p = format!("Software\\TomBoxTest-{}-b", std::process::id());
        seed(&p, 4, &7u32.to_le_bytes());
        let mut ft = FileTime { low: 9, high: 9 };
        let r = unsafe { install_time_under(HKCU, &p, w(FULL).as_ptr(), &mut ft) };
        wipe(&p);
        assert_eq!(r, ERROR_REGISTRY_CORRUPT_TYPE);
        assert_eq!(ft, FileTime { low: 9, high: 9 });
    }

    #[test]
    fn absent_key_is_not_found() {
        let p = format!("Software\\TomBoxTest-{}-c", std::process::id());
        let mut ft = FileTime::default();
        assert_eq!(unsafe { install_time_under(HKCU, &p, w(FULL).as_ptr(), &mut ft) }, ERROR_NOT_FOUND);
    }

    fn unique(tag: &str) -> (String, String) {
        let n = format!("TomBox.Xref{tag}{}", std::process::id());
        (format!("{n}_1.0.0.0_x64__8wekyb3d8bbwe"), format!("{n}_8wekyb3d8bbwe"))
    }

    #[test]
    fn family_xref_add_is_read_by_get_package_install_time() {
        let (full, family) = unique("A");
        let fw = w(&full);
        let mut ft = FileTime::default();
        unsafe {
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), ERROR_NOT_FOUND);
            assert_eq!(AddPackageToFamilyXref(null(), fw.as_ptr(), null(), 1, 0x01d9_1234_5678_9abc), 0);
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), 0);
            assert_eq!(ft, FileTime { low: 0x5678_9abc, high: 0x01d9_1234 });
            let me = effective_user().unwrap();
            let fam = repository::store().family(&me, &family).unwrap();
            assert_eq!(fam.packages.values().next().unwrap().flags, 1);
            let mut sid: *mut c_void = null_mut();
            assert!(DeriveAppContainerSidFromAppContainerName(w(&family).as_ptr(), &mut sid) >= 0);
            let cap = String::from_utf16_lossy(&capability_form(sid_string(sid).unwrap().encode_utf16().collect()));
            FreeSid(sid);
            assert!(cap.starts_with("S-1-15-3-"));
            assert_eq!(fam.dacl_sddl, format!("D:AR(A;OICI;KR;;;S-1-15-2-1)(A;OICI;KR;;;{cap})"));
            assert_eq!(AddPackageToFamilyXref(null(), fw.as_ptr(), null(), 0, 0x0000_0002_0000_0001), 0);
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), 0);
            assert_eq!(ft, FileTime { low: 1, high: 2 });
            assert_eq!(RemovePackageFromFamilyXref(null(), fw.as_ptr()), 0);
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), ERROR_NOT_FOUND);
            assert!(repository::store().family(&me, &family).is_none());
            assert_eq!(RemovePackageFromFamilyXref(null(), fw.as_ptr()), 0);
        }
    }

    #[test]
    fn family_xref_flags_two_uses_all_app_packages() {
        let (full, family) = unique("B");
        let fw = w(&full);
        unsafe {
            assert_eq!(AddPackageToFamilyXref(null(), fw.as_ptr(), w("not-a-sid").as_ptr(), 2, 5), 0);
            let fam = repository::store().family(&effective_user().unwrap(), &family).unwrap();
            assert_eq!(fam.dacl_sddl, "D:AR(A;OICI;KR;;;S-1-15-2-1)(A;OICI;KR;;;S-1-15-3-1)");
            assert_eq!(RemovePackageFromFamilyXref(null(), fw.as_ptr()), 0);
        }
    }

    #[test]
    fn family_xref_bad_package_sid_is_converter_error() {
        let (full, _) = unique("C");
        let fw = w(&full);
        let mut ft = FileTime::default();
        unsafe {
            let hr = AddPackageToFamilyXref(null(), fw.as_ptr(), w("S-1-15-2-zz").as_ptr(), 0, 5);
            assert_eq!(hr, hresult_from_win32(1337));
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), ERROR_NOT_FOUND);
        }
    }

    #[test]
    fn family_xref_argument_and_name_errors_are_hresults() {
        let bad = w("not a package name");
        unsafe {
            assert_eq!(AddPackageToFamilyXref(null(), null(), null(), 0, 0), E_INVALIDARG);
            assert_eq!(RemovePackageFromFamilyXref(null(), null()), E_INVALIDARG);
            let mut id = [0u8; 0xf0];
            let mut n = id.len() as u32;
            let pc = PackageIdFromFullName(bad.as_ptr(), 0, &mut n, id.as_mut_ptr());
            assert_ne!(pc, 0);
            assert_eq!(AddPackageToFamilyXref(null(), bad.as_ptr(), null(), 0, 0), hresult_from_win32(pc as u32));
            assert_eq!(RemovePackageFromFamilyXref(null(), bad.as_ptr()), hresult_from_win32(pc as u32));
        }
    }

    #[test]
    fn family_xref_other_user_is_not_the_current_user() {
        let (full, _) = unique("D");
        let fw = w(&full);
        let mut ft = FileTime::default();
        let system: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
        unsafe {
            let hr = AddPackageToFamilyXref(system.as_ptr() as *const c_void, fw.as_ptr(), null(), 0, 9);
            assert!(hr == 0 || hr == hresult_from_win32(5), "{hr:#x}");
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), ERROR_NOT_FOUND);
            let ghost: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 0x78, 0x56, 0x34, 0x12];
            assert_eq!(AddPackageToFamilyXref(ghost.as_ptr() as *const c_void, fw.as_ptr(), null(), 0, 9), hresult_from_win32(2));
        }
    }

    #[test]
    fn capability_form_rewrites_char_seven() {
        let f = |s: &str| String::from_utf16_lossy(&capability_form(s.encode_utf16().collect()));
        assert_eq!(f("S-1-15-2-1"), "S-1-15-3-1");
        assert_eq!(f("S-1-15-"), "S-1-15-3");
        assert_eq!(f("S-1"), "S-1");
    }

    #[test]
    fn overlong_path_is_invalid_parameter() {
        let p = "x".repeat(PATH_CAP);
        let mut ft = FileTime::default();
        assert_eq!(unsafe { install_time_under(HKCU, &p, w(FULL).as_ptr(), &mut ft) }, ERROR_INVALID_PARAMETER);
    }
}
