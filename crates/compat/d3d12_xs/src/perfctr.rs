use core::ffi::c_void;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use compat_common::record;

use crate::device_wrap::Guid;

pub const DEVICE_SLOT: usize = 82;
pub const BLOCKS: usize = 33;
pub const DESC_BYTES: usize = 0x244;
pub const RESULT_BYTES: u32 = 0x1ef8;
pub const RESULT_VERSION: u32 = 2;
pub const FIRST_WIDE_BLOCK: u8 = 0x15;
pub const SQ_BLOCK: u8 = 0xb;

pub(crate) const S_OK: i32 = 0;
pub(crate) const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
pub(crate) const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
pub(crate) const E_POINTER: i32 = 0x8000_4003u32 as i32;
pub(crate) const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const PRIVATE_DATA_NOT_FOUND: i32 = 0x8879_0002u32 as i32;

pub const DURANGO_HZ: u64 = 853_333_333;
pub const DURANGO_S_HZ: u64 = 914_285_714;
pub const SCORPIO_HZ: u64 = 1_172_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockInfo {
    pub slots: u8,
    pub instances: u16,
    pub engines: u16,
}

const fn bi(slots: u8, instances: u16, engines: u16) -> BlockInfo {
    BlockInfo { slots, instances, engines }
}

pub const DURANGO: [BlockInfo; BLOCKS] = [
    bi(2, 1, 1), bi(2, 1, 1), bi(2, 1, 1), bi(2, 1, 1), bi(2, 1, 1), bi(4, 2, 2), bi(4, 2, 2), bi(4, 1, 2),
    bi(8, 1, 2), bi(4, 1, 2), bi(4, 1, 2), bi(16, 1, 2), bi(2, 6, 2), bi(2, 6, 2), bi(4, 6, 2), bi(4, 4, 1),
    bi(4, 2, 1), bi(4, 1, 1), bi(4, 1, 2), bi(4, 1, 1), bi(4, 1, 1), bi(4, 1, 1), bi(4, 1, 1), bi(4, 2, 1),
    bi(4, 2, 1), bi(2, 1, 1), bi(2, 1, 1), bi(4, 2, 1), bi(4, 2, 1), bi(4, 1, 1), bi(4, 1, 1), bi(4, 1, 1),
    bi(4, 1, 1),
];

pub const SCORPIO: [BlockInfo; BLOCKS] = [
    bi(2, 1, 1), bi(2, 1, 1), bi(2, 1, 1), bi(2, 1, 1), bi(2, 1, 1), bi(4, 2, 4), bi(4, 2, 4), bi(4, 1, 4),
    bi(8, 1, 4), bi(8, 1, 4), bi(4, 1, 4), bi(16, 1, 4), bi(5, 10, 4), bi(2, 10, 4), bi(8, 10, 4), bi(4, 8, 1),
    bi(4, 2, 1), bi(4, 1, 1), bi(4, 1, 4), bi(4, 2, 1), bi(4, 1, 1), bi(4, 1, 1), bi(4, 1, 1), bi(4, 2, 1),
    bi(4, 2, 1), bi(2, 1, 1), bi(2, 1, 1), bi(4, 6, 1), bi(4, 4, 1), bi(4, 1, 1), bi(4, 1, 1), bi(4, 1, 1),
    bi(4, 1, 1),
];

pub const DESC_OFFSET: [u32; BLOCKS] = [
    0x10, 0x18, 0x20, 0x28, 0x30, 0x38, 0x48, 0x58, 0x68, 0x88, 0xa8, 0xb8, 0xf8, 0x10c, 0x114, 0x134, 0x144,
    0x154, 0x164, 0x174, 0x184, 0x194, 0x1a4, 0x1b4, 0x1c4, 0x1d4, 0x1dc, 0x1e4, 0x1f4, 0x204, 0x214, 0x224,
    0x234,
];

