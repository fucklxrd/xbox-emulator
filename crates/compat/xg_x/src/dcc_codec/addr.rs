use super::bits::{copy_bit, insert_bits, log2c, mask, remove_bits, shr_keep};
use super::Dcc;

const X: u8 = 0;
const Y: u8 = 1;
const Z: u8 = 2;

fn layout(mode: u32, bpe: u32) -> &'static [(u8, u8)] {
    match (mode, bpe) {
        (0, 1) => &[(X, 0), (X, 1), (X, 2), (Y, 1), (Y, 0), (Y, 2), (X, 3), (X, 4)],
        (0, 2) => &[(X, 0), (X, 1), (X, 2), (Y, 0), (Y, 1), (Y, 2), (X, 3)],
        (0, 4) => &[(X, 0), (X, 1), (Y, 0), (X, 2), (Y, 1), (Y, 2)],
        (0, 8) => &[(X, 0), (Y, 0), (X, 1), (X, 2), (Y, 1)],
        (3, 1) => &[(Y, 0), (Y, 1), (Y, 2), (X, 1), (X, 0), (X, 2), (X, 3), (X, 4)],
        (3, 2) => &[(Y, 0), (Y, 1), (Y, 2), (X, 0), (X, 1), (X, 2), (X, 3)],
        (3, 4) => &[(Y, 0), (Y, 1), (X, 0), (Y, 2), (X, 1), (X, 2)],
        (3, 8) => &[(Y, 0), (X, 0), (Y, 1), (X, 1), (X, 2)],
        (4, 1) | (4, 2) => &[(X, 0), (Y, 0), (X, 1), (Y, 1), (Z, 0), (Z, 1), (X, 2), (Y, 2)],
        (4, 4) => &[(X, 0), (Y, 0), (X, 1), (Z, 0), (Y, 1), (Z, 1)],
        (4, 8) | (4, 16) => &[(X, 0), (Y, 0), (Z, 0), (X, 1), (Y, 1)],
        (5, _) => &[(X, 0), (Y, 0), (X, 1), (X, 2), (X, 3), (X, 4), (X, 5), (X, 6)],
        _ => &[],
    }
}

fn layout_low(mode: u32, bpe: u32) -> &'static [(u8, u8)] {
    match (mode, bpe) {
        (0, 1) => &[(X, 0), (Y, 0), (X, 1), (Y, 1), (X, 2)],
        (0, 2) => &[(X, 0), (Y, 0), (X, 1), (X, 2)],
        (0, 4) => &[(X, 0), (Y, 0), (X, 1)],
        (3, 1) => &[(Y, 0), (X, 0), (Y, 1), (X, 1), (Y, 2)],
        (3, 2) => &[(Y, 0), (X, 0), (Y, 1), (Y, 2)],
        (3, 4) => &[(Y, 0), (X, 0), (Y, 1)],
        _ => &[],
    }
}

pub(crate) fn channels_1b4d0(d: &Dcc, p: usize) -> u32 {
    match d.g(p + 0x10).wrapping_sub(1) {
        0..=7 => 4,
        8..=11 => 2,
        _ => d.g(8),
    }
}

pub(crate) fn mode_1b440(d: &Dcc, p: usize) -> u32 {
    let v = d.g(p + 0x14);
    if v != 0 {
        return v;
    }
    if d.g(8).wrapping_sub(1) <= 1 {
        return 1;
    }
    match d.g(p + 0x10).wrapping_sub(1) {
        0..=7 => 3,
        8..=11 => 1,
        _ => match d.g(0x10) {
            0 => 2,
            c => (c != 1) as u32 + 2,
        },
    }
}

