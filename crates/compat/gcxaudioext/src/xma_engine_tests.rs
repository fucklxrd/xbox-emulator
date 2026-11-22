use super::acp_state::*;
use super::acp_tests::{load_audio_dlls, serial};
use super::xma_engine::*;
use super::*;
use std::sync::Mutex;

static CALLS: Mutex<Vec<(usize, u64, u32)>> = Mutex::new(Vec::new());

unsafe extern "system" fn cb0(_t: *mut c_void, ctx: u64, v: u32) {
    CALLS.lock().unwrap().push((0, ctx, v));
}
unsafe extern "system" fn cb1(_t: *mut c_void, ctx: u64, v: u32) {
    CALLS.lock().unwrap().push((1, ctx, v));
}
unsafe extern "system" fn cb2(_t: *mut c_void, ctx: u64, v: u32) {
    CALLS.lock().unwrap().push((2, ctx, v));
}

#[repr(C)]
struct Callback {
    _vtbl: *const usize,
}

fn callback_object() -> Box<Callback> {
    let v: &'static [usize; 3] = Box::leak(Box::new([cb0 as *const () as usize, cb1 as *const () as usize, cb2 as *const () as usize]));
    Box::new(Callback { _vtbl: v.as_ptr() })
}

fn take_calls() -> Vec<(usize, u64, u32)> {
    core::mem::take(&mut *CALLS.lock().unwrap())
}

unsafe fn slot<T>(e: *mut XmaBufferEngine, i: usize) -> T {
    let p = *(*(e as *const *const usize)).add(i);
    core::mem::transmute_copy(&p)
}

unsafe fn qi(e: *mut XmaBufferEngine, iid: &[u8; 16], out: *mut *mut c_void) -> i32 {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine, *const [u8; 16], *mut *mut c_void) -> i32>(e, 0)(e, iid, out)
}
unsafe fn add_ref(e: *mut XmaBufferEngine) -> u32 {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine) -> u32>(e, 1)(e)
}
unsafe fn release(e: *mut XmaBufferEngine) -> u32 {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine) -> u32>(e, 2)(e)
}
unsafe fn initialize(e: *mut XmaBufferEngine, rate: u32, ch: u32) -> i32 {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine, u32, u32) -> i32>(e, 3)(e, rate, ch)
}
unsafe fn set_output(e: *mut XmaBufferEngine, p: *mut u8, frames: u32) -> i32 {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine, *mut u8, u32) -> i32>(e, 4)(e, p, frames)
}
unsafe fn stream_count(e: *mut XmaBufferEngine) -> u32 {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine) -> u32>(e, 7)(e)
}
unsafe fn process(e: *mut XmaBufferEngine, bytes: u32, out: &mut ProcessOutput) -> i32 {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine, u32, u32, *mut ProcessOutput) -> i32>(e, 9)(e, 0, bytes, out)
}
unsafe fn tick(e: *mut XmaBufferEngine, cb: *mut Callback) {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine, *mut Callback)>(e, 10)(e, cb)
}
unsafe fn submit(e: *mut XmaBufferEngine, b: &XmaBuffer) -> i32 {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine, *const XmaBuffer) -> i32>(e, 12)(e, b)
}
unsafe fn activate(e: *mut XmaBufferEngine) -> i32 {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine) -> i32>(e, 13)(e)
}
unsafe fn discontinuity(e: *mut XmaBufferEngine) {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine)>(e, 14)(e)
}
unsafe fn flush(e: *mut XmaBufferEngine, cb: *mut Callback, keep: &mut u32) {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine, *mut Callback, *mut u32)>(e, 15)(e, cb, keep)
}
unsafe fn exit_loop(e: *mut XmaBufferEngine) {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine)>(e, 18)(e)
}

unsafe fn apu_alloc(m: isize, size: u32) -> *mut u8 {
    let p = GetProcAddress(m, b"ApuAlloc\0".as_ptr());
    let f: unsafe extern "system" fn(*mut *mut c_void, *mut u32, u32, u32, u32) -> i32 = core::mem::transmute(p);
    let mut v: *mut c_void = core::ptr::null_mut();
    let mut a = 0u32;
    assert_eq!(f(&mut v, &mut a, size, 0x800, 1), 0);
    v as *mut u8
}

unsafe fn apu_free(m: isize, p: *mut u8) {
    let f = GetProcAddress(m, b"ApuFree\0".as_ptr());
    let f: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(f);
    f(p as *mut c_void);
}

