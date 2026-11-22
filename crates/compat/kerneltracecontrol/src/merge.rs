pub const ERROR_UNIDENTIFIED_ERROR: u32 = 0x507;

pub static CONSOLE: console_image::ConsoleDll = console_image::ConsoleDll::new("KernelTraceControl.dll", "TOMBOX_KERNELTRACECONTROL_CONSOLE", "kerneltracecontrol_console.dll", "CreateMergedTraceFile", &[]);

type Merge = unsafe extern "system" fn(*const u16, *const *const u16, u32, u32) -> u32;

pub fn own() -> usize {
    own as *const () as usize
}

pub fn module() -> usize {
    CONSOLE.module(own())
}

#[no_mangle]
pub unsafe extern "system" fn CreateMergedTraceFile(merged: *const u16, inputs: *const *const u16, count: u32, flags: u32) -> u32 {
    match CONSOLE.proc(own(), "CreateMergedTraceFile") {
        Some(p) => {
            let f: Merge = core::mem::transmute(p);
            f(merged, inputs, count, flags)
        }
        None => {
            CONSOLE.missing("CreateMergedTraceFile", ERROR_UNIDENTIFIED_ERROR as usize);
            ERROR_UNIDENTIFIED_ERROR
        }
    }
}
