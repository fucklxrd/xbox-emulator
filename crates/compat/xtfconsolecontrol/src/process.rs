use crate::chk::chk;
use crate::op::*;
use crate::privilege;
use crate::CONSOLE_EXTENSION_ID;
use core::ffi::c_void;
use std::path::{Path, PathBuf};
use xtfmsg::chk::Activity;
use xtfmsg::com::{self, array, Com, Obj, IID_XBTP_CLIENT_SESSION};
use xtfmsg::msg::{deserialize_body, serialize_end_body, set_json_u32, ErrorContext};
use xtfmsg::sys::*;
use xtfmsg::xbtp;

pub const GETRUNNINGPROCESSES_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\getrunningprocesses.cpp";
pub const CRASHDUMP_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\crashdump.cpp";

pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
pub const MAX_PATH: usize = 0x104;
pub const STRSAFE_E_INSUFFICIENT_BUFFER: HRESULT = 0x8007_007Au32 as i32;

pub const DUMP_TRIAGE: (u32, u32, &str) = (0x1001a4, 0x450, "triage");
pub const DUMP_MINI: (u32, u32, &str) = (0x40121, 0x410, "mini");
pub const DUMP_HEAP: (u32, u32, &str) = (0x51b27, 0x450, "heap");

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> HANDLE;
    fn CloseHandle(h: HANDLE) -> i32;
    fn FlushFileBuffers(h: HANDLE) -> i32;
    fn SetLastError(e: u32);
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: HANDLE, flags: u32) -> HANDLE;
}

type MiniDumpWriteDumpFn = unsafe extern "system" fn(HANDLE, u32, HANDLE, u32, *const c_void, *const c_void, *const c_void) -> i32;

const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
const ERROR_PROC_NOT_FOUND: u32 = 127;

unsafe fn mini_dump_write_dump(process: HANDLE, pid: u32, file: HANDLE, kind: u32) -> i32 {
    static F: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let f = *F.get_or_init(|| {
        let m = LoadLibraryExW(w("api-ms-win-core-debug-minidump-l1-1-0.dll").as_ptr(), core::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32);
        let m = if m.is_null() { LoadLibraryExW(w("dbgcore.dll").as_ptr(), core::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) } else { m };
        if m.is_null() {
            return 0;
        }
        GetProcAddress(m, b"MiniDumpWriteDump\0".as_ptr()) as usize
    });
    if f == 0 {
        SetLastError(ERROR_PROC_NOT_FOUND);
        return 0;
    }
    let f: MiniDumpWriteDumpFn = core::mem::transmute(f);
    f(process, pid, file, kind, core::ptr::null(), core::ptr::null(), core::ptr::null())
}

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

pub fn hosted_processes() -> Vec<u32> {
    vec![unsafe { GetCurrentProcessId() }]
}

fn strip_ci<'a>(p: &'a str, root: &str) -> Option<&'a str> {
    let root = root.trim_end_matches('\\');
    if p.len() < root.len() || !p.is_char_boundary(root.len()) || !p[..root.len()].eq_ignore_ascii_case(root) {
        return None;
    }
    let rest = &p[root.len()..];
    if rest.is_empty() {
        Some("")
    } else {
        rest.strip_prefix('\\')
    }
}

fn verbatim_free(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\");
    s.strip_prefix("\\\\?\\").map(str::to_string).unwrap_or(s)
}

pub fn console_path_of_host(host: &Path) -> Option<String> {
    let h = verbatim_free(host);
    if let Some(d) = compat_common::gdk::fs::game_drive() {
        if let Some(rest) = strip_ci(&h, &verbatim_free(&d.root)) {
            return Some(format!("{}:\\{}", d.letter, rest));
        }
    }
    let root = verbatim_free(&console_sandbox::fs::fs_root());
    let rest = strip_ci(&h, &root)?;
    let mut it = rest.split('\\').filter(|c| !c.is_empty());
    let drive = it.next()?;
    if drive.len() != 1 || !drive.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    let tail: Vec<&str> = it.collect();
    Some(format!("{}:\\{}", drive.to_ascii_uppercase(), tail.join("\\")))
}

pub fn full_path_name(p: &str) -> String {
    let mut s = p.replace('/', "\\");
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        let up = (b[0] as char).to_ascii_uppercase();
        s.replace_range(0..1, &up.to_string());
        if s.len() == 2 {
            s.push('\\');
        }
    }
    s
}

