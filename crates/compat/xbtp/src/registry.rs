use crate::base::{log_error_args, log_info_args};
use crate::msvc::{cpp_delete, cpp_new, SharedPtr};
use crate::sys::*;
use core::ffi::c_void;
use core::ptr::null_mut;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

pub type HKEY = *mut c_void;

pub const HKEY_CURRENT_USER: HKEY = 0xFFFF_FFFF_8000_0001u64 as usize as HKEY;
pub const HKEY_LOCAL_MACHINE: HKEY = 0xFFFF_FFFF_8000_0002u64 as usize as HKEY;
pub const EXTENSIONS_KEY: &str = r"Software\Microsoft\Durango\XTF\Extensions";
pub const E_OBJECT_ALREADY_EXISTS: HRESULT = 0x8007_1392u32 as i32;
pub const E_PATH_NOT_FOUND: HRESULT = 0x8007_0003u32 as i32;
pub const SERVER_ENTRY_ORDINAL: usize = 4;
pub const IID_OPERATION_HANDLER_CONTAINER: GUID = GUID::new(0xe2b6_c063, 0x7d82, 0x414a, [0xbe, 0x86, 0x67, 0xed, 0x1b, 0x44, 0x77, 0x97]);

pub const LOAD_FAILURE_NONE: u32 = 0;
pub const LOAD_FAILURE_LOAD_LIBRARY: u32 = 1;
pub const LOAD_FAILURE_SERVER_ENTRY_POINT: u32 = 2;
pub const LOAD_FAILURE_NOT_REGISTERED: u32 = 3;

const KEY_ACCESS_TRIES: [u32; 3] = [0xf003f, 0x2001f, 0x20019];
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_NO_MORE_ITEMS: i32 = 0x103;
const REG_SZ: u32 = 1;

#[link(name = "kernel32")]
extern "system" {
    fn FreeLibrary(h: HANDLE) -> i32;
    fn ExpandEnvironmentStringsW(src: *const u16, dst: *mut u16, n: u32) -> u32;
    fn GetFullPathNameW(name: *const u16, n: u32, buf: *mut u16, file: *mut *mut u16) -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(key: HKEY, sub: *const u16, opts: u32, sam: u32, out: *mut HKEY) -> i32;
    fn RegEnumKeyExW(key: HKEY, index: u32, name: *mut u16, len: *mut u32, reserved: *mut u32, class: *mut u16, class_len: *mut u32, time: *mut u64) -> i32;
    fn RegQueryValueExW(key: HKEY, name: *const u16, reserved: *mut u32, ty: *mut u32, data: *mut u8, len: *mut u32) -> i32;
    fn RegCloseKey(key: HKEY) -> i32;
}

pub type ServerEntryFn = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

fn guid_args(g: &GUID) -> [usize; 11] {
    let d = g.data4;
    [g.data1 as usize, g.data2 as usize, g.data3 as usize, d[0] as usize, d[1] as usize, d[2] as usize, d[3] as usize, d[4] as usize, d[5] as usize, d[6] as usize, d[7] as usize]
}

fn wz(v: &[u16]) -> Vec<u16> {
    let mut w: Vec<u16> = v.iter().copied().take_while(|c| *c != 0).collect();
    w.push(0);
    w
}

unsafe fn vslot<F: Copy>(obj: *mut c_void, slot: usize) -> F {
    let vt = *(obj as *const *const usize);
    core::mem::transmute_copy::<usize, F>(&*vt.add(slot))
}

unsafe fn com_add_ref(obj: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = vslot(obj, 1);
    f(obj);
}

unsafe fn com_release(obj: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = vslot(obj, 2);
    f(obj);
}

pub struct ExtensionModule {
    pub hmodule: HANDLE,
    pub file_name: Vec<u16>,
    pub pinned: bool,
}

unsafe impl Send for ExtensionModule {}
unsafe impl Sync for ExtensionModule {}

pub unsafe fn module_file_name(h: HANDLE) -> Result<Vec<u16>, HRESULT> {
    let mut buf = vec![0u16; 0x104];
    let n = GetModuleFileNameW(h, buf.as_mut_ptr(), 0x104);
    if n == 0 {
        let e = GetLastError();
        log_error_args(b"Error %d GetModuleFileNameW(0x%08X, %u)\0", &[e as usize, h as usize, 0x104]);
        return Err(hr_from_win32(e));
    }
    buf.truncate(n as usize);
    Ok(buf)
}

unsafe fn full_path(p: &[u16]) -> Result<Vec<u16>, HRESULT> {
    let src = wz(p);
    let n = GetFullPathNameW(src.as_ptr(), 0, null_mut(), null_mut());
    if n == 0 {
        let e = GetLastError();
        return Err(if e == 0 { E_FAIL } else { hr_from_win32(e) });
    }
    let mut buf = vec![0u16; n as usize + 1];
    let m = GetFullPathNameW(src.as_ptr(), buf.len() as u32, buf.as_mut_ptr(), null_mut());
    if m == 0 {
        let e = GetLastError();
        return Err(if e == 0 { E_FAIL } else { hr_from_win32(e) });
    }
    buf.truncate(m as usize);
    Ok(buf)
}

pub unsafe fn split_parent(p: &[u16]) -> Result<(Vec<u16>, Vec<u16>), HRESULT> {
    let mut f = full_path(p)?;
    while f.len() > 1 && f.last() == Some(&(b'\\' as u16)) {
        f.pop();
    }
    match f.iter().rposition(|c| *c == b'\\' as u16) {
        Some(i) => Ok((f[..i].to_vec(), f[i + 1..].to_vec())),
        None => Err(E_PATH_NOT_FOUND),
    }
}

