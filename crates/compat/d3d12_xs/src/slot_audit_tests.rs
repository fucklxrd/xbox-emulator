use core::ffi::c_void;

extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
}

#[link(name = "dbghelp")]
extern "system" {
    fn SymSetOptions(opts: u32) -> u32;
    fn SymInitialize(process: *mut c_void, path: *const u8, invade: i32) -> i32;
    fn SymLoadModuleEx(process: *mut c_void, file: *mut c_void, image: *const u8, module: *const u8, base: u64, size: u32, data: *mut c_void, flags: u32) -> u64;
    fn SymFromAddr(process: *mut c_void, addr: u64, disp: *mut u64, info: *mut u8) -> i32;
}

extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}

static SYM_LOCK: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

fn sym_init(ready: &mut bool) {
    if *ready {
        return;
    }
    *ready = true;
    unsafe {
        SymSetOptions(0x2);
        let exe = std::env::current_exe().unwrap_or_default();
        let dir = exe.parent().map(|d| d.to_string_lossy().replace('/', "\\")).unwrap_or_default();
        let c = std::ffi::CString::new(dir).unwrap();
        SymInitialize(GetCurrentProcess(), c.as_ptr() as *const u8, 0);
        let image = std::ffi::CString::new(exe.to_string_lossy().replace('/', "\\")).unwrap();
        let base = GetModuleHandleW(core::ptr::null()) as u64;
        SymLoadModuleEx(GetCurrentProcess(), core::ptr::null_mut(), image.as_ptr() as *const u8, core::ptr::null(), base, 0, core::ptr::null_mut(), 0);
    }
}

pub fn name_of(addr: usize) -> String {
    if addr == 0 {
        return "console_null_entry".to_string();
    }
    let mut ready = SYM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    sym_init(&mut ready);
    let mut buf = vec![0u8; 88 + 1024];
    buf[0..4].copy_from_slice(&88u32.to_le_bytes());
    buf[80..84].copy_from_slice(&1024u32.to_le_bytes());
    let mut disp = 0u64;
    let ok = unsafe { SymFromAddr(GetCurrentProcess(), addr as u64, &mut disp, buf.as_mut_ptr()) };
    if ok == 0 {
        return format!("{addr:#x} err={}", std::io::Error::last_os_error());
    }
    let len = u32::from_le_bytes(buf[76..80].try_into().unwrap()) as usize;
    let raw = String::from_utf8_lossy(&buf[84..84 + len.min(1024)]).into_owned();
    let s = demangle_v0_tail(&raw).unwrap_or(raw);
    if disp != 0 { format!("{s}+{disp:#x}") } else { s }
}

pub fn parse_ranges(s: &str) -> Vec<usize> {
    let mut v = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('-') {
            Some((a, b)) => v.extend(a.parse::<usize>().unwrap()..=b.parse::<usize>().unwrap()),
            None => v.push(part.parse().unwrap()),
        }
    }
    v
}

const DEVICE_DONE: &str = "0-43,45-81,83-89";
const CMDLIST_DONE: &str = "0-63,65-69,71-73,75-79,81-85,87-100,102-109";

pub fn done_field(json: &str, section: &str) -> Option<String> {
    let root = json.find("\"d3d12x\"")?;
    let sec = root + json[root..].find(&format!("\"{section}\""))?;
    let key = sec + json[sec..].find("\"done\"")?;
    let open = key + 6 + json[key + 6..].find('"')? + 1;
    let close = open + json[open..].find('"')?;
    Some(json[open..close].to_string())
}

fn progress_done(section: &str, fallback: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../tools/progress/progress.json");
    std::fs::read_to_string(path).ok().and_then(|j| done_field(&j, section)).unwrap_or_else(|| fallback.to_string())
}

const PLACEHOLDERS: [&str; 3] = ["dev_stub", "inserted_stub", "probe_impl"];

