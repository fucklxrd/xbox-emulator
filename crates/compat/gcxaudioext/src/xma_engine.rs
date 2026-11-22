use core::ffi::c_void;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crate::acp_state::{self, heap_alloc, heap_free, ShapeAllocator, STATE_ALLOCATOR, STATE_DEBUG};

pub const IID_IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46];
pub const IID_XMA_BUFFER_ENGINE: [u8; 16] = [0x6e, 0x81, 0xc8, 0xb2, 0xd2, 0x04, 0xde, 0x4a, 0x9b, 0x99, 0x4b, 0x43, 0x74, 0xe2, 0xa7, 0xaa];
pub const S_OK: i32 = 0;
pub const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
pub const E_OUTOFMEMORY: i32 = 0x8007_000Eu32 as i32;
pub const E_ACCESSDENIED: i32 = 0x8007_0005u32 as i32;
pub const E_UNEXPECTED: i32 = 0x8000_FFFFu32 as i32;
pub const XAUDIO2_E_INVALID_CALL: i32 = 0x8896_0001u32 as i32;
pub const QUEUE_EMPTY: i32 = 0x8031_0002u32 as i32;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const XAUDIO2_END_OF_STREAM: u32 = 0x40;
pub const XAUDIO2_LOOP_INFINITE: u32 = 0xff;
pub const MAX_QUEUED_BUFFERS: usize = 64;
pub const XMA_BYTES_PER_PACKET: u32 = 0x800;
pub const XMA_READBUFFER_MAX_BYTES: u32 = 0x7f_f800;
pub const XMA_SUBFRAME_SAMPLES: u32 = 128;
pub const VTABLE_SLOTS: usize = 20;
const OUTPUT_BLOCK_BYTES: usize = 0x100;
const OUTPUT_MAX_BYTES: u32 = 0x1f00;
const DEFAULT_SUBFRAMES: u32 = 4;

