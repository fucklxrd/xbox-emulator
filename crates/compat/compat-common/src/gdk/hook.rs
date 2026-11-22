use super::{log, query_api_impl};
use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn K32EnumProcessModules(p: *mut c_void, mods: *mut *mut c_void, cb: u32, needed: *mut u32) -> i32;
    fn GetModuleFileNameW(m: *mut c_void, buf: *mut u16, n: u32) -> u32;
    fn GetModuleHandleW(name: *const u16) -> *mut u8;
    fn GetSystemWindowsDirectoryW(buf: *mut u16, n: u32) -> u32;
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old: *mut u32) -> i32;
}

static ORIG_GPA: AtomicUsize = AtomicUsize::new(0);
static ORIG_REG_OPEN: AtomicUsize = AtomicUsize::new(0);
static ORIG_REG_OPEN_EX: AtomicUsize = AtomicUsize::new(0);
static ORIG_REG_QUERY: AtomicUsize = AtomicUsize::new(0);
static ORIG_REG_CLOSE: AtomicUsize = AtomicUsize::new(0);
static CONSOLE_MODE: AtomicUsize = AtomicUsize::new(0);

pub const OEM_KEY: &str = "Software\\Microsoft\\Windows NT\\CurrentVersion\\OEM";
const HKLM: usize = 0xffff_ffff_8000_0002;
const SENTINEL: usize = 0x5842_4F58_4F45_4D11;
pub const PASSWORDLESS_DEVICE_KEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\PasswordLess\\Device";
const SENTINEL_PASSWORDLESS: usize = 0x5842_4F58_5057_4C11;

pub fn emulated_hklm_key(root: &std::path::Path, sub: &str) -> Option<std::path::PathBuf> {
    if !sub.eq_ignore_ascii_case(PASSWORDLESS_DEVICE_KEY) {
        return None;
    }
    Some(sub.split('\\').filter(|c| !c.is_empty()).fold(root.join("hklm"), |a, c| a.join(c)))
}

pub fn emulated_hklm_open(root: &std::path::Path, sub: &str) -> Option<i32> {
    let dir = emulated_hklm_key(root, sub)?;
    Some(if dir.is_dir() { 0 } else { 2 })
}

pub fn emulated_hklm_dword(root: &std::path::Path, value: &str) -> Option<u32> {
    let dir = emulated_hklm_key(root, PASSWORDLESS_DEVICE_KEY)?;
    std::fs::read_to_string(dir.join(format!("{value}.dword"))).ok()?.trim().parse().ok()
}

unsafe fn emulated_open(hkey: usize, sub: *const u16, out: *mut usize) -> Option<i32> {
    if hkey != HKLM || CONSOLE_MODE.load(Ordering::Relaxed) == 0 || out.is_null() {
        return None;
    }
    let r = emulated_hklm_open(&super::data_root(), &super::wstr(sub)?)?;
    if r == 0 {
        *out = SENTINEL_PASSWORDLESS;
    }
    Some(r)
}

