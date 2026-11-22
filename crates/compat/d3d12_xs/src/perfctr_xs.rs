use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use compat_common::record;

use crate::device_wrap::Guid;
use crate::perfctr::{
    create_object, get_private, guid_at, put_private, rd32, reader, set_addref, set_get_device, set_release, set_resolve, set_sample_size, set_set_name, set_set_private_iface, Layout, PcSource,
    Private, SetObj, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, E_POINTER, S_OK,
};

pub const XS_BLOCKS: usize = 29;
pub const XS_DESC_BYTES: usize = 0x200;
pub const XS_RESULT_BYTES: u32 = 0x2518;
pub const XS_RESULT_VERSION: u32 = 1;
pub const XS_SQ_BLOCK: u8 = 0x17;
pub const E_FAIL: i32 = 0x8000_4005u32 as i32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XsBlock {
    pub slots: u8,
    pub instances: u16,
    pub engines: u16,
    pub arrays: u16,
}

const fn xb(slots: u8, instances: u16, engines: u16, arrays: u16) -> XsBlock {
    XsBlock { slots, instances, engines, arrays }
}

pub const ANACONDA: [XsBlock; XS_BLOCKS] = [
    xb(4, 2, 2, 2), xb(4, 1, 1, 1), xb(4, 4, 1, 1), xb(2, 1, 1, 1), xb(2, 1, 1, 1), xb(2, 1, 1, 1), xb(4, 2, 2, 2), xb(2, 1, 1, 1),
    xb(4, 1, 1, 1), xb(12, 1, 1, 1), xb(4, 1, 2, 2), xb(4, 4, 2, 2), xb(4, 4, 1, 1), xb(4, 20, 1, 1), xb(2, 1, 1, 1), xb(1, 1, 1, 1),
    xb(1, 1, 1, 1), xb(1, 1, 1, 1), xb(8, 1, 1, 1), xb(4, 1, 2, 2), xb(8, 2, 2, 2), xb(1, 1, 1, 1), xb(4, 1, 1, 2), xb(16, 1, 1, 2),
    xb(4, 2, 1, 2), xb(4, 1, 2, 2), xb(5, 14, 2, 2), xb(8, 14, 2, 2), xb(1, 14, 2, 2),
];

pub const LOCKHART: [XsBlock; XS_BLOCKS] = [
    xb(4, 2, 2, 1), xb(4, 1, 1, 1), xb(4, 4, 1, 1), xb(2, 1, 1, 1), xb(2, 1, 1, 1), xb(2, 1, 1, 1), xb(4, 2, 2, 1), xb(2, 1, 1, 1),
    xb(4, 1, 1, 1), xb(12, 1, 1, 1), xb(4, 1, 2, 2), xb(4, 4, 2, 2), xb(4, 4, 1, 1), xb(4, 8, 1, 1), xb(2, 1, 1, 1), xb(1, 1, 1, 1),
    xb(1, 1, 1, 1), xb(1, 1, 1, 1), xb(8, 1, 1, 1), xb(4, 1, 2, 2), xb(8, 2, 2, 1), xb(1, 1, 1, 1), xb(4, 1, 1, 1), xb(16, 1, 1, 1),
    xb(4, 2, 1, 1), xb(4, 1, 2, 1), xb(5, 12, 2, 1), xb(8, 12, 2, 1), xb(1, 12, 2, 1),
];

pub const XS_DESC_OFFSET: [u32; XS_BLOCKS] = [
    0x10, 0x20, 0x30, 0x40, 0x48, 0x50, 0x58, 0x68, 0x70, 0x80, 0xb0, 0xc0, 0xd0, 0xe0, 0xf0, 0xf8, 0xfc, 0x100, 0x104, 0x124, 0x134,
    0x154, 0x158, 0x168, 0x1a8, 0x1b8, 0x1c8, 0x1dc, 0x1fc,
];

pub const XS_RESULT_BASE: [u32; XS_BLOCKS] = [
    0x8, 0x108, 0x128, 0x1a8, 0x1b8, 0x1c8, 0x1d8, 0x2d8, 0x2e8, 0x308, 0x368, 0x3e8, 0x5e8, 0x668, 0x8e8, 0x8f8, 0x900, 0x908, 0x910,
    0x950, 0x9d0, 0xbd0, 0xbd8, 0xc58, 0xd58, 0xdd8, 0xe58, 0x1678, 0x2378,
];

