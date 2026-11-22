use crate::base::{log_error_args, log_warning_args};
use crate::dispatcher::{DispatcherImpl, ServerSession, ServerSessionVtbl};
use crate::handshake::{deserialize_json_message, Json, SYSTEM_NOT_READY};
use crate::registry::{module_file_name, registry_impl, xtf_load_library};
use crate::sys::*;
use crate::xbtp2_ext::{com_release, HandlerVtbl, Message, MetadataVtbl, XbtpCreateExtensionMetadata, XbtpCreateMessage, IID_XBTP_EXTENSION_METADATA, IID_XBTP_MESSAGE, IID_XBTP_OPERATION_HANDLER};
use crate::xbtp2_json::{ArrVtbl, JsonArray, XbtpCreateJsonArray, IID_IUNKNOWN};
use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::{OnceLock, Weak};

#[link(name = "version")]
extern "system" {
    fn GetFileVersionInfoSizeExW(flags: u32, file: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoExW(flags: u32, file: *const u16, handle: u32, len: u32, data: *mut c_void) -> i32;
    fn VerQueryValueW(block: *const c_void, sub: *const u16, buf: *mut *mut c_void, len: *mut u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleExW(flags: u32, name: *const u16, out: *mut HANDLE) -> i32;
    fn FreeLibrary(h: HANDLE) -> i32;
}

pub const SERVICE_INFO_EXTENSION: GUID = GUID::new(0x206a9d73, 0x95e4, 0x4a5d, [0x93, 0xc2, 0x13, 0x3c, 0x3d, 0x92, 0x46, 0x00]);
pub const VERSION_OPERATION: u32 = 1;
pub const EXTENSION_VERSION_OPERATION: u32 = 2;
pub const TRANSPORT_TEST_OPERATION: u32 = 3;
pub const VM_READY_OPERATION: u32 = 5;
pub const PLATFORM_TYPE_PROPERTY: &str = "Version";
pub const VERSION_PROPERTY: &str = "Platform";
pub const LVER_PLATFORM: &str = "Game OS";
pub const UPTIME_LOW_PROPERTY: &str = "SystemUptimeLDW";
pub const UPTIME_HIGH_PROPERTY: &str = "SystemUptimeHDW";
pub const EXTENSIONS_PROPERTY: &str = "Extensions";
pub const PROCESS_PROPERTY: &str = "Process";
pub const BUFFER_SIZE_PROPERTY: &str = "BufferSize";
pub const LOOP_COUNT_PROPERTY: &str = "LoopCount";
pub const VM_READY_PROPERTY: &str = "VMReady";
pub const DESCRIPTION_PROPERTY: &str = "Description";
pub const PATH_PROPERTY: &str = "Path";
pub const FILE_VERSION_PROPERTY: &str = "FileVersion";
pub const PRODUCT_VERSION_PROPERTY: &str = "ProductVersion";
pub const KERNEL_VERSION_PROPERTY: &str = "KernelVersion";
pub const ID_PROPERTY: &str = "ID";
pub const MAX_LOOP_COUNT: f64 = 134_217_728.0;
pub const MAX_BUFFER_SIZE: f64 = 1_048_576.0;
pub const DEFAULT_RECEIVE_TIMEOUT_MS: u32 = 30_000;
const MAX_PATH: usize = 0x104;

pub struct ModuleVersionInfo {
    pub product_version: Option<Vec<u16>>,
    pub file_description: Option<Vec<u16>>,
    pub file_version: Option<Vec<u16>>,
}

unsafe fn query_string(block: &[u8], name: &str) -> Option<Vec<u16>> {
    let sub = wide(&format!("\\StringFileInfo\\040904B0\\{}", name));
    let mut p: *mut c_void = null_mut();
    let mut n = 0u32;
    if VerQueryValueW(block.as_ptr() as *const c_void, sub.as_ptr(), &mut p, &mut n) == 0 {
        return None;
    }
    if n == 0 || p.is_null() {
        return None;
    }
    let s = core::slice::from_raw_parts(p as *const u16, n as usize);
    if s[n as usize - 1] != 0 {
        return None;
    }
    if s[0] == 0 {
        return None;
    }
    Some(s[..wcslen(p as *const u16)].to_vec())
}

unsafe fn version_block(path: &[u16]) -> Result<Vec<u8>, HRESULT> {
    let p = wz(path);
    let mut h = 0u32;
    let size = GetFileVersionInfoSizeExW(0, p.as_ptr(), &mut h);
    if size == 0 {
        let hr = hr_last_error();
        if hr < 0 {
            return Err(hr);
        }
    }
    let mut buf = vec![0u8; size as usize];
    if GetFileVersionInfoExW(0, p.as_ptr(), 0, size, buf.as_mut_ptr() as *mut c_void) == 0 {
        let hr = hr_last_error();
        if hr < 0 {
            return Err(hr);
        }
    }
    Ok(buf)
}

pub unsafe fn module_version_info(path: &[u16]) -> Option<ModuleVersionInfo> {
    let block = version_block(path).ok()?;
    let root = wide("\\");
    let mut p: *mut c_void = null_mut();
    let mut n = 0u32;
    if VerQueryValueW(block.as_ptr() as *const c_void, root.as_ptr(), &mut p, &mut n) == 0 {
        if hr_last_error() < 0 {
            return None;
        }
    }
    Some(ModuleVersionInfo {
        product_version: query_string(&block, PRODUCT_VERSION_PROPERTY),
        file_description: query_string(&block, "FileDescription"),
        file_version: query_string(&block, FILE_VERSION_PROPERTY),
    })
}

fn wz(v: &[u16]) -> Vec<u16> {
    let mut o: Vec<u16> = v.iter().copied().take_while(|c| *c != 0).collect();
    o.push(0);
    o
}

unsafe fn own_module() -> HANDLE {
    let mut h: HANDLE = null_mut();
    GetModuleHandleExW(6, own_module as *const () as *const u16, &mut h);
    h
}

unsafe fn fixed_file_version(block: &[u8]) -> Option<[u16; 4]> {
    let root = wide("\\");
    let mut p: *mut c_void = null_mut();
    let mut n = 0u32;
    if VerQueryValueW(block.as_ptr() as *const c_void, root.as_ptr(), &mut p, &mut n) == 0 || p.is_null() || (n as usize) < 0x34 {
        return None;
    }
    let f = core::slice::from_raw_parts(p as *const u32, 13);
    if f[0] != 0xFEEF04BD {
        return None;
    }
    Some([(f[2] >> 16) as u16, f[2] as u16, (f[3] >> 16) as u16, f[3] as u16])
}

unsafe fn has_file_version_string(block: &[u8]) -> bool {
    let t = wide("\\VarFileInfo\\Translation");
    let mut p: *mut c_void = null_mut();
    let mut n = 0u32;
    let mut tables: Vec<String> = Vec::new();
    if VerQueryValueW(block.as_ptr() as *const c_void, t.as_ptr(), &mut p, &mut n) != 0 && !p.is_null() {
        let w = core::slice::from_raw_parts(p as *const u16, (n / 2) as usize);
        for pair in w.chunks(2) {
            if pair.len() == 2 {
                tables.push(format!("{:04x}{:04x}", pair[0], pair[1]));
            }
        }
    }
    tables.push("040904b0".to_string());
    for tbl in tables {
        let sub = wide(&format!("\\StringFileInfo\\{}\\FileVersion", tbl));
        let mut v: *mut c_void = null_mut();
        let mut vn = 0u32;
        if VerQueryValueW(block.as_ptr() as *const c_void, sub.as_ptr(), &mut v, &mut vn) != 0 && !v.is_null() {
            return true;
        }
    }
    false
}

pub fn version_string_from_self() -> &'static [u16] {
    static CACHE: OnceLock<Vec<u16>> = OnceLock::new();
    CACHE.get_or_init(|| unsafe {
        let h = own_module();
        let path = match module_file_name(h) {
            Ok(p) => p,
            Err(_) => return vec![0],
        };
        let block = match version_block(&path) {
            Ok(b) => b,
            Err(_) => return vec![0],
        };
        if !has_file_version_string(&block) {
            return vec![0];
        }
        match fixed_file_version(&block) {
            Some(v) => wide(&format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3])),
            None => vec![0],
        }
    })
}