pub const RESULT_BASE: [u32; BLOCKS] = [
    0x8, 0x18, 0x28, 0x38, 0x48, 0x58, 0x158, 0x258, 0x2d8, 0x3d8, 0x4d8, 0x558, 0x758, 0xd98, 0x1018, 0x1a18,
    0x1b18, 0x1b58, 0x1b78, 0x1bf8, 0x1c38, 0x1c58, 0x1c78, 0x1c98, 0x1cd8, 0x1d18, 0x1d28, 0x1d38, 0x1df8,
    0x1e78, 0x1e98, 0x1eb8, 0x1ed8,
];

pub const RESULT_STRIDE: [u32; BLOCKS] = [
    8, 8, 8, 8, 8, 0x40, 0x40, 0x20, 0x20, 0x20, 0x20, 0x20, 0x140, 0x140, 0x140, 0x40, 0x10, 8, 0x20, 0x10, 8,
    8, 8, 0x10, 0x10, 8, 8, 0x30, 0x20, 8, 8, 8, 8,
];

pub const BLOCK_NAMES: [&str; BLOCKS] = [
    "GRBM", "SRBM", "CPF", "CPG", "CPC", "CB", "DB", "PA_SU", "PA_SC", "SX", "SPI", "SQ", "TA", "TD", "TCP", "TCC",
    "TCA", "GDS", "VGT", "IA", "WD", "MC_MCBVM", "MMIO_0x4E6", "MC_MCDVM", "MMIO_0x4EA", "MC_VM_L2", "MMIO_0x4EE",
    "MC_ARB", "MC_CITF", "MC_HUB", "MMIO_0xDE5", "MMIO_0xDE9", "MMIO_0xDED",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcSource {
    ConsoleClock { hz: u64 },
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CounterEntry {
    pub block: u8,
    pub slot: u8,
    pub select: u32,
    pub instances: u16,
    pub engines: u16,
    pub wide: bool,
    pub sample_offset: u32,
    pub result_offset: u32,
    pub source: PcSource,
}

impl CounterEntry {
    pub fn values(&self) -> u32 {
        if self.wide { self.instances as u32 } else { self.instances as u32 * self.engines as u32 }
    }
    pub fn bytes(&self) -> u32 {
        8 * self.instances as u32 * self.engines as u32
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterSet {
    pub scorpio: bool,
    pub sq_ctrl: u16,
    pub se_mask: u8,
    pub entries: Vec<CounterEntry>,
    pub sample_bytes: u32,
}

pub(crate) fn rd32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

pub fn clock_hz(device_type: u32) -> u64 {
    match device_type {
        3 => DURANGO_S_HZ,
        4 | 5 => SCORPIO_HZ,
        _ => DURANGO_HZ,
    }
}

pub fn scorpio_for(device_type: u32) -> bool {
    crate::console_slots_dev2::kmd_info(device_type).mode == 1
}

pub fn pc_source_for(device_type: u32, block: u8, select: u32) -> PcSource {
    let always = match block {
        0 => select & 0x3f == 0,
        2..=4 => select & 0x3ff == 0,
        _ => false,
    };
    if always { PcSource::ConsoleClock { hz: clock_hz(device_type) } } else { PcSource::None }
}

pub fn pc_source(scorpio: bool, block: u8, select: u32) -> PcSource {
    let t = device_type();
    let t = if scorpio_for(t) == scorpio { t } else if scorpio { 4 } else { 2 };
    pc_source_for(t, block, select)
}

pub fn build(desc: &[u8; DESC_BYTES], device_type: u32) -> CounterSet {
    let scorpio = scorpio_for(device_type);
    let table = if scorpio { &SCORPIO } else { &DURANGO };
    let sq = rd32(desc, 0xc);
    let mask = rd32(desc, 8);
    let mut set = CounterSet {
        scorpio,
        sq_ctrl: if sq == 0 { 0xffff } else { sq as u16 },
        se_mask: if mask == 0 { 0xf } else { mask as u8 },
        entries: Vec::new(),
        sample_bytes: 0,
    };
    for b in 0..BLOCKS {
        let info = table[b];
        let base = DESC_OFFSET[b] as usize;
        let mut used: u8 = 0;
        let mut out = RESULT_BASE[b];
        for k in 0..info.slots as usize {
            let sel = rd32(desc, base + 4 * k);
            let skip = sel == u32::MAX || (b as u8 == SQ_BLOCK && k > 0 && used != 0 && rd32(desc, base + 4 * (k - 1)) == u32::MAX);
            if !skip {
                if used >= info.slots {
                    continue;
                }
                let e = CounterEntry {
                    block: b as u8,
                    slot: used,
                    select: sel,
                    instances: info.instances,
                    engines: info.engines,
                    wide: b as u8 >= FIRST_WIDE_BLOCK,
                    sample_offset: set.sample_bytes,
                    result_offset: out,
                    source: pc_source_for(device_type, b as u8, sel),
                };
                set.sample_bytes += e.bytes();
                set.entries.push(e);
                used += 1;
            }
            out += RESULT_STRIDE[b];
        }
    }
    set
}

pub fn resolve_into(set: &CounterSet, raw: impl Fn(&CounterEntry, u32) -> Option<u64>, out: &mut [u8; RESULT_BYTES as usize]) {
    out[0..4].copy_from_slice(&RESULT_BYTES.to_le_bytes());
    out[4..8].copy_from_slice(&RESULT_VERSION.to_le_bytes());
    out[8..].fill(0xff);
    for e in &set.entries {
        for i in 0..e.bytes() / 8 {
            if let Some(v) = raw(e, i) {
                let o = (e.result_offset + 8 * i) as usize;
                out[o..o + 8].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
}

static QI_IIDS: [Guid; 52] = [
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
    Guid { d1: 0x0ec870a6, d2: 0x5d7e, d3: 0x4c22, d4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed] },
    Guid { d1: 0xc54a6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14] },
    Guid { d1: 0x6b3b2502, d2: 0x6e51, d3: 0x45b3, d4: [0x90, 0xee, 0x98, 0x84, 0x26, 0x5e, 0x8d, 0xf3] },
    Guid { d1: 0x696442be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] },
    Guid { d1: 0x6102dee4, d2: 0xaf59, d3: 0x4b09, d4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24] },
    Guid { d1: 0x0a753dcf, d2: 0xc4d8, d3: 0x4b91, d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76] },
    Guid { d1: 0x433685fe, d2: 0xe22b, d3: 0x4ca0, d4: [0xa8, 0xdb, 0xb5, 0xb4, 0xf4, 0xdd, 0x0e, 0x4a] },
    Guid { d1: 0x765a30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] },
    Guid { d1: 0x8efb471d, d2: 0x616c, d3: 0x4f49, d4: [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51] },
    Guid { d1: 0x0d9658ae, d2: 0xed45, d3: 0x469e, d4: [0xa6, 0x1d, 0x97, 0x0e, 0xc5, 0x83, 0xca, 0xb4] },
    Guid { d1: 0xc36a797c, d2: 0xec80, d3: 0x4f0a, d4: [0x89, 0x85, 0xa7, 0xb2, 0x47, 0x50, 0x82, 0xd1] },
    Guid { d1: 0xae02eedb, d2: 0xc735, d3: 0x4690, d4: [0x8d, 0x52, 0x5a, 0x8d, 0xc2, 0x02, 0x13, 0xaa] },
    Guid { d1: 0xdb6f6ddb, d2: 0xac77, d3: 0x4e88, d4: [0x82, 0x53, 0x81, 0x9d, 0xf9, 0xbb, 0xf1, 0x40] },
    Guid { d1: 0xa04bfb29, d2: 0x08ef, d3: 0x43d6, d4: [0xa4, 0x9c, 0xa9, 0xbd, 0xbd, 0xcb, 0xe6, 0x86] },
    Guid { d1: 0x9d06dffa, d2: 0xd1e5, d3: 0x4d07, d4: [0x83, 0xa8, 0x1b, 0xb1, 0x23, 0xf2, 0xf8, 0x41] },
    Guid { d1: 0x177700f9, d2: 0x876a, d3: 0x4436, d4: [0xb3, 0x68, 0x36, 0xa6, 0x04, 0xf8, 0x2c, 0xef] },
    Guid { d1: 0x77acce80, d2: 0x638e, d3: 0x4e65, d4: [0x88, 0x95, 0xc1, 0xf2, 0x33, 0x86, 0x86, 0x3e] },
    Guid { d1: 0x30baa41e, d2: 0xb15b, d3: 0x475c, d4: [0xa0, 0xbb, 0x1a, 0xf5, 0xc5, 0xb6, 0x43, 0x28] },
    Guid { d1: 0x4609acf9, d2: 0xab9d, d3: 0x4392, d4: [0x97, 0x1d, 0xa5, 0xe2, 0x59, 0xca, 0x4d, 0x98] },
    Guid { d1: 0x547e33c7, d2: 0xff86, d3: 0x4cd9, d4: [0xbe, 0xa3, 0x5d, 0x4a, 0x28, 0x37, 0x53, 0x96] },
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
    Guid { d1: 0x9598d782, d2: 0x875d, d3: 0x4fa0, d4: [0xa0, 0x8e, 0xdc, 0x25, 0xec, 0x17, 0xd0, 0x2c] },
];

fn device_type() -> u32 {
    compat_common::gdk::identity::device_type_value()
}

pub enum Private {
    Data(Vec<u8>),
    Iface(usize),
}

pub(crate) enum Layout {
    One(Arc<CounterSet>),
    Xs(Arc<crate::perfctr_xs::XsCounterSet>),
}

#[repr(C)]
pub(crate) struct SetObj {
    pub(crate) vtable: *const usize,
    pub(crate) refs: AtomicU32,
    pub(crate) device: usize,
    pub(crate) layout: Layout,
    pub(crate) private: Mutex<Vec<(Guid, Private)>>,
}

fn live() -> &'static Mutex<HashMap<usize, Arc<CounterSet>>> {
    static M: OnceLock<Mutex<HashMap<usize, Arc<CounterSet>>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn counter_set_for(ptr: usize) -> Option<Arc<CounterSet>> {
    live().lock().unwrap_or_else(|e| e.into_inner()).get(&ptr).cloned()
}

pub fn is_live(ptr: usize) -> bool {
    live().lock().unwrap_or_else(|e| e.into_inner()).contains_key(&ptr)
}

static READER: AtomicUsize = AtomicUsize::new(0);

pub fn set_value_reader(f: fn(usize, u64) -> Option<u64>) {
    READER.store(f as *const () as usize, Ordering::Release);
}

pub(crate) fn reader() -> Option<fn(usize, u64) -> Option<u64>> {
    let p = READER.load(Ordering::Acquire);
    if p == 0 { None } else { Some(unsafe { core::mem::transmute::<usize, fn(usize, u64) -> Option<u64>>(p) }) }
}

unsafe fn vslot(obj: usize, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

pub(crate) unsafe fn com_addref(obj: usize) -> u32 {
    let f: unsafe extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(vslot(obj, 1)) };
    unsafe { f(obj) }
}

unsafe fn com_release(obj: usize) -> u32 {
    let f: unsafe extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(vslot(obj, 2)) };
    unsafe { f(obj) }
}