pub const XS_RESULT_STRIDE: [u32; XS_BLOCKS] = [
    0x40, 8, 0x20, 8, 8, 8, 0x40, 8, 8, 8, 0x20, 0x80, 0x20, 0xa0, 8, 8, 8, 8, 8, 0x20, 0x40, 8, 0x20, 0x10, 0x20, 0x20, 0x1a0, 0x1a0,
    0x1a0,
];

pub const XS_BLOCK_NAMES: [&str; XS_BLOCKS] = [
    "CB", "CHA", "CHC", "CPG", "CPC", "CPF", "DB", "GCR", "GDS", "GE", "GL1A", "GL1C", "GL2A", "GL2C", "GRBM", "GRBM_SE0", "GRBM_SE1",
    "MMIO_0x1164", "PA_PH", "RMI", "PA_SC", "MMIO_0x12B8", "SPI", "SQ", "PA_SU", "SX", "TA", "TCP", "TD",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XsEntry {
    pub block: u8,
    pub slot: u8,
    pub select: u32,
    pub instances: u16,
    pub engines: u16,
    pub arrays: u16,
    pub harvested: u16,
    pub sample_offset: u32,
    pub result_offset: u32,
    pub source: PcSource,
}

impl XsEntry {
    pub fn values(&self) -> u32 {
        (self.instances as u32 * self.engines as u32 - self.harvested as u32) * self.arrays as u32
    }
    pub fn bytes(&self) -> u32 {
        8 * self.values()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XsCounterSet {
    pub lockhart: bool,
    pub sq_ctrl: u16,
    pub entries: Vec<XsEntry>,
    pub sample_bytes: u32,
}

pub fn lockhart_for(device_type: u32) -> bool {
    crate::console_slots_dev2::kmd_info(device_type).mode == 1
}

pub fn xs_select(block: u8, sel: u32) -> u32 {
    match block {
        0xe..=0x10 => sel & 0x3f,
        XS_SQ_BLOCK => 0xf000 | (sel & 0x1ff),
        _ => sel & 0x3ff,
    }
}

pub fn xs_harvested(block: u8, lockhart: bool) -> u16 {
    match (block, lockhart) {
        (0x1a..=0x1c, true) => 4,
        (0x1a..=0x1c, false) => 2,
        _ => 0,
    }
}

pub fn xs_pc_source(device_type: u32, block: u8, select: u32) -> PcSource {
    let always = match block {
        3..=5 => select & 0x3ff == 0,
        0xe..=0x10 => select & 0x3f == 0,
        _ => false,
    };
    if always { PcSource::ConsoleClock { hz: crate::console_slots_dev2::kmd_info(device_type).word0 } } else { PcSource::None }
}

pub fn xs_build(desc: &[u8; XS_DESC_BYTES], device_type: u32) -> XsCounterSet {
    let lockhart = lockhart_for(device_type);
    let table = if lockhart { &LOCKHART } else { &ANACONDA };
    let sq = rd32(desc, 0xc);
    let mut set = XsCounterSet { lockhart, sq_ctrl: if sq == 0 { 0xffff } else { sq as u16 }, entries: Vec::new(), sample_bytes: 0 };
    for b in 0..XS_BLOCKS {
        let info = table[b];
        let base = XS_DESC_OFFSET[b] as usize;
        let mut used: u8 = 0;
        for k in 0..info.slots as usize {
            let sel = rd32(desc, base + 4 * k);
            let skip = sel == u32::MAX || (b as u8 == XS_SQ_BLOCK && k > 0 && used != 0 && rd32(desc, base + 4 * (k - 1)) == u32::MAX);
            if skip || used >= info.slots {
                continue;
            }
            let select = xs_select(b as u8, sel);
            let e = XsEntry {
                block: b as u8,
                slot: used,
                select,
                instances: info.instances,
                engines: info.engines,
                arrays: info.arrays,
                harvested: xs_harvested(b as u8, lockhart),
                sample_offset: set.sample_bytes,
                result_offset: XS_RESULT_BASE[b] + k as u32 * XS_RESULT_STRIDE[b],
                source: xs_pc_source(device_type, b as u8, select),
            };
            set.sample_bytes += e.bytes();
            set.entries.push(e);
            used += 1;
        }
    }
    set
}

pub fn xs_resolve_into(set: &XsCounterSet, mut raw: impl FnMut(&XsEntry, u32) -> Option<u64>, out: &mut [u8; XS_RESULT_BYTES as usize]) {
    out[0..4].copy_from_slice(&XS_RESULT_BYTES.to_le_bytes());
    out[4..8].copy_from_slice(&XS_RESULT_VERSION.to_le_bytes());
    out[8..].fill(0xff);
    for e in &set.entries {
        for i in 0..e.values() {
            if let Some(v) = raw(e, i) {
                let o = (e.result_offset + 8 * i) as usize;
                out[o..o + 8].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
}

pub fn xs_raw_value(rd: Option<fn(usize, u64) -> Option<u64>>, res: usize, offset: u64, e: &XsEntry, i: u32) -> Option<u64> {
    if e.source == PcSource::None {
        return None;
    }
    rd.and_then(|f| f(res, offset + e.sample_offset as u64 + 8 * i as u64))
}

pub fn xs_resolve_resource(set: &XsCounterSet, res: usize, offset: u64, out: &mut [u8; XS_RESULT_BYTES as usize], rd: Option<fn(usize, u64) -> Option<u64>>) -> usize {
    let mut fed = 0usize;
    xs_resolve_into(
        set,
        |e, i| {
            let v = xs_raw_value(rd, res, offset, e, i);
            fed += v.is_some() as usize;
            v
        },
        out,
    );
    fed
}

static RESOLVE_LOG: AtomicUsize = AtomicUsize::new(0);

pub(crate) unsafe fn resolve_object(set: &Arc<XsCounterSet>, res: usize, offset: u64, out: *mut u8) -> i32 {
    let dst = unsafe { &mut *(out as *mut [u8; XS_RESULT_BYTES as usize]) };
    let fed = xs_resolve_resource(set, res, offset, dst, reader());
    if RESOLVE_LOG.fetch_add(1, Ordering::Relaxed) < 4 {
        let none = set.entries.iter().filter(|e| e.source == PcSource::None).count();
        record("d3d12_xs", "perfctr", &format!("Scarlett ResolveSamples {} entries, {} bytes, {fed} values from the CL sampler, {none} entries without a PC source", set.entries.len(), set.sample_bytes));
    }
    S_OK
}

pub static XS_QI_IIDS: [Guid; 55] = [
    Guid { d1: 0xaceeea63, d2: 0xe0a9, d3: 0x4a1c, d4: [0xbb, 0xec, 0x71, 0xb2, 0xf4, 0x85, 0xf7, 0x58] },
    Guid { d1: 0xaec22fb8, d2: 0x76f3, d3: 0x4639, d4: [0x9b, 0xe0, 0x28, 0xeb, 0x43, 0xa6, 0x7a, 0x2e] },
    Guid { d1: 0x3d3e0379, d2: 0xf9de, d3: 0x4d58, d4: [0xbb, 0x6c, 0x18, 0xd6, 0x29, 0x92, 0xf1, 0xa6] },
    Guid { d1: 0x7b7166ec, d2: 0x21c7, d3: 0x44ae, d4: [0xb2, 0x1a, 0xc9, 0xae, 0x32, 0x1a, 0xe3, 0x69] },
    Guid { d1: 0x770aae78, d2: 0xf26f, d3: 0x4dba, d4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87] },
    Guid { d1: 0x50c83a1c, d2: 0xe072, d3: 0x4c48, d4: [0x87, 0xb0, 0x36, 0x30, 0xfa, 0x36, 0xa6, 0xd0] },
    Guid { d1: 0x2411e7e1, d2: 0x12ac, d3: 0x4ccf, d4: [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0] },
    Guid { d1: 0x29038f61, d2: 0x3839, d3: 0x4626, d4: [0x91, 0xfd, 0x08, 0x68, 0x79, 0x01, 0x1a, 0x05] },
    Guid { d1: 0x0aa1ae0a, d2: 0xfa0e, d3: 0x4b84, d4: [0x86, 0x44, 0xe0, 0x5f, 0xf8, 0xe5, 0xac, 0xb5] },
    Guid { d1: 0x54ec77fa, d2: 0x1377, d3: 0x44e6, d4: [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c] },
    Guid { d1: 0x77db970f, d2: 0x6276, d3: 0x48ba, d4: [0xba, 0x28, 0x07, 0x01, 0x43, 0xb4, 0x39, 0x2c] },
    Guid { d1: 0x05008617, d2: 0xfbfd, d3: 0x4051, d4: [0xa7, 0x90, 0x14, 0x48, 0x84, 0xb4, 0xf6, 0xa9] },
    Guid { d1: 0x310d36a0, d2: 0xd2e7, d3: 0x4c0a, d4: [0xaa, 0x04, 0x6a, 0x9d, 0x23, 0xb8, 0x88, 0x6a] },
    Guid { d1: 0x790a45f7, d2: 0x0d42, d3: 0x4876, d4: [0x98, 0x3a, 0x0a, 0x55, 0xcf, 0xe6, 0xf4, 0xaa] },
    Guid { d1: 0x8ba5fb08, d2: 0x5195, d3: 0x40e2, d4: [0xac, 0x58, 0x0d, 0x98, 0x9c, 0x3a, 0x01, 0x02] },
    Guid { d1: 0x189819f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] },
    Guid { d1: 0x7116d91c, d2: 0xe7e4, d3: 0x47ce, d4: [0xb8, 0xc6, 0xec, 0x81, 0x68, 0xf4, 0x37, 0xe5] },
    Guid { d1: 0x5b160d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] },
    Guid { d1: 0x8754318e, d2: 0xd3a9, d3: 0x4541, d4: [0x98, 0xcf, 0x64, 0x5b, 0x50, 0xdc, 0x48, 0x74] },
    Guid { d1: 0x55050859, d2: 0x4024, d3: 0x474c, d4: [0x98, 0xcf, 0x64, 0x5b, 0x51, 0xdc, 0x48, 0x74] },
    Guid { d1: 0xc3827890, d2: 0x4024, d3: 0x474c, d4: [0x98, 0xcf, 0x64, 0x5b, 0x51, 0xdc, 0x48, 0x74] },
    Guid { d1: 0x0ec870a6, d2: 0x5d7e, d3: 0x4c22, d4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed] },
    Guid { d1: 0xc54a6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14] },
    Guid { d1: 0x6b3b2502, d2: 0x6e51, d3: 0x45b3, d4: [0x90, 0xee, 0x98, 0x84, 0x26, 0x5e, 0x8d, 0xf3] },
    Guid { d1: 0x572f7389, d2: 0x2168, d3: 0x49e3, d4: [0x96, 0x93, 0xd6, 0xdf, 0x58, 0x71, 0xbf, 0x6d] },
    Guid { d1: 0x696442be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] },
    Guid { d1: 0x6102dee4, d2: 0xaf59, d3: 0x4b09, d4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24] },
    Guid { d1: 0x0a753dcf, d2: 0xc4d8, d3: 0x4b91, d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76] },
    Guid { d1: 0x433685fe, d2: 0xe22b, d3: 0x4ca0, d4: [0xa8, 0xdb, 0xb5, 0xb4, 0xf4, 0xdd, 0x0e, 0x4a] },
    Guid { d1: 0x765a30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] },
    Guid { d1: 0x8efb471d, d2: 0x616c, d3: 0x4f49, d4: [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51] },
    Guid { d1: 0x0d9658ae, d2: 0xed45, d3: 0x469e, d4: [0xa6, 0x1d, 0x97, 0x0e, 0xc5, 0x83, 0xca, 0xb4] },
    Guid { d1: 0xc36a797c, d2: 0xec80, d3: 0x4f0a, d4: [0x89, 0x85, 0xa7, 0xb2, 0x47, 0x50, 0x82, 0xd1] },
    Guid { d1: 0xae02eedb, d2: 0xc735, d3: 0x4690, d4: [0x8d, 0x52, 0x5a, 0x8d, 0xc2, 0x02, 0x13, 0xaa] },
    Guid { d1: 0x77acce80, d2: 0x638e, d3: 0x4e65, d4: [0x88, 0x95, 0xc1, 0xf2, 0x33, 0x86, 0x86, 0x3e] },
    Guid { d1: 0x30baa41e, d2: 0xb15b, d3: 0x475c, d4: [0xa0, 0xbb, 0x1a, 0xf5, 0xc5, 0xb6, 0x43, 0x28] },
    Guid { d1: 0x81dadc15, d2: 0x2bad, d3: 0x4392, d4: [0x93, 0xc5, 0x10, 0x13, 0x45, 0xc4, 0xaa, 0x98] },
    Guid { d1: 0xe865df17, d2: 0xa9ee, d3: 0x46f9, d4: [0xa4, 0x63, 0x30, 0x98, 0x31, 0x5a, 0xa2, 0xe5] },
    Guid { d1: 0x8b4f173b, d2: 0x2fea, d3: 0x4b80, d4: [0x8f, 0x58, 0x43, 0x07, 0x19, 0x1a, 0xb9, 0x5d] },
    Guid { d1: 0xc70b221b, d2: 0x40e4, d3: 0x4a17, d4: [0x89, 0xaf, 0x02, 0x5a, 0x07, 0x27, 0xa6, 0xdc] },
    Guid { d1: 0x5c014b53, d2: 0x68a1, d3: 0x4b9b, d4: [0x8b, 0xd1, 0xdd, 0x60, 0x46, 0xb9, 0x35, 0x8b] },
    Guid { d1: 0x728f1fa2, d2: 0xea11, d3: 0x4a9f, d4: [0x9b, 0x52, 0x9b, 0x42, 0xa4, 0x84, 0xd5, 0xc9] },
    Guid { d1: 0x553103fb, d2: 0x1fe7, d3: 0x4557, d4: [0xbb, 0x38, 0x94, 0x6d, 0x7d, 0x0e, 0x7c, 0xa7] },
    Guid { d1: 0x38c3e585, d2: 0xff17, d3: 0x412c, d4: [0x91, 0x50, 0x4f, 0xc6, 0xf9, 0xd7, 0x2a, 0x28] },
    Guid { d1: 0xc64226a8, d2: 0x9201, d3: 0x46af, d4: [0xb4, 0xcc, 0x53, 0xfb, 0x9f, 0xf7, 0x41, 0x4f] },
    Guid { d1: 0x80eabf42, d2: 0x2568, d3: 0x4e5e, d4: [0xbd, 0x82, 0xc3, 0x7f, 0x86, 0x96, 0x1d, 0xc3] },
    Guid { d1: 0x1bc60fb0, d2: 0xedd6, d3: 0x4b78, d4: [0xa3, 0x5b, 0xb2, 0xa4, 0xf2, 0xd9, 0x4d, 0x9f] },
    Guid { d1: 0x7071e1f0, d2: 0xe84b, d3: 0x4b33, d4: [0x97, 0x4f, 0x12, 0xfa, 0x49, 0xde, 0x65, 0xc5] },
    Guid { d1: 0xc4fec28f, d2: 0x7966, d3: 0x4e95, d4: [0x9f, 0x94, 0xf4, 0x31, 0xcb, 0x56, 0xc3, 0xb9] },
    Guid { d1: 0x3b60536e, d2: 0xad29, d3: 0x4e64, d4: [0xa2, 0x69, 0xf8, 0x53, 0x83, 0x7e, 0x5e, 0x53] },
    Guid { d1: 0xc59b6bdc, d2: 0x7720, d3: 0x4074, d4: [0xa1, 0x36, 0x17, 0xa1, 0x56, 0x03, 0x74, 0x70] },
    Guid { d1: 0xaeb2543a, d2: 0x167f, d3: 0x4682, d4: [0xac, 0xc8, 0xd1, 0x59, 0xed, 0x4a, 0x62, 0x09] },
    Guid { d1: 0x304fdb32, d2: 0xbede, d3: 0x410a, d4: [0x85, 0x45, 0x94, 0x3a, 0xc6, 0xa4, 0x61, 0x38] },
    Guid { d1: 0x8455293a, d2: 0x0cbd, d3: 0x4831, d4: [0x9b, 0x39, 0xfb, 0xdb, 0xab, 0x72, 0x47, 0x23] },
    Guid { d1: 0x94971eca, d2: 0x2bdb, d3: 0x4769, d4: [0x88, 0xcf, 0x36, 0x75, 0xea, 0x75, 0x7e, 0xbc] },
];

