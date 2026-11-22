use compat_common::gdk::{self, package, Guid};
use core::ffi::c_void;
use std::ffi::CString;

unsafe fn method(clsid: Guid, iid: Guid, slot: usize) -> (*mut c_void, usize) {
    let mut obj: *mut c_void = core::ptr::null_mut();
    assert_eq!(gdk::query_api_impl(&clsid, &iid, &mut obj), 0);
    (obj, *(*(obj as *const *const usize)).add(slot))
}

#[repr(C)]
struct Sel {
    kind: u32,
    value: usize,
}

#[repr(C)]
struct Block {
    queue: usize,
    context: usize,
    callback: usize,
    internal: [u8; 32],
}

const LAYOUT: &str = r#"<Package>
  <Chunk Id="Registration"><FileGroup Include="appxmanifest.xml" SourcePath="." DestinationPath="." /></Chunk>
  <Chunk Id="2" Marker="Launch">
    <FileGroup Include="Game.exe" SourcePath="." DestinationPath="." />
    <FileGroup Include="pak0.pak" SourcePath=".\Content\Paks" DestinationPath="\Content\Paks" />
  </Chunk>
  <Chunk Id="3" OnDemand="true" Tags="StW"><FileGroup Include="*.stw" SourcePath=".\Content\StW" DestinationPath="\Content\StW" /></Chunk>
  <Chunk Id="4" Languages="de-DE"><FileGroup Include="pak2.pak" SourcePath=".\Content\Paks" DestinationPath="\Content\Paks" /></Chunk>
</Package>"#;

static SEEN: std::sync::Mutex<Vec<(u32, u32, u32)>> = std::sync::Mutex::new(Vec::new());

unsafe extern "system" fn on_chunk(_ctx: *mut c_void, s: *const Sel, avail: u32) -> bool {
    SEEN.lock().unwrap().push(((*s).kind, if (*s).kind == 2 { (*s).value as u32 } else { 0 }, avail));
    true
}

fn rd(p: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(p[i..i + 8].try_into().unwrap())
}