pub(crate) fn guid_at(p: *const Guid) -> Option<Guid> {
    if p.is_null() { None } else { Some(unsafe { core::ptr::read_unaligned(p) }) }
}

unsafe extern "system" fn set_qi(this: *mut SetObj, riid: *const Guid, ppv: *mut usize) -> i32 {
    let hit = guid_at(riid).is_some_and(|g| QI_IIDS.iter().any(|k| *k == g));
    if hit {
        unsafe { set_addref(this) };
    }
    if !ppv.is_null() {
        unsafe { *ppv = if hit { this as usize } else { 0 } };
    }
    if hit { S_OK } else { E_NOINTERFACE }
}

pub(crate) unsafe extern "system" fn set_addref(this: *mut SetObj) -> u32 {
    let o = unsafe { &*this };
    unsafe { com_addref(o.device) };
    o.refs.fetch_add(1, Ordering::AcqRel) + 1
}

pub(crate) unsafe extern "system" fn set_release(this: *mut SetObj) -> u32 {
    let o = unsafe { &*this };
    let n = o.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    let device = o.device;
    if n == 0 {
        live().lock().unwrap_or_else(|e| e.into_inner()).remove(&(this as usize));
        crate::perfctr_xs::forget(this as usize);
        let b = unsafe { Box::from_raw(this) };
        let items = core::mem::take(&mut *b.private.lock().unwrap_or_else(|e| e.into_inner()));
        for (_, p) in items {
            if let Private::Iface(i) = p {
                unsafe { com_release(i) };
            }
        }
        drop(b);
    }
    unsafe { com_release(device) };
    n
}

