use core::ffi::c_void;
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl Guid {
    pub fn bytes(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&self.data1.to_le_bytes());
        b[4..6].copy_from_slice(&self.data2.to_le_bytes());
        b[6..8].copy_from_slice(&self.data3.to_le_bytes());
        b[8..16].copy_from_slice(&self.data4);
        b
    }
}

pub type EnableCallback = unsafe extern "system" fn(*const Guid, u32, u8, u64, u64, *const c_void, *mut c_void);

#[repr(C)]
pub struct EventDescriptor {
    pub id: u16,
    pub version: u8,
    pub channel: u8,
    pub level: u8,
    pub opcode: u8,
    pub task: u16,
    pub keyword: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventData {
    pub ptr: u64,
    pub size: u32,
    pub kind: u32,
}

#[link(name = "advapi32")]
extern "system" {
    fn EventRegister(id: *const Guid, cb: Option<EnableCallback>, ctx: *mut c_void, handle: *mut u64) -> u32;
    fn EventUnregister(handle: u64) -> u32;
    fn EventSetInformation(handle: u64, class: u32, info: *const c_void, len: u32) -> u32;
    fn EventWriteTransfer(handle: u64, desc: *const EventDescriptor, activity: *const Guid, related: *const Guid, n: u32, data: *const EventData) -> u32;
}

pub const PROVIDER_GROUP: Guid = Guid { data1: 0x4f50731a, data2: 0x89cf, data3: 0x4782, data4: [0xb3, 0xe0, 0xdc, 0xe8, 0xc9, 0x04, 0x76, 0xba] };
pub const NETWORK_PROFILE_GUID: Guid = Guid { data1: 0xed795972, data2: 0x60e8, data3: 0x4815, data4: [0x86, 0x34, 0xcf, 0xaa, 0x21, 0xa8, 0x9d, 0xe7] };
pub const AGGREGATE_GUID: Guid = Guid { data1: 0x703fcc13, data2: 0xb66f, data3: 0x5868, data4: [0xdd, 0xd9, 0xe2, 0xdb, 0x7f, 0x38, 0x1f, 0xfb] };

pub struct Provider {
    pub level_plus1: AtomicU32,
    pub any: AtomicU64,
    pub all: AtomicU64,
    pub handle: AtomicU64,
    pub callback: AtomicUsize,
    pub context: AtomicPtr<c_void>,
    pub guid: Guid,
    pub name: &'static str,
    traits: OnceLock<Vec<u8>>,
}

pub fn hr_from_win32(e: u32) -> i32 {
    if e as i32 <= 0 {
        e as i32
    } else {
        ((e & 0xffff) | 0x8007_0000) as i32
    }
}

impl Provider {
    pub const fn new(guid: Guid, name: &'static str) -> Provider {
        Provider {
            level_plus1: AtomicU32::new(0),
            any: AtomicU64::new(0),
            all: AtomicU64::new(0),
            handle: AtomicU64::new(0),
            callback: AtomicUsize::new(0),
            context: AtomicPtr::new(core::ptr::null_mut()),
            guid,
            name,
            traits: OnceLock::new(),
        }
    }

    pub fn traits(&self) -> &[u8] {
        self.traits.get_or_init(|| {
            let mut t = vec![0u8, 0u8];
            t.extend_from_slice(self.name.as_bytes());
            t.push(0);
            t.extend_from_slice(&[0x13, 0x00, 0x01]);
            t.extend_from_slice(&PROVIDER_GROUP.bytes());
            let n = t.len() as u16;
            t[0..2].copy_from_slice(&n.to_le_bytes());
            t
        })
    }

    pub fn register(&'static self, cb: Option<EnableCallback>, ctx: *mut c_void) -> i32 {
        self.callback.store(cb.map_or(0, |f| f as usize), Ordering::SeqCst);
        self.context.store(ctx, Ordering::SeqCst);
        let traits = self.traits();
        let mut h = 0u64;
        let st = unsafe { EventRegister(&self.guid, Some(provider_enable_callback), self as *const Provider as *mut c_void, &mut h) };
        self.handle.store(h, Ordering::SeqCst);
        if st != 0 {
            return hr_from_win32(st);
        }
        unsafe { EventSetInformation(h, 2, traits.as_ptr() as *const c_void, traits.len() as u32) };
        0
    }

    pub fn unregister(&self) -> u32 {
        let h = self.handle.swap(0, Ordering::SeqCst);
        self.level_plus1.store(0, Ordering::SeqCst);
        unsafe { EventUnregister(h) }
    }

    pub fn callback_is(&self, f: EnableCallback) -> bool {
        self.callback.load(Ordering::SeqCst) == f as usize
    }

    pub fn write(&self, desc: &EventDescriptor, meta: &[u8], fields: &[EventData]) -> u32 {
        let traits = self.traits();
        let mut data = Vec::with_capacity(fields.len() + 2);
        data.push(EventData { ptr: traits.as_ptr() as u64, size: traits.len() as u32, kind: 2 });
        data.push(EventData { ptr: meta.as_ptr() as u64, size: u16::from_le_bytes([meta[0], meta[1]]) as u32, kind: 1 });
        data.extend_from_slice(fields);
        unsafe { EventWriteTransfer(self.handle.load(Ordering::SeqCst), desc, core::ptr::null(), core::ptr::null(), data.len() as u32, data.as_ptr()) }
    }
}

pub unsafe extern "system" fn provider_enable_callback(src: *const Guid, enabled: u32, level: u8, any: u64, all: u64, filter: *const c_void, ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let p = &*(ctx as *const Provider);
    match enabled {
        0 => p.level_plus1.store(0, Ordering::SeqCst),
        1 => {
            p.any.store(any, Ordering::SeqCst);
            p.all.store(all, Ordering::SeqCst);
            p.level_plus1.store(if level == 0 { 0x100 } else { level as u32 + 1 }, Ordering::SeqCst);
        }
        _ => {}
    }
    let cb = p.callback.load(Ordering::SeqCst);
    if cb != 0 {
        let f: EnableCallback = core::mem::transmute(cb);
        f(src, enabled, level, any, all, filter, p.context.load(Ordering::SeqCst));
    }
}
