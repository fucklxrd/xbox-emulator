use super::oracle_dec::{copy, defined, smear};
use super::oracle_enc::{console_encode, console_encode_at};
use crate::dcc_oracle::Con;
use crate::oracle::{oracle, SERIAL};

#[inline(never)]
fn drive_regs(c: &Con, v: u64) -> (Vec<u8>, u8) {
    let f = c.h + 0x26220;
    let obj = c.p();
    unsafe {
        core::arch::asm!(
            "push rbx", "push rsi", "push rdi", "push r12", "push r13", "push r14", "push r15", "push rbp",
            "mov rbp, rsp", "and rsp, -16", "sub rsp, 32",
            "mov rbx, rdx", "mov rsi, rdx", "mov rdi, rdx", "mov r12, rdx", "mov r13, rdx", "mov r14, rdx", "mov r15, rdx",
            "call rax",
            "mov rsp, rbp",
            "pop rbp", "pop r15", "pop r14", "pop r13", "pop r12", "pop rdi", "pop rsi", "pop rbx",
            inout("rax") f => _, inout("rcx") obj => _, inout("rdx") v => _, clobber_abi("system"),
        );
    }
    let p = c.pull();
    (p.b[1].to_vec(), p.r8(0x58))
}

#[inline(never)]
fn drive_deep(c: &Con, fill: u8) -> (Vec<u8>, u8) {
    let mut pad = [0u8; 0x3000];
    for x in pad.iter_mut() {
        unsafe { core::ptr::write_volatile(x, fill) };
    }
    c.call1(0x26220);
    let p = c.pull();
    unsafe { core::ptr::read_volatile(&pad[0x1777]) };
    (p.b[1].to_vec(), p.r8(0x58))
}

fn hexb(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()).collect()
}

#[test]
fn verify_enc_rejected_cases_console_stability() {
    let Ok(path) = std::env::var("TOMBOX_DCC_VERIFY_CASES") else { return };
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    let text = std::fs::read_to_string(path).unwrap();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let w: Vec<&str> = line.split_whitespace().collect();
        let v: Vec<u32> = w[..6].iter().map(|x| x.parse().unwrap()).collect();
        let (f, sw, dim, p0, p1, cnt) = (v[0], v[1], v[2], v[3], v[4], v[5] as usize);
        let src = hexb(w[6]);
        let mut nd = vec![0u8; 256 * cnt];
        let mut nm = vec![0u8; cnt];
        let nr = crate::dcc::encode(cnt as u32, &src, &mut nd, &mut nm, None, f, sw, dim, p0, p1);
        let mut runs = Vec::new();
        for (fl, sm) in [(0xaau8, 0xa5u8), (0x55, 0x5a), (0x00, 0x00), (0xff, 0xff), (0x13, 0x31), (0xc3, 0x3c)] {
            runs.push(console_encode(&o, &src, cnt, f, sw, dim, p0, p1, fl, sm));
        }
        for sh in [8usize, 0x18, 0x1000 + 0x28, 0x10000 + 0x30, 0x40, 0x88] {
            runs.push(console_encode_at(&o, &src, cnt, f, sw, dim, p0, p1, 0xaa, 0xa5, sh));
        }
        let export_stable = runs.iter().all(|x| *x == runs[0]);
        let mut block_unstable = Vec::new();
        if let Some(c) = Con::new(&o, f, sw, dim, p0, p1) {
            let base = defined(c.pull());
            for i in 0..cnt {
                let mut s = copy(&base);
                s.b[0].copy_from_slice(&src[i * 256..i * 256 + 256]);
                s.w64(0x13c0, 0);
                s.w32(0xf0, 0);
                let mut outs = Vec::new();
                for (k, val) in [(0u8, 0x1111_2222_3333_4444u64), (1, 3), (2, 0), (3, u64::MAX), (4, 0x8000_0000_0000_0001)] {
                    c.push(&s);
                    let _ = smear(0x77 ^ k);
                    outs.push(drive_regs(&c, val));
                }
                for fill in [0x00u8, 0xff, 0x5a, 0xa5, 0x3c] {
                    c.push(&s);
                    let _ = smear(fill);
                    outs.push(drive_deep(&c, fill));
                }
                if outs.iter().any(|x| *x != outs[0]) {
                    block_unstable.push(i);
                }
            }
        }
        println!("case f={} sw={} dim={} p0={} p1={} cnt={}: native hr={:#x} console hr={:#x} export_stable={} driver_unstable_blocks={:?} console_meta={:02x?}", f, sw, dim, p0, p1, cnt, nr as u32, runs[0].0 as u32, export_stable, block_unstable, runs[0].2);
    }
}