pub fn get_private(items: &[(Guid, Private)], g: &Guid, size: &mut u32, data: *mut u8) -> i32 {
    match items.iter().find(|(k, _)| k == g) {
        None => {
            *size = 0;
            PRIVATE_DATA_NOT_FOUND
        }
        Some((_, Private::Iface(i))) => {
            unsafe { com_addref(*i) };
            if !data.is_null() && *size >= 8 {
                unsafe { core::ptr::write_unaligned(data as *mut usize, *i) };
            }
            *size = 8;
            S_OK
        }
        Some((_, Private::Data(v))) => {
            if !data.is_null() && *size != 0 {
                let n = (*size as usize).min(v.len());
                unsafe { core::ptr::copy_nonoverlapping(v.as_ptr(), data, n) };
            }
            *size = v.len() as u32;
            S_OK
        }
    }
}

pub fn put_private(items: &mut Vec<(Guid, Private)>, g: Guid, p: Option<Private>) -> i32 {
    let at = items.iter().position(|(k, _)| *k == g);
    let old = match (at, p) {
        (Some(i), Some(p)) => Some(core::mem::replace(&mut items[i].1, p)),
        (None, Some(p)) => {
            items.insert(0, (g, p));
            None
        }
        (Some(i), None) => Some(items.remove(i).1),
        (None, None) => return PRIVATE_DATA_NOT_FOUND,
    };
    if let Some(Private::Iface(i)) = old {
        unsafe { com_release(i) };
    }
    S_OK
}

