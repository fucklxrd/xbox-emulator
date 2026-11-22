use super::addr::{coord_1b540, index_1c960, mode_1b440, reorder_1dd10};
use super::bits::{key_decode, key_overflows, sector_compressed};
use super::{Dcc, Stop, R};

pub(crate) const RAN_OUT: &str = "Ran out of compressed block data";
pub(crate) const NO_BASE: &str = "The first bin of compressed block must have a base or be constant 0";

pub(crate) fn shl8(v: u8, c: u32) -> u8 {
    let c = c & 31;
    if c >= 8 {
        0
    } else {
        v << c
    }
}

pub(crate) fn shr8(v: u8, c: u32) -> u8 {
    let c = c & 31;
    if c >= 8 {
        0
    } else {
        v >> c
    }
}

pub(crate) fn kd(key: u32) -> R<[u32; 16]> {
    if key_overflows(key) {
        return Err(Stop::Fault("key decode 0x1de40 writes past its 8-dword stack buffer over the /GS cookie"));
    }
    Ok(key_decode(key))
}

fn lowest(v: u32) -> u32 {
    (0..32).find(|&i| v >> i & 1 != 0).unwrap_or(u32::MAX)
}

fn highest(v: u32) -> u32 {
    (0..32).rev().find(|&i| v >> i & 1 != 0).unwrap_or(u32::MAX)
}

pub(crate) fn neighbor(d: &Dcc, e: u32, ord: u32) -> u32 {
    let c = coord_1b540(d, e, ord);
    let pick = |k: usize| {
        if c[k] != 0 {
            let mut t = c;
            t[k] = t[k].wrapping_sub(1);
            index_1c960(d, &t, ord)
        } else {
            e
        }
    };
    let (ax, ay, az) = (pick(0), pick(1), pick(2));
    let mut n = if ax < e { ax } else { e };
    for a in [ay, az] {
        if a < e {
            n = if n == e { a } else { a.max(n) };
        }
    }
    n.min(e)
}

pub(crate) fn split(d: &Dcc) -> R<(u32, bool, u32)> {
    let bpe = d.g(4);
    let v = d.g(0x24).wrapping_sub(1);
    let (q, f) = if v <= 7 {
        (bpe >> 2, false)
    } else if v <= 11 {
        (bpe >> 1, false)
    } else {
        let c = d.g(8);
        if c == 0 {
            return Err(Stop::Fault("integer divide by zero: bytes per element / component count"));
        }
        (bpe / c, d.g(0x10) <= 1)
    };
    Ok((q, f, if f { bpe.wrapping_sub(q) } else { q }))
}

pub(crate) fn shift_of(d: &Dcc, e: u32, q: u32, f: bool, thr: u32) -> R<u32> {
    let w = coord_1b540(d, e, d.g(0xb0))[3];
    let mut al = w >= thr;
    if !f {
        al = !al;
    }
    let sv = if al { d.g(0x2c) } else { d.g(0x30) };
    if q == 0 {
        return Err(Stop::Fault("integer divide by zero: channel byte split"));
    }
    let sh = sv.wrapping_sub((w % q) << 3);
    Ok(if (sh as i32) <= 0 { 0 } else { sh })
}

fn narrow(d: &mut Dcc, mode: bool) -> (u32, u32) {
    let (sb, sm) = (d.g(4), d.g(0));
    let mut c = sb;
    if sb == 16 {
        d.p(4, 8);
        c = 8;
    }
    if mode && sm == 3 {
        d.p(0, (c >= 8) as u32);
    }
    (sb, sm)
}

fn ran_out(d: &mut Dcc, sector: u32, text: &str) {
    d.msg.push(format!("Key={:02x} Sector={}: {}", d.o[0x34], sector as i32, text));
    if d.o[0x418..0x420].iter().all(|&b| b == 0) {
        d.o[0x418] = 1;
    }
}

pub(crate) fn unpack(d: &mut Dcc) -> R<()> {
    d.p(0xb0, 4);
    let (sb, sm) = narrow(d, true);
    let r = unpack_body(d);
    d.p(4, sb);
    d.p(0, sm);
    r
}