pub(crate) fn coord_1b540(d: &Dcc, index: u32, ordering: u32) -> [u32; 5] {
    let bpe = d.g(4);
    let (ch, bpc) = if ordering <= 1 {
        (1, bpe)
    } else {
        let c = channels_1b4d0(d, 0x14);
        (c, bpe / c)
    };
    let split_ch = (2..=4).contains(&ordering);
    let split_b = ordering == 3 || ordering == 4;
    let mut esi = log2c(if ordering == 4 { 0x40 } else { 0x20 });
    let top = esi - 1;
    let mut idx = index;
    let (mut byte, mut chn) = (0u32, 0u32);
    if split_b {
        let lo = esi.wrapping_sub(log2c(bpc));
        if top as i32 >= lo as i32 {
            byte = idx.wrapping_shr(lo) & mask(top.wrapping_sub(lo) + 1);
        }
        if bpc > 1 {
            idx = remove_bits(idx, top, lo);
            esi = esi.wrapping_sub(log2c(bpc));
        } else {
            idx = remove_bits(idx, top, esi);
        }
    }
    if split_ch {
        let lo = esi.wrapping_sub(log2c(ch));
        let t = esi.wrapping_sub(1);
        if t as i32 >= lo as i32 {
            chn = idx.wrapping_shr(lo) & mask(t.wrapping_sub(lo) + 1);
        }
        if ch > 1 {
            esi = esi.wrapping_sub(log2c(ch));
        }
        idx = remove_bits(idx, t, esi);
    }
    if !split_b && bpc > 1 {
        let c = log2c(bpc);
        byte = idx & mask(c);
        idx = shr_keep(idx, c);
    }
    if !split_ch && ch > 1 {
        let c = log2c(ch);
        chn = idx & mask(c);
        idx = shr_keep(idx, c);
    }
    let mut v = [0u32; 3];
    let mode = d.g(0);
    if mode == 1 || mode == 2 {
        let e = idx;
        v[0] = (e & 1) | ((e >> 2) & 1) << 1 | ((e >> 4) & 1) << 2 | ((e >> 6) & 1) << 3 | ((e >> 7) & 1) << 4;
        v[1] = ((e >> 1) & 1) | ((e >> 3) & 1) << 1 | ((e >> 5) & 1) << 2;
    } else {
        for (k, &(var, b)) in layout(mode, bpe).iter().enumerate() {
            copy_bit(&mut v[var as usize], b as u32, idx, k as u32);
        }
    }
    if ordering != 0 {
        for (k, &(var, b)) in layout_low(mode, bpe).iter().enumerate() {
            copy_bit(&mut v[var as usize], b as u32, idx, k as u32);
        }
    }
    [v[0], v[1], v[2], chn.wrapping_mul(bpc).wrapping_add(byte), 0]
}

pub(crate) fn index_1c960(d: &Dcc, c: &[u32; 5], ordering: u32) -> u32 {
    let mode = d.g(0);
    let bpe = d.g(4);
    let v = [c[0], c[1], c[2]];
    let mut r = 0u32;
    if mode == 1 || mode == 2 {
        let (x, y) = (v[0], v[1]);
        r = (x & 1) | (y & 1) << 1 | ((x >> 1) & 1) << 2 | ((y >> 1) & 1) << 3 | ((x >> 2) & 1) << 4 | ((y >> 2) & 1) << 5 | ((x >> 3) & 1) << 6 | ((x >> 4) & 1) << 7;
    } else {
        for (k, &(var, b)) in layout(mode, bpe).iter().enumerate() {
            copy_bit(&mut r, k as u32, v[var as usize], b as u32);
        }
    }
    if ordering != 0 {
        for (k, &(var, b)) in layout_low(mode, bpe).iter().enumerate() {
            copy_bit(&mut r, k as u32, v[var as usize], b as u32);
        }
    }
    let (ch, bpc) = if ordering <= 1 {
        (1, bpe)
    } else {
        let c = channels_1b4d0(d, 0x14);
        (c, bpe / c)
    };
    let split_ch = (2..=4).contains(&ordering);
    let split_b = ordering == 3 || ordering == 4;
    let mut esi = log2c(if ordering == 4 { 0x40 } else { 0x20 });
    let off = c[3];
    let chn = (off / bpc) % ch;
    let byte = off % bpc;
    esi = esi.wrapping_sub(log2c(ch)).wrapping_sub(log2c(bpc));
    if !split_ch && ch > 1 {
        let k = log2c(ch);
        esi = esi.wrapping_add(k);
        r = r.wrapping_shl(k) | (mask(k) & chn);
    }
    if !split_b && bpc > 1 {
        let k = log2c(bpc);
        esi = esi.wrapping_add(k);
        r = r.wrapping_shl(k) | (mask(k) & byte);
    }
    if split_ch {
        let k = if ch > 1 {
            esi = esi.wrapping_add(log2c(ch));
            log2c(ch)
        } else {
            0
        };
        r = insert_bits(r, chn, esi.wrapping_sub(1), esi.wrapping_sub(k));
    }
    if split_b {
        let k = if bpc > 1 {
            esi = esi.wrapping_add(log2c(bpc));
            log2c(bpc)
        } else {
            0
        };
        r = insert_bits(r, byte, esi.wrapping_sub(1), esi.wrapping_sub(k));
    }
    r
}

pub(crate) fn reorder_1dd10(d: &mut Dcc, dst: usize, src: usize, ordering: u32) {
    let from = d.s[src];
    let mut scratch = d.sc;
    for s in 0..4u32 {
        if d.g(0x38) >> s & 1 == 0 {
            continue;
        }
        for i in 0..64u32 {
            let b = s * 64 + i;
            let c = coord_1b540(d, b, d.g(0xb0));
            let k = index_1c960(d, &c, ordering);
            scratch[(k & 0x3f) as usize] = from[b as usize];
        }
        d.s[dst][(s * 64) as usize..(s * 64 + 64) as usize].copy_from_slice(&scratch[..]);
    }
    d.sc = scratch;
    d.p(0xb0, ordering);
}
