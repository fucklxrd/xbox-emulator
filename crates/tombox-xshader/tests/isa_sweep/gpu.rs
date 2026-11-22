use super::d3d::{self, Gpu, Guid};
use crate::vcall;
use core::ffi::c_void;
use std::sync::{Mutex, OnceLock};

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

pub struct Dev {
    pub gpu: Gpu,
    pub rs: *mut c_void,
    pub name: &'static str,
}

unsafe impl Send for Dev {}

static DEV: OnceLock<Mutex<Result<Dev, String>>> = OnceLock::new();

pub fn hw() -> bool {
    std::env::var_os("TOMBOX_EXEC_HW").is_some()
}

unsafe fn warp_gpu() -> Result<Gpu, String> {
    const IID_FACTORY4: Guid = Guid(0x1bc6ea02, 0xef36, 0x464f, [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a]);
    const IID_ADAPTER: Guid = Guid(0x2411e7e1, 0x12ac, 0x4ccf, [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0]);
    let dxgi = LoadLibraryA(b"dxgi.dll\0".as_ptr());
    let cf: unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(dxgi, b"CreateDXGIFactory1\0".as_ptr()));
    let mut f: *mut c_void = core::ptr::null_mut();
    if cf(&IID_FACTORY4, &mut f) < 0 {
        return Err("CreateDXGIFactory1".into());
    }
    let mut ad: *mut c_void = core::ptr::null_mut();
    if vcall!(f, 27, fn(*const Guid, *mut *mut c_void) -> i32, &IID_ADAPTER, &mut ad) < 0 {
        return Err("EnumWarpAdapter".into());
    }
    let d3dl = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let create: unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(d3dl, b"D3D12CreateDevice\0".as_ptr()));
    let mut device: *mut c_void = core::ptr::null_mut();
    if create(ad, 0xc100, &d3d::IID_DEVICE, &mut device) < 0 {
        return Err("D3D12CreateDevice(WARP)".into());
    }
    let qd: [u32; 4] = [0, 0, 0, 0];
    let mut queue: *mut c_void = core::ptr::null_mut();
    vcall!(device, 8, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &d3d::IID_QUEUE, &mut queue);
    let mut alloc: *mut c_void = core::ptr::null_mut();
    vcall!(device, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &d3d::IID_ALLOCATOR, &mut alloc);
    let mut list: *mut c_void = core::ptr::null_mut();
    vcall!(device, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, alloc, core::ptr::null_mut(), &d3d::IID_GFX_LIST, &mut list);
    vcall!(list, 9, fn() -> i32);
    let mut fence: *mut c_void = core::ptr::null_mut();
    vcall!(device, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &d3d::IID_FENCE, &mut fence);
    Ok(Gpu { device, queue, alloc, list, fence, value: 0, info: core::ptr::null_mut(), open: false })
}

unsafe fn root_sig(g: &Gpu) -> Result<*mut c_void, String> {
    let d3dl = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let ser: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 =
        core::mem::transmute(GetProcAddress(d3dl, b"D3D12SerializeRootSignature\0".as_ptr()));
    let range: [u32; 5] = [1, 8, 0, 0, 0xffff_ffff];
    let mut ps = [0u8; 32];
    ps[8..12].copy_from_slice(&1u32.to_le_bytes());
    ps[16..24].copy_from_slice(&(range.as_ptr() as u64).to_le_bytes());
    let mut desc = [0u8; 40];
    desc[0..4].copy_from_slice(&1u32.to_le_bytes());
    desc[8..16].copy_from_slice(&(ps.as_ptr() as u64).to_le_bytes());
    let mut blob: *mut c_void = core::ptr::null_mut();
    let mut err: *mut c_void = core::ptr::null_mut();
    if ser(desc.as_ptr(), 1, &mut blob, &mut err) < 0 {
        return Err("serialize root signature".into());
    }
    let p = vcall!(blob, 3, fn() -> *const u8);
    let n = vcall!(blob, 4, fn() -> usize);
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(g.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, p, n, &d3d::IID_ROOTSIG, &mut rs);
    if hr < 0 {
        return Err(format!("CreateRootSignature {hr:#x}"));
    }
    Ok(rs)
}