#[repr(C, packed)]
#[derive(Clone, Copy, Default, Debug)]
pub struct XmaBuffer {
    pub flags: u32,
    pub audio_bytes: u32,
    pub audio_data: u64,
    pub play_begin: u32,
    pub play_length: u32,
    pub loop_begin: u32,
    pub loop_length: u32,
    pub loop_count: u32,
    pub context: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProcessOutput {
    pub data: *mut u8,
    pub context: u64,
    pub bytes: u32,
    pub reserved: u32,
    pub bytes_valid: u32,
    pub end_of_stream: u8,
    pub buffer_end: u8,
    pub pad: [u8; 2],
}

#[derive(Clone, Copy, Debug)]
struct Done {
    remaining: u32,
    flags: u32,
    context: u64,
}

pub struct StreamConfig {
    pub sample_rate: u32,
    pub out_frames: u32,
    pub channels: u32,
    pub subframes: u32,
}

struct Decode {
    packets: Vec<usize>,
    next: usize,
    decoder: Option<xma::Xma2Stream>,
    pcm: VecDeque<i16>,
    pos: u64,
    cursor: u64,
    end: u64,
    loop_begin: u64,
    loop_end: u64,
    loops_left: u32,
    loops_done: u32,
    exhausted: bool,
}

struct Stream {
    context: u32,
    channels: usize,
    rate: u32,
    capacity: usize,
    ring: VecDeque<i16>,
    decode: Option<Decode>,
}

struct Inner {
    channels: u32,
    stream_count: u32,
    out: usize,
    out_frames: u32,
    warn: u32,
    queue_ready: bool,
    queue: VecDeque<XmaBuffer>,
    active: VecDeque<XmaBuffer>,
    completed: VecDeque<Done>,
    streams: Option<Vec<Stream>>,
    rate: u32,
    loop_count: u32,
    reported: u32,
    pending: Vec<(usize, u64, u32)>,
    running: bool,
    commit: bool,
    staging: Vec<i16>,
}

#[repr(C)]
pub struct XmaBufferEngine {
    vtbl: *const usize,
    refs: AtomicU32,
    state: *mut u8,
    inner: Mutex<Inner>,
}

static MODULE_REFS: AtomicU32 = AtomicU32::new(0);

pub fn module_refs() -> u32 {
    MODULE_REFS.load(Ordering::SeqCst)
}

#[link(name = "kernel32")]
extern "system" {
    fn OutputDebugStringA(s: *const u8);
}

fn debug_print(warn: u32, msg: &str) {
    if warn == 0 {
        return;
    }
    let mut s = format!("XAudio2: {msg}").into_bytes();
    s.push(0);
    unsafe { OutputDebugStringA(s.as_ptr()) };
}

fn round_up(x: u32, y: u32) -> u32 {
    let v = x.wrapping_add(y).wrapping_sub(1);
    v - v % y
}

pub fn stream_configs(sample_rate: u32, channels: u32, out_frames: u32) -> Vec<StreamConfig> {
    let size_channels = if channels > 1 { 2 } else { 1 };
    let bytes_per_frame = size_channels * 2;
    let max_frames = OUTPUT_MAX_BYTES / bytes_per_frame;
    let block_frames = OUTPUT_BLOCK_BYTES as u32 / bytes_per_frame;
    let mut base = round_up(out_frames, block_frames);
    let mut subframes = DEFAULT_SUBFRAMES;
    let mut sub_frames = subframes * XMA_SUBFRAME_SAMPLES;
    while subframes != 0 && base + sub_frames * 2 > max_frames {
        subframes /= 2;
        sub_frames /= 2;
    }
    base += sub_frames * 2;
    let count = (channels + 1) / 2;
    (0..count)
        .map(|i| StreamConfig { sample_rate, out_frames: base, channels: if i == count - 1 && channels % 2 == 1 { 1 } else { 2 }, subframes })
        .collect()
}

fn ring_capacity(c: &StreamConfig) -> usize {
    let bytes = c.out_frames as usize * c.channels as usize * 2;
    let blocks = (bytes / OUTPUT_BLOCK_BYTES) & 0x1f;
    let blocks = if blocks == 0 { 32 } else { blocks };
    (blocks - 1) * OUTPUT_BLOCK_BYTES / (c.channels as usize * 2)
}

unsafe fn allocator_of(state: *mut u8) -> *mut ShapeAllocator {
    *(state.add(STATE_ALLOCATOR) as *const *mut ShapeAllocator)
}

unsafe fn create_group(state: *mut u8, rate: u32, channels: u32, out_frames: u32) -> Result<Vec<Stream>, i32> {
    let a = allocator_of(state);
    if a.is_null() {
        return Err(E_UNEXPECTED);
    }
    let mut v: Vec<Stream> = Vec::new();
    for c in stream_configs(rate, channels, out_frames) {
        match acp_state::alloc_xma_context(a) {
            Ok(i) => v.push(Stream { context: i, channels: c.channels as usize, rate, capacity: ring_capacity(&c), ring: VecDeque::new(), decode: None }),
            Err(hr) => {
                free_group(v);
                return Err(hr);
            }
        }
    }
    Ok(v)
}

fn free_group(v: Vec<Stream>) {
    for s in v {
        acp_state::free_xma_context(s.context);
    }
}

unsafe fn packet_offsets(data: usize, bytes: u32, stream: u32) -> Vec<usize> {
    let mut v = Vec::new();
    let count = (bytes / XMA_BYTES_PER_PACKET) as usize;
    let mut idx = stream as usize;
    while idx < count {
        let p = data + idx * XMA_BYTES_PER_PACKET as usize;
        v.push(p);
        let skip = *(p as *const u8).add(3) as usize;
        if skip == 0xff {
            break;
        }
        idx += skip + 1;
    }
    v
}


impl Decode {
    unsafe fn new(b: &XmaBuffer, stream: u32) -> Decode {
        let packets = packet_offsets(b.audio_data as usize, b.audio_bytes, stream);
        let begin = b.play_begin as u64;
        let end = if b.play_length != 0 { begin + b.play_length as u64 } else { u64::MAX };
        let mut d = Decode { packets, next: 0, decoder: None, pcm: VecDeque::new(), pos: 0, cursor: begin, end, loop_begin: 0, loop_end: u64::MAX, loops_left: 0, loops_done: 0, exhausted: false };
        if b.loop_count != 0 {
            let lb = b.loop_begin as u64;
            let le = if b.loop_length != 0 { lb + b.loop_length as u64 } else { end };
            if le > lb {
                d.loop_begin = lb;
                d.loop_end = le;
                d.loops_left = b.loop_count.min(XAUDIO2_LOOP_INFINITE);
            }
        }
        d
    }