unsafe fn query_emulated_dword(name: *const u16, kind: *mut u32, data: *mut u8, cb: *mut u32) -> i32 {
    let Some(v) = super::wstr(name).and_then(|n| emulated_hklm_dword(&super::data_root(), &n)) else {
        return 2;
    };
    if !kind.is_null() {
        *kind = 4;
    }
    if !data.is_null() {
        if cb.is_null() || *cb < 4 {
            if !cb.is_null() {
                *cb = 4;
            }
            return 234;
        }
        core::ptr::write_unaligned(data as *mut u32, v);
    }
    if !cb.is_null() {
        *cb = 4;
    }
    0
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn module_path(m: *mut c_void) -> String {
    let mut buf = [0u16; 1024];
    let n = GetModuleFileNameW(m, buf.as_mut_ptr(), buf.len() as u32) as usize;
    String::from_utf16_lossy(&buf[..n.min(buf.len())])
}

unsafe fn is_xgameruntime(h: *mut c_void) -> bool {
    !h.is_null() && module_path(h).to_ascii_lowercase().ends_with("\\xgameruntime.dll")
}

unsafe fn is_query_name(name: *const u8) -> bool {
    if (name as usize) < 0x10000 {
        return name as usize == 3;
    }
    super::cstr(name).as_deref() == Some("QueryApiImpl")
}

pub unsafe extern "system" fn get_proc_address_hook(h: *mut c_void, name: *const u8) -> *mut c_void {
    let p = crate::os_host::routes_rt::resolve_proc(h, name);
    if !p.is_null() {
        return p;
    }
    let o = ORIG_GPA.load(Ordering::Acquire);
    if o == 0 {
        return core::ptr::null_mut();
    }
    let real: unsafe extern "system" fn(*mut c_void, *const u8) -> *mut c_void = core::mem::transmute(o);
    let r = real(h, name);
    if !r.is_null() && is_query_name(name) && is_xgameruntime(h) {
        super::set_real_query(r as usize);
        log("GetProcAddress", &format!("xgameruntime!QueryApiImpl {:#x} -> emulated runtime", r as usize));
        return query_api_impl as *mut c_void;
    }
    super::mediacontrol::route(h, name, r)
}

pub fn console_mode(device: &str) -> Option<u32> {
    match device.to_ascii_lowercase().as_str() {
        "anaconda" | "scarlett" | "seriesx" | "xsx" => Some(0x64),
        "lockhart" | "seriess" | "xss" => Some(0x60),
        "devkit" => Some(0x68),
        "pc" | "off" | "" => None,
        x => u32::from_str_radix(x.trim_start_matches("0x"), 16).ok(),
    }
}

unsafe fn is_oem(hkey: usize, sub: *const u16) -> bool {
    hkey == HKLM && CONSOLE_MODE.load(Ordering::Relaxed) != 0 && super::wstr(sub).map(|s| s.eq_ignore_ascii_case(OEM_KEY)).unwrap_or(false)
}

unsafe extern "system" fn reg_open_hook(hkey: usize, sub: *const u16, out: *mut usize) -> i32 {
    if is_oem(hkey, sub) && !out.is_null() {
        *out = SENTINEL;
        return 0;
    }
    if let Some(r) = emulated_open(hkey, sub, out) {
        return r;
    }
    let f: unsafe extern "system" fn(usize, *const u16, *mut usize) -> i32 = core::mem::transmute(ORIG_REG_OPEN.load(Ordering::Acquire));
    f(hkey, sub, out)
}

unsafe extern "system" fn reg_open_ex_hook(hkey: usize, sub: *const u16, opts: u32, sam: u32, out: *mut usize) -> i32 {
    if is_oem(hkey, sub) && !out.is_null() {
        *out = SENTINEL;
        return 0;
    }
    if let Some(r) = emulated_open(hkey, sub, out) {
        return r;
    }
    let f: unsafe extern "system" fn(usize, *const u16, u32, u32, *mut usize) -> i32 = core::mem::transmute(ORIG_REG_OPEN_EX.load(Ordering::Acquire));
    f(hkey, sub, opts, sam, out)
}

pub unsafe fn query_console_mode(name: *const u16, kind: *mut u32, data: *mut u8, cb: *mut u32) -> i32 {
    if super::wstr(name).map(|s| s.eq_ignore_ascii_case("ConsoleMode")) != Some(true) {
        return 2;
    }
    if !kind.is_null() {
        *kind = 4;
    }
    let v = CONSOLE_MODE.load(Ordering::Relaxed) as u32;
    if !data.is_null() {
        if cb.is_null() || *cb < 4 {
            if !cb.is_null() {
                *cb = 4;
            }
            return 234;
        }
        core::ptr::write_unaligned(data as *mut u32, v);
    }
    if !cb.is_null() {
        *cb = 4;
    }
    0
}

unsafe extern "system" fn reg_query_hook(hkey: usize, name: *const u16, reserved: *mut u32, kind: *mut u32, data: *mut u8, cb: *mut u32) -> i32 {
    if hkey == SENTINEL {
        return query_console_mode(name, kind, data, cb);
    }
    if hkey == SENTINEL_PASSWORDLESS {
        return query_emulated_dword(name, kind, data, cb);
    }
    let f: unsafe extern "system" fn(usize, *const u16, *mut u32, *mut u32, *mut u8, *mut u32) -> i32 = core::mem::transmute(ORIG_REG_QUERY.load(Ordering::Acquire));
    f(hkey, name, reserved, kind, data, cb)
}

unsafe extern "system" fn reg_close_hook(hkey: usize) -> i32 {
    if hkey == SENTINEL || hkey == SENTINEL_PASSWORDLESS {
        return 0;
    }
    let f: unsafe extern "system" fn(usize) -> i32 = core::mem::transmute(ORIG_REG_CLOSE.load(Ordering::Acquire));
    f(hkey)
}

struct Target {
    name: &'static str,
    hook: usize,
    orig: &'static AtomicUsize,
}

unsafe fn rd32(p: *const u8) -> u32 {
    core::ptr::read_unaligned(p as *const u32)
}

pub unsafe fn patch_module(base: *mut u8, targets: &[(&str, usize, &AtomicUsize)]) -> usize {
    if base.is_null() || core::ptr::read_unaligned(base as *const u16) != 0x5a4d {
        return 0;
    }
    let pe = base.add(rd32(base.add(0x3c)) as usize);
    if rd32(pe) != 0x4550 || core::ptr::read_unaligned(pe.add(24) as *const u16) != 0x20b {
        return 0;
    }
    let dir = pe.add(24 + 0x70 + 8);
    let (rva, size) = (rd32(dir) as usize, rd32(dir.add(4)) as usize);
    if rva == 0 || size == 0 {
        return 0;
    }
    let mut patched = 0;
    let mut desc = base.add(rva);
    loop {
        let int_rva = rd32(desc) as usize;
        let name_rva = rd32(desc.add(12)) as usize;
        let iat_rva = rd32(desc.add(16)) as usize;
        if name_rva == 0 && iat_rva == 0 {
            break;
        }
        if int_rva != 0 && iat_rva != 0 {
            let mut i = 0usize;
            loop {
                let thunk = core::ptr::read_unaligned(base.add(int_rva + i * 8) as *const u64);
                if thunk == 0 {
                    break;
                }
                if thunk & (1 << 63) == 0 {
                    let fname = super::cstr(base.add(thunk as usize + 2)).unwrap_or_default();
                    for (tname, hook, orig) in targets {
                        if fname == *tname {
                            let slot = base.add(iat_rva + i * 8) as *mut usize;
                            let cur = core::ptr::read_volatile(slot);
                            if cur != *hook {
                                let _ = orig.compare_exchange(0, cur, Ordering::AcqRel, Ordering::Acquire);
                                let mut old = 0u32;
                                if VirtualProtect(slot as *mut c_void, 8, 0x40, &mut old) != 0 {
                                    core::ptr::write_volatile(slot, *hook);
                                    let mut tmp = 0u32;
                                    VirtualProtect(slot as *mut c_void, 8, old, &mut tmp);
                                    patched += 1;
                                }
                            }
                        }
                    }
                }
                i += 1;
            }
        }
        desc = desc.add(20);
    }
    patched
}

fn own_module() -> *mut c_void {
    extern "system" {
        fn GetModuleHandleExW(flags: u32, name: *const u16, out: *mut *mut c_void) -> i32;
    }
    let mut h: *mut c_void = core::ptr::null_mut();
    unsafe { GetModuleHandleExW(0x6, own_module as *const u16, &mut h) };
    h
}

unsafe fn modules() -> Vec<*mut c_void> {
    let mut v: Vec<*mut c_void> = vec![core::ptr::null_mut(); 1024];
    let mut needed = 0u32;
    if K32EnumProcessModules(GetCurrentProcess(), v.as_mut_ptr(), (v.len() * 8) as u32, &mut needed) == 0 {
        return Vec::new();
    }
    v.truncate((needed as usize / 8).min(1024));
    v
}

unsafe fn windows_dir() -> String {
    let mut buf = [0u16; 512];
    let n = GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) as usize;
    let mut s = String::from_utf16_lossy(&buf[..n.min(buf.len())]).to_ascii_lowercase();
    if !s.ends_with('\\') {
        s.push('\\');
    }
    s
}