pub unsafe fn xtf_load_library(file: &[u16]) -> (HRESULT, HANDLE) {
    let exe = match module_file_name(null_mut()) {
        Ok(v) => v,
        Err(hr) => return (hr, null_mut()),
    };
    let (dir, _) = match split_parent(&exe) {
        Ok(v) => v,
        Err(hr) => return (hr, null_mut()),
    };
    let (_, name) = match split_parent(file) {
        Ok(v) => v,
        Err(hr) => return (hr, null_mut()),
    };
    let mut local = dir;
    local.push(b'\\' as u16);
    local.extend_from_slice(&name);
    let local = wz(&local);
    let h = LoadLibraryExW(local.as_ptr(), null_mut(), 0);
    if !h.is_null() {
        log_info_args(b"Loaded DLL \"%ls\"\0", &[local.as_ptr() as usize]);
        return (S_OK, h);
    }
    let orig = wz(file);
    let h = LoadLibraryExW(orig.as_ptr(), null_mut(), 0);
    if !h.is_null() {
        log_info_args(b"Loaded DLL \"%ls\"\0", &[orig.as_ptr() as usize]);
        return (S_OK, h);
    }
    let hr = hr_last_error();
    log_error_args(b"Error 0x%X LoadLibrary(%ls)\0", &[hr as u32 as usize, orig.as_ptr() as usize]);
    (hr, null_mut())
}

impl ExtensionModule {
    pub fn empty() -> Self {
        ExtensionModule { hmodule: null_mut(), file_name: Vec::new(), pinned: false }
    }

    pub unsafe fn load(&mut self, file: &[u16]) -> HRESULT {
        let (hr, h) = xtf_load_library(file);
        if hr < 0 {
            return hr;
        }
        self.unload();
        self.hmodule = h;
        self.pinned = false;
        self.file_name.clear();
        match module_file_name(h) {
            Ok(n) => {
                self.file_name = n;
                S_OK
            }
            Err(hr) => {
                self.unload();
                FreeLibrary(h);
                hr
            }
        }
    }

    pub unsafe fn unload(&mut self) -> HRESULT {
        if self.hmodule.is_null() {
            return S_OK;
        }
        let h = self.hmodule;
        self.hmodule = null_mut();
        let name = wz(&self.file_name);
        self.file_name.clear();
        if self.pinned {
            return S_OK;
        }
        log_info_args(b"Unloading DLL \"%ls\"\0", &[name.as_ptr() as usize]);
        if FreeLibrary(h) == 0 {
            let e = GetLastError();
            log_error_args(b"Error %d FreeLibrary(0x%08X)\0", &[e as usize, h as usize]);
            return hr_from_win32(e);
        }
        S_OK
    }

    pub unsafe fn server_entry_point(&self) -> Result<ServerEntryFn, HRESULT> {
        let p = GetProcAddress(self.hmodule, SERVER_ENTRY_ORDINAL as *const u8);
        if p.is_null() {
            let e = GetLastError();
            log_error_args(b"Error %d GetProcAddress(0x%08X, %hs)\0", &[e as usize, self.hmodule as usize, b"#4\0".as_ptr() as usize]);
            return Err(hr_from_win32(e));
        }
        Ok(core::mem::transmute::<*mut c_void, ServerEntryFn>(p))
    }
}

impl Drop for ExtensionModule {
    fn drop(&mut self) {
        unsafe {
            self.unload();
        }
    }
}

pub struct MessageRouter {
    handlers: Mutex<BTreeMap<u32, usize>>,
}

unsafe impl Send for MessageRouter {}
unsafe impl Sync for MessageRouter {}

impl Default for MessageRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageRouter {
    pub fn new() -> Self {
        MessageRouter { handlers: Mutex::new(BTreeMap::new()) }
    }

    pub unsafe fn initialize(&self, container: *mut c_void) -> HRESULT {
        if container.is_null() {
            return E_INVALIDARG;
        }
        let mut count: u32 = 0;
        let get_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT = vslot(container, 3);
        let hr = get_count(container, &mut count);
        if hr < 0 || count == 0 {
            return hr;
        }
        let mut list: Vec<*mut c_void> = Vec::new();
        if list.try_reserve_exact(count as usize).is_err() {
            return E_OUTOFMEMORY;
        }
        list.resize(count as usize, null_mut());
        let get_handlers: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT = vslot(container, 4);
        let hr = get_handlers(container, count * 8, list.as_mut_ptr());
        if hr < 0 {
            return hr;
        }
        let mut hr = hr;
        for h in list {
            if h.is_null() {
                continue;
            }
            com_add_ref(h);
            let mut op: u32 = 0;
            let get_op: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT = vslot(h, 3);
            hr = get_op(h, &mut op);
            if hr < 0 {
                return hr;
            }
            let mut m = self.handlers.lock().unwrap();
            if m.contains_key(&op) {
                return E_OBJECT_ALREADY_EXISTS;
            }
            m.insert(op, h as usize);
            hr = S_OK;
        }
        hr
    }

    pub fn handler(&self, op: u32) -> *mut c_void {
        self.handlers.lock().unwrap().get(&op).copied().unwrap_or(0) as *mut c_void
    }

    pub fn handler_count(&self) -> usize {
        self.handlers.lock().unwrap().len()
    }
}

impl Drop for MessageRouter {
    fn drop(&mut self) {
        let m = core::mem::take(&mut *self.handlers.lock().unwrap());
        for (_, h) in m {
            unsafe { com_release(h as *mut c_void) };
        }
    }
}

unsafe fn drop_router(p: *mut c_void) {
    drop(Box::from_raw(p as *mut MessageRouter));
}

pub unsafe fn new_router() -> Option<SharedPtr<MessageRouter>> {
    let p = Box::into_raw(Box::new(MessageRouter::new()));
    let sp = SharedPtr::new_with(p, drop_router);
    if sp.is_none() {
        drop(Box::from_raw(p));
    }
    sp
}

