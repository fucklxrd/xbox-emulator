use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{read_u64, CloseHandle, Ctx, GetThreadContext, OpenThread, ReadProcessMemory, HANDLE};

const BINNED_STATIC_RVA: u64 = 0x8df_9b08;
const GMALLOC_RVA: u64 = 0x8e2_2480;
const BINNED_VTBL_RVA: u64 = 0x624_c568;
const FREE_RET_RVA: u64 = 0x4e_55f9;
const TABLES: u64 = 0x2d;
const PAGE: u64 = 0x1_0000;

#[repr(C)]
#[derive(Default)]
struct Mbi {
    base: u64,
    alloc_base: u64,
    alloc_protect: u32,
    partition: u32,
    size: u64,
    state: u32,
    protect: u32,
    kind: u32,
    pad: u32,
}

extern "system" {
    fn VirtualQueryEx(h: HANDLE, addr: *const c_void, mbi: *mut Mbi, len: usize) -> usize;
}

static DONE: AtomicBool = AtomicBool::new(false);

fn read(h: HANDLE, addr: u64, n: usize) -> Option<Vec<u8>> {
    let mut b = vec![0u8; n];
    let mut got = 0usize;
    let ok = unsafe { ReadProcessMemory(h, addr as *const c_void, b.as_mut_ptr(), n, &mut got) };
    if ok != 0 && got == n { Some(b) } else { None }
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn query(h: HANDLE, addr: u64) -> Option<Mbi> {
    let mut m = Mbi::default();
    let n = unsafe { VirtualQueryEx(h, addr as *const c_void, &mut m, core::mem::size_of::<Mbi>()) };
    if n == 0 { None } else { Some(m) }
}

fn describe(h: HANDLE, addr: u64) -> String {
    match query(h, addr) {
        Some(m) => format!("base={:#x} alloc_base={:#x} size={:#x} state={:#x} protect={:#x} type={:#x}", m.base, m.alloc_base, m.size, m.state, m.protect, m.kind),
        None => "VirtualQueryEx failed".into(),
    }
}

fn freed_pointer(h: HANDLE, tid: u32, exe: u64) -> Option<u64> {
    let ht = unsafe { OpenThread(0x0008 | 0x0040, 0, tid) };
    if ht == 0 {
        return None;
    }
    let mut ctx = Ctx([0u8; 1256]);
    ctx.0[0x30..0x34].copy_from_slice(&0x0010_0001u32.to_le_bytes());
    let ok = unsafe { GetThreadContext(ht, ctx.0.as_mut_ptr()) };
    unsafe { CloseHandle(ht) };
    if ok == 0 {
        return None;
    }
    let rsp = u64_at(&ctx.0, 0x98);
    let n = 0x8000usize;
    let mut buf = vec![0u8; n];
    let mut got = 0usize;
    unsafe { ReadProcessMemory(h, rsp as *const c_void, buf.as_mut_ptr(), n, &mut got) };
    let want = exe + FREE_RET_RVA;
    let mut i = 8;
    while i + 8 <= got {
        if u64_at(&buf, i) == want {
            return Some(u64_at(&buf, i - 8));
        }
        i += 8;
    }
    None
}

fn walk(h: HANDLE, head: u64, cap: usize) -> (usize, u64) {
    let (mut n, mut taken, mut p) = (0usize, 0u64, head);
    while p != 0 && n < cap {
        let Some(b) = read(h, p, 0x20) else { break };
        taken += u16::from_le_bytes([b[0], b[1]]) as u64;
        n += 1;
        p = u64_at(&b, 0x10);
    }
    (n, taken)
}

fn rtti_name(h: HANDLE, exe: u64, vt: u64) -> String {
    let col = match read_u64(h, vt - 8) { Some(c) if c >= exe && c < exe + 0xa00_0000 => c, _ => return String::new() };
    let Some(b) = read(h, col, 0x18) else { return String::new() };
    if u32::from_le_bytes(b[0..4].try_into().unwrap()) != 1 {
        return String::new();
    }
    let td = exe + u32::from_le_bytes(b[0xc..0x10].try_into().unwrap()) as u64;
    let Some(nm) = read(h, td + 0x10, 96) else { return String::new() };
    let end = nm.iter().position(|&c| c == 0).unwrap_or(nm.len());
    String::from_utf8_lossy(&nm[..end]).into_owned()
}

pub fn on_ods(h: HANDLE, tid: u32, exe: u64, threads: usize, s: &str, modules: &BTreeMap<u64, String>) {
    if !s.contains("Corruption Canary") || DONE.swap(true, Ordering::AcqRel) {
        return;
    }
    let exe_name = modules.get(&exe).cloned().unwrap_or_default();
    if !exe_name.to_ascii_lowercase().starts_with("fortniteclient-xsx") {
        return;
    }
    let t0 = std::time::Instant::now();
    let gm = read_u64(h, exe + GMALLOC_RVA).unwrap_or(0);
    let b = read_u64(h, exe + BINNED_STATIC_RVA).unwrap_or(0);
    let vt = read_u64(h, b).unwrap_or(0);
    eprintln!("[HEAP] census at first canary failure: tid={tid} threads={threads} GMalloc={gm:#x} (vtbl rva {:#x}) binned2={b:#x} vtbl rva {:#x} expected {BINNED_VTBL_RVA:#x}",
        read_u64(h, gm).unwrap_or(0).wrapping_sub(exe), vt.wrapping_sub(exe));
    if b == 0 || vt != exe + BINNED_VTBL_RVA {
        eprintln!("[HEAP] binned2 instance not found; census skipped");
        return;
    }
    let Some(pa) = read(h, b + 0x478, 0x90) else { eprintln!("[HEAP] page allocator unreadable"); return };
    let (base, small_end, end) = (u64_at(&pa, 8), u64_at(&pa, 0x10), u64_at(&pa, 0x18));
    eprintln!("[HEAP] page allocator: enabled={} base={base:#x} small_end={small_end:#x} end={end:#x} committed={:#x} os_base={:#x}", pa[0], u64_at(&pa, 0x20), u64_at(&pa, 0x28));
    eprintln!("[HEAP] chunk lists: uncommitted[0..3]={:#x} {:#x} {:#x} partial[0..3]={:#x} {:#x} {:#x} full[0..3]={:#x} {:#x} {:#x}",
        u64_at(&pa, 0x40), u64_at(&pa, 0x48), u64_at(&pa, 0x50), u64_at(&pa, 0x70), u64_at(&pa, 0x78), u64_at(&pa, 0x80), u64_at(&pa, 0x58), u64_at(&pa, 0x60), u64_at(&pa, 0x68));
    if let Some(p) = freed_pointer(h, tid, exe) {
        let inside = p >= base && p < small_end;
        eprintln!("[HEAP] freed pointer {p:#x}: in small range={inside} page {}", describe(h, p));
        if let Some(hdr) = read(h, p & !(PAGE - 1), 16) {
            eprintln!("[HEAP] freed pointer page header: block={:#x} pool={} canary={:#x} free={}", u16::from_le_bytes([hdr[0], hdr[1]]), hdr[2], hdr[3], u32::from_le_bytes(hdr[4..8].try_into().unwrap()));
        }
    } else {
        eprintln!("[HEAP] freed pointer not found on the faulting thread stack");
    }
    let mut total_pools = 0usize;
    let mut sizes = Vec::new();
    for i in 0..TABLES {
        let Some(t) = read(h, b + 0x28 + i * 0x18, 0x18) else { continue };
        let bs = u32::from_le_bytes(t[0x10..0x14].try_into().unwrap()) as u64;
        let (na, ta) = walk(h, u64_at(&t, 0), 200_000);
        let (ne, te) = walk(h, u64_at(&t, 8), 200_000);
        total_pools += na + ne;
        sizes.push((i, bs));
        if na + ne != 0 {
            eprintln!("[HEAP] table {i:2} block={bs:#6x}: active pools={na} exhausted pools={ne} taken blocks={} live bytes={:#x}", ta + te, (ta + te) * bs);
        }
    }
    eprintln!("[HEAP] small pools listed: {total_pools} ({:#x} bytes of 64 KiB pages); small range holds {} pages", total_pools as u64 * PAGE, (small_end - base) / PAGE);
    let mut per: BTreeMap<u8, Vec<u64>> = BTreeMap::new();
    let (mut committed, mut headed) = (0u64, 0u64);
    let mut a = base;
    while a < small_end {
        match query(h, a) {
            Some(m) if m.size != 0 => {
                let hi = (m.base + m.size).min(small_end);
                if m.state == 0x1000 {
                    let mut p = a;
                    while p < hi {
                        committed += 1;
                        if let Some(hdr) = read(h, p, 4) {
                            if hdr[3] == 0xe3 {
                                headed += 1;
                                per.entry(hdr[2]).or_default().push(p);
                            }
                        }
                        p += PAGE;
                    }
                }
                a = hi.max(a + PAGE);
            }
            _ => a += PAGE,
        }
    }
    eprintln!("[HEAP] small range scan: committed pages={committed} with pool headers={headed}");
    let mut by: Vec<(u8, usize)> = per.iter().map(|(k, v)| (*k, v.len())).collect();
    by.sort_by(|x, y| y.1.cmp(&x.1));
    for (idx, n) in by.iter().take(8) {
        let bs = sizes.iter().find(|s| s.0 == *idx as u64).map(|s| s.1).unwrap_or(0);
        eprintln!("[HEAP] pool index {idx} block={bs:#x}: {n} pages ({:#x} bytes)", *n as u64 * PAGE);
    }
    for (idx, n) in by.iter().take(3) {
        let bs = sizes.iter().find(|s| s.0 == *idx as u64).map(|s| s.1).unwrap_or(0);
        if bs < 16 {
            continue;
        }
        let pages = &per[idx];
        let step = (pages.len() / 48).max(1);
        let mut vt: BTreeMap<u64, usize> = BTreeMap::new();
        let mut txt: BTreeMap<String, usize> = BTreeMap::new();
        let mut blocks = 0usize;
        for p in pages.iter().step_by(step).take(48) {
            let Some(pg) = read(h, *p, PAGE as usize) else { continue };
            let mut k = 1usize;
            while PAGE as usize >= k * bs as usize + 16 {
                let o = PAGE as usize - k * bs as usize;
                k += 1;
                let v = u64_at(&pg, o);
                blocks += 1;
                if v >= exe && v < exe + 0xa00_0000 {
                    *vt.entry(v).or_default() += 1;
                } else {
                    let s: String = pg[o..o + 16.min(bs as usize)].iter().map(|&c| if (0x20..0x7f).contains(&c) { c as char } else { '.' }).collect();
                    if s.chars().filter(|c| *c != '.').count() >= 6 {
                        *txt.entry(s).or_default() += 1;
                    }
                }
            }
        }
        let mut top: Vec<(u64, usize)> = vt.into_iter().collect();
        top.sort_by(|x, y| y.1.cmp(&x.1));
        eprintln!("[HEAP] pool index {idx} (block {bs:#x}, {n} pages): sampled {blocks} blocks; first-qword EXE pointers:");
        for (v, c) in top.iter().take(12) {
            eprintln!("[HEAP]    {c:6}  EXE+{:#x} {}", v - exe, rtti_name(h, exe, *v));
        }
        let mut tt: Vec<(String, usize)> = txt.into_iter().collect();
        tt.sort_by(|x, y| y.1.cmp(&x.1));
        for (s, c) in tt.iter().take(8) {
            eprintln!("[HEAP]    {c:6}  text {s:?}");
        }
    }
    eprintln!("[HEAP] census took {:.1} ms", t0.elapsed().as_secs_f64() * 1e3);
}
