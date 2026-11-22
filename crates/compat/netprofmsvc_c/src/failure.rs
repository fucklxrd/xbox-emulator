use crate::tlg::{EventData, EventDescriptor};
use crate::PROVIDER;
use core::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::OnceLock;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CallContextInfo {
    pub context_id: i32,
    pub name: *const u8,
    pub message: *const u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FailureInfo {
    pub failure_type: i32,
    pub hr: i32,
    pub failure_id: i32,
    pub message: *const u16,
    pub thread_id: u32,
    pub code: *const u8,
    pub function: *const u8,
    pub file: *const u8,
    pub line: u32,
    pub failure_count: i32,
    pub call_context: *const u8,
    pub originating: CallContextInfo,
    pub current: CallContextInfo,
    pub module: *const u8,
    pub return_address: *mut c_void,
    pub caller_return_address: *mut c_void,
}

pub const FAILURE_FIELDS: [(&str, u8); 17] = [
    ("hresult", 0x08),
    ("fileName", 0x02),
    ("lineNumber", 0x08),
    ("module", 0x02),
    ("failureType", 0x08),
    ("message", 0x01),
    ("threadId", 0x08),
    ("callContext", 0x02),
    ("originatingContextId", 0x08),
    ("originatingContextName", 0x02),
    ("originatingContextMessage", 0x01),
    ("currentContextId", 0x08),
    ("currentContextName", 0x02),
    ("currentContextMessage", 0x01),
    ("failureId", 0x08),
    ("failureCount", 0x08),
    ("function", 0x02),
];

pub fn failure_metadata() -> &'static [u8] {
    static META: OnceLock<Vec<u8>> = OnceLock::new();
    META.get_or_init(|| {
        let mut m = vec![0u8, 0u8, 0u8];
        m.extend_from_slice(b"Failure\0");
        for (name, ty) in FAILURE_FIELDS {
            m.extend_from_slice(name.as_bytes());
            m.push(0);
            m.push(ty);
        }
        let n = m.len() as u16;
        m[0..2].copy_from_slice(&n.to_le_bytes());
        m
    })
}

pub const FAILURE_DESCRIPTOR: EventDescriptor = EventDescriptor { id: 0, version: 0, channel: 0x0b, level: 2, opcode: 0, task: 0, keyword: 0 };

static EMPTY_A: [u8; 1] = [0];
static EMPTY_W: [u16; 1] = [0];

unsafe fn ansi(p: *const u8) -> EventData {
    if p.is_null() {
        return EventData { ptr: EMPTY_A.as_ptr() as u64, size: 1, kind: 0 };
    }
    let mut n = 0usize;
    while *p.add(n) != 0 {
        n += 1;
    }
    EventData { ptr: p as u64, size: n as u32 + 1, kind: 0 }
}

unsafe fn wide(p: *const u16) -> EventData {
    if p.is_null() {
        return EventData { ptr: EMPTY_W.as_ptr() as u64, size: 2, kind: 0 };
    }
    let mut n = 0usize;
    while *p.add(n) != 0 {
        n += 1;
    }
    EventData { ptr: p as u64, size: (n as u32 + 1) * 2, kind: 0 }
}

fn int<T>(p: &T) -> EventData {
    EventData { ptr: p as *const T as u64, size: 4, kind: 0 }
}

pub unsafe fn failure_fields(f: &FailureInfo) -> [EventData; 17] {
    [
        int(&f.hr),
        ansi(f.file),
        int(&f.line),
        ansi(f.module),
        int(&f.failure_type),
        wide(f.message),
        int(&f.thread_id),
        ansi(f.call_context),
        int(&f.originating.context_id),
        ansi(f.originating.name),
        wide(f.originating.message),
        int(&f.current.context_id),
        ansi(f.current.name),
        wide(f.current.message),
        int(&f.failure_id),
        int(&f.failure_count),
        ansi(f.function),
    ]
}

pub unsafe fn write_failure(f: &FailureInfo) -> Option<u32> {
    if PROVIDER.level_plus1.load(Ordering::SeqCst) <= 2 {
        return None;
    }
    let fields = failure_fields(f);
    Some(PROVIDER.write(&FAILURE_DESCRIPTOR, failure_metadata(), &fields))
}

pub unsafe extern "system" fn result_logging_callback(f: *const FailureInfo, _msg: *mut u16, _cch: usize) {
    write_failure(&*f);
}
