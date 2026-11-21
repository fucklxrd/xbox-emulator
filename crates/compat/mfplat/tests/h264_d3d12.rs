use core::ffi::c_void;
use core::mem::ManuallyDrop;
#[path = "../src/reset.rs"]
#[allow(dead_code)]
mod reset;
#[path = "../../d3d12_xs/src/video_probe.rs"]
mod video_probe;
mod device_wrap {
    #[repr(C)]
    pub struct Guid {
        pub d1: u32,
        pub d2: u16,
        pub d3: u16,
        pub d4: [u8; 8],
    }
}
use windows::core::{Interface, GUID};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_NV12;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

#[path = "common/media.rs"]
mod media;
const CLSID_MSH264_DECODER: GUID = GUID::from_u128(0x62ce7e72_4c71_4d20_b15d_452831a87d9d);
const MF_SA_D3D12_AWARE: GUID = GUID::from_u128(0x77e4a1b1_66bb_4ba4_9cd6_b1c1fbc6b5ee);
const MF_VERSION_FULL: u32 = 0x0002_0070;
const MF_E_NOTACCEPTING: i32 = 0xC00D_36B5u32 as i32;
const MF_E_TRANSFORM_NEED_MORE_INPUT: i32 = 0xC00D_6D72u32 as i32;
const MF_E_TRANSFORM_STREAM_CHANGE: i32 = 0xC00D_6D61u32 as i32;

struct Report {
    frames: usize,
    d3d12_resources: usize,
    sync_objects: usize,
    provides_samples: bool,
    format: Option<i32>,
    presented: usize,
    black: usize,
}

struct Presenter {
    dev: ID3D12Device,
    queue: ID3D12CommandQueue,
    alloc: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    fence: ID3D12Fence,
    value: u64,
    readback: Option<(ID3D12Resource, [D3D12_PLACED_SUBRESOURCE_FOOTPRINT; 2], [u32; 2])>,
    probe: Option<(usize, ID3D12Resource)>,
}

type ProbeCopy = unsafe extern "system" fn(*mut c_void, *const D3D12_TEXTURE_COPY_LOCATION, u32, u32, u32, *const D3D12_TEXTURE_COPY_LOCATION, *const D3D12_BOX);

fn transition(res: &ID3D12Resource, sub: u32, before: D3D12_RESOURCE_STATES, after: D3D12_RESOURCE_STATES) -> D3D12_RESOURCE_BARRIER {
    D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: unsafe { core::mem::transmute_copy(res) },
                Subresource: sub,
                StateBefore: before,
                StateAfter: after,
            }),
        },
    }
}