fn silent_frame(w: &mut xma::bits::BitWriter, channels: usize, more: bool) {
    w.put(15, if channels == 1 { 31 } else { 36 });
    w.put(1, 1);
    w.put(1, 0);
    if channels == 2 {
        w.put(1, 0);
    }
    w.put(8, 0);
    w.put(1, 0);
    w.put(1, 0);
    w.put(1, 0);
    if channels == 2 {
        w.put(1, 0);
        w.put(1, 1);
        w.put(1, 0);
    }
    for _ in 0..channels {
        w.put(1, 0);
    }
    w.put(1, 0);
    w.put(1, more as u32);
}

fn silent_packet(channels: usize, frames: u32, skip: u32) -> Vec<u8> {
    let mut w = xma::bits::BitWriter::default();
    w.put(6, frames);
    w.put(15, 0);
    w.put(3, 1);
    w.put(8, skip);
    for i in 0..frames {
        silent_frame(&mut w, channels, i + 1 < frames);
    }
    let mut b = w.buf;
    b.resize(2048, 0);
    b
}

struct Setup {
    m: isize,
    state: *mut u8,
    engine: *mut XmaBufferEngine,
    out: Vec<i16>,
}

unsafe fn setup(rate: u32, channels: u32, frames: u32) -> Setup {
    let m = load_audio_dlls();
    let mut state: *mut u8 = core::ptr::null_mut();
    assert_eq!(CreateXAudioAcpState(core::ptr::null(), 0, &mut state), 0);
    let mut e: *mut XmaBufferEngine = core::ptr::null_mut();
    assert_eq!(CreateXmaBufferEngine(state, &mut e), 0);
    assert!(!e.is_null());
    let mut out = vec![0i16; frames as usize * channels as usize];
    assert_eq!(set_output(e, out.as_mut_ptr() as *mut u8, frames), 0);
    assert_eq!(initialize(e, rate, channels), 0);
    Setup { m, state, engine: e, out }
}

unsafe fn teardown(s: Setup) {
    assert_eq!(release(s.engine), 0);
    DestroyXAudioAcpState(s.state);
    drop(s.out);
}

unsafe fn put_buffer(m: isize, data: &[u8]) -> *mut u8 {
    let p = apu_alloc(m, data.len() as u32);
    core::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
    p
}

unsafe fn drain(e: *mut XmaBufferEngine, cb: *mut Callback, channels: usize, request: u32) -> (Vec<i16>, Vec<ProcessOutput>) {
    let mut pcm = Vec::new();
    let mut ends = Vec::new();
    for _ in 0..200_000 {
        tick(e, cb);
        let mut o: ProcessOutput = core::mem::zeroed();
        assert_eq!(process(e, request * channels as u32 * 2, &mut o), 0);
        if o.bytes != 0 {
            let s = core::slice::from_raw_parts(o.data as *const i16, o.bytes as usize / 2);
            pcm.extend_from_slice(s);
        }
        if o.buffer_end != 0 {
            ends.push(o);
            if o.end_of_stream != 0 {
                break;
            }
        }
    }
    (pcm, ends)
}

fn xma_owner_bits() -> Vec<u8> {
    owner_flags(XMA_KIND).iter().map(|b| b & OWNER_XMA_IN_USE).collect()
}

#[test]
fn query_interface_add_ref_and_release_follow_console_slots() {
    let _g = serial();
    unsafe {
        let s = setup(48000, 2, 256);
        let e = s.engine;
        let mut p: *mut c_void = core::ptr::null_mut();
        assert_eq!(qi(e, &IID_IUNKNOWN, &mut p), 0);
        assert_eq!(p, e as *mut c_void);
        assert_eq!(qi(e, &IID_XMA_BUFFER_ENGINE, &mut p), 0);
        assert_eq!(p, e as *mut c_void);
        let other = [7u8; 16];
        p = 1 as *mut c_void;
        assert_eq!(qi(e, &other, &mut p), E_NOINTERFACE);
        assert!(p.is_null());
        assert_eq!(add_ref(e), 4);
        assert_eq!(release(e), 3);
        assert_eq!(release(e), 2);
        assert_eq!(release(e), 1);
        assert_eq!(initialize(e, 48000, 2), E_UNEXPECTED);
        teardown(s);
    }
}

