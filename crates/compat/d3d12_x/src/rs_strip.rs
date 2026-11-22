use core::ffi::c_void;

pub const XBOX_ROOT_FLAGS: u32 = 0x0100_0000 | 0x0200_0000 | 0x8000_0000;
pub const DESC_FLAGS: usize = 32;
pub const VERSIONED_DESC_FLAGS: usize = 8 + DESC_FLAGS;
pub const RTS0_FLAGS_OFFSET: usize = 20;

pub type SerializeFn = unsafe extern "system" fn(*const c_void, u32, *mut *mut c_void, *mut *mut c_void) -> i32;
pub type SerializeVersionedFn = unsafe extern "system" fn(*const c_void, *mut *mut c_void, *mut *mut c_void) -> i32;
pub type DeserializerFn = unsafe extern "system" fn(*const c_void, usize, *const c_void, *mut *mut c_void) -> i32;

fn rd(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

pub fn rts0_flags_at(blob: &[u8]) -> Option<usize> {
    if blob.get(0..4) != Some(b"DXBC") {
        return (blob.len() >= RTS0_FLAGS_OFFSET + 4).then_some(RTS0_FLAGS_OFFSET);
    }
    let n = rd(blob, 28)? as usize;
    for i in 0..n.min(64) {
        let off = rd(blob, 32 + i * 4)? as usize;
        if blob.get(off..off + 4) == Some(b"RTS0") {
            let size = rd(blob, off + 4)? as usize;
            let body = off + 8;
            if size >= RTS0_FLAGS_OFFSET + 4 && body + size <= blob.len() {
                return Some(body + RTS0_FLAGS_OFFSET);
            }
            return None;
        }
    }
    None
}

pub fn set_blob_flags(blob: &mut [u8], f: impl FnOnce(u32) -> u32) -> bool {
    let Some(at) = rts0_flags_at(blob) else { return false };
    let Some(old) = rd(blob, at) else { return false };
    blob[at..at + 4].copy_from_slice(&f(old).to_le_bytes());
    if blob.get(0..4) == Some(b"DXBC") {
        blob[4..20].fill(0);
    }
    true
}

unsafe fn vslot(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

unsafe fn restore_blob(blob: *mut c_void, xbox: u32) {
    if blob.is_null() || xbox == 0 {
        return;
    }
    let ptr: unsafe extern "system" fn(*mut c_void) -> *mut u8 = unsafe { core::mem::transmute(vslot(blob, 3)) };
    let size: unsafe extern "system" fn(*mut c_void) -> usize = unsafe { core::mem::transmute(vslot(blob, 4)) };
    let (p, n) = unsafe { (ptr(blob), size(blob)) };
    if p.is_null() || n == 0 {
        return;
    }
    set_blob_flags(unsafe { core::slice::from_raw_parts_mut(p, n) }, |f| f | xbox);
}

unsafe fn strip_desc(desc: *const c_void, at: usize) -> Option<(Vec<u8>, u32)> {
    if desc.is_null() {
        return None;
    }
    let flags = unsafe { core::ptr::read_unaligned((desc as *const u8).add(at) as *const u32) };
    let xbox = flags & XBOX_ROOT_FLAGS;
    if xbox == 0 {
        return None;
    }
    let mut d = unsafe { core::slice::from_raw_parts(desc as *const u8, at + 4) }.to_vec();
    d[at..at + 4].copy_from_slice(&(flags & !XBOX_ROOT_FLAGS).to_le_bytes());
    Some((d, xbox))
}

pub unsafe fn serialize(f: SerializeFn, desc: *const c_void, version: u32, ppblob: *mut *mut c_void, pperror: *mut *mut c_void) -> i32 {
    let Some((d, xbox)) = (unsafe { strip_desc(desc, DESC_FLAGS) }) else {
        return unsafe { f(desc, version, ppblob, pperror) };
    };
    let hr = unsafe { f(d.as_ptr() as *const c_void, version, ppblob, pperror) };
    if hr >= 0 && !ppblob.is_null() {
        unsafe { restore_blob(*ppblob, xbox) };
    }
    hr
}

pub unsafe fn serialize_versioned(f: SerializeVersionedFn, desc: *const c_void, ppblob: *mut *mut c_void, pperror: *mut *mut c_void) -> i32 {
    let Some((d, xbox)) = (unsafe { strip_desc(desc, VERSIONED_DESC_FLAGS) }) else {
        return unsafe { f(desc, ppblob, pperror) };
    };
    let hr = unsafe { f(d.as_ptr() as *const c_void, ppblob, pperror) };
    if hr >= 0 && !ppblob.is_null() {
        unsafe { restore_blob(*ppblob, xbox) };
    }
    hr
}

pub unsafe fn deserializer(f: DeserializerFn, versioned: bool, data: *const c_void, size: usize, riid: *const c_void, pp: *mut *mut c_void) -> i32 {
    if data.is_null() || size == 0 {
        return unsafe { f(data, size, riid, pp) };
    }
    let src = unsafe { core::slice::from_raw_parts(data as *const u8, size) };
    let xbox = rts0_flags_at(src).and_then(|at| rd(src, at)).map_or(0, |fl| fl & XBOX_ROOT_FLAGS);
    if xbox == 0 {
        return unsafe { f(data, size, riid, pp) };
    }
    let mut pc = src.to_vec();
    set_blob_flags(&mut pc, |fl| fl & !XBOX_ROOT_FLAGS);
    let hr = unsafe { f(pc.as_ptr() as *const c_void, pc.len(), riid, pp) };
    if hr >= 0 && !pp.is_null() && !unsafe { *pp }.is_null() {
        let o = unsafe { *pp };
        let (slot, at) = if versioned { (4, VERSIONED_DESC_FLAGS) } else { (3, DESC_FLAGS) };
        let get: unsafe extern "system" fn(*mut c_void) -> *const u8 = unsafe { core::mem::transmute(vslot(o, slot)) };
        let desc = unsafe { get(o) };
        if !desc.is_null() {
            let p = unsafe { desc.add(at) } as *mut u32;
            unsafe { core::ptr::write_unaligned(p, core::ptr::read_unaligned(p) | xbox) };
        }
        if versioned {
            unsafe { restore_versions(o, xbox) };
        }
    }
    hr
}

pub const ROOT_SIGNATURE_VERSIONS: [u32; 3] = [1, 2, 3];

unsafe fn restore_versions(o: *mut c_void, xbox: u32) {
    let at_version: unsafe extern "system" fn(*mut c_void, u32, *mut *const u8) -> i32 = unsafe { core::mem::transmute(vslot(o, 3)) };
    for v in ROOT_SIGNATURE_VERSIONS {
        let mut desc: *const u8 = core::ptr::null();
        if unsafe { at_version(o, v, &mut desc) } >= 0 && !desc.is_null() {
            let p = unsafe { desc.add(VERSIONED_DESC_FLAGS) } as *mut u32;
            unsafe { core::ptr::write_unaligned(p, core::ptr::read_unaligned(p) | xbox) };
        }
    }
}