pub struct JsonArr(pub *mut JsonArray);

impl JsonArr {
    pub unsafe fn new() -> Result<JsonArr, HRESULT> {
        let mut p: *mut c_void = null_mut();
        let hr = XbtpCreateJsonArray(&mut p);
        if hr < 0 {
            return Err(hr);
        }
        Ok(JsonArr(p as *mut JsonArray))
    }

    pub unsafe fn add_object(&self, o: &Json) -> HRESULT {
        let vt = &**(self.0 as *const *const ArrVtbl);
        (vt.add_object)(self.0, o.0 as *mut c_void)
    }
}

impl Drop for JsonArr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { com_release(self.0 as *mut c_void) };
        }
    }
}

unsafe fn set_opt_string(o: &Json, name: &str, v: &Option<Vec<u16>>) -> HRESULT {
    match v {
        Some(s) => {
            let z = wz(s);
            o.set_string(name, z.as_ptr())
        }
        None => S_OK,
    }
}

pub unsafe fn fill_module_data(name: &[u16], data: &Json) -> HRESULT {
    let file = wz(name);
    let (hr, h) = xtf_load_library(name);
    if hr < 0 || h.is_null() {
        log_warning_args(b"Unable to load module info for '%ls': hr=0x%X.\0", &[file.as_ptr() as usize, hr as u32 as usize]);
        return S_OK;
    }
    let hr = 'run: {
        let path = match module_file_name(h) {
            Ok(p) => p,
            Err(hr) => {
                log_warning_args(b"Unable to acquire module filename for '%ls': hr=0x%X\0", &[file.as_ptr() as usize, hr as u32 as usize]);
                break 'run S_OK;
            }
        };
        let vi = match module_version_info(&path) {
            Some(v) => v,
            None => break 'run S_OK,
        };
        let hr = set_opt_string(data, DESCRIPTION_PROPERTY, &vi.file_description);
        if hr < 0 {
            break 'run hr;
        }
        let hr = set_opt_string(data, FILE_VERSION_PROPERTY, &vi.file_version);
        if hr < 0 {
            break 'run hr;
        }
        let hr = set_opt_string(data, PRODUCT_VERSION_PROPERTY, &vi.product_version);
        if hr < 0 {
            break 'run hr;
        }
        let p = wz(&path);
        data.set_string(PATH_PROPERTY, p.as_ptr())
    };
    FreeLibrary(h);
    hr
}