#[test]
fn initialize_takes_one_shape_xma_context_per_stream_and_release_returns_them() {
    let _g = serial();
    unsafe {
        let before = module_refs();
        let s = setup(48000, 5, 512);
        assert_eq!(stream_count(s.engine), 3);
        assert_eq!(module_refs(), before + 1);
        let used = xma_owner_bits().iter().filter(|b| **b != 0).count();
        assert_eq!(used, 3);
        assert_eq!(owner_flags(XMA_KIND).len(), DEFAULT_XMA_CONTEXTS as usize);
        let st = s.state;
        let m = s.m;
        assert_eq!(release(s.engine), 0);
        assert_eq!(module_refs(), before);
        assert!(xma_owner_bits().iter().all(|b| *b == 0));
        let _ = m;
        DestroyXAudioAcpState(st);
    }
}

#[test]
fn stream_configs_match_console_output_sizing() {
    let c = stream_configs(48000, 2, 256);
    assert_eq!(c.len(), 1);
    assert_eq!((c[0].out_frames, c[0].channels, c[0].subframes), (1280, 2, 4));
    let c = stream_configs(44100, 1, 256);
    assert_eq!((c[0].out_frames, c[0].channels, c[0].subframes), (256 + 1024, 1, 4));
    let c = stream_configs(48000, 3, 1024);
    assert_eq!(c.len(), 2);
    assert_eq!((c[0].channels, c[1].channels), (2, 1));
    assert_eq!((c[0].out_frames, c[0].subframes), (1024 + 512, 2));
}

#[test]
fn submit_source_buffer_validates_like_the_console() {
    let _g = serial();
    unsafe {
        let s = setup(48000, 1, 256);
        let e = s.engine;
        let pkt = silent_packet(1, 2, 0);
        let host = pkt.clone();
        let mut b = XmaBuffer { audio_bytes: 2048, audio_data: host.as_ptr() as u64, ..Default::default() };
        assert_eq!(submit(e, &b), XAUDIO2_E_INVALID_CALL);
        let apu = put_buffer(s.m, &pkt);
        b.audio_data = apu as u64;
        b.audio_bytes = 2000;
        assert_eq!(submit(e, &b), XAUDIO2_E_INVALID_CALL);
        b.audio_bytes = 2048;
        b.play_begin = 100;
        assert_eq!(submit(e, &b), XAUDIO2_E_INVALID_CALL);
        b.play_begin = 0;
        b.play_length = 130;
        assert_eq!(submit(e, &b), XAUDIO2_E_INVALID_CALL);
        b.play_length = 0;
        *apu.add(2) &= !7;
        assert_eq!(submit(e, &b), XAUDIO2_E_INVALID_CALL);
        *apu.add(2) |= 1;
        assert_eq!(submit(e, &b), 0);
        assert_eq!(activate(e), 0);
        assert_eq!(activate(e), QUEUE_EMPTY);
        teardown(s);
        apu_free(s_m(), apu);
    }
}

fn s_m() -> isize {
    unsafe { GetModuleHandleW(wide("acphal.dll").as_ptr()) }
}

#[test]
fn silent_mono_buffer_plays_its_play_region_and_loops_with_callbacks() {
    let _g = serial();
    take_calls();
    unsafe {
        let s = setup(48000, 1, 256);
        let e = s.engine;
        let mut cb = callback_object();
        let mut data = Vec::new();
        for _ in 0..4 {
            data.extend(silent_packet(1, 4, 0));
        }
        let apu = put_buffer(s.m, &data);
        let b = XmaBuffer { flags: XAUDIO2_END_OF_STREAM, audio_bytes: data.len() as u32, audio_data: apu as u64, play_begin: 512, play_length: 2048, loop_begin: 1024, loop_length: 512, loop_count: 2, context: 0x1234 };
        assert_eq!(submit(e, &b), 0);
        assert_eq!(activate(e), 0);
        let (pcm, ends) = drain(e, &mut *cb, 1, 256);
        assert_eq!(pcm.len(), 2048 + 2 * 512);
        assert!(pcm.iter().all(|v| *v == 0));
        assert_eq!(ends.len(), 1);
        assert_eq!({ ends[0].context }, 0x1234);
        assert_eq!(ends[0].end_of_stream, 1);
        let calls = take_calls();
        assert_eq!(calls.first(), Some(&(0, 0x1234, XAUDIO2_END_OF_STREAM)));
        assert_eq!(calls.iter().filter(|c| c.0 == 2).count(), 2);
        teardown(s);
        apu_free(s_m(), apu);
    }
}

