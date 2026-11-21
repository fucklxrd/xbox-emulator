#[path = "common/media.rs"]
mod media;

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
#[path = "../src/reset.rs"]
#[allow(dead_code)]
mod reset;
#[allow(dead_code)]
mod exports {
    include!(concat!(env!("OUT_DIR"), "/forwarded.rs"));
}
use exports::{CONSOLE_EXPORTS, FORWARDED, FORWARDED_ALIASES};
use reset::{query, release, Guid, IID_ID3D12DEVICE, IID_IUNKNOWN};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

type H = *mut c_void;

extern "system" {
    fn LoadLibraryW(name: *const u16) -> H;
    fn GetProcAddress(h: H, name: *const u8) -> *mut c_void;
    fn GetModuleHandleW(name: *const u16) -> H;
    fn GetModuleHandleExW(flags: u32, addr: *const u16, out: *mut H) -> i32;
    fn SetDllDirectoryW(dir: *const u16) -> i32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: H) -> H;
    fn CloseHandle(h: H) -> i32;
}

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
    fn CoTaskMemFree(p: *mut c_void);
}

const MF_VERSION: u32 = 0x0002_0070;
const MF_E_SHUTDOWN: i32 = 0xC00D_3E85u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const READER_FIRST_VIDEO: u32 = 0xFFFF_FFFC;
const READER_FIRST_AUDIO: u32 = 0xFFFF_FFFD;
const READERF_ERROR: u32 = 0x1;
const READERF_ENDOFSTREAM: u32 = 0x2;