pub unsafe fn executable_module_data(data: &Json) -> HRESULT {
    let mut buf = vec![0u16; MAX_PATH];
    let n = GetModuleFileNameW(null_mut(), buf.as_mut_ptr(), MAX_PATH as u32);
    if n as usize >= MAX_PATH {
        return hr_last_error();
    }
    buf.truncate(n as usize);
    let hr = fill_module_data(&buf, data);
    if hr < 0 {
        return hr;
    }
    let nt = wide("ntdll.dll");
    let h = GetModuleHandleW(nt.as_ptr());
    let path = match module_file_name(h) {
        Ok(p) => p,
        Err(hr) => return hr,
    };
    match module_version_info(&path) {
        Some(vi) => match vi.file_version {
            Some(v) => {
                let z = wz(&v);
                data.set_string(KERNEL_VERSION_PROPERTY, z.as_ptr())
            }
            None => data.set_string(KERNEL_VERSION_PROPERTY, core::ptr::null()),
        },
        None => {
            let p = wz(&path);
            log_warning_args(b"Unable to load module info for '%ls'.\0", &[p.as_ptr() as usize]);
            S_OK
        }
    }
}

pub fn guid_text(g: &GUID) -> String {
    format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        g.data1, g.data2, g.data3, g.data4[0], g.data4[1], g.data4[2], g.data4[3], g.data4[4], g.data4[5], g.data4[6], g.data4[7]
    )
}

pub unsafe fn fill_registered_extensions(d: &DispatcherImpl, arr: &JsonArr) -> HRESULT {
    let entries = registry_impl(d.registry.ptr).dlls.entries.lock().unwrap().clone();
    for e in entries.iter() {
        if e.file_name.is_empty() || e.file_name[0] == 0 {
            continue;
        }
        let obj = match Json::new() {
            Ok(o) => o,
            Err(hr) => return hr,
        };
        let hr = fill_module_data(&e.file_name, &obj);
        if hr < 0 {
            return hr;
        }
        let id = wide(&guid_text(&e.id));
        let hr = obj.set_string(ID_PROPERTY, id.as_ptr());
        if hr < 0 {
            return hr;
        }
        let hr = arr.add_object(&obj);
        if hr < 0 {
            return hr;
        }
    }
    S_OK
}

unsafe fn write_reply(body: &Json, reply: *mut c_void) -> HRESULT {
    let text = match body.stringify() {
        Ok(t) => t,
        Err(hr) => return hr,
    };
    crate::xbtp2_ext::reply_set_json_w(reply, text.0)
}

pub enum ServiceOp {
    Version,
    ExtensionVersion(Weak<DispatcherImpl>),
    TransportTest(Weak<DispatcherImpl>),
    VmReady(Weak<DispatcherImpl>),
}