#[test]
fn exit_loop_and_flush_release_buffers_through_callback_slot_one() {
    let _g = serial();
    take_calls();
    unsafe {
        let s = setup(48000, 2, 256);
        let e = s.engine;
        let mut cb = callback_object();
        let data = [silent_packet(2, 8, 0), silent_packet(2, 8, 0)].concat();
        let apu = put_buffer(s.m, &data);
        let mut b = XmaBuffer { audio_bytes: data.len() as u32, audio_data: apu as u64, loop_count: XAUDIO2_LOOP_INFINITE, context: 1, ..Default::default() };
        assert_eq!(submit(e, &b), 0);
        b.loop_count = 0;
        b.context = 2;
        assert_eq!(submit(e, &b), 0);
        b.context = 3;
        assert_eq!(submit(e, &b), 0);
        for _ in 0..3 {
            assert_eq!(activate(e), 0);
        }
        tick(e, &mut *cb);
        let mut o: ProcessOutput = core::mem::zeroed();
        for _ in 0..40 {
            tick(e, &mut *cb);
            process(e, 1024, &mut o);
        }
        assert!(take_calls().iter().filter(|c| c.0 == 2).count() >= 1);
        let mut keep = 1u32;
        flush(e, &mut *cb, &mut keep);
        assert_eq!(keep, 0);
        assert_eq!(take_calls(), vec![(1, 2, 0), (1, 3, 0)]);
        exit_loop(e);
        discontinuity(e);
        let (pcm, ends) = drain(e, &mut *cb, 2, 256);
        assert!(!pcm.is_empty());
        assert_eq!(ends.len(), 1);
        assert_eq!({ ends[0].context }, 1);
        teardown(s);
        apu_free(s_m(), apu);
    }
}

const XDEFIANT_DATA: &str = r"D:\xbox-pc-port\games\XDefiant\extracted\Mount\hunter\sdf\scarlett\data";

fn load_riff(file: &str, off: u64, size: usize) -> (u32, u32, Vec<u8>) {
    use std::io::{Read, Seek, SeekFrom};
    let path = format!(r"{XDEFIANT_DATA}\{file}");
    let mut f = std::fs::File::open(&path).unwrap_or_else(|e| panic!("real XMA2 source {path}: {e}"));
    f.seek(SeekFrom::Start(off)).unwrap();
    let mut b = vec![0u8; size];
    f.read_exact(&mut b).unwrap();
    assert_eq!(&b[0..4], b"RIFF");
    let mut q = 12;
    let (mut ch, mut rate, mut data) = (0, 0, Vec::new());
    while q + 8 <= b.len() {
        let id = &b[q..q + 4];
        let len = u32::from_le_bytes(b[q + 4..q + 8].try_into().unwrap()) as usize;
        let body = &b[q + 8..(q + 8 + len).min(b.len())];
        if id == b"fmt " {
            ch = u16::from_le_bytes([body[2], body[3]]) as u32;
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
        } else if id == b"data" {
            data = body.to_vec();
        }
        q += 8 + len + (len & 1);
    }
    (ch, rate, data)
}

#[test]
fn real_stereo_xma2_decodes_identically_to_the_shared_decoder() {
    let _g = serial();
    take_calls();
    let (ch, rate, data) = load_riff("sdf-A-0288.sdfdata", 0x3959878, 45064);
    assert_eq!(ch, 2);
    let data = &data[..data.len() / 2048 * 2048];
    let mut reference = Vec::new();
    let mut d = xma::Xma2Stream::new_hardware(rate, 2).unwrap();
    let mut idx = 0;
    while idx < data.len() / 2048 {
        let p = &data[idx * 2048..idx * 2048 + 2048];
        d.decode_packet(p, &mut reference);
        let skip = p[3] as usize;
        if skip == 0xff {
            break;
        }
        idx += skip + 1;
    }
    assert!(reference.iter().any(|v| *v != 0));
    unsafe {
        let s = setup(rate, 2, 512);
        let e = s.engine;
        let mut cb = callback_object();
        let apu = put_buffer(s.m, data);
        let b = XmaBuffer { flags: XAUDIO2_END_OF_STREAM, audio_bytes: data.len() as u32, audio_data: apu as u64, context: 0x77, ..Default::default() };
        assert_eq!(submit(e, &b), 0);
        assert_eq!(activate(e), 0);
        let (pcm, ends) = drain(e, &mut *cb, 2, 512);
        assert_eq!(ends.len(), 1);
        assert_eq!(pcm.len(), reference.len());
        assert!(pcm == reference);
        assert_eq!(take_calls(), vec![(0, 0x77, XAUDIO2_END_OF_STREAM)]);
        teardown(s);
        apu_free(s_m(), apu);
    }
}