#[repr(C)]
pub struct ExtensionDefinition {
    pub id: GUID,
    pub flags: u32,
    pub router: SharedPtr<MessageRouter>,
    pub module: Option<Arc<ExtensionModule>>,
    pub reserved: [u64; 3],
}

unsafe fn drop_definition(p: *mut c_void) {
    core::ptr::drop_in_place(p as *mut ExtensionDefinition);
    cpp_delete(p);
}

pub unsafe fn new_definition(id: GUID, router: SharedPtr<MessageRouter>, module: Option<Arc<ExtensionModule>>) -> Option<SharedPtr<ExtensionDefinition>> {
    let p = cpp_new(core::mem::size_of::<ExtensionDefinition>()) as *mut ExtensionDefinition;
    if p.is_null() {
        return None;
    }
    p.write(ExtensionDefinition { id, flags: 0, router, module, reserved: [0; 3] });
    let sp = SharedPtr::new_with(p, drop_definition);
    if sp.is_none() {
        drop_definition(p as *mut c_void);
    }
    sp
}

pub struct DllRegistryEntry {
    pub id: GUID,
    pub file_name: Vec<u16>,
}

unsafe fn expand_environment(s: &[u16]) -> Result<Vec<u16>, HRESULT> {
    let src = wz(s);
    let fail = || {
        let e = GetLastError();
        if e == 0 {
            E_FAIL
        } else {
            hr_from_win32(e)
        }
    };
    let n = ExpandEnvironmentStringsW(src.as_ptr(), null_mut(), 0);
    if n == 0 {
        return Err(fail());
    }
    let mut buf = vec![0u16; n as usize];
    let m = ExpandEnvironmentStringsW(src.as_ptr(), buf.as_mut_ptr(), n);
    if m == 0 {
        return Err(fail());
    }
    Ok(wz(&buf)[..].split_last().map(|(_, r)| r.to_vec()).unwrap_or_default())
}

struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { RegCloseKey(self.0) };
        }
    }
}

unsafe fn open_key(root: HKEY, sub: &[u16]) -> Result<RegKey, HRESULT> {
    let name = wz(sub);
    let mut hr = E_UNEXPECTED;
    let mut out: HKEY = null_mut();
    for sam in KEY_ACCESS_TRIES {
        if hr >= 0 {
            break;
        }
        hr = hr_from_win32(RegOpenKeyExW(root, name.as_ptr(), 0, sam, &mut out) as u32);
    }
    if hr < 0 {
        return Err(hr);
    }
    Ok(RegKey(out))
}

unsafe fn query_default_sz(key: &RegKey) -> Result<Option<Vec<u16>>, HRESULT> {
    let mut ty: u32 = 0;
    let mut size: u32 = 0;
    let r = RegQueryValueExW(key.0, core::ptr::null(), null_mut(), &mut ty, null_mut(), &mut size);
    if r == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if r != 0 {
        return Err(hr_from_win32(r as u32));
    }
    if ty != REG_SZ {
        return Err(E_UNEXPECTED);
    }
    let chars = (size / 2) as usize;
    if chars <= 1 {
        return Ok(None);
    }
    let mut buf = vec![0u16; chars];
    let mut size2 = size;
    let mut ty2: u32 = 0;
    let r = RegQueryValueExW(key.0, core::ptr::null(), null_mut(), &mut ty2, buf.as_mut_ptr() as *mut u8, &mut size2);
    if r != 0 {
        return Err(hr_from_win32(r as u32));
    }
    if ty2 != REG_SZ {
        return Err(E_UNEXPECTED);
    }
    let w = wz(&buf);
    Ok(Some(w[..w.len() - 1].to_vec()))
}

pub fn parse_guid_text(s: &[u16]) -> Option<GUID> {
    let widths = [8usize, 4, 4, 2, 2, 2, 2, 2, 2, 2, 2];
    let seps: [&[u16]; 11] = [&[b'{' as u16], &[b'-' as u16], &[b'-' as u16], &[b'-' as u16], &[], &[b'-' as u16], &[], &[], &[], &[], &[]];
    let mut i = 0usize;
    let mut vals = [0u32; 11];
    let is_ws = |c: u16| c == 0x20 || (0x09..=0x0d).contains(&c);
    for f in 0..11 {
        for &c in seps[f] {
            if f == 0 {
                while i < s.len() && is_ws(s[i]) {
                    i += 1;
                }
            }
            if i >= s.len() || s[i] != c {
                return None;
            }
            i += 1;
        }
        let mut left = widths[f];
        while i < s.len() && is_ws(s[i]) && left > 0 {
            i += 1;
            left -= 1;
        }
        if left > 0 && i + 1 < s.len() && s[i] == b'0' as u16 && (s[i + 1] == b'x' as u16 || s[i + 1] == b'X' as u16) && left >= 2 {
            i += 2;
            left -= 2;
        }
        let mut v: u32 = 0;
        let mut n = 0usize;
        while left > 0 && i < s.len() {
            let c = s[i];
            let d = match c {
                0x30..=0x39 => c - 0x30,
                0x41..=0x46 => c - 0x41 + 10,
                0x61..=0x66 => c - 0x61 + 10,
                _ => break,
            };
            v = (v << 4) | d as u32;
            i += 1;
            n += 1;
            left -= 1;
        }
        if n == 0 {
            return None;
        }
        vals[f] = v;
    }
    let mut d4 = [0u8; 8];
    for k in 0..8 {
        d4[k] = vals[3 + k] as u8;
    }
    Some(GUID::new(vals[0], vals[1] as u16, vals[2] as u16, d4))
}

pub struct DllRegistry {
    pub entries: Mutex<Vec<Arc<DllRegistryEntry>>>,
    pub root: HKEY,
    pub key: Vec<u16>,
}

