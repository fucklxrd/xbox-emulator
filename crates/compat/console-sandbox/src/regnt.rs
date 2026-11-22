use crate::registry::{store, Value, STATUS_INVALID_HANDLE, STATUS_INVALID_PARAMETER, STATUS_OBJECT_NAME_NOT_FOUND, STATUS_SUCCESS};
use crate::wstr::from_unicode_string;

pub const STATUS_BUFFER_OVERFLOW: i32 = 0x8000_0005u32 as i32;
pub const STATUS_NO_MORE_ENTRIES: i32 = 0x8000_001Au32 as i32;
pub const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023u32 as i32;
pub const STATUS_INVALID_INFO_CLASS: i32 = 0xC000_0003u32 as i32;
pub const STATUS_CANNOT_DELETE: i32 = 0xC000_0121u32 as i32;
pub const STATUS_KEY_DELETED: i32 = 0xC000_017Cu32 as i32;

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect()
}

fn le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

pub fn value_info(v: &Value, class: u32) -> Option<(Vec<u8>, usize)> {
    let name = utf16(&v.name);
    match class {
        0 => {
            let mut b = Vec::new();
            b.extend_from_slice(&le(0));
            b.extend_from_slice(&le(v.kind));
            b.extend_from_slice(&le(name.len() as u32));
            b.extend_from_slice(&name);
            Some((b, 12))
        }
        1 => {
            let off = (20 + name.len() + 7) & !7;
            let mut b = Vec::new();
            b.extend_from_slice(&le(0));
            b.extend_from_slice(&le(v.kind));
            b.extend_from_slice(&le(off as u32));
            b.extend_from_slice(&le(v.data.len() as u32));
            b.extend_from_slice(&le(name.len() as u32));
            b.extend_from_slice(&name);
            b.resize(off, 0);
            b.extend_from_slice(&v.data);
            Some((b, 20))
        }
        2 => {
            let mut b = Vec::new();
            b.extend_from_slice(&le(0));
            b.extend_from_slice(&le(v.kind));
            b.extend_from_slice(&le(v.data.len() as u32));
            b.extend_from_slice(&v.data);
            Some((b, 12))
        }
        _ => None,
    }
}

pub fn key_info(name: &str, written: u64, subkeys: usize, values: usize, max_name: usize, max_vname: usize, max_vdata: usize, class: u32) -> Option<(Vec<u8>, usize)> {
    let n = utf16(name);
    let mut b = Vec::new();
    match class {
        0 => {
            b.extend_from_slice(&written.to_le_bytes());
            b.extend_from_slice(&le(0));
            b.extend_from_slice(&le(n.len() as u32));
            b.extend_from_slice(&n);
            Some((b, 16))
        }
        1 => {
            b.extend_from_slice(&written.to_le_bytes());
            b.extend_from_slice(&le(0));
            b.extend_from_slice(&le(u32::MAX));
            b.extend_from_slice(&le(0));
            b.extend_from_slice(&le(n.len() as u32));
            b.extend_from_slice(&n);
            Some((b, 24))
        }
        2 => {
            b.extend_from_slice(&written.to_le_bytes());
            b.extend_from_slice(&le(0));
            b.extend_from_slice(&le(u32::MAX));
            b.extend_from_slice(&le(0));
            b.extend_from_slice(&le(subkeys as u32));
            b.extend_from_slice(&le((max_name * 2) as u32));
            b.extend_from_slice(&le(0));
            b.extend_from_slice(&le(values as u32));
            b.extend_from_slice(&le((max_vname * 2) as u32));
            b.extend_from_slice(&le(max_vdata as u32));
            Some((b, 44))
        }
        _ => None,
    }
}

unsafe fn emit(bytes: &[u8], fixed: usize, out: *mut u8, len: u32, result: *mut u32) -> i32 {
    if !result.is_null() {
        *result = bytes.len() as u32;
    }
    let cap = len as usize;
    if cap < fixed {
        return STATUS_BUFFER_TOO_SMALL;
    }
    if out.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let n = bytes.len().min(cap);
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), out, n);
    if n < bytes.len() {
        STATUS_BUFFER_OVERFLOW
    } else {
        STATUS_SUCCESS
    }
}

fn key_of(h: usize) -> Option<String> {
    let s = store();
    let p = s.join(h, "")?;
    let l = p.to_lowercase();
    if s.handles.contains_key(&h) && !s.keys.contains_key(&l) {
        return Some(String::new());
    }
    Some(l)
}

