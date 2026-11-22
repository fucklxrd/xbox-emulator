use crate::op::*;
use core::ffi::c_void;
use xtfmsg::chk::Activity;
use xtfmsg::com::{Com, Obj};
use xtfmsg::msg::{deserialize_body, get_json_u32, serialize_end_body, set_json_i32, set_json_u32};
use xtfmsg::sys::*;
use xtfmsg::xbtp;

pub const VIDEOIMPACTED_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\videoimpactedbyscreenshot.cpp";
pub const SCREENSHOT_DIR: &str = "T:\\XTF";
pub const SCREENSHOT_SOURCE_FILE: &str = "T:\\XTF\\ScreenshotSourceIp.txt";
pub const E_POINTER: HRESULT = 0x8000_4003u32 as i32;
pub const ERROR_ALREADY_EXISTS: u32 = 183;
pub const LINE_CHARS: usize = 0x400;

const PRINTF_LEGACY_WIDE_SPECIFIERS: u64 = 4;

extern "C" {
    fn _wfopen_s(f: *mut *mut c_void, name: *const u16, mode: *const u16) -> i32;
    fn fgetws(buf: *mut u16, n: i32, f: *mut c_void) -> *mut u16;
    fn fclose(f: *mut c_void) -> i32;
    fn __stdio_common_vfwprintf(options: u64, f: *mut c_void, fmt: *const u16, locale: *mut c_void, args: *const usize) -> i32;
}

#[repr(C)]
struct FindData {
    attributes: u32,
    creation: [u32; 2],
    access: [u32; 2],
    write: [u32; 2],
    size_high: u32,
    size_low: u32,
    reserved: [u32; 2],
    name: [u16; 260],
    alt: [u16; 14],
    tail: [u32; 4],
}

#[link(name = "kernel32")]
extern "system" {
    fn FindFirstFileExW(name: *const u16, level: u32, data: *mut FindData, op: u32, filter: *mut c_void, flags: u32) -> HANDLE;
    fn FindClose(h: HANDLE) -> i32;
    fn GetSystemTimeAsFileTime(ft: *mut u64);
    fn CreateDirectoryW(name: *const u16, sa: *const c_void) -> i32;
}

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

pub fn host_file(console: &str) -> Vec<u16> {
    let h = crate::process::host_of_console(console).expect("console drive path");
    w(&h.to_string_lossy())
}

fn errno_hr(e: i32) -> HRESULT {
    hr_from_win32(e as u32)
}

fn last_error_or_fail() -> HRESULT {
    let e = unsafe { GetLastError() };
    if e == 0 {
        E_FAIL
    } else {
        hr_from_win32(e)
    }
}

pub unsafe extern "system" fn mark_screenshot_taken_handler(_session: usize, request: usize, reply: usize) -> HRESULT {
    let activity = Activity::start(b"XtfConsoleControl: MarkScreenshotTakenHandler\0");
    if request == 0 || reply == 0 {
        activity.stop();
        return E_POINTER;
    }
    let mut op = Op::new(VIDEOIMPACTED_CPP, "MarkScreenshotTakenHandler", reply);
    let mut body = Com::null();
    let mut req = Com::null();
    let mut source = Bstr::null();
    'done: {
        let hr = xbtp::create_json_object(body.put());
        if hr < 0 {
            op.fail(hr, 28, "XbtpCreateJsonObject(&replyBody)");
            break 'done;
        }
        let hr = deserialize_body(request as Obj, req.put());
        if hr < 0 {
            op.fail(hr, 29, "XtfDeserializeJsonBody(pReceived, &reqBody)");
            break 'done;
        }
        let hr = get_string(req.get(), "SourceNameInitiatingScreenshot", &mut source);
        if hr < 0 {
            op.fail(hr, 30, "reqBody->GetString(SourceNameInitiatingScreenshotProperty, &sourceAddress)");
            break 'done;
        }
        crate::process::ensure_console_drives();
        let dir = host_file(SCREENSHOT_DIR);
        if CreateDirectoryW(dir.as_ptr(), core::ptr::null()) == 0 {
            let hr = hr_from_win32(GetLastError());
            if hr != hr_from_win32(ERROR_ALREADY_EXISTS) && hr < 0 {
                op.fail(hr, 41, "hr");
                break 'done;
            }
        }
        let path = host_file(SCREENSHOT_SOURCE_FILE);
        let mode = w("w");
        let mut f: *mut c_void = core::ptr::null_mut();
        let e = _wfopen_s(&mut f, path.as_ptr(), mode.as_ptr());
        let hr = errno_hr(e);
        op.check(hr, 45, "_wfopen_s(&f, ScreenshotSourceFilepath, L\"w\")");
        if e != 0 {
            break 'done;
        }
        let fmt = w("%ws");
        let args = [source.ptr() as usize];
        let n = __stdio_common_vfwprintf(PRINTF_LEGACY_WIDE_SPECIFIERS, f, fmt.as_ptr(), core::ptr::null_mut(), args.as_ptr());
        if n <= 0 {
            op.hr = last_error_or_fail();
            fclose(f);
            break 'done;
        }
        let e = fclose(f);
        op.check(errno_hr(e), 47, "fclose(f)");
    }
    if op.hr < 0 {
        op.xce_empty();
    }
    op.complete(reply, body.get());
    drop(req);
    drop(source);
    let hr = op.hr;
    drop(op.ctx);
    drop(body);
    activity.stop();
    hr
}

