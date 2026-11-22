use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

pub const ERROR_OUTOFMEMORY: u32 = 0xe;
pub const ERROR_BAD_LENGTH: u32 = 0x18;
pub const ERROR_INVALID_PARAMETER: u32 = 0x57;
pub const ERROR_INVALID_FLAGS: u32 = 0x3ec;
pub const EXTENSION_FLAG: u32 = 0x8000_0000;
pub const EVENT_TRACE_CONTROL_UPDATE: u32 = 2;

pub const OFF_BUFFER_SIZE: usize = 0x00;
pub const OFF_GUID: usize = 0x18;
pub const OFF_ENABLE_FLAGS: usize = 0x48;
pub const OFF_STATS: usize = 0x50;
pub const OFF_LOG_FILE_NAME: usize = 0x70;
pub const OFF_LOGGER_NAME: usize = 0x74;
pub const PROPS_MIN: usize = 0x78;

pub const SYSTEM_TRACE_PROVIDER: [u8; 16] = guid(0x3b9c9951, 0x3480, 0x4220, [0x93, 0x77, 0x9c, 0x8e, 0x51, 0x84, 0xf5, 0xcd]);
pub const HEAP_GUID: [u8; 16] = guid(0x222962ab, 0x6180, 0x4b88, [0xa8, 0x25, 0x34, 0x6b, 0x75, 0xf2, 0xa2, 0x4a]);
pub const KERNEL_LOGGER: &str = "NT Kernel Logger";

pub const GROUP_MAP: [(u32, u32); 8] = [
    (0x10, 0x2000_0004),
    (0x20, 0x2000_0080),
    (0x40, 0x2000_4000),
    (0x80, 0x4000_0040),
    (0x80_0000, 0x2000_0010),
    (0x100_0000, 0x2000_0002),
    (0x4000, 0x2000_8000),
    (0x800, 0x2000_0200),
];