pub fn query_full_process_image_name(pid: u32) -> Option<String> {
    if pid != unsafe { GetCurrentProcessId() } {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let c = console_path_of_host(&exe)?;
    if c.len() >= MAX_PATH {
        return None;
    }
    Some(c)
}

pub unsafe fn get_process_image_file_name(pid: u32, out: &mut Option<String>) -> HRESULT {
    let f = "K32GetProcessImageFileNameW";
    let mut restore = false;
    let hr = privilege::set_privilege(privilege::SE_DEBUG_NAME, true, Some(&mut restore));
    chk(GETRUNNINGPROCESSES_CPP, f, 206, "XtfSetPrivilege(SE_DEBUG_NAME, true, &fRestoreState)", hr, "");
    if hr < 0 {
        return hr;
    }
    let mut hr = hr;
    match query_full_process_image_name(pid) {
        None => *out = None,
        Some(img) => {
            let full = full_path_name(&img);
            hr = S_OK;
            chk(GETRUNNINGPROCESSES_CPP, f, 217, "GetFullPathName(szFileName, &strPath)", hr, "");
            *out = Some(full);
        }
    }
    privilege::set_privilege(privilege::SE_DEBUG_NAME, restore, None);
    hr
}

unsafe fn copy_context(ctx: &ErrorContext) -> ErrorContext {
    if !ctx.errors.is_null() {
        com::add_ref(ctx.errors);
    }
    if !ctx.body.is_null() {
        com::add_ref(ctx.body);
    }
    ErrorContext { errors: ctx.errors, body: ctx.body, end_message: ctx.end_message, checkpoint: ctx.checkpoint, hr: ctx.hr, done: ctx.done }
}

unsafe fn fill_process_data(context: ErrorContext, processes: Obj, pid: u32) -> HRESULT {
    let mut op = Op { file: GETRUNNINGPROCESSES_CPP, func: "FillProcessData", ctx: context, hr: S_OK };
    let mut entry = Com::null();
    'done: {
        let hr = xbtp::create_json_object(entry.put());
        if hr < 0 {
            op.fail(hr, 306, "XbtpCreateJsonObject(&processEntry)");
            break 'done;
        }
        let mut file = None;
        let hr = get_process_image_file_name(pid, &mut file);
        if hr < 0 {
            op.fail(hr, 309, "GetProcessImageFileName(dwProcessId, &strFileName)");
            break 'done;
        }
        let name: Option<String> = match file.as_deref() {
            Some(s) if !s.is_empty() => Some(s.to_string()),
            _ if pid == 0 => Some("Idle".into()),
            _ if pid == 4 => Some("System Process".into()),
            _ => None,
        };
        if let Some(n) = name {
            let hr = set_string_text(entry.get(), "ProcessName", &n);
            if hr < 0 {
                op.fail(hr, 341, "processEntry->SetString(ProcessNameProperty, pszFileName)");
                break 'done;
            }
        }
        let n = w("ProcessId");
        let hr = set_json_u32(entry.get(), n.as_ptr(), pid);
        if hr < 0 {
            op.fail(hr, 343, "XtfSetJsonProperty(processEntry, PidProperty, dwProcessId)");
            break 'done;
        }
        let hr = array::append_object(processes, entry.get());
        if hr < 0 {
            op.fail(hr, 344, "processes->AppendObject(processEntry)");
            break 'done;
        }
    }
    let hr = op.hr;
    drop(entry);
    drop(op.ctx);
    hr
}

pub unsafe extern "system" fn get_running_processes_handler(_session: usize, _request: usize, reply: usize) -> HRESULT {
    let activity = Activity::start(b"XtfConsoleControl: GetRunningProcessesHandler\0");
    let mut op = Op::new(GETRUNNINGPROCESSES_CPP, "GetRunningProcessesHandler", reply);
    let mut body = Com::null();
    let mut processes = Com::null();
    'done: {
        let pids = hosted_processes();
        let hr = xbtp::create_json_object(body.put());
        if hr < 0 {
            op.fail(hr, 440, "XbtpCreateJsonObject(&replyBody)");
            break 'done;
        }
        let hr = xbtp::create_json_array(processes.put());
        if hr < 0 {
            op.fail(hr, 441, "XbtpCreateJsonArray(&processes)");
            break 'done;
        }
        for pid in pids {
            let copy = copy_context(&op.ctx);
            let hr = fill_process_data(copy, processes.get(), pid);
            if hr < 0 {
                op.fail(hr, 445, "FillProcessData(context, processes, pProcesses[iProcess])");
                break 'done;
            }
        }
        let n = w("Processes");
        let hr = xtfmsg::com::json::set_array(body.get(), n.as_ptr(), processes.get());
        if hr < 0 {
            op.fail(hr, 448, "replyBody->SetArray(ProcessesArrayProperty, processes)");
            break 'done;
        }
        let hr = serialize_end_body(body.get(), reply as Obj);
        if hr < 0 {
            op.fail(hr, 449, "XtfSerializeJsonBody(replyBody, reply)");
            break 'done;
        }
    }
    if op.hr < 0 {
        op.xce_empty();
    }
    drop(processes);
    drop(body);
    op.finish(activity);
    S_OK
}