#[test]
fn fully_installed_title_answers_like_a_console() {
    if let Ok(dir) = std::env::var("TOMBOX_TITLE_DIR") {
        check(std::path::Path::new(&dir));
        return;
    }
    let d = std::env::temp_dir().join(format!("tombox_pkg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("Content").join("Paks")).unwrap();
    std::fs::create_dir_all(d.join("Content").join("StW")).unwrap();
    std::fs::write(d.join("MicrosoftGame.config"), "<Game><Identity Name=\"Pub.Pkg\" Publisher=\"CN=X\" Version=\"1.0.0.0\" /><TitleId>12345678</TitleId></Game>").unwrap();
    std::fs::write(d.join("layout_x.xml"), LAYOUT).unwrap();
    std::fs::write(d.join("Game.exe"), vec![0u8; 1000]).unwrap();
    std::fs::write(d.join("Content").join("Paks").join("pak0.pak"), vec![0u8; 3000]).unwrap();
    std::fs::write(d.join("Content").join("Paks").join("pak2.pak"), vec![0u8; 500]).unwrap();
    std::fs::write(d.join("Content").join("StW").join("a.stw"), vec![0u8; 70]).unwrap();
    std::fs::write(d.join("Content").join("StW").join("b.stw"), vec![0u8; 30]).unwrap();
    std::fs::write(d.join("Content").join("StW").join("c.txt"), vec![0u8; 9]).unwrap();
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["fully_installed_title_answers_like_a_console", "--exact", "--nocapture", "--test-threads=1"])
        .env("TOMBOX_TITLE_DIR", &d)
        .env("TOMBOX_GDK_ROOT", d.join("gdkroot"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    let _ = std::fs::remove_dir_all(&d);
    assert!(out.status.success() && text.contains("1 passed"), "{text}");
}

fn check(d: &std::path::Path) {
    assert_eq!(gdk::title().dir.as_deref(), Some(d));

    let l = package::load_layout(d);
    assert_eq!(l.chunks, vec![2, 3, 4]);
    assert!(l.entries[0].launch && l.entries[1].on_demand);
    assert_eq!(l.chunk_bytes(d, 2), 4000);
    assert_eq!(l.chunk_bytes(d, 3), 100);
    assert_eq!(l.chunk_bytes(d, 4), 500);

    unsafe {
        let (o, getid) = method(package::CLSID, package::IID_V1, 3);
        let getid: unsafe extern "system" fn(*mut c_void, usize, *mut u8) -> i32 = core::mem::transmute(getid);
        let mut idb = [0u8; 64];
        assert_eq!(getid(o, idb.len(), idb.as_mut_ptr()), 0);
        let id = CString::from_vec_with_nul(idb[..idb.iter().position(|&c| c == 0).unwrap() + 1].to_vec()).unwrap();
        assert_eq!(id.to_str().unwrap(), "Pub.Pkg");
        let other = CString::new("Other.Pkg").unwrap();

        let (_, create) = method(package::CLSID, package::IID_V1, 5);
        let create: unsafe extern "system" fn(*mut c_void, *const i8, u32, *const Sel, u32, usize, *mut usize) -> i32 = core::mem::transmute(create);
        let (_, progress) = method(package::CLSID, package::IID_V1, 7);
        let progress: unsafe extern "system" fn(*mut c_void, usize, *mut u8) = core::mem::transmute(progress);
        let (_, close) = method(package::CLSID, package::IID_V1, 6);
        let close: unsafe extern "system" fn(*mut c_void, usize) = core::mem::transmute(close);
        let mut m = 0usize;
        assert_eq!(create(o, id.as_ptr(), 0, core::ptr::null(), 1000, 0, &mut m), 0);
        let mut p = [0u8; 32];
        progress(o, m, p.as_mut_ptr());
        assert_eq!((rd(&p, 0), rd(&p, 8), rd(&p, 16), p[24], p[25]), (4600, 4600, 4000, 1, 1));
        close(o, m);
        assert_eq!(create(o, other.as_ptr(), 0, core::ptr::null(), 1000, 0, &mut m), gdk::E_NOT_FOUND);
        let de = CString::new("de-DE").unwrap();
        let sel = [Sel { kind: 0, value: de.as_ptr() as usize }];
        assert_eq!(create(o, id.as_ptr(), 1, sel.as_ptr(), 1000, 0, &mut m), 0);
        progress(o, m, p.as_mut_ptr());
        assert_eq!((rd(&p, 0), rd(&p, 8)), (500, 500));
        close(o, m);
        let jp = CString::new("ja-JP").unwrap();
        m = 0x55;
        assert_eq!(create(o, id.as_ptr(), 1, [Sel { kind: 0, value: jp.as_ptr() as usize }].as_ptr(), 1000, 0, &mut m), package::E_GAMEPACKAGE_INVALID_SELECTOR);
        assert_eq!(create(o, id.as_ptr(), 1, [Sel { kind: 2, value: 77 }].as_ptr(), 1000, 0, &mut m), package::E_GAMEPACKAGE_INVALID_SELECTOR);

        let (_, find) = method(package::CLSID, package::IID_V1, 12);
        let find: unsafe extern "system" fn(*mut c_void, *const i8, u32, *const Sel, *mut u32) -> i32 = core::mem::transmute(find);
        let mut a = 99u32;
        assert_eq!(find(o, id.as_ptr(), 1, [Sel { kind: 2, value: 3 }].as_ptr(), &mut a), 0);
        assert_eq!(a, package::READY);
        let stw = CString::new("StW").unwrap();
        assert_eq!(find(o, id.as_ptr(), 1, [Sel { kind: 1, value: stw.as_ptr() as usize }].as_ptr(), &mut a), 0);
        assert_eq!(a, package::READY);
        let it = CString::new("it-IT").unwrap();
        a = 99;
        assert_eq!(find(o, id.as_ptr(), 1, [Sel { kind: 0, value: it.as_ptr() as usize }].as_ptr(), &mut a), package::E_GAMEPACKAGE_INVALID_SELECTOR);
        assert_eq!(a, package::UNAVAILABLE);
        assert_eq!(package::UNAVAILABLE, 3);
        let other_pkg = CString::new("Not.This.Package").unwrap();
        a = 99;
        assert_eq!(find(o, other_pkg.as_ptr(), 1, [Sel { kind: 2, value: 3 }].as_ptr(), &mut a), gdk::E_NOT_FOUND);
        assert_eq!(a, package::UNAVAILABLE);
        assert_eq!(package::FIND_SWALLOWED, [0x8007_0002, 0x8007_0570, 0x8007_01e7]);
        assert_eq!(find(o, id.as_ptr(), 0, core::ptr::null(), &mut a), gdk::E_INVALIDARG);
        assert_eq!(find(o, id.as_ptr(), 1, [Sel { kind: 9, value: 0 }].as_ptr(), &mut a), gdk::E_INVALIDARG);

        let (_, en) = method(package::CLSID, package::IID_V1, 13);
        let en: unsafe extern "system" fn(*mut c_void, *const i8, u32, *mut c_void, usize) -> i32 = core::mem::transmute(en);
        assert_eq!(en(o, id.as_ptr(), 2, core::ptr::null_mut(), on_chunk as usize), 0);
        assert_eq!(en(o, id.as_ptr(), 0, core::ptr::null_mut(), on_chunk as usize), 0);
        assert_eq!(*SEEN.lock().unwrap(), vec![(2, 2, 0), (2, 3, 0), (2, 4, 0), (0, 0, 0)]);

        let (_, order) = method(package::CLSID, package::IID_V1, 14);
        let order: unsafe extern "system" fn(*mut c_void, *const i8, u32, *const Sel) -> i32 = core::mem::transmute(order);
        assert_eq!(order(o, id.as_ptr(), 1, [Sel { kind: 2, value: 4 }].as_ptr()), 0);
        assert_eq!(order(o, id.as_ptr(), 1, [Sel { kind: 2, value: 77 }].as_ptr()), package::E_GAMEPACKAGE_INVALID_SELECTOR);

        let (_, install) = method(package::CLSID, package::IID_V1, 16);
        let install: unsafe extern "system" fn(*mut c_void, *const i8, u32, *const Sel, u32, bool, *mut Block) -> i32 = core::mem::transmute(install);
        let (xa, status) = method(gdk::xasync::XASYNC, gdk::xasync::XASYNC, 3);
        let status: unsafe extern "system" fn(*mut c_void, *mut Block, bool) -> i32 = core::mem::transmute(status);
        let (_, result) = method(package::CLSID, package::IID_V1, 17);
        let result: unsafe extern "system" fn(*mut c_void, *mut Block, *mut usize) -> i32 = core::mem::transmute(result);
        let mut b = Box::new(Block { queue: 0, context: 0, callback: 0, internal: [0; 32] });
        assert_eq!(install(o, id.as_ptr(), 1, [Sel { kind: 2, value: 77 }].as_ptr(), 1000, false, &mut *b), package::E_GAMEPACKAGE_INVALID_SELECTOR);
        assert_eq!(install(o, id.as_ptr(), 1, [Sel { kind: 1, value: stw.as_ptr() as usize }].as_ptr(), 1000, false, &mut *b), 0);
        assert_eq!(status(xa, &mut *b, true), 0);
        let mut im = 0usize;
        assert_eq!(result(o, &mut *b, &mut im), 0);
        progress(o, im, p.as_mut_ptr());
        assert_eq!((rd(&p, 0), rd(&p, 8), p[24], p[25]), (100, 100, 1, 1));
        close(o, im);
    }
}