pub const CLASSES: [[u8; 16]; 32] = [
    guid(0x68fdd900, 0x4a3e, 0x11d1, [0x84, 0xf4, 0x00, 0x00, 0xf8, 0x04, 0x64, 0xe3]),
    guid(0x3d6fa8d4, 0xfe05, 0x11d0, [0x9d, 0xda, 0x00, 0xc0, 0x4f, 0xd7, 0xba, 0x7c]),
    guid(0x3d6fa8d3, 0xfe05, 0x11d0, [0x9d, 0xda, 0x00, 0xc0, 0x4f, 0xd7, 0xba, 0x7c]),
    guid(0x3d6fa8d0, 0xfe05, 0x11d0, [0x9d, 0xda, 0x00, 0xc0, 0x4f, 0xd7, 0xba, 0x7c]),
    guid(0x90cbdc39, 0x4a3e, 0x11d1, [0x84, 0xf4, 0x00, 0x00, 0xf8, 0x04, 0x64, 0xe3]),
    guid(0x3d6fa8d1, 0xfe05, 0x11d0, [0x9d, 0xda, 0x00, 0xc0, 0x4f, 0xd7, 0xba, 0x7c]),
    guid(0x9a280ac0, 0xc8e0, 0x11d1, [0x84, 0xe2, 0x00, 0xc0, 0x4f, 0xb9, 0x98, 0xa2]),
    guid(0x3282fc76, 0xfeed, 0x498e, [0x8a, 0xa7, 0xe7, 0x0f, 0x45, 0x9d, 0x43, 0x0e]),
    guid(0xbf3a50c5, 0xa9c9, 0x4988, [0xa0, 0x05, 0x2d, 0xf0, 0xb7, 0xc8, 0x0f, 0x80]),
    guid(0xae53722e, 0xc863, 0x11d2, [0x86, 0x59, 0x00, 0xc0, 0x4f, 0xa3, 0x21, 0xa1]),
    guid(0x13976d09, 0xa327, 0x438c, [0x95, 0x0b, 0x7f, 0x03, 0x19, 0x28, 0x15, 0xc7]),
    guid(0x01853a65, 0x418f, 0x4f36, [0xae, 0xfc, 0xdc, 0x0f, 0x1d, 0x2f, 0xd2, 0x35]),
    guid(0x99134383, 0x5248, 0x43fc, [0x83, 0x4b, 0x52, 0x94, 0x54, 0xe7, 0x5d, 0xf3]),
    guid(0x42695762, 0xea50, 0x497a, [0x90, 0x68, 0x5c, 0xbb, 0xb3, 0x5e, 0x0b, 0x95]),
    guid(0x0268a8b6, 0x74fd, 0x4302, [0x9d, 0xd0, 0x6e, 0x8f, 0x17, 0x95, 0xc0, 0xcf]),
    guid(0xce1dbfb4, 0x137e, 0x4da6, [0x87, 0xb0, 0x3f, 0x59, 0xaa, 0x10, 0x2c, 0xbc]),
    guid(0x222962ab, 0x6180, 0x4b88, [0xa8, 0x25, 0x34, 0x6b, 0x75, 0xf2, 0xa2, 0x4a]),
    guid(0x89497f50, 0xeffe, 0x4440, [0x8c, 0xf2, 0xce, 0x6b, 0x1c, 0xdc, 0xac, 0xa7]),
    guid(0xe43445e0, 0x0903, 0x48c3, [0xb8, 0x78, 0xff, 0x0f, 0xcc, 0xeb, 0xdd, 0x04]),
    guid(0xa9152f00, 0x3f58, 0x4bee, [0x92, 0xa1, 0x70, 0xc7, 0xd0, 0x79, 0xd5, 0xdd]),
    guid(0x2cb15d1d, 0x5fc1, 0x11d2, [0xab, 0xe1, 0x00, 0xa0, 0xc9, 0x11, 0xf5, 0x18]),
    guid(0xb2d14872, 0x7c5b, 0x463d, [0x84, 0x19, 0xee, 0x9b, 0xf7, 0xd2, 0x3e, 0x04]),
    guid(0x7687a439, 0xf752, 0x45b8, [0xb7, 0x41, 0x32, 0x1a, 0xec, 0x0f, 0x8d, 0xf9]),
    guid(0x3ac66736, 0xcc59, 0x4cff, [0x81, 0x15, 0x8d, 0xf5, 0x0e, 0x39, 0x81, 0x6b]),
    guid(0xdef2fe46, 0x7bd6, 0x4b80, [0xbd, 0x94, 0xf5, 0x7f, 0xe2, 0x0d, 0x0c, 0xe3]),
    guid(0x9aec974b, 0x5b8e, 0x4118, [0x9b, 0x92, 0x31, 0x86, 0xd8, 0x00, 0x2c, 0xe5]),
    guid(0x45d8cccd, 0x539f, 0x4b72, [0xa8, 0xb7, 0x5c, 0x68, 0x31, 0x42, 0x60, 0x9a]),
    guid(0xd837ca92, 0x12b9, 0x44a5, [0xad, 0x6a, 0x3a, 0x65, 0xb3, 0x57, 0x8a, 0xa8]),
    guid(0xc861d0e2, 0xa2c1, 0x4d36, [0x9f, 0x9c, 0x97, 0x0b, 0xab, 0x94, 0x3a, 0x12]),
    guid(0x7f2a405c, 0x69b5, 0x4bf9, [0xa1, 0xf5, 0x30, 0xe8, 0xf1, 0xaf, 0xab, 0x5e]),
    guid(0x2ce9a149, 0xeffe, 0x42f0, [0xa6, 0x35, 0xa1, 0xd3, 0x9e, 0x26, 0xc8, 0xf2]),
    guid(0xff1fd2fd, 0x6008, 0x42bb, [0x9e, 0x75, 0x00, 0xa2, 0x00, 0x51, 0xf3, 0xbe]),
];

pub const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClassicEventId {
    pub guid: [u8; 16],
    pub kind: u8,
    pub reserved: [u8; 7],
}

#[repr(C)]
struct OsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    csd: [u16; 128],
}

#[link(name = "kernel32")]
extern "system" {
    fn GetVersionExW(v: *mut OsVersionInfoW) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn StartTraceW(h: *mut u64, name: *const u16, p: *mut c_void) -> u32;
    fn ControlTraceW(h: u64, name: *const u16, p: *mut c_void, code: u32) -> u32;
    fn EnableTraceEx(provider: *const [u8; 16], source: *const [u8; 16], h: u64, enable: u32, level: u8, any: u64, all: u64, prop: u32, filter: *mut c_void) -> u32;
}

static OBJECTS: AtomicU32 = AtomicU32::new(0);

pub fn os_version() -> (u32, u32) {
    let mut v = OsVersionInfoW { size: core::mem::size_of::<OsVersionInfoW>() as u32, major: 0, minor: 0, build: 0, platform: 0, csd: [0; 128] };
    if unsafe { GetVersionExW(&mut v) } == 0 {
        return (0, 0);
    }
    (v.major, v.minor)
}

pub fn ext_limit() -> u32 {
    let (ma, mi) = os_version();
    if ma < 6 || (ma == 6 && mi < 1) {
        0x10
    } else {
        0x100
    }
}

