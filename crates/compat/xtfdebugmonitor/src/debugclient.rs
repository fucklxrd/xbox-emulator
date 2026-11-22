use crate::chk::chk;
use crate::monitor::*;
use crate::win::*;
use std::sync::atomic::Ordering;
use xtfmsg::com::{json, Com, Obj};
use xtfmsg::sys::*;
use xtfmsg::xbtp;

pub const DEBUGCLIENT_CPP: &str = "xbox\\devex\\xtf\\dll\\debugmonitor\\debugclient.cpp";

pub const FILTER_BREAK_ON_EXCEPTION: u32 = 0x1;
pub const FILTER_EXCEPTION: u32 = 0x2;
pub const FILTER_CREATE_THREAD: u32 = 0x4;
pub const FILTER_CREATE_PROCESS: u32 = 0x8;
pub const FILTER_EXIT_THREAD: u32 = 0x10;
pub const FILTER_EXIT_PROCESS: u32 = 0x20;
pub const FILTER_LOAD_DLL: u32 = 0x40;
pub const FILTER_UNLOAD_DLL: u32 = 0x80;
pub const FILTER_OUTPUT_DEBUG_STRING: u32 = 0x100;
pub const FILTER_RIP: u32 = 0x200;

struct Num {
    name: &'static str,
    value: f64,
    line: u32,
    expr: &'static str,
}

struct Text {
    wide: bool,
    name: &'static str,
    set_line: u32,
    ansi_line: u32,
    string_line: u32,
    set_expr: &'static str,
    ansi_expr: &'static str,
    string_expr: &'static str,
}

struct Shape {
    gate: u32,
    func: &'static str,
    create_line: u32,
    send_line: u32,
}

const EVENT_CODE: &str = "body->SetNumber(EventCodeProperty, entry.eventCode)";
const PROCESS_ID: &str = "body->SetNumber(ProcessIdProperty, entry.pid)";
const THREAD_ID: &str = "body->SetNumber(ThreadIdProperty, entry.tid)";
const ADDRESS: &str = "XtfSetJsonProperty64(body, AddressProperty, entry.address)";
const SEND_MORE: &str = "XtfSendMoreJsonMessage(m_pSession, body)";
const CREATE_BODY: &str = "XbtpCreateJsonObject(&body)";

fn head(e: &Entry, base: u32) -> [Num; 3] {
    [
        Num { name: "EventCode", value: e.code as f64, line: base + 1, expr: EVENT_CODE },
        Num { name: "ProcessId", value: e.pid as f64, line: base + 2, expr: PROCESS_ID },
        Num { name: "ThreadId", value: e.tid as f64, line: base + 3, expr: THREAD_ID },
    ]
}

unsafe fn bstr_wide(text: &[u16]) -> Result<BSTR, HRESULT> {
    if text.is_empty() {
        return Ok(core::ptr::null_mut());
    }
    let mut z = text.to_vec();
    z.push(0);
    let b = SysAllocString(z.as_ptr());
    if b.is_null() {
        return Err(E_OUTOFMEMORY);
    }
    Ok(b)
}

pub unsafe fn bstr_ansi(text: &[u8]) -> Result<BSTR, HRESULT> {
    if text.is_empty() {
        return Ok(core::ptr::null_mut());
    }
    let mut z = text.to_vec();
    z.push(0);
    let n = MultiByteToWideChar(0, 0, z.as_ptr(), -1, core::ptr::null_mut(), 0);
    if n == 0 {
        let hr = hr_last_error();
        if hr < 0 {
            return Err(hr);
        }
        return Err(E_OUTOFMEMORY);
    }
    let b = SysAllocStringLen(core::ptr::null(), (n - 1) as u32);
    if b.is_null() {
        return Err(E_OUTOFMEMORY);
    }
    if MultiByteToWideChar(0, 0, z.as_ptr(), -1, b, n) == 0 {
        *b = 0;
        let hr = hr_last_error();
        if hr < 0 {
            SysFreeString(b);
            return Err(hr);
        }
    }
    Ok(b)
}

