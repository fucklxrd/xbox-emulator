use core::ffi::c_void;
use gcxaudioext::acp_state::{CreateXAudioAcpState, DestroyXAudioAcpState};
use gcxaudioext::xma_engine::*;
use std::sync::Mutex;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> isize;
    fn GetProcAddress(m: isize, name: *const u8) -> *mut c_void;
}

static SERIAL: Mutex<()> = Mutex::new(());
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

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn load_dll(name: &str) -> isize {
    let exe = std::env::current_exe().unwrap();
    let dll = exe.ancestors().map(|d| d.join(name)).find(|p| p.is_file()).unwrap_or_else(|| panic!("build {name} first"));
    let m = unsafe { LoadLibraryW(wide(dll.to_str().unwrap()).as_ptr()) };
    assert_ne!(m, 0);
    m
}

unsafe fn apu_alloc(m: isize, size: u32) -> *mut u8 {
    let f: unsafe extern "system" fn(*mut *mut c_void, *mut u32, u32, u32, u32) -> i32 = core::mem::transmute(GetProcAddress(m, b"ApuAlloc\0".as_ptr()));
    let mut v: *mut c_void = core::ptr::null_mut();
    let mut a = 0u32;
    assert_eq!(f(&mut v, &mut a, size, 0x800, 1), 0);
    v as *mut u8
}

unsafe fn slot<T>(e: *mut XmaBufferEngine, i: usize) -> T {
    let p = *(*(e as *const *const usize)).add(i);
    core::mem::transmute_copy(&p)
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
unsafe fn process(e: *mut XmaBufferEngine, bytes: u32, out: *mut ProcessOutput) -> i32 {
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
unsafe fn stop(e: *mut XmaBufferEngine, cb: *mut Callback, no_wait: u32) {
    slot::<unsafe extern "system" fn(*mut XmaBufferEngine, *mut Callback, u32)>(e, 19)(e, cb, no_wait)
}

struct Env {
    m: isize,
    state: *mut u8,
    engine: *mut XmaBufferEngine,
    _out: Vec<i16>,
}

unsafe fn env(init: bool) -> Env {
    load_dll("xmem.dll");
    let m = load_dll("acphal.dll");
    let mut state: *mut u8 = core::ptr::null_mut();
    assert_eq!(CreateXAudioAcpState(core::ptr::null(), 0, &mut state), 0);
    let mut e: *mut XmaBufferEngine = 0x1 as *mut XmaBufferEngine;
    assert_eq!(CreateXmaBufferEngine(state, &mut e), 0);
    assert!(!e.is_null());
    let mut out = vec![0i16; 1024 * 2];
    assert_eq!(set_output(e, out.as_mut_ptr() as *mut u8, 1024), 0);
    if init {
        assert_eq!(initialize(e, 48000, 2), 0);
    }
    Env { m, state, engine: e, _out: out }
}

unsafe fn done(v: Env) {
    assert_eq!(release(v.engine), 0);
    DestroyXAudioAcpState(v.state);
}

unsafe fn packet_buffer(m: isize) -> *mut u8 {
    let p = apu_alloc(m, 0x800);
    core::ptr::write_bytes(p, 0, 0x800);
    *p.add(2) = 1;
    *p.add(3) = 0xff;
    p
}

fn buffer(p: *mut u8, context: u64) -> XmaBuffer {
    XmaBuffer { flags: 0, audio_bytes: 0x800, audio_data: p as u64, play_begin: 0, play_length: 0, loop_begin: 0, loop_length: 0, loop_count: 0, context }
}

#[test]
fn second_initialize_is_unexpected_and_keeps_the_streams() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let v = env(true);
        assert_eq!(stream_count(v.engine), 1);
        assert_eq!(initialize(v.engine, 48000, 6), E_UNEXPECTED);
        assert_eq!(stream_count(v.engine), 1);
        done(v);
    }
}

#[test]
fn queue_calls_before_initialize_fail_after_validation() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let v = env(false);
        let p = packet_buffer(v.m);
        let mut b = buffer(p, 1);
        assert_eq!(submit(v.engine, &b), E_INVALIDARG);
        b.audio_bytes = 0x801;
        assert_eq!(submit(v.engine, &b), XAUDIO2_E_INVALID_CALL);
        assert_eq!(activate(v.engine), E_INVALIDARG);
        done(v);
    }
}

#[test]
fn every_validation_rule_returns_invalid_call_independently() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let v = env(true);
        let p = packet_buffer(v.m);
        let ok = buffer(p, 1);
        let mut heap = vec![0u8; 0x1000];
        heap[2] = 1;
        let cases: Vec<XmaBuffer> = vec![
            XmaBuffer { audio_data: heap.as_mut_ptr() as u64, ..ok },
            XmaBuffer { audio_data: p as u64 + 0x10, ..ok },
            XmaBuffer { audio_bytes: 0x7f_f800 + 0x800, ..ok },
            XmaBuffer { audio_bytes: 0, play_begin: 0x40, ..ok },
            XmaBuffer { play_length: 0x81, ..ok },
        ];
        for c in &cases {
            assert_eq!(submit(v.engine, c), XAUDIO2_E_INVALID_CALL);
        }
        *p.add(2) = 2;
        assert_eq!(submit(v.engine, &ok), XAUDIO2_E_INVALID_CALL);
        *p.add(2) = 0x09;
        assert_eq!(submit(v.engine, &ok), 0);
        assert_eq!(activate(v.engine), 0);
        assert_eq!(activate(v.engine), QUEUE_EMPTY);
        done(v);
    }
}

