use std::path::PathBuf;

fn built(file: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let mut dirs: Vec<PathBuf> = std::env::var_os("CARGO_TARGET_DIR").map(|d| vec![PathBuf::from(d).join("debug")]).unwrap_or_default();
    dirs.extend(exe.ancestors().skip(1).flat_map(|a| [a.to_path_buf(), a.join("debug")]));
    dirs.into_iter()
        .map(|d| d.join(file))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("{file} not built; run cargo build -p kerneltracecontrol before cargo test"))
}

#[test]
fn export_table_matches_console_ordinals() {
    let rows = xone_exports::parse(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/exports.txt")).unwrap());
    let mut want: Vec<(u16, String)> = rows.iter().filter(|e| e.status != "missing").map(|e| (e.ordinal, e.name.clone())).collect();
    want.sort();
    let got = xone_exports::pe_exports(&std::fs::read(built("kerneltracecontrol.dll")).unwrap()).unwrap();
    assert_eq!(got, want);
    assert!(got.iter().any(|(o, n)| *o == 1 && n == "CreateMergedTraceFile"));
}

extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut core::ffi::c_void;
    fn GetProcAddress(h: *mut core::ffi::c_void, name: *const u8) -> *mut core::ffi::c_void;
}

fn wide(s: &std::path::Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[test]
fn built_dll_forwards_create_merged_trace_file_to_the_console_image() {
    let dumped = std::fs::read_dir(r"D:\xbox-pc-port\games").map(|r| r.flatten().any(|t| {
        let e = t.path().join("extracted");
        e.join(r"EmbeddedXvd\Windows\System32\KernelTraceControl.dll").is_file()
            || std::fs::read_dir(&e).map(|i| i.flatten().any(|x| x.path().join(r"EmbeddedXvd\Windows\System32\KernelTraceControl.dll").is_file())).unwrap_or(false)
    })).unwrap_or(false);
    let dll = built("kerneltracecontrol.dll");
    let h = unsafe { LoadLibraryW(wide(&dll).as_ptr()) };
    assert!(!h.is_null());
    let p = unsafe { GetProcAddress(h, b"CreateMergedTraceFile\0".as_ptr()) };
    assert!(!p.is_null());
    let f: unsafe extern "system" fn(*const u16, *const *const u16, u32, u32) -> u32 = unsafe { core::mem::transmute(p) };
    let dir = std::env::temp_dir().join(format!("tombox-ktc-dll-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = wide(&dir.join("out.etl"));
    let missing = wide(&dir.join("absent.etl"));
    let inputs = [missing.as_ptr()];
    let r = unsafe { f(out.as_ptr(), inputs.as_ptr(), 1, 0) };
    assert_eq!(r, if dumped { 2 } else { 0x507 });
    let _ = std::fs::remove_dir_all(&dir);
}
