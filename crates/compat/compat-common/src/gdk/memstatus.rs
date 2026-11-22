use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> usize;
}

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    fn NtQueryInformationProcess(process: isize, class: u32, info: *mut u8, len: u32, ret: *mut u32) -> i32;
}

const WS_VERSION_2_TITLE: u32 = 0x1000_0001;
const STATUS_SIZE: u32 = 64;
pub const PAGE_SIZE: u64 = 0x1000;
const PROCESS_QUOTA_LIMITS: u32 = 1;
const PROCESS_VM_COUNTERS: u32 = 3;
pub const NO_PAGEFILE_LIMIT: u64 = u64::MAX;

static ORIG_GMSE: AtomicUsize = AtomicUsize::new(0);
static WS_FN: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleView {
    pub total: u64,
    pub used: u64,
    pub commit_limit: u64,
    pub committed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quota {
    pub pagefile_limit: u64,
    pub pagefile_usage: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shaped {
    pub load: u32,
    pub total_phys: u64,
    pub avail_phys: u64,
    pub total_page_file: u64,
    pub avail_page_file: u64,
}

fn working_set_fn() -> usize {
    let f = WS_FN.load(Ordering::Acquire);
    if f != 0 {
        return f;
    }
    let f = unsafe {
        let h = GetModuleHandleA(b"xmem.dll\0".as_ptr());
        if h.is_null() {
            return 0;
        }
        GetProcAddress(h, b"XMemGetWorkingSetStatistics\0".as_ptr())
    };
    WS_FN.store(f, Ordering::Release);
    f
}

pub fn title_view() -> Option<TitleView> {
    let f = working_set_fn();
    if f == 0 {
        return None;
    }
    let mut buf = [0u8; 0x50];
    let ok = unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(u32, *mut u8) -> i32>(f)(WS_VERSION_2_TITLE, buf.as_mut_ptr()) };
    let q = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().unwrap());
    let (total, used, commit_limit, committed) = (q(0), q(8), q(0x28), q(0x30));
    if ok == 0 || total == 0 {
        return None;
    }
    let commit_limit = if commit_limit == 0 { total } else { commit_limit };
    Some(TitleView { total, used: used.min(total), commit_limit, committed: committed.min(commit_limit) })
}

pub fn process_quota() -> Quota {
    let mut ql = [0u8; 0x30];
    let mut vm = [0u8; 0x58];
    let mut r = 0u32;
    let a = unsafe { NtQueryInformationProcess(-1, PROCESS_QUOTA_LIMITS, ql.as_mut_ptr(), ql.len() as u32, &mut r) };
    let b = unsafe { NtQueryInformationProcess(-1, PROCESS_VM_COUNTERS, vm.as_mut_ptr(), vm.len() as u32, &mut r) };
    if a < 0 || b < 0 {
        return Quota { pagefile_limit: NO_PAGEFILE_LIMIT, pagefile_usage: 0 };
    }
    Quota { pagefile_limit: u64::from_le_bytes(ql[0x20..0x28].try_into().unwrap()), pagefile_usage: u64::from_le_bytes(vm[0x48..0x50].try_into().unwrap()) }
}

pub fn compute(v: TitleView, q: Quota) -> Shaped {
    let avail = v.total.saturating_sub(v.used);
    let load = if avail < PAGE_SIZE * 100 { 100 } else { (v.total.saturating_sub(avail) as u128 * 100 / v.total as u128) as u32 };
    let mut total_page_file = v.commit_limit;
    let mut avail_page_file = v.commit_limit.saturating_sub(v.committed);
    if q.pagefile_limit != NO_PAGEFILE_LIMIT {
        if q.pagefile_limit < v.commit_limit / PAGE_SIZE {
            total_page_file = q.pagefile_limit.wrapping_mul(PAGE_SIZE);
        }
        let used_pages = q.pagefile_usage / PAGE_SIZE;
        let left = if q.pagefile_limit > used_pages { (q.pagefile_limit - used_pages).wrapping_mul(PAGE_SIZE) } else { 0 };
        avail_page_file = avail_page_file.min(left);
    }
    Shaped { load, total_phys: v.total, avail_phys: avail, total_page_file, avail_page_file }
}

pub unsafe fn shape(status: *mut u8, s: Shaped) {
    core::ptr::write_unaligned(status.add(4) as *mut u32, s.load);
    core::ptr::write_unaligned(status.add(8) as *mut u64, s.total_phys);
    core::ptr::write_unaligned(status.add(16) as *mut u64, s.avail_phys);
    core::ptr::write_unaligned(status.add(24) as *mut u64, s.total_page_file);
    core::ptr::write_unaligned(status.add(32) as *mut u64, s.avail_page_file);
}