const fn g(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Guid {
    Guid { d1, d2, d3, d4 }
}

const MF_MT_MAJOR_TYPE: Guid = g(0x48eba18e, 0xf8c9, 0x4687, [0xbf, 0x11, 0x0a, 0x74, 0xc9, 0xf9, 0x6a, 0x8f]);
const MF_MT_SUBTYPE: Guid = g(0xf7e34c9a, 0x42e8, 0x4714, [0xb7, 0x4b, 0xcb, 0x29, 0xd7, 0x2c, 0x35, 0xe5]);
const MF_MT_FRAME_SIZE: Guid = g(0x1652c33d, 0xd6b2, 0x4012, [0xb8, 0x34, 0x72, 0x03, 0x08, 0x49, 0xa3, 0x7d]);
const MF_MT_FRAME_RATE: Guid = g(0xc459a2e8, 0x3d2c, 0x4e44, [0xb1, 0x32, 0xfe, 0xe5, 0x15, 0x6c, 0x7b, 0xb0]);
const MF_MT_PIXEL_ASPECT_RATIO: Guid = g(0xc6376a1e, 0x8d0a, 0x4027, [0xbe, 0x45, 0x6d, 0x9a, 0x0a, 0xd3, 0x9b, 0xb6]);
const MF_MT_INTERLACE_MODE: Guid = g(0xe2724bb8, 0xe676, 0x4806, [0xb4, 0xb2, 0xa8, 0xd6, 0xef, 0xb4, 0x4c, 0xcd]);
const MF_MT_AVG_BITRATE: Guid = g(0x20332624, 0xfb0d, 0x4d9e, [0xbd, 0x0d, 0xcb, 0xf6, 0x78, 0x6c, 0x10, 0x2e]);
const MF_MT_AUDIO_NUM_CHANNELS: Guid = g(0x37e48bf5, 0x645e, 0x4c5b, [0x89, 0xde, 0xad, 0xa9, 0xe2, 0x9b, 0x69, 0x6a]);
const MF_MT_AUDIO_SAMPLES_PER_SECOND: Guid = g(0x5faeeae7, 0x0290, 0x4c31, [0x9e, 0x8a, 0xc5, 0x34, 0xf6, 0x8d, 0x9d, 0xba]);
const MF_MT_AUDIO_BITS_PER_SAMPLE: Guid = g(0xf2deb57f, 0x40fa, 0x4764, [0xaa, 0x33, 0xed, 0x4f, 0x2d, 0x1f, 0xf6, 0x69]);
const MF_SOURCE_READER_D3D_MANAGER: Guid = g(0xec822da2, 0xe1e9, 0x4b29, [0xa0, 0xd8, 0x56, 0x3c, 0x71, 0x9f, 0x52, 0x69]);
const MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING: Guid = g(0x0f81da2c, 0xb537, 0x4672, [0xa8, 0xb2, 0xa6, 0x81, 0xb1, 0x73, 0x07, 0xa3]);
const MF_SINK_WRITER_DISABLE_THROTTLING: Guid = g(0x08b845d8, 0x2b74, 0x4afe, [0x9d, 0x53, 0xbe, 0x16, 0xd2, 0xd5, 0xae, 0x4f]);
const MF_MEDIATYPE_VIDEO: Guid = g(0x73646976, 0x0000, 0x0010, [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
const MF_MEDIATYPE_AUDIO: Guid = g(0x73647561, 0x0000, 0x0010, [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
const MF_AUDIOFORMAT_PCM: Guid = g(0x00000001, 0x0000, 0x0010, [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
const MF_VIDEOFORMAT_RGB32: Guid = g(0x00000016, 0x0000, 0x0010, [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
const MF_VIDEOFORMAT_NV12: Guid = g(0x3231564e, 0x0000, 0x0010, [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
const MF_VIDEOFORMAT_H264: Guid = g(0x34363248, 0x0000, 0x0010, [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
const IID_IMFMEDIASOURCE: Guid = g(0x279a808d, 0xaec7, 0x40c8, [0x9c, 0x6b, 0xa6, 0xb4, 0x92, 0xc7, 0x8a, 0x66]);
const IID_IMFDXGIBUFFER: Guid = g(0xe7174cfa, 0x1c9e, 0x48b1, [0x88, 0x66, 0x62, 0x62, 0x26, 0xbf, 0xc2, 0x58]);
const IID_ID3D12RESOURCE: Guid = g(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);

struct Proxy {
    dir: PathBuf,
    proxy: usize,
    sys: usize,
    rw: usize,
}

static PROXY: OnceLock<Proxy> = OnceLock::new();
static MF_LOCK: Mutex<()> = Mutex::new(());

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn wp(p: &Path) -> Vec<u16> {
    w(&p.to_string_lossy())
}

fn built_dll() -> PathBuf {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let target = tmp.parent().unwrap().to_path_buf();
    let exe = std::env::current_exe().unwrap();
    let mut cands = Vec::new();
    for profile in ["release", "debug"] {
        cands.push(target.join(profile).join("mfplat.dll"));
        cands.push(target.join(profile).join("deps").join("mfplat.dll"));
    }
    for a in exe.ancestors().take(4) {
        cands.push(a.join("mfplat.dll"));
        cands.push(a.join("deps").join("mfplat.dll"));
    }
    let mut found: Vec<PathBuf> = cands.into_iter().filter(|p| p.exists()).collect();
    found.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    found.pop().expect("built mfplat.dll (cdylib) not found next to the test binary")
}

fn module_of(addr: usize) -> usize {
    let mut h: H = core::ptr::null_mut();
    let ok = unsafe { GetModuleHandleExW(4 | 2, addr as *const u16, &mut h) };
    if ok == 0 {
        0
    } else {
        h as usize
    }
}

fn proxy() -> &'static Proxy {
    PROXY.get_or_init(|| {
        let already = unsafe { GetModuleHandleW(w("mfplat.dll").as_ptr()) };
        assert!(already.is_null(), "system mfplat.dll was loaded before the proxy; the binding test would be meaningless");
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("mfplat_proxy_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(built_dll(), dir.join("mfplat.dll")).unwrap();
        let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        std::fs::copy(Path::new(&sysroot).join("System32").join("mfplat.dll"), dir.join("mfplat_sys.dll")).unwrap();
        std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
        assert!(unsafe { SetDllDirectoryW(wp(&dir).as_ptr()) } != 0);
        let proxy = unsafe { LoadLibraryW(wp(&dir.join("mfplat.dll")).as_ptr()) };
        assert!(!proxy.is_null(), "load proxy mfplat.dll");
        let sys = unsafe { LoadLibraryW(wp(&dir.join("mfplat_sys.dll")).as_ptr()) };
        assert!(!sys.is_null(), "load mfplat_sys.dll");
        let rw = unsafe { LoadLibraryW(w("mfreadwrite.dll").as_ptr()) };
        assert!(!rw.is_null(), "load system mfreadwrite.dll");
        unsafe { CoInitializeEx(core::ptr::null_mut(), 0) };
        Proxy { dir, proxy: proxy as usize, sys: sys as usize, rw: rw as usize }
    })
}

fn px(name: &str) -> usize {
    let p = proxy();
    let c = format!("{name}\0");
    let a = unsafe { GetProcAddress(p.proxy as H, c.as_ptr()) } as usize;
    assert!(a != 0, "proxy does not export {name}");
    a
}

fn rwx(name: &str) -> usize {
    let p = proxy();
    let c = format!("{name}\0");
    let a = unsafe { GetProcAddress(p.rw as H, c.as_ptr()) } as usize;
    assert!(a != 0, "mfreadwrite does not export {name}");
    a
}

unsafe fn vt(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

macro_rules! call {
    ($obj:expr, $slot:expr, fn($($t:ty),*) -> $r:ty, $($a:expr),*) => {{
        let o: *mut c_void = $obj;
        let f: unsafe extern "system" fn(*mut c_void $(, $t)*) -> $r = unsafe { core::mem::transmute(vt(o, $slot)) };
        unsafe { f(o $(, $a)*) }
    }};
}

macro_rules! export {
    ($addr:expr, fn($($t:ty),*) -> $r:ty) => {{
        let f: unsafe extern "system" fn($($t),*) -> $r = unsafe { core::mem::transmute($addr) };
        f
    }};
}

fn hr_ok(hr: i32, what: &str) {
    assert!(hr >= 0, "{what} failed hr={:#010x}", hr as u32);
}

struct Started;

impl Drop for Started {
    fn drop(&mut self) {
        shutdown();
    }
}

fn startup() -> Started {
    let f = export!(px("MFStartup"), fn(u32, u32) -> i32);
    hr_ok(unsafe { f(MF_VERSION, 0) }, "MFStartup");
    Started
}

fn shutdown() {
    let f = export!(px("MFShutdown"), fn() -> i32);
    hr_ok(unsafe { f() }, "MFShutdown");
}

fn attributes(n: u32) -> *mut c_void {
    let f = export!(px("MFCreateAttributes"), fn(*mut *mut c_void, u32) -> i32);
    let mut a = core::ptr::null_mut();
    hr_ok(unsafe { f(&mut a, n) }, "MFCreateAttributes");
    a
}

fn set_u32(a: *mut c_void, k: &Guid, v: u32) {
    hr_ok(call!(a, 21, fn(*const Guid, u32) -> i32, k, v), "SetUINT32");
}

fn set_u64(a: *mut c_void, k: &Guid, v: u64) {
    hr_ok(call!(a, 22, fn(*const Guid, u64) -> i32, k, v), "SetUINT64");
}

fn set_guid(a: *mut c_void, k: &Guid, v: &Guid) {
    hr_ok(call!(a, 24, fn(*const Guid, *const Guid) -> i32, k, v), "SetGUID");
}

fn set_unknown(a: *mut c_void, k: &Guid, v: *mut c_void) {
    hr_ok(call!(a, 27, fn(*const Guid, *mut c_void) -> i32, k, v), "SetUnknown");
}

fn get_u32(a: *mut c_void, k: &Guid) -> u32 {
    let mut v = 0u32;
    hr_ok(call!(a, 7, fn(*const Guid, *mut u32) -> i32, k, &mut v), "GetUINT32");
    v
}

fn get_guid(a: *mut c_void, k: &Guid) -> Guid {
    let mut v = IID_IUNKNOWN;
    hr_ok(call!(a, 10, fn(*const Guid, *mut Guid) -> i32, k, &mut v), "GetGUID");
    v
}

fn media_type() -> *mut c_void {
    let f = export!(px("MFCreateMediaType"), fn(*mut *mut c_void) -> i32);
    let mut t = core::ptr::null_mut();
    hr_ok(unsafe { f(&mut t) }, "MFCreateMediaType");
    t
}

fn memory_buffer(len: u32) -> *mut c_void {
    let f = export!(px("MFCreateMemoryBuffer"), fn(u32, *mut *mut c_void) -> i32);
    let mut b = core::ptr::null_mut();
    hr_ok(unsafe { f(len, &mut b) }, "MFCreateMemoryBuffer");
    b
}

fn sample() -> *mut c_void {
    let f = export!(px("MFCreateSample"), fn(*mut *mut c_void) -> i32);
    let mut s = core::ptr::null_mut();
    hr_ok(unsafe { f(&mut s) }, "MFCreateSample");
    s
}

fn byte_stream(path: &Path) -> *mut c_void {
    let f = export!(px("MFCreateFile"), fn(u32, u32, u32, *const u16, *mut *mut c_void) -> i32);
    let mut bs = core::ptr::null_mut();
    hr_ok(unsafe { f(1, 0, 0, wp(path).as_ptr(), &mut bs) }, "MFCreateFile");
    bs
}

fn sample_bytes(s: *mut c_void) -> u32 {
    let mut n = 0u32;
    hr_ok(call!(s, 45, fn(*mut u32) -> i32, &mut n), "GetTotalLength");
    n
}

struct ReadStats {
    samples: u32,
    bytes: u64,
    dxgi_samples: u32,
    d3d12_on_device: u32,
}

fn drain(reader: *mut c_void, stream: u32) -> ReadStats {
    drain_on(reader, stream, core::ptr::null_mut())
}

fn d3d12_resource_on(dxgi: *mut c_void, device: *mut c_void) -> bool {
    let mut res: *mut c_void = core::ptr::null_mut();
    if call!(dxgi, 3, fn(*const Guid, *mut *mut c_void) -> i32, &IID_ID3D12RESOURCE, &mut res) < 0 || res.is_null() {
        return false;
    }
    let mut dev: *mut c_void = core::ptr::null_mut();
    let ok = call!(res, 7, fn(*const Guid, *mut *mut c_void) -> i32, &IID_ID3D12DEVICE, &mut dev) >= 0 && dev == device;
    unsafe {
        if !dev.is_null() {
            release(dev);
        }
        release(res);
    }
    ok
}

fn drain_on(reader: *mut c_void, stream: u32, device: *mut c_void) -> ReadStats {
    let mut st = ReadStats { samples: 0, bytes: 0, dxgi_samples: 0, d3d12_on_device: 0 };
    for _ in 0..100_000 {
        let mut actual = 0u32;
        let mut flags = 0u32;
        let mut ts = 0i64;
        let mut s: *mut c_void = core::ptr::null_mut();
        let hr = call!(reader, 9, fn(u32, u32, *mut u32, *mut u32, *mut i64, *mut *mut c_void) -> i32, stream, 0, &mut actual, &mut flags, &mut ts, &mut s);
        hr_ok(hr, "IMFSourceReader::ReadSample");
        assert_eq!(flags & READERF_ERROR, 0, "reader error flag");
        if !s.is_null() {
            st.samples += 1;
            st.bytes += sample_bytes(s) as u64;
            let mut b: *mut c_void = core::ptr::null_mut();
            if call!(s, 40, fn(u32, *mut *mut c_void) -> i32, 0, &mut b) >= 0 && !b.is_null() {
                if let Ok(d) = unsafe { query(b, &IID_IMFDXGIBUFFER) } {
                    st.dxgi_samples += 1;
                    if !device.is_null() && d3d12_resource_on(d, device) {
                        st.d3d12_on_device += 1;
                    }
                    unsafe { release(d) };
                }
                unsafe { release(b) };
            }
            unsafe { release(s) };
        }
        if flags & READERF_ENDOFSTREAM != 0 {
            return st;
        }
    }
    panic!("reader never reached end of stream");
}

fn write_wav(path: &Path, rate: u32, channels: u16, frames: u32) -> u32 {
    let data_len = frames * channels as u32 * 2;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    v.extend_from_slice(&(channels * 2).to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        let s = ((i as f32 * 440.0 * 2.0 * core::f32::consts::PI / rate as f32).sin() * 8000.0) as i16;
        for _ in 0..channels {
            v.extend_from_slice(&s.to_le_bytes());
        }
    }
    std::fs::write(path, v).unwrap();
    data_len
}

unsafe fn imports_from(module: usize, dll: &str) -> Vec<(String, usize)> {
    let b = module as *const u8;
    let rd32 = |o: usize| unsafe { (b.add(o) as *const u32).read_unaligned() };
    let rd64 = |o: usize| unsafe { (b.add(o) as *const u64).read_unaligned() };
    let pe = rd32(0x3c) as usize;
    let opt = pe + 24;
    let delay = rd32(opt + 112 + 13 * 8) as usize;
    let mut out = Vec::new();
    let mut d = delay;
    loop {
        let name = rd32(d + 4) as usize;
        let hmod = rd32(d + 8) as usize;
        let ft = rd32(d + 12) as usize;
        let oft = rd32(d + 16) as usize;
        if name == 0 {
            break;
        }
        let n = unsafe { std::ffi::CStr::from_ptr(b.add(name) as *const i8) }.to_string_lossy().to_string();
        if n.eq_ignore_ascii_case(dll) {
            out.push(("<module>".to_string(), rd64(hmod) as usize));
            let mut i = 0;
            loop {
                let thunk = rd64(oft + i * 8);
                if thunk == 0 {
                    break;
                }
                let fname = if thunk & (1 << 63) != 0 {
                    format!("#{}", thunk & 0xffff)
                } else {
                    unsafe { std::ffi::CStr::from_ptr(b.add(thunk as usize + 2) as *const i8) }.to_string_lossy().to_string()
                };
                out.push((fname, rd64(ft + i * 8) as usize));
                i += 1;
            }
        }
        d += 32;
    }
    out
}

#[test]
fn every_console_and_pc_export_resolves_through_proxy() {
    let p = proxy();
    let natives = [
        "MFStartup",
        "MFShutdown",
        "MFResetDXGIDeviceManagerX",
        "MFGetPlatform",
        "LFGetGlobalPool",
        "MFBlockThread",
        "MFUnblockThread",
        "MFJoinIoPort",
        "MFGetPrivateWorkqueues",
    ];
    for n in natives {
        assert_eq!(module_of(px(n)), p.proxy, "{n} must be implemented inside the proxy");
    }
    for n in FORWARDED {
        let m = module_of(px(n));
        assert!(m != 0 && m != p.proxy, "{n} must be forwarded out of the proxy (landed in {m:#x})");
    }
    for (alias, real) in FORWARDED_ALIASES {
        let a = px(alias);
        let c = format!("{real}\0");
        let r = unsafe { GetProcAddress(p.sys as H, c.as_ptr()) } as usize;
        assert_eq!(a, r, "{alias} must forward to mfplat_sys!{real}");
    }
    let mut missing = Vec::new();
    for n in CONSOLE_EXPORTS {
        let c = format!("{n}\0");
        if unsafe { GetProcAddress(p.proxy as H, c.as_ptr()) }.is_null() {
            missing.push(*n);
        }
    }
    assert!(missing.is_empty(), "console mfplat exports missing from proxy: {missing:?}");
    assert_eq!(CONSOLE_EXPORTS.len(), 140);
    for n in ["MFCreateAttributes", "MFCreateMediaType", "MFCreateSample", "MFllMulDiv"] {
        let c = format!("{n}\0");
        let real = unsafe { GetProcAddress(p.sys as H, c.as_ptr()) } as usize;
        assert_eq!(px(n), real, "{n} forwarder must resolve to the exact mfplat_sys export");
    }
    let rtwq = unsafe { GetModuleHandleW(w("rtworkq.dll").as_ptr()) } as usize;
    assert_eq!(module_of(px("MFInvokeCallback")), rtwq, "chained forwarder mfplat_sys -> RTWorkQ");
}

#[test]
fn pc_mfreadwrite_binds_its_mfplat_imports_to_the_proxy() {
    let p = proxy();
    let loaded = unsafe { GetModuleHandleW(w("mfplat.dll").as_ptr()) } as usize;
    assert_eq!(loaded, p.proxy, "base name mfplat.dll must be the proxy");
    {
        let _l = MF_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _s = startup();
        let path = p.dir.join("bind.wav");
        write_wav(&path, 48_000, 2, 4_800);
        let from_bs = export!(rwx("MFCreateSourceReaderFromByteStream"), fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32);
        let bs = byte_stream(&path);
        let mut reader = core::ptr::null_mut();
        hr_ok(unsafe { from_bs(bs, core::ptr::null_mut(), &mut reader) }, "MFCreateSourceReaderFromByteStream");
        drain(reader, READER_FIRST_AUDIO);
        unsafe {
            release(reader);
            release(bs);
        }
    }
    let kb = unsafe { LoadLibraryW(w("kernelbase.dll").as_ptr()) };
    let resolve = unsafe { GetProcAddress(kb, b"ResolveDelayLoadsFromDll\0".as_ptr()) };
    assert!(!resolve.is_null(), "kernelbase!ResolveDelayLoadsFromDll");
    let resolve = export!(resolve, fn(*mut c_void, *const u8, u32) -> i32);
    hr_ok(unsafe { resolve(p.rw as *mut c_void, b"MFPlat.DLL\0".as_ptr(), 0) }, "ResolveDelayLoadsFromDll(mfreadwrite, MFPlat.DLL)");
    let imps = unsafe { imports_from(p.rw, "mfplat.dll") };
    let (_, hmod) = imps.first().cloned().expect("mfreadwrite delay-imports mfplat.dll");
    assert!(hmod == 0 || hmod == p.proxy, "mfreadwrite's delay-load module handle for MFPlat.DLL is {hmod:#x}, not the proxy");
    let names = &imps[1..];
    assert!(names.len() >= 20, "mfreadwrite delay-imports {} names from mfplat", names.len());
    let (lo, hi) = (p.rw, p.rw + 0x0100_0000);
    let mut bound = 0;
    for (name, addr) in names {
        let c = format!("{name}\0");
        let via_proxy = unsafe { GetProcAddress(p.proxy as H, c.as_ptr()) } as usize;
        assert!(via_proxy != 0, "proxy cannot satisfy mfreadwrite's delay import {name}");
        if !(lo..hi).contains(addr) {
            assert_eq!(*addr, via_proxy, "mfreadwrite!{name} must be bound through the proxy");
            bound += 1;
        }
    }
    assert_eq!(bound, names.len(), "ResolveDelayLoadsFromDll must have bound every MFPlat.DLL delay import of mfreadwrite to the proxy");
}

#[test]
fn startup_and_console_only_exports_follow_console_semantics() {
    let _l = MF_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let get_private = export!(px("MFGetPrivateWorkqueues"), fn(*mut u32) -> i32);
    let block = export!(px("MFBlockThread"), fn() -> i32);
    let unblock = export!(px("MFUnblockThread"), fn() -> i32);
    let platform = export!(px("MFGetPlatform"), fn() -> *mut c_void);
    let pool = export!(px("LFGetGlobalPool"), fn() -> *mut c_void);
    let join = export!(px("MFJoinIoPort"), fn(H) -> i32);
    let mut v = 7u32;
    assert_eq!(unsafe { get_private(&mut v) }, MF_E_SHUTDOWN);
    assert_eq!(v, 7);
    assert_eq!(unsafe { block() }, MF_E_SHUTDOWN);
    assert_eq!(unsafe { unblock() }, MF_E_SHUTDOWN);
    assert!(unsafe { pool() }.is_null());
    let s1 = startup();
    assert_eq!(unsafe { get_private(core::ptr::null_mut()) }, E_INVALIDARG);
    assert_eq!(unsafe { get_private(&mut v) }, 0);
    assert_eq!(v, host_assign_queues());
    assert_eq!(unsafe { block() }, 0);
    assert_eq!(unsafe { unblock() }, 0);
    assert!(unsafe { platform() }.is_null());
    assert!(unsafe { pool() }.is_null());
    let path = proxy().dir.join("iocp.bin");
    std::fs::write(&path, [0u8; 16]).unwrap();
    let h = unsafe { CreateFileW(wp(&path).as_ptr(), 0x8000_0000, 1, core::ptr::null_mut(), 3, 0x4000_0000, core::ptr::null_mut()) };
    assert!(h as isize != -1);
    hr_ok(unsafe { join(h) }, "MFJoinIoPort");
    unsafe { CloseHandle(h) };
    let s2 = startup();
    drop(s2);
    assert_eq!(unsafe { block() }, 0, "still started after nested MFStartup/MFShutdown");
    drop(s1);
    assert_eq!(unsafe { get_private(&mut v) }, MF_E_SHUTDOWN);
    let log = std::fs::read_to_string(proxy().dir.join("tombox_compat.log")).unwrap_or_default();
    assert!(log.contains("[mfplat] MFStartup version=0x20070"), "MFStartup must be logged: {log}");
}

#[test]
fn forwarded_exports_keep_full_signatures() {
    let _l = MF_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _mf = startup();
    let muldiv = export!(px("MFllMulDiv"), fn(i64, i64, i64, i64) -> i64);
    let cases: [(i64, i64, i64, i64); 4] = [(1 << 40, 3 << 20, 7, 5), (-123_456_789, 987_654_321, 1000, 0), (10_000_000, 48_000, 44_100, 22_050), (i64::MAX / 3, 3, 4, 1)];
    for (a, b, c, d) in cases {
        let want = ((a as i128 * b as i128 + d as i128) / c as i128) as i64;
        assert_eq!(unsafe { muldiv(a, b, c, d) }, want, "MFllMulDiv({a},{b},{c},{d})");
    }
    let frt = export!(px("MFFrameRateToAverageTimePerFrame"), fn(u32, u32, *mut u64) -> i32);
    let mut t = 0u64;
    hr_ok(unsafe { frt(30, 1, &mut t) }, "MFFrameRateToAverageTimePerFrame");
    assert_eq!(t, 333_333);
    let a = attributes(4);
    let k = MF_MT_AUDIO_NUM_CHANNELS;
    set_u32(a, &k, 6);
    assert_eq!(get_u32(a, &k), 6);
    unsafe { release(a) };
    let b = memory_buffer(1024);
    let mut max = 0u32;
    hr_ok(call!(b, 7, fn(*mut u32) -> i32, &mut max), "GetMaxLength");
    assert_eq!(max, 1024);
    let aligned = export!(px("MFCreateAlignedMemoryBuffer"), fn(u32, u32, *mut *mut c_void) -> i32);
    let mut ab = core::ptr::null_mut();
    hr_ok(unsafe { aligned(100, 63, &mut ab) }, "MFCreateAlignedMemoryBuffer");
    let mut ptr: *mut u8 = core::ptr::null_mut();
    hr_ok(call!(ab, 3, fn(*mut *mut u8, *mut u32, *mut u32) -> i32, &mut ptr, core::ptr::null_mut(), core::ptr::null_mut()), "Lock");
    assert_eq!(ptr as usize % 64, 0);
    call!(ab, 4, fn() -> i32,);
    let s = sample();
    hr_ok(call!(s, 42, fn(*mut c_void) -> i32, b), "AddBuffer");
    let mut cnt = 0u32;
    hr_ok(call!(s, 39, fn(*mut u32) -> i32, &mut cnt), "GetBufferCount");
    assert_eq!(cnt, 1);
    unsafe {
        release(s);
        release(b);
        release(ab);
    }
    let mt = media_type();
    set_guid(mt, &MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_AUDIO);
    set_guid(mt, &MF_MT_SUBTYPE, &MF_AUDIOFORMAT_PCM);
    set_u32(mt, &MF_MT_AUDIO_NUM_CHANNELS, 2);
    set_u32(mt, &MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000);
    set_u32(mt, &MF_MT_AUDIO_BITS_PER_SAMPLE, 16);
    let wfx_of = export!(px("MFCreateWaveFormatExFromMFMediaType"), fn(*mut c_void, *mut *mut u8, *mut u32, u32) -> i32);
    let mut wfx: *mut u8 = core::ptr::null_mut();
    let mut size = 0u32;
    hr_ok(unsafe { wfx_of(mt, &mut wfx, &mut size, 0) }, "MFCreateWaveFormatExFromMFMediaType");
    let fmt = unsafe { core::slice::from_raw_parts(wfx, 16) };
    assert_eq!(u16::from_le_bytes([fmt[0], fmt[1]]), 1);
    assert_eq!(u16::from_le_bytes([fmt[2], fmt[3]]), 2);
    assert_eq!(u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]), 48_000);
    assert_eq!(u16::from_le_bytes([fmt[14], fmt[15]]), 16);
    unsafe { CoTaskMemFree(wfx as *mut c_void) };
    unsafe { release(mt) };
}

#[test]
fn pc_source_reader_reads_through_proxy_from_byte_stream_and_media_source() {
    let _l = MF_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _mf = startup();
    let path = proxy().dir.join("tone.wav");
    let data_len = write_wav(&path, 48_000, 2, 24_000);
    let from_bs = export!(rwx("MFCreateSourceReaderFromByteStream"), fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32);
    let from_src = export!(rwx("MFCreateSourceReaderFromMediaSource"), fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32);

    let bs = byte_stream(&path);
    let attrs = attributes(1);
    let mut reader = core::ptr::null_mut();
    hr_ok(unsafe { from_bs(bs, attrs, &mut reader) }, "MFCreateSourceReaderFromByteStream");
    let mut native = core::ptr::null_mut();
    hr_ok(call!(reader, 5, fn(u32, u32, *mut *mut c_void) -> i32, READER_FIRST_AUDIO, 0, &mut native), "GetNativeMediaType");
    assert_eq!(get_guid(native, &MF_MT_MAJOR_TYPE), MF_MEDIATYPE_AUDIO);
    assert_eq!(get_u32(native, &MF_MT_AUDIO_SAMPLES_PER_SECOND), 48_000);
    unsafe { release(native) };
    let a = drain(reader, READER_FIRST_AUDIO);
    assert!(a.samples > 0);
    assert_eq!(a.bytes, data_len as u64, "byte-stream reader must return every PCM byte");
    unsafe {
        release(reader);
        release(attrs);
        release(bs);
    }

    let resolver_new = export!(px("MFCreateSourceResolver"), fn(*mut *mut c_void) -> i32);
    let mut resolver = core::ptr::null_mut();
    hr_ok(unsafe { resolver_new(&mut resolver) }, "MFCreateSourceResolver");
    let bs = byte_stream(&path);
    let mut kind = 0u32;
    let mut unk: *mut c_void = core::ptr::null_mut();
    let url = wp(&path);
    hr_ok(
        call!(resolver, 4, fn(*mut c_void, *const u16, u32, *mut c_void, *mut u32, *mut *mut c_void) -> i32, bs, url.as_ptr(), 1, core::ptr::null_mut(), &mut kind, &mut unk),
        "IMFSourceResolver::CreateObjectFromByteStream",
    );
    let source = unsafe { query(unk, &IID_IMFMEDIASOURCE) }.expect("IMFMediaSource");
    let mut reader = core::ptr::null_mut();
    hr_ok(unsafe { from_src(source, core::ptr::null_mut(), &mut reader) }, "MFCreateSourceReaderFromMediaSource");
    let b = drain(reader, READER_FIRST_AUDIO);
    assert_eq!(b.bytes, data_len as u64, "media-source reader must return every PCM byte");
    unsafe {
        release(reader);
        release(source);
        release(unk);
        release(bs);
        release(resolver);
    }
}

#[repr(C)]
struct FakeXboxDevice {
    vtable: *const usize,
    real: *mut c_void,
    refs: AtomicU32,
}

unsafe extern "system" fn fx_qi(this: *mut FakeXboxDevice, riid: *const Guid, ppv: *mut *mut c_void) -> i32 {
    let g = unsafe { *riid };
    if g == IID_ID3D12DEVICE || g == IID_IUNKNOWN {
        unsafe {
            (*this).refs.fetch_add(1, Ordering::SeqCst);
            *ppv = this as *mut c_void;
        }
        return 0;
    }
    match unsafe { query((*this).real, &g) } {
        Ok(p) => {
            unsafe { *ppv = p };
            0
        }
        Err(hr) => {
            unsafe { *ppv = core::ptr::null_mut() };
            hr
        }
    }
}

unsafe extern "system" fn fx_addref(this: *mut FakeXboxDevice) -> u32 {
    unsafe { (*this).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn fx_release(this: *mut FakeXboxDevice) -> u32 {
    unsafe { (*this).refs.fetch_sub(1, Ordering::SeqCst) - 1 }
}

fn d3d12_device() -> Option<*mut c_void> {
    let h = unsafe { LoadLibraryW(w("d3d12.dll").as_ptr()) };
    if h.is_null() {
        return None;
    }
    let p = unsafe { GetProcAddress(h, b"D3D12CreateDevice\0".as_ptr()) };
    if p.is_null() {
        return None;
    }
    let create = export!(p, fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32);
    let mut dev = core::ptr::null_mut();
    let hr = unsafe { create(core::ptr::null_mut(), 0xb000, &IID_ID3D12DEVICE, &mut dev) };
    static HAD_DEVICE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if hr >= 0 && !dev.is_null() {
        HAD_DEVICE.store(true, Ordering::SeqCst);
        return Some(dev);
    }
    assert!(!HAD_DEVICE.load(Ordering::SeqCst), "D3D12CreateDevice failed hr={:#010x} after an earlier device existed in this process; the device was lost", hr as u32);
    None
}

#[repr(C)]
struct HeapProps {
    ty: u32,
    cpu_page: u32,
    pool: u32,
    creation_mask: u32,
    visible_mask: u32,
}

#[repr(C)]
struct ResourceDesc {
    dimension: u32,
    alignment: u64,
    width: u64,
    height: u32,
    depth: u16,
    mips: u16,
    format: u32,
    sample_count: u32,
    sample_quality: u32,
    layout: u32,
    flags: u32,
}

#[test]
fn dxgi_surface_buffer_x_wraps_a_d3d12_resource() {
    let _l = MF_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(dev) = d3d12_device() else {
        eprintln!("no D3D12 device; skipping");
        return;
    };
    let _mf = startup();
    let heap = HeapProps { ty: 1, cpu_page: 0, pool: 0, creation_mask: 1, visible_mask: 1 };
    let desc = ResourceDesc { dimension: 3, alignment: 0, width: 64, height: 64, depth: 1, mips: 1, format: 87, sample_count: 1, sample_quality: 0, layout: 0, flags: 0 };
    let mut tex: *mut c_void = core::ptr::null_mut();
    hr_ok(
        call!(dev, 27, fn(*const HeapProps, u32, *const ResourceDesc, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, &heap, 0, &desc, 0, core::ptr::null(), &IID_ID3D12RESOURCE, &mut tex),
        "CreateCommittedResource",
    );
    let mk = export!(px("MFCreateDXGISurfaceBufferX"), fn(*const Guid, *mut c_void, u32, i32, *mut *mut c_void) -> i32);
    let mut buf = core::ptr::null_mut();
    hr_ok(unsafe { mk(&IID_ID3D12RESOURCE, tex, 0, 0, &mut buf) }, "MFCreateDXGISurfaceBufferX(ID3D12Resource)");
    let dx = unsafe { query(buf, &IID_IMFDXGIBUFFER) }.expect("IMFDXGIBuffer");
    let mut back: *mut c_void = core::ptr::null_mut();
    hr_ok(call!(dx, 3, fn(*const Guid, *mut *mut c_void) -> i32, &IID_ID3D12RESOURCE, &mut back), "IMFDXGIBuffer::GetResource");
    assert_eq!(back, tex);
    let mut sub = 99u32;
    hr_ok(call!(dx, 4, fn(*mut u32) -> i32, &mut sub), "GetSubresourceIndex");
    assert_eq!(sub, 0);
    let mut bad = core::ptr::null_mut();
    assert!(unsafe { mk(&IID_ID3D12RESOURCE, tex, 0, 0, core::ptr::null_mut()) } < 0);
    assert!(unsafe { mk(&IID_ID3D12RESOURCE, core::ptr::null_mut(), 0, 0, &mut bad) } < 0);
    unsafe {
        release(back);
        release(dx);
        release(buf);
        release(tex);
        release(dev);
    }
}

fn write_h264_mp4(path: &Path, w_: u32, h_: u32, frames: u32) -> Result<(), String> {
    let mk = export!(rwx("MFCreateSinkWriterFromURL"), fn(*const u16, *mut c_void, *mut c_void, *mut *mut c_void) -> i32);
    let attrs = attributes(1);
    set_u32(attrs, &MF_SINK_WRITER_DISABLE_THROTTLING, 1);
    let mut writer = core::ptr::null_mut();
    let hr = unsafe { mk(wp(path).as_ptr(), core::ptr::null_mut(), attrs, &mut writer) };
    if hr < 0 {
        return Err(format!("MFCreateSinkWriterFromURL hr={:#010x}", hr as u32));
    }
    let size = ((w_ as u64) << 32) | h_ as u64;
    let out = media_type();
    set_guid(out, &MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO);
    set_guid(out, &MF_MT_SUBTYPE, &MF_VIDEOFORMAT_H264);
    set_u32(out, &MF_MT_AVG_BITRATE, 1_000_000);
    set_u32(out, &MF_MT_INTERLACE_MODE, 2);
    set_u64(out, &MF_MT_FRAME_SIZE, size);
    set_u64(out, &MF_MT_FRAME_RATE, (30 << 32) | 1);
    set_u64(out, &MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1);
    let mut idx = 0u32;
    let hr = call!(writer, 3, fn(*mut c_void, *mut u32) -> i32, out, &mut idx);
    if hr < 0 {
        return Err(format!("AddStream(H264) hr={:#010x}", hr as u32));
    }
    let inp = media_type();
    set_guid(inp, &MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO);
    set_guid(inp, &MF_MT_SUBTYPE, &MF_VIDEOFORMAT_RGB32);
    set_u32(inp, &MF_MT_INTERLACE_MODE, 2);
    set_u64(inp, &MF_MT_FRAME_SIZE, size);
    set_u64(inp, &MF_MT_FRAME_RATE, (30 << 32) | 1);
    set_u64(inp, &MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1);
    let hr = call!(writer, 4, fn(u32, *mut c_void, *mut c_void) -> i32, idx, inp, core::ptr::null_mut());
    if hr < 0 {
        return Err(format!("SetInputMediaType(RGB32) hr={:#010x}", hr as u32));
    }
    hr_ok(call!(writer, 5, fn() -> i32,), "BeginWriting");
    let len = w_ * h_ * 4;
    for i in 0..frames {
        let b = memory_buffer(len);
        let mut p: *mut u8 = core::ptr::null_mut();
        hr_ok(call!(b, 3, fn(*mut *mut u8, *mut u32, *mut u32) -> i32, &mut p, core::ptr::null_mut(), core::ptr::null_mut()), "Lock");
        let px_ = unsafe { core::slice::from_raw_parts_mut(p, len as usize) };
        for (j, c) in px_.chunks_mut(4).enumerate() {
            let x = (j as u32 % w_) as u8;
            c.copy_from_slice(&[x.wrapping_add(i as u8 * 8), (i * 16) as u8, 0x80, 0xff]);
        }
        call!(b, 4, fn() -> i32,);
        hr_ok(call!(b, 6, fn(u32) -> i32, len), "SetCurrentLength");
        let s = sample();
        hr_ok(call!(s, 42, fn(*mut c_void) -> i32, b), "AddBuffer");
        hr_ok(call!(s, 36, fn(i64) -> i32, i as i64 * 333_333), "SetSampleTime");
        hr_ok(call!(s, 38, fn(i64) -> i32, 333_333), "SetSampleDuration");
        hr_ok(call!(writer, 6, fn(u32, *mut c_void) -> i32, idx, s), "WriteSample");
        unsafe {
            release(s);
            release(b);
        }
    }
    hr_ok(call!(writer, 11, fn() -> i32,), "Finalize");
    unsafe {
        release(inp);
        release(out);
        release(writer);
        release(attrs);
    }
    Ok(())
}

#[test]
fn source_reader_decodes_video_with_d3d12_manager_reset_by_console_export() {
    let _l = MF_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(real) = d3d12_device() else {
        eprintln!("no D3D12 device; skipping");
        return;
    };
    let _mf = startup();
    let path = proxy().dir.join("movie.mp4");
    let frames = 20;
    if let Err(e) = write_h264_mp4(&path, 320, 240, frames) {
        eprintln!("no H.264 encoder on this machine ({e}); skipping video decode");
        unsafe { release(real) };
        return;
    }
    let vt_: [usize; 3] = [fx_qi as *const () as usize, fx_addref as *const () as usize, fx_release as *const () as usize];
    let mut xbox = FakeXboxDevice { vtable: vt_.as_ptr(), real, refs: AtomicU32::new(1) };
    let xp = &mut xbox as *mut FakeXboxDevice as *mut c_void;

    let mk_mgr = export!(px("MFCreateDXGIDeviceManager"), fn(*mut u32, *mut *mut c_void) -> i32);
    let reset_x = export!(px("MFResetDXGIDeviceManagerX"), fn(*mut c_void, *mut c_void, u32) -> i32);
    let mut token = 0u32;
    let mut mgr = core::ptr::null_mut();
    hr_ok(unsafe { mk_mgr(&mut token, &mut mgr) }, "MFCreateDXGIDeviceManager");
    hr_ok(unsafe { reset_x(mgr, xp, token) }, "MFResetDXGIDeviceManagerX");
    assert_eq!(xbox.refs.load(Ordering::SeqCst), 1, "proxy must not leak the console device");

    let from_bs = export!(rwx("MFCreateSourceReaderFromByteStream"), fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32);
    let mut results = Vec::new();
    for (label, with_mgr, subtype) in [("software NV12", false, MF_VIDEOFORMAT_NV12), ("D3D12 manager NV12", true, MF_VIDEOFORMAT_NV12), ("D3D12 manager RGB32", true, MF_VIDEOFORMAT_RGB32)] {
        let attrs = attributes(2);
        if with_mgr {
            set_unknown(attrs, &MF_SOURCE_READER_D3D_MANAGER, mgr);
        }
        set_u32(attrs, &MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1);
        let bs = byte_stream(&path);
        let mut reader = core::ptr::null_mut();
        hr_ok(unsafe { from_bs(bs, attrs, &mut reader) }, &format!("MFCreateSourceReaderFromByteStream ({label})"));
        let want = media_type();
        set_guid(want, &MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO);
        set_guid(want, &MF_MT_SUBTYPE, &subtype);
        hr_ok(call!(reader, 7, fn(u32, *mut u32, *mut c_void) -> i32, READER_FIRST_VIDEO, core::ptr::null_mut(), want), &format!("SetCurrentMediaType ({label})"));
        let st = drain_on(reader, READER_FIRST_VIDEO, real);
        eprintln!("{label}: {} frames, {} bytes, {} D3D-backed, {} ID3D12Resource on the real device", st.samples, st.bytes, st.dxgi_samples, st.d3d12_on_device);
        assert_eq!(st.samples, frames, "{label}: every encoded frame must decode");
        if st.dxgi_samples == 0 {
            assert!(st.bytes >= frames as u64 * 320 * 240, "{label}: system-memory frames must carry pixels");
        } else {
            assert_eq!(st.d3d12_on_device, st.dxgi_samples, "{label}: GPU frames must be ID3D12Resources of the device given to MFResetDXGIDeviceManagerX");
        }
        results.push((label, st.dxgi_samples));
        unsafe {
            release(want);
            release(reader);
            release(bs);
            release(attrs);
        }
    }
    unsafe {
        release(mgr);
        release(real);
    }
    let log = std::fs::read_to_string(proxy().dir.join("tombox_compat.log")).unwrap_or_default();
    assert!(log.contains("MFResetDXGIDeviceManagerX") && log.contains("ResetDevice hr=0x00000000"), "reset must be logged");
    eprintln!("D3D-backed samples per mode: {results:?}");
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn png_rgb(path: &Path, w: u32, h: u32, rgb: &[u8]) {
    let mut raw = Vec::with_capacity((w as usize * 3 + 1) * h as usize);
    for y in 0..h as usize {
        raw.push(0);
        raw.extend_from_slice(&rgb[y * w as usize * 3..(y + 1) * w as usize * 3]);
    }
    let mut z = vec![0x78, 0x01];
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    let chunks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, c) in chunks.iter().enumerate() {
        z.push(if i + 1 == chunks.len() { 1 } else { 0 });
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |ty: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = ty.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    std::fs::write(path, out).expect("write png");
}

fn get_u64(a: *mut c_void, k: &Guid) -> u64 {
    let mut v = 0u64;
    hr_ok(call!(a, 8, fn(*const Guid, *mut u64) -> i32, k, &mut v), "GetUINT64");
    v
}

struct MovieStats {
    frames: u32,
    saved: Vec<(u32, f64, f64)>,
    monotonic: bool,
    width: u32,
    height: u32,
    d3d12_frames: u32,
    last_ts: i64,
}

fn decode_movie(path: &Path, mgr: *mut c_void, device: *mut c_void, out_dir: &Path, max_frames: u32, every: u32) -> MovieStats {
    decode_movie_as(path, mgr, device, out_dir, max_frames, every, &MF_VIDEOFORMAT_RGB32)
}

fn decode_movie_as(path: &Path, mgr: *mut c_void, device: *mut c_void, out_dir: &Path, max_frames: u32, every: u32, subtype: &Guid) -> MovieStats {
    let nv12 = *subtype == MF_VIDEOFORMAT_NV12;
    let resolver_new = export!(px("MFCreateSourceResolver"), fn(*mut *mut c_void) -> i32);
    let from_src = export!(rwx("MFCreateSourceReaderFromMediaSource"), fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32);
    let mut resolver = core::ptr::null_mut();
    hr_ok(unsafe { resolver_new(&mut resolver) }, "MFCreateSourceResolver");
    let bs = byte_stream(path);
    let mut kind = 0u32;
    let mut unk: *mut c_void = core::ptr::null_mut();
    let url = wp(path);
    hr_ok(
        call!(resolver, 4, fn(*mut c_void, *const u16, u32, *mut c_void, *mut u32, *mut *mut c_void) -> i32, bs, url.as_ptr(), 1, core::ptr::null_mut(), &mut kind, &mut unk),
        "IMFSourceResolver::CreateObjectFromByteStream",
    );
    let source = unsafe { query(unk, &IID_IMFMEDIASOURCE) }.expect("IMFMediaSource");
    let attrs = attributes(2);
    if !mgr.is_null() {
        set_unknown(attrs, &MF_SOURCE_READER_D3D_MANAGER, mgr);
    }
    set_u32(attrs, &MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1);
    let mut reader = core::ptr::null_mut();
    hr_ok(unsafe { from_src(source, attrs, &mut reader) }, "MFCreateSourceReaderFromMediaSource");
    let mut native = core::ptr::null_mut();
    hr_ok(call!(reader, 5, fn(u32, u32, *mut *mut c_void) -> i32, READER_FIRST_VIDEO, 0, &mut native), "GetNativeMediaType");
    let native_sub = get_guid(native, &MF_MT_SUBTYPE);
    assert_eq!(native_sub, MF_VIDEOFORMAT_H264, "{}: video track is not H.264", path.display());
    unsafe { release(native) };
    let want = media_type();
    set_guid(want, &MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO);
    set_guid(want, &MF_MT_SUBTYPE, subtype);
    hr_ok(call!(reader, 7, fn(u32, *mut u32, *mut c_void) -> i32, READER_FIRST_VIDEO, core::ptr::null_mut(), want), "SetCurrentMediaType");
    let mut cur = core::ptr::null_mut();
    hr_ok(call!(reader, 6, fn(u32, *mut *mut c_void) -> i32, READER_FIRST_VIDEO, &mut cur), "GetCurrentMediaType");
    let fs = get_u64(cur, &MF_MT_FRAME_SIZE);
    unsafe { release(cur) };
    let (w, h) = ((fs >> 32) as u32, fs as u32);
    let stem = path.file_stem().unwrap().to_string_lossy().replace(' ', "_");
    let mut st = MovieStats { frames: 0, saved: Vec::new(), monotonic: true, width: w, height: h, d3d12_frames: 0, last_ts: -1 };
    while st.frames < max_frames {
        let mut actual = 0u32;
        let mut flags = 0u32;
        let mut ts = 0i64;
        let mut s: *mut c_void = core::ptr::null_mut();
        hr_ok(call!(reader, 9, fn(u32, u32, *mut u32, *mut u32, *mut i64, *mut *mut c_void) -> i32, READER_FIRST_VIDEO, 0, &mut actual, &mut flags, &mut ts, &mut s), "ReadSample");
        assert_eq!(flags & READERF_ERROR, 0, "reader error flag");
        if !s.is_null() {
            if ts <= st.last_ts {
                st.monotonic = false;
            }
            st.last_ts = ts;
            let mut b0: *mut c_void = core::ptr::null_mut();
            if call!(s, 40, fn(u32, *mut *mut c_void) -> i32, 0, &mut b0) >= 0 && !b0.is_null() {
                if let Ok(d) = unsafe { query(b0, &IID_IMFDXGIBUFFER) } {
                    if !device.is_null() && d3d12_resource_on(d, device) {
                        st.d3d12_frames += 1;
                    }
                    unsafe { release(d) };
                }
                unsafe { release(b0) };
            }
            if st.frames % every == 0 {
                let mut buf: *mut c_void = core::ptr::null_mut();
                hr_ok(call!(s, 41, fn(*mut *mut c_void) -> i32, &mut buf), "ConvertToContiguousBuffer");
                let mut p: *mut u8 = core::ptr::null_mut();
                let (mut max, mut len) = (0u32, 0u32);
                hr_ok(call!(buf, 3, fn(*mut *mut u8, *mut u32, *mut u32) -> i32, &mut p, &mut max, &mut len), "IMFMediaBuffer::Lock");
                let need = if nv12 { w as u64 * h as u64 * 3 / 2 } else { w as u64 * h as u64 * 4 };
                assert!(len as u64 >= need, "{}: frame {} has {len} bytes for {w}x{h}", path.display(), st.frames);
                let mut rgb = Vec::with_capacity((w * h * 3) as usize);
                let (mut sum, mut sq) = (0f64, 0f64);
                if nv12 {
                    let px_ = unsafe { core::slice::from_raw_parts(p, (w * h) as usize) };
                    for &y8 in px_ {
                        rgb.extend_from_slice(&[y8, y8, y8]);
                        sum += y8 as f64;
                        sq += y8 as f64 * y8 as f64;
                    }
                } else {
                    let px_ = unsafe { core::slice::from_raw_parts(p, (w * h * 4) as usize) };
                    for q in px_.chunks(4) {
                        rgb.extend_from_slice(&[q[2], q[1], q[0]]);
                        let y = 0.299 * q[2] as f64 + 0.587 * q[1] as f64 + 0.114 * q[0] as f64;
                        sum += y;
                        sq += y * y;
                    }
                }
                let unlock: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(vt(buf, 4)) };
                unsafe { unlock(buf) };
                unsafe { release(buf) };
                let n = (w * h) as f64;
                let mean = sum / n;
                let var = sq / n - mean * mean;
                png_rgb(&out_dir.join(format!("{stem}_{:05}_{}ms.png", st.frames, ts / 10_000)), w, h, &rgb);
                st.saved.push((st.frames, mean, var));
            }
            st.frames += 1;
            unsafe { release(s) };
        }
        if flags & READERF_ENDOFSTREAM != 0 {
            break;
        }
    }
    unsafe {
        release(want);
        release(reader);
        release(attrs);
        release(source);
        release(unk);
        release(bs);
        release(resolver);
    }
    st
}

fn movie_names() -> Vec<String> {
    std::env::var("TOMBOX_MF_MOVIES").map(|v| v.split(';').map(String::from).collect()).unwrap_or_else(|_| vec!["SatelliteConnection.mp4".into(), "Onboarding_Appended_Intro.mp4".into(), "WargamesVictory.mp4".into()])
}

fn fortnite_movies() -> Vec<PathBuf> {
    let dir = PathBuf::from(std::env::var("TOMBOX_FORTNITE_MOVIES").unwrap_or_else(|_| r"D:\xbox-pc-port\games\Fortnite\extracted\Mount\FortniteGame\Content\Movies".into()));
    movie_names().iter().map(|n| dir.join(n)).filter(|p| p.is_file()).collect()
}

fn decode_movies_nonblack(movies: &[PathBuf], mode: &str) {
    let _l = MF_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if movies.is_empty() {
        eprintln!("no movies found; skipping");
        return;
    }
    let Some(real) = d3d12_device() else {
        eprintln!("no D3D12 device; skipping");
        return;
    };
    let _mf = startup();
    let out_dir = std::env::var("TOMBOX_MF_PNG_DIR").map(PathBuf::from).unwrap_or_else(|_| proxy().dir.join("frames"));
    let vt_: [usize; 3] = [fx_qi as *const () as usize, fx_addref as *const () as usize, fx_release as *const () as usize];
    let mut xbox = FakeXboxDevice { vtable: vt_.as_ptr(), real, refs: AtomicU32::new(1) };
    let xp = &mut xbox as *mut FakeXboxDevice as *mut c_void;
    let mk_mgr = export!(px("MFCreateDXGIDeviceManager"), fn(*mut u32, *mut *mut c_void) -> i32);
    let reset_x = export!(px("MFResetDXGIDeviceManagerX"), fn(*mut c_void, *mut c_void, u32) -> i32);
    let mut token = 0u32;
    let mut mgr = core::ptr::null_mut();
    hr_ok(unsafe { mk_mgr(&mut token, &mut mgr) }, "MFCreateDXGIDeviceManager");
    hr_ok(unsafe { reset_x(mgr, xp, token) }, "MFResetDXGIDeviceManagerX");
    let max_frames: u32 = std::env::var("TOMBOX_MF_MAX_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    let sub = out_dir.join(mode);
    std::fs::create_dir_all(&sub).unwrap();
    let mut black = Vec::new();
    for m in movies {
        let fmt = if mode == "d3d12-nv12" { MF_VIDEOFORMAT_NV12 } else { MF_VIDEOFORMAT_RGB32 };
        let st = decode_movie_as(m, if mode == "software" { core::ptr::null_mut() } else { mgr }, real, &sub, max_frames, 30, &fmt);
        let best = st.saved.iter().cloned().fold((0, 0f64, 0f64), |a, b| if b.2 > a.2 { b } else { a });
        eprintln!(
            "[{mode}] {}: {}x{}, {} frames decoded (last pts {} ms, {} as ID3D12Resource on the console device), saved {:?}, best frame {} mean {:.1} variance {:.1}",
            m.display(), st.width, st.height, st.frames, st.last_ts / 10_000, st.d3d12_frames, st.saved.iter().map(|s| s.0).collect::<Vec<_>>(), best.0, best.1, best.2
        );
        assert!(st.frames > 0, "{}: no frames decoded", m.display());
        assert!(st.monotonic, "{}: sample times must increase", m.display());
        assert!(st.width > 0 && st.height > 0);
        if mode != "software" {
            assert_eq!(st.d3d12_frames, st.frames, "{}: every {mode} frame must be an ID3D12Resource on the console device", m.display());
        }
        if !(best.1 > 8.0 && best.2 > 50.0) {
            black.push(format!("[{mode}] {}: every saved frame is black or flat (best mean {:.1}, variance {:.1})", m.display(), best.1, best.2));
        }
    }
    eprintln!("frames written to {}", sub.display());
    unsafe {
        release(mgr);
        release(real);
    }
    assert!(black.is_empty(), "{black:#?}");
}

#[test]
fn fortnite_movies_decode_to_nonblack_frames_in_software() {
    decode_movies_nonblack(&fortnite_movies(), "software");
}

#[test]
#[ignore = "host MF: a second D3D12 source reader on one device intermittently hits DXGI_ERROR_DEVICE_HUNG, also with plain ResetDevice; see BLOCKERS"]
fn fortnite_movies_decode_to_nonblack_nv12_on_console_d3d12_manager() {
    decode_movies_nonblack(&fortnite_movies(), "d3d12-nv12");
}

#[test]
#[ignore = "host MF: D3D12 source reader RGB32 output is all zero at 1920x1080 (640x360 works); Fortnite uses NV12; see BLOCKERS"]
fn fortnite_movies_decode_to_nonblack_rgb32_on_console_d3d12_manager() {
    decode_movies_nonblack(&fortnite_movies(), "d3d12");
}

#[test]
fn lobby_video_placeholder_decodes_to_nonblack_frames_on_every_path() {
    let clip = media::staged_fragment("init.mp4", "seg0.m4s").unwrap_or_else(media::patterned_h264);
    let dir = std::env::temp_dir().join(format!("tombox_placeholder_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lobby_placeholder.mp4");
    std::fs::write(&path, &clip).unwrap();
    for mode in ["software", "d3d12-nv12", "d3d12"] {
        decode_movies_nonblack(std::slice::from_ref(&path), mode);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(key: isize, subkey: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
}

fn host_assign_queues() -> u32 {
    let key: Vec<u16> = concat!(r"SOFTWARE\Microsoft\Windows Media Foundation\RT", "\0").encode_utf16().collect();
    let value: Vec<u16> = "AssignQueues\0".encode_utf16().collect();
    let mut v = 0u32;
    let mut len = 4u32;
    let r = unsafe { RegGetValueW(0x8000_0002u32 as i32 as isize, key.as_ptr(), value.as_ptr(), 0x10, core::ptr::null_mut(), (&mut v as *mut u32).cast(), &mut len) };
    (r == 0 && v != 0) as u32
}
