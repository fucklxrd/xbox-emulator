#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Hooked(&'static str),
    HookedByOtherOwner(&'static str),
    NameMatchPc(&'static str),
    Emulated(&'static str),
}

pub const AFFINITY: &str = "crates\\compat\\compat-common\\src\\os_host\\affinity.rs";
pub const PLM: &str = "crates\\compat\\compat-common\\src\\os_host\\plm.rs";
pub const DLLDIR: &str = "crates\\compat\\compat-common\\src\\os_host\\dlldir.rs";
pub const MINUSERX_QUEUE: &str = "crates\\compat\\minuserx\\src\\queue.rs";
pub const MINAPPMODELX: &str = "crates\\compat\\minappmodelx\\src\\lib.rs";
pub const GDK_REGISTRY: &str = "crates\\compat\\compat-common\\src\\gdk\\hook.rs";

const USER_SAME: &str = "same Win32 signature on console (minuserx exports wrap an internal CoreWindow-backed implementation that returns an HRESULT mapped to the Win32 result); the API set resolves to PC user32. Console differences are placement only (one fullscreen core window), handled by the window clamp in crates\\compat\\d3d12_xs\\src\\present.rs, so no hook";

pub const ROWS: &[(&str, &str, Status)] = &[
    ("kernel32legacy.dll", "SetThreadAffinityMask", Status::Hooked(AFFINITY)),
    ("kernel32legacy.dll", "GetProcessAffinityMask", Status::Hooked(AFFINITY)),
    ("kernel32legacy.dll", "SetProcessAffinityMask", Status::Hooked(AFFINITY)),
    ("kernelbase.dll", "CreateThread", Status::Hooked(AFFINITY)),
    ("kernelbase.dll", "GetCurrentProcessorNumber", Status::Hooked(AFFINITY)),
    ("kernel32legacy.dll", "SetDllDirectoryW", Status::Hooked(DLLDIR)),
    ("kernel32legacy.dll", "GetDllDirectoryW", Status::NameMatchPc("console: RtlSetLastWin32Error(0) + LdrGetDllDirectory, returns length in chars; same as PC KernelBase")),
    ("kernel32legacy.dll", "GetComputerNameW", Status::NameMatchPc("console: COMPUTERNAME environment variable first, then KernelBase global data; same as PC")),
    ("kernel32legacy.dll", "MoveFileW", Status::NameMatchPc("console: MoveFileWithProgressW(src, dst, NULL, NULL, MOVEFILE_COPY_ALLOWED); same as PC")),
    ("kernel32legacy.dll", "RegisterWaitForSingleObject", Status::NameMatchPc("console: std-handle pseudo handles from PEB then RtlRegisterWait; same as PC")),
    ("kernel32legacy.dll", "UnregisterWait", Status::NameMatchPc("console: NULL -> ERROR_INVALID_HANDLE, RtlDeregisterWait, STATUS_PENDING -> FALSE with ERROR_IO_PENDING; same as PC")),
    ("advapi32legacy.dll", "RegOpenKeyW", Status::HookedByOtherOwner(GDK_REGISTRY)),
    ("minappmodelx.dll", "RegisterAppStateChangeNotification", Status::Emulated(MINAPPMODELX)),
    ("minappmodelx.dll", "UnregisterAppStateChangeNotification", Status::Emulated(MINAPPMODELX)),
    ("minappmodelx.dll", "RegisterAppConstrainedChangeNotification", Status::Emulated(MINAPPMODELX)),
    ("minappmodelx.dll", "UnregisterAppConstrainedChangeNotification", Status::Emulated(MINAPPMODELX)),
    ("minuserx.dll", "CreateWindowExA", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "CreateWindowExW", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "DefWindowProcW", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "DestroyWindow", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "DispatchMessageW", Status::Emulated(MINUSERX_QUEUE)),
    ("minuserx.dll", "GetWindowLongPtrA", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "LoadCursorW", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "PeekMessageW", Status::Emulated(MINUSERX_QUEUE)),
    ("minuserx.dll", "PostMessageW", Status::Emulated(MINUSERX_QUEUE)),
    ("minuserx.dll", "PostQuitMessage", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "RegisterClassExA", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "RegisterClassExW", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "RegisterClassW", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "SendMessageW", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "SetTimer", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "SetWindowLongPtrA", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "SetWindowLongPtrW", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "ShowCursor", Status::NameMatchPc("console keeps one process-wide display counter starting at 0 (cursor shown) and updates the CoreWindow cursor at the 0/-1 crossings; PC keeps it per input thread. Trackmania calls it from its window thread, so the result is the same")),
    ("minuserx.dll", "ShowWindow", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "TranslateMessage", Status::NameMatchPc(USER_SAME)),
    ("minuserx.dll", "UnregisterClassW", Status::NameMatchPc(USER_SAME)),
];

pub fn summary() -> String {
    let hooked = ROWS.iter().filter(|r| matches!(r.2, Status::Hooked(_))).count();
    let other = ROWS.iter().filter(|r| matches!(r.2, Status::HookedByOtherOwner(_))).count();
    let emulated = ROWS.iter().filter(|r| matches!(r.2, Status::Emulated(_))).count();
    format!("{} rows: {hooked} hooked here, {other} hooked by another owner, {emulated} emulated by a compat crate, {} name-match pc", ROWS.len(), ROWS.len() - hooked - other - emulated)
}