fn unpack_body(d: &mut Dcc) -> R<()> {
    let key = d.g(0x34);
    let k0 = kd(key)?;
    let old = d.g(0x38);
    let mut m = 0;
    for i in 0..4 {
        if old >> i & 1 != 0 {
            m |= k0[i];
        }
    }
    d.p(0x38, m);
    for k in 0..4u32 {
        if !d.mask_bit(k) {
            continue;
        }
        let off = (k * 64) as usize;
        let hm = k0[4 + k as usize];
        if !sector_compressed(key, off as u32) {
            let s = d.s[1];
            d.s[0][off..off + 64].copy_from_slice(&s[off..off + 64]);
            continue;
        }
        let lo = k0[k as usize];
        let (lw, hg) = (lowest(lo), highest(lo));
        if lw != k || lw > hg {
            continue;
        }
        if hg > 3 {
            return Err(Stop::Undef("sector mask beyond the block"));
        }
        let mut cur = off as u32;
        for j in lw..=hg {
            let ju = j as usize;
            let v = d.s[1][cur as usize] as u32;
            d.p(0x3e0 + 4 * ju, v);
            let v = d.s[1][cur as usize + 1] as u32;
            d.p(0x344 + 4 * ju, v);
            cur += 2;
            d.s[2][ju * 64..ju * 64 + 64].fill(0);
        }
        let lim = (hg + 1) * 64;
        let avail = |cur: u32| cur < lim && hm >> ((cur >> 5) & 31) & 1 != 0;
        for j in lw..=hg {
            let ju = j as usize;
            d.o[0x180 + ju] = j as u8;
            for i in 0..8u32 {
                let iu = i as usize;
                let ord = d.g(0xb0);
                let esi = (j * 8 + i) * 8;
                let n = neighbor(d, esi, ord);
                let ns = n >> 6;
                let mut r8 = ns != j;
                let al = if ns != j {
                    if d.o[0x180 + ju] as u32 != ns {
                        d.o[0x180 + ju] = ns as u8;
                    }
                    d.o[0x20] == 0
                } else {
                    true
                };
                if esi == n || !(ns >= lw && al) {
                    r8 = false;
                }
                d.put_bit(0x110 + 4 * ju, i, r8)?;
                let bit = 1u32 << i;
                let p304 = 0x304 + ju * 8 + iu;
                let p160 = 0x160 + ju * 8 + iu;
                if d.g(0x344 + 4 * ju) & bit != 0 {
                    d.o[p304] = 0;
                }
                if d.g(0x3e0 + 4 * ju) & bit != 0 {
                    if !avail(cur) {
                        ran_out(d, j, RAN_OUT);
                        return Ok(());
                    }
                    let mut v = d.s[1][cur as usize];
                    cur += 1;
                    if d.g(0x344 + 4 * ju) & bit == 0 {
                        let tz = (0..32u32).find(|&e| shr8(v, e) & 1 != 0).unwrap_or(u32::MAX);
                        let dl = tz.wrapping_add(1) & 0xff;
                        v = shl8(shr8(v, dl), dl);
                        d.o[p304] = dl as u8;
                    }
                    d.o[p160] = v;
                    d.s[2][esi as usize] = v;
                } else if d.g(0x344 + 4 * ju) & bit != 0 {
                    d.o[p160] = 0;
                    d.s[2][esi as usize] = 0;
                } else if r8 {
                    if j == 0 {
                        return Err(Stop::Fault("unpack reads the previous sector's bin widths at sector -1"));
                    }
                    d.o[p304] = d.o[p304 - 8];
                } else if i == 0 {
                    ran_out(d, j, NO_BASE);
                    d.o[p304] = 0;
                    d.o[p160] = 0xee;
                } else {
                    d.o[p304] = d.o[p304 - 1];
                }
            }
        }
        for j in lw..=hg {
            let ju = j as usize;
            for i in 0..8usize {
                let cnt = d.o[0x304 + ju * 8 + i] as i32;
                let mut n = 0i32;
                while n < cnt {
                    if !avail(cur) {
                        ran_out(d, j, RAN_OUT);
                        return Ok(());
                    }
                    let v = d.s[1][cur as usize];
                    cur += 1;
                    let base = (i + ju * 8) * 8;
                    for b in 0..8 {
                        d.s[2][base + b] |= shl8((v >> b) & 1, n as u32);
                    }
                    n += 1;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn tid(d: &mut Dcc) -> R<()> {
    let sb = d.g(4);
    if sb == 16 {
        d.p(4, 8);
    }
    let r = tid_body(d);
    d.p(4, sb);
    r
}

fn tid_body(d: &mut Dcc) -> R<()> {
    reorder_1dd10(d, 2, 2, 3);
    let key = d.g(0x34);
    for q in 0..8u32 {
        if !d.mask_bit(q >> 1) || !sector_compressed(key, q * 32) {
            continue;
        }
        let qu = q as usize;
        for i in 0..8u32 {
            let c = coord_1b540(d, q * 32 + 4 * i, 3);
            let a = index_1c960(d, &c, 4);
            if a & 7 == 0 {
                let e = (a >> 3) & 7;
                let r10 = (a >> 6) as usize;
                if r10 >= 4 {
                    return Err(Stop::Undef("tid swizzle index beyond the block"));
                }
                d.o[0x120 + 8 * qu + i as usize] = d.o[0x160 + r10 * 8 + e as usize];
                let on = d.g(0x3e0 + 4 * r10) >> e & 1 != 0;
                d.put_bit(0x3f0 + 4 * qu, i, on)?;
                let on = d.g(0x110 + 4 * r10) >> e & 1 != 0;
                d.put_bit(0xf0 + 4 * qu, i, on)?;
            } else {
                d.put_bit(0x3f0 + 4 * qu, i, false)?;
                d.put_bit(0xf0 + 4 * qu, i, false)?;
            }
        }
    }
    Ok(())
}

fn flip(v: u8) -> u8 {
    let h = v >> 1;
    if v & 1 != 0 {
        !h
    } else {
        h
    }
}

pub(crate) fn dedelta(d: &mut Dcc) {
    let key = d.g(0x34);
    for k in 0..4u32 {
        if !d.mask_bit(k) || !sector_compressed(key, k * 64) {
            continue;
        }
        for e in k * 64..k * 64 + 64 {
            let skip = e & 3 == 0 && d.bit(0x3f0 + 4 * (e >> 5) as usize, (e >> 2) & 7);
            if !skip {
                d.s[2][e as usize] = flip(d.s[2][e as usize]);
            }
        }
    }
}

pub(crate) fn calc(d: &mut Dcc) -> R<()> {
    let (sb, sm) = narrow(d, true);
    let r = calc_body(d);
    d.p(4, sb);
    d.p(0, sm);
    r
}

fn calc_body(d: &mut Dcc) -> R<()> {
    let (q, f, thr) = split(d)?;
    let key = d.g(0x34);
    for k in 0..4u32 {
        if !d.mask_bit(k) || !sector_compressed(key, k * 64) {
            continue;
        }
        let lo = kd(key)?[k as usize];
        for e in k * 64..k * 64 + 64 {
            let n = neighbor(d, e, d.g(0xb0));
            let ns = n >> 6;
            let mut r10 = ns != k;
            let r9 = !(ns != k && d.o[0x20] != 0);
            let r8 = ns != k && lo >> (ns & 31) & 1 == 0;
            let cl = if r8 { false } else { r9 };
            let go = e != n && cl;
            if !go {
                r10 = false;
            }
            if e & 7 == 0 {
                d.put_bit(0xf0 + 4 * (e >> 5) as usize, (e >> 2) & 7, r10)?;
            }
            if !go {
                continue;
            }
            if e & 3 == 0 && d.bit(0x3f0 + 4 * (e >> 5) as usize, (e >> 2) & 7) {
                continue;
            }
            let mut p = d.s[2][e as usize];
            let rf = d.s[2][n as usize];
            if d.g(0x2c) != 0 || d.g(0x30) != 0 {
                let sh = shift_of(d, e, q, f, thr)?;
                p = shl8(p, sh);
            }
            d.s[2][e as usize] = p.wrapping_add(rf);
        }
    }
    Ok(())
}

pub(crate) fn color_shift(d: &Dcc, q4: u32, idx: u32) -> R<u32> {
    if q4 == 0 {
        return Err(Stop::Fault("integer divide by zero in the color transform: bytes per element / 4"));
    }
    let sh = d.g(0x30).wrapping_sub((idx % q4) << 3);
    Ok(if (sh as i32) <= 0 { 0 } else { sh })
}

pub(crate) fn color(d: &mut Dcc) -> R<()> {
    let m = mode_1b440(d, 0x14);
    if m == 1 {
        return Ok(());
    }
    let q4 = d.g(4) >> 2;
    let key = d.g(0x34);
    for t in 0..8u32 {
        if !d.mask_bit(t >> 1) || !sector_compressed(key, t * 32) {
            continue;
        }
        let (ea, es, er): (u32, u32, u32) = if m == 2 { (8, 8u32.wrapping_neg(), 8) } else { (16, 8, 8u32.wrapping_neg()) };
        let base = t * 32 + ea;
        for idx in 0..8u32 {
            let sh = color_shift(d, q4, idx)?;
            let p = (base + idx) as usize;
            let ps = (base + idx).wrapping_add(es) as usize;
            let pr = (base + idx).wrapping_add(er) as usize;
            let a = d.s[2][ps];
            let dd = shl8(shr8(a, sh), sh);
            let e = d.s[2][p];
            d.s[2][pr] = d.s[2][pr].wrapping_add(dd);
            d.s[2][p] = a;
            d.s[2][ps] = e.wrapping_add(dd);
        }
    }
    Ok(())
}

pub(crate) fn signfix(d: &mut Dcc) -> R<()> {
    if d.g(0xc) == 1 {
        return Ok(());
    }
    let c = d.g(8);
    if c == 0 {
        return Err(Stop::Fault("integer divide by zero in the sign fix: component count 0"));
    }
    let ebx = d.g(4) / c;
    let mut r10 = d.g(4);
    let mut r9 = 0u32;
    loop {
        let mut r8 = r10;
        if d.mask_bit(r9 >> 6) && r10 != 0 {
            let mut cc = 0u32;
            loop {
                let sel = if ebx == 1 {
                    cc & 3 != 0
                } else {
                    if ebx == 0 {
                        return Err(Stop::Fault("integer divide by zero in the sign fix: bytes per element < component count"));
                    }
                    cc % ebx == ebx - 1
                };
                if sel {
                    let o = cc.wrapping_add(r9) as usize;
                    if o >= 256 {
                        return Err(Stop::Undef("sign fix outside the block"));
                    }
                    let v = d.s[2][o];
                    d.s[2][o] = if (v as i8) < 0 { v ^ 0x7f } else { v };
                }
                r8 = d.g(4);
                cc += 1;
                if cc >= r8 {
                    break;
                }
            }
        }
        if r8 == 0 {
            return Err(Stop::Fault("sign fix never advances with 0 bytes per element"));
        }
        r9 = r9.wrapping_add(r8);
        r10 = r8;
        if r9 >= 0x100 {
            return Ok(());
        }
    }
}

pub(crate) fn chan_table(v: u32) -> R<(u32, u32, u32, u32, bool, bool)> {
    Ok(match v {
        1 => (11, 10, 0, 11, false, false),
        2 => (10, 11, 0, 11, false, false),
        3 => (10, 10, 2, 10, false, false),
        4 => (10, 10, 2, 10, false, true),
        5 => (11, 10, 0, 11, true, false),
        6 => (10, 11, 0, 11, true, false),
        7 => (10, 10, 2, 10, true, false),
        8 => (10, 10, 2, 10, true, true),
        10 => (5, 5, 0, 6, false, false),
        11 => (5, 5, 1, 5, false, false),
        12 => (5, 5, 1, 5, false, true),
        _ => return Err(Stop::Undef("channel layout table default takes its widths from the object pointer")),
    })
}

pub(crate) fn channel(d: &mut Dcc) -> R<()> {
    let v = d.g(0x24);
    if v == 0 || v == 9 {
        return Ok(());
    }
    let (a, b, cw, p, b6f, b77) = chan_table(v)?;
    let key = d.g(0x34);
    let mut o = 0u32;
    loop {
        let sec = o >> 6;
        if d.mask_bit(sec) && sector_compressed(key, sec << 6) {
            let bpe = d.g(4);
            if bpe == 2 || bpe == 4 {
                let ou = o as usize;
                let s = &d.s[2];
                let l: [u32; 4] = if bpe == 2 {
                    [s[ou] as u32, s[ou + 1] as u32, 0, 0]
                } else {
                    [s[ou] as u32, s[ou + 2] as u32, s[ou + 1] as u32, s[ou + 3] as u32]
                };
                let bt = |k: u32| l[0].wrapping_shr(k) & 1 != 0 && k < 32;
                let (x, y, z, w);
                if (p as i32) < 8 {
                    let mut eax = l[1] & 1;
                    let mut r8 = (l[1] >> 1) & 1;
                    let mut r10 = 0u32;
                    if a > 1 {
                        r10 = 1;
                        if l[0] & 1 != 0 {
                            eax |= 2;
                        }
                    }
                    for (lvl, bitv) in [(1u32, 2u32), (2, 4), (3, 8), (4, 0x10)] {
                        if lvl > 1 && a > lvl {
                            if bt(r10) {
                                eax |= bitv;
                            }
                            r10 += 1;
                        }
                        if b > lvl {
                            if bt(r10) {
                                r8 |= bitv;
                            }
                            r10 += 1;
                        }
                    }
                    w = if cw > 0 { (l[1] & 0xff) >> 7 } else { 0 };
                    let ecx = 7u32.wrapping_sub(cw);
                    z = if (ecx as i32) < 2 {
                        0
                    } else {
                        let e = ecx - 1;
                        (l[1] >> 2) & if e == 32 { u32::MAX } else { 1u32.wrapping_shl(e).wrapping_sub(1) }
                    };
                    x = eax;
                    y = r8;
                } else {
                    let (r12, e8, r15) = ((p as i32) - 8, (a as i32) - 8, (b as i32) - 8);
                    let (mut eax, mut edx, mut r8, mut edi) = (0u32, 0u32, 0u32, 0u32);
                    let mut r11 = 0u32;
                    if e8 > 0 {
                        r11 = 1;
                        eax = l[0] & 1;
                    }
                    if r12 > 0 {
                        edx = bt(r11) as u32;
                        r11 += 1;
                    }
                    if r15 > 0 {
                        r8 = bt(r11) as u32;
                        r11 += 1;
                    }
                    for (lvl, bitv) in [(1i32, 2u32), (2, 4)] {
                        if e8 > lvl {
                            if bt(r11) {
                                eax |= bitv;
                            }
                            r11 += 1;
                        }
                        if r12 > lvl {
                            if bt(r11) {
                                edx |= bitv;
                            }
                            r11 += 1;
                        }
                        if r15 > lvl {
                            if bt(r11) {
                                r8 |= bitv;
                            }
                            r11 += 1;
                        }
                    }
                    if cw as i32 > 0 {
                        edi = bt(r11) as u32;
                        r11 += 1;
                    }
                    if cw as i32 > 1 && bt(r11) {
                        edi |= 2;
                    }
                    let (sa, sbb, sp) = if b6f {
                        eax <<= 8;
                        edx <<= 8;
                        r8 <<= 8;
                        (0u32, 0u32, 0u32)
                    } else {
                        (e8 as u32, r15 as u32, r12 as u32)
                    };
                    eax |= l[2].wrapping_shl(sa);
                    r8 |= l[3].wrapping_shl(sbb);
                    edx |= l[1].wrapping_shl(sp);
                    x = eax;
                    y = r8;
                    z = edx;
                    w = edi;
                }
                let v = if b77 {
                    (((w.wrapping_shl(b) | y).wrapping_shl(p) | z).wrapping_shl(a)) | x
                } else {
                    w | ((y.wrapping_shl(p) | z).wrapping_shl(a) | x).wrapping_shl(cw)
                };
                let vb = v.to_le_bytes();
                d.s[2][ou] = vb[0];
                d.s[2][ou + 1] = vb[1];
                if d.g(4) == 4 {
                    d.s[2][ou + 2] = vb[2];
                    d.s[2][ou + 3] = vb[3];
                }
            }
        }
        let st = d.g(4);
        if st == 0 {
            return Err(Stop::Fault("channel pass never advances with 0 bytes per element"));
        }
        o = o.wrapping_add(st);
        if o >= 0x100 {
            return Ok(());
        }
    }
}

pub(crate) fn finish(d: &mut Dcc) -> R<()> {
    let saved = d.g(0x38);
    let key = d.g(0x34);
    for k in 0..4u32 {
        let on = saved >> k & 1 != 0 && sector_compressed(key, k * 64);
        d.put_bit(0x38, k, on)?;
    }
    reorder_1dd10(d, 0, 2, 0);
    d.p(0x38, saved);
    Ok(())
}

pub(crate) fn run(d: &mut Dcc) -> R<()> {
    d.o[0x418..0x420].fill(0);
    unpack(d)?;
    tid(d)?;
    dedelta(d);
    calc(d)?;
    reorder_1dd10(d, 2, 2, 2);
    color(d)?;
    reorder_1dd10(d, 2, 2, 1);
    signfix(d)?;
    channel(d)?;
    finish(d)
}
