use super::*;

const CONSOLE_PATH: &str = r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\gamecoreclient.dll";

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

#[link(name = "runtimeobject")]
extern "system" {
    fn WindowsCreateString(s: *const u16, len: u32, out: *mut Hstring) -> i32;
    fn WindowsDeleteString(s: Hstring) -> i32;
    fn GetRestrictedErrorInfo(info: *mut *mut c_void) -> i32;
}

#[link(name = "oleaut32")]
extern "system" {
    fn SysFreeString(s: *mut u16);
    fn SysStringLen(s: *mut u16) -> u32;
}

struct Console {
    module: *mut c_void,
}

impl Console {
    fn load() -> Option<Console> {
        if !std::path::Path::new(CONSOLE_PATH).is_file() {
            eprintln!("{CONSOLE_PATH} absent, oracle skipped");
            return None;
        }
        let w: Vec<u16> = CONSOLE_PATH.encode_utf16().chain(Some(0)).collect();
        let h = unsafe { LoadLibraryW(w.as_ptr()) };
        assert!(!h.is_null(), "console gamecoreclient.dll must load on the PC for the oracle");
        Some(Console { module: h })
    }
    unsafe fn sym<T: Copy>(&self, n: &str) -> T {
        let c = format!("{n}\0");
        let p = GetProcAddress(self.module, c.as_ptr());
        assert!(!p.is_null(), "{n}");
        core::mem::transmute_copy(&p)
    }
}

fn hstring(s: &[u16]) -> Hstring {
    let mut h = core::ptr::null_mut();
    assert_eq!(unsafe { WindowsCreateString(s.as_ptr(), s.len() as u32, &mut h) }, S_OK);
    h
}

fn take_error_message() -> Option<(i32, String)> {
    let mut info = core::ptr::null_mut();
    if unsafe { GetRestrictedErrorInfo(&mut info) } != S_OK || info.is_null() {
        return None;
    }
    unsafe {
        let vt = *(info as *const *const usize);
        let details: unsafe extern "system" fn(*mut c_void, *mut *mut u16, *mut i32, *mut *mut u16, *mut *mut u16) -> i32 = core::mem::transmute(*vt.add(3));
        let mut d = core::ptr::null_mut();
        let mut e = 0;
        let mut r = core::ptr::null_mut();
        let mut sid = core::ptr::null_mut();
        assert_eq!(details(info, &mut d, &mut e, &mut r, &mut sid), S_OK);
        let msg = if r.is_null() { String::new() } else { String::from_utf16_lossy(core::slice::from_raw_parts(r, SysStringLen(r) as usize)) };
        for b in [d, r, sid] {
            if !b.is_null() {
                SysFreeString(b);
            }
        }
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
        release(info);
        Some((e, msg))
    }
}

type Activation = unsafe extern "system" fn(Hstring, *mut *mut c_void) -> i32;
type ClassObject = unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32;

fn activation_cases(f: Activation) -> Vec<(i32, bool, Option<(i32, String)>)> {
    let name: Vec<u16> = "Windows.Xbox.GameCore.Client".encode_utf16().collect();
    let embedded: Vec<u16> = "Game\0Core".encode_utf16().collect();
    let mut out = Vec::new();
    for s in [core::ptr::null_mut(), hstring(&name), hstring(&embedded)] {
        take_error_message();
        let mut factory = 0x1234usize as *mut c_void;
        let hr = unsafe { f(s, &mut factory) };
        out.push((hr, factory.is_null(), take_error_message()));
        if !s.is_null() {
            unsafe { WindowsDeleteString(s) };
        }
    }
    out
}

#[test]
fn dll_get_activation_factory_has_no_activatable_classes() {
    let ours = activation_cases(DllGetActivationFactory);
    assert_eq!(ours[0].0, E_INVALIDARG);
    assert_eq!(ours[1].0, CLASS_E_CLASSNOTAVAILABLE);
    assert_eq!(ours[2].0, E_INVALIDARG);
    assert!(ours.iter().all(|r| r.1), "factory is cleared on every path");
    if let Some((e, msg)) = &ours[0].2 {
        assert_eq!(*e, E_INVALIDARG);
        assert_eq!(msg, ACTIVATION_ARG_NAME);
    }
    if let Some(c) = Console::load() {
        let theirs = activation_cases(unsafe { c.sym::<Activation>("DllGetActivationFactory") });
        assert_eq!(ours, theirs);
    }
}

fn class_cases(f: ClassObject) -> Vec<(i32, bool)> {
    let unknown = Guid { data1: 0x0BADC0DE, data2: 1, data3: 2, data4: [3, 4, 5, 6, 7, 8, 9, 10] };
    let mut ppv = 0x1234usize as *mut c_void;
    let hr = unsafe { f(&unknown, &IID_ICLASSFACTORY, &mut ppv) };
    vec![(hr, ppv.is_null())]
}

const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_ICLASSFACTORY: Guid = Guid { data1: 1, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const CASE: &str = "TOMBOX_TEST_GAMECORECLIENT_CASE";

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(r: *mut c_void, flags: u32) -> i32;
}

fn consoles() -> Vec<String> {
    console_image::dump_dirs(std::path::Path::new(console_image::GAMES)).into_iter().map(|d| d.join("gamecoreclient.dll")).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()).collect()
}