impl ServiceOp {
    pub fn operation_id(&self) -> u32 {
        match self {
            ServiceOp::Version => VERSION_OPERATION,
            ServiceOp::ExtensionVersion(_) => EXTENSION_VERSION_OPERATION,
            ServiceOp::TransportTest(_) => TRANSPORT_TEST_OPERATION,
            ServiceOp::VmReady(_) => VM_READY_OPERATION,
        }
    }

    pub unsafe fn execute(&self, session: *mut c_void, received: *mut c_void, reply: *mut c_void) -> HRESULT {
        match self {
            ServiceOp::Version => version_execute(reply),
            ServiceOp::ExtensionVersion(d) => extension_version_execute(d, reply),
            ServiceOp::TransportTest(_) => transport_test_execute(session, received),
            ServiceOp::VmReady(_) => vm_ready_execute(reply),
        }
    }
}

pub unsafe fn version_execute(reply: *mut c_void) -> HRESULT {
    let uptime = GetTickCount64();
    let body = match Json::new() {
        Ok(j) => j,
        Err(hr) => return hr,
    };
    let platform = wide(LVER_PLATFORM);
    let hr = body.set_string(PLATFORM_TYPE_PROPERTY, platform.as_ptr());
    if hr < 0 {
        return hr;
    }
    let hr = body.set_string(VERSION_PROPERTY, version_string_from_self().as_ptr());
    if hr < 0 {
        return hr;
    }
    let hr = body.set_number(UPTIME_HIGH_PROPERTY, (uptime >> 32) as u32 as f64);
    if hr < 0 {
        return hr;
    }
    let hr = body.set_number(UPTIME_LOW_PROPERTY, uptime as u32 as f64);
    if hr < 0 {
        return hr;
    }
    write_reply(&body, reply)
}

pub unsafe fn extension_version_execute(d: &Weak<DispatcherImpl>, reply: *mut c_void) -> HRESULT {
    let body = match Json::new() {
        Ok(j) => j,
        Err(hr) => return hr,
    };
    let process = match Json::new() {
        Ok(j) => j,
        Err(hr) => return hr,
    };
    let hr = executable_module_data(&process);
    if hr < 0 {
        return hr;
    }
    let hr = body.set_object(PROCESS_PROPERTY, &process);
    if hr < 0 {
        return hr;
    }
    let arr = match JsonArr::new() {
        Ok(a) => a,
        Err(hr) => return hr,
    };
    if let Some(dd) = d.upgrade() {
        let hr = fill_registered_extensions(&dd, &arr);
        if hr < 0 {
            return hr;
        }
    }
    let hr = body.set_array(EXTENSIONS_PROPERTY, arr.0 as *mut c_void);
    if hr < 0 {
        return hr;
    }
    write_reply(&body, reply)
}

pub unsafe fn transport_test_execute(session: *mut c_void, received: *mut c_void) -> HRESULT {
    let m = received as *mut Message;
    let mut len = 0u32;
    let mut body: *mut u8 = null_mut();
    let hr = ((*(*m).vtbl).get_json_body)(m, &mut len, &mut body);
    if hr < 0 {
        return hr;
    }
    if body.is_null() {
        return E_UNEXPECTED;
    }
    let req = match deserialize_json_message(received) {
        Ok(j) => j,
        Err(hr) => return hr,
    };
    let mut loops = 0f64;
    let hr = req.get_number(LOOP_COUNT_PROPERTY, &mut loops);
    if hr < 0 {
        return hr;
    }
    let mut size = 0f64;
    let hr = req.get_number(BUFFER_SIZE_PROPERTY, &mut size);
    if hr < 0 {
        return hr;
    }
    if loops > MAX_LOOP_COUNT || size > MAX_BUFFER_SIZE {
        return E_UNEXPECTED;
    }
    let loop_count = loops as i64 as u32;
    let buffer_size = size as i64 as u32;
    let mut data: Vec<u8> = Vec::new();
    if data.try_reserve_exact(buffer_size as usize).is_err() {
        return E_OUTOFMEMORY;
    }
    data.resize(buffer_size as usize, 0);
    for (i, b) in data.iter_mut().enumerate() {
        *b = i as u8;
    }
    let vt = &**(session as *const *const ServerSessionVtbl);
    let s = session as *mut ServerSession;
    for _ in 0..loop_count {
        let mut msg: *mut c_void = null_mut();
        let hr = (vt.receive_more_message)(s, DEFAULT_RECEIVE_TIMEOUT_MS, &mut msg);
        if hr < 0 {
            return hr;
        }
        let mm = msg as *mut Message;
        let mut n = 0u32;
        let mut p: *mut u8 = null_mut();
        let hr = ((*(*mm).vtbl).get_binary_body)(mm, &mut n, &mut p);
        com_release(msg);
        if hr < 0 {
            return hr;
        }
        if n != buffer_size {
            return E_FAIL;
        }
    }
    for _ in 0..loop_count {
        let mut msg: *mut c_void = null_mut();
        let hr = XbtpCreateMessage(&IID_XBTP_MESSAGE, &mut msg);
        if hr < 0 {
            return hr;
        }
        let mm = msg as *mut Message;
        let hr = ((*(*mm).vtbl).set_binary_body)(mm, buffer_size, data.as_ptr());
        if hr < 0 {
            com_release(msg);
            return hr;
        }
        let hr = (vt.send_more_message)(s, msg);
        com_release(msg);
        if hr < 0 {
            return hr;
        }
    }
    S_OK
}