unsafe impl Send for DllRegistry {}
unsafe impl Sync for DllRegistry {}

impl DllRegistry {
    pub fn new(root: HKEY, key: &str) -> Self {
        DllRegistry { entries: Mutex::new(Vec::new()), root, key: key.encode_utf16().collect() }
    }

    pub unsafe fn add(&self, id: &GUID, file: Option<&[u16]>) -> HRESULT {
        let src: Vec<u16> = file.map(|f| f.iter().copied().take_while(|c| *c != 0).collect()).unwrap_or_default();
        let expanded = match expand_environment(&src) {
            Ok(v) => v,
            Err(hr) => return hr,
        };
        self.entries.lock().unwrap().push(Arc::new(DllRegistryEntry { id: *id, file_name: expanded }));
        S_OK
    }

    pub fn find(&self, id: &GUID) -> Option<Arc<DllRegistryEntry>> {
        self.entries.lock().unwrap().iter().find(|e| e.id == *id).cloned()
    }

    pub unsafe fn load_entry(&self, id: &GUID, name: &[u16]) -> HRESULT {
        let parent = match open_key(self.root, &self.key) {
            Ok(k) => k,
            Err(hr) => return hr,
        };
        let sub = match open_key(parent.0, name) {
            Ok(k) => k,
            Err(hr) => return hr,
        };
        let file = match query_default_sz(&sub) {
            Ok(f) => f,
            Err(hr) => return hr,
        };
        self.add(id, file.as_deref())
    }

    pub unsafe fn load_entry_by_name(&self, name: &[u16]) -> HRESULT {
        match parse_guid_text(name) {
            Some(g) => self.load_entry(&g, name),
            None => E_INVALIDARG,
        }
    }

    pub unsafe fn load_hive(&self) -> HRESULT {
        let parent = match open_key(self.root, &self.key) {
            Ok(k) => k,
            Err(hr) => return hr,
        };
        let mut index = 0u32;
        loop {
            let mut name = [0u16; 0x40];
            let mut len: u32 = 0x40;
            let r = RegEnumKeyExW(parent.0, index, name.as_mut_ptr(), &mut len, null_mut(), null_mut(), null_mut(), null_mut());
            if r == ERROR_NO_MORE_ITEMS {
                return S_OK;
            }
            let hr = hr_from_win32(r as u32);
            if hr < 0 {
                return if hr == E_ABORT { S_OK } else { hr };
            }
            self.load_entry_by_name(&name[..len as usize]);
            index += 1;
        }
    }
}

pub struct RegistryImpl {
    pub lock: Mutex<()>,
    pub extensions: Mutex<HashMap<GUID, SharedPtr<ExtensionDefinition>>>,
    pub dlls: DllRegistry,
}

impl RegistryImpl {
    pub fn new() -> Self {
        RegistryImpl { lock: Mutex::new(()), extensions: Mutex::new(HashMap::new()), dlls: DllRegistry::new(HKEY_LOCAL_MACHINE, EXTENSIONS_KEY) }
    }

    unsafe fn find_definition(&self, id: &GUID) -> Option<SharedPtr<ExtensionDefinition>> {
        self.extensions.lock().unwrap().get(id).map(|s| s.clone_ref())
    }

    unsafe fn add_definition(&self, id: &GUID, def: SharedPtr<ExtensionDefinition>) -> HRESULT {
        let mut m = self.extensions.lock().unwrap();
        if m.contains_key(id) {
            return E_OBJECT_ALREADY_EXISTS;
        }
        m.insert(*id, def);
        S_OK
    }

    pub unsafe fn register_extension(&self, id: &GUID, path: *const u16) -> HRESULT {
        let a = guid_args(id);
        if self.dlls.find(id).is_some() {
            log_error_args(b"Extension {%08X-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X} already registered\0", &a);
            return E_INVALIDARG;
        }
        let p = core::slice::from_raw_parts(path, wcslen(path));
        let hr = self.dlls.add(id, Some(p));
        if hr < 0 {
            return hr;
        }
        let mut args = a.to_vec();
        args.push(path as usize);
        log_info_args(b"{%08X-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X} %ws dynamically registered\0", &args);
        hr
    }

    pub unsafe fn finish_load(&self, id: &GUID, module: Arc<ExtensionModule>, entry: ServerEntryFn, out: &mut SharedPtr<ExtensionDefinition>, reason: &mut u32) -> HRESULT {
        let mut container: *mut c_void = null_mut();
        let hr = entry(id, &IID_OPERATION_HANDLER_CONTAINER, &mut container);
        let result = (|| {
            if hr < 0 {
                let mut args = guid_args(id).to_vec();
                args.push(hr as u32 as usize);
                log_error_args(b"Extension {%08X-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X} ServerEntryPoint returned error 0x%X\0", &args);
                *reason = LOAD_FAILURE_SERVER_ENTRY_POINT;
                return hr;
            }
            let router = match new_router() {
                Some(r) => r,
                None => return E_OUTOFMEMORY,
            };
            let hr = (*router.ptr).initialize(container);
            if hr < 0 {
                return hr;
            }
            match new_definition(*id, router, Some(module.clone())) {
                Some(d) => {
                    d.assign_into(out);
                    S_OK
                }
                None => E_OUTOFMEMORY,
            }
        })();
        if !container.is_null() {
            com_release(container);
        }
        result
    }