fn demangle_v0_tail(name: &str) -> Option<String> {
    if name.contains("::") {
        return None;
    }
    for module in ["device_wrap", "cmdlist_wrap", "cmdlist_validate"] {
        let marker = format!("{}{module}", module.len());
        if let Some(p) = name.find(&marker) {
            let rest = &name[p + marker.len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let n: usize = digits.parse().ok()?;
            let item = rest.get(digits.len()..digits.len() + n)?;
            return Some(format!("d3d12_xs::{module}::{item}"));
        }
    }
    None
}

fn is_placeholder(raw: &str) -> bool {
    let name = demangle_v0_tail(raw).unwrap_or_else(|| raw.to_string());
    let name = name.as_str();
    let tail = name.rsplit("::").next().unwrap_or(name);
    let numbered = |c: char| tail.len() > 1 && tail.starts_with(c) && tail[1..].chars().all(|x| x.is_ascii_digit());
    let module = name.rsplit("::").nth(1).unwrap_or("");
    name.contains(PLACEHOLDERS[0]) || name.contains(PLACEHOLDERS[1]) || name.contains(PLACEHOLDERS[2])
        || (module == "device_wrap" && numbered('d'))
        || (module == "cmdlist_wrap" && numbered('p'))
}

static FAKE_FNS: [u8; 256] = [0xc3; 256];

fn fake_real_list() -> (Vec<usize>, Box<usize>) {
    let base = FAKE_FNS.as_ptr() as usize;
    let vt: Vec<usize> = (0..128).map(|i| base + i).collect();
    let obj = Box::new(vt.as_ptr() as usize);
    (vt, obj)
}

pub fn device_table() -> Vec<(usize, String)> {
    let v = crate::device_wrap::vtable();
    (0..crate::console_slots::DEVICE_SLOTS).map(|i| (i, name_of(unsafe { *v.add(i) }))).collect()
}

pub fn cmdlist_table() -> Vec<(usize, String)> {
    let (vt, obj) = fake_real_list();
    crate::cmdlist_wrap::apply(&*obj as *const usize as *mut c_void);
    let base = FAKE_FNS.as_ptr() as usize;
    let out = (0..crate::console_slots::CMDLIST_SLOTS)
        .map(|i| {
            let p = crate::cmdlist_wrap::vt_get(i);
            let n = if p >= base && p < base + 256 { format!("pc_passthrough_slot_{}", p - base) } else { name_of(p) };
            (i, n)
        })
        .collect();
    core::mem::forget(vt);
    core::mem::forget(obj);
    out
}

const CHILD_ENV: &str = "TOMBOX_SLOT_AUDIT_CHILD";

#[test]
fn done_slots_have_real_handlers() {
    run_audit_child();
}

fn run_audit_child() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "slot_audit_tests::audit_in_own_process", "--include-ignored", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && text.contains("1 passed"), "slot audit child failed:\n{text}");
}

#[test]
#[ignore = "builds the global Xbox command-list table from a fake PC list; run only by done_slots_have_real_handlers in its own process"]
fn audit_in_own_process() {
    if std::env::var(CHILD_ENV).as_deref() != Ok("1") {
        return run_audit_child();
    }
    let dev = device_table();
    let cl = cmdlist_table();
    let mut report = String::new();
    let mut bad = Vec::new();
    let dev_done = parse_ranges(&progress_done("Device", DEVICE_DONE));
    let cl_done = parse_ranges(&progress_done("CommandList", CMDLIST_DONE));
    for (i, n) in &dev {
        report.push_str(&format!("Device {i} {n}\n"));
        if dev_done.contains(i) && is_placeholder(n) {
            bad.push(format!("Device {i} {n}"));
        }
    }
    for (i, n) in &cl {
        report.push_str(&format!("CommandList {i} {n}\n"));
        if cl_done.contains(i) && is_placeholder(n) {
            bad.push(format!("CommandList {i} {n}"));
        }
    }
    if let Ok(path) = std::env::var("TOMBOX_SLOT_AUDIT_OUT") {
        std::fs::write(path, &report).unwrap();
    }
    assert!(bad.is_empty(), "done slots still on a placeholder: {bad:?}");
    assert_eq!(cl[crate::cmdlist_wrap::CONSOLE_NULL_SLOT].1, "console_null_entry", "xgs12 graphics list slot 66 is a null vtable entry");
}

#[test]
fn placeholder_names_are_recognised() {
    assert!(is_placeholder("d3d12_xs::device_wrap::d44"));
    assert!(is_placeholder("d3d12_xs::cmdlist_wrap::p103"));
    assert!(is_placeholder("d3d12_xs::cmdlist_wrap::inserted_stub"));
    assert!(is_placeholder("d3d12_xs::device_wrap::dev_stub"));
    assert!(!is_placeholder("d3d12_xs::cmdlist_validate::p18"));
    assert!(!is_placeholder("d3d12_xs::device_x::register_page_pool_x"));
    assert!(is_placeholder("RNvNtCsc5KWz7vp7Lc_8d3d12_xs12cmdlist_wrap3p66"));
    assert!(is_placeholder("RNvNtCsc5KWz7vp7Lc_8d3d12_xs11device_wrap3d82"));
    assert!(!is_placeholder("RNvNtCsc5KWz7vp7Lc_8d3d12_xs11device_wrap3f31"));
    assert!(!is_placeholder("RNvNtCsc5KWz7vp7Lc_8d3d12_xs16cmdlist_validate3p18"));
    assert_eq!(parse_ranges("0-2,5"), vec![0, 1, 2, 5]);
    let j = r#"{"x":{"done":"9"},"d3d12x":{"Device":{"total":90,"done":"0-3,5"},"CommandList":{"done":"1-2"}}}"#;
    assert_eq!(done_field(j, "Device").as_deref(), Some("0-3,5"));
    assert_eq!(done_field(j, "CommandList").as_deref(), Some("1-2"));
}