pub struct Props(pub Vec<u64>);

impl Props {
    pub fn zeroed(size: usize) -> Props {
        Props(vec![0u64; size.div_ceil(8)])
    }
    pub fn bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.0.as_ptr() as *const u8, self.0.len() * 8) }
    }
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.0.as_mut_ptr() as *mut u8, self.0.len() * 8) }
    }
    pub fn ptr(&mut self) -> *mut c_void {
        self.0.as_mut_ptr() as *mut c_void
    }
    pub fn u32_at(&self, o: usize) -> u32 {
        u32::from_le_bytes(self.bytes()[o..o + 4].try_into().unwrap())
    }
    pub fn u16_at(&self, o: usize) -> u16 {
        u16::from_le_bytes(self.bytes()[o..o + 2].try_into().unwrap())
    }
    pub fn set_u32(&mut self, o: usize, v: u32) {
        self.bytes_mut()[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    pub fn set_u16(&mut self, o: usize, v: u16) {
        self.bytes_mut()[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }
}

unsafe fn rd32(p: *const u8, o: usize) -> u32 {
    (p.add(o) as *const u32).read_unaligned()
}

pub fn hook_ids(events: &[ClassicEventId], limit: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for e in events {
        if let Some(i) = CLASSES.iter().position(|g| *g == e.guid) {
            if out.len() as u32 >= limit {
                break;
            }
            out.push(((i as u32) << 8) | e.kind as u32);
        }
    }
    out
}

pub fn group_mask(flags: u32) -> [u32; 8] {
    let mut m = [0u32; 8];
    for bit in 0..32 {
        let f = 1u32 << bit;
        if flags & f == 0 {
            continue;
        }
        let v = GROUP_MAP.iter().find(|(k, _)| k & f != 0).map(|(_, v)| *v).unwrap_or(f);
        m[(v >> 29) as usize] |= v & 0x1fff_ffff;
    }
    m
}

pub fn add_item(p: &mut Props, data: &[u8], kind: u16) -> Result<(), u32> {
    let end = p.u32_at(OFF_BUFFER_SIZE) as usize;
    if (p.u32_at(OFF_ENABLE_FLAGS) as i32) >= 0 || p.u16_at(OFF_ENABLE_FLAGS) == 0 {
        return Err(ERROR_INVALID_FLAGS);
    }
    let hdr = p.u16_at(OFF_ENABLE_FLAGS) as usize;
    if hdr > end || hdr + 4 > end {
        return Err(ERROR_BAD_LENGTH);
    }
    let mut at = hdr + 4;
    for _ in 0..p.u16_at(hdr + 2) {
        let n = p.u16_at(at) as usize;
        if n == 0 {
            return Err(ERROR_INVALID_FLAGS);
        }
        at += n * 4;
        if at > end {
            return Err(ERROR_BAD_LENGTH);
        }
    }
    let dwords = ((data.len() + 4) >> 2) as u16 + (data.len() & 3 != 0) as u16;
    p.set_u16(at + 2, kind);
    p.set_u16(at, dwords);
    at += 4;
    if at > end {
        return Err(ERROR_BAD_LENGTH);
    }
    p.bytes_mut()[at..at + data.len()].copy_from_slice(data);
    let s = p.u16_at(hdr);
    p.set_u16(hdr, s + dwords);
    let c = p.u16_at(hdr + 2);
    p.set_u16(hdr + 2, c + 1);
    Ok(())
}

fn words(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub unsafe fn build(props: *const u8, stack: &[ClassicEventId], pids: &[u32], zero_guid: bool) -> Result<Props, u32> {
    let ids = hook_ids(stack, ext_limit());
    let size0 = rd32(props, OFF_BUFFER_SIZE) as usize;
    let flags = rd32(props, OFF_ENABLE_FLAGS);
    let mut size = size0 + 4;
    if flags != 0 {
        size += 0x24;
    }
    if !ids.is_empty() {
        size += ids.len() * 4 + 4;
    }
    if !pids.is_empty() {
        size += pids.len() * 4 + 4;
    }
    let mut p = Props::zeroed(size);
    core::ptr::copy_nonoverlapping(props, p.bytes_mut().as_mut_ptr(), size0);
    p.set_u32(OFF_BUFFER_SIZE, size as u32);
    if zero_guid {
        p.bytes_mut()[OFF_GUID..OFF_GUID + 16].fill(0);
    }
    p.set_u32(OFF_ENABLE_FLAGS, EXTENSION_FLAG);
    p.set_u16(OFF_ENABLE_FLAGS, size0 as u16);
    p.bytes_mut()[OFF_ENABLE_FLAGS + 2] = 0xff;
    if size0 < size {
        p.set_u32(size0, 1);
        if flags != 0 {
            add_item(&mut p, &words(&group_mask(flags)), 1)?;
        }
        if !pids.is_empty() {
            add_item(&mut p, &words(pids), 2)?;
        }
        if !ids.is_empty() {
            add_item(&mut p, &words(&ids), 3)?;
        }
        if p.u16_at(size0 + 2) == 0 {
            p.set_u32(OFF_ENABLE_FLAGS, 0);
        }
    }
    Ok(p)
}

unsafe fn copy_stats(dst: *mut u8, src: &Props) {
    core::ptr::copy_nonoverlapping(src.bytes()[OFF_STATS..OFF_STATS + 0x20].as_ptr(), dst.add(OFF_STATS), 0x20);
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn slice_or_empty<'a, T>(p: *const T, n: u32) -> &'a [T] {
    if p.is_null() || n == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(p, n as usize)
    }
}

#[no_mangle]
pub unsafe extern "system" fn StartKernelTrace(h: *mut u64, props: *mut u8, stack: *const ClassicEventId, count: u32) -> u32 {
    if count > ext_limit() {
        return ERROR_INVALID_PARAMETER;
    }
    if (rd32(props, OFF_ENABLE_FLAGS) as i32) < 0 {
        return ERROR_INVALID_FLAGS;
    }
    let sys = core::slice::from_raw_parts(props.add(OFF_GUID), 16) == SYSTEM_TRACE_PROVIDER;
    let mut p = match build(props, slice_or_empty(stack, count), &[], sys) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let kl = wide(KERNEL_LOGGER);
    let name: *const u16 = if sys {
        let off = p.u32_at(OFF_LOGGER_NAME) as usize;
        if off == 0 {
            return ERROR_INVALID_PARAMETER;
        }
        if off > p.u32_at(OFF_BUFFER_SIZE) as usize {
            return ERROR_BAD_LENGTH;
        }
        p.bytes().as_ptr().add(off) as *const u16
    } else {
        kl.as_ptr()
    };
    let st = StartTraceW(h, name, p.ptr());
    if st != 0 {
        return st;
    }
    if sys {
        EnableTraceEx(&SYSTEM_TRACE_PROVIDER, core::ptr::null(), *h, 1, 0, rd32(props, OFF_ENABLE_FLAGS) as u64, 0, 0, core::ptr::null_mut());
    }
    copy_stats(props, &p);
    0
}

pub unsafe fn heap_trace(start: bool, h: *mut u64, props: *mut u8, name: *const u16, pids: *const u32, npids: u32, stack: *const ClassicEventId, nstack: u32) -> u32 {
    if nstack > ext_limit() || name.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    if (rd32(props, OFF_ENABLE_FLAGS) as i32) < 0 {
        return ERROR_INVALID_FLAGS;
    }
    if h.is_null() && start {
        return ERROR_INVALID_PARAMETER;
    }
    let mut p = match build(props, slice_or_empty(stack, nstack), slice_or_empty(pids, npids), false) {
        Ok(p) => p,
        Err(e) => return e,
    };
    p.bytes_mut()[OFF_GUID..OFF_GUID + 16].copy_from_slice(&HEAP_GUID);
    let st = if start {
        StartTraceW(h, name, p.ptr())
    } else {
        p.set_u32(OFF_LOG_FILE_NAME, 0);
        ControlTraceW(0, name, p.ptr(), EVENT_TRACE_CONTROL_UPDATE)
    };
    if st != 0 {
        return st;
    }
    copy_stats(props, &p);
    0
}

#[no_mangle]
pub unsafe extern "system" fn StartHeapTrace(h: *mut u64, props: *mut u8, name: *const u16, pids: *const u32, npids: u32, stack: *const ClassicEventId, nstack: u32) -> u32 {
    heap_trace(true, h, props, name, pids, npids, stack, nstack)
}

#[no_mangle]
pub unsafe extern "system" fn UpdateHeapTrace(props: *mut u8, name: *const u16, pids: *const u32, npids: u32, stack: *const ClassicEventId, nstack: u32) -> u32 {
    heap_trace(false, core::ptr::null_mut(), props, name, pids, npids, stack, nstack)
}

pub fn object_created() {
    OBJECTS.fetch_add(1, Ordering::SeqCst);
}

pub fn object_released() {
    OBJECTS.fetch_sub(1, Ordering::SeqCst);
}

#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> i32 {
    (OBJECTS.load(Ordering::SeqCst) != 0) as i32
}

pub mod merge;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_merge;