impl Presenter {
    fn new(dev: &ID3D12Device, queue: &ID3D12CommandQueue) -> Self {
        unsafe {
            let alloc: ID3D12CommandAllocator = dev.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT).unwrap();
            let list: ID3D12GraphicsCommandList = dev.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &alloc, None).unwrap();
            list.Close().unwrap();
            let fence: ID3D12Fence = dev.CreateFence(0, D3D12_FENCE_FLAG_NONE).unwrap();
            Presenter { dev: dev.clone(), queue: queue.clone(), alloc, list, fence, value: 0, readback: None, probe: None }
        }
    }

    fn with_probe(mut self, w: u32, h: u32) -> Self {
        let heap = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() };
        let td = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: w as u64,
            Height: h,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            ..Default::default()
        };
        let mut t: Option<ID3D12Resource> = None;
        unsafe { self.dev.CreateCommittedResource(&heap, D3D12_HEAP_FLAG_NONE, &td, D3D12_RESOURCE_STATE_COMMON, None, &mut t) }.unwrap();
        let real16 = unsafe { *(*(self.list.as_raw() as *const *const usize)).add(16) };
        self.probe = Some((video_probe::install(real16), t.unwrap()));
        self
    }

    fn plane_subresource(desc: &D3D12_RESOURCE_DESC, sub: u32, plane: u32) -> u32 {
        sub + plane * desc.MipLevels as u32 * desc.DepthOrArraySize as u32
    }

    fn ensure_readback(&mut self, desc: &D3D12_RESOURCE_DESC, sub: u32) {
        if self.readback.is_some() {
            return;
        }
        let mut fp = [D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); 2];
        let mut rows = [0u32; 2];
        let mut base = 0u64;
        for plane in 0..2u32 {
            let mut total = 0u64;
            let mut nrows = 0u32;
            unsafe {
                self.dev.GetCopyableFootprints(desc, Self::plane_subresource(desc, sub, plane), 1, base, Some(&mut fp[plane as usize]), Some(&mut nrows), None, Some(&mut total));
            }
            rows[plane as usize] = nrows;
            base = (base + total + 511) & !511;
        }
        let heap = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_READBACK, ..Default::default() };
        let bd = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Width: base,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            ..Default::default()
        };
        let mut rb: Option<ID3D12Resource> = None;
        unsafe { self.dev.CreateCommittedResource(&heap, D3D12_HEAP_FLAG_NONE, &bd, D3D12_RESOURCE_STATE_COPY_DEST, None, &mut rb) }.unwrap();
        self.readback = Some((rb.unwrap(), fp, rows));
    }

    fn present(&mut self, res: &ID3D12Resource, sub: u32, sync: &IMFD3D12SynchronizationObjectCommands) -> bool {
        let desc = unsafe { res.GetDesc() };
        self.ensure_readback(&desc, sub);
        let (rb, fp, rows) = self.readback.clone().unwrap();
        unsafe {
            self.alloc.Reset().unwrap();
            self.list.Reset(&self.alloc, None).unwrap();
            let subs = [Self::plane_subresource(&desc, sub, 0), Self::plane_subresource(&desc, sub, 1)];
            let to_src: Vec<_> = subs.iter().map(|&s| transition(res, s, D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATE_COPY_SOURCE)).collect();
            self.list.ResourceBarrier(&to_src);
            for plane in 0..2 {
                let dst = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: core::mem::transmute_copy(&rb),
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: fp[plane] },
                };
                let src = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: core::mem::transmute_copy(res),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: subs[plane] },
                };
                self.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
                if let Some((hook, target)) = &self.probe {
                    let hook: ProbeCopy = core::mem::transmute(*hook);
                    let pdst = D3D12_TEXTURE_COPY_LOCATION {
                        pResource: core::mem::transmute_copy(target),
                        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: plane as u32 },
                    };
                    let bx = D3D12_BOX { left: 0, top: 0, front: 0, right: 640 >> plane, bottom: 360 >> plane, back: 1 };
                    hook(self.list.as_raw(), &pdst, 0, 0, 0, &src, &bx);
                }
            }
            let back: Vec<_> = subs.iter().map(|&s| transition(res, s, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_COMMON)).collect();
            self.list.ResourceBarrier(&back);
            self.list.Close().unwrap();
            sync.EnqueueResourceReadyWait(&self.queue).expect("EnqueueResourceReadyWait");
            self.queue.ExecuteCommandLists(&[Some(self.list.cast::<ID3D12CommandList>().unwrap())]);
            self.value += 1;
            self.queue.Signal(&self.fence, self.value).unwrap();
            sync.EnqueueResourceRelease(&self.queue).expect("EnqueueResourceRelease");
            self.fence.SetEventOnCompletion(self.value, windows::Win32::Foundation::HANDLE::default()).unwrap();
            let mut p: *mut c_void = core::ptr::null_mut();
            rb.Map(0, None, Some(&mut p)).unwrap();
            let base = p as *const u8;
            let sample = |plane: usize, x: u32, y: u32, bpp: u32| -> u8 {
                let f = fp[plane];
                let y = y.min(rows[plane].saturating_sub(1));
                *base.add(f.Offset as usize + (y * f.Footprint.RowPitch + x * bpp) as usize)
            };
            let w = desc.Width as u32;
            let h = desc.Height;
            let mut ok = true;
            for &(x, y) in &[(w / 2, h / 2), (8, 8), (w.saturating_sub(16).min(624), h.saturating_sub(16).min(344))] {
                let luma = sample(0, x, y, 1);
                let cb = sample(1, x / 2, y / 2, 2);
                let cr = *base.add(fp[1].Offset as usize + ((y / 2).min(rows[1].saturating_sub(1)) * fp[1].Footprint.RowPitch + (x / 2) * 2 + 1) as usize);
                if !(14..=18).contains(&luma) || !(126..=130).contains(&cb) || !(126..=130).contains(&cr) {
                    eprintln!("pixel ({x},{y}) Y={luma} Cb={cb} Cr={cr}");
                    ok = false;
                }
            }
            rb.Unmap(0, None);
            ok
        }
    }
}