fn targets(registry: bool) -> Vec<Target> {
    let mut t = vec![Target { name: "GetProcAddress", hook: get_proc_address_hook as usize, orig: &ORIG_GPA }];
    if registry {
        t.push(Target { name: "RegOpenKeyW", hook: reg_open_hook as usize, orig: &ORIG_REG_OPEN });
        t.push(Target { name: "RegOpenKeyExW", hook: reg_open_ex_hook as usize, orig: &ORIG_REG_OPEN_EX });
        t.push(Target { name: "RegQueryValueExW", hook: reg_query_hook as usize, orig: &ORIG_REG_QUERY });
        t.push(Target { name: "RegCloseKey", hook: reg_close_hook as usize, orig: &ORIG_REG_CLOSE });
    }
    for (name, hook, orig) in super::fs::targets() {
        t.push(Target { name, hook, orig });
    }
    t
}

fn lists() -> (Vec<(&'static str, usize, &'static AtomicUsize)>, Vec<(&'static str, usize, &'static AtomicUsize)>) {
    let registry = CONSOLE_MODE.load(Ordering::Relaxed) != 0;
    let base: Vec<(&'static str, usize, &'static AtomicUsize)> = targets(registry).iter().map(|x| (x.name, x.hook, x.orig)).collect();
    let mut title = base.clone();
    title.extend(super::identity::targets());
    title.extend(super::memstatus::targets());
    title.extend(crate::kmd_bridge::targets());
    (base, title)
}