struct Bstr(BSTR);

impl Drop for Bstr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { SysFreeString(self.0) };
        }
    }
}

impl DebugClient {
    fn chk(&self, func: &str, line: u32, expr: &str, hr: HRESULT) {
        chk(DEBUGCLIENT_CPP, func, line, expr, hr, "");
    }

    unsafe fn send(&self, shape: &Shape, nums: &[Num], address: Option<(u32, u64)>, text: Option<(&Text, &Entry)>) -> HRESULT {
        if shape.gate != 0 && self.filter_flags & shape.gate == 0 {
            return S_OK;
        }
        let mut body = Com::null();
        let hr = xbtp::create_json_object(body.put());
        self.chk(shape.func, shape.create_line, CREATE_BODY, hr);
        if hr < 0 {
            return hr;
        }
        for n in nums {
            let w = wide(n.name);
            let hr = json::set_number(body.get(), w.as_ptr(), n.value);
            self.chk(shape.func, n.line, n.expr, hr);
            if hr < 0 {
                return hr;
            }
        }
        if let Some((line, a)) = address {
            let w = wide("Address");
            let hr = xtfmsg::msg::set_json_property64(body.get(), w.as_ptr(), a);
            self.chk(shape.func, line, ADDRESS, hr);
            if hr < 0 {
                return hr;
            }
        }
        if let Some((t, e)) = text {
            let converted = if t.wide {
                let r = bstr_wide(&e.wide_message());
                self.chk(shape.func, t.set_line, t.set_expr, r.as_ref().err().copied().unwrap_or(S_OK));
                r
            } else {
                let r = bstr_ansi(e.ansi_message());
                self.chk(shape.func, t.ansi_line, t.ansi_expr, r.as_ref().err().copied().unwrap_or(S_OK));
                r
            };
            let s = match converted {
                Ok(b) => Bstr(b),
                Err(hr) => return hr,
            };
            let w = wide(t.name);
            let hr = json::set_string(body.get(), w.as_ptr(), s.0);
            self.chk(shape.func, t.string_line, t.string_expr, hr);
            if hr < 0 {
                return hr;
            }
        }
        let hr = xtfmsg::msg::send_more_json_message(self.session as Obj, body.get());
        self.chk(shape.func, shape.send_line, SEND_MORE, hr);
        hr
    }

    pub unsafe fn send_on_attach(&self, attach_result: HRESULT) -> HRESULT {
        let shape = Shape { gate: 0, func: "DebugClient::SendOnAttach", create_line: 172, send_line: 175 };
        let nums = [
            Num { name: "AttachResult", value: attach_result as f64, line: 173, expr: "body->SetNumber(AttachResultProperty, (double)attachResult)" },
            Num { name: "ClientIdentifier", value: self.session_identifier as f64, line: 174, expr: "body->SetNumber(ClientSessionIdentifierProperty, m_sessionIdentifier)" },
        ];
        self.send(&shape, &nums, None, None)
    }

    pub unsafe fn send_on_exception(&self, e: &Entry) -> HRESULT {
        let shape = Shape { gate: FILTER_EXCEPTION, func: "DebugClient::SendOnExceptionDebugEvent", create_line: 195, send_line: 203 };
        let [a, b, c] = head(e, 195);
        let nums = [
            a,
            b,
            c,
            Num { name: "ExceptionCode", value: e.data1 as f64, line: 199, expr: "body->SetNumber(ExceptionCodeProperty, entry.data1)" },
            Num { name: "ExceptionFirstChance", value: e.data2 as f64, line: 200, expr: "body->SetNumber(ExceptionFirstChanceProperty, entry.data2)" },
            Num { name: "ContinueRequested", value: e.data3 as f64, line: 201, expr: "body->SetNumber(ContinueRequestedProperty, entry.data3)" },
        ];
        self.send(&shape, &nums, Some((202, e.address)), None)
    }

