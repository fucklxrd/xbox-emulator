pub const TB_CENTROID_SPACE: u32 = 4_000_004;
pub const TB_CENTROID_REGISTER: u32 = 0;
pub const TB_CENTROID_ROOT_DWORDS: u32 = 12;
pub const TB_CENTROID_FLAG_CUSTOM: u32 = 1;
pub const TB_CENTROID_FLAG_POSITIONS: u32 = 2;
pub const TB_CENTROID_MAX_POSITION_SAMPLES: u32 = 8;
pub const TB_CENTROID_PC_DEFINE: &str = "TB_CENTROID_PC";
pub const TB_CENTROID_CENTER: u32 = u32::MAX;
pub const CONSOLE_DEFAULT: [[u32; 2]; 5] = [[0, 0], [0x1010_1010, 0x1010_1010], [0x3210_3210, 0x3210_3210], [0x7654_3210, 0x7654_3210], [0xb9f2_6cd7, 0x8513_ea40]];

pub type Constants = [u32; TB_CENTROID_ROOT_DWORDS as usize];

pub fn pack(c: &[u8; 16]) -> [u32; 2] {
    let mut w = [0u32; 2];
    for i in 0..8 {
        w[0] |= (c[i] as u32 & 0xf) << (4 * i);
        w[1] |= (c[8 + i] as u32 & 0xf) << (4 * i);
    }
    w
}

pub fn root_constants(custom: Option<[u8; 16]>, applied_positions: Option<(&[u8; 128], u32)>) -> Constants {
    let mut c = [0u32; TB_CENTROID_ROOT_DWORDS as usize];
    if let Some(t) = custom {
        let w = pack(&t);
        c[0] = w[0];
        c[1] = w[1];
        c[2] |= TB_CENTROID_FLAG_CUSTOM;
    }
    if let Some((pos, n)) = applied_positions.filter(|&(_, n)| (2..=TB_CENTROID_MAX_POSITION_SAMPLES).contains(&n)) {
        for p in 0..4usize {
            for s in 0..n as usize {
                let x = pos[32 * p + 2 * s] as u32 & 0xf;
                let y = pos[32 * p + 2 * s + 1] as u32 & 0xf;
                let b = p * 8 + s;
                c[4 + b / 4] |= (x | (y << 4)) << ((b % 4) * 8);
            }
        }
        c[2] |= TB_CENTROID_FLAG_POSITIONS;
    }
    c
}

pub fn console_default(samples: u32) -> [u32; 2] {
    let li = if samples == 0 { 0 } else { 31 - samples.leading_zeros() };
    CONSOLE_DEFAULT[li.min(4) as usize]
}

pub fn pick(table: [u32; 2], coverage: u32, samples: u32) -> u32 {
    let full = if samples >= 32 { u32::MAX } else { (1u32 << samples) - 1 };
    let cov = coverage & full;
    if samples <= 1 || cov == full || cov == 0 {
        return TB_CENTROID_CENTER;
    }
    for k in 0..16 {
        let s = (table[k / 8] >> ((k % 8) * 4)) & 0xf;
        if s < samples && (cov >> s) & 1 != 0 {
            return s;
        }
    }
    TB_CENTROID_CENTER
}

pub fn pick_for(c: &Constants, coverage: u32, samples: u32) -> u32 {
    let t = if c[2] & TB_CENTROID_FLAG_CUSTOM != 0 { [c[0], c[1]] } else { console_default(samples) };
    pick(t, coverage, samples)
}

pub fn snapped_offset(c: &Constants, sample: u32, px: u32, py: u32) -> Option<[i8; 2]> {
    if c[2] & TB_CENTROID_FLAG_POSITIONS == 0 || sample >= TB_CENTROID_MAX_POSITION_SAMPLES {
        return None;
    }
    let b = (((px & 1) | ((py & 1) << 1)) * 8 + sample) as usize;
    let v = (c[4 + b / 4] >> ((b % 4) * 8)) & 0xff;
    let sx = |n: u32| (((n & 0xf) << 4) as u8 as i8) >> 4;
    Some([sx(v), sx(v >> 4)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_nibbles_like_the_console_umd() {
        let mut c = [0u8; 16];
        for (i, v) in c.iter_mut().enumerate() {
            *v = (15 - i) as u8 | 0xf0;
        }
        assert_eq!(pack(&c), [0x89ab_cdef, 0x0123_4567]);
        let k = root_constants(Some(c), None);
        assert_eq!(&k[..3], &[0x89ab_cdef, 0x0123_4567, TB_CENTROID_FLAG_CUSTOM]);
        assert_eq!(root_constants(None, None), [0; 12]);
    }

    #[test]
    fn packs_per_quad_pixel_positions() {
        let mut pos = [0u8; 128];
        for p in 0..4 {
            for s in 0..4 {
                pos[32 * p + 2 * s] = (s as i8 - 8 + p as i8) as u8;
                pos[32 * p + 2 * s + 1] = (7 - s as i8 - p as i8) as u8;
            }
        }
        let c = root_constants(None, Some((&pos, 4)));
        assert_eq!(c[2], TB_CENTROID_FLAG_POSITIONS);
        for p in 0..4u32 {
            for s in 0..4u32 {
                let want = [s as i8 - 8 + p as i8, 7 - s as i8 - p as i8];
                assert_eq!(snapped_offset(&c, s, p & 1, p >> 1), Some(want));
                assert_eq!(snapped_offset(&c, s, (p & 1) + 6, (p >> 1) + 2), Some(want));
            }
        }
        assert_eq!(root_constants(None, Some((&pos, 16)))[2], 0);
        assert_eq!(snapped_offset(&root_constants(None, None), 0, 0, 0), None);
    }

    #[test]
    fn picks_first_covered_sample_in_priority_order() {
        let t = pack(&[3, 2, 1, 0, 3, 2, 1, 0, 3, 2, 1, 0, 3, 2, 1, 0]);
        assert_eq!(pick(t, 0b0011, 4), 1);
        assert_eq!(pick(t, 0b1001, 4), 3);
        assert_eq!(pick(t, 0b0001, 4), 0);
        assert_eq!(pick(t, 0b1111, 4), TB_CENTROID_CENTER);
        assert_eq!(pick(t, 0, 4), TB_CENTROID_CENTER);
        assert_eq!(pick(t, 1, 1), TB_CENTROID_CENTER);
        let hi = pack(&[9, 9, 9, 9, 9, 9, 9, 9, 2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(pick(hi, 0b0101, 4), 2);
        let none = root_constants(None, None);
        assert_eq!(pick_for(&none, 0b0110, 4), 1);
        assert_eq!(pick_for(&none, 0x6000, 16), 13);
        assert_eq!(console_default(8), [0x7654_3210, 0x7654_3210]);
        assert_eq!(console_default(1), [0, 0]);
    }
}