unsafe fn list_for<'a>(m: *mut c_void, base: &'a [(&'static str, usize, &'static AtomicUsize)], title: &'a [(&'static str, usize, &'static AtomicUsize)]) -> &'a [(&'static str, usize, &'static AtomicUsize)] {
    if super::identity::is_tombox_module(m) { base } else { title }
}

pub fn patch_all() -> (usize, usize) {
    let (base, title) = lists();
    unsafe {
        let win = windows_dir();
        let (mut mods, mut slots) = (0, 0);
        let me = own_module();
        let me = if me == GetModuleHandleW(core::ptr::null()) as *mut c_void { core::ptr::null_mut() } else { me };
        for m in modules() {
            if m == me || module_path(m).to_ascii_lowercase().starts_with(&win) {
                continue;
            }
            let n = patch_module(m as *mut u8, list_for(m, &base, &title));
            if n > 0 {
                mods += 1;
                slots += n;
            }
        }
        (mods, slots)
    }
}

pub fn patch_late(module: *mut u8) -> usize {
    let (base, title) = lists();
    unsafe { patch_module(module, list_for(module as *mut c_void, &base, &title)) }
}

pub fn target_family() -> Option<String> {
    static F: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    F.get_or_init(|| {
        let dir = super::title().dir.as_ref()?;
        dir.ancestors().take(5).find_map(|d| std::fs::read_to_string(d.join("MicrosoftGame.config")).ok().and_then(|t| family_of(&t)))
    }).clone()
}

pub fn family_of(t: &str) -> Option<String> {
    let mut rest = t;
    while let Some(i) = rest.find("TargetDeviceFamily=\"") {
        let v = &rest[i + 20..];
        let end = v.find('"')?;
        let val = v[..end].trim().to_ascii_lowercase();
        let before = &rest[..i];
        let in_comment = before.rfind("<!--").map(|c| before[c..].find("-->").is_none()).unwrap_or(false);
        if !in_comment {
            return Some(val);
        }
        rest = &v[end..];
    }
    None
}

pub fn default_device(family: Option<&str>) -> &'static str {
    match family {
        Some("xboxone") | Some("scarlett") => "anaconda",
        _ => "",
    }
}

pub fn install() -> String {
    let device = super::identity::device();
    let mode = console_mode(&device);
    CONSOLE_MODE.store(mode.unwrap_or(0) as usize, Ordering::Relaxed);
    let preloaded = unsafe { !GetModuleHandleW(wide("xgameruntime.dll").as_ptr()).is_null() };
    let late = super::late::watch();
    let (mods, slots) = patch_all();
    let mut s = format!("import hooks: {slots} slots in {mods} modules, late loads {}; {}", if late { "watched" } else { "NOT watched" }, super::fs::describe());
    let cpu = super::identity::cpu();
    s.push_str(&format!("; identity {} (XSystemGetDeviceType {}), title CPU view {} cores mask {:#x}{}", super::identity::device_form(), super::identity::device_type_value(), cpu.count, cpu.mask, if super::identity::targets().is_empty() { " (GetSystemInfo hooks off)" } else { "" }));
    match mode {
        Some(m) => s.push_str(&format!("; XSystemGetDeviceType registry wrap ON (ConsoleMode={m:#x}, device_type={device})")),
        None => s.push_str(&format!("; device type registry wrap off (device_type '{device}', family {:?})", target_family())),
    }
    if preloaded {
        s.push_str("; WARNING xgameruntime.dll was already loaded - a title that resolved QueryApiImpl earlier keeps the real runtime");
    }
    s
}

#[cfg(test)]
mod emulated_hklm_tests {
    use super::*;

    #[test]
    fn passwordless_device_key_reads_the_emulated_hklm_store() {
        let root = std::env::temp_dir().join(format!("tombox_cc_hklm_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(emulated_hklm_open(&root, "Software\\Microsoft\\Windows NT\\CurrentVersion\\OEM"), None);
        assert_eq!(emulated_hklm_open(&root, PASSWORDLESS_DEVICE_KEY), Some(2));
        assert_eq!(emulated_hklm_dword(&root, "DevicePasswordLessBuildVersion"), None);
        let dir = emulated_hklm_key(&root, &PASSWORDLESS_DEVICE_KEY.to_ascii_lowercase()).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(emulated_hklm_open(&root, PASSWORDLESS_DEVICE_KEY), Some(0));
        std::fs::write(dir.join("DevicePasswordLessBuildVersion.dword"), "2\n").unwrap();
        assert_eq!(emulated_hklm_dword(&root, "DevicePasswordLessBuildVersion"), Some(2));
        assert_eq!(emulated_hklm_dword(&root, "Other"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