fn init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION_FULL, MFSTARTUP_FULL).expect("MFStartup");
    }
}

fn real_device() -> Option<ID3D12Device> {
    let mut dev: Option<ID3D12Device> = None;
    unsafe { D3D12CreateDevice(None, D3D_FEATURE_LEVEL_11_0, &mut dev) }.ok()?;
    dev
}

fn set_output_nv12(t: &IMFTransform) {
    for i in 0.. {
        let ty = unsafe { t.GetOutputAvailableType(0, i) }.expect("output type with NV12");
        if unsafe { ty.GetGUID(&MF_MT_SUBTYPE) }.ok() == Some(MFVideoFormat_NV12) {
            unsafe { t.SetOutputType(0, &ty, 0) }.expect("SetOutputType NV12");
            return;
        }
    }
}

fn configure(t: &IMFTransform) {
    let mt = unsafe { MFCreateMediaType() }.unwrap();
    unsafe {
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).unwrap();
        mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).unwrap();
        mt.SetUINT64(&MF_MT_FRAME_SIZE, (640u64 << 32) | 360).unwrap();
        mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).unwrap();
        t.SetInputType(0, &mt, 0).expect("SetInputType H264");
    }
    set_output_nv12(t);
    unsafe {
        t.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0).unwrap();
        t.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0).unwrap();
    }
}

fn input_sample() -> IMFSample {
    let stream = media::black_h264();
    unsafe {
        let buf = MFCreateMemoryBuffer(stream.len() as u32).unwrap();
        let mut p: *mut u8 = core::ptr::null_mut();
        buf.Lock(&mut p, None, None).unwrap();
        core::ptr::copy_nonoverlapping(stream.as_ptr(), p, stream.len());
        buf.Unlock().unwrap();
        buf.SetCurrentLength(stream.len() as u32).unwrap();
        let s = MFCreateSample().unwrap();
        s.AddBuffer(&buf).unwrap();
        s.SetSampleTime(0).unwrap();
        s.SetSampleDuration(10_000_000).unwrap();
        s
    }
}

fn inspect(sample: &IMFSample, presenter: &mut Option<Presenter>, r: &mut Report) {
    let queue = presenter.as_ref().map(|p| p.queue.clone());
    let queue = queue.as_ref();
    r.frames += 1;
    let buf = unsafe { sample.GetBufferByIndex(0) }.unwrap();
    let Ok(dxgi) = buf.cast::<IMFDXGIBuffer>() else { return };
    let mut res: *mut c_void = core::ptr::null_mut();
    if unsafe { dxgi.GetResource(&ID3D12Resource::IID, &mut res) }.is_ok() && !res.is_null() {
        let res = unsafe { ID3D12Resource::from_raw(res) };
        let desc = unsafe { res.GetDesc() };
        r.format = Some(desc.Format.0);
        r.d3d12_resources += 1;
        let _ = unsafe { dxgi.GetSubresourceIndex() }.unwrap();
        let mut sync: *mut c_void = core::ptr::null_mut();
        if unsafe { dxgi.GetUnknown(&MF_D3D12_SYNCHRONIZATION_OBJECT, &IMFD3D12SynchronizationObjectCommands::IID, &mut sync) }.is_ok() && !sync.is_null() {
            let sync = unsafe { IMFD3D12SynchronizationObjectCommands::from_raw(sync) };
            r.sync_objects += 1;
            if let (Some(_), Some(p)) = (queue, presenter.as_mut()) {
                let sub = unsafe { dxgi.GetSubresourceIndex() }.unwrap();
                r.presented += 1;
                if p.present(&res, sub, &sync) {
                    r.black += 1;
                }
            }
        }
    }
}