pub const WKPDID_D3D_DEBUG_OBJECT_NAME_W: Guid = Guid { d1: 0x4cca5fd8, d2: 0x921f, d3: 0x42c8, d4: [0x85, 0x66, 0x70, 0xca, 0xf2, 0xa9, 0xb7, 0x41] };

fn live() -> &'static Mutex<HashMap<usize, Arc<XsCounterSet>>> {
    static M: OnceLock<Mutex<HashMap<usize, Arc<XsCounterSet>>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn xs_counter_set_for(ptr: usize) -> Option<Arc<XsCounterSet>> {
    live().lock().unwrap_or_else(|e| e.into_inner()).get(&ptr).cloned()
}

pub(crate) fn forget(ptr: usize) {
    live().lock().unwrap_or_else(|e| e.into_inner()).remove(&ptr);
}

unsafe extern "system" fn xs_qi(this: *mut SetObj, riid: *const Guid, ppv: *mut usize) -> i32 {
    let hit = guid_at(riid).is_some_and(|g| XS_QI_IIDS.iter().any(|k| *k == g));
    if hit {
        unsafe { set_addref(this) };
    }
    if !ppv.is_null() {
        unsafe { *ppv = if hit { this as usize } else { 0 } };
    }
    if hit { S_OK } else { E_NOINTERFACE }
}

