use core::ffi::c_void;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::wstr::{from_ptr, from_unicode_string, put_wide};

pub const HKEY_CLASSES_ROOT: usize = 0xFFFF_FFFF_8000_0000;
pub const HKEY_CURRENT_USER: usize = 0xFFFF_FFFF_8000_0001;
pub const HKEY_LOCAL_MACHINE: usize = 0xFFFF_FFFF_8000_0002;
pub const HKEY_USERS: usize = 0xFFFF_FFFF_8000_0003;

pub const REG_NONE: u32 = 0;
pub const REG_SZ: u32 = 1;
pub const REG_EXPAND_SZ: u32 = 2;
pub const REG_BINARY: u32 = 3;
pub const REG_DWORD: u32 = 4;
pub const REG_MULTI_SZ: u32 = 7;
pub const REG_QWORD: u32 = 11;

pub const STATUS_SUCCESS: i32 = 0;
pub const STATUS_INVALID_HANDLE: i32 = 0xC000_0008u32 as i32;
pub const STATUS_INVALID_PARAMETER: i32 = 0xC000_000Du32 as i32;
pub const STATUS_OBJECT_NAME_NOT_FOUND: i32 = 0xC000_0034u32 as i32;
pub const STATUS_ACCESS_VIOLATION: i32 = 0xC000_0005u32 as i32;

pub const ERROR_KEY_DELETED: u32 = 1018;
pub const STATUS_KEY_DELETED: i32 = 0xC000_017Cu32 as i32;
pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_MORE_DATA: u32 = 234;
pub const ERROR_NO_MORE_ITEMS: u32 = 259;
pub const ERROR_UNSUPPORTED_TYPE: u32 = 1630;

const RRF_NOEXPAND: u32 = 0x1000_0000;
const RRF_ZEROONFAILURE: u32 = 0x2000_0000;
const HANDLE_BASE: usize = 0x7F5A_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value {
    pub name: String,
    pub kind: u32,
    pub data: Vec<u8>,
}

#[derive(Default, Clone, Debug)]
pub struct Key {
    pub path: String,
    pub values: Vec<Value>,
    pub subkeys: BTreeMap<String, String>,
    pub written: u64,
}

#[derive(Default)]
pub struct Store {
    pub keys: HashMap<String, Key>,
    pub(crate) handles: HashMap<usize, String>,
    next: usize,
    loaded: bool,
}

pub(crate) fn store() -> MutexGuard<'static, Store> {
    static S: OnceLock<Mutex<Store>> = OnceLock::new();
    let mut g = S.get_or_init(|| Mutex::new(Store::default())).lock().unwrap_or_else(|e| e.into_inner());
    if !g.loaded {
        g.loaded = true;
        g.load();
    }
    g
}

pub(crate) fn now() -> u64 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (d.as_nanos() / 100) as u64 + 116_444_736_000_000_000
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()).collect()
}

pub fn canonical(path: &str) -> String {
    let parts: Vec<&str> = path.split('\\').filter(|p| !p.is_empty()).collect();
    format!("\\{}", parts.join("\\"))
}

impl Store {
    fn file() -> std::path::PathBuf {
        crate::root().join("registry.txt")
    }

