use crate::registry::{store, ERROR_FILE_NOT_FOUND, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use crate::wstr::{from_ptr, put_wide};

pub const ERROR_ACCESS_DENIED: u32 = 5;

pub unsafe extern "system" fn reg_query_value_ex_w(h: usize, name: *const u16, _res: *mut u32, kind: *mut u32, data: *mut u8, size: *mut u32) -> u32 {
    if !data.is_null() && size.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let v = {
        let s = store();
        let Some(p) = s.join(h, "") else { return ERROR_INVALID_HANDLE };
        let Some(k) = s.keys.get(&p.to_lowercase()) else { return ERROR_INVALID_HANDLE };
        let n = from_ptr(name);
        match k.values.iter().find(|v| v.name.eq_ignore_ascii_case(&n)) {
            Some(v) => v.clone(),
            None => return ERROR_FILE_NOT_FOUND,
        }
    };
    if !kind.is_null() {
        *kind = v.kind;
    }
    if size.is_null() {
        return ERROR_SUCCESS;
    }
    let need = v.data.len() as u32;
    if data.is_null() {
        *size = need;
        return ERROR_SUCCESS;
    }
    if *size < need {
        *size = need;
        return ERROR_MORE_DATA;
    }
    core::ptr::copy_nonoverlapping(v.data.as_ptr(), data, v.data.len());
    *size = need;
    ERROR_SUCCESS
}

fn subkey_at(h: usize, index: u32) -> Result<(String, u64), u32> {
    let s = store();
    let Some(p) = s.join(h, "") else { return Err(ERROR_INVALID_HANDLE) };
    let l = p.to_lowercase();
    let Some(k) = s.keys.get(&l) else { return Err(ERROR_INVALID_HANDLE) };
    let Some((low, shown)) = k.subkeys.iter().nth(index as usize) else { return Err(ERROR_NO_MORE_ITEMS) };
    let written = s.keys.get(&format!("{l}\\{low}")).map(|c| c.written).unwrap_or(0);
    Ok((shown.clone(), written))
}

pub unsafe extern "system" fn reg_enum_key_ex_w(h: usize, index: u32, name: *mut u16, name_len: *mut u32, _res: *mut u32, class: *mut u16, class_len: *mut u32, written: *mut u64) -> u32 {
    if name.is_null() || name_len.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let (n, w) = match subkey_at(h, index) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let wn: Vec<u16> = n.encode_utf16().collect();
    if (*name_len as usize) < wn.len() + 1 {
        return ERROR_MORE_DATA;
    }
    put_wide(name, &wn);
    *name_len = wn.len() as u32;
    if !class.is_null() && !class_len.is_null() && *class_len > 0 {
        *class = 0;
    }
    if !class_len.is_null() {
        *class_len = 0;
    }
    if !written.is_null() {
        *written = w;
    }
    ERROR_SUCCESS
}

pub unsafe extern "system" fn reg_enum_key_w(h: usize, index: u32, name: *mut u16, cch: u32) -> u32 {
    if name.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let (n, _) = match subkey_at(h, index) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let wn: Vec<u16> = n.encode_utf16().collect();
    if (cch as usize) < wn.len() + 1 {
        return ERROR_MORE_DATA;
    }
    put_wide(name, &wn);
    ERROR_SUCCESS
}

pub unsafe extern "system" fn reg_delete_value_w(h: usize, name: *const u16) -> u32 {
    let mut s = store();
    let Some(p) = s.join(h, "") else { return ERROR_INVALID_HANDLE };
    let n = from_ptr(name);
    let Some(k) = s.keys.get_mut(&p.to_lowercase()) else { return ERROR_INVALID_HANDLE };
    let before = k.values.len();
    k.values.retain(|v| !v.name.eq_ignore_ascii_case(&n));
    if k.values.len() == before {
        return ERROR_FILE_NOT_FOUND;
    }
    s.save();
    ERROR_SUCCESS
}

pub unsafe extern "system" fn reg_delete_key_w(h: usize, sub: *const u16) -> u32 {
    if sub.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let mut s = store();
    let Some(p) = s.join(h, &from_ptr(sub)) else { return ERROR_INVALID_HANDLE };
    let l = p.to_lowercase();
    let Some(k) = s.keys.get(&l) else { return ERROR_FILE_NOT_FOUND };
    if !k.subkeys.is_empty() {
        return ERROR_ACCESS_DENIED;
    }
    s.keys.remove(&l);
    if let Some((parent, leaf)) = l.rsplit_once('\\') {
        if let Some(pk) = s.keys.get_mut(parent) {
            pk.subkeys.remove(leaf);
            pk.written = crate::registry::now();
        }
    }
    s.save();
    ERROR_SUCCESS
}