unsafe extern "system" fn xs_get_private(this: *mut SetObj, g: *const Guid, size: *mut u32, data: *mut u8) -> i32 {
    let Some(g) = guid_at(g) else {
        record("d3d12_xs", "perfctr", "Scarlett GetPrivateData with a null guid: the console dereferences it and faults");
        return E_POINTER;
    };
    if g == WKPDID_D3D_DEBUG_OBJECT_NAME_W {
        if !size.is_null() {
            unsafe { *size = 0 };
        }
        return E_FAIL;
    }
    if size.is_null() {
        record("d3d12_xs", "perfctr", "Scarlett GetPrivateData with a null size: the console dereferences it and faults");
        return E_POINTER;
    }
    let o = unsafe { &*this };
    let items = o.private.lock().unwrap_or_else(|e| e.into_inner());
    get_private(&items, &g, unsafe { &mut *size }, data)
}

unsafe extern "system" fn xs_set_private(this: *mut SetObj, g: *const Guid, size: u32, data: *const u8) -> i32 {
    let Some(g) = guid_at(g) else {
        record("d3d12_xs", "perfctr", "Scarlett SetPrivateData with a null guid: the console dereferences it and faults");
        return E_POINTER;
    };
    if g == WKPDID_D3D_DEBUG_OBJECT_NAME_W {
        return S_OK;
    }
    let o = unsafe { &*this };
    let p = if data.is_null() { None } else { Some(Private::Data(unsafe { core::slice::from_raw_parts(data, size as usize) }.to_vec())) };
    put_private(&mut o.private.lock().unwrap_or_else(|e| e.into_inner()), g, p)
}