fn decode(t: &IMFTransform, mut presenter: Option<Presenter>) -> Report {
    let info = unsafe { t.GetOutputStreamInfo(0) }.unwrap();
    let provides = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32) != 0;
    let mut r = Report { frames: 0, d3d12_resources: 0, sync_objects: 0, provides_samples: provides, format: None, presented: 0, black: 0 };
    let s = input_sample();
    let mut fed = false;
    let mut drained = false;
    for _ in 0..400 {
        if !fed {
            match unsafe { t.ProcessInput(0, &s, 0) } {
                Ok(()) => fed = true,
                Err(e) if e.code().0 == MF_E_NOTACCEPTING => {}
                Err(e) => panic!("ProcessInput {e:?}"),
            }
        }
        let info = unsafe { t.GetOutputStreamInfo(0) }.unwrap();
        let provides = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32) != 0;
        let pre = if provides {
            None
        } else {
            unsafe {
                let b = MFCreateAlignedMemoryBuffer(info.cbSize.max(640 * 368 * 3 / 2), info.cbAlignment.saturating_sub(1)).unwrap();
                let smp = MFCreateSample().unwrap();
                smp.AddBuffer(&b).unwrap();
                Some(smp)
            }
        };
        let mut out = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(pre), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
        let mut status = 0u32;
        let hr = unsafe { t.ProcessOutput(0, &mut out, &mut status) };
        let sample = unsafe { ManuallyDrop::take(&mut out[0].pSample) };
        let _ = unsafe { ManuallyDrop::take(&mut out[0].pEvents) };
        match hr {
            Ok(()) => {
                if let Some(sm) = sample {
                    inspect(&sm, &mut presenter, &mut r);
                }
            }
            Err(e) if e.code().0 == MF_E_TRANSFORM_STREAM_CHANGE => set_output_nv12(t),
            Err(e) if e.code().0 == MF_E_TRANSFORM_NEED_MORE_INPUT => {
                if fed && !drained {
                    unsafe {
                        t.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0).unwrap();
                        t.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0).unwrap();
                    }
                    drained = true;
                } else if drained {
                    break;
                }
            }
            Err(e) => panic!("ProcessOutput {e:?}"),
        }
    }
    r
}

fn avgs(line: &str) -> Vec<u64> {
    line.split("avg=").skip(1).filter_map(|s| s.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()).collect()
}

#[test]
fn electra_dma_copy_probe_reads_back_decoded_frame() {
    init();
    let Some(dev) = real_device() else {
        eprintln!("no D3D12 device; skipping");
        return;
    };
    let mut token = 0u32;
    let mut mgr: Option<IMFDXGIDeviceManager> = None;
    unsafe { MFCreateDXGIDeviceManager(&mut token, &mut mgr) }.unwrap();
    let mgr = mgr.unwrap();
    assert_eq!(unsafe { reset::reset_device_manager(mgr.as_raw(), dev.as_raw(), token) }.hr, 0);
    let queue: ID3D12CommandQueue = unsafe {
        dev.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC { Type: D3D12_COMMAND_LIST_TYPE_DIRECT, ..Default::default() })
    }
    .unwrap();
    let hw: IMFTransform = unsafe { CoCreateInstance(&CLSID_MSH264_DECODER, None, CLSCTX_INPROC_SERVER) }.unwrap();
    unsafe { hw.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, mgr.as_raw() as usize) }.unwrap();
    configure(&hw);
    let r = decode(&hw, Some(Presenter::new(&dev, &queue).with_probe(640, 360)));
    assert_eq!(r.presented, r.frames);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut res = video_probe::results();
    while res.len() < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
        res = video_probe::results();
    }
    for l in &res {
        eprintln!("probe: {l}");
    }
    assert_eq!(res.len(), 2, "probe must report both NV12 plane readbacks");
    let y = avgs(&res[0]);
    let uv = avgs(&res[1]);
    assert_eq!(y.len(), 1, "{}", res[0]);
    assert_eq!(uv.len(), 2, "{}", res[1]);
    assert!((15..=17).contains(&y[0]), "Y plane of Electra destination: {}", res[0]);
    assert!(uv.iter().all(|v| (127..=129).contains(v)), "CbCr plane of Electra destination: {}", res[1]);
}