    pub unsafe fn send_on_create_thread(&self, e: &Entry) -> HRESULT {
        let shape = Shape { gate: FILTER_CREATE_THREAD, func: "DebugClient::SendOnCreateThreadDebugEvent", create_line: 223, send_line: 228 };
        self.send(&shape, &head(e, 223), Some((227, e.address)), None)
    }

    pub unsafe fn send_on_create_process(&self, e: &Entry) -> HRESULT {
        let shape = Shape { gate: FILTER_CREATE_PROCESS, func: "DebugClient::SendOnCreateProcessDebugEvent", create_line: 248, send_line: 253 };
        self.send(&shape, &head(e, 248), Some((252, e.address)), None)
    }

    unsafe fn send_exit(&self, e: &Entry, gate: u32, func: &'static str, base: u32) -> HRESULT {
        let shape = Shape { gate, func, create_line: base, send_line: base + 5 };
        let [a, b, c] = head(e, base);
        let nums = [a, b, c, Num { name: "ExitCode", value: e.data1 as f64, line: base + 4, expr: "body->SetNumber(ExitCodeProperty, entry.data1)" }];
        self.send(&shape, &nums, None, None)
    }

    pub unsafe fn send_on_exit_thread(&self, e: &Entry) -> HRESULT {
        self.send_exit(e, FILTER_EXIT_THREAD, "DebugClient::SendOnExitThreadDebugEvent", 273)
    }

    pub unsafe fn send_on_exit_process(&self, e: &Entry) -> HRESULT {
        self.send_exit(e, FILTER_EXIT_PROCESS, "DebugClient::SendOnExitProcessDebugEvent", 298)
    }

    pub unsafe fn send_on_load_dll(&self, e: &Entry) -> HRESULT {
        let shape = Shape { gate: FILTER_LOAD_DLL, func: "DebugClient::SendOnLoadDllDebugEvent", create_line: 324, send_line: 341 };
        let text = Text {
            wide: e.data1 != 0,
            name: "Message",
            set_line: 333,
            ansi_line: 337,
            string_line: 340,
            set_expr: "strConverted.Set(reinterpret_cast<LPCWSTR>(entry.message))",
            ansi_expr: "strConverted.SetAnsi(reinterpret_cast<LPCSTR>(entry.message))",
            string_expr: "body->SetString(MessageProperty, strConverted)",
        };
        self.send(&shape, &head(e, 324), Some((328, e.address)), Some((&text, e)))
    }

    pub unsafe fn send_on_unload_dll(&self, e: &Entry) -> HRESULT {
        let shape = Shape { gate: FILTER_UNLOAD_DLL, func: "DebugClient::SendOnUnloadDllDebugEvent", create_line: 361, send_line: 366 };
        self.send(&shape, &head(e, 361), Some((365, e.address)), None)
    }

    pub unsafe fn send_on_output_debug_string(&self, e: &Entry) -> HRESULT {
        let shape = Shape { gate: FILTER_OUTPUT_DEBUG_STRING, func: "DebugClient::SendOnOutputDebugStringEvent", create_line: 387, send_line: 403 };
        let text = Text {
            wide: e.data1 != 0,
            name: "DebugString",
            set_line: 395,
            ansi_line: 399,
            string_line: 402,
            set_expr: "strConverted.Set(reinterpret_cast<LPCWSTR>(entry.message))",
            ansi_expr: "strConverted.SetAnsi(reinterpret_cast<LPCSTR>(entry.message))",
            string_expr: "body->SetString(DebugStringProperty, strConverted)",
        };
        self.send(&shape, &head(e, 387), None, Some((&text, e)))
    }

