use core::ffi::c_void;

pub mod acp_state;
pub mod loudness;
pub mod xapo_format;
pub mod xma_engine;

pub const XAUDIO2_DEFAULT_PROCESSOR: u32 = 0x20;
pub const DEBUG_CONFIGURATION_OFFSET: usize = 0x390;
pub const DEBUG_CONFIGURATION_SIZE: usize = 0x18;
pub const ALLOCATOR_ID_XAUDIO: u64 = 0x81;
pub const E_NOT_FOUND: i32 = 0x8007_0490u32 as i32;
pub const CLSCTX_ALL: u32 = 0x17;
pub const E_RENDER: u32 = 0;
pub const DEVICE_STATEMASK_ALL: u32 = 0xf;
pub const CSTR_EQUAL: i32 = 2;
const DLL_PROCESS_ATTACH: u32 = 1;
const CM_GET_DEVICE_INTERFACE_LIST_ALL_DEVICES: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Guid(pub u32, pub u16, pub u16, pub [u8; 8]);

pub const CLSID_MM_DEVICE_ENUMERATOR: Guid = Guid(0xbcde0395, 0xe52f, 0x467c, [0x8e, 0x3d, 0xc4, 0x57, 0x92, 0x91, 0x69, 0x2e]);
pub const IID_IMM_DEVICE_ENUMERATOR: Guid = Guid(0xa95664d2, 0x9614, 0x4f35, [0xa7, 0x46, 0xde, 0x8d, 0xb6, 0x36, 0x17, 0xe6]);
pub const DEVINTERFACE_AUDIO_RENDER: Guid = Guid(0xe6327cad, 0xdcec, 0x4949, [0xae, 0x8a, 0x99, 0x1e, 0x97, 0x6a, 0x79, 0xd2]);

#[link(name = "kernel32")]
extern "system" {
    fn DisableThreadLibraryCalls(m: isize) -> i32;
    fn GetModuleHandleW(name: *const u16) -> isize;
    fn LoadLibraryW(name: *const u16) -> isize;
    fn GetProcAddress(m: isize, name: *const u8) -> *const c_void;
    fn CompareStringOrdinal(a: *const u16, na: i32, b: *const u16, nb: i32, ignore_case: i32) -> i32;
}

#[link(name = "ole32")]
extern "system" {
    fn CoCreateInstance(clsid: *const Guid, outer: *mut c_void, ctx: u32, iid: *const Guid, out: *mut *mut c_void) -> i32;
    fn CoTaskMemFree(p: *mut c_void);
}

#[link(name = "cfgmgr32")]
extern "system" {
    fn CM_Get_Device_Interface_List_SizeW(len: *mut u32, class: *const Guid, device: *const u16, flags: u32) -> u32;
    fn CM_Get_Device_Interface_ListW(class: *const Guid, device: *const u16, buf: *mut u16, len: u32, flags: u32) -> u32;
    fn CM_MapCrToWin32Err(cr: u32, default: u32) -> u32;
}

pub const fn xalloc_attributes(allocator_id: u64, object_type: u64, memory_type: u64, page_size: u64, alignment: u64, reserved: u64) -> u64 {
    let mut a = object_type & 0x3fff;
    a |= (allocator_id & 0xff) << 16;
    a |= (alignment & 0x1f) << 24;
    a |= (memory_type & 0xf) << 29;
    a |= (page_size & 3) << 14;
    a |= (reserved & 1) << 33;
    a & 0x3_ffff_ffff
}

pub const XAUDIO_HEAP_ATTRIBUTES: u64 = xalloc_attributes(ALLOCATOR_ID_XAUDIO, 0, 0, 0, 4, 0);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn xmem_proc(name: &[u8]) -> *const c_void {
    let dll = wide("xmem.dll");
    let mut m = GetModuleHandleW(dll.as_ptr());
    if m == 0 {
        m = LoadLibraryW(dll.as_ptr());
    }
    if m == 0 {
        return core::ptr::null();
    }
    GetProcAddress(m, name.as_ptr())
}

#[no_mangle]
pub unsafe extern "system" fn DllMain(module: isize, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        DisableThreadLibraryCalls(module);
    }
    1
}

#[no_mangle]
pub extern "system" fn GetXAudioDefaultProcessorForCurrentPlatform() -> u32 {
    XAUDIO2_DEFAULT_PROCESSOR
}

#[no_mangle]
pub extern "system" fn OnXAudioThreadAffinityMaskChanged(mask: u64) -> i32 {
    let _lowest = (mask & mask.wrapping_sub(1)) ^ mask;
    0
}

#[no_mangle]
pub unsafe extern "system" fn GetXAudioHeap(heap: *mut *mut c_void) -> i32 {
    *heap = core::ptr::null_mut();
    0
}

#[no_mangle]
pub unsafe extern "system" fn XAudioHeapAlloc(_heap: *mut c_void, size: usize) -> *mut c_void {
    let f = xmem_proc(b"XMemAlloc\0");
    if f.is_null() {
        return core::ptr::null_mut();
    }
    let f: unsafe extern "system" fn(usize, u64) -> *mut c_void = core::mem::transmute(f);
    f(size, XAUDIO_HEAP_ATTRIBUTES)
}

