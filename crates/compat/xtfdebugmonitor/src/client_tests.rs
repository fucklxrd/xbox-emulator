use crate::client::*;
use crate::tests::{process_object, query, LIVE_LOCK};
use crate::win::INVALID_HANDLE_VALUE;
use crate::*;
use core::ptr::null_mut;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use xtfmsg::com::{self, Com, Obj, IID_XBTP_EXTENSION_METADATA};
use xtfmsg::fixture::load_xbtp;
use xtfmsg::sys::*;

const CHILD_ENV: &str = "XTFDEBUGMONITOR_CHILD";
const CHILD_TEST: &str = "tests::debug_child_main";
const CHILD_TICK: &str = "xtfdebugmonitor child tick";
const TITLE_TCP_PORT: u32 = 0x107d;
const RAW_TCP_PORT: u32 = 48231;

fn guard() -> MutexGuard<'static, ()> {
    LIVE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[repr(C)]
struct DispatcherVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(Obj) -> u32,
    shutdown: unsafe extern "system" fn(Obj) -> HRESULT,
    add_extension: unsafe extern "system" fn(Obj, *const GUID, Obj) -> HRESULT,
}

const IID_XBTP_DISPATCHER: GUID = GUID::new(0xe6358d7e, 0x3b86, 0x425b, [0xb3, 0xd7, 0x3d, 0x21, 0x82, 0xb9, 0x76, 0x6a]);

type StartDispatcherFn = unsafe extern "C" fn(u32, i32, *const GUID, *mut Obj) -> HRESULT;
type OpenSessionEx2Fn = unsafe extern "C" fn(*const u16, u32, *const OpenSessionOptions, *const GUID, *const GUID, *mut Obj) -> HRESULT;

struct Dispatcher(Obj);

impl Dispatcher {
    unsafe fn start(port: u32) -> Dispatcher {
        load_xbtp();
        let f: StartDispatcherFn = core::mem::transmute(xtfmsg::xbtp::proc(b"XbtpStartDispatcherEx\0").unwrap());
        let mut out: Obj = null_mut();
        let hr = f(port, 0, &IID_XBTP_DISPATCHER, &mut out);
        assert_eq!(hr, S_OK, "XbtpStartDispatcherEx on loopback port {port} failed: 0x{:08X}", hr as u32);
        Dispatcher(out)
    }

    unsafe fn vt(&self) -> &DispatcherVtbl {
        &**(self.0 as *const *const DispatcherVtbl)
    }