#[test]
fn flush_keep_matrix_matches_console() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let v = env(true);
        let cb = Box::into_raw(callback_object());
        take_calls();
        let mut keep = 0u32;
        flush(v.engine, cb, &mut keep);
        assert_eq!(keep, 1);
        let mut keep = 1u32;
        flush(v.engine, cb, &mut keep);
        assert_eq!(keep, 1);
        assert!(take_calls().is_empty());
        let p = packet_buffer(v.m);
        assert_eq!(submit(v.engine, &buffer(p, 0xa)), 0);
        assert_eq!(activate(v.engine), 0);
        let mut keep = 1u32;
        flush(v.engine, cb, &mut keep);
        assert_eq!(keep, 0);
        assert!(take_calls().is_empty());
        assert_eq!(submit(v.engine, &buffer(p, 0xb)), 0);
        assert_eq!(submit(v.engine, &buffer(p, 0xc)), 0);
        assert_eq!(activate(v.engine), 0);
        assert_eq!(activate(v.engine), 0);
        let mut keep = 1u32;
        flush(v.engine, cb, &mut keep);
        assert_eq!(keep, 0);
        assert_eq!(take_calls(), vec![(1, 0xb, 0), (1, 0xc, 0)]);
        let mut keep = 0u32;
        flush(v.engine, cb, &mut keep);
        assert_eq!(keep, 1);
        assert_eq!(take_calls(), vec![(1, 0xa, 0)]);
        let mut keep = 0u32;
        flush(v.engine, cb, &mut keep);
        assert_eq!(keep, 1);
        assert!(take_calls().is_empty());
        done(v);
        drop(Box::from_raw(cb));
    }
}

#[test]
fn discontinuity_marks_the_tail_of_the_active_list() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let v = env(true);
        let cb = Box::into_raw(callback_object());
        take_calls();
        discontinuity(v.engine);
        let p = packet_buffer(v.m);
        assert_eq!(submit(v.engine, &buffer(p, 0x10)), 0);
        assert_eq!(submit(v.engine, &buffer(p, 0x11)), 0);
        discontinuity(v.engine);
        assert_eq!(activate(v.engine), 0);
        assert_eq!(activate(v.engine), 0);
        discontinuity(v.engine);
        tick(v.engine, cb);
        let first = take_calls();
        assert_eq!(first.first(), Some(&(0, 0x10, 0)));
        let mut keep = 1u32;
        flush(v.engine, cb, &mut keep);
        assert_eq!(take_calls(), vec![(1, 0x11, 0)]);
        done(v);
        drop(Box::from_raw(cb));
    }
}

#[test]
fn process_zeroes_the_output_and_returns_ok_without_data() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let v = env(false);
        let mut raw = [0xccu8; 0x20];
        assert_eq!(process(v.engine, 0, raw.as_mut_ptr() as *mut ProcessOutput), 0);
        assert!(raw.iter().all(|b| *b == 0));
        raw = [0xccu8; 0x20];
        assert_eq!(process(v.engine, 0x1000, raw.as_mut_ptr() as *mut ProcessOutput), 0);
        assert!(raw.iter().all(|b| *b == 0));
        done(v);
        let v = env(true);
        raw = [0xccu8; 0x20];
        assert_eq!(process(v.engine, 0x1000, raw.as_mut_ptr() as *mut ProcessOutput), 0);
        assert!(raw.iter().all(|b| *b == 0));
        assert_eq!(core::mem::size_of::<ProcessOutput>(), 0x20);
        assert_eq!(core::mem::size_of::<XmaBuffer>(), 0x2c);
        done(v);
    }
}

#[test]
fn stop_without_completed_buffers_calls_nothing_and_restarts_the_head() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let v = env(true);
        let cb = Box::into_raw(callback_object());
        take_calls();
        stop(v.engine, cb, 0);
        assert!(take_calls().is_empty());
        let p = packet_buffer(v.m);
        assert_eq!(submit(v.engine, &XmaBuffer { flags: XAUDIO2_END_OF_STREAM, ..buffer(p, 0x20) }), 0);
        assert_eq!(activate(v.engine), 0);
        tick(v.engine, cb);
        assert_eq!(take_calls().first(), Some(&(0, 0x20, XAUDIO2_END_OF_STREAM)));
        stop(v.engine, cb, 1);
        assert!(take_calls().iter().all(|c| c.0 != 0));
        tick(v.engine, cb);
        assert_eq!(take_calls().first(), Some(&(0, 0x20, XAUDIO2_END_OF_STREAM)));
        done(v);
        drop(Box::from_raw(cb));
    }
}

#[test]
fn vtable_has_twenty_distinct_console_ordered_entries() {
    let v = vtable();
    let s: Vec<usize> = (0..VTABLE_SLOTS).map(|i| unsafe { *v.add(i) }).collect();
    assert_eq!(s[16], s[17]);
    let mut d = s.clone();
    d.sort();
    d.dedup();
    assert_eq!(d.len(), VTABLE_SLOTS - 1);
    assert_eq!(IID_XMA_BUFFER_ENGINE, [0x6e, 0x81, 0xc8, 0xb2, 0xd2, 0x04, 0xde, 0x4a, 0x9b, 0x99, 0x4b, 0x43, 0x74, 0xe2, 0xa7, 0xaa]);
}