    fn looping(&self) -> bool {
        self.loops_left != 0
    }

    fn take_loop(&mut self) -> bool {
        if !self.looping() || self.packets.is_empty() {
            return false;
        }
        if self.loops_left != XAUDIO2_LOOP_INFINITE {
            self.loops_left -= 1;
        }
        self.loops_done = self.loops_done.wrapping_add(1);
        let lb = self.loop_begin;
        if lb >= self.pos {
            self.cursor = lb;
        } else {
            self.next = 0;
            self.decoder = None;
            self.pcm.clear();
            self.pos = 0;
            self.cursor = lb;
        }
        true
    }

    unsafe fn decode_more(&mut self, channels: usize, rate: u32) -> bool {
        if self.next >= self.packets.len() {
            return false;
        }
        if self.decoder.is_none() {
            match xma::Xma2Stream::new_hardware(rate, channels) {
                Ok(d) => self.decoder = Some(d),
                Err(_) => {
                    self.next = self.packets.len();
                    return false;
                }
            }
        }
        let p = core::slice::from_raw_parts(self.packets[self.next] as *const u8, XMA_BYTES_PER_PACKET as usize);
        self.next += 1;
        let mut out = Vec::new();
        self.decoder.as_mut().unwrap().decode_packet(p, &mut out);
        self.pcm.extend(out);
        true
    }

    unsafe fn fill(&mut self, ring: &mut VecDeque<i16>, capacity: usize, channels: usize, rate: u32) {
        let mut idle_loops = 0u32;
        while !self.exhausted && ring.len() / channels < capacity {
            let limit = if self.looping() { self.loop_end.min(self.end) } else { self.end };
            if self.cursor >= limit {
                if self.looping() && idle_loops < 2 && self.take_loop() {
                    idle_loops += 1;
                    continue;
                }
                self.exhausted = true;
                break;
            }
            let have = self.pos + (self.pcm.len() / channels) as u64;
            if self.cursor >= have {
                if self.decode_more(channels, rate) {
                    continue;
                }
                if self.looping() && idle_loops < 2 && self.take_loop() {
                    idle_loops += 1;
                    continue;
                }
                self.exhausted = true;
                break;
            }
            let from = (self.cursor - self.pos) as usize * channels;
            let room = capacity - ring.len() / channels;
            let n = ((have - self.cursor) as usize).min(room).min((limit - self.cursor).min(usize::MAX as u64) as usize);
            ring.extend(self.pcm.range(from..from + n * channels));
            self.cursor += n as u64;
            if n != 0 {
                idle_loops = 0;
            }
            if !self.looping() {
                let used = ((self.cursor - self.pos) as usize * channels).min(self.pcm.len());
                self.pcm.drain(..used);
                self.pos = self.cursor;
            }
        }
    }
}

impl Stream {
    fn busy(&self) -> bool {
        matches!(&self.decode, Some(d) if !d.exhausted)
    }

    fn frames(&self) -> u32 {
        (self.ring.len() / self.channels) as u32
    }

    unsafe fn service(&mut self) {
        let (ch, rate, cap) = (self.channels, self.rate, self.capacity);
        if let Some(d) = self.decode.as_mut() {
            d.fill(&mut self.ring, cap, ch, rate);
        }
    }

    fn loops_left(&self) -> u32 {
        self.decode.as_ref().map(|d| d.loops_left).unwrap_or(0)
    }