fn reference_stream(data: &[u8], rate: u32, stream: usize, channels: usize) -> Vec<i16> {
    let mut out = Vec::new();
    let mut d = xma::Xma2Stream::new_hardware(rate, channels).unwrap();
    let mut idx = stream;
    let count = data.len() / 2048;
    while idx < count {
        let p = &data[idx * 2048..idx * 2048 + 2048];
        d.decode_packet(p, &mut out);
        let skip = p[3] as usize;
        if skip == 0xff {
            break;
        }
        idx += skip + 1;
    }
    out
}

#[test]
fn two_queued_buffers_play_back_to_back_without_losing_the_ring_tail() {
    let _g = serial();
    take_calls();
    let (_, rate, data) = load_riff("sdf-A-0288.sdfdata", 0x3959878, 45064);
    let data = &data[..data.len() / 2048 * 2048];
    let reference = reference_stream(data, rate, 0, 2);
    unsafe {
        let s = setup(rate, 2, 512);
        let e = s.engine;
        let mut cb = callback_object();
        let apu = put_buffer(s.m, data);
        let mut b = XmaBuffer { audio_bytes: data.len() as u32, audio_data: apu as u64, context: 0xa, ..Default::default() };
        assert_eq!(submit(e, &b), 0);
        b.context = 0xb;
        b.flags = XAUDIO2_END_OF_STREAM;
        assert_eq!(submit(e, &b), 0);
        assert_eq!(activate(e), 0);
        assert_eq!(activate(e), 0);
        let (pcm, ends) = drain(e, &mut *cb, 2, 512);
        assert_eq!(ends.iter().map(|o| o.context).collect::<Vec<_>>(), vec![0xa, 0xb]);
        assert_eq!(ends[0].end_of_stream, 0);
        assert_eq!(ends[1].end_of_stream, 1);
        assert_eq!(pcm.len(), reference.len() * 2);
        assert!(pcm[..reference.len()] == reference[..]);
        assert!(pcm[reference.len()..] == reference[..]);
        assert_eq!(take_calls(), vec![(0, 0xa, 0), (0, 0xb, XAUDIO2_END_OF_STREAM)]);
        teardown(s);
        apu_free(s_m(), apu);
    }
}

#[test]
fn real_quad_xma2_walks_packet_skips_and_interleaves_into_the_output_buffer() {
    let _g = serial();
    take_calls();
    let (ch, rate, data) = load_riff("sdf-A-0287.sdfdata", 0xbc1b70, 1071112);
    assert_eq!(ch, 4);
    let data = &data[..data.len() / 2048 * 2048];
    let a = reference_stream(data, rate, 0, 2);
    let b2 = reference_stream(data, rate, 1, 2);
    let frames = (a.len() / 2).min(b2.len() / 2);
    assert!(frames > 0);
    let mut reference = Vec::with_capacity(frames * 4);
    for k in 0..frames {
        reference.extend_from_slice(&[a[2 * k], a[2 * k + 1], b2[2 * k], b2[2 * k + 1]]);
    }
    unsafe {
        let s = setup(rate, 4, 512);
        let e = s.engine;
        assert_eq!(stream_count(e), 2);
        let mut cb = callback_object();
        let apu = put_buffer(s.m, data);
        let b = XmaBuffer { flags: XAUDIO2_END_OF_STREAM, audio_bytes: data.len() as u32, audio_data: apu as u64, context: 0x44, ..Default::default() };
        assert_eq!(submit(e, &b), 0);
        assert_eq!(activate(e), 0);
        let (pcm, ends) = drain(e, &mut *cb, 4, 512);
        assert_eq!(ends.len(), 1);
        assert_eq!({ ends[0].context }, 0x44);
        assert_eq!(pcm.len(), reference.len());
        assert!(pcm == reference);
        assert!(pcm.iter().any(|v| *v != 0));
        teardown(s);
        apu_free(s_m(), apu);
    }
}