    unsafe fn add_debug_monitor(&self) {
        let mut md: Obj = null_mut();
        assert_eq!(XtfOrdinal_4(&DEBUG_MONITOR_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
        assert_eq!((self.vt().add_extension)(self.0, &DEBUG_MONITOR_EXTENSION_ID, md), S_OK);
        com::release(md);
    }

    unsafe fn add_ops(&self, ops: &[(u32, xtfmsg::com::metadata::OperationFn)]) {
        let mut md: Obj = null_mut();
        assert_eq!(xtfmsg::xbtp::create_extension_metadata(&IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
        for (id, f) in ops {
            assert_eq!(xtfmsg::com::metadata::add_function(md, *id, Some(*f)), S_OK);
        }
        assert_eq!((self.vt().add_extension)(self.0, &DEBUG_MONITOR_EXTENSION_ID, md), S_OK);
        com::release(md);
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        unsafe {
            (self.vt().shutdown)(self.0);
            (self.vt().release)(self.0);
        }
    }
}

#[repr(C)]
struct Callback {
    vtbl: *const DebugMonitorCallbackVtbl,
    refs: AtomicU32,
    seen: Mutex<Vec<(u32, String)>>,
    result: HRESULT,
}

unsafe extern "system" fn cb_qi(_o: Obj, _iid: *const GUID, _out: *mut Obj) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn cb_add_ref(o: Obj) -> u32 {
    (*(o as *mut Callback)).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn cb_release(o: Obj) -> u32 {
    (*(o as *mut Callback)).refs.fetch_sub(1, Ordering::SeqCst) - 1
}

unsafe extern "system" fn cb_on_string(o: Obj, data: *const DebugStringData) -> HRESULT {
    let c = &*(o as *mut Callback);
    let text = if (*data).debug_string.is_null() { String::new() } else { from_wide((*data).debug_string) };
    c.seen.lock().unwrap().push(((*data).process_id, text));
    c.result
}

static CB_VTBL: DebugMonitorCallbackVtbl = DebugMonitorCallbackVtbl { query_interface: cb_qi, add_ref: cb_add_ref, release: cb_release, on_output_debug_string: cb_on_string };

impl Callback {
    fn new(result: HRESULT) -> Box<Callback> {
        Box::new(Callback { vtbl: &CB_VTBL, refs: AtomicU32::new(1), seen: Mutex::new(Vec::new()), result })
    }
    fn obj(&self) -> Obj {
        self as *const Callback as Obj
    }
    fn seen(&self) -> Vec<(u32, String)> {
        self.seen.lock().unwrap().clone()
    }
}

struct Client(*mut DebugMonitorClient);

impl Client {
    unsafe fn create(address: &str) -> (HRESULT, Client) {
        let a = wide(address);
        let mut out: Obj = null_mut();
        let hr = XtfCreateDebugMonitorClient(a.as_ptr(), &IID_XTF_DEBUG_MONITOR_CLIENT, &mut out);
        (hr, Client(out as *mut DebugMonitorClient))
    }
    unsafe fn vt(&self) -> &DebugMonitorClientVtbl {
        &*(*self.0).vtbl
    }
    unsafe fn start(&self, pid: u32, cb: Obj) -> HRESULT {
        (self.vt().start)(self.0, pid, 0, cb)
    }
    unsafe fn stop(&self, pid: u32) -> HRESULT {
        (self.vt().stop)(self.0, pid)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { (self.vt().release)(self.0) };
        }
    }
}

struct Child_(Child);

impl Child_ {
    fn spawn(mode: &str) -> Child_ {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([CHILD_TEST, "--exact", "--ignored", "--test-threads=1", "-q"])
            .env(CHILD_ENV, mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        assert_ne!(child.id(), std::process::id());
        Child_(child)
    }
    fn pid(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Child_ {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn fields(json: &str) -> Vec<String> {
    let mut v: Vec<String> = json.trim_matches(|c| c == '{' || c == '}').split(',').map(str::to_string).collect();
    v.sort();
    v
}

struct ForceDetach(u32);

impl Drop for ForceDetach {
    fn drop(&mut self) {
        unsafe {
            let req = xtfmsg::fixture::message_with_json(&format!("{{\"ProcessId\":{},\"ForceAll\":1}}", self.0));
            let reply = xtfmsg::fixture::end_message();
            crate::server::detach_process_handler(0, req.get() as usize, reply.get() as usize);
        }
    }
}

#[test]
fn factory_parses_address_and_reports_console_errors() {
    load_xbtp();
    unsafe {
        let (hr, c) = Client::create("");
        assert_eq!(hr, E_NO_IPADDRESS_IN_CONNSTR);
        assert!(c.0.is_null());
        let (hr, c) = Client::create("127.0.0.1/bogus");
        assert_eq!(hr, E_INVALID_VM_TARGET);
        assert!(c.0.is_null());
        let mut out = 0x55usize as Obj;
        assert_eq!(XtfCreateDebugMonitorClient(null_mut(), &IID_XTF_DEBUG_MONITOR_CLIENT, &mut out), E_INVALIDARG);
        assert!(out.is_null());
        let a = wide("127.0.0.1");
        let bad = GUID::new(1, 2, 3, [4; 8]);
        let mut out = 0x55usize as Obj;
        assert_eq!(XtfCreateDebugMonitorClient(a.as_ptr(), &bad, &mut out), E_NOINTERFACE);
        assert!(out.is_null());
        for (addr, ip, port) in [("127.0.0.1", "127.0.0.1", XBTP_PORT_TITLE), ("127.0.0.1/system", "127.0.0.1", XBTP_PORT_SYSTEM), ("127.0.0.1/Host", "127.0.0.1", XBTP_PORT_TITLE), ("127.0.0.1+KEY/title", "127.0.0.1+KEY", XBTP_PORT_TITLE)] {
            let (hr, c) = Client::create(addr);
            assert_eq!(hr, S_OK, "{addr}");
            assert_eq!(from_wide((*c.0).address), ip, "{addr}");
            assert_eq!((*c.0).port, port, "{addr}");
            assert_eq!((*c.0).pump_thread, INVALID_HANDLE_VALUE);
            assert_eq!((*c.0).client_id, 0.0);
        }
        let (_, c) = Client::create("127.0.0.1");
        for iid in [IID_IUNKNOWN, IID_XTF_DEBUG_MONITOR_CLIENT_BASE, IID_XTF_DEBUG_MONITOR_CLIENT] {
            let mut o: Obj = null_mut();
            assert_eq!((c.vt().query_interface)(c.0, &iid, &mut o), S_OK);
            assert_eq!(o, c.0 as Obj);
            assert_eq!((c.vt().release)(c.0), 1);
        }
        let mut o = 0x77usize as Obj;
        assert_eq!((c.vt().query_interface)(c.0, &bad, &mut o), E_NOINTERFACE);
        assert_eq!(o, 0x77usize as Obj);
        assert_eq!(c.start(1234, null_mut()), E_INVALIDARG);
        assert!((*c.0).retrieve_session.is_null());
    }
}

#[test]
fn split_console_address_matches_console_rules() {
    unsafe {
        let run = |s: &str, with_key: bool| -> (HRESULT, String, String, u32) {
            let a = wide(s);
            let mut ip: BSTR = null_mut();
            let mut key: BSTR = null_mut();
            let mut port = 99u32;
            let hr = split_console_address(a.as_ptr(), XBTP_PORT_TITLE, &mut ip, if with_key { &mut key } else { null_mut() }, &mut port);
            let r = (hr, if ip.is_null() { String::new() } else { from_wide(ip) }, if key.is_null() { String::new() } else { from_wide(key) }, port);
            if !ip.is_null() {
                SysFreeString(ip);
            }
            if !key.is_null() {
                SysFreeString(key);
            }
            r
        };
        assert_eq!(run("10.0.0.5+abc/system", true), (S_OK, "10.0.0.5".into(), "abc".into(), XBTP_PORT_SYSTEM));
        assert_eq!(run("10.0.0.5+abc", true), (S_OK, "10.0.0.5".into(), "abc".into(), XBTP_PORT_TITLE));
        assert_eq!(run("10.0.0.5+abc", false), (S_OK, "10.0.0.5+abc".into(), String::new(), XBTP_PORT_TITLE));
        assert_eq!(run("/title", true).0, E_NO_IPADDRESS_IN_CONNSTR);
        assert_eq!(run(&"a".repeat(0x200), true).0, E_NO_IPADDRESS_IN_CONNSTR);
        assert_eq!(run(&"a".repeat(0x1ff), true).0, S_OK);
        assert_eq!(run("h/HOST", true).3, XBTP_PORT_HOST);
        assert_eq!(run("h/x", true), (E_INVALID_VM_TARGET, String::new(), String::new(), 99));
    }
}

#[test]
fn remote_console_address_fails_could_not_connect_without_socket() {
    load_xbtp();
    unsafe {
        let (hr, c) = Client::create("10.255.0.7");
        assert_eq!(hr, S_OK);
        let cb = Callback::new(S_OK);
        let t = Instant::now();
        assert_eq!(c.start(4242, cb.obj()), E_COULD_NOT_CONNECT);
        let spent = t.elapsed();
        assert!(spent >= Duration::from_secs(20) && spent < Duration::from_secs(40), "console open retry loop took {spent:?}");
        assert!((*c.0).pump_thread == INVALID_HANDLE_VALUE);
        assert!(cb.seen().is_empty());
    }
}

static FALLBACK_SEEN: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());

unsafe fn record_op(op: u32, session: usize, received: usize) -> String {
    let text = xtfmsg::fixture::json_text(received as Obj).unwrap_or_default();
    FALLBACK_SEEN.lock().unwrap().push((op, text.clone()));
    let _ = session;
    text
}

unsafe fn send_more(session: usize, body: &str) {
    let m = xtfmsg::fixture::message_with_json(body);
    assert_eq!(com::session::send_more_message(session as Obj, m.get()), S_OK);
}

unsafe extern "system" fn legacy_retrieve(session: usize, received: usize, _reply: usize) -> HRESULT {
    record_op(OP_RETRIEVE_DEBUG_MESSAGES, session, received);
    send_more(session, "{\"DebugString\":\"legacy one\"}");
    send_more(session, "{\"DebugString\":\"legacy two\"}");
    S_OK
}

unsafe extern "system" fn legacy_detach(session: usize, received: usize, _reply: usize) -> HRESULT {
    record_op(crate::server::OP_DETACH_PROCESS, session, received);
    S_OK
}

unsafe extern "system" fn attach_then_bad_event(session: usize, received: usize, _reply: usize) -> HRESULT {
    record_op(crate::server::OP_MONITOR_PROCESS, session, received);
    send_more(session, "{\"AttachResult\":0,\"ClientIdentifier\":7}");
    send_more(session, "{\"EventCode\":3,\"ProcessId\":1}");
    S_OK
}

unsafe extern "system" fn attach_failed(session: usize, received: usize, _reply: usize) -> HRESULT {
    record_op(crate::server::OP_MONITOR_PROCESS, session, received);
    send_more(session, &format!("{{\"AttachResult\":{},\"ClientIdentifier\":3}}", 0x80070005u32 as i32));
    S_OK
}

#[test]
fn legacy_protocol_fallback_and_pump_errors_over_loopback() {
    let _g = guard();
    unsafe {
        FALLBACK_SEEN.lock().unwrap().clear();
        let d = Dispatcher::start(TITLE_TCP_PORT);
        d.add_ops(&[(OP_RETRIEVE_DEBUG_MESSAGES, legacy_retrieve), (crate::server::OP_DETACH_PROCESS, legacy_detach)]);
        let (hr, c) = Client::create("127.0.0.1");
        assert_eq!(hr, S_OK);
        let cb = Callback::new(S_OK);
        assert_eq!(c.start(4321, cb.obj()), S_OK);
        assert_eq!((*c.0).client_id, 0.0);
        let got = wait_for("legacy strings", 20, || {
            let s = cb.seen();
            if s.len() == 2 { Some(s) } else { None }
        });
        assert_eq!(got, vec![(4321, "legacy one".to_string()), (4321, "legacy two".to_string())]);
        assert_eq!(c.stop(4321), S_OK);
        assert_eq!((*c.0).pump_thread, INVALID_HANDLE_VALUE);
        let seen = FALLBACK_SEEN.lock().unwrap().clone();
        assert_eq!(seen, vec![(OP_RETRIEVE_DEBUG_MESSAGES, "{\"ProcessId\":4321}".to_string()), (crate::server::OP_DETACH_PROCESS, "{\"ProcessId\":4321}".to_string())]);
        drop(c);
        drop(d);

        FALLBACK_SEEN.lock().unwrap().clear();
        let d = Dispatcher::start(TITLE_TCP_PORT);
        d.add_ops(&[(crate::server::OP_MONITOR_PROCESS, attach_then_bad_event), (crate::server::OP_DETACH_PROCESS, legacy_detach)]);
        let (_, c) = Client::create("127.0.0.1");
        let cb = Callback::new(S_OK);
        assert_eq!(c.start(55, cb.obj()), S_OK);
        assert_eq!((*c.0).client_id, 7.0);
        let thread = (*c.0).pump_thread;
        wait_for("pump exit on body without DebugString", 20, || (crate::win::WaitForSingleObject(thread, 0) == 0).then_some(()));
        let mut code = 0u32;
        assert_ne!(GetExitCodeThread(thread, &mut code), 0);
        assert_eq!(code, 1);
        assert!(cb.seen().is_empty());
        assert_eq!(c.stop(55), S_OK);
        let seen = FALLBACK_SEEN.lock().unwrap().clone();
        assert_eq!(seen[0], (crate::server::OP_MONITOR_PROCESS, "{\"ProcessId\":55}".to_string()));
        assert_eq!(seen[1].0, crate::server::OP_DETACH_PROCESS);
        assert_eq!(fields(&seen[1].1), vec!["\"ClientIdentifier\":7".to_string(), "\"ProcessId\":55".to_string()]);
        drop(c);
        drop(d);

        let d = Dispatcher::start(TITLE_TCP_PORT);
        d.add_ops(&[(crate::server::OP_MONITOR_PROCESS, attach_failed)]);
        let (_, c) = Client::create("127.0.0.1");
        let cb = Callback::new(S_OK);
        assert_eq!(c.start(56, cb.obj()), 0x80070005u32 as i32);
        assert_eq!((*c.0).client_id, 3.0);
        assert_eq!((*c.0).pump_thread, INVALID_HANDLE_VALUE);
        drop(c);
        drop(d);

        let d = Dispatcher::start(TITLE_TCP_PORT);
        d.add_ops(&[(crate::server::OP_DETACH_PROCESS, legacy_detach)]);
        let (_, c) = Client::create("127.0.0.1");
        let cb = Callback::new(S_OK);
        assert_eq!(c.start(57, cb.obj()), S_OK);
        let thread = (*c.0).pump_thread;
        wait_for("pump exit on undefined legacy op", 20, || (crate::win::WaitForSingleObject(thread, 0) == 0).then_some(()));
        let mut code = 0u32;
        assert_ne!(GetExitCodeThread(thread, &mut code), 0);
        assert_eq!(code, 1);
        assert_eq!(c.stop(57), S_OK);
        drop(c);
        drop(d);
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn GetExitCodeThread(h: HANDLE, code: *mut u32) -> BOOL;
}

#[test]
fn client_end_to_end_streams_debug_strings_from_spawned_child() {
    let _g = guard();
    unsafe {
        let d = Dispatcher::start(TITLE_TCP_PORT);
        d.add_debug_monitor();
        let child = Child_::spawn("ods");
        let pid = child.pid();
        let _detach = ForceDetach(pid);
        let (hr, c) = Client::create("127.0.0.1");
        assert_eq!(hr, S_OK);
        let cb = Callback::new(S_OK);
        assert_eq!(c.start(pid, cb.obj()), S_OK, "Start against spawned child {pid}");
        assert!((*c.0).client_id >= 1.0);
        assert_ne!((*c.0).pump_thread, INVALID_HANDLE_VALUE);
        let got = wait_for("child debug strings through the client", 30, || {
            let s = cb.seen();
            if s.iter().filter(|(_, t)| t.contains(CHILD_TICK)).count() >= 2 { Some(s) } else { None }
        });
        assert!(got.iter().all(|(p, _)| *p == pid));
        let q = query();
        let entry = process_object(&q, pid).expect("debug monitor lists the child");
        assert!(entry.contains("\"IsActive\":1"), "{entry}");
        assert_eq!(c.stop(pid), S_OK);
        assert_eq!((*c.0).pump_thread, INVALID_HANDLE_VALUE);
        wait_for("worker exit after Stop", 20, || {
            let q = query();
            process_object(&q, pid).filter(|e| e.contains("\"IsActive\":0")).map(|_| ())
        });
        assert!(child.0.id() == pid);
        let mut ch = child;
        assert!(ch.0.try_wait().unwrap().is_none(), "child must survive DebugActiveProcessStop");
        drop(c);
        drop(d);
    }
}

unsafe fn raw_open(port: u32) -> Com {
    let f: OpenSessionEx2Fn = core::mem::transmute(xtfmsg::xbtp::proc(b"XbtpOpenSessionEx2\0").unwrap());
    let mut o = OpenSessionOptions { size: 0x58, session_key: [0; 0x20], skip_ready_check: 0, allow_unneeded_key: 0, quiet: 1, reserved: 0, timeout_ms: 5000 };
    o.quiet = 1;
    let host = wide("127.0.0.1");
    let mut s = Com::null();
    assert_eq!(f(host.as_ptr(), port, &o, &DEBUG_MONITOR_EXTENSION_ID, &xtfmsg::com::IID_XBTP_CLIENT_SESSION, s.put()), S_OK);
    s
}

#[test]
fn raw_session_monitor_process_reports_create_and_exit_of_spawned_child() {
    let _g = guard();
    unsafe {
        let d = Dispatcher::start(RAW_TCP_PORT);
        d.add_debug_monitor();
        let mut child = Child_::spawn("sleep");
        let pid = child.pid();
        let _detach = ForceDetach(pid);
        let s = raw_open(RAW_TCP_PORT);
        let flags = crate::debugclient::FILTER_CREATE_PROCESS | crate::debugclient::FILTER_EXIT_PROCESS;
        let req = xtfmsg::fixture::message_with_json(&format!("{{\"ProcessId\":{pid},\"FilterFlags\":{flags}}}"));
        let mut ex = 0u32;
        assert_eq!(com::client_session::send_begin_message(s.get(), crate::server::OP_MONITOR_PROCESS, req.get(), &mut ex), S_OK);
        let mut msgs: Vec<String> = Vec::new();
        let next = |what: &str| -> (i16, i32, String) {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let mut t = 0i16;
                let mut st = 0i32;
                let mut m = Com::null();
                let hr = session_receive_message(s.get(), 500, ex, &mut t, &mut st, m.put());
                if hr == E_PENDING {
                    assert!(Instant::now() < deadline, "timed out waiting for {what}");
                    continue;
                }
                assert_eq!(hr, S_OK, "{what}");
                let text = if m.is_null() { String::new() } else { xtfmsg::fixture::json_text(m.get()).unwrap_or_default() };
                return (t, st, text);
            }
        };
        let (t, st, attach) = next("attach result");
        assert_eq!((t, st), (XBTP_MESSAGE_TYPE_MORE, 0));
        assert!(attach.contains("\"AttachResult\":0"), "{attach}");
        let (_, _, created) = next("create process event");
        assert!(created.contains("\"EventCode\":3") && created.contains(&format!("\"ProcessId\":{pid}")), "{created}");
        msgs.push(created);
        let _ = child.0.kill();
        let (t, _, exited) = next("exit process event");
        assert_eq!(t, XBTP_MESSAGE_TYPE_MORE);
        assert!(exited.contains("\"EventCode\":5") && exited.contains(&format!("\"ProcessId\":{pid}")), "{exited}");
        let _ = child.0.wait();
        let ctl = raw_open(RAW_TCP_PORT);
        let dreq = xtfmsg::fixture::message_with_json(&format!("{{\"ProcessId\":{pid},\"ForceAll\":1}}"));
        let mut dex = 0u32;
        assert_eq!(com::client_session::send_begin_message(ctl.get(), crate::server::OP_DETACH_PROCESS, dreq.get(), &mut dex), S_OK);
        let mut dst = 1u32;
        let mut dreply = Com::null();
        assert_eq!(com::client_session::receive_end_message(ctl.get(), 10000, dex, &mut dst, dreply.put()), S_OK);
        assert_eq!(dst, 0);
        let mut tail = Vec::new();
        loop {
            let (t, st, text) = next("end of exchange");
            tail.push((t, st, text));
            if t == XBTP_MESSAGE_TYPE_END {
                break;
            }
        }
        assert_eq!(tail.last().unwrap().1, 0);
        let _ = msgs;
        drop(s);
        drop(d);
    }
}