pub fn wtoi(s: &str) -> i32 {
    let mut it = s.chars().peekable();
    while matches!(it.peek(), Some(c) if *c == ' ' || ('\t'..='\r').contains(c)) {
        it.next();
    }
    let neg = match it.peek() {
        Some('-') => {
            it.next();
            true
        }
        Some('+') => {
            it.next();
            false
        }
        _ => false,
    };
    let mut v: i64 = 0;
    let mut over = false;
    while let Some(c) = it.peek().copied() {
        let Some(d) = c.to_digit(10) else { break };
        it.next();
        v = v * 10 + d as i64;
        if v > i32::MAX as i64 + 1 {
            over = true;
            v = i32::MAX as i64 + 1;
        }
    }
    if neg {
        if over || v > i32::MAX as i64 + 1 {
            i32::MIN
        } else {
            (-v) as i32
        }
    } else if v > i32::MAX as i64 {
        i32::MAX
    } else {
        v as i32
    }
}

pub fn dump_kind(dump_type: i32) -> Option<(u32, u32, &'static str)> {
    match dump_type {
        0 => Some(DUMP_TRIAGE),
        1 => Some(DUMP_MINI),
        2 => Some(DUMP_HEAP),
        _ => None,
    }
}

pub fn dump_file_name(image: Option<&str>, t: (u16, u16, u16, u16, u16), kind: &str) -> Result<String, HRESULT> {
    let s = format!("D:\\{}_{}-{}-{}-{}-{}-{}.dmp", image.unwrap_or("(null)"), t.0, t.1, t.2, t.3, t.4, kind);
    if s.encode_utf16().count() >= MAX_PATH {
        return Err(STRSAFE_E_INSUFFICIENT_BUFFER);
    }
    Ok(s)
}

pub fn ensure_console_drives() {
    for d in ['D', 'T'] {
        let _ = std::fs::create_dir_all(console_sandbox::fs::drive_dir(d));
    }
}

pub fn host_of_console(console: &str) -> Option<PathBuf> {
    console_sandbox::fs::resolve(console).map(|(_, h)| h)
}

pub const ERROR_INVALID_NAME: u32 = 123;

pub fn dump_host_path(console: &str) -> Option<PathBuf> {
    let host = host_of_console(console)?;
    let root = console_sandbox::fs::drive_dir('D');
    let rel = host.strip_prefix(&root).ok()?;
    if rel.components().all(|c| matches!(c, std::path::Component::Normal(_))) && rel.components().count() > 0 {
        Some(host)
    } else {
        None
    }
}