unsafe extern "system" fn set_get_private(this: *mut SetObj, g: *const Guid, size: *mut u32, data: *mut u8) -> i32 {
    let (Some(g), false) = (guid_at(g), size.is_null()) else {
        record("d3d12_xs", "perfctr", "GetPrivateData with a null guid or size: the console dereferences it and faults");
        return E_POINTER;
    };
    let o = unsafe { &*this };
    let items = o.private.lock().unwrap_or_else(|e| e.into_inner());
    get_private(&items, &g, unsafe { &mut *size }, data)
}

unsafe extern "system" fn set_set_private(this: *mut SetObj, g: *const Guid, size: u32, data: *const u8) -> i32 {
    let Some(g) = guid_at(g) else {
        record("d3d12_xs", "perfctr", "SetPrivateData with a null guid: the console dereferences it and faults");
        return E_POINTER;
    };
    let o = unsafe { &*this };
    let p = if data.is_null() { None } else { Some(Private::Data(unsafe { core::slice::from_raw_parts(data, size as usize) }.to_vec())) };
    put_private(&mut o.private.lock().unwrap_or_else(|e| e.into_inner()), g, p)
}

pub(crate) unsafe extern "system" fn set_set_private_iface(this: *mut SetObj, g: *const Guid, iface: usize) -> i32 {
    let Some(g) = guid_at(g) else {
        record("d3d12_xs", "perfctr", "SetPrivateDataInterface with a null guid: the console dereferences it and faults");
        return E_POINTER;
    };
    let o = unsafe { &*this };
    let p = if iface == 0 {
        None
    } else {
        unsafe { com_addref(iface) };
        Some(Private::Iface(iface))
    };
    put_private(&mut o.private.lock().unwrap_or_else(|e| e.into_inner()), g, p)
}