pub unsafe fn vm_ready_execute(reply: *mut c_void) -> HRESULT {
    let ready = if SYSTEM_NOT_READY.load(Ordering::SeqCst) == 0 { 1.0 } else { 0.0 };
    let body = match Json::new() {
        Ok(j) => j,
        Err(hr) => return hr,
    };
    let hr = body.set_number(VM_READY_PROPERTY, ready);
    if hr < 0 {
        return hr;
    }
    write_reply(&body, reply)
}

#[repr(C)]
pub struct ServiceOperationAdapter {
    pub vtbl: *const HandlerVtbl,
    pub refs: AtomicU32,
    pub op: ServiceOp,
}

pub static SERVICE_ADAPTER_VTBL: HandlerVtbl = HandlerVtbl {
    query_interface: sa_qi,
    add_ref: sa_add_ref,
    release: sa_release,
    get_operation_id: sa_get_operation_id,
    invoke: sa_invoke,
};

pub fn new_service_adapter(op: ServiceOp) -> *mut c_void {
    Box::into_raw(Box::new(ServiceOperationAdapter { vtbl: &SERVICE_ADAPTER_VTBL, refs: AtomicU32::new(1), op })) as *mut c_void
}

unsafe extern "system" fn sa_qi(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    if *iid == IID_IUNKNOWN || *iid == IID_XBTP_OPERATION_HANDLER {
        *out = this;
        sa_add_ref(this);
        return S_OK;
    }
    *out = null_mut();
    E_NOINTERFACE
}

unsafe extern "system" fn sa_add_ref(this: *mut c_void) -> u32 {
    (*(this as *mut ServiceOperationAdapter)).refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn sa_release(this: *mut c_void) -> u32 {
    let a = this as *mut ServiceOperationAdapter;
    let n = (*a).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        drop(Box::from_raw(a));
    }
    n
}

unsafe extern "system" fn sa_get_operation_id(this: *mut c_void, out: *mut u32) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = (*(this as *mut ServiceOperationAdapter)).op.operation_id();
    S_OK
}

unsafe extern "system" fn sa_invoke(this: *mut c_void, session: usize, received: usize, reply: usize) -> HRESULT {
    (*(this as *mut ServiceOperationAdapter)).op.execute(session as *mut c_void, received as *mut c_void, reply as *mut c_void)
}

pub unsafe fn create_service_info_extension_metadata(d: Option<&std::sync::Arc<DispatcherImpl>>, out: *mut *mut c_void) -> HRESULT {
    let d = match d {
        Some(d) => d,
        None => return E_INVALIDARG,
    };
    let mut md: *mut c_void = null_mut();
    let hr = XbtpCreateExtensionMetadata(&IID_XBTP_EXTENSION_METADATA, &mut md);
    if hr < 0 {
        return hr;
    }
    let w = std::sync::Arc::downgrade(d);
    let ops = [ServiceOp::Version, ServiceOp::ExtensionVersion(w.clone()), ServiceOp::TransportTest(w.clone()), ServiceOp::VmReady(w)];
    let mv = &**(md as *const *const MetadataVtbl);
    for op in ops {
        let a = new_service_adapter(op);
        let hr = (mv.add_handler)(md as *mut _, a);
        com_release(a);
        if hr < 0 {
            com_release(md);
            return hr;
        }
    }
    *out = md;
    S_OK
}

pub fn log_pipe_failure(hr: HRESULT) {
    unsafe { log_error_args(b"Failed 0x%X to initialize xvm named pipes\0", &[hr as u32 as usize]) };
}