pub fn seconds_since(now: u64, written: u64) -> u64 {
    now.wrapping_sub(written) / 10_000_000
}

pub unsafe extern "system" fn is_video_impacted_by_screenshots_handler(_session: usize, request: usize, reply: usize) -> HRESULT {
    let activity = Activity::start(b"XtfConsoleControl: IsVideoImpactedByScreenshotsHandler\0");
    if request == 0 || reply == 0 {
        activity.stop();
        return E_POINTER;
    }
    let mut op = Op::new(VIDEOIMPACTED_CPP, "IsVideoImpactedByScreenshotsHandler", reply);
    let mut body = Com::null();
    let mut req = Com::null();
    let mut source = Bstr::null();
    'done: {
        let hr = xbtp::create_json_object(body.put());
        if hr < 0 {
            op.fail(hr, 85, "XbtpCreateJsonObject(&replyBody)");
            break 'done;
        }
        let hr = deserialize_body(request as Obj, req.put());
        if hr < 0 {
            op.fail(hr, 86, "XtfDeserializeJsonBody(pReceived, &reqBody)");
            break 'done;
        }
        let mut duration = 0u32;
        let n = w("DurationInSeconds");
        let hr = get_json_u32(req.get(), n.as_ptr(), &mut duration);
        if hr < 0 {
            op.fail(hr, 87, "XtfGetJsonProperty(reqBody, DurationInSecondsProperty, &durationInSeconds)");
            break 'done;
        }
        let mut impacted = 0i32;
        let mut seconds = 0u64;
        let path = host_file(SCREENSHOT_SOURCE_FILE);
        let mut fd: FindData = core::mem::zeroed();
        let h = FindFirstFileExW(path.as_ptr(), 1, &mut fd, 0, core::ptr::null_mut(), 0);
        'read: {
            if h == INVALID_HANDLE_VALUE {
                let e = GetLastError();
                if e == 2 || e == 3 {
                    break 'read;
                }
                let hr = hr_from_win32(e);
                op.check(hr, 98, "result");
                if e != 0 {
                    break 'done;
                }
                break 'read;
            }
            let written = ((fd.write[1] as u64) << 32) | fd.write[0] as u64;
            FindClose(h);
            let mut now = 0u64;
            GetSystemTimeAsFileTime(&mut now);
            seconds = seconds_since(now, written);
            if seconds > duration as u64 {
                break 'read;
            }
            impacted = 1;
            let mode = w("r");
            let mut f: *mut c_void = core::ptr::null_mut();
            let e = _wfopen_s(&mut f, path.as_ptr(), mode.as_ptr());
            op.check(errno_hr(e), 123, "_wfopen_s(&f, ScreenshotSourceFilepath, L\"r\")");
            if e != 0 {
                break 'done;
            }
            let mut line = vec![0u16; LINE_CHARS];
            if fgetws(line.as_mut_ptr(), LINE_CHARS as i32, f).is_null() {
                op.hr = last_error_or_fail();
                fclose(f);
                break 'done;
            }
            let e = fclose(f);
            op.check(errno_hr(e), 125, "fclose(f)");
            if e != 0 {
                break 'done;
            }
            if line[0] != 0 {
                let b = SysAllocString(line.as_ptr());
                if b.is_null() {
                    op.fail(E_OUTOFMEMORY, 127, "sourceAddress.Set(host)");
                    break 'done;
                }
                source = Bstr(b);
            }
        }
        let n = w("IsImpacted");
        let hr = set_json_i32(body.get(), n.as_ptr(), impacted);
        if hr < 0 {
            op.fail(hr, 131, "XtfSetJsonProperty(replyBody, IsVideoImpactedByScreenshotProperty, fIsImpacted)");
            break 'done;
        }
        let n = w("SecondsSinceLastScreenshot");
        let hr = set_json_u32(body.get(), n.as_ptr(), seconds as u32);
        if hr < 0 {
            op.fail(hr, 132, "XtfSetJsonProperty(replyBody, SecondsSinceLastScreenshotProperty, (UINT32)secondsSinceLastScreenshot)");
            break 'done;
        }
        let hr = set_string(body.get(), "SourceNameInitiatingScreenshot", source.ptr());
        if hr < 0 {
            op.fail(hr, 133, "replyBody->SetString(SourceNameInitiatingScreenshotProperty, sourceAddress.Ptr())");
            break 'done;
        }
        let hr = serialize_end_body(body.get(), reply as Obj);
        if hr < 0 {
            op.fail(hr, 134, "XtfSerializeJsonBody(replyBody, pReply)");
            break 'done;
        }
    }
    if op.hr < 0 {
        op.xce_empty();
    }
    op.complete(reply, body.get());
    drop(req);
    drop(source);
    let hr = op.hr;
    drop(op.ctx);
    drop(body);
    activity.stop();
    hr
}