    fn loops_done(&self) -> u32 {
        self.decode.as_ref().map(|d| d.loops_done).unwrap_or(0)
    }
}

unsafe fn callback(cb: *mut c_void, slot: usize, context: u64, value: u32) {
    if cb.is_null() {
        return;
    }
    let f: unsafe extern "system" fn(*mut c_void, u64, u32) = core::mem::transmute(*(*(cb as *const *const usize)).add(slot));
    f(cb, context, value);
}

impl Inner {
    fn group(&self) -> &[Stream] {
        self.streams.as_deref().unwrap_or(&[])
    }

    fn group_mut(&mut self) -> &mut [Stream] {
        self.streams.as_deref_mut().unwrap_or(&mut [])
    }

    unsafe fn service(&mut self) {
        for s in self.group_mut() {
            s.service();
        }
    }

    fn any_busy(&self) -> bool {
        self.group().iter().any(|s| s.busy())
    }

    fn available(&self) -> u32 {
        self.group().iter().map(|s| s.frames()).min().unwrap_or(0)
    }

    unsafe fn start_next(&mut self) {
        let Some(head) = self.active.front().copied() else { return };
        for (i, s) in self.group_mut().iter_mut().enumerate() {
            s.decode = Some(Decode::new(&head, i as u32));
        }
        self.loop_count = if head.loop_count == XAUDIO2_LOOP_INFINITE { 0xfe } else { head.loop_count & 0xff };
        self.reported = 0;
        self.service();
        self.commit = true;
        self.pending.push((0, head.context, head.flags));
        self.running = true;
    }

    fn loop_callbacks(&mut self) {
        let Some(head) = self.active.front().copied() else { return };
        let done = self.group().iter().map(|s| s.loops_done()).min().unwrap_or(0);
        for _ in self.reported..done {
            self.pending.push((2, head.context, head.flags));
        }
        self.reported = self.reported.max(done);
        if head.loop_count != XAUDIO2_LOOP_INFINITE {
            self.loop_count = self.group().iter().map(|s| s.loops_left()).max().unwrap_or(0);
        }
    }

    fn pop_active(&mut self) {
        let Some(b) = self.active.pop_front() else { return };
        let remaining = self.group().first().map(|s| s.frames()).unwrap_or(0);
        self.completed.push_back(Done { remaining, flags: b.flags, context: b.context });
        self.reported = 0;
    }

    fn bytes_per_frame(&self) -> u32 {
        self.channels * 2
    }

    fn validate(&self, b: &XmaBuffer) -> bool {
        let mut ok = true;
        let data = b.audio_data as usize;
        let (bytes, begin, length) = (b.audio_bytes, b.play_begin, b.play_length);
        if !acp_state::apu_address_valid(data as *const c_void, XMA_BYTES_PER_PACKET) {
            debug_print(self.warn, &format!("XMA buffer pAudioData ({data:#x}) must be allocated with ApuAlloc and aligned to {XMA_BYTES_PER_PACKET} bytes"));
            ok = false;
        }
        let meta = if ok { unsafe { *(data as *const u8).add(2) as u32 & 7 } } else { 0 };
        if meta != 1 {
            debug_print(self.warn, &format!("First packet's metadata is {meta:#x}; should be 0x1 for XMA2 data"));
            ok = false;
        }
        if bytes % XMA_BYTES_PER_PACKET != 0 {
            debug_print(self.warn, &format!("XMA buffer size ({bytes}) must be a multiple of XMA_BYTES_PER_PACKET ({XMA_BYTES_PER_PACKET})"));
            ok = false;
        }
        if bytes > XMA_READBUFFER_MAX_BYTES {
            debug_print(self.warn, &format!("XMA buffer size ({bytes}) cannot exceed XMA_READBUFFER_MAX_BYTES ({XMA_READBUFFER_MAX_BYTES})"));
            ok = false;
        }
        if begin % XMA_SUBFRAME_SAMPLES != 0 {
            debug_print(self.warn, &format!("PlayBegin ({begin}) must fall on a {XMA_SUBFRAME_SAMPLES}-sample boundary for XMA buffers"));
            ok = false;
        }
        if length % XMA_SUBFRAME_SAMPLES != 0 {
            debug_print(self.warn, &format!("PlayLength ({length}) must be a multiple of {XMA_SUBFRAME_SAMPLES} samples for XMA buffers"));
            ok = false;
        }
        ok
    }