    fn load(&mut self) {
        let Ok(text) = std::fs::read_to_string(Self::file()) else { return };
        let mut cur: Option<String> = None;
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.first().copied() {
                Some("K") if f.len() >= 3 => {
                    let k = self.ensure(f[1]);
                    if let Some(key) = self.keys.get_mut(&k) {
                        key.written = f[2].parse().unwrap_or(0);
                    }
                    cur = Some(k);
                }
                Some("V") if f.len() >= 4 => {
                    if let Some(k) = cur.as_ref().and_then(|c| self.keys.get_mut(c)) {
                        k.values.push(Value { name: f[1].to_string(), kind: f[2].parse().unwrap_or(0), data: unhex(f[3]) });
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn save(&self) {
        let mut keys: Vec<&Key> = self.keys.values().collect();
        keys.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
        let mut out = String::new();
        for k in keys {
            out.push_str(&format!("K\t{}\t{}\n", k.path, k.written));
            for v in &k.values {
                out.push_str(&format!("V\t{}\t{}\t{}\n", v.name, v.kind, hex(&v.data)));
            }
        }
        let f = Self::file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, out);
    }

    pub(crate) fn ensure(&mut self, path: &str) -> String {
        let path = canonical(path);
        let lower = path.to_lowercase();
        if self.keys.contains_key(&lower) {
            return lower;
        }
        let comps: Vec<&str> = path.split('\\').filter(|p| !p.is_empty()).collect();
        let mut acc = String::new();
        let mut parent: Option<String> = None;
        for c in comps {
            acc.push('\\');
            acc.push_str(c);
            let l = acc.to_lowercase();
            let fresh = !self.keys.contains_key(&l);
            if fresh {
                self.keys.insert(l.clone(), Key { path: acc.clone(), written: now(), ..Key::default() });
            }
            if let Some(p) = parent.as_ref().and_then(|p| self.keys.get_mut(p)) {
                p.subkeys.entry(c.to_lowercase()).or_insert_with(|| c.to_string());
                if fresh {
                    p.written = now();
                }
            }
            parent = Some(l);
        }
        lower
    }

    pub(crate) fn handle(&mut self, key: String) -> usize {
        self.next += 1;
        let h = HANDLE_BASE + self.next * 4;
        self.handles.insert(h, key);
        h
    }

    fn base(&self, h: usize) -> Option<String> {
        match h {
            HKEY_LOCAL_MACHINE => Some("\\registry\\machine".into()),
            HKEY_USERS => Some("\\registry\\user".into()),
            HKEY_CURRENT_USER => Some("\\registry\\user\\.default".into()),
            HKEY_CLASSES_ROOT => Some("\\registry\\machine\\software\\classes".into()),
            _ => self.handles.get(&h).map(|k| self.keys.get(k).map(|k| k.path.clone()).unwrap_or_else(|| k.clone())),
        }
    }

    pub(crate) fn deleted(&self, h: usize) -> bool {
        self.handles.get(&h).is_some_and(|k| !self.keys.contains_key(k))
    }

    pub(crate) fn join(&self, h: usize, sub: &str) -> Option<String> {
        let b = self.base(h)?;
        Some(if sub.is_empty() { canonical(&b) } else { canonical(&format!("{b}\\{sub}")) })
    }
}

pub fn is_handle(h: usize) -> bool {
    store().handles.contains_key(&h)
}

pub fn get(path: &str, name: &str) -> Option<Value> {
    let s = store();
    let k = s.keys.get(&canonical(path).to_lowercase())?;
    k.values.iter().find(|v| v.name.eq_ignore_ascii_case(name)).cloned()
}

pub fn get_string(path: &str, name: &str) -> Option<String> {
    let v = get(path, name)?;
    if v.kind != REG_SZ && v.kind != REG_EXPAND_SZ {
        return None;
    }
    let w: Vec<u16> = v.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    Some(String::from_utf16_lossy(&w))
}

pub fn set(path: &str, name: &str, kind: u32, data: &[u8]) {
    let mut s = store();
    let k = s.ensure(path);
    set_in(&mut s, &k, name, kind, data);
    s.save();
}

pub fn set_string(path: &str, name: &str, value: &str) {
    let b: Vec<u8> = value.encode_utf16().chain(Some(0)).flat_map(|c| c.to_le_bytes()).collect();
    set(path, name, REG_SZ, &b);
}

pub fn create_key(path: &str) {
    let mut s = store();
    s.ensure(path);
    s.save();
}

pub fn key_exists(path: &str) -> bool {
    store().keys.contains_key(&canonical(path).to_lowercase())
}

pub fn values(path: &str) -> Vec<Value> {
    store().keys.get(&canonical(path).to_lowercase()).map(|k| k.values.clone()).unwrap_or_default()
}

pub fn reset() {
    let mut s = store();
    s.keys.clear();
    s.handles.clear();
    s.save();
}

pub(crate) fn set_in(s: &mut Store, key: &str, name: &str, kind: u32, data: &[u8]) {
    if let Some(k) = s.keys.get_mut(key) {
        match k.values.iter_mut().find(|v| v.name.eq_ignore_ascii_case(name)) {
            Some(v) => {
                v.kind = kind;
                v.data = data.to_vec();
            }
            None => k.values.push(Value { name: name.to_string(), kind, data: data.to_vec() }),
        }
        k.written = now();
    }
}

pub(crate) unsafe fn object_path(s: &Store, oa: *const u8) -> Result<String, i32> {
    if oa.is_null() {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let root = (oa.add(8) as *const usize).read_unaligned();
    let name = from_unicode_string((oa.add(0x10) as *const *const u8).read_unaligned());
    if root == 0 {
        if !name.to_lowercase().starts_with("\\registry") {
            return Err(STATUS_OBJECT_NAME_NOT_FOUND);
        }
        return Ok(canonical(&name));
    }
    s.join(root, &name).ok_or(STATUS_INVALID_HANDLE)
}

pub unsafe extern "system" fn nt_create_key(out: *mut usize, _access: u32, oa: *const u8, _title: u32, _class: *const u8, _options: u32, disposition: *mut u32) -> i32 {
    if out.is_null() {
        return STATUS_ACCESS_VIOLATION;
    }
    let mut s = store();
    if !oa.is_null() && s.deleted((oa.add(8) as *const usize).read_unaligned()) {
        return STATUS_KEY_DELETED;
    }
    let path = match object_path(&s, oa) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let existed = s.keys.contains_key(&path.to_lowercase());
    let k = s.ensure(&path);
    *out = s.handle(k);
    if !disposition.is_null() {
        *disposition = if existed { 2 } else { 1 };
    }
    s.save();
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_open_key(out: *mut usize, _access: u32, oa: *const u8) -> i32 {
    if out.is_null() {
        return STATUS_ACCESS_VIOLATION;
    }
    let mut s = store();
    let path = match object_path(&s, oa) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let l = path.to_lowercase();
    if !s.keys.contains_key(&l) {
        return STATUS_OBJECT_NAME_NOT_FOUND;
    }
    *out = s.handle(l);
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_set_value_key(h: usize, name: *const u8, _title: u32, kind: u32, data: *const u8, size: u32) -> i32 {
    let mut s = store();
    let Some(k) = s.handles.get(&h).cloned() else { return STATUS_INVALID_HANDLE };
    if s.deleted(h) {
        return STATUS_KEY_DELETED;
    }
    let n = from_unicode_string(name);
    let d = if data.is_null() { Vec::new() } else { std::slice::from_raw_parts(data, size as usize).to_vec() };
    set_in(&mut s, &k, &n, kind, &d);
    s.save();
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_flush_key(h: usize) -> i32 {
    let s = store();
    if !s.handles.contains_key(&h) {
        return STATUS_INVALID_HANDLE;
    }
    s.save();
    STATUS_SUCCESS
}

pub fn close(h: usize) -> bool {
    store().handles.remove(&h).is_some()
}

fn predefined(h: usize) -> bool {
    (HKEY_CLASSES_ROOT..=0xFFFF_FFFF_8000_0007).contains(&h)
}

pub unsafe extern "system" fn reg_open_key_ex_w(h: usize, sub: *const u16, _opt: u32, _sam: u32, out: *mut usize) -> u32 {
    if out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let mut s = store();
    let Some(p) = s.join(h, &from_ptr(sub)) else { return ERROR_INVALID_HANDLE };
    let l = p.to_lowercase();
    if !s.keys.contains_key(&l) {
        return ERROR_FILE_NOT_FOUND;
    }
    *out = s.handle(l);
    ERROR_SUCCESS
}

pub unsafe extern "system" fn reg_create_key_ex_w(h: usize, sub: *const u16, _res: u32, _class: *const u16, _opt: u32, _sam: u32, _sa: *const c_void, out: *mut usize, disposition: *mut u32) -> u32 {
    if out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let mut s = store();
    if s.deleted(h) {
        return ERROR_KEY_DELETED;
    }
    let Some(p) = s.join(h, &from_ptr(sub)) else { return ERROR_INVALID_HANDLE };
    let existed = s.keys.contains_key(&p.to_lowercase());
    let k = s.ensure(&p);
    *out = s.handle(k);
    if !disposition.is_null() {
        *disposition = if existed { 2 } else { 1 };
    }
    s.save();
    ERROR_SUCCESS
}

pub unsafe extern "system" fn reg_close_key(h: usize) -> u32 {
    if predefined(h) || close(h) {
        ERROR_SUCCESS
    } else {
        ERROR_INVALID_HANDLE
    }
}

pub unsafe extern "system" fn reg_set_value_ex_w(h: usize, name: *const u16, _res: u32, kind: u32, data: *const u8, size: u32) -> u32 {
    let mut s = store();
    if s.deleted(h) {
        return ERROR_KEY_DELETED;
    }
    let Some(p) = s.join(h, "") else { return ERROR_INVALID_HANDLE };
    let k = s.ensure(&p);
    let d = if data.is_null() { Vec::new() } else { std::slice::from_raw_parts(data, size as usize).to_vec() };
    set_in(&mut s, &k, &from_ptr(name), kind, &d);
    s.save();
    ERROR_SUCCESS
}

fn is_string(kind: u32) -> bool {
    kind == REG_SZ || kind == REG_EXPAND_SZ || kind == REG_MULTI_SZ
}

fn terminated(kind: u32, mut d: Vec<u8>) -> Vec<u8> {
    if is_string(kind) {
        if d.len() % 2 == 1 {
            d.push(0);
        }
        let tail: &[u8] = if kind == REG_MULTI_SZ { &[0, 0, 0, 0] } else { &[0, 0] };
        if !d.ends_with(tail) {
            if d.ends_with(&[0, 0]) && kind == REG_MULTI_SZ {
                d.extend_from_slice(&[0, 0]);
            } else {
                d.extend_from_slice(tail);
            }
        }
    }
    d
}

pub unsafe extern "system" fn reg_get_value_w(h: usize, sub: *const u16, name: *const u16, flags: u32, kind_out: *mut u32, data: *mut u8, size: *mut u32) -> u32 {
    if !data.is_null() && size.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let cap = if size.is_null() { 0 } else { *size as usize };
    let fail = |e: u32| {
        if flags & RRF_ZEROONFAILURE != 0 && !data.is_null() {
            core::ptr::write_bytes(data, 0, cap);
        }
        e
    };
    let found = {
        let s = store();
        let Some(p) = s.join(h, &from_ptr(sub)) else { return fail(ERROR_INVALID_HANDLE) };
        let Some(k) = s.keys.get(&p.to_lowercase()) else { return fail(ERROR_FILE_NOT_FOUND) };
        let n = from_ptr(name);
        k.values.iter().find(|v| v.name.eq_ignore_ascii_case(&n)).cloned()
    };
    let Some(v) = found else { return fail(ERROR_FILE_NOT_FOUND) };
    let (kind, bytes) = if v.kind == REG_EXPAND_SZ && flags & RRF_NOEXPAND == 0 {
        let w: Vec<u16> = v.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        let e = crate::system::expand(&String::from_utf16_lossy(&w));
        (REG_SZ, e.encode_utf16().chain(Some(0)).flat_map(|c| c.to_le_bytes()).collect())
    } else {
        (v.kind, terminated(v.kind, v.data.clone()))
    };
    let allowed = flags & 0xffff;
    let bit = if kind > 15 { 0 } else { 1u32 << kind };
    if allowed != 0xffff && allowed & bit == 0 {
        return fail(ERROR_UNSUPPORTED_TYPE);
    }
    if !kind_out.is_null() {
        *kind_out = kind;
    }
    if size.is_null() {
        return ERROR_SUCCESS;
    }
    let need = bytes.len() as u32;
    if data.is_null() {
        *size = need;
        return ERROR_SUCCESS;
    }
    if *size < need {
        *size = need;
        return fail(ERROR_MORE_DATA);
    }
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
    *size = need;
    ERROR_SUCCESS
}

pub unsafe extern "system" fn reg_query_info_key_w(
    h: usize,
    class: *mut u16,
    class_len: *mut u32,
    _res: *mut u32,
    subkeys: *mut u32,
    max_subkey: *mut u32,
    max_class: *mut u32,
    values: *mut u32,
    max_value_name: *mut u32,
    max_value: *mut u32,
    sd: *mut u32,
    written: *mut u64,
) -> u32 {
    let s = store();
    let Some(p) = s.join(h, "") else { return ERROR_INVALID_HANDLE };
    let Some(k) = s.keys.get(&p.to_lowercase()) else { return ERROR_INVALID_HANDLE };
    if !class.is_null() && !class_len.is_null() && *class_len > 0 {
        *class = 0;
    }
    let put = |p: *mut u32, v: usize| {
        if !p.is_null() {
            *p = v as u32;
        }
    };
    put(class_len, 0);
    put(subkeys, k.subkeys.len());
    put(max_subkey, k.subkeys.values().map(|n| n.encode_utf16().count()).max().unwrap_or(0));
    put(max_class, 0);
    put(values, k.values.len());
    put(max_value_name, k.values.iter().map(|v| v.name.encode_utf16().count()).max().unwrap_or(0));
    put(max_value, k.values.iter().map(|v| v.data.len()).max().unwrap_or(0));
    put(sd, 0);
    if !written.is_null() {
        *written = k.written;
    }
    ERROR_SUCCESS
}

pub unsafe extern "system" fn reg_enum_value_w(h: usize, index: u32, name: *mut u16, name_len: *mut u32, _res: *mut u32, kind: *mut u32, data: *mut u8, size: *mut u32) -> u32 {
    if name.is_null() || name_len.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let v = {
        let s = store();
        let Some(p) = s.join(h, "") else { return ERROR_INVALID_HANDLE };
        let Some(k) = s.keys.get(&p.to_lowercase()) else { return ERROR_INVALID_HANDLE };
        match k.values.get(index as usize) {
            Some(v) => v.clone(),
            None => return ERROR_NO_MORE_ITEMS,
        }
    };
    let w: Vec<u16> = v.name.encode_utf16().collect();
    if (*name_len as usize) < w.len() + 1 {
        return ERROR_MORE_DATA;
    }
    put_wide(name, &w);
    *name_len = w.len() as u32;
    if !kind.is_null() {
        *kind = v.kind;
    }
    if !size.is_null() {
        let need = v.data.len() as u32;
        if !data.is_null() {
            if *size < need {
                *size = need;
                return ERROR_MORE_DATA;
            }
            core::ptr::copy_nonoverlapping(v.data.as_ptr(), data, v.data.len());
        }
        *size = need;
    }
    ERROR_SUCCESS
}
