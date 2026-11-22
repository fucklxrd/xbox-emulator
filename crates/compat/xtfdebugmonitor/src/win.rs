use core::ffi::c_void;
pub use xtfcommon::sys::{CloseHandle, GetLastError, MultiByteToWideChar, WaitForSingleObject, INFINITE, INVALID_HANDLE_VALUE};
use xtfmsg::sys::{BOOL, HANDLE};

pub type ThreadProc = unsafe extern "system" fn(*mut c_void) -> u32;

#[link(name = "kernel32")]
extern "system" {
    pub fn CreateEventW(sa: *mut c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    pub fn SetEvent(h: HANDLE) -> BOOL;
    pub fn WaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32) -> u32;
    pub fn CreateThread(sa: *mut c_void, stack: usize, start: ThreadProc, param: *mut c_void, flags: u32, tid: *mut u32) -> HANDLE;
    pub fn DebugActiveProcess(pid: u32) -> BOOL;
    pub fn DebugActiveProcessStop(pid: u32) -> BOOL;
    pub fn WaitForDebugEvent(ev: *mut DebugEvent, ms: u32) -> BOOL;
    pub fn ContinueDebugEvent(pid: u32, tid: u32, status: u32) -> BOOL;
    pub fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    pub fn ReadProcessMemory(h: HANDLE, addr: *const c_void, buf: *mut c_void, n: usize, read: *mut usize) -> BOOL;
    pub fn Sleep(ms: u32);
}

pub const DBG_CONTINUE: u32 = 0x0001_0002;
pub const DBG_EXCEPTION_NOT_HANDLED: u32 = 0x8001_0001;
pub const PROCESS_VM_READ: u32 = 0x10;
pub const ERROR_SEM_TIMEOUT: u32 = 0x79;
pub const E_POINTER: i32 = 0x8000_4003u32 as i32;
pub const E_ILLEGAL_METHOD_CALL: i32 = 0x8000_000Au32 as i32;
pub const E_NOT_SET: i32 = 0x8007_0490u32 as i32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DebugEvent {
    pub code: u32,
    pub pid: u32,
    pub tid: u32,
    pub pad: u32,
    pub u: [u64; 20],
}

impl DebugEvent {
    pub fn zeroed() -> DebugEvent {
        DebugEvent { code: 0, pid: 0, tid: 0, pad: 0, u: [0; 20] }
    }
    fn bytes(&self) -> &[u8; 0xb0] {
        unsafe { &*(self as *const DebugEvent as *const [u8; 0xb0]) }
    }
    pub fn u16_at(&self, off: usize) -> u16 {
        u16::from_le_bytes(self.bytes()[off..off + 2].try_into().unwrap())
    }
    pub fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes(self.bytes()[off..off + 4].try_into().unwrap())
    }
    pub fn u64_at(&self, off: usize) -> u64 {
        u64::from_le_bytes(self.bytes()[off..off + 8].try_into().unwrap())
    }
}

pub fn last_error_hr() -> i32 {
    xtfmsg::sys::hr_from_win32(unsafe { GetLastError() })
}

pub fn last_error_or_fail() -> i32 {
    xtfmsg::sys::hr_last_error_or_fail()
}