    pub unsafe fn load_extension(&self, id: &GUID, entry: &DllRegistryEntry, out: &mut SharedPtr<ExtensionDefinition>, reason: &mut u32) -> HRESULT {
        let mut module = ExtensionModule::empty();
        let hr = module.load(&entry.file_name);
        if hr < 0 {
            let name = wz(&entry.file_name);
            let mut args = vec![name.as_ptr() as usize];
            args.extend_from_slice(&guid_args(id));
            log_error_args(b"Could not load extension file %ws for {%08X-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X}\0", &args);
            *reason = LOAD_FAILURE_LOAD_LIBRARY;
            return hr;
        }
        let ep = match module.server_entry_point() {
            Ok(f) => f,
            Err(hr) => {
                let mut args = guid_args(id).to_vec();
                args.push(hr as u32 as usize);
                log_error_args(b"Extension {%08X-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X} ServerEntryPoint returned error 0x%X\0", &args);
                *reason = LOAD_FAILURE_SERVER_ENTRY_POINT;
                return hr;
            }
        };
        self.finish_load(id, Arc::new(module), ep, out, reason)
    }

    pub unsafe fn lookup(&self, id: &GUID, out: *mut SharedPtr<ExtensionDefinition>, reason: *mut u32) -> bool {
        let _g = self.lock.lock().unwrap();
        *reason = LOAD_FAILURE_NONE;
        let mut local = SharedPtr::<ExtensionDefinition>::default();
        let mut found = false;
        if let Some(d) = self.find_definition(id) {
            local = d;
            found = true;
        } else {
            match self.dlls.find(id) {
                None => *reason = LOAD_FAILURE_NOT_REGISTERED,
                Some(entry) => {
                    let mut r = LOAD_FAILURE_NONE;
                    let hr = self.load_extension(id, &entry, &mut local, &mut r);
                    *reason = r;
                    if hr >= 0 {
                        self.add_definition(id, local.clone_ref());
                        found = true;
                    } else {
                        local.release();
                    }
                }
            }
        }
        local.assign_into(out);
        found
    }

    pub unsafe fn is_valid(&self, id: &GUID) -> bool {
        self.find_definition(id).is_some() || self.dlls.find(id).is_some()
    }
}

impl Default for RegistryImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct XbtpExtensionRegistry {
    pub m_impl: *mut RegistryImpl,
}

pub unsafe fn registry_impl<'a>(this: *mut XbtpExtensionRegistry) -> &'a RegistryImpl {
    &*(*this).m_impl
}

pub unsafe fn discover_registry_extensions(this: *mut XbtpExtensionRegistry) -> HRESULT {
    let imp = registry_impl(this);
    let _g = imp.lock.lock().unwrap();
    imp.dlls.load_hive();
    S_OK
}

pub unsafe fn set_hive_root(this: *mut XbtpExtensionRegistry, root: HKEY, key: &str) {
    let imp = &mut *(*this).m_impl;
    imp.dlls.root = root;
    imp.dlls.key = key.encode_utf16().collect();
}

pub unsafe fn lookup(this: *mut XbtpExtensionRegistry, id: &GUID) -> (bool, SharedPtr<ExtensionDefinition>, u32) {
    let mut sp = SharedPtr::default();
    let mut reason = 0u32;
    let ok = registry_impl(this).lookup(id, &mut sp, &mut reason);
    (ok, sp, reason)
}

#[export_name = "??1XbtpExtensionRegistry@@QEAA@XZ"]
pub unsafe extern "C" fn XbtpExtensionRegistry_dtor(this: *mut XbtpExtensionRegistry) {
    let p = core::mem::replace(&mut (*this).m_impl, null_mut());
    if !p.is_null() {
        drop(Box::from_raw(p));
    }
}

#[export_name = "?Initialize@XbtpExtensionRegistry@@QEAAJXZ"]
pub unsafe extern "C" fn XbtpExtensionRegistry_Initialize(this: *mut XbtpExtensionRegistry) -> HRESULT {
    let layout = std::alloc::Layout::new::<RegistryImpl>();
    let p = std::alloc::alloc(layout) as *mut RegistryImpl;
    if p.is_null() {
        (*this).m_impl = null_mut();
        return E_OUTOFMEMORY;
    }
    p.write(RegistryImpl::new());
    (*this).m_impl = p;
    if (*p).extensions.get_mut().unwrap().try_reserve(13).is_err() {
        return E_OUTOFMEMORY;
    }
    S_OK
}

#[export_name = "?AddExtension@XbtpExtensionRegistry@@QEAAJV?$shared_ptr@UExtensionDefinition@@@std@@@Z"]
pub unsafe extern "C" fn XbtpExtensionRegistry_AddExtension(this: *mut XbtpExtensionRegistry, ext: *mut SharedPtr<ExtensionDefinition>) -> HRESULT {
    let mut arg = SharedPtr::take(ext);
    if arg.ptr.is_null() {
        arg.release();
        return E_INVALIDARG;
    }
    let imp = registry_impl(this);
    let hr = {
        let _g = imp.lock.lock().unwrap();
        let id = (*arg.ptr).id;
        imp.add_definition(&id, arg.clone_ref())
    };
    arg.release();
    hr
}

#[export_name = "?RegisterExtension@XbtpExtensionRegistry@@QEAAJAEBU_GUID@@PEBG@Z"]
pub unsafe extern "C" fn XbtpExtensionRegistry_RegisterExtension(this: *mut XbtpExtensionRegistry, id: *const GUID, path: *const u16) -> HRESULT {
    if path.is_null() {
        return E_INVALIDARG;
    }
    registry_impl(this).register_extension(&*id, path)
}

#[export_name = "?LookupExtension@XbtpExtensionRegistry@@QEAA_NAEBU_GUID@@PEAV?$shared_ptr@UExtensionDefinition@@@std@@PEAW4ExtensionLoadFailureReason@1@@Z"]
pub unsafe extern "C" fn XbtpExtensionRegistry_LookupExtension(this: *mut XbtpExtensionRegistry, id: *const GUID, out: *mut SharedPtr<ExtensionDefinition>, reason: *mut u32) -> bool {
    if out.is_null() || reason.is_null() {
        return false;
    }
    registry_impl(this).lookup(&*id, out, reason)
}