pub(crate) unsafe extern "system" fn set_set_name(_this: *mut SetObj, _name: *const u16) -> i32 {
    S_OK
}

pub(crate) unsafe extern "system" fn set_get_device(this: *mut SetObj, _riid: *const Guid, ppv: *mut usize) -> i32 {
    let o = unsafe { &*this };
    let r = unsafe { com_addref(o.device) };
    if !ppv.is_null() {
        unsafe { *ppv = o.device };
    }
    r as i32
}

pub(crate) unsafe extern "system" fn set_sample_size(this: *mut SetObj) -> u64 {
    match &unsafe { &*this }.layout {
        Layout::One(s) => s.sample_bytes as u64,
        Layout::Xs(s) => s.sample_bytes as u64,
    }
}

unsafe fn resource_width(res: usize) -> u64 {
    let mut desc = [0u8; 0x38];
    let f: unsafe extern "system" fn(usize, *mut u8) -> *mut u8 = unsafe { core::mem::transmute(vslot(res, 10)) };
    unsafe { f(res, desc.as_mut_ptr()) };
    if rd32(&desc, 0) == 1 { u64::from_le_bytes(desc[8..16].try_into().unwrap()) } else { 0 }
}

pub(crate) unsafe fn map_read(res: usize, offset: u64, len: u64) -> Option<Vec<u8>> {
    let width = unsafe { resource_width(res) };
    if width == 0 || offset >= width {
        return None;
    }
    let end = (offset + len).min(width);
    let range: [usize; 2] = [offset as usize, end as usize];
    let map: unsafe extern "system" fn(usize, u32, *const [usize; 2], *mut *mut u8) -> i32 = unsafe { core::mem::transmute(vslot(res, 8)) };
    let mut p: *mut u8 = core::ptr::null_mut();
    if unsafe { map(res, 0, &range, &mut p) } < 0 || p.is_null() {
        return None;
    }
    let v = unsafe { core::slice::from_raw_parts(p.add(offset as usize), (end - offset) as usize) }.to_vec();
    let none: [usize; 2] = [0, 0];
    let unmap: unsafe extern "system" fn(usize, u32, *const [usize; 2]) = unsafe { core::mem::transmute(vslot(res, 9)) };
    unsafe { unmap(res, 0, &none) };
    Some(v)
}

pub fn raw_value(bytes: Option<&[u8]>, rd: Option<fn(usize, u64) -> Option<u64>>, res: usize, offset: u64, e: &CounterEntry, i: u32) -> Option<u64> {
    if e.source == PcSource::None {
        return None;
    }
    let at = e.sample_offset as u64 + 8 * i as u64;
    rd.and_then(|f| f(res, offset + at)).or_else(|| {
        let a = at as usize;
        bytes.and_then(|b| b.get(a..a + 8)).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
    })
}

static RESOLVE_LOG: AtomicUsize = AtomicUsize::new(0);

pub(crate) unsafe extern "system" fn set_resolve(this: *mut SetObj, res: usize, offset: u64, out: *mut u8) -> i32 {
    if res == 0 || out.is_null() {
        record("d3d12_xs", "perfctr", "ResolveSamples with a null resource or output: the console dereferences it and faults");
        return E_POINTER;
    }
    let set = match &unsafe { &*this }.layout {
        Layout::One(s) => s.clone(),
        Layout::Xs(s) => return unsafe { crate::perfctr_xs::resolve_object(s, res, offset, out) },
    };
    let dst = unsafe { &mut *(out as *mut [u8; RESULT_BYTES as usize]) };
    let mapped = unsafe { resolve_resource(&set, res, offset, dst, reader()) };
    if RESOLVE_LOG.fetch_add(1, Ordering::Relaxed) < 4 {
        let none = set.entries.iter().filter(|e| e.source == PcSource::None).count();
        record("d3d12_xs", "perfctr", &format!("ResolveSamples {} entries, {} bytes, cpu-mapped={mapped}, {none} entries without a PC source", set.entries.len(), set.sample_bytes));
    }
    S_OK
}