unsafe extern "system" fn global_memory_status_ex_hook(status: *mut u8) -> i32 {
    let f: unsafe extern "system" fn(*mut u8) -> i32 = core::mem::transmute(ORIG_GMSE.load(Ordering::Acquire));
    let r = f(status);
    if r == 0 || status.is_null() || core::ptr::read_unaligned(status as *const u32) < STATUS_SIZE {
        return r;
    }
    if let Some(v) = title_view() {
        let q = process_quota();
        shape(status, compute(v, q));
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| super::log("GlobalMemoryStatusEx", &format!("title partition total {:#x} used {:#x}, commit {:#x}/{:#x}, pagefile limit {:#x}", v.total, v.used, v.committed, v.commit_limit, q.pagefile_limit)));
    }
    r
}

pub fn targets() -> Vec<(&'static str, usize, &'static AtomicUsize)> {
    if super::setting("console_memstatus").as_deref() == Some("0") {
        return Vec::new();
    }
    vec![("GlobalMemoryStatusEx", global_memory_status_ex_hook as usize, &ORIG_GMSE)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNLIMITED: Quota = Quota { pagefile_limit: NO_PAGEFILE_LIMIT, pagefile_usage: 0 };

    fn view(total: u64, used: u64) -> TitleView {
        TitleView { total, used, commit_limit: total, committed: used }
    }

    #[test]
    fn status_reports_title_partition_and_keeps_host_virtual_fields() {
        let mut s = [0xccu8; 64];
        s[0..4].copy_from_slice(&64u32.to_le_bytes());
        let host_virtual = [0x11u8; 24];
        s[40..64].copy_from_slice(&host_virtual);
        unsafe { shape(s.as_mut_ptr(), compute(view(0x3_6000_0000, 0x1_b000_0000), UNLIMITED)) };
        let q = |o: usize| u64::from_le_bytes(s[o..o + 8].try_into().unwrap());
        assert_eq!(u32::from_le_bytes(s[0..4].try_into().unwrap()), 64);
        assert_eq!(u32::from_le_bytes(s[4..8].try_into().unwrap()), 50);
        assert_eq!(q(8), 0x3_6000_0000);
        assert_eq!(q(16), 0x1_b000_0000);
        assert_eq!(q(24), q(8));
        assert_eq!(q(32), q(16));
        assert_eq!(&s[40..64], &host_virtual);
    }

    #[test]
    fn memory_load_truncates_like_console_kernelbase() {
        assert_eq!(compute(view(3000 * PAGE_SIZE, 2000 * PAGE_SIZE), UNLIMITED).load, 66);
        assert_eq!(compute(view(1000 * PAGE_SIZE, 899 * PAGE_SIZE), UNLIMITED).load, 89);
    }

    #[test]
    fn under_one_hundred_free_pages_reports_full_load() {
        let total = 0x3_6000_0000;
        assert_eq!(compute(view(total, total - 99 * PAGE_SIZE), UNLIMITED).load, 100);
        assert_eq!(compute(view(total, total - 100 * PAGE_SIZE), UNLIMITED).load, 99);
        assert_eq!(compute(view(total, total), UNLIMITED).load, 100);
        assert_eq!(compute(view(total, total), UNLIMITED).avail_phys, 0);
    }

    #[test]
    fn page_file_fields_follow_commit_and_job_pagefile_limit() {
        let v = TitleView { total: 0x4000_0000, used: 0x1000_0000, commit_limit: 0x4000_0000, committed: 0x1800_0000 };
        let s = compute(v, UNLIMITED);
        assert_eq!((s.total_page_file, s.avail_page_file), (0x4000_0000, 0x2800_0000));
        let s = compute(v, Quota { pagefile_limit: 0x2_0000, pagefile_usage: 0x1000_0000 });
        assert_eq!(s.total_page_file, 0x2000_0000);
        assert_eq!(s.avail_page_file, 0x1000_0000);
        let s = compute(v, Quota { pagefile_limit: 0x8_0000, pagefile_usage: 0x1000_0000 });
        assert_eq!(s.total_page_file, 0x4000_0000);
        assert_eq!(s.avail_page_file, 0x2800_0000);
        let s = compute(v, Quota { pagefile_limit: 0x1_0000, pagefile_usage: 0x2000_0000 });
        assert_eq!(s.avail_page_file, 0);
    }

    #[test]
    fn host_quota_query_succeeds() {
        let q = process_quota();
        assert!(q.pagefile_usage > 0);
    }

    #[test]
    fn hook_is_offered_for_titles() {
        let names: Vec<&str> = targets().iter().map(|t| t.0).collect();
        assert_eq!(names, ["GlobalMemoryStatusEx"]);
    }
}