#[no_mangle]
pub unsafe extern "system" fn XAudioHeapFree(_heap: *mut c_void, p: *mut c_void) -> i32 {
    let f = xmem_proc(b"XMemFree\0");
    if !f.is_null() {
        let f: unsafe extern "system" fn(*mut c_void, u64) = core::mem::transmute(f);
        f(p, XAUDIO_HEAP_ATTRIBUTES);
    }
    1
}

#[no_mangle]
pub unsafe extern "system" fn SetXAudio2DebugConfiguration(state: *mut u8, config: *const u8, _reserved: *mut c_void) {
    if !state.is_null() && !config.is_null() {
        core::ptr::copy_nonoverlapping(config, state.add(DEBUG_CONFIGURATION_OFFSET), DEBUG_CONFIGURATION_SIZE);
    }
}

unsafe fn slot(o: *mut c_void, i: usize) -> usize {
    *(*(o as *const *const usize)).add(i)
}

unsafe fn release(o: *mut c_void) {
    if !o.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(slot(o, 2));
        f(o);
    }
}

unsafe fn wcslen(p: *const u16) -> usize {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    n
}

pub unsafe fn endpoint_interface_id(endpoint_id: *const u16) -> Result<Vec<u16>, i32> {
    let id = core::slice::from_raw_parts(endpoint_id, wcslen(endpoint_id));
    let mut instance: Vec<u16> = "SWD\\MMDEVAPI\\".encode_utf16().collect();
    instance.extend_from_slice(id);
    instance.push(0);
    let hr = |cr: u32| (0x8007_0000 | (CM_MapCrToWin32Err(cr, 1168) & 0xffff)) as i32;
    let mut len = 0u32;
    let cr = CM_Get_Device_Interface_List_SizeW(&mut len, &DEVINTERFACE_AUDIO_RENDER, instance.as_ptr(), CM_GET_DEVICE_INTERFACE_LIST_ALL_DEVICES);
    if cr != 0 {
        return Err(hr(cr));
    }
    let mut buf = vec![0u16; len.max(1) as usize];
    let cr = CM_Get_Device_Interface_ListW(&DEVINTERFACE_AUDIO_RENDER, instance.as_ptr(), buf.as_mut_ptr(), buf.len() as u32, CM_GET_DEVICE_INTERFACE_LIST_ALL_DEVICES);
    if cr != 0 {
        return Err(hr(cr));
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if n == 0 {
        return Err(E_NOT_FOUND);
    }
    buf.truncate(n);
    buf.push(0);
    Ok(buf)
}

#[no_mangle]
pub unsafe extern "system" fn GetAudioEndpointIdFromDeviceInterfaceId(device_interface_id: *const u16, endpoint_id: *mut *mut u16) -> i32 {
    *endpoint_id = core::ptr::null_mut();
    let mut enumerator: *mut c_void = core::ptr::null_mut();
    let mut collection: *mut c_void = core::ptr::null_mut();
    let mut found: *mut u16 = core::ptr::null_mut();
    let mut hr = CoCreateInstance(&CLSID_MM_DEVICE_ENUMERATOR, core::ptr::null_mut(), CLSCTX_ALL, &IID_IMM_DEVICE_ENUMERATOR, &mut enumerator);
    if hr >= 0 {
        let f: unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> i32 = core::mem::transmute(slot(enumerator, 3));
        hr = f(enumerator, E_RENDER, DEVICE_STATEMASK_ALL, &mut collection);
    }
    let mut count = 0u32;
    if hr >= 0 {
        let f: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 = core::mem::transmute(slot(collection, 3));
        hr = f(collection, &mut count);
    }
    if hr >= 0 {
        let mut i = 0u32;
        while hr >= 0 && i < count {
            let mut device: *mut c_void = core::ptr::null_mut();
            let item: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 = core::mem::transmute(slot(collection, 4));
            hr = item(collection, i, &mut device);
            let mut id: *mut u16 = core::ptr::null_mut();
            if hr >= 0 {
                let get_id: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> i32 = core::mem::transmute(slot(device, 5));
                hr = get_id(device, &mut id);
            }
            let mut matched = false;
            if hr >= 0 {
                if let Ok(path) = endpoint_interface_id(id) {
                    matched = CompareStringOrdinal(path.as_ptr(), -1, device_interface_id, -1, 1) == CSTR_EQUAL;
                }
            }
            release(device);
            if matched {
                found = id;
                break;
            }
            if !id.is_null() {
                CoTaskMemFree(id as *mut c_void);
            }
            i += 1;
        }
    }
    *endpoint_id = found;
    release(collection);
    release(enumerator);
    hr
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod acp_tests;
#[cfg(test)]
mod loudness_tests;
#[cfg(test)]
pub(crate) mod diagveh;
#[cfg(test)]
mod xma_engine_tests;