pub unsafe fn resolve_resource(set: &CounterSet, res: usize, offset: u64, out: &mut [u8; RESULT_BYTES as usize], rd: Option<fn(usize, u64) -> Option<u64>>) -> bool {
    let bytes = unsafe { map_read(res, offset, set.sample_bytes as u64) };
    resolve_into(set, |e, i| raw_value(bytes.as_deref(), rd, res, offset, e, i), out);
    bytes.is_some()
}

fn set_vtable() -> *const usize {
    static VT: OnceLock<[usize; 10]> = OnceLock::new();
    VT.get_or_init(|| {
        [
            set_qi as *const () as usize,
            set_addref as *const () as usize,
            set_release as *const () as usize,
            set_get_private as *const () as usize,
            set_set_private as *const () as usize,
            set_set_private_iface as *const () as usize,
            set_set_name as *const () as usize,
            set_get_device as *const () as usize,
            set_sample_size as *const () as usize,
            set_resolve as *const () as usize,
        ]
    })
    .as_ptr()
}

pub(crate) fn create_object(device: usize, layout: Layout, vtable: *const usize) -> usize {
    let o = Box::new(SetObj { vtable, refs: AtomicU32::new(1), device, layout, private: Mutex::new(Vec::new()) });
    let p = Box::into_raw(o) as usize;
    unsafe { com_addref(device) };
    p
}

pub fn create(device: usize, set: CounterSet) -> usize {
    let set = Arc::new(set);
    let p = create_object(device, Layout::One(set.clone()), set_vtable());
    live().lock().unwrap_or_else(|e| e.into_inner()).insert(p, set);
    p
}

pub fn retail_counters_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let p = "d:\\D3D__RetailGpuCounters";
        compat_common::gdk::fs::translate(p).is_some_and(|t| std::fs::metadata(&t).is_ok())
    })
}

static CREATE_LOG: AtomicUsize = AtomicUsize::new(0);

pub unsafe fn create_counter_set(device_type: u32, this: usize, desc: *const u8, ppv: *mut usize) -> i32 {
    if crate::console_slots_dev2::family(device_type) == crate::console_slots_dev2::Family::Scarlett {
        return unsafe { crate::perfctr_xs::create_counter_set_xs(retail_counters_enabled(), device_type, this, desc, ppv) };
    }
    if desc.is_null() {
        record("d3d12_xs", "perfctr", "CreateCounterSetX with a null desc: the console dereferences it and faults");
        return E_INVALIDARG;
    }
    let mut d = [0u8; DESC_BYTES];
    unsafe { core::ptr::copy_nonoverlapping(desc, d.as_mut_ptr(), DESC_BYTES) };
    let set = build(&d, device_type);
    if CREATE_LOG.fetch_add(1, Ordering::Relaxed) < 8 {
        let list: Vec<String> = set.entries.iter().map(|e| format!("{}[{}]={:#x}{}", BLOCK_NAMES[e.block as usize], e.slot, e.select, if e.source == PcSource::None { "" } else { "*" })).collect();
        record("d3d12_xs", "perfctr", &format!("CreateCounterSetX scorpio={} sq_ctrl={:#x} mask={:#x} sample={} bytes: {}", set.scorpio, set.sq_ctrl, set.se_mask, set.sample_bytes, list.join(" ")));
    }
    let p = create(this, set);
    if !ppv.is_null() {
        unsafe { *ppv = p };
    }
    S_OK
}

unsafe extern "system" fn create_counter_set_x(this: *mut c_void, desc: *const u8, _riid: *const Guid, ppv: *mut usize) -> i32 {
    unsafe { create_counter_set(device_type(), this as usize, desc, ppv) }
}

pub fn install_device(v: &mut [usize]) {
    if DEVICE_SLOT < v.len() {
        v[DEVICE_SLOT] = create_counter_set_x as *const () as usize;
    }
}