pub fn with_dev<R>(f: impl FnOnce(&mut Dev) -> Result<R, String>) -> Result<R, String> {
    let m = DEV.get_or_init(|| {
        Mutex::new(unsafe {
            let g = if hw() {
                Gpu::new(false).map(|mut g| {
                    g.open = false;
                    g
                })
            } else {
                warp_gpu()
            };
            g.and_then(|g| root_sig(&g).map(|rs| Dev { gpu: g, rs, name: if hw() { "hardware" } else { "WARP" } }))
        })
    });
    let mut d = m.lock().unwrap_or_else(|e| e.into_inner());
    match d.as_mut() {
        Ok(d) => f(d),
        Err(e) => Err(format!("no D3D12 device: {e}")),
    }
}

pub fn uav_layout(hlsl: &str) -> (u32, u32) {
    if hlsl.contains("RWStructuredBuffer<TbElem_ubuf0>") {
        let n: u32 = hlsl.split("struct TbElem_ubuf0 { uint w[").nth(1).and_then(|s| s.split(']').next()).and_then(|s| s.parse().ok()).unwrap_or(1);
        (n * 4, 0)
    } else if hlsl.contains("RWStructuredBuffer<uint> ubuf0") {
        (4, 0)
    } else if hlsl.contains("RWBuffer<uint4>") {
        (0, 3)
    } else if hlsl.contains("RWBuffer<uint>") {
        (0, 42)
    } else {
        (0, 41)
    }
}

pub unsafe fn dispatch(d: &mut Dev, code: &[u8], layout: (u32, u32), data: &[u32]) -> Result<Vec<u32>, String> {
    let g = &mut d.gpu;
    let nbytes = (data.len() * 4) as u64;
    let mut desc = [0u8; 56];
    desc[0..8].copy_from_slice(&(d.rs as u64).to_le_bytes());
    desc[8..16].copy_from_slice(&(code.as_ptr() as u64).to_le_bytes());
    desc[16..24].copy_from_slice(&(code.len() as u64).to_le_bytes());
    let mut pso: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(g.device, 11, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, desc.as_ptr(), &d3d::IID_PSO, &mut pso);
    if hr < 0 {
        return Err(format!("CreateComputePipelineState hr={:#010x}{}", hr as u32, g.messages()));
    }
    let mut kill = vec![pso];
    let r = (|| -> Result<Vec<u32>, String> {
        let (vh, vcpu, vgpu, vinc) = g.heap(0, 8, true)?;
        kill.push(vh);
        let ub = g.buffer(nbytes, 1, d3d::COPY_DEST, true)?;
        kill.push(ub);
        let up = g.buffer(nbytes, 2, d3d::GENERIC_READ, false)?;
        kill.push(up);
        let rb = g.buffer(nbytes, 3, d3d::COPY_DEST, false)?;
        kill.push(rb);
        let bytes: Vec<u8> = data.iter().flat_map(|w| w.to_le_bytes()).collect();
        Gpu::map_write(up, &bytes);
        let (stride, fmt) = layout;
        let esz = if stride != 0 { stride } else if fmt == 3 { 16 } else { 4 };
        for slot in 0..8usize {
            let mut ud = [0u8; 40];
            ud[0..4].copy_from_slice(&fmt.to_le_bytes());
            ud[4..8].copy_from_slice(&1u32.to_le_bytes());
            ud[16..20].copy_from_slice(&((nbytes as u32) / esz).to_le_bytes());
            ud[20..24].copy_from_slice(&stride.to_le_bytes());
            vcall!(g.device, 19, fn(*mut c_void, *mut c_void, *const u8, usize) -> (), ub, core::ptr::null_mut(), ud.as_ptr(), vcpu + slot * vinc);
        }
        g.begin();
        let l = g.list;
        vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), ub, 0, up, 0, nbytes);
        g.barrier(ub, d3d::COPY_DEST, 8);
        let heaps = [vh];
        vcall!(l, 28, fn(u32, *const *mut c_void) -> (), 1, heaps.as_ptr());
        vcall!(l, 29, fn(*mut c_void) -> (), d.rs);
        vcall!(l, 31, fn(u32, u64) -> (), 0, vgpu);
        vcall!(l, 25, fn(*mut c_void) -> (), pso);
        vcall!(l, 14, fn(u32, u32, u32) -> (), 1, 1, 1);
        g.barrier(ub, 8, d3d::COPY_SOURCE);
        vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), rb, 0, ub, 0, nbytes);
        g.submit()?;
        let b = Gpu::map_read(rb, nbytes as usize).ok_or("map readback")?;
        Ok(b.chunks(4).map(|x| u32::from_le_bytes(x.try_into().unwrap())).collect())
    })();
    for k in kill {
        d3d::release(k);
    }
    r
}