extern "system" {
    fn VirtualProtect(addr: *mut core::ffi::c_void, size: usize, prot: u32, old: *mut u32) -> i32;
}

static REAL_VSPRINTF: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static PAD: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
type Vsp = unsafe extern "system" fn(u64, *mut u8, usize, *const u8, usize, usize) -> i32;

#[inline(never)]
unsafe extern "system" fn deep_vsprintf(o: u64, b: *mut u8, n: usize, f: *const u8, l: usize, a: usize) -> i32 {
    let mut pad = [0u8; 0x700];
    let v = PAD.load(std::sync::atomic::Ordering::SeqCst);
    for (i, x) in pad.iter_mut().enumerate() {
        let w = if v & 1 != 0 { v ^ (i as u8).wrapping_mul(0x3b) } else { v };
        core::ptr::write_volatile(x, w);
    }
    let real: Vsp = core::mem::transmute(REAL_VSPRINTF.load(std::sync::atomic::Ordering::SeqCst));
    let r = real(o, b, n, f, l, a);
    core::ptr::read_volatile(&pad[0x345]);
    r
}

fn with_deep_crt<R>(h: usize, pad: u8, f: impl FnOnce() -> R) -> R {
    let t = (h + 0x4e980) as *const u8;
    let slot = unsafe {
        assert_eq!((*t, *t.add(1)), (0xff, 0x25));
        let disp = core::ptr::read_unaligned(t.add(2) as *const i32);
        (h as isize + 0x4e986 + disp as isize) as *mut usize
    };
    let mut old = 0u32;
    unsafe {
        assert!(VirtualProtect(slot as *mut _, 8, 0x04, &mut old) != 0);
        REAL_VSPRINTF.store(*slot, std::sync::atomic::Ordering::SeqCst);
        PAD.store(pad, std::sync::atomic::Ordering::SeqCst);
        *slot = deep_vsprintf as usize;
    }
    let r = f();
    unsafe {
        *slot = REAL_VSPRINTF.load(std::sync::atomic::Ordering::SeqCst);
        let mut o2 = 0u32;
        VirtualProtect(slot as *mut _, 8, old, &mut o2);
    }
    r
}

#[test]
fn verify_enc_stable_rejections_per_block() {
    let Ok(path) = std::env::var("TOMBOX_DCC_VERIFY_BLOCKS") else { return };
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    let text = std::fs::read_to_string(path).unwrap();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let w: Vec<&str> = line.split_whitespace().collect();
        let v: Vec<u32> = w[..6].iter().map(|x| x.parse().unwrap()).collect();
        let (f, sw, dim, p0, p1, cnt) = (v[0], v[1], v[2], v[3], v[4], v[5] as usize);
        let src = hexb(w[6]);
        let c = Con::new(&o, f, sw, dim, p0, p1).unwrap();
        let base = defined(c.pull());
        for i in 0..cnt {
            let blk = &src[i * 256..i * 256 + 256];
            let mut nd = [0u8; 256];
            let mut nm = [0u8; 1];
            let nr = crate::dcc::encode(1, blk, &mut nd, &mut nm, None, f, sw, dim, p0, p1);
            let mut s = copy(&base);
            s.b[0].copy_from_slice(blk);
            s.w64(0x13c0, 0);
            s.w32(0xf0, 0);
            let mut outs = Vec::new();
            for k in [0x11u8, 0xee, 0x00, 0x01, 0x80, 0x7f, 0x02, 0x40, 0x23, 0xc5, 0xfe, 0x5a, 0xaa, 0xf0, 0x0e, 0xff] {
                outs.push(with_deep_crt(o.h, k, || {
                    c.push(&s);
                    let _ = smear(k);
                    c.call1(0x26220);
                    let p = c.pull();
                    (p.b[1].to_vec(), p.r8(0x58))
                }));
            }
            for val in [0u64, 3, u64::MAX, 0x1111_2222_3333_4444] {
                c.push(&s);
                let _ = smear(val as u8);
                outs.push(drive_regs(&c, val));
            }
            let stable = outs.iter().all(|x| *x == outs[0]);
            let console_matches_native_when_ok = nr == 0 && outs[0].0 == nd.to_vec() && outs[0].1 == nm[0];
            println!("block f={} sw={} dim={} p0={} p1={} #{}: native hr={:#x} console stable={} meta={:#04x} native_meta={:#04x} same_as_native={} bpe={} s0={}", f, sw, dim, p0, p1, i, nr as u32, stable, outs[0].1, nm[0], console_matches_native_when_ok, base.r32(4), base.r32(0));
            if nr != 0 && stable {
                println!("  STABLE_REJECT block src={}", blk.iter().map(|b| format!("{:02x}", b)).collect::<String>());
            }
        }
    }
}