unsafe fn vcall<T: Copy>(obj: *mut c_void, slot: usize) -> T {
    let vt = *(obj as *const *const usize);
    core::mem::transmute_copy(&*vt.add(slot))
}

unsafe fn release(obj: *mut c_void) -> u32 {
    vcall::<unsafe extern "system" fn(*mut c_void) -> u32>(obj, 2)(obj)
}

unsafe fn create(factory: *mut c_void, out: &mut *mut c_void) -> i32 {
    vcall::<unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32>(factory, 3)(factory, core::ptr::null_mut(), &IID_IUNKNOWN, out)
}

#[test]
fn class_objects_run_every_console_dump_under_the_sandbox() {
    let mut cases = consoles();
    if cases.is_empty() {
        eprintln!("no console gamecoreclient.dll in the title dumps, oracle skipped");
        return;
    }
    cases.push("auto".into());
    let exe = std::env::current_exe().unwrap();
    for c in cases {
        let out = std::process::Command::new(&exe).args(["--exact", "tests::inner_class_oracle", "--nocapture", "--test-threads=1"]).env(CASE, &c).output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && text.contains("ORACLE OK"), "case {c}:\n{text}");
    }
}

#[test]
fn inner_class_oracle() {
    let Ok(case) = std::env::var(CASE) else { return };
    let dir = std::env::temp_dir().join(format!("gamecoreclient_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    if case == "auto" {
        std::env::remove_var("TOMBOX_GAMECORECLIENT_CONSOLE");
    } else {
        std::env::set_var("TOMBOX_GAMECORECLIENT_CONSOLE", &case);
    }
    assert_eq!(DllCanUnloadNow(), S_OK, "nothing loaded yet: no console objects can be alive");
    assert!(CONSOLE.handle.get().is_none(), "DllCanUnloadNow never loads the console on demand");
    assert!(unsafe { CoInitializeEx(core::ptr::null_mut(), 0) } >= 0);

    assert_eq!(class_cases(DllGetClassObject), vec![(CLASS_E_CLASSNOTAVAILABLE, true)]);
    let m = *CONSOLE.handle.get().expect("console loaded by DllGetClassObject");
    assert_ne!(m, 0);
    let path = console_sandbox::modules::module_file(m).unwrap();
    if case != "auto" {
        assert!(path.to_string_lossy().eq_ignore_ascii_case(&case), "{} != {case}", path.display());
    }
    let routed = unsafe { console_sandbox::unrouted(m) };
    for bad in ["OpenSCManagerA", "OpenServiceA", "NotifyServiceStatusChangeA", "CloseServiceHandle", "RegOpenKeyExW", "RegQueryValueExW", "CreateFileW", "NtOpenFile", "NtDeviceIoControlFile", "NtQueryInformationToken"] {
        assert!(!routed.iter().any(|u| u.ends_with(&format!("!{bad}"))), "{bad} reaches the PC: {routed:?}");
    }
    let theirs: ClassObject = unsafe { core::mem::transmute(console_image::symbol(m, "DllGetClassObject").unwrap()) };
    let their_unload: unsafe extern "system" fn() -> i32 = unsafe { core::mem::transmute(console_image::symbol(m, "DllCanUnloadNow").unwrap()) };
    assert_eq!(class_cases(theirs), class_cases(DllGetClassObject));

    let mut f1 = core::ptr::null_mut();
    assert_eq!(unsafe { DllGetClassObject(&CLSID_SIPC_PROXY_CONTROL, &IID_ICLASSFACTORY, &mut f1) }, S_OK);
    assert!(!f1.is_null());
    let mut obj = core::ptr::null_mut();
    assert_eq!(unsafe { create(f1, &mut obj) }, S_OK, "SipcProxyControl constructs: RpcSs status notification and COM security run in the sandbox");
    assert!(!obj.is_null());
    assert_eq!(DllCanUnloadNow(), S_FALSE, "a live console object keeps the module");
    assert_eq!(DllCanUnloadNow(), unsafe { their_unload() });
    unsafe { release(obj) };

    let mut f2 = core::ptr::null_mut();
    assert_eq!(unsafe { DllGetClassObject(&CLSID_SECOND, &IID_ICLASSFACTORY, &mut f2) }, S_OK);
    assert!(!f2.is_null());
    let mut obj2 = core::ptr::null_mut();
    let hr = unsafe { create(f2, &mut obj2) };
    assert_eq!(hr, S_OK, "7FA7A8FD constructs once the XBOX://LI launch claim and the xvcmgmt queue exist; got {hr:#x}");
    assert!(!obj2.is_null());

    let li = console_sandbox::claims::launch_instance();
    let conns = console_sandbox::xvcmgmt::host_connections(li);
    assert_eq!(conns.len(), 1, "the console queue client connected to the emulated host on {{c12a416b, XBOX://LI}}");
    assert!(console_sandbox::xvcmgmt::host_take_received(conns[0]).is_empty(), "construction sends nothing to the host");
    unsafe {
        release(obj2);
        release(f1);
        release(f2);
    }
    let after = DllCanUnloadNow();
    assert_eq!(after, S_FALSE, "the console keeps its init-once queue singleton for the module lifetime");
    assert_eq!(after, unsafe { their_unload() });
    println!("ORACLE OK {} unload-after-release={after}", path.display());
}