unsafe fn open_console_process(access: u32, pid: u32) -> HANDLE {
    if pid != GetCurrentProcessId() {
        SetLastError(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }
    OpenProcess(access, 0, pid)
}

unsafe fn notify(op: u32, func: &str, open_line: u32, send_line: u32, send_expr: &str) {
    let mut session = Com::null();
    let hr = xtfmsg::api::open_pipe_to_system(&CONSOLE_EXTENSION_ID, &IID_XBTP_CLIENT_SESSION, session.put());
    chk(CRASHDUMP_CPP, func, open_line, "XtfOpenPipeToSystem(ConsoleExtensionId, IID_PPV_ARGS(&session))", hr, "");
    if hr < 0 {
        return;
    }
    let mut response = Com::null();
    let hr = xtfmsg::api::send_json_message(session.get(), core::ptr::null_mut(), op, response.put());
    chk(CRASHDUMP_CPP, func, send_line, send_expr, hr, "");
}

pub unsafe fn notify_crash_dump_start() {
    notify(crate::OP_NOTIFY_CRASH_DUMP_START, "XtfNotifyCrashDumpStart", 503, 504, "XtfSendJsonMessage(session, NULL, NotifyCrashDumpStartOperationId, &response)");
}

pub unsafe fn notify_crash_dump_finish() {
    notify(crate::OP_NOTIFY_CRASH_DUMP_FINISH, "XtfNotifyCrashDumpFinish", 517, 518, "XtfSendJsonMessage(session, NULL, NotifyCrashDumpFinishOperationId, &response)");
}

fn last_error_hr() -> HRESULT {
    hr_from_win32(unsafe { GetLastError() })
}

pub unsafe extern "system" fn generate_stack_dump_handler(_session: usize, request: usize, reply: usize) -> HRESULT {
    let activity = Activity::start(b"XtfConsoleControl: GenerateStackDump\0");
    let mut op = Op::new(CRASHDUMP_CPP, "GenerateStackDump", reply);
    let mut req = Com::null();
    let mut body = Com::null();
    let mut pid_s = Bstr::null();
    let mut type_s = Bstr::null();
    let mut image_s = Bstr::null();
    let mut file: HANDLE = INVALID_HANDLE_VALUE;
    let mut process: HANDLE = core::ptr::null_mut();
    'done: {
        let hr = deserialize_body(request as Obj, req.put());
        if hr < 0 {
            op.fail(hr, 552, "XtfDeserializeJsonBody(received, &reqBody)");
            break 'done;
        }
        let hr = get_string(req.get(), "ProcessId", &mut pid_s);
        if hr < 0 {
            op.fail(hr, 553, "reqBody->GetString(ProcessIdERAProperty, &ProcessIdString)");
            break 'done;
        }
        let hr = get_string(req.get(), "DumpType", &mut type_s);
        if hr < 0 {
            op.fail(hr, 554, "reqBody->GetString(DumpTypeProperty, &DumpTypeString)");
            break 'done;
        }
        let hr = get_string(req.get(), "ImageName", &mut image_s);
        if hr < 0 {
            op.fail(hr, 555, "reqBody->GetString(ExecutableImageNameProperty, &imageNameString)");
            break 'done;
        }
        let (Some(pid_text), Some(type_text)) = (pid_s.text(), type_s.text()) else {
            op.fail(E_INVALIDARG, 558, "E_INVALIDARG");
            break 'done;
        };
        let pid = wtoi(&pid_text);
        let dump_type = wtoi(&type_text);
        if pid == 0 {
            op.fail(E_INVALIDARG, 567, "E_INVALIDARG");
            break 'done;
        }
        let Some((kind, access, kind_name)) = dump_kind(dump_type) else {
            op.fail(E_INVALIDARG, 610, "E_INVALIDARG");
            break 'done;
        };
        let image = image_s.text();
        let name = match dump_file_name(image.as_deref(), crate::system::system_time_fields(), kind_name) {
            Ok(n) => n,
            Err(hr) => {
                op.fail(hr, 623, "StringCchPrintf(wzFileName, ARRAYSIZE(wzFileName), L\"D:\\\\%s_%u-%u-%u-%u-%u-%s.dmp\", imageNameString, SystemTime.wYear, SystemTime.wMonth, SystemTime.wDay, SystemTime.wHour, SystemTime.wMinute, pcwzDumpType)");
                break 'done;
            }
        };
        ensure_console_drives();
        let wname = w(&name);
        file = if dump_host_path(&name).is_some() {
            console_sandbox::fs::create_file_w(wname.as_ptr(), 0xC000_0000, 0, core::ptr::null(), 2, 0x80, 0) as HANDLE
        } else {
            SetLastError(ERROR_INVALID_NAME);
            INVALID_HANDLE_VALUE
        };
        if file == INVALID_HANDLE_VALUE {
            let hr = last_error_hr();
            if hr < 0 {
                op.fail(hr, 636, "HRESULT_FROM_WIN32(GetLastError())");
                break 'done;
            }
        }
        let hr = xbtp::create_json_object(body.put());
        if hr < 0 {
            op.fail(hr, 639, "XbtpCreateJsonObject(&replyBody)");
            break 'done;
        }
        process = open_console_process(access, pid as u32);
        if process.is_null() {
            let hr = last_error_hr();
            if hr < 0 {
                op.fail(hr, 644, "HRESULT_FROM_WIN32(GetLastError())");
                break 'done;
            }
        }
        notify_crash_dump_start();
        'dump: {
            if mini_dump_write_dump(process, pid as u32, file, kind) == 0 {
                let hr = last_error_hr();
                if hr < 0 {
                    op.fail(hr, 660, "HRESULT_FROM_WIN32(GetLastError())");
                    break 'dump;
                }
            }
            if FlushFileBuffers(file) == 0 {
                let hr = last_error_hr();
                if hr < 0 {
                    op.fail(hr, 665, "HRESULT_FROM_WIN32(GetLastError())");
                    break 'dump;
                }
            }
            let hr = set_string(body.get(), "FilePath", wname.as_ptr());
            if hr < 0 {
                op.fail(hr, 668, "replyBody->SetString(DumpFilePathProperty, wzFileName)");
                break 'dump;
            }
        }
        notify_crash_dump_finish();
    }
    if file != INVALID_HANDLE_VALUE && !file.is_null() {
        CloseHandle(file);
    }
    if !process.is_null() {
        CloseHandle(process);
    }
    let _ = serialize_end_body(body.get(), reply as Obj);
    op.complete(reply, body.get());
    drop(req);
    let hr = op.hr;
    drop(op.ctx);
    drop(body);
    activity.stop();
    hr
}