#[export_name = "?IsExtensionValid@XbtpExtensionRegistry@@QEAA_NAEBU_GUID@@@Z"]
pub unsafe extern "C" fn XbtpExtensionRegistry_IsExtensionValid(this: *mut XbtpExtensionRegistry, id: *const GUID) -> bool {
    registry_impl(this).is_valid(&*id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU32, Ordering};

    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(key: HKEY, sub: *const u16, reserved: u32, class: *mut u16, opts: u32, sam: u32, sa: *mut c_void, out: *mut HKEY, disp: *mut u32) -> i32;
        fn RegSetValueExW(key: HKEY, name: *const u16, reserved: u32, ty: u32, data: *const u8, len: u32) -> i32;
        fn RegDeleteTreeW(key: HKEY, sub: *const u16) -> i32;
    }

    const G1: GUID = GUID::new(0x1111_2222, 0x3333, 0x4444, [1, 2, 3, 4, 5, 6, 7, 8]);
    const G2: GUID = GUID::new(0xa1b2_c3d4, 0xe5f6, 0x0718, [0x29, 0x3a, 0x4b, 0x5c, 0x6d, 0x7e, 0x8f, 0x90]);
    const G3: GUID = GUID::new(0x0bad_f00d, 1, 2, [3, 4, 5, 6, 7, 8, 9, 10]);

    fn new_registry() -> XbtpExtensionRegistry {
        let mut r = XbtpExtensionRegistry { m_impl: null_mut() };
        assert_eq!(unsafe { XbtpExtensionRegistry_Initialize(&mut r) }, S_OK);
        assert!(!r.m_impl.is_null());
        r
    }

    fn static_definition(id: GUID) -> SharedPtr<ExtensionDefinition> {
        unsafe { new_definition(id, new_router().unwrap(), None).unwrap() }
    }

    #[test]
    fn definition_layout_matches_console() {
        assert_eq!(core::mem::size_of::<ExtensionDefinition>(), 0x48);
        assert_eq!(core::mem::offset_of!(ExtensionDefinition, id), 0);
        assert_eq!(core::mem::offset_of!(ExtensionDefinition, flags), 0x10);
        assert_eq!(core::mem::offset_of!(ExtensionDefinition, router), 0x18);
        assert_eq!(core::mem::offset_of!(ExtensionDefinition, module), 0x28);
        assert_eq!(core::mem::size_of::<XbtpExtensionRegistry>(), 8);
    }

    #[test]
    fn initialize_and_destructor() {
        let mut r = new_registry();
        unsafe { XbtpExtensionRegistry_dtor(&mut r) };
        assert!(r.m_impl.is_null());
        unsafe { XbtpExtensionRegistry_dtor(&mut r) };
    }

    #[test]
    fn add_extension_then_lookup_and_duplicate() {
        let mut r = new_registry();
        unsafe {
            let mut empty = SharedPtr::<ExtensionDefinition>::default();
            assert_eq!(XbtpExtensionRegistry_AddExtension(&mut r, &mut empty), E_INVALIDARG);
            let def = static_definition(G1);
            let mut a = def.clone_ref();
            assert_eq!(XbtpExtensionRegistry_AddExtension(&mut r, &mut a), S_OK);
            assert!(a.ptr.is_null());
            assert_eq!(def.use_count(), 2);
            let mut b = def.clone_ref();
            assert_eq!(XbtpExtensionRegistry_AddExtension(&mut r, &mut b), E_OBJECT_ALREADY_EXISTS);
            assert_eq!(def.use_count(), 2);
            let mut out = SharedPtr::<ExtensionDefinition>::default();
            let mut reason = 99u32;
            assert!(XbtpExtensionRegistry_LookupExtension(&mut r, &G1, &mut out, &mut reason));
            assert_eq!(reason, LOAD_FAILURE_NONE);
            assert_eq!(out.ptr, def.ptr);
            assert!(!XbtpExtensionRegistry_LookupExtension(&mut r, &G1, null_mut(), &mut reason));
            assert!(!XbtpExtensionRegistry_LookupExtension(&mut r, &G1, &mut out, null_mut()));
            drop(out);
            XbtpExtensionRegistry_dtor(&mut r);
            assert_eq!(def.use_count(), 1);
        }
    }

    #[test]
    fn lookup_unknown_reports_not_registered() {
        let mut r = new_registry();
        unsafe {
            let mut out = static_definition(G3);
            let mut reason = 0u32;
            assert!(!XbtpExtensionRegistry_LookupExtension(&mut r, &G2, &mut out, &mut reason));
            assert_eq!(reason, LOAD_FAILURE_NOT_REGISTERED);
            assert!(out.ptr.is_null());
            assert!(!XbtpExtensionRegistry_IsExtensionValid(&mut r, &G2));
            XbtpExtensionRegistry_dtor(&mut r);
        }
    }

    #[test]
    fn register_extension_rules_and_load_failure_reason() {
        let mut r = new_registry();
        unsafe {
            assert_eq!(XbtpExtensionRegistry_RegisterExtension(&mut r, &G2, core::ptr::null()), E_INVALIDARG);
            let path = wide(r"%SystemRoot%\xbtp_missing_extension_7f3a.dll");
            assert_eq!(XbtpExtensionRegistry_RegisterExtension(&mut r, &G2, path.as_ptr()), S_OK);
            assert!(XbtpExtensionRegistry_IsExtensionValid(&mut r, &G2));
            assert_eq!(XbtpExtensionRegistry_RegisterExtension(&mut r, &G2, path.as_ptr()), E_INVALIDARG);
            let e = registry_impl(&mut r).dlls.find(&G2).unwrap();
            let s = String::from_utf16_lossy(&e.file_name);
            assert!(!s.contains('%'), "{}", s);
            assert!(s.ends_with(r"\xbtp_missing_extension_7f3a.dll"));
            let mut out = SharedPtr::<ExtensionDefinition>::default();
            let mut reason = 0u32;
            assert!(!XbtpExtensionRegistry_LookupExtension(&mut r, &G2, &mut out, &mut reason));
            assert_eq!(reason, LOAD_FAILURE_LOAD_LIBRARY);
            assert!(out.ptr.is_null());
            XbtpExtensionRegistry_dtor(&mut r);
        }
    }

    #[test]
    fn guid_text_parsing_matches_scanf_format() {
        let w: Vec<u16> = "{A1B2C3D4-E5F6-0718-293A-4B5C6D7E8F90}".encode_utf16().collect();
        assert_eq!(parse_guid_text(&w), Some(G2));
        let l: Vec<u16> = "{a1b2c3d4-e5f6-0718-293a-4b5c6d7e8f90}".encode_utf16().collect();
        assert_eq!(parse_guid_text(&l), Some(G2));
        let bad: Vec<u16> = "Extension".encode_utf16().collect();
        assert_eq!(parse_guid_text(&bad), None);
        let short: Vec<u16> = "{A1B2C3D4-E5F6-0718-293A}".encode_utf16().collect();
        assert_eq!(parse_guid_text(&short), None);
    }

    #[test]
    fn hive_discovery_reads_subkeys() {
        unsafe {
            let base = format!(r"Software\TomBoxXbtpRegistryTest_{}", GetCurrentProcessId());
            let ext = format!(r"{}\Extensions", base);
            let mk = |sub: &str, val: Option<&str>| {
                let mut k: HKEY = null_mut();
                let n = wide(&format!(r"{}\{}", ext, sub));
                assert_eq!(RegCreateKeyExW(HKEY_CURRENT_USER, n.as_ptr(), 0, null_mut(), 0, 0xf003f, null_mut(), &mut k, null_mut()), 0);
                if let Some(v) = val {
                    let d = wide(v);
                    assert_eq!(RegSetValueExW(k, core::ptr::null(), 0, REG_SZ, d.as_ptr() as *const u8, (d.len() * 2) as u32), 0);
                }
                RegCloseKey(k);
            };
            mk("{A1B2C3D4-E5F6-0718-293A-4B5C6D7E8F90}", Some(r"C:\nowhere\ext_a.dll"));
            mk("{11112222-3333-4444-0102-030405060708}", None);
            mk("not-a-guid", Some(r"C:\nowhere\ext_c.dll"));
            let mut r = new_registry();
            set_hive_root(&mut r, HKEY_CURRENT_USER, &ext);
            assert_eq!(discover_registry_extensions(&mut r), S_OK);
            assert!(XbtpExtensionRegistry_IsExtensionValid(&mut r, &G2));
            assert!(XbtpExtensionRegistry_IsExtensionValid(&mut r, &G1));
            assert!(!XbtpExtensionRegistry_IsExtensionValid(&mut r, &G3));
            let imp = registry_impl(&mut r);
            assert_eq!(String::from_utf16_lossy(&imp.dlls.find(&G2).unwrap().file_name), r"C:\nowhere\ext_a.dll");
            assert!(imp.dlls.find(&G1).unwrap().file_name.is_empty());
            assert_eq!(imp.dlls.entries.lock().unwrap().len(), 2);
            let p = wide(r"C:\other.dll");
            assert_eq!(XbtpExtensionRegistry_RegisterExtension(&mut r, &G2, p.as_ptr()), E_INVALIDARG);
            XbtpExtensionRegistry_dtor(&mut r);
            let mut r2 = new_registry();
            set_hive_root(&mut r2, HKEY_CURRENT_USER, &format!(r"{}\Missing", base));
            assert_eq!(registry_impl(&mut r2).dlls.load_hive(), hr_from_win32(2));
            assert_eq!(discover_registry_extensions(&mut r2), S_OK);
            XbtpExtensionRegistry_dtor(&mut r2);
            let b = wide(&base);
            RegDeleteTreeW(HKEY_CURRENT_USER, b.as_ptr());
            let bk = wide(&base);
            let mut k: HKEY = null_mut();
            let _ = RegOpenKeyExW(HKEY_CURRENT_USER, bk.as_ptr(), 0, 0x20019, &mut k);
            if !k.is_null() {
                RegCloseKey(k);
                let parent = wide("Software");
                let mut pk: HKEY = null_mut();
                RegOpenKeyExW(HKEY_CURRENT_USER, parent.as_ptr(), 0, 0xf003f, &mut pk);
                let leaf = wide(&format!("TomBoxXbtpRegistryTest_{}", GetCurrentProcessId()));
                RegDeleteTreeW(pk, leaf.as_ptr());
                RegCloseKey(pk);
            }
        }
    }

    #[repr(C)]
    struct FakeObj {
        vtbl: *const usize,
        refs: AtomicU32,
        op: u32,
    }

    static HANDLER_RELEASES: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn h_qi(_o: *mut c_void, _i: *const GUID, _p: *mut *mut c_void) -> HRESULT {
        E_NOINTERFACE
    }
    unsafe extern "system" fn h_add(o: *mut c_void) -> u32 {
        (*(o as *mut FakeObj)).refs.fetch_add(1, Ordering::SeqCst) + 1
    }
    unsafe extern "system" fn h_rel(o: *mut c_void) -> u32 {
        HANDLER_RELEASES.fetch_add(1, Ordering::SeqCst);
        (*(o as *mut FakeObj)).refs.fetch_sub(1, Ordering::SeqCst) - 1
    }
    unsafe extern "system" fn h_op(o: *mut c_void, out: *mut u32) -> HRESULT {
        *out = (*(o as *mut FakeObj)).op;
        S_OK
    }

    struct Vt<const N: usize>([*const c_void; N]);

    unsafe impl<const N: usize> Sync for Vt<N> {}

    static HANDLER_VTBL: Vt<4> = Vt([h_qi as *const c_void, h_add as *const c_void, h_rel as *const c_void, h_op as *const c_void]);

    #[repr(C)]
    struct FakeContainer {
        vtbl: *const usize,
        refs: AtomicU32,
        handlers: Vec<*mut c_void>,
    }

    unsafe extern "system" fn c_add(o: *mut c_void) -> u32 {
        (*(o as *mut FakeContainer)).refs.fetch_add(1, Ordering::SeqCst) + 1
    }
    unsafe extern "system" fn c_rel(o: *mut c_void) -> u32 {
        (*(o as *mut FakeContainer)).refs.fetch_sub(1, Ordering::SeqCst) - 1
    }
    unsafe extern "system" fn c_count(o: *mut c_void, out: *mut u32) -> HRESULT {
        *out = (*(o as *mut FakeContainer)).handlers.len() as u32;
        S_OK
    }
    unsafe extern "system" fn c_get(o: *mut c_void, size: u32, arr: *mut *mut c_void) -> HRESULT {
        let c = &*(o as *mut FakeContainer);
        if size as usize != c.handlers.len() * 8 {
            return E_INVALIDARG;
        }
        for (i, h) in c.handlers.iter().enumerate() {
            *arr.add(i) = *h;
        }
        S_OK
    }

    static CONTAINER_VTBL: Vt<5> = Vt([h_qi as *const c_void, c_add as *const c_void, c_rel as *const c_void, c_count as *const c_void, c_get as *const c_void]);

    static mut CONTAINER: *mut FakeContainer = null_mut();

    unsafe extern "system" fn fake_entry(id: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        assert_eq!(*id, G3);
        assert_eq!(*iid, IID_OPERATION_HANDLER_CONTAINER);
        c_add(CONTAINER as *mut c_void);
        *out = CONTAINER as *mut c_void;
        S_OK
    }

    unsafe extern "system" fn failing_entry(_id: *const GUID, _iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        *out = null_mut();
        E_NOINTERFACE
    }

    #[test]
    fn server_entry_point_builds_router_from_container() {
        unsafe {
            let mut h1 = FakeObj { vtbl: HANDLER_VTBL.0.as_ptr() as *const usize, refs: AtomicU32::new(1), op: 7 };
            let mut h2 = FakeObj { vtbl: HANDLER_VTBL.0.as_ptr() as *const usize, refs: AtomicU32::new(1), op: 9 };
            let mut c = FakeContainer { vtbl: CONTAINER_VTBL.0.as_ptr() as *const usize, refs: AtomicU32::new(1), handlers: vec![&mut h1 as *mut FakeObj as *mut c_void, null_mut(), &mut h2 as *mut FakeObj as *mut c_void] };
            CONTAINER = &mut c;
            let imp = RegistryImpl::new();
            let mut out = SharedPtr::<ExtensionDefinition>::default();
            let mut reason = 0u32;
            assert_eq!(imp.finish_load(&G3, Arc::new(ExtensionModule::empty()), fake_entry, &mut out, &mut reason), S_OK);
            assert_eq!(reason, 0);
            assert_eq!(c.refs.load(Ordering::SeqCst), 1);
            assert_eq!(h1.refs.load(Ordering::SeqCst), 2);
            let def = &*out.ptr;
            assert_eq!(def.id, G3);
            assert_eq!(def.flags, 0);
            assert!(def.module.is_some());
            let router = &*def.router.ptr;
            assert_eq!(router.handler_count(), 2);
            assert_eq!(router.handler(7), &mut h1 as *mut FakeObj as *mut c_void);
            assert_eq!(router.handler(9), &mut h2 as *mut FakeObj as *mut c_void);
            assert!(router.handler(8).is_null());
            drop(out);
            assert_eq!(h1.refs.load(Ordering::SeqCst), 1);
            assert_eq!(h2.refs.load(Ordering::SeqCst), 1);
            let mut out2 = SharedPtr::<ExtensionDefinition>::default();
            assert_eq!(imp.finish_load(&G3, Arc::new(ExtensionModule::empty()), failing_entry, &mut out2, &mut reason), E_NOINTERFACE);
            assert_eq!(reason, LOAD_FAILURE_SERVER_ENTRY_POINT);
            assert!(out2.ptr.is_null());
            (*(c.handlers[0] as *mut FakeObj)).op = 9;
            let mut out3 = SharedPtr::<ExtensionDefinition>::default();
            reason = 0;
            assert_eq!(imp.finish_load(&G3, Arc::new(ExtensionModule::empty()), fake_entry, &mut out3, &mut reason), E_OBJECT_ALREADY_EXISTS);
            assert_eq!(reason, 0);
            assert!(out3.ptr.is_null());
            CONTAINER = null_mut();
        }
    }

    #[test]
    fn router_rejects_null_container() {
        let r = MessageRouter::new();
        assert_eq!(unsafe { r.initialize(null_mut()) }, E_INVALIDARG);
    }

    #[test]
    fn module_load_unload_and_missing_file() {
        unsafe {
            let mut m = ExtensionModule::empty();
            let sys = wide(r"C:\Windows\System32\version.dll");
            let ws: Vec<u16> = sys[..sys.len() - 1].to_vec();
            assert_eq!(m.load(&ws), S_OK);
            assert!(!m.hmodule.is_null());
            assert!(String::from_utf16_lossy(&m.file_name).to_ascii_lowercase().ends_with("version.dll"));
            assert_eq!(m.unload(), S_OK);
            assert!(m.hmodule.is_null());
            let bad: Vec<u16> = r"C:\nowhere\none.dll".encode_utf16().collect();
            assert!(m.load(&bad) < 0);
        }
    }
}