#[test]
fn h264_decoder_hardware_d3d12_and_software_fallback() {
    init();
    let Some(dev) = real_device() else {
        eprintln!("no D3D12 device; skipping");
        return;
    };
    let mut token = 0u32;
    let mut mgr: Option<IMFDXGIDeviceManager> = None;
    unsafe { MFCreateDXGIDeviceManager(&mut token, &mut mgr) }.unwrap();
    let mgr = mgr.unwrap();
    let out = unsafe { reset::reset_device_manager(mgr.as_raw(), dev.as_raw(), token) };
    assert_eq!(out.hr, 0);
    let queue: ID3D12CommandQueue = unsafe {
        dev.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC { Type: D3D12_COMMAND_LIST_TYPE_DIRECT, ..Default::default() })
    }
    .unwrap();

    let hw: IMFTransform = unsafe { CoCreateInstance(&CLSID_MSH264_DECODER, None, CLSCTX_INPROC_SERVER) }.expect("CMSH264DecoderMFT");
    let attrs = unsafe { hw.GetAttributes() }.unwrap();
    let d3d12_aware = unsafe { attrs.GetUINT32(&MF_SA_D3D12_AWARE) }.unwrap_or(0);
    unsafe {
        attrs.SetUINT32(&CODECAPI_AVLowLatencyMode, 1).unwrap();
        attrs.SetUINT32(&MF_SA_MINIMUM_OUTPUT_SAMPLE_COUNT, 5).unwrap();
    }
    let set = unsafe { hw.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, mgr.as_raw() as usize) };
    eprintln!("hw: MF_SA_D3D12_AWARE={d3d12_aware} SET_D3D_MANAGER(D3D12 manager) -> {:?}", set.as_ref().err().map(|e| e.code()));
    assert!(set.is_ok(), "MS H.264 MFT rejected the D3D12 device manager: {set:?}");
    configure(&hw);
    let r = decode(&hw, Some(Presenter::new(&dev, &queue)));
    eprintln!(
        "hw: frames={} d3d12_resources={} sync_objects={} provides_samples={} format={:?} presented={} black={}",
        r.frames, r.d3d12_resources, r.sync_objects, r.provides_samples, r.format, r.presented, r.black
    );
    assert_eq!(r.presented, r.frames);
    assert_eq!(r.black, r.frames, "decoded NV12 frames copied through the queue must read back as black (Y=16, CbCr=128)");
    assert!(r.frames >= 1);
    assert!(r.provides_samples);
    assert_eq!(r.d3d12_resources, r.frames);
    assert_eq!(r.sync_objects, r.frames);
    assert_eq!(r.format, Some(DXGI_FORMAT_NV12.0));

    let sw: IMFTransform = unsafe { CoCreateInstance(&CLSID_MSH264_DECODER, None, CLSCTX_INPROC_SERVER) }.unwrap();
    unsafe { sw.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, mgr.as_raw() as usize) }.unwrap();
    unsafe { sw.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, 0) }.expect("switch to software (SET_D3D_MANAGER 0)");
    configure(&sw);
    let r = decode(&sw, None);
    eprintln!("sw: frames={} d3d12_resources={} provides_samples={}", r.frames, r.d3d12_resources, r.provides_samples);
    assert!(r.frames >= 1);
    assert!(!r.provides_samples);
    assert_eq!(r.d3d12_resources, 0);
}