    fn drain_lists(&mut self) {
        self.active.clear();
        self.completed.clear();
    }
}

impl XmaBufferEngine {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    unsafe fn run_with(&self, cb: *mut c_void, f: impl FnOnce(&mut Inner)) {
        let mut g = self.lock();
        f(&mut g);
        let calls = core::mem::take(&mut g.pending);
        drop(g);
        for (slot, ctx, v) in calls {
            callback(cb, slot, ctx, v);
        }
    }
}

unsafe fn eng<'a>(this: *mut XmaBufferEngine) -> &'a XmaBufferEngine {
    &*this
}

unsafe extern "system" fn query_interface(this: *mut XmaBufferEngine, riid: *const [u8; 16], ppv: *mut *mut c_void) -> i32 {
    let id = *riid;
    if id == IID_IUNKNOWN || id == IID_XMA_BUFFER_ENGINE {
        *ppv = this as *mut c_void;
        add_ref(this);
        S_OK
    } else {
        *ppv = core::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn add_ref(this: *mut XmaBufferEngine) -> u32 {
    eng(this).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn release(this: *mut XmaBufferEngine) -> u32 {
    let n = eng(this).refs.fetch_sub(1, Ordering::SeqCst) - 1;
    if n != 0 {
        return n;
    }
    let mut inner = core::ptr::read(&(*this).inner).into_inner().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = inner.streams.take() {
        free_group(v);
    }
    inner.drain_lists();
    drop(inner);
    heap_free(this as *mut u8);
    MODULE_REFS.fetch_sub(1, Ordering::SeqCst);
    0
}

unsafe extern "system" fn initialize(this: *mut XmaBufferEngine, rate: u32, channels: u32) -> i32 {
    let e = eng(this);
    let mut g = e.lock();
    g.channels = channels;
    if g.queue_ready {
        return E_UNEXPECTED;
    }
    g.queue_ready = true;
    match create_group(e.state, rate, channels, g.out_frames) {
        Ok(v) => {
            g.stream_count = v.len() as u32;
            g.rate = rate;
            g.streams = Some(v);
            S_OK
        }
        Err(hr) => hr,
    }
}

unsafe extern "system" fn set_output_buffer(this: *mut XmaBufferEngine, p: *mut u8, frames: u32) -> i32 {
    let mut g = eng(this).lock();
    g.out = p as usize;
    g.out_frames = frames;
    S_OK
}

unsafe extern "system" fn set_sample_rate(this: *mut XmaBufferEngine, rate: u32) -> i32 {
    let e = eng(this);
    let mut g = e.lock();
    match create_group(e.state, rate, g.channels, g.out_frames) {
        Ok(v) => {
            if let Some(old) = g.streams.take() {
                free_group(old);
            }
            g.stream_count = v.len() as u32;
            g.rate = rate;
            g.streams = Some(v);
            S_OK
        }
        Err(hr) => hr,
    }
}

unsafe extern "system" fn destroy_streams(this: *mut XmaBufferEngine) {
    let mut g = eng(this).lock();
    if let Some(v) = g.streams.take() {
        free_group(v);
    }
    g.drain_lists();
}

unsafe extern "system" fn stream_count(this: *mut XmaBufferEngine) -> u32 {
    eng(this).lock().stream_count
}

unsafe extern "system" fn wait_commit(this: *mut XmaBufferEngine) {
    let mut g = eng(this).lock();
    g.service();
    g.commit = false;
}

unsafe extern "system" fn process(this: *mut XmaBufferEngine, _flags: u32, bytes: u32, out: *mut ProcessOutput) -> i32 {
    core::ptr::write_bytes(out as *mut u8, 0, core::mem::size_of::<ProcessOutput>());
    if bytes == 0 {
        return S_OK;
    }
    let mut g = eng(this).lock();
    let bpf = g.bytes_per_frame();
    if bpf == 0 {
        return S_OK;
    }
    g.service();
    let n = g.available().min(bytes / bpf) as usize;
    if n == 0 {
        return S_OK;
    }
    let channels = g.channels as usize;
    let direct = channels <= 2 || g.out == 0 || g.out_frames == 0;
    let mut buf = core::mem::take(&mut g.staging);
    if direct {
        buf.clear();
        buf.resize(n * channels, 0);
    }
    let (base, frames) = (g.out as *mut i16, g.out_frames as usize);
    for st in g.group_mut().iter_mut().enumerate() {
        let (s, st) = st;
        let sc = st.channels;
        let src: Vec<i16> = st.ring.drain(..n * sc).collect();
        for k in 0..n {
            for c in 0..sc {
                if direct {
                    buf[k * channels + s * 2 + c] = src[k * sc + c];
                } else {
                    *base.add((k % frames) * channels + s * 2 + c) = src[k * sc + c];
                }
            }
        }
    }
    g.staging = buf;
    let ptr = if direct { g.staging.as_mut_ptr() as *mut u8 } else { g.out as *mut u8 };
    g.service();
    let o = &mut *out;
    o.data = ptr;
    o.bytes = (n as u32) * bpf;
    o.bytes_valid = o.bytes;
    let n = n as u32;
    let mut i = 0;
    while i < g.completed.len() {
        let d = g.completed[i];
        if n < d.remaining {
            g.completed[i].remaining -= n;
            i += 1;
        } else {
            o.buffer_end = 1;
            if d.flags & XAUDIO2_END_OF_STREAM != 0 {
                o.end_of_stream = 1;
            }
            o.context = d.context;
            g.completed.remove(i);
        }
    }
    S_OK
}

unsafe extern "system" fn tick(this: *mut XmaBufferEngine, cb: *mut c_void) {
    eng(this).run_with(cb, |g| {
        g.service();
        let was = g.running;
        g.running = g.any_busy();
        if !was {
            if !g.running {
                g.start_next();
            }
        } else if !g.running {
            g.loop_callbacks();
            g.pop_active();
            g.start_next();
        } else if !g.active.is_empty() {
            g.loop_callbacks();
        }
    });
}

unsafe extern "system" fn commit(this: *mut XmaBufferEngine) {
    let mut g = eng(this).lock();
    g.service();
    g.commit = true;
}

unsafe extern "system" fn submit_source_buffer(this: *mut XmaBufferEngine, b: *const XmaBuffer) -> i32 {
    let mut g = eng(this).lock();
    let queued = g.active.len();
    if queued >= MAX_QUEUED_BUFFERS {
        debug_print(g.warn, &format!("The number of queued buffers on this voice ({queued}), exceeds the limit of {MAX_QUEUED_BUFFERS} buffers"));
    }
    let mut c = core::ptr::read_unaligned(b);
    if !g.validate(&c) {
        return XAUDIO2_E_INVALID_CALL;
    }
    if c.loop_count != 0 && c.loop_length == 0 && c.play_length != 0 {
        c.loop_length = c.play_begin.wrapping_add(c.play_length).wrapping_sub(c.loop_begin);
    }
    if !g.queue_ready {
        return E_INVALIDARG;
    }
    g.queue.push_back(c);
    S_OK
}

unsafe extern "system" fn activate_next(this: *mut XmaBufferEngine) -> i32 {
    let mut g = eng(this).lock();
    if !g.queue_ready {
        return E_INVALIDARG;
    }
    match g.queue.pop_front() {
        Some(b) => {
            g.active.push_back(b);
            S_OK
        }
        None => QUEUE_EMPTY,
    }
}

unsafe extern "system" fn discontinuity(this: *mut XmaBufferEngine) {
    let mut g = eng(this).lock();
    if let Some(t) = g.active.back_mut() {
        t.flags |= XAUDIO2_END_OF_STREAM;
    }
}

unsafe extern "system" fn flush(this: *mut XmaBufferEngine, cb: *mut c_void, keep: *mut u32) {
    eng(this).run_with(cb, |g| {
        let k = *keep;
        *keep = 0;
        if g.active.is_empty() {
            *keep = 1;
            return;
        }
        let skip = if k != 0 { 1 } else { 0 };
        if k == 0 {
            *keep = 1;
        }
        let rest: Vec<XmaBuffer> = g.active.drain(skip..).collect();
        for b in rest {
            g.pending.push((1, b.context, 0));
        }
    });
}

unsafe extern "system" fn nop(_this: *mut XmaBufferEngine) {}

unsafe extern "system" fn exit_loop(this: *mut XmaBufferEngine) {
    let mut g = eng(this).lock();
    g.loop_count = 0;
    for s in g.group_mut() {
        if let Some(d) = s.decode.as_mut() {
            d.loops_left = 0;
        }
    }
}

unsafe extern "system" fn stop(this: *mut XmaBufferEngine, cb: *mut c_void, _no_wait: u32) {
    eng(this).run_with(cb, |g| {
        for s in g.group_mut() {
            s.ring.clear();
            s.decode = None;
        }
        g.running = false;
        let done: Vec<Done> = g.completed.drain(..).collect();
        for d in done {
            g.pending.push((1, d.context, 0));
        }
    });
}

struct Vtable([usize; VTABLE_SLOTS]);

static VTABLE: std::sync::OnceLock<Vtable> = std::sync::OnceLock::new();

pub fn vtable() -> *const usize {
    VTABLE
        .get_or_init(|| {
            Vtable([
                query_interface as *const () as usize,
                add_ref as *const () as usize,
                release as *const () as usize,
                initialize as *const () as usize,
                set_output_buffer as *const () as usize,
                set_sample_rate as *const () as usize,
                destroy_streams as *const () as usize,
                stream_count as *const () as usize,
                wait_commit as *const () as usize,
                process as *const () as usize,
                tick as *const () as usize,
                commit as *const () as usize,
                submit_source_buffer as *const () as usize,
                activate_next as *const () as usize,
                discontinuity as *const () as usize,
                flush as *const () as usize,
                nop as *const () as usize,
                nop as *const () as usize,
                exit_loop as *const () as usize,
                stop as *const () as usize,
            ])
        })
        .0
        .as_ptr()
}

fn warn_level(state: *mut u8) -> u32 {
    let d = unsafe { core::ptr::read_unaligned(state.add(STATE_DEBUG) as *const [u32; 2]) };
    if d[1] & 1 != 0 {
        2
    } else if d[0] & 1 != 0 {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "system" fn CreateXmaBufferEngine(state: *mut u8, out: *mut *mut XmaBufferEngine) -> i32 {
    *out = core::ptr::null_mut();
    let hr = acp_state::initialize_state(state, core::ptr::null());
    if hr < 0 {
        return hr;
    }
    let p = heap_alloc(core::mem::size_of::<XmaBufferEngine>()) as *mut XmaBufferEngine;
    if p.is_null() {
        return E_OUTOFMEMORY;
    }
    core::ptr::write(
        p,
        XmaBufferEngine {
            vtbl: vtable(),
            refs: AtomicU32::new(0),
            state,
            inner: Mutex::new(Inner {
                channels: 0,
                stream_count: 0,
                out: 0,
                out_frames: 0,
                warn: warn_level(state),
                queue_ready: false,
                queue: VecDeque::new(),
                active: VecDeque::new(),
                completed: VecDeque::new(),
                streams: None,
                rate: 0,
                loop_count: 0,
                reported: 0,
                pending: Vec::new(),
                running: false,
                commit: false,
                staging: Vec::new(),
            }),
        },
    );
    MODULE_REFS.fetch_add(1, Ordering::SeqCst);
    add_ref(p);
    *out = p;
    S_OK
}