fn xs_vtable() -> *const usize {
    static VT: OnceLock<[usize; 10]> = OnceLock::new();
    VT.get_or_init(|| {
        [
            xs_qi as *const () as usize,
            set_addref as *const () as usize,
            set_release as *const () as usize,
            xs_get_private as *const () as usize,
            xs_set_private as *const () as usize,
            set_set_private_iface as *const () as usize,
            set_set_name as *const () as usize,
            set_get_device as *const () as usize,
            set_sample_size as *const () as usize,
            set_resolve as *const () as usize,
        ]
    })
    .as_ptr()
}

pub fn xs_create(device: usize, set: XsCounterSet) -> usize {
    let set = Arc::new(set);
    let p = create_object(device, Layout::Xs(set.clone()), xs_vtable());
    live().lock().unwrap_or_else(|e| e.into_inner()).insert(p, set);
    p
}

static CREATE_LOG: AtomicUsize = AtomicUsize::new(0);

pub unsafe fn create_counter_set_xs(enabled: bool, device_type: u32, this: usize, desc: *const u8, ppv: *mut usize) -> i32 {
    if !enabled {
        return E_NOTIMPL;
    }
    if desc.is_null() {
        record("d3d12_xs", "perfctr", "Scarlett CreateCounterSetX with a null desc: the console dereferences it and faults");
        return E_INVALIDARG;
    }
    let mut d = [0u8; XS_DESC_BYTES];
    unsafe { core::ptr::copy_nonoverlapping(desc, d.as_mut_ptr(), XS_DESC_BYTES) };
    let set = xs_build(&d, device_type);
    if CREATE_LOG.fetch_add(1, Ordering::Relaxed) < 8 {
        let list: Vec<String> = set.entries.iter().map(|e| format!("{}[{}]={:#x}{}", XS_BLOCK_NAMES[e.block as usize], e.slot, e.select, if e.source == PcSource::None { "" } else { "*" })).collect();
        record("d3d12_xs", "perfctr", &format!("Scarlett CreateCounterSetX (retail counter file present) lockhart={} sq_ctrl={:#x} sample={} bytes: {}", set.lockhart, set.sq_ctrl, set.sample_bytes, list.join(" ")));
    }
    let p = xs_create(this, set);
    if !ppv.is_null() {
        unsafe { *ppv = p };
    }
    S_OK
}
