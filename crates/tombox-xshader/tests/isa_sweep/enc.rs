pub const S_ENDPGM: u32 = 0xbf81_0000;
pub const LIT: u32 = 0xff;
pub const VCC: u32 = 106;
pub const EXEC: u32 = 126;
pub const M0: u32 = 124;
pub const SCC: u32 = 253;

pub fn v(n: u32) -> u32 {
    256 + n
}

pub fn ic(n: i32) -> u32 {
    if n >= 0 { 128 + n as u32 } else { 192 + (-n) as u32 }
}

pub fn vop1(op: u32, vdst: u32, src0: u32) -> u32 {
    0x7e00_0000 | (vdst << 17) | (op << 9) | src0
}

pub fn vop2(op: u32, vdst: u32, src0: u32, vsrc1: u32) -> u32 {
    (op << 25) | (vdst << 17) | (vsrc1 << 9) | src0
}

pub fn vopc(op: u32, src0: u32, vsrc1: u32) -> u32 {
    0x7c00_0000 | (op << 17) | (vsrc1 << 9) | src0
}

pub fn vop3(op: u32, vdst: u32, srcs: [u32; 3]) -> [u32; 2] {
    [0xd400_0000 | (op << 16) | vdst, srcs[0] | (srcs[1] << 9) | (srcs[2] << 18)]
}

pub fn vop3m(op: u32, vdst: u32, srcs: [u32; 3], abs: u32, neg: u32, opsel: u32, clamp: bool) -> [u32; 2] {
    [0xd400_0000 | (op << 16) | ((clamp as u32) << 15) | (opsel << 11) | (abs << 8) | vdst, srcs[0] | (srcs[1] << 9) | (srcs[2] << 18) | (neg << 29)]
}

pub fn vop3o(op: u32, vdst: u32, srcs: [u32; 3], abs: u32, neg: u32, clamp: bool, omod: u32) -> [u32; 2] {
    [0xd400_0000 | (op << 16) | ((clamp as u32) << 15) | (abs << 8) | vdst, srcs[0] | (srcs[1] << 9) | (srcs[2] << 18) | (omod << 27) | (neg << 29)]
}

pub fn vop3b(op: u32, vdst: u32, sdst: u32, srcs: [u32; 3]) -> [u32; 2] {
    [0xd400_0000 | (op << 16) | (sdst << 8) | vdst, srcs[0] | (srcs[1] << 9) | (srcs[2] << 18)]
}

pub fn vop3p(op: u32, vdst: u32, srcs: [u32; 3], opsel: u32, opsel_hi: u32, neg_lo: u32, neg_hi: u32, clamp: bool) -> [u32; 2] {
    [
        0xcc00_0000 | (op << 16) | ((clamp as u32) << 15) | (((opsel_hi >> 2) & 1) << 14) | (opsel << 11) | (neg_hi << 8) | vdst,
        srcs[0] | (srcs[1] << 9) | (srcs[2] << 18) | ((opsel_hi & 3) << 27) | (neg_lo << 29),
    ]
}

pub fn sdwa(src0: u32, dst_sel: u32, dst_unused: u32, s0_sel: u32, s0_sext: bool, s1_sel: u32, s1_sext: bool) -> u32 {
    src0 | (dst_sel << 8) | (dst_unused << 11) | (s0_sel << 16) | ((s0_sext as u32) << 19) | (s1_sel << 24) | ((s1_sext as u32) << 27)
}

pub fn sop2(op: u32, sdst: u32, s0: u32, s1: u32) -> u32 {
    0x8000_0000 | (op << 23) | (sdst << 16) | (s1 << 8) | s0
}

pub fn sop1(op: u32, sdst: u32, s0: u32) -> u32 {
    0xbe80_0000 | (sdst << 16) | (op << 8) | s0
}

pub fn sopk(op: u32, sdst: u32, imm: u16) -> u32 {
    0xb000_0000 | (op << 23) | (sdst << 16) | imm as u32
}

pub fn sopc(op: u32, s0: u32, s1: u32) -> u32 {
    0xbf00_0000 | (op << 16) | (s1 << 8) | s0
}

pub fn mubuf(op: u32, offset: u32, offen: bool, vaddr: u32, vdata: u32) -> [u32; 2] {
    [0xe000_0000 | (op << 18) | ((offen as u32) << 12) | offset, vaddr | (vdata << 8) | (0x80 << 24)]
}

pub fn scc_to(vd: u32) -> Vec<u32> {
    vec![sop2(0x0a, 40, ic(1), ic(0)), vop1(1, vd, 40)]
}

pub fn vcc_to(vd: u32) -> Vec<u32> {
    vec![vop2(0x01, vd, ic(0), 31)]
}

pub fn mask_to(vd: u32, s: u32) -> Vec<u32> {
    vop3(0x101, vd, [ic(0), ic(1), s]).to_vec()
}

pub fn vcc_from(vsrc: u32) -> Vec<u32> {
    vec![vop2(0x1b, 30, ic(1), vsrc), vopc(0xc5, ic(0), 30)]
}

pub fn mask_from(sd: u32, vsrc: u32) -> Vec<u32> {
    let mut w = vec![vop2(0x1b, 30, ic(1), vsrc)];
    w.extend_from_slice(&vop3(0xc5, sd, [ic(0), v(30), 0]));
    w
}

pub fn scc_from(s: u32) -> Vec<u32> {
    vec![sop2(0x0e, 41, s, ic(1))]
}

pub fn s2v(vd: u32, s: u32) -> u32 {
    vop1(1, vd, s)
}

pub fn vmov(vd: u32, src: u32) -> u32 {
    vop1(1, vd, src)
}