    pub unsafe fn send_on_rip(&self, e: &Entry) -> HRESULT {
        let shape = Shape { gate: FILTER_RIP, func: "DebugClient::SendOnRipEvent", create_line: 423, send_line: 429 };
        let [a, b, c] = head(e, 423);
        let nums = [
            a,
            b,
            c,
            Num { name: "ExceptionCode", value: e.data1 as f64, line: 427, expr: "body->SetNumber(ExceptionCodeProperty, entry.data1)" },
            Num { name: "ExitType", value: e.data2 as f64, line: 428, expr: "body->SetNumber(ExitTypeProperty, entry.data2)" },
        ];
        self.send(&shape, &nums, None, None)
    }

    pub unsafe fn process_next_event(&self) -> HRESULT {
        let idx = (self.read_head.load(Ordering::SeqCst) & 0xff) as usize;
        let e = lock(&self.monitor.ring[idx]).clone();
        let f = "DebugClient::ProcessNextEvent";
        let mut hr = S_OK;
        let r = match e.code {
            EXCEPTION_DEBUG_EVENT => Some((120, "SendOnExceptionDebugEvent(entry)", self.send_on_exception(&e))),
            CREATE_THREAD_DEBUG_EVENT => Some((124, "SendOnCreateThreadDebugEvent(entry)", self.send_on_create_thread(&e))),
            CREATE_PROCESS_DEBUG_EVENT => Some((128, "SendOnCreateProcessDebugEvent(entry)", self.send_on_create_process(&e))),
            EXIT_THREAD_DEBUG_EVENT => Some((132, "SendOnExitThreadDebugEvent(entry)", self.send_on_exit_thread(&e))),
            EXIT_PROCESS_DEBUG_EVENT => Some((136, "SendOnExitProcessDebugEvent(entry)", self.send_on_exit_process(&e))),
            LOAD_DLL_DEBUG_EVENT => Some((140, "SendOnLoadDllDebugEvent(entry)", self.send_on_load_dll(&e))),
            UNLOAD_DLL_DEBUG_EVENT => Some((144, "SendOnUnloadDllDebugEvent(entry)", self.send_on_unload_dll(&e))),
            OUTPUT_DEBUG_STRING_EVENT => Some((148, "SendOnOutputDebugStringEvent(entry)", self.send_on_output_debug_string(&e))),
            RIP_EVENT => Some((152, "SendOnRipEvent(entry)", self.send_on_rip(&e))),
            _ => None,
        };
        if let Some((line, expr, r)) = r {
            hr = r;
            self.chk(f, line, expr, hr);
        }
        self.read_head.store(((idx + 1) & 0xff) as u32, Ordering::SeqCst);
        hr
    }

    pub unsafe fn monitor(&self) -> HRESULT {
        let f = "DebugClient::Monitor";
        let mut hr = S_OK;
        let mut sent_attach = false;
        if self.init_result < 0 {
            self.send_on_attach(self.init_result);
            chk(DEBUGCLIENT_CPP, f, 52, "hr", S_OK, "");
        }
        self.monitoring.store(true, Ordering::SeqCst);
        let m = &self.monitor;
        'run: loop {
            if m.attach_done.load(Ordering::SeqCst) {
                if !sent_attach {
                    self.send_on_attach(m.attach_result.load(Ordering::SeqCst));
                    sent_attach = true;
                }
                if !m.active.load(Ordering::SeqCst) {
                    break 'run;
                }
                while self.read_head.load(Ordering::SeqCst) != m.write_head.load(Ordering::SeqCst) {
                    hr = self.process_next_event();
                    self.chk(f, 77, "ProcessNextEvent()", hr);
                    if hr < 0 {
                        break 'run;
                    }
                }
            }
            let handles = [self.stop_event, self.event_ready];
            let w = WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE);
            if w == 0 {
                break 'run;
            }
            if w == 1 {
                continue;
            }
            hr = hr_last_error();
            break 'run;
        }
        self.monitoring.store(false, Ordering::SeqCst);
        hr
    }
}