pub unsafe extern "system" fn nt_query_value_key(h: usize, name: *const u8, class: u32, out: *mut u8, len: u32, result: *mut u32) -> i32 {
    let Some(k) = key_of(h) else { return STATUS_INVALID_HANDLE };
    if k.is_empty() {
        return STATUS_KEY_DELETED;
    }
    let n = from_unicode_string(name);
    let v = {
        let s = store();
        s.keys.get(&k).and_then(|key| key.values.iter().find(|v| v.name.eq_ignore_ascii_case(&n)).cloned())
    };
    let Some(v) = v else { return STATUS_OBJECT_NAME_NOT_FOUND };
    let Some((b, fixed)) = value_info(&v, class) else { return STATUS_INVALID_INFO_CLASS };
    emit(&b, fixed, out, len, result)
}

pub unsafe extern "system" fn nt_enumerate_value_key(h: usize, index: u32, class: u32, out: *mut u8, len: u32, result: *mut u32) -> i32 {
    let Some(k) = key_of(h) else { return STATUS_INVALID_HANDLE };
    if k.is_empty() {
        return STATUS_KEY_DELETED;
    }
    let v = store().keys.get(&k).and_then(|key| key.values.get(index as usize).cloned());
    let Some(v) = v else { return STATUS_NO_MORE_ENTRIES };
    let Some((b, fixed)) = value_info(&v, class) else { return STATUS_INVALID_INFO_CLASS };
    emit(&b, fixed, out, len, result)
}

fn summary(k: &str) -> Option<(String, u64, usize, usize, usize, usize, usize)> {
    let s = store();
    let key = s.keys.get(k)?;
    let name = key.path.rsplit('\\').next().unwrap_or("").to_string();
    Some((
        name,
        key.written,
        key.subkeys.len(),
        key.values.len(),
        key.subkeys.values().map(|n| n.encode_utf16().count()).max().unwrap_or(0),
        key.values.iter().map(|v| v.name.encode_utf16().count()).max().unwrap_or(0),
        key.values.iter().map(|v| v.data.len()).max().unwrap_or(0),
    ))
}

pub unsafe extern "system" fn nt_enumerate_key(h: usize, index: u32, class: u32, out: *mut u8, len: u32, result: *mut u32) -> i32 {
    let Some(k) = key_of(h) else { return STATUS_INVALID_HANDLE };
    if k.is_empty() {
        return STATUS_KEY_DELETED;
    }
    let child = {
        let s = store();
        let Some(key) = s.keys.get(&k) else { return STATUS_INVALID_HANDLE };
        key.subkeys.keys().nth(index as usize).map(|c| format!("{k}\\{c}"))
    };
    let Some(c) = child else { return STATUS_NO_MORE_ENTRIES };
    let Some((name, w, sk, vs, mn, mvn, mvd)) = summary(&c) else { return STATUS_NO_MORE_ENTRIES };
    let Some((b, fixed)) = key_info(&name, w, sk, vs, mn, mvn, mvd, class) else { return STATUS_INVALID_INFO_CLASS };
    emit(&b, fixed, out, len, result)
}

pub unsafe extern "system" fn nt_query_key(h: usize, class: u32, out: *mut u8, len: u32, result: *mut u32) -> i32 {
    let Some(k) = key_of(h) else { return STATUS_INVALID_HANDLE };
    if k.is_empty() {
        return STATUS_KEY_DELETED;
    }
    let Some((name, w, sk, vs, mn, mvn, mvd)) = summary(&k) else { return STATUS_INVALID_HANDLE };
    let Some((b, fixed)) = key_info(&name, w, sk, vs, mn, mvn, mvd, class) else { return STATUS_INVALID_INFO_CLASS };
    emit(&b, fixed, out, len, result)
}

pub unsafe extern "system" fn nt_delete_key(h: usize) -> i32 {
    let mut s = store();
    let Some(k) = s.handles.get(&h).cloned() else { return STATUS_INVALID_HANDLE };
    let Some(key) = s.keys.get(&k) else { return STATUS_KEY_DELETED };
    if !key.subkeys.is_empty() {
        return STATUS_CANNOT_DELETE;
    }
    s.keys.remove(&k);
    if let Some((parent, leaf)) = k.rsplit_once('\\') {
        if let Some(p) = s.keys.get_mut(parent) {
            p.subkeys.remove(leaf);
            p.written = crate::registry::now();
        }
    }
    s.save();
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_delete_value_key(h: usize, name: *const u8) -> i32 {
    let mut s = store();
    let Some(k) = s.handles.get(&h).cloned() else { return STATUS_INVALID_HANDLE };
    let n = from_unicode_string(name);
    let Some(key) = s.keys.get_mut(&k) else { return STATUS_KEY_DELETED };
    let before = key.values.len();
    key.values.retain(|v| !v.name.eq_ignore_ascii_case(&n));
    if key.values.len() == before {
        return STATUS_OBJECT_NAME_NOT_FOUND;
    }
    key.written = crate::registry::now();
    s.save();
    STATUS_SUCCESS
}
