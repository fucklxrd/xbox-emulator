use super::fp::*;
use super::*;

#[derive(Clone, Copy)]
enum K {
    F32,
    F16,
    F64,
    I32,
    U32,
    I64,
    U64,
    I16,
    U16,
    C32,
    C16,
    C64,
}

fn cmp_of(k: K, code: u32, x: &In) -> bool {
    let w = |i: usize| x.d[2 * i] as u64 | ((x.d[2 * i + 1] as u64) << 32);
    match k {
        K::F32 => fcmp(code, f(x.d[0]) as f64, f(x.d[1]) as f64),
        K::F16 => fcmp(code, h2d(x.d[0] & 0xffff), h2d(x.d[1] & 0xffff)),
        K::F64 => fcmp(code, f64::from_bits(w(0)), f64::from_bits(w(1))),
        K::I32 => icmp(code, x.d[0] as i32 as i128, x.d[1] as i32 as i128),
        K::U32 => icmp(code, x.d[0] as i128, x.d[1] as i128),
        K::I64 => icmp(code, w(0) as i64 as i128, w(1) as i64 as i128),
        K::U64 => icmp(code, w(0) as i128, w(1) as i128),
        K::I16 => icmp(code, x.d[0] as u16 as i16 as i128, x.d[1] as u16 as i16 as i128),
        K::U16 => icmp(code, (x.d[0] & 0xffff) as i128, (x.d[1] & 0xffff) as i128),
        K::C32 => class_bits32(x.d[0]) & x.d[1] != 0,
        K::C16 => class_bits16(x.d[0]) & x.d[1] != 0,
        K::C64 => class_bits64(w(0)) & x.d[2] != 0,
    }
}

fn wide(k: K) -> bool {
    matches!(k, K::F64 | K::I64 | K::U64 | K::C64)
}

fn class_of(k: K) -> Class {
    match k {
        K::F32 => Class::F32,
        K::C32 => Class::F32Cls,
        K::F16 | K::C16 => Class::F16,
        K::F64 | K::C64 => Class::F64,
        K::I64 | K::U64 => Class::U64,
        _ => Class::U32,
    }
}

fn exec_quarter() -> Vec<u32> {
    let mut w = vec![sop1(3, VCC, ic(-1)), vop2(0x1b, 30, ic(3), 0)];
    w.extend_from_slice(&vop3(0xc5, EXEC, [ic(3), v(30), 0]));
    w
}

fn flt(k: K) -> Option<u32> {
    match k {
        K::F32 => Some(0x8000_0000),
        K::F16 => Some(0x8000),
        K::F64 => Some(0x8000_0000),
        _ => None,
    }
}

fn one(s: &mut Suite, m: &'static str, op: u32, k: K, code: u32, x_: bool) {
    let class = class_of(k);
    let (s0, s1) = if wide(k) { (v(10), 12) } else { (v(10), 11) };
    let s1_3 = v(s1);
    let e32 = vopc(op, s0, s1);
    let r = move |x: &In| cmp_of(k, code, x) as u32;
    if !x_ {
        s.add(m, "VOPC", "e32 vcc", class, &[], &[e32], &vcc_to(OUT), &[Cmp::X], move |x| o(r(x)));
        let mut post = vec![sop1(3, 42, EXEC), sop1(3, EXEC, ic(-1))];
        post.extend(vcc_to(OUT));
        s.add(m, "VOPC", "e32 lanes%4==3 inactive", class, &exec_quarter(), &[e32], &post, &[Cmp::X], move |x| o(if x.lane % 4 == 3 { 0 } else { r(x) }));
        s.add(m, "VOPC", "e64 sdst=s42", class, &[], &vop3(op, 42, [s0, s1_3, 0]), &mask_to(OUT, 42), &[Cmp::X], move |x| o(r(x)));
        if !wide(k) {
            s.add(m, "VOPC", "e32 src0=sgpr", class, &[], &[vopc(op, 10, s1)], &vcc_to(OUT), &[Cmp::X], move |x| o(r(x)));
        }
        if let Some(sb) = flt(k) {
            let (i0, i1) = if wide(k) { (1, 3) } else { (0, 1) };
            s.add(m, "VOPC", "e64 neg0 abs1", class, &[], &vop3m(op, 42, [s0, s1_3, 0], 2, 1, 0, false), &mask_to(OUT, 42), &[Cmp::X], move |x| {
                let mut y = In { d: x.d, lane: x.lane };
                y.d[i0] ^= sb;
                y.d[i1] &= !sb;
                o(cmp_of(k, code, &y) as u32)
            });
        }
    } else {
        let mut post = vec![sop1(3, 42, EXEC), sop1(3, EXEC, ic(-1))];
        post.extend(mask_to(OUT, 42));
        post.extend(vcc_to(OUT + 1));
        s.add(m, "VOPC", "e32 exec, vcc untouched", class, &[sop1(3, VCC, ic(-1))], &[e32], &post, &[Cmp::X, Cmp::X], move |x| o2(r(x), 1));
        s.add(m, "VOPC", "e32 lanes%4==3 inactive", class, &exec_quarter(), &[e32], &post, &[Cmp::X, Cmp::X], move |x| o2(if x.lane % 4 == 3 { 0 } else { r(x) }, 1));
        s.add(m, "VOPC", "e64", class, &[sop1(3, VCC, ic(-1))], &vop3(op, EXEC, [s0, s1_3, 0]), &post, &[Cmp::X, Cmp::X], move |x| o2(r(x), 1));
    }
}

pub fn add(s: &mut Suite) {
    for op in 0u32..0x100 {
        let Some(m) = tombox_xshader::rdna2::mnem::vopc(op) else { continue };
        let cmpx = m.starts_with("v_cmpx_");
        let k = if m.contains("class_f32") {
            K::C32
        } else if m.contains("class_f16") {
            K::C16
        } else if m.contains("class_f64") {
            K::C64
        } else if m.ends_with("_f32") {
            K::F32
        } else if m.ends_with("_f16") {
            K::F16
        } else if m.ends_with("_f64") {
            K::F64
        } else if m.ends_with("_i32") {
            K::I32
        } else if m.ends_with("_u32") {
            K::U32
        } else if m.ends_with("_i64") {
            K::I64
        } else if m.ends_with("_u64") {
            K::U64
        } else if m.ends_with("_i16") {
            K::I16
        } else {
            K::U16
        };
        let fcode = |n: &str| ["f", "lt", "eq", "le", "gt", "lg", "ge", "o", "u", "nge", "nlg", "ngt", "nle", "neq", "nlt", "tru"].iter().position(|c| *c == n).unwrap() as u32;
        let icode = |n: &str| ["f", "lt", "eq", "le", "gt", "ne", "ge", "t"].iter().position(|c| *c == n).unwrap() as u32;
        let name = m.trim_start_matches("v_cmpx_").trim_start_matches("v_cmp_");
        let cond = name.split('_').next().unwrap();
        let code = match k {
            K::F32 | K::F16 | K::F64 => fcode(cond),
            K::C32 | K::C16 | K::C64 => 0,
            _ => icode(cond),
        };
        one(s, m, op, k, code, cmpx);
    }
}
