use crate::merge::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_INVALID_PARAMETER: u32 = 0x57;
const MODE_EVENT_RECORD: u32 = 0x1000_0000;
const LOGFILE_SIZE: usize = 448;
const INVALID_TRACE: u64 = u64::MAX;

#[link(name = "advapi32")]
extern "system" {
    fn OpenTraceW(logfile: *mut u8) -> u64;
    fn ProcessTrace(h: *const u64, n: u32, start: *const i64, end: *const i64) -> u32;
    fn CloseTrace(h: u64) -> u32;
}

thread_local! {
    static SEEN: RefCell<HashMap<[u8; 16], u32>> = RefCell::new(HashMap::new());
}

unsafe extern "system" fn on_event(rec: *const u8) {
    let mut g = [0u8; 16];
    core::ptr::copy_nonoverlapping(rec.add(0x18), g.as_mut_ptr(), 16);
    SEEN.with(|s| *s.borrow_mut().entry(g).or_insert(0) += 1);
}

fn wide(s: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn events(path: &Path) -> Option<HashMap<[u8; 16], u32>> {
    let name = wide(path);
    let mut lf = vec![0u64; LOGFILE_SIZE / 8];
    let b = lf.as_mut_ptr() as *mut u8;
    unsafe {
        (b as *mut *const u16).write(name.as_ptr());
        (b.add(28) as *mut u32).write(MODE_EVENT_RECORD);
        (b.add(424) as *mut usize).write(on_event as *const () as usize);
        SEEN.with(|s| s.borrow_mut().clear());
        let h = OpenTraceW(b);
        if h == INVALID_TRACE {
            return None;
        }
        let r = ProcessTrace(&h, 1, core::ptr::null(), core::ptr::null());
        CloseTrace(h);
        if r != 0 {
            return None;
        }
    }
    Some(SEEN.with(|s| s.borrow().clone()))
}

fn fixture(tag: &str) -> Option<(PathBuf, PathBuf)> {
    let src = std::fs::read_dir(r"C:\Windows\Logs\SIH").ok()?.flatten().map(|e| e.path()).find(|p| p.extension().map(|x| x.eq_ignore_ascii_case("etl")).unwrap_or(false))?;
    let dir = std::env::temp_dir().join(format!("tombox-ktc-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("in.etl");
    std::fs::copy(&src, &input).ok()?;
    Some((dir, input))
}

fn merge(out: &Path, inputs: &[&Path], flags: u32) -> u32 {
    let o = wide(out);
    let w: Vec<Vec<u16>> = inputs.iter().map(|p| wide(p)).collect();
    let ptrs: Vec<*const u16> = w.iter().map(|v| v.as_ptr()).collect();
    unsafe { CreateMergedTraceFile(o.as_ptr(), ptrs.as_ptr(), ptrs.len() as u32, flags) }
}

fn console_present() -> bool {
    let dumped = console_image::dump_dirs(Path::new(console_image::GAMES)).iter().any(|d| d.join("KernelTraceControl.dll").is_file());
    assert!(!dumped || module() != 0, "console KernelTraceControl.dll is dumped but did not load");
    module() != 0
}

fn total(m: &HashMap<[u8; 16], u32>) -> u32 {
    m.values().sum()
}

#[test]
fn console_merge_keeps_every_input_event() {
    if !console_present() {
        return;
    }
    let Some((dir, input)) = fixture("keep") else { return };
    let Some(src) = events(&input) else { return };
    let out = dir.join("merged.etl");
    assert_eq!(merge(&out, &[&input], 0), 0);
    let got = events(&out).expect("merged ETL opens and processes");
    for (g, n) in &src {
        assert!(got.get(g).copied().unwrap_or(0) >= *n, "provider {g:02x?} lost events");
    }
    assert!(total(&got) >= total(&src));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn console_merge_bit6_adds_rundown_events() {
    if !console_present() {
        return;
    }
    let Some((dir, input)) = fixture("rundown") else { return };
    let plain = dir.join("plain.etl");
    let rundown = dir.join("rundown.etl");
    assert_eq!(merge(&plain, &[&input], 0), 0);
    assert_eq!(merge(&rundown, &[&input], 0x40), 0);
    let a = events(&plain).expect("plain merge opens");
    let b = events(&rundown).expect("rundown merge opens");
    assert!(total(&b) > total(&a), "{} vs {}", total(&b), total(&a));
    assert!(std::fs::metadata(&rundown).unwrap().len() > std::fs::metadata(&plain).unwrap().len());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn console_merge_reports_console_win32_errors() {
    if !console_present() {
        return;
    }
    let Some((dir, input)) = fixture("errors") else { return };
    assert_eq!(merge(&dir.join("o1.etl"), &[&dir.join("absent.etl")], 0), ERROR_FILE_NOT_FOUND);
    assert_eq!(merge(&dir.join("o2.etl"), &[&input], 0x4000_1000), ERROR_INVALID_PARAMETER);
    let w = wide(&input);
    let ptrs = [w.as_ptr()];
    assert_eq!(unsafe { CreateMergedTraceFile(core::ptr::null(), ptrs.as_ptr(), 1, 0) }, ERROR_UNIDENTIFIED_ERROR);
    let _ = std::fs::remove_dir_all(&dir);
}
