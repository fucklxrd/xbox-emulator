mod audio;
mod chat;
mod heapcheck;
mod sys_stage;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(m: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetModuleHandleExW(flags: u32, name: *const u16, out: *mut *mut c_void) -> i32;
    fn GetModuleFileNameW(m: *mut c_void, buf: *mut u16, n: u32) -> u32;
    fn GetLastError() -> u32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn GetExitCodeProcess(h: *mut c_void, code: *mut u32) -> i32;
    fn CloseHandle(h: *mut c_void) -> i32;
}

fn pid_alive(pid: u32) -> bool {
    unsafe {
        let h = OpenProcess(0x1000, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0 && code == 259;
        CloseHandle(h);
        ok
    }
}

#[link(name = "winhttp")]
extern "system" {
    fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> *mut c_void;
    fn WinHttpConnect(s: *mut c_void, server: *const u16, port: u16, r: u32) -> *mut c_void;
    fn WinHttpOpenRequest(c: *mut c_void, verb: *const u16, obj: *const u16, ver: *const u16, refr: *const u16, acc: *const *const u16, flags: u32) -> *mut c_void;
    fn WinHttpSetOption(h: *mut c_void, opt: u32, buf: *mut c_void, len: u32) -> i32;
    fn WinHttpQueryOption(h: *mut c_void, opt: u32, buf: *mut c_void, len: *mut u32) -> i32;
    fn WinHttpSetStatusCallback(h: *mut c_void, cb: usize, flags: u32, r: usize) -> usize;
    fn WinHttpSendRequest(h: *mut c_void, hdr: *const u16, hl: u32, opt: *mut c_void, ol: u32, total: u32, ctx: usize) -> i32;
    fn WinHttpReceiveResponse(h: *mut c_void, r: *mut c_void) -> i32;
    fn WinHttpQueryHeaders(h: *mut c_void, level: u32, name: *const u16, buf: *mut c_void, len: *mut u32, idx: *mut u32) -> i32;
    fn WinHttpQueryDataAvailable(h: *mut c_void, n: *mut u32) -> i32;
    fn WinHttpReadData(h: *mut c_void, buf: *mut c_void, len: u32, read: *mut u32) -> i32;
    fn WinHttpCloseHandle(h: *mut c_void) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

fn guid(s: &str) -> Guid {
    let h: String = s.chars().filter(|c| *c != '-').collect();
    let b = |i: usize, n: usize| u64::from_str_radix(&h[i..i + n], 16).unwrap();
    let mut d4 = [0u8; 8];
    for (i, x) in d4.iter_mut().enumerate() {
        *x = b(16 + i * 2, 2) as u8;
    }
    Guid { d1: b(0, 8) as u32, d2: b(8, 4) as u16, d3: b(12, 4) as u16, d4 }
}

const S_OK: i32 = 0;
const E_PENDING: i32 = 0x8000_000Au32 as i32;
const E_ABORT: i32 = 0x8000_4004u32 as i32;
const E_FAIL: i32 = 0x8000_4005u32 as i32;
const E_NOT_SUFFICIENT_BUFFER: i32 = 0x8007_007Au32 as i32;
const ANCHOR: u64 = 0x5842_4F58_5553_4552;
const GAME_ORDER: &str = "mfplat.dll,XAudio2_9.dll,xmem.dll,d3d12_xs.dll,xcrdapi.dll,dxcompiler_xs.dll,acphal.dll,AudioHALWrapper.dll";
const OPTIONAL: &[&str] = &["AudioHALWrapper.dll"];
const FN_CLIENT: &str = "ec684b8c687f479fadea3cb2ad83f5c6:e1f31c211f28413186262d37a13fc84d";

type QueryFn = unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32;

#[repr(C)]
struct Block {
    queue: usize,
    context: usize,
    callback: usize,
    internal: [u8; 32],
}

impl Block {
    fn boxed(queue: usize) -> Box<Block> {
        Box::new(Block { queue, context: 0, callback: 0, internal: [0; 32] })
    }
    fn ptr(&mut self) -> *mut c_void {
        self as *mut Block as *mut c_void
    }
}

#[repr(C)]
struct ProviderData {
    block: *mut c_void,
    buffer_size: usize,
    buffer: *mut u8,
    context: *mut c_void,
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn module_path(h: *mut c_void) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;

    let mut buf = [0u16; 1024];
    let n = GetModuleFileNameW(h, buf.as_mut_ptr(), buf.len() as u32) as usize;
    (n != 0 && n < buf.len()).then(|| std::ffi::OsString::from_wide(&buf[..n]).into())
}

unsafe fn module_of(addr: usize) -> Option<String> {
    let mut h: *mut c_void = core::ptr::null_mut();
    if addr == 0 || GetModuleHandleExW(6, addr as *const u16, &mut h) == 0 {
        return None;
    }
    Some(module_path(h)?.file_name()?.to_string_lossy().into_owned())
}

fn mod_name(addr: usize) -> String {
    unsafe { module_of(addr) }.unwrap_or_else(|| format!("<no module @{addr:#x}>"))
}

unsafe fn anchors(base: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let rd32 = |p: usize| core::ptr::read_unaligned(p as *const u32) as usize;
    let rd16 = |p: usize| core::ptr::read_unaligned(p as *const u16) as usize;
    if rd16(base) != 0x5a4d {
        return out;
    }
    let nt = base + rd32(base + 0x3c);
    let count = rd16(nt + 6);
    let first = nt + 24 + rd16(nt + 20);
    for i in 0..count {
        let s = first + i * 40;
        let vsize = rd32(s + 8);
        let va = rd32(s + 12);
        let ch = rd32(s + 36) as u32;
        if ch & 0x2000_0000 != 0 || ch & 0x4000_0000 == 0 {
            continue;
        }
        let mut p = (base + va + 7) & !7;
        while p + 8 <= base + va + vsize {
            if core::ptr::read_volatile(p as *const u64) == ANCHOR {
                out.push(p);
            }
            p += 8;
        }
    }
    out
}

struct Ctx {
    modules: Vec<(String, usize)>,
    query: usize,
    xgr: usize,
    gdk_owner: String,
    net_owner: String,
    user: usize,
    token: String,
    client_token: String,
    account_id: String,
    access_token: String,
    port: u16,
    stage: PathBuf,
    log: PathBuf,
    gate: bool,
    async_impl: String,
}

static CTX: Mutex<Option<Ctx>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Ctx) -> R) -> R {
    let mut g = CTX.lock().unwrap_or_else(|e| e.into_inner());
    f(g.as_mut().expect("context"))
}

#[derive(Default)]
struct Tally {
    pass: usize,
    fail: usize,
    skip: usize,
}

static TALLY: Mutex<Tally> = Mutex::new(Tally { pass: 0, fail: 0, skip: 0 });

enum Out {
    Pass(String),
    Fail(String),
    Skip(String),
}

static QUIET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static TIMES: Mutex<Vec<(String, f64, bool)>> = Mutex::new(Vec::new());
static JSON_OUT: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

fn report(name: &str, o: Out) -> bool {
    let mut t = TALLY.lock().unwrap_or_else(|e| e.into_inner());
    match o {
        Out::Pass(m) => {
            t.pass += 1;
            if !QUIET.load(Ordering::Relaxed) {
                println!("PASS  {name}: {m}");
            }
            true
        }
        Out::Fail(m) => {
            t.fail += 1;
            println!("FAIL  {name}: {m}");
            false
        }
        Out::Skip(m) => {
            t.skip += 1;
            println!("SKIP  {name}: {m}");
            false
        }
    }
}

fn timing_table() {
    let times = TIMES.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut names: Vec<String> = Vec::new();
    for (n, _, _) in &times {
        if !names.contains(n) {
            names.push(n.clone());
        }
    }
    let mut rows = Vec::new();
    let (mut online_avg, mut total_avg) = (0.0f64, 0.0f64);
    for n in &names {
        let v: Vec<f64> = times.iter().filter(|(k, _, _)| k == n).map(|(_, ms, _)| *ms).collect();
        let ok = times.iter().filter(|(k, _, _)| k == n).all(|(_, _, ok)| *ok);
        let min = v.iter().cloned().fold(f64::MAX, f64::min);
        let max = v.iter().cloned().fold(0.0, f64::max);
        let avg = v.iter().sum::<f64>() / v.len() as f64;
        let stage = stage_of(n);
        if stage != "setup" {
            online_avg += avg;
        }
        total_avg += avg;
        rows.push(serde_json::json!({"name": n, "stage": stage, "runs": v.len(), "ms": v, "min_ms": min, "avg_ms": avg, "max_ms": max, "pass": ok}));
    }
    println!();
    println!("{:<56} {:>6} {:>4} {:>9} {:>9} {:>9}  {}", "step", "stage", "n", "min ms", "avg ms", "max ms", "status");
    for r in &rows {
        println!("{:<56} {:>6} {:>4} {:>9.2} {:>9.2} {:>9.2}  {}", r["name"].as_str().unwrap_or(""), r["stage"].as_str().unwrap_or(""), r["runs"], r["min_ms"].as_f64().unwrap_or(0.0), r["avg_ms"].as_f64().unwrap_or(0.0), r["max_ms"].as_f64().unwrap_or(0.0), if r["pass"] == true { "ok" } else { "FAIL" });
    }
    println!("{:<56} {:>6} {:>4} {:>9} {:>9.2}", "online total (login + epic, avg per run)", "", "", "", online_avg);
    println!("{:<56} {:>6} {:>4} {:>9} {:>9.2}", "all steps (avg)", "", "", "", total_avg);
    let out = JSON_OUT.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some((path, cfg)) = out {
        let t = TALLY.lock().unwrap_or_else(|e| e.into_inner());
        let doc = serde_json::json!({
            "tool": "xhost",
            "config": serde_json::from_str::<serde_json::Value>(&cfg).unwrap_or(serde_json::Value::Null),
            "pass": t.pass, "fail": t.fail, "skip": t.skip,
            "online_total_avg_ms": online_avg,
            "all_steps_avg_ms": total_avg,
            "steps": rows,
        });
        drop(t);
        match std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap_or_default()) {
            Ok(_) => println!("timings written to {}", path.display()),
            Err(e) => println!("FAIL  json: cannot write {}: {e}", path.display()),
        }
    }
}

fn stage_of(n: &str) -> &'static str {
    if n.starts_with("epic ") {
        "epic"
    } else if n.starts_with("login ") {
        "login"
    } else {
        "setup"
    }
}

fn finish() -> ! {
    timing_table();
    let t = TALLY.lock().unwrap_or_else(|e| e.into_inner());
    println!("RESULT: {} pass, {} fail, {} skip", t.pass, t.fail, t.skip);
    use std::io::Write;
    let _ = std::io::stdout().flush();
    std::process::exit(if t.fail > 0 { 1 } else { 0 })
}

static XUID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static STEP_MS: AtomicUsize = AtomicUsize::new(10_000);

fn step(name: &str, f: impl FnOnce() -> Result<String, Out> + Send + 'static) -> bool {
    let (tx, rx) = channel();
    let started = Instant::now();
    std::thread::Builder::new()
        .name(format!("xhost-{name}"))
        .spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            let _ = tx.send(r);
        })
        .expect("spawn step");
    let limit = STEP_MS.load(Ordering::Relaxed) as u64;
    let got = rx.recv_timeout(Duration::from_millis(limit));
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let ok = match got {
        Ok(Ok(Ok(m))) => report(name, Out::Pass(format!("{m} ({ms:.1} ms)"))),
        Ok(Ok(Err(Out::Skip(m)))) => {
            report(name, Out::Skip(m));
            return false;
        }
        Ok(Ok(Err(o))) => report(name, o),
        Ok(Err(p)) => {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            report(name, Out::Fail(format!("panicked: {msg}")))
        }
        Err(_) => {
            report(name, Out::Fail(format!("timed out after {limit} ms (hang in this step)")));
            TIMES.lock().unwrap_or_else(|e| e.into_inner()).push((name.to_string(), ms, false));
            finish()
        }
    };
    TIMES.lock().unwrap_or_else(|e| e.into_inner()).push((name.to_string(), ms, ok));
    heapcheck::after_step(name);
    ok
}

fn fail<T>(m: impl Into<String>) -> Result<T, Out> {
    Err(Out::Fail(m.into()))
}

fn check(ok: bool, m: impl FnOnce() -> String) -> Result<(), Out> {
    if ok {
        Ok(())
    } else {
        Err(Out::Fail(m()))
    }
}

unsafe extern "system" fn fake_endpoint(_t: *mut c_void, _local: u64, kind: u32, count: usize, buf: *mut u16, used: *mut usize) -> i32 {
    let s: Vec<u16> = format!("{{0.0.{}.00000000}}.{{7a0b0b0b-0000-4000-8000-00000000000{}}}", if kind == 0 { 0 } else { 1 }, kind).encode_utf16().chain(Some(0)).collect();
    FAKE_EP_CALLS.fetch_add(1, Ordering::SeqCst);
    if !used.is_null() {
        *used = s.len();
    }
    if buf.is_null() || count < s.len() {
        return if buf.is_null() { 0 } else { 0x8007007Au32 as i32 };
    }
    core::ptr::copy_nonoverlapping(s.as_ptr(), buf, s.len());
    0
}

static FAKE_EP_CALLS: AtomicUsize = AtomicUsize::new(0);

extern "system" {
    fn VirtualProtect(addr: *mut c_void, size: usize, prot: u32, old: *mut u32) -> i32;
}

unsafe fn query(clsid: &str, iid: &str) -> Result<*mut c_void, Out> {
    let q: QueryFn = core::mem::transmute(with(|c| c.query));
    let (c, i) = (guid(clsid), guid(iid));
    let mut obj: *mut c_void = core::ptr::null_mut();
    let hr = q(&c, &i, &mut obj);
    if hr < 0 || obj.is_null() {
        return fail(format!("QueryApiImpl({clsid}, {iid}) hr={hr:#010x}"));
    }
    Ok(obj)
}

unsafe fn slot(obj: *mut c_void, n: usize) -> usize {
    *(*(obj as *const *const usize)).add(n)
}

const XASYNC: &str = "073b7dcb-1fcf-4030-94be-e3c9eb623428";
const XUSER: &str = "01acd177-91f9-4763-a38e-ccbb55ce32e0";
const XUSER_V1: &str = "eb9bf948-18dc-4d82-bbcc-40e0a809c4c0";
const XUSER_V2: &str = "1bf2f8c5-d507-4e52-bb05-f726d0e71161";
const XUSER_GT: &str = "cef4fac0-7676-4a94-a119-4c43f9eb5b74";
const XNET: &str = "37e56907-2f10-41e8-b72f-36edb185331a";
const XNET_V2: &str = "bf2346b2-39af-4658-b5ea-44713c7e83b3";

struct XA {
    obj: *mut c_void,
}

unsafe impl Send for XA {}

impl XA {
    unsafe fn get() -> Result<XA, Out> {
        Ok(XA { obj: query(XASYNC, XASYNC)? })
    }
    unsafe fn status(&self, b: *mut c_void, wait: bool) -> i32 {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, bool) -> i32 = core::mem::transmute(slot(self.obj, 3));
        f(self.obj, b, wait)
    }
    unsafe fn size(&self, b: *mut c_void) -> (i32, usize) {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut usize) -> i32 = core::mem::transmute(slot(self.obj, 4));
        let mut n = 0usize;
        let hr = f(self.obj, b, &mut n);
        (hr, n)
    }
    unsafe fn cancel(&self, b: *mut c_void) {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(slot(self.obj, 5));
        f(self.obj, b)
    }
    unsafe fn run(&self, b: *mut c_void, work: unsafe extern "system" fn(*mut c_void) -> i32) -> i32 {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, usize) -> i32 = core::mem::transmute(slot(self.obj, 6));
        f(self.obj, b, work as usize)
    }
    unsafe fn begin(&self, b: *mut c_void, ctx: *mut c_void, identity: usize, name: &'static [u8], p: Provider) -> i32 {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, usize, *const u8, Provider) -> i32 = core::mem::transmute(slot(self.obj, 7));
        f(self.obj, b, ctx, identity, name.as_ptr(), p)
    }
    unsafe fn schedule(&self, b: *mut c_void, delay: u32) -> i32 {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> i32 = core::mem::transmute(slot(self.obj, 9));
        f(self.obj, b, delay)
    }
    unsafe fn complete(&self, b: *mut c_void, hr: i32, size: usize) {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, i32, usize) = core::mem::transmute(slot(self.obj, 10));
        f(self.obj, b, hr, size)
    }
    unsafe fn result(&self, b: *mut c_void, identity: usize, buf: &mut [u8]) -> (i32, usize) {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, usize, usize, *mut c_void, *mut usize) -> i32 = core::mem::transmute(slot(self.obj, 11));
        let mut used = 0usize;
        let hr = f(self.obj, b, identity, buf.len(), buf.as_mut_ptr() as *mut c_void, &mut used);
        (hr, used)
    }
    unsafe fn queue_create(&self, work: u32, completion: u32) -> (i32, usize) {
        let f: unsafe extern "system" fn(*mut c_void, u32, u32, *mut usize) -> i32 = core::mem::transmute(slot(self.obj, 12));
        let mut q = 0usize;
        let hr = f(self.obj, work, completion, &mut q);
        (hr, q)
    }
    unsafe fn dispatch(&self, q: usize, port: u32, ms: u32) -> bool {
        let f: unsafe extern "system" fn(*mut c_void, usize, u32, u32) -> bool = core::mem::transmute(slot(self.obj, 16));
        f(self.obj, q, port, ms)
    }
    unsafe fn queue_close(&self, q: usize) {
        let f: unsafe extern "system" fn(*mut c_void, usize) = core::mem::transmute(slot(self.obj, 17));
        f(self.obj, q)
    }
    unsafe fn poll(&self, b: *mut c_void, ms: u64) -> i32 {
        let start = Instant::now();
        loop {
            let hr = self.status(b, false);
            if hr != E_PENDING || start.elapsed() > Duration::from_millis(ms) {
                return hr;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    unsafe fn finish(&self, b: *mut c_void, identity: usize, op: &str) -> Result<Vec<u8>, Out> {
        let hr = self.poll(b, wait_ms());
        check(hr == S_OK, || format!("{op}: XAsyncGetStatus hr={hr:#010x}"))?;
        let (hr, n) = self.size(b);
        check(hr == S_OK, || format!("{op}: XAsyncGetResultSize hr={hr:#010x}"))?;
        let mut buf = vec![0u8; n];
        if n == 0 {
            return Ok(buf);
        }
        let (hr, used) = self.result(b, identity, &mut buf);
        check(hr == S_OK && used == n, || format!("{op}: XAsyncGetResult hr={hr:#010x} used={used} size={n}"))?;
        Ok(buf)
    }
}

fn wait_ms() -> u64 {
    (STEP_MS.load(Ordering::Relaxed) as u64 * 3 / 4).max(500)
}

type Provider = unsafe extern "system" fn(u32, *const ProviderData) -> i32;

struct Job {
    xa: usize,
    work: Mutex<Option<Box<dyn FnOnce() -> Result<Vec<u8>, i32> + Send>>>,
    result: Mutex<Option<Vec<u8>>>,
    cancels: AtomicUsize,
    cleaned: AtomicUsize,
}

unsafe extern "system" fn job_provider(op: u32, d: *const ProviderData) -> i32 {
    let j = &*((*d).context as *const Job);
    let xa = XA { obj: j.xa as *mut c_void };
    match op {
        1 => {
            let work = j.work.lock().unwrap_or_else(|e| e.into_inner()).take();
            let Some(work) = work else { return E_PENDING };
            let block = (*d).block as usize;
            let jp = j as *const Job as usize;
            std::thread::spawn(move || {
                let j = &*(jp as *const Job);
                let xa = XA { obj: j.xa as *mut c_void };
                match work() {
                    Ok(v) => {
                        let n = v.len();
                        *j.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
                        xa.complete(block as *mut c_void, S_OK, n);
                    }
                    Err(hr) => xa.complete(block as *mut c_void, hr, 0),
                }
            });
            let _ = xa;
            E_PENDING
        }
        2 => {
            let g = j.result.lock().unwrap_or_else(|e| e.into_inner());
            let Some(v) = g.as_ref() else { return E_FAIL };
            if (*d).buffer_size < v.len() {
                return E_NOT_SUFFICIENT_BUFFER;
            }
            core::ptr::copy_nonoverlapping(v.as_ptr(), (*d).buffer, v.len());
            S_OK
        }
        3 => {
            j.cancels.fetch_add(1, Ordering::SeqCst);
            xa.complete((*d).block, E_ABORT, 0);
            S_OK
        }
        4 => {
            j.cleaned.fetch_add(1, Ordering::SeqCst);
            S_OK
        }
        _ => S_OK,
    }
}

static WORK_RAN: AtomicUsize = AtomicUsize::new(0);
static CALLBACK_RAN: AtomicUsize = AtomicUsize::new(0);
static CALLBACK_HR: AtomicI32 = AtomicI32::new(1);
static CB_XA: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn run_work(_b: *mut c_void) -> i32 {
    WORK_RAN.fetch_add(1, Ordering::SeqCst);
    S_OK
}

unsafe extern "system" fn run_done(b: *mut c_void) {
    let xa = XA { obj: CB_XA.load(Ordering::SeqCst) as *mut c_void };
    CALLBACK_HR.store(xa.status(b, false), Ordering::SeqCst);
    CALLBACK_RAN.fetch_add(1, Ordering::SeqCst);
}

#[derive(Debug)]
enum Note {
    Sending(i32),
    SendDone,
    Headers,
    Data(u32),
    Read(u32),
    Error(usize, u32),
}

static VOBJ: AtomicUsize = AtomicUsize::new(0);
static VFN: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn http_cb(h: *mut c_void, ctx: usize, status: u32, info: *mut c_void, len: u32) {
    if ctx == 0 {
        return;
    }
    let tx = &*(ctx as *const Mutex<Sender<Note>>);
    let send = |n: Note| {
        let _ = tx.lock().map(|t| t.send(n));
    };
    match status {
        0x0000_0010 => {
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *const u8) -> i32 = core::mem::transmute(VFN.load(Ordering::SeqCst));
            let hr = f(VOBJ.load(Ordering::SeqCst) as *mut c_void, h, core::ptr::null());
            send(Note::Sending(hr));
        }
        0x0040_0000 => send(Note::SendDone),
        0x0002_0000 => send(Note::Headers),
        0x0004_0000 => send(Note::Data(if info.is_null() { 0 } else { *(info as *const u32) })),
        0x0008_0000 => send(Note::Read(len)),
        0x0020_0000 => {
            let r = &*(info as *const [usize; 2]);
            send(Note::Error(r[0], r[1] as u32));
        }
        _ => {}
    }
}

struct HttpResp {
    status: u32,
    body: Vec<u8>,
    verify: Option<i32>,
    url: String,
}

fn session() -> usize {
    static S: AtomicUsize = AtomicUsize::new(0);
    let s = S.load(Ordering::SeqCst);
    if s != 0 {
        return s;
    }
    let h = unsafe { WinHttpOpen(w("tombox-xhost").as_ptr(), 1, core::ptr::null(), core::ptr::null(), 0x3000_0000) } as usize;
    S.store(h, Ordering::SeqCst);
    h
}

unsafe fn request_url(r: *mut c_void) -> String {
    let mut buf = vec![0u16; 4096];
    let mut n = (buf.len() * 2) as u32;
    if WinHttpQueryOption(r, 34, buf.as_mut_ptr() as *mut c_void, &mut n) == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..(n as usize / 2).min(buf.len())]).trim_end_matches('\0').to_string()
}

fn http(method: &str, host: &str, path: &str, headers: &str, body: &[u8]) -> Result<HttpResp, String> {
    let (port, gate) = with(|c| (c.port, c.gate));
    if !gate {
        return Err("network safety gate not passed; request not issued".into());
    }
    unsafe {
        let s = session() as *mut c_void;
        if s.is_null() {
            return Err(format!("WinHttpOpen failed err={}", GetLastError()));
        }
        let c = WinHttpConnect(s, w(host).as_ptr(), 443, 0);
        if c.is_null() {
            return Err(format!("WinHttpConnect({host}) failed err={}", GetLastError()));
        }
        let r = WinHttpOpenRequest(c, w(method).as_ptr(), w(path).as_ptr(), core::ptr::null(), core::ptr::null(), core::ptr::null(), 0x0080_0000);
        if r.is_null() {
            let e = GetLastError();
            WinHttpCloseHandle(c);
            return Err(format!("WinHttpOpenRequest failed err={e}"));
        }
        let url = request_url(r);
        let local = format!("http://127.0.0.1:{port}/");
        if port == 0 || !url.to_ascii_lowercase().starts_with(&local) {
            WinHttpCloseHandle(r);
            WinHttpCloseHandle(c);
            return Err(format!("request URL '{url}' is not the in-process backend {local}; refused to send"));
        }
        if WinHttpSetOption(r, 47, core::ptr::null_mut(), 0) == 0 {
            let e = GetLastError();
            WinHttpCloseHandle(r);
            WinHttpCloseHandle(c);
            return Err(format!("WinHttpSetOption(CLIENT_CERT_CONTEXT, null) failed err={e}"));
        }
        let (tx, rx): (Sender<Note>, Receiver<Note>) = channel();
        let ctx = Box::leak(Box::new(Mutex::new(tx))) as *const Mutex<Sender<Note>> as usize;
        WinHttpSetStatusCallback(r, http_cb as *const () as usize, 0xffff_ffff, 0);
        let hdr = w(headers);
        let mut b = body.to_vec();
        let hl = if headers.is_empty() { 0 } else { u32::MAX };
        let hp = if headers.is_empty() { core::ptr::null() } else { hdr.as_ptr() };
        let bp = if b.is_empty() { core::ptr::null_mut() } else { b.as_mut_ptr() as *mut c_void };
        if WinHttpSendRequest(r, hp, hl, bp, b.len() as u32, b.len() as u32, ctx) == 0 {
            let e = GetLastError();
            WinHttpCloseHandle(r);
            WinHttpCloseHandle(c);
            return Err(format!("WinHttpSendRequest failed err={e}"));
        }
        let deadline = Instant::now() + Duration::from_millis(wait_ms());
        let mut verify = None;
        let mut status = 0u32;
        let mut out = Vec::new();
        let mut chunk = vec![0u8; 0];
        let res = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let n = match rx.recv_timeout(left) {
                Ok(n) => n,
                Err(_) => break Err(format!("no WinHTTP callback within the step budget (verify={verify:?} status={status})")),
            };
            match n {
                Note::Sending(hr) => {
                    verify = Some(hr);
                    if hr < 0 {
                        break Err(format!("XNetworkingVerifyServerCertificate rejected the request hr={hr:#010x}"));
                    }
                }
                Note::SendDone => {
                    if WinHttpReceiveResponse(r, core::ptr::null_mut()) == 0 {
                        break Err(format!("WinHttpReceiveResponse err={}", GetLastError()));
                    }
                }
                Note::Headers => {
                    let mut n = 4u32;
                    if WinHttpQueryHeaders(r, 19 | 0x2000_0000, core::ptr::null(), &mut status as *mut u32 as *mut c_void, &mut n, core::ptr::null_mut()) == 0 {
                        break Err(format!("WinHttpQueryHeaders(STATUS_CODE) err={}", GetLastError()));
                    }
                    if WinHttpQueryDataAvailable(r, core::ptr::null_mut()) == 0 {
                        break Err(format!("WinHttpQueryDataAvailable err={}", GetLastError()));
                    }
                }
                Note::Data(0) => break Ok(()),
                Note::Data(n) => {
                    chunk = vec![0u8; n as usize];
                    if WinHttpReadData(r, chunk.as_mut_ptr() as *mut c_void, n, core::ptr::null_mut()) == 0 {
                        break Err(format!("WinHttpReadData err={}", GetLastError()));
                    }
                }
                Note::Read(len) => {
                    out.extend_from_slice(&chunk[..(len as usize).min(chunk.len())]);
                    if WinHttpQueryDataAvailable(r, core::ptr::null_mut()) == 0 {
                        break Err(format!("WinHttpQueryDataAvailable err={}", GetLastError()));
                    }
                }
                Note::Error(api, err) => break Err(format!("REQUEST_ERROR api={api} err={err} verify={verify:?}")),
            }
        };
        WinHttpSetStatusCallback(r, 0, 0xffff_ffff, 0);
        WinHttpCloseHandle(r);
        WinHttpCloseHandle(c);
        drop(chunk);
        res.map(|_| HttpResp { status, body: out, verify, url })
    }
}

fn encode(resp: &HttpResp) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&resp.status.to_le_bytes());
    v.extend_from_slice(&resp.verify.unwrap_or(1).to_le_bytes());
    v.extend_from_slice(&(resp.url.len() as u32).to_le_bytes());
    v.extend_from_slice(resp.url.as_bytes());
    v.extend_from_slice(&resp.body);
    v
}

fn decode(v: &[u8]) -> HttpResp {
    let u = |i: usize| u32::from_le_bytes(v[i..i + 4].try_into().unwrap());
    let ul = u(8) as usize;
    let verify = u(4) as i32;
    HttpResp {
        status: u(0),
        verify: if verify == 1 { None } else { Some(verify) },
        url: String::from_utf8_lossy(&v[12..12 + ul]).into_owned(),
        body: v[12 + ul..].to_vec(),
    }
}

static HC_NAME: &[u8] = b"HCHttpCallPerformAsync\0";

fn http_async(method: &str, host: &str, path: &str, headers: &str, body: &[u8]) -> Result<HttpResp, Out> {
    let (m, h, p, hd, b) = (method.to_string(), host.to_string(), path.to_string(), headers.to_string(), body.to_vec());
    unsafe {
        let xa = XA::get()?;
        let err: Mutex<Option<String>> = Mutex::new(None);
        let err = Box::leak(Box::new(err)) as &'static Mutex<Option<String>>;
        let job = Box::leak(Box::new(Job {
            xa: xa.obj as usize,
            work: Mutex::new(Some(Box::new(move || match http(&m, &h, &p, &hd, &b) {
                Ok(r) => Ok(encode(&r)),
                Err(e) => {
                    *err.lock().unwrap() = Some(e);
                    Err(E_FAIL)
                }
            }))),
            result: Mutex::new(None),
            cancels: AtomicUsize::new(0),
            cleaned: AtomicUsize::new(0),
        }));
        let mut blk = Block::boxed(0);
        let identity = job_provider as *const () as usize;
        let hr = xa.begin(blk.ptr(), job as *mut Job as *mut c_void, identity, HC_NAME, job_provider);
        check(hr == S_OK, || format!("XAsyncBegin hr={hr:#010x}"))?;
        let hr = xa.schedule(blk.ptr(), 0);
        check(hr == S_OK, || format!("XAsyncSchedule hr={hr:#010x}"))?;
        let hr = xa.poll(blk.ptr(), wait_ms());
        if hr != S_OK {
            let e = err.lock().unwrap().clone().unwrap_or_default();
            return fail(format!("XAsyncGetStatus hr={hr:#010x} {e}"));
        }
        let v = xa.finish(blk.ptr(), identity, "HCHttpCallPerformAsync")?;
        std::mem::forget(blk);
        Ok(decode(&v))
    }
}

fn json(r: &HttpResp) -> serde_json::Value {
    serde_json::from_slice(&r.body).unwrap_or(serde_json::Value::Null)
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn urlenc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn hr_ok(r: &HttpResp, want: u32, what: &str) -> Result<(), Out> {
    check(r.verify == Some(S_OK), || format!("{what}: XNetworkingVerifyServerCertificate in SENDING_REQUEST = {:?}, expected Some(0)", r.verify))?;
    check(r.status == want, || format!("{what}: HTTP {} (want {want}) body {}", r.status, String::from_utf8_lossy(&r.body).chars().take(200).collect::<String>()))
}

fn env_is_pid(k: &str) -> bool {
    std::env::var(k).ok().as_deref() == Some(std::process::id().to_string().as_str())
}

fn wait_env(k: &str, ms: u64) -> Option<String> {
    let start = Instant::now();
    loop {
        if let Ok(v) = std::env::var(k) {
            return Some(v);
        }
        if start.elapsed() > Duration::from_millis(ms) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

unsafe fn jump_target(p: *const u8) -> Option<usize> {
    if p.is_null() {
        return None;
    }
    let b = core::slice::from_raw_parts(p, 12);
    if b[0] == 0x48 && b[1] == 0xB8 && b[10] == 0xFF && b[11] == 0xE0 {
        Some(u64::from_le_bytes(b[2..10].try_into().unwrap()) as usize)
    } else {
        None
    }
}

unsafe fn export(dll: &str, f: &str) -> *const u8 {
    let mut h: *mut c_void = core::ptr::null_mut();
    GetModuleHandleExW(2, w(dll).as_ptr(), &mut h);
    if h.is_null() {
        return core::ptr::null();
    }
    GetProcAddress(h, format!("{f}\0").as_ptr()) as *const u8
}

fn arg(args: &[String], k: &str) -> Option<String> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1).cloned())
}

fn default_dir() -> PathBuf {
    for d in ["release", "debug"] {
        let p = PathBuf::from(r"D:\xbox-pc-port\emulator\target-qa").join(d);
        if p.join("mfplat.dll").exists() && p.join("xmem.dll").exists() {
            return p;
        }
    }
    PathBuf::from(r"D:\xbox-pc-port\emulator\target-qa\release")
}

fn stage(dir: &Path, order: &[String]) -> Result<(PathBuf, Vec<String>), String> {
    let root = std::env::temp_dir().join("tombox").join("xhost");
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let live = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()).map_or(false, pid_alive);
            if !live {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let st = root.join(format!("{}", std::process::id()));
    std::fs::create_dir_all(&st).map_err(|e| format!("create {}: {e}", st.display()))?;
    let mut have = Vec::new();
    for n in order {
        let src = dir.join(n);
        if !src.exists() {
            if OPTIONAL.iter().any(|o| o.eq_ignore_ascii_case(n)) {
                continue;
            }
            return Err(format!("{} not built", src.display()));
        }
        std::fs::copy(&src, st.join(n)).map_err(|e| format!("copy {}: {e}", src.display()))?;
        have.push(n.clone());
    }
    for extra in ["opus.dll", "SKP_SILK_SDK.dll"] {
        if dir.join(extra).exists() {
            let _ = std::fs::copy(dir.join(extra), st.join(extra));
        }
    }
    sys_stage::stage_sys_companions(&st, &have)?;
    Ok((st, have))
}

fn setup_env(st: &Path, load_only: bool) -> PathBuf {
    for k in ["TOMBOX_GDK_OWNER", "TOMBOX_NET_GUARD_OWNER", "TOMBOX_OSHOST_OWNER", "TOMBOX_OSHOST_SIGNAL", "TOMBOX_NET_INPROC_PORT", "TOMBOX_NET_LOCAL_PORT", "TOMBOX_PAD_OWNER", "TOMBOX_ALLOW_NETWORK", "TOMBOX_GDK_XASYNC", "TOMBOX_GDK_USER_HANDLE"] {
        std::env::remove_var(k);
    }
    let log = st.join("tombox_compat.log");
    let _ = std::fs::remove_file(&log);
    std::env::set_var("TOMBOX_GDK_RUNTIME", "1");
    std::env::set_var("TOMBOX_NET_REDIRECT", if load_only { "" } else { "127.0.0.1" });
    std::env::set_var("TOMBOX_NET_BACKEND", if load_only { "" } else { "builtin" });
    std::env::set_var("TOMBOX_LOG", &log);
    std::env::set_var("TOMBOX_GDK_ROOT", st.join("gdk"));
    std::env::set_var("TOMBOX_PAD_DIR", st.join("pad"));
    log
}

static ATHENA_RVN: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);
static CORE_RVN: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);

type Chk = fn(&serde_json::Value, &HttpResp) -> Result<String, String>;

const FN_HOST: &str = "fortnite-public-service-prod11.ol.epicgames.com";
const ACC_HOST: &str = "account-public-service-prod.ol.epicgames.com";

fn fill(t: &str) -> String {
    let id = with(|c| c.account_id.clone());
    t.replace("{id}", &id).replace("{athena_rvn}", &ATHENA_RVN.load(Ordering::SeqCst).to_string()).replace("{core_rvn}", &CORE_RVN.load(Ordering::SeqCst).to_string())
}

fn account() -> String {
    with(|c| c.account_id.clone())
}

fn epic(name: &'static str, method: &'static str, host: &'static str, path: &'static str, body: &'static str, want: u32, chk: Chk) {
    step(name, move || {
        let at = with(|c| c.access_token.clone());
        if at.is_empty() {
            return fail("no account token from the login stage");
        }
        let ctype = if body.starts_with('{') || body.starts_with('[') { "application/json" } else { "application/x-www-form-urlencoded" };
        let hdr = if body.is_empty() { format!("Authorization: bearer {at}\r\n") } else { format!("Authorization: bearer {at}\r\nContent-Type: {ctype}\r\n") };
        let p = fill(path);
        let r = http_async(method, host, &p, &hdr, fill(body).as_bytes())?;
        hr_ok(&r, want, name)?;
        let j = json(&r);
        match chk(&j, &r) {
            Ok(m) => Ok(format!("{} {} B; {m}", r.status, r.body.len())),
            Err(e) => fail(format!("{method} {}: {e}; body {}", p.split('?').next().unwrap_or(""), String::from_utf8_lossy(&r.body).chars().take(160).collect::<String>())),
        }
    });
}

fn is_arr(j: &serde_json::Value, what: &str) -> Result<usize, String> {
    j.as_array().map(|a| a.len()).ok_or_else(|| format!("{what}: not a JSON array"))
}

fn is_obj(j: &serde_json::Value, what: &str) -> Result<usize, String> {
    j.as_object().map(|a| a.len()).ok_or_else(|| format!("{what}: not a JSON object"))
}

fn profile(j: &serde_json::Value, want: &str) -> Result<i64, String> {
    if j["profileId"] != want {
        return Err(format!("profileId {} != {want}", j["profileId"]));
    }
    let rvn = j["profileRevision"].as_i64().ok_or("no profileRevision")?;
    j["profileCommandRevision"].as_i64().ok_or("no profileCommandRevision")?;
    j["serverTime"].as_str().ok_or("no serverTime")?;
    Ok(rvn)
}

fn full_profile(j: &serde_json::Value, want: &str) -> Result<(i64, usize), String> {
    let rvn = profile(j, want)?;
    let c = &j["profileChanges"][0];
    if c["changeType"] != "fullProfileUpdate" {
        return Err(format!("profileChanges[0].changeType {} (want fullProfileUpdate for rvn=-1)", c["changeType"]));
    }
    if c["profile"]["profileId"] != want {
        return Err(format!("embedded profile id {}", c["profile"]["profileId"]));
    }
    let items = c["profile"]["items"].as_object().ok_or("profile.items is not an object")?.len();
    c["profile"]["stats"]["attributes"].as_object().ok_or("profile.stats.attributes missing")?;
    Ok((rvn, items))
}

fn epic_flow() {
    epic("epic account externalAuths", "GET", ACC_HOST, "/account/api/public/account/{id}/externalAuths", "", 200, |j, _| is_arr(j, "externalAuths").map(|n| format!("{n} entries")));
    epic("epic account bulk", "GET", ACC_HOST, "/account/api/public/account?accountId={id}", "", 200, |j, _| {
        is_arr(j, "bulk")?;
        if j[0]["id"] != account().as_str() {
            return Err(format!("first id {} != {}", j[0]["id"], account()));
        }
        Ok(format!("displayName {}", j[0]["displayName"]))
    });
    epic("epic user setting", "POST", FN_HOST, "/api/v1/user/setting", "{\"accountId\":\"{id}\",\"settings\":[]}", 200, |j, _| is_arr(j, "user setting").map(|n| format!("{n} settings")));
    epic("epic lightswitch", "GET", FN_HOST, "/lightswitch/api/service/bulk/status?serviceId=Fortnite", "", 200, |j, _| {
        if j[0]["status"] != "UP" {
            return Err(format!("status {}", j[0]["status"]));
        }
        let play = j[0]["allowedActions"].as_array().map_or(false, |a| a.iter().any(|x| x == "PLAY"));
        if !play {
            return Err("allowedActions lacks PLAY".into());
        }
        Ok("UP, PLAY allowed".into())
    });
    epic("epic enabled_features", "GET", FN_HOST, "/fortnite/api/game/v2/enabled_features", "", 200, |j, _| is_arr(j, "enabled_features").map(|n| format!("{n} features")));
    step("epic cloudstorage system", || {
        let at = with(|c| c.access_token.clone());
        let hdr = format!("Authorization: bearer {at}\r\n");
        let r = http_async("GET", FN_HOST, "/fortnite/api/cloudstorage/system", &hdr, b"")?;
        hr_ok(&r, 200, "cloudstorage system")?;
        let j = json(&r);
        let list = j.as_array().ok_or_else(|| Out::Fail("system list is not an array".into()))?;
        let mut fetched = 0usize;
        for e in list {
            let name = e["uniqueFilename"].as_str().ok_or_else(|| Out::Fail(format!("entry without uniqueFilename: {e}")))?;
            let len = e["length"].as_u64().unwrap_or(0) as usize;
            let f = http_async("GET", FN_HOST, &format!("/fortnite/api/cloudstorage/system/{name}"), &hdr, b"")?;
            hr_ok(&f, 200, name)?;
            check(e["length"].is_null() || f.body.len() == len, || format!("{name}: {} bytes, list says {len}", f.body.len()))?;
            fetched += 1;
        }
        Ok(format!("{} files listed and fetched, sizes match", fetched))
    });
    epic("epic cloudstorage user", "GET", FN_HOST, "/fortnite/api/cloudstorage/user/{id}", "", 200, |j, _| is_arr(j, "user files").map(|n| format!("{n} files")));
    epic("epic content-controls", "GET", FN_HOST, "/content-controls/{id}", "", 200, |j, _| is_obj(j, "content-controls").map(|n| format!("{n} keys")));
    epic("epic QueryProfile common_public", "POST", FN_HOST, "/fortnite/api/game/v2/profile/{id}/client/QueryProfile?profileId=common_public&rvn=-1", "{}", 200, |j, _| full_profile(j, "common_public").map(|(r, n)| format!("rvn {r}, {n} items")));
    epic("epic QueryProfile common_core", "POST", FN_HOST, "/fortnite/api/game/v2/profile/{id}/client/QueryProfile?profileId=common_core&rvn=-1", "{}", 200, |j, _| {
        let (r, n) = full_profile(j, "common_core")?;
        CORE_RVN.store(r, Ordering::SeqCst);
        Ok(format!("rvn {r}, {n} items"))
    });
    epic("epic QueryProfile athena", "POST", FN_HOST, "/fortnite/api/game/v2/profile/{id}/client/QueryProfile?profileId=athena&rvn=-1", "{}", 200, |j, _| {
        let (r, n) = full_profile(j, "athena")?;
        if n == 0 {
            return Err("athena profile has no items (no locker)".into());
        }
        ATHENA_RVN.store(r, Ordering::SeqCst);
        Ok(format!("rvn {r}, {n} items, season {}", j["profileChanges"][0]["profile"]["stats"]["attributes"]["season_num"]))
    });
    epic("epic SetMtxPlatform", "POST", FN_HOST, "/fortnite/api/game/v2/profile/{id}/client/SetMtxPlatform?profileId=common_core&rvn={core_rvn}", "{\"newPlatform\":\"XboxLive\"}", 200, |j, _| profile(j, "common_core").map(|r| format!("rvn {r}")));
    epic("epic keychain", "GET", FN_HOST, "/fortnite/api/storefront/v2/keychain?numKeysDownloaded=0", "", 200, |j, _| {
        let n = is_arr(j, "keychain")?;
        let ok = j.as_array().unwrap().iter().all(|k| k.as_str().map_or(false, |s| s.contains(':')));
        if n == 0 || !ok {
            return Err(format!("{n} keys, all 'guid:key' strings = {ok}"));
        }
        Ok(format!("{n} keys"))
    });
    epic("epic receipts", "GET", FN_HOST, "/fortnite/api/receipts/v1/account/{id}/receipts", "", 200, |j, _| is_arr(j, "receipts").map(|n| format!("{n} receipts")));
    epic("epic friends blocklist", "GET", FN_HOST, "/friends/api/v1/{id}/blocklist", "", 200, |j, _| is_arr(j, "blocklist").map(|n| format!("{n} blocked")));
    epic("epic socialban", "GET", FN_HOST, "/socialban/api/public/v1/{id}", "", 200, |j, _| {
        j["bans"].as_array().ok_or("no bans array")?;
        j["warnings"].as_array().ok_or("no warnings array")?;
        Ok("no bans".into())
    });
    epic("epic friends summary", "GET", FN_HOST, "/friends/api/v1/{id}/summary", "", 200, |j, _| {
        j["friends"].as_array().ok_or("no friends array")?;
        Ok(format!("{} keys", is_obj(j, "summary")?))
    });
    epic("epic party user", "GET", FN_HOST, "/party/api/v1/Fortnite/user/{id}", "", 200, |j, _| {
        j["current"].as_array().ok_or("no current array")?;
        Ok(format!("{} keys", is_obj(j, "party user")?))
    });
    epic("epic presence last-online", "GET", FN_HOST, "/presence/api/v1/_/{id}/last-online", "", 200, |j, _| is_obj(j, "last-online").map(|n| format!("{n} keys")));
    epic("epic sdk default", "GET", FN_HOST, "/sdk/v1/default?platformId=XBL", "", 200, |j, _| {
        is_obj(&j["client"], "client")?;
        is_obj(&j["services"], "services")?;
        Ok("client + services".into())
    });
    epic("epic EOS auth token", "POST", FN_HOST, "/auth/v1/oauth/token", "grant_type=external_auth&external_auth_type=epicgames_access_token&deployment_id=62a9473a2dca46b29ccf17577fcf42d7&nonce=xhost", 200, |j, _| {
        let at = j["access_token"].as_str().unwrap_or("");
        let pu = j["product_user_id"].as_str().unwrap_or("");
        if at.is_empty() || !pu.starts_with(account().as_str()) {
            return Err(format!("access_token '{at}' product_user_id '{pu}'"));
        }
        Ok(format!("product_user_id {pu}"))
    });
    epic("epic sdk product", "GET", FN_HOST, "/sdk/v1/product/prod-fn?platformId=XBL&deploymentId=62a9473a2dca46b29ccf17577fcf42d7", "", 200, |j, _| is_obj(j, "sdk product").map(|n| format!("{n} keys")));
    epic("epic content pages", "GET", FN_HOST, "/content/api/pages/fortnite-game/", "", 200, |j, _| {
        is_obj(&j["subgameselectdata"], "subgameselectdata")?;
        Ok(format!("{} sections", is_obj(j, "pages")?))
    });
    epic("epic calendar timeline", "GET", FN_HOST, "/fortnite/api/calendar/v1/timeline", "", 200, |j, _| {
        let n = is_obj(&j["channels"], "channels")?;
        j["currentTime"].as_str().ok_or("no currentTime")?;
        Ok(format!("{n} channels"))
    });
    epic("epic friends recent", "GET", FN_HOST, "/friends/api/v1/{id}/recent/fortnite", "", 200, |j, _| is_arr(j, "recent").map(|n| format!("{n}")));
    epic("epic friends settings", "GET", FN_HOST, "/friends/api/v1/{id}/settings", "", 200, |j, _| is_obj(j, "settings").map(|n| format!("{n} keys")));
    epic("epic presence subscriptions", "GET", FN_HOST, "/presence/api/v1/_/{id}/settings/subscriptions", "", 200, |j, _| is_arr(j, "subscriptions").map(|n| format!("{n}")));
    epic("epic party notifications", "GET", FN_HOST, "/party/api/v1/Fortnite/user/{id}/notifications/undelivered/count", "", 200, |j, _| {
        j["pings"].as_u64().ok_or("no pings")?;
        j["invites"].as_u64().ok_or("no invites")?;
        Ok("pings/invites counts".into())
    });
    epic("epic catalog", "GET", FN_HOST, "/fortnite/api/storefront/v2/catalog", "", 200, |j, _| {
        let n = j["storefronts"].as_array().ok_or("no storefronts")?.len();
        if n == 0 {
            return Err("empty storefronts".into());
        }
        Ok(format!("{n} storefronts"))
    });
    epic("epic privacy", "GET", FN_HOST, "/fortnite/api/game/v2/privacy/account/{id}", "", 200, |j, _| {
        if j["accountId"] != account().as_str() {
            return Err(format!("accountId {}", j["accountId"]));
        }
        Ok(format!("optOutOfPublicLeaderboards {}", j["optOutOfPublicLeaderboards"]))
    });
    epic("epic QueryProfile creative", "POST", FN_HOST, "/fortnite/api/game/v2/profile/{id}/client/QueryProfile?profileId=creative&rvn=-1", "{}", 200, |j, _| full_profile(j, "creative").map(|(r, n)| format!("rvn {r}, {n} items")));
    epic("epic events download", "GET", FN_HOST, "/api/v1/events/Fortnite/download/{id}?region=NONE&platform=XSX&teamAccountIds={id}", "", 200, |j, _| {
        if j["player"]["accountId"] != account().as_str() {
            return Err(format!("player.accountId {}", j["player"]["accountId"]));
        }
        Ok(format!("{} events", j["events"].as_array().map_or(0, |a| a.len())))
    });
    epic("epic stats", "GET", FN_HOST, "/statsproxy/api/statsv2/account/{id}", "", 200, |j, _| {
        if j["accountId"] != account().as_str() {
            return Err(format!("accountId {}", j["accountId"]));
        }
        Ok("accountId matches".into())
    });
    epic("epic ClientQuestLogin", "POST", FN_HOST, "/fortnite/api/game/v2/profile/{id}/client/ClientQuestLogin?profileId=athena&rvn={athena_rvn}", "{}", 200, |j, _| profile(j, "athena").map(|r| format!("rvn {r}, {} changes", j["profileChanges"].as_array().map_or(0, |a| a.len()))));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("xhost [--dir <dll dir>] [--order a.dll,b.dll,...] [--net-trigger xmem|mfplat] [--step-ms N] [--async real|emulated] [--no-epic] [--load-only] [--repeat N] [--json <file>]");
        println!("default dir: target-qa\\release (or debug); default order (game import order): {GAME_ORDER}");
        return;
    }
    if let Some(ms) = arg(&args, "--step-ms").and_then(|v| v.parse().ok()) {
        STEP_MS.store(ms, Ordering::Relaxed);
    }
    let dir = arg(&args, "--dir").map(PathBuf::from).or_else(|| std::env::var_os("XHOST_DLL_DIR").map(PathBuf::from)).unwrap_or_else(default_dir);
    let order: Vec<String> = arg(&args, "--order").unwrap_or_else(|| GAME_ORDER.into()).split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let trigger = arg(&args, "--net-trigger").unwrap_or_else(|| "xmem".into());
    let repeat: usize = arg(&args, "--repeat").and_then(|v| v.parse().ok()).unwrap_or(1).max(1);
    let load_only = args.iter().any(|a| a == "--load-only");
    if load_only && order.is_empty() {
        eprintln!("--load-only requires at least one DLL in the load order");
        std::process::exit(2);
    }
    let epic = !load_only && !args.iter().any(|a| a == "--no-epic");
    if args.iter().any(|a| a == "--audio") {
        let frames = arg(&args, "--frames").and_then(|v| v.parse().ok()).unwrap_or(300);
        if args.iter().any(|a| a == "--gc2") {
            match audio::gamechat2(&dir, frames) {
                Ok(r) => {
                    println!("PASS  GameChat2 call pattern (heap validated): {r}");
                    std::process::exit(0)
                }
                Err(e) => {
                    println!("FAIL  GameChat2 call pattern: {e}");
                    std::process::exit(1)
                }
            }
        }
        match audio::run(&dir, frames) {
            Ok(r) => {
                println!("PASS  audio codecs (heap validated): {r}");
                std::process::exit(0)
            }
            Err(e) => {
                println!("FAIL  audio codecs: {e}");
                std::process::exit(1)
            }
        }
    }
    if args.iter().any(|a| a == "--heapcheck") || std::env::var_os("XHOST_HEAPCHECK").is_some() {
        heapcheck::enable();
        println!("heapcheck: HeapValidate on all process heaps after every step");
    }
    if let Some(j) = arg(&args, "--json") {
        let cfg = serde_json::json!({"dir": dir.display().to_string(), "order": order, "net_trigger": trigger, "repeat": repeat, "epic": epic, "load_only": load_only, "step_ms": STEP_MS.load(Ordering::Relaxed)}).to_string();
        *JSON_OUT.lock().unwrap() = Some((PathBuf::from(j), cfg));
    }
    println!("xhost: dll dir {}; order {}; net_guard trigger {trigger}", dir.display(), order.join(" > "));
    let (st, have) = match stage(&dir, &order) {
        Ok(x) => x,
        Err(e) => {
            println!("SKIP  stage: {e} (build: cargo build --release -p d3d12_xs -p xmem -p acphal -p dxcompiler_xs -p mfplat -p xcrdapi -p audiohalwrapper -p xaudio2_9)");
            std::process::exit(3);
        }
    };
    let log = setup_env(&st, load_only);
    if let Some(a) = arg(&args, "--async") {
        std::env::set_var("TOMBOX_GDK_XASYNC", a);
    }
    *CTX.lock().unwrap() = Some(Ctx {
        modules: Vec::new(), query: 0, xgr: 0, gdk_owner: String::new(), net_owner: String::new(), user: 0,
        token: String::new(), client_token: String::new(), account_id: String::new(), access_token: String::new(),
        port: 0, stage: st.clone(), log: log.clone(), gate: false, async_impl: String::new(),
    });
    println!("xhost: staged in {}; TOMBOX_LOG={}", st.display(), log.display());

    for n in have.clone() {
        let p = st.join(&n);
        step(&format!("load {n}"), move || unsafe {
            let h = LoadLibraryW(w(&p.to_string_lossy()).as_ptr());
            if h.is_null() {
                return fail(format!("LoadLibraryW({}) err={}", p.display(), GetLastError()));
            }
            if load_only {
                let loaded = module_path(h).ok_or_else(|| Out::Fail("GetModuleFileNameW could not identify the loaded DLL".into()))?;
                let actual = loaded.canonicalize().map_err(|e| Out::Fail(format!("resolve {}: {e}", loaded.display())))?;
                let expected = p.canonicalize().map_err(|e| Out::Fail(format!("resolve {}: {e}", p.display())))?;
                check(actual == expected, || format!("loaded {} instead of {}", actual.display(), expected.display()))?;
            } else {
                let loaded = mod_name(h as usize);
                check(loaded.eq_ignore_ascii_case(&n), || format!("loaded module is {loaded}, not the staged {n}"))?;
            }
            with(|c| c.modules.push((n.clone(), h as usize)));
            Ok(format!("base {:#x}", h as usize))
        });
    }

    if load_only {
        if have.is_empty() {
            report("load order", Out::Fail("none of the requested DLLs were staged".into()));
        }
        finish();
    }

    step("ownership after load", || {
        let gdk = env_is_pid("TOMBOX_GDK_OWNER");
        let os = env_is_pid("TOMBOX_OSHOST_OWNER");
        let sig = std::env::var("TOMBOX_OSHOST_SIGNAL").ok().and_then(|v| usize::from_str_radix(&v, 16).ok()).unwrap_or(0);
        check(gdk, || "TOMBOX_GDK_OWNER not claimed by this process (no compat DLL ran the GDK early init)".into())?;
        let os_owner = if sig != 0 { mod_name(sig) } else { "-".into() };
        Ok(format!("GDK early-init claimed; os_host owner {} ({os_owner})", if os { "claimed" } else { "not yet" }))
    });

    let t = trigger.clone();
    step("net_guard install", move || unsafe {
        match t.as_str() {
            "mfplat" => {
                let f = export("mfplat.dll", "MFStartup");
                check(!f.is_null(), || "mfplat!MFStartup not exported".into())?;
                let f: unsafe extern "system" fn(u32, u32) -> i32 = core::mem::transmute(f);
                let hr = f(0x0002_0070, 0);
                check(hr >= 0, || format!("MFStartup hr={hr:#010x}"))?;
            }
            _ => {
                let f = export("xmem.dll", "XMemGetPartitionHandleAndId");
                check(!f.is_null(), || "xmem!XMemGetPartitionHandleAndId not exported".into())?;
                let f: unsafe extern "system" fn(u32, *mut usize, *mut u32) -> u8 = core::mem::transmute(f);
                let (mut h, mut id) = (0usize, 0u32);
                f(0, &mut h, &mut id);
            }
        }
        let owner = wait_env("TOMBOX_NET_GUARD_OWNER", wait_ms());
        check(owner.as_deref() == Some(std::process::id().to_string().as_str()), || format!("TOMBOX_NET_GUARD_OWNER={owner:?} after the trigger call"))?;
        let port = wait_env("TOMBOX_NET_INPROC_PORT", wait_ms()).and_then(|p| p.parse::<u16>().ok()).unwrap_or(0);
        check(port != 0, || "TOMBOX_NET_INPROC_PORT not published: builtin backend did not start".into())?;
        with(|c| c.port = port);
        Ok(format!("policy installed by this process via {t}; builtin backend 127.0.0.1:{port}"))
    });

    step("network safety gate", || unsafe {
        let entries = [("ws2_32.dll", "getaddrinfo"), ("ws2_32.dll", "GetAddrInfoW"), ("ws2_32.dll", "connect"), ("winhttp.dll", "WinHttpConnect"), ("winhttp.dll", "WinHttpOpenRequest"), ("winhttp.dll", "WinHttpSendRequest")];
        let staged: Vec<String> = with(|c| c.modules.iter().map(|(n, _)| n.to_ascii_lowercase()).collect());
        let mut owners = Vec::new();
        for (dll, f) in entries {
            let p = export(dll, f);
            let Some(t) = jump_target(p) else { return fail(format!("{dll}!{f} is not hooked (no mov rax/jmp rax prologue)")) };
            let m = mod_name(t);
            check(staged.contains(&m.to_ascii_lowercase()), || format!("{dll}!{f} jumps to {m}, not a staged compat DLL"))?;
            owners.push(m);
        }
        owners.dedup();
        check(owners.len() == 1, || format!("network hooks split across DLL copies: {owners:?}"))?;
        with(|c| {
            c.net_owner = owners[0].clone();
            c.gate = true;
        });
        Ok(format!("6 entry points redirect into {}", owners[0]))
    });

    let gdk_ok = step("xgameruntime QueryApiImpl hook", || unsafe {
        let h = LoadLibraryExW(w("xgameruntime.dll").as_ptr(), core::ptr::null_mut(), 0x800);
        if h.is_null() {
            return Err(Out::Skip("System32 xgameruntime.dll not present (Gaming Services not installed)".into()));
        }
        let q = GetProcAddress(h, b"QueryApiImpl\0".as_ptr()) as usize;
        let qm = mod_name(q);
        check(!qm.eq_ignore_ascii_case("xgameruntime.dll"), || format!("GetProcAddress(xgameruntime, QueryApiImpl) returned the real {q:#x}: import hook not applied to the host"))?;
        let order: Vec<String> = with(|c| c.modules.iter().map(|(n, _)| n.clone()).collect());
        check(order.iter().any(|n| n.eq_ignore_ascii_case(&qm)), || format!("QueryApiImpl resolved into {qm}, not a staged compat DLL"))?;
        with(|c| {
            c.query = q;
            c.xgr = h as usize;
            c.gdk_owner = qm.clone();
        });
        let first = order.first().cloned().unwrap_or_default();
        Ok(format!("hooked -> {qm} (gdk owner; first loaded {first})"))
    });

    if gdk_ok {
        step("InitializeApiImpl", || unsafe {
            let h = with(|c| c.xgr) as *mut c_void;
            let f = GetProcAddress(h, b"InitializeApiImpl\0".as_ptr());
            check(!f.is_null(), || "xgameruntime!InitializeApiImpl missing".into())?;
            let fm = mod_name(f as usize);
            check(fm.eq_ignore_ascii_case("xgameruntime.dll"), || format!("InitializeApiImpl resolved into {fm}"))?;
            let f: unsafe extern "system" fn(u32, u32) -> i32 = core::mem::transmute(f);
            let hr = f(0x4a61, 0x2712);
            check(hr >= 0, || format!("InitializeApiImpl(0x4a61, 0x2712) hr={hr:#010x}"))?;
            Ok(format!("hr={hr:#010x}"))
        });

        step("XAsync interface", || unsafe {
            let xa = XA::get()?;
            let m = mod_name(slot(xa.obj, 3));
            let real = m.eq_ignore_ascii_case("xgameruntime.dll");
            let gdk = with(|c| c.gdk_owner.clone());
            check(real || m.eq_ignore_ascii_case(&gdk), || format!("XAsyncGetStatus lives in {m}, neither the real runtime nor the gdk owner {gdk}"))?;
            with(|c| c.async_impl = m.clone());
            Ok(format!("XAsyncGetStatus in {m} ({})", if real { "Gaming Runtime implementation" } else { "emulated in gdk owner" }))
        });

        step("XTaskQueue manual + XAsyncRun", || unsafe {
            let xa = XA::get()?;
            CB_XA.store(xa.obj as usize, Ordering::SeqCst);
            let (hr, q) = xa.queue_create(0, 0);
            check(hr == S_OK && q != 0, || format!("XTaskQueueCreate(Manual, Manual) hr={hr:#010x} q={q:#x}"))?;
            let (w0, c0) = (WORK_RAN.load(Ordering::SeqCst), CALLBACK_RAN.load(Ordering::SeqCst));
            let mut b = Block::boxed(q);
            b.callback = run_done as *const () as usize;
            let hr = xa.run(b.ptr(), run_work);
            check(hr == S_OK, || format!("XAsyncRun hr={hr:#010x}"))?;
            check(WORK_RAN.load(Ordering::SeqCst) == w0, || "work ran before the work port was dispatched".into())?;
            let st = xa.status(b.ptr(), false);
            check(st == E_PENDING, || format!("status before dispatch {st:#010x}, want E_PENDING"))?;
            let mut d = 0;
            let start = Instant::now();
            while WORK_RAN.load(Ordering::SeqCst) == w0 && start.elapsed() < Duration::from_millis(wait_ms()) {
                if xa.dispatch(q, 0, 50) {
                    d += 1;
                }
            }
            check(WORK_RAN.load(Ordering::SeqCst) == w0 + 1, || "work callback never ran on the manual work port".into())?;
            let start = Instant::now();
            while CALLBACK_RAN.load(Ordering::SeqCst) == c0 && start.elapsed() < Duration::from_millis(wait_ms()) {
                xa.dispatch(q, 1, 50);
            }
            check(CALLBACK_RAN.load(Ordering::SeqCst) == c0 + 1, || "completion callback never ran on the manual completion port".into())?;
            let cb_hr = CALLBACK_HR.load(Ordering::SeqCst);
            check(cb_hr == S_OK, || format!("status inside completion callback {cb_hr:#010x}"))?;
            let st = xa.status(b.ptr(), false);
            check(st == S_OK, || format!("final status {st:#010x}"))?;
            xa.queue_close(q);
            std::mem::forget(b);
            Ok(format!("work + completion dispatched on manual ports ({d} work dispatch)"))
        });

        step("XAsyncBegin provider + GetResult", || unsafe {
            let xa = XA::get()?;
            let job = Box::leak(Box::new(Job {
                xa: xa.obj as usize,
                work: Mutex::new(Some(Box::new(|| {
                    std::thread::sleep(Duration::from_millis(20));
                    Ok(0x1122_3344_5566_7788u64.to_le_bytes().to_vec())
                }))),
                result: Mutex::new(None),
                cancels: AtomicUsize::new(0),
                cleaned: AtomicUsize::new(0),
            }));
            let mut b = Block::boxed(0);
            let id = job_provider as *const () as usize;
            let hr = xa.begin(b.ptr(), job as *mut Job as *mut c_void, id, b"XhostValueAsync\0", job_provider);
            check(hr == S_OK, || format!("XAsyncBegin hr={hr:#010x}"))?;
            let hr = xa.schedule(b.ptr(), 0);
            check(hr == S_OK, || format!("XAsyncSchedule hr={hr:#010x}"))?;
            let v = xa.finish(b.ptr(), id, "value")?;
            check(v == 0x1122_3344_5566_7788u64.to_le_bytes(), || format!("result bytes {v:02x?}"))?;
            let start = Instant::now();
            while job.cleaned.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_millis(500) {
                std::thread::sleep(Duration::from_millis(2));
            }
            check(job.cleaned.load(Ordering::SeqCst) == 1, || format!("provider Cleanup ran {} times after GetResult", job.cleaned.load(Ordering::SeqCst)))?;
            std::mem::forget(b);
            Ok("8-byte result returned, Cleanup once".into())
        });

        step("XAsyncCancel", || unsafe {
            let xa = XA::get()?;
            let job = Box::leak(Box::new(Job { xa: xa.obj as usize, work: Mutex::new(None), result: Mutex::new(None), cancels: AtomicUsize::new(0), cleaned: AtomicUsize::new(0) }));
            let mut b = Block::boxed(0);
            let id = job_provider as *const () as usize;
            let hr = xa.begin(b.ptr(), job as *mut Job as *mut c_void, id, b"XhostCancelAsync\0", job_provider);
            check(hr == S_OK, || format!("XAsyncBegin hr={hr:#010x}"))?;
            let hr = xa.schedule(b.ptr(), 0);
            check(hr == S_OK, || format!("XAsyncSchedule hr={hr:#010x}"))?;
            std::thread::sleep(Duration::from_millis(30));
            let st = xa.status(b.ptr(), false);
            check(st == E_PENDING, || format!("status before cancel {st:#010x}"))?;
            xa.cancel(b.ptr());
            let st = xa.poll(b.ptr(), wait_ms());
            check(st == E_ABORT, || format!("status after XAsyncCancel {st:#010x}, want E_ABORT"))?;
            check(job.cancels.load(Ordering::SeqCst) == 1, || format!("provider Cancel op ran {} times", job.cancels.load(Ordering::SeqCst)))?;
            std::mem::forget(b);
            Ok("E_ABORT after provider Cancel".into())
        });

        step("XUser sign-in (QueryApiImpl)", || unsafe {
            let xa = XA::get()?;
            let u = query(XUSER, XUSER_V1)?;
            let add: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32 = core::mem::transmute(slot(u, 7));
            let add_res: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut usize) -> i32 = core::mem::transmute(slot(u, 8));
            let mut b = Block::boxed(0);
            let hr = add(u, 1, b.ptr());
            check(hr == S_OK, || format!("XUserAddAsync hr={hr:#010x}"))?;
            let hr = xa.poll(b.ptr(), wait_ms());
            check(hr == S_OK, || format!("XUserAddAsync status {hr:#010x}"))?;
            let mut user = 0usize;
            let hr = add_res(u, b.ptr(), &mut user);
            check(hr == S_OK && user != 0, || format!("XUserAddResult hr={hr:#010x} user={user:#x}"))?;
            let um = mod_name(user);
            let gdk = with(|c| c.gdk_owner.clone());
            check(um.eq_ignore_ascii_case(&gdk), || format!("user handle {user:#x} lives in {um}, not the gdk owner {gdk}"))?;
            let get_id: unsafe extern "system" fn(*mut c_void, usize, *mut u64) -> i32 = core::mem::transmute(slot(u, 11));
            let mut xuid = 0u64;
            let hr = get_id(u, user, &mut xuid);
            check(hr == S_OK && xuid != 0, || format!("XUserGetId hr={hr:#010x} xuid={xuid}"))?;
            let find_id: unsafe extern "system" fn(*mut c_void, u64, *mut usize) -> i32 = core::mem::transmute(slot(u, 12));
            let mut u2 = 0usize;
            let hr = find_id(u, xuid, &mut u2);
            check(hr == S_OK && u2 == user, || format!("XUserFindUserById hr={hr:#010x} -> {u2:#x}, want {user:#x}"))?;
            let find_local: unsafe extern "system" fn(*mut c_void, u64, *mut usize) -> i32 = core::mem::transmute(slot(u, 10));
            let mut u3 = 0usize;
            let hr = find_local(u, 1, &mut u3);
            check(hr == S_OK && u3 == user, || format!("XUserFindUserByLocalId(1) hr={hr:#010x} -> {u3:#x}"))?;
            let state: unsafe extern "system" fn(*mut c_void, usize, *mut u32) -> i32 = core::mem::transmute(slot(u, 14));
            let mut s = 9u32;
            let hr = state(u, user, &mut s);
            check(hr == S_OK && s == 0, || format!("XUserGetState hr={hr:#010x} state={s}"))?;
            let priv_: unsafe extern "system" fn(*mut c_void, usize, u32, u32, *mut bool, *mut u32) -> i32 = core::mem::transmute(slot(u, 20));
            let (mut has, mut why) = (false, 9u32);
            let hr = priv_(u, user, 0, 254, &mut has, &mut why);
            check(hr == S_OK && has, || format!("XUserCheckPrivilege(Multiplayer) hr={hr:#010x} has={has}"))?;
            let g = query(XUSER, XUSER_GT)?;
            let gt: unsafe extern "system" fn(*mut c_void, usize, u32, usize, *mut u8, *mut usize) -> i32 = core::mem::transmute(slot(g, 3));
            let mut buf = [0u8; 64];
            let mut used = 0usize;
            let hr = gt(g, user, 0, buf.len(), buf.as_mut_ptr(), &mut used);
            check(hr == S_OK && used > 1, || format!("XUserGetGamertag hr={hr:#010x} used={used}"))?;
            let tag = String::from_utf8_lossy(&buf[..used - 1]).into_owned();
            let u2o = query(XUSER, XUSER_V2)?;
            let add2: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32 = core::mem::transmute(slot(u2o, 7));
            let add2_res: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut usize) -> i32 = core::mem::transmute(slot(u2o, 8));
            let mut b2 = Block::boxed(0);
            check(add2(u2o, 1, b2.ptr()) == S_OK, || "XUserAddAsync (v2 iid) failed".into())?;
            check(xa.poll(b2.ptr(), wait_ms()) == S_OK, || "XUserAddAsync (v2 iid) did not complete".into())?;
            let mut uv2 = 0usize;
            let hr = add2_res(u2o, b2.ptr(), &mut uv2);
            check(hr == S_OK && uv2 == user, || format!("v2 XUserAddResult -> {uv2:#x}, v1 gave {user:#x}"))?;
            with(|c| c.user = user);
            XUID.store(xuid, Ordering::SeqCst);
            std::mem::forget(b);
            std::mem::forget(b2);
            Ok(format!("user {user:#x} in {um}, xuid {xuid}, gamertag '{tag}', v1/v2 handles agree"))
        });

        if let Some(gc) = arg(&args, "--chat").or_else(|| args.iter().any(|a| a == "--chat-default").then(|| r"G:\FortniteGame\Binaries\XSX\GameChat2.dll".to_string())) {
            let secs: u64 = arg(&args, "--seconds").and_then(|v| v.parse().ok()).unwrap_or(15);
            let ok = step("GameChat2 voice pipeline (heap watched every 0.5 ms)", move || {
                let xuid = XUID.load(Ordering::SeqCst);
                if std::env::var_os("XHOST_FAKE_ENDPOINT").is_some() {
                    unsafe {
                        let dev = query("7d824997-10dc-45ab-86b7-2737767c0bf1", "7d824997-10dc-45ab-86b7-2737767c0bf1")?;
                        let vt = *(dev as *const *mut usize);
                        let mut old = 0u32;
                        VirtualProtect(vt.add(6) as *mut c_void, 8, 0x04, &mut old);
                        *vt.add(6) = fake_endpoint as usize;
                        VirtualProtect(vt.add(6) as *mut c_void, 8, old, &mut old);
                    }
                    println!("      XUserGetDefaultAudioEndpointUtf16 patched to return a non-existent endpoint id (no device is opened)");
                }
                let mut newmods = Vec::new();
                let r = chat::run(Path::new(&gc), xuid, secs, &mut newmods);
                println!("      default audio endpoint queries: {}", FAKE_EP_CALLS.load(Ordering::SeqCst));
                for m in &newmods {
                    println!("      loaded by GameChat2: {m}");
                }
                match r {
                    Ok(s) => Ok(s),
                    Err(e) => fail(e),
                }
            });
            std::process::exit(if ok { 0 } else { 1 });
        }

        step("XUserGetTokenAndSignature", || unsafe {
            let xa = XA::get()?;
            let user = with(|c| c.user);
            check(user != 0, || "no signed-in user".into())?;
            let u = query(XUSER, XUSER_V1)?;
            let url = "https://account-public-service-prod.ol.epicgames.com/account/api/oauth/token";
            let tok: unsafe extern "system" fn(*mut c_void, usize, u32, *const u8, *const u8, usize, *const c_void, usize, *const c_void, *mut c_void) -> i32 = core::mem::transmute(slot(u, 23));
            let tok_res: unsafe extern "system" fn(*mut c_void, *mut c_void, usize, *mut u8, *mut *mut u8, *mut usize) -> i32 = core::mem::transmute(slot(u, 25));
            let mut b = Block::boxed(0);
            let hr = tok(u, user, 0, b"POST\0".as_ptr(), format!("{url}\0").as_ptr(), 0, core::ptr::null(), 0, core::ptr::null(), b.ptr());
            check(hr == S_OK, || format!("XUserGetTokenAndSignatureAsync hr={hr:#010x}"))?;
            check(xa.poll(b.ptr(), wait_ms()) == S_OK, || "token async did not complete".into())?;
            let (hr, n) = xa.size(b.ptr());
            check(hr == S_OK && n > 32, || format!("token result size hr={hr:#010x} n={n}"))?;
            let mut buf = vec![0u8; n];
            let mut ptr: *mut u8 = core::ptr::null_mut();
            let mut used = 0usize;
            let hr = tok_res(u, b.ptr(), n, buf.as_mut_ptr(), &mut ptr, &mut used);
            check(hr == S_OK && ptr == buf.as_mut_ptr(), || format!("XUserGetTokenAndSignatureResult hr={hr:#010x}"))?;
            let tp = core::ptr::read_unaligned(buf.as_ptr().add(16) as *const usize);
            let inside = tp >= buf.as_ptr() as usize && tp < buf.as_ptr() as usize + n;
            check(inside, || format!("token pointer {tp:#x} not fixed up into the result buffer"))?;
            let token = std::ffi::CStr::from_ptr(tp as *const i8).to_string_lossy().into_owned();
            check(token.starts_with("XBL3.0 x="), || format!("token '{token}'"))?;
            let tok16: unsafe extern "system" fn(*mut c_void, usize, u32, *const u16, *const u16, usize, *const c_void, usize, *const c_void, *mut c_void) -> i32 = core::mem::transmute(slot(u, 26));
            let tok16_res: unsafe extern "system" fn(*mut c_void, *mut c_void, usize, *mut u8, *mut *mut u8, *mut usize) -> i32 = core::mem::transmute(slot(u, 28));
            let mut b2 = Block::boxed(0);
            let hr = tok16(u, user, 0, w("POST").as_ptr(), w(url).as_ptr(), 0, core::ptr::null(), 0, core::ptr::null(), b2.ptr());
            check(hr == S_OK && xa.poll(b2.ptr(), wait_ms()) == S_OK, || format!("Utf16 token async hr={hr:#010x}"))?;
            let (_, n2) = xa.size(b2.ptr());
            let mut buf2 = vec![0u8; n2];
            let mut p2: *mut u8 = core::ptr::null_mut();
            let hr = tok16_res(u, b2.ptr(), n2, buf2.as_mut_ptr(), &mut p2, &mut used);
            check(hr == S_OK, || format!("Utf16 token result hr={hr:#010x}"))?;
            let tp2 = core::ptr::read_unaligned(buf2.as_ptr().add(16) as *const usize) as *const u16;
            let cnt = core::ptr::read_unaligned(buf2.as_ptr() as *const usize);
            let t16 = String::from_utf16_lossy(core::slice::from_raw_parts(tp2, cnt.saturating_sub(1)));
            check(t16 == token, || format!("Utf16 token '{t16}' differs from ANSI '{token}'"))?;
            with(|c| c.token = token.clone());
            std::mem::forget(b);
            std::mem::forget(b2);
            Ok(format!("'{token}' (ANSI and Utf16 agree)"))
        });

        step("XUser handle identity across DLL copies", || unsafe {
            let user = with(|c| c.user);
            let (gdk, net) = with(|c| (c.gdk_owner.clone(), c.net_owner.clone()));
            check(user != 0 && !net.is_empty(), || "needs sign-in and net_guard owner".into())?;
            let mut rows = Vec::new();
            for (n, b) in with(|c| c.modules.clone()) {
                for a in anchors(b) {
                    rows.push((n.clone(), a));
                }
            }
            let of = |m: &str| rows.iter().filter(|(n, _)| n.eq_ignore_ascii_case(m)).map(|r| r.1).collect::<Vec<usize>>();
            let own = of(&gdk);
            check(own == vec![user], || format!("gdk owner {gdk} user anchors {own:x?}, want exactly [XUserAddResult {user:#x}]"))?;
            let wraps = of(&net);
            check(wraps.len() == 1, || format!("net_guard owner {net} has {} user anchors {wraps:x?}, want exactly one", wraps.len()))?;
            let wrap = wraps[0];
            let u = query(XUSER, XUSER_V1)?;
            let cmp: unsafe extern "system" fn(*mut c_void, usize, usize) -> i32 = core::mem::transmute(slot(u, 5));
            let c = cmp(u, user, wrap);
            check(c == 0, || format!(
                "the title's EXE XUser wrappers would return {wrap:#x} (gdk_user::handle -> gdk::user::HANDLE_ANCHOR in {net}, the net_guard owner that runs gdk_user::install), QueryApiImpl XUser returns {user:#x} (HANDLE_ANCHOR in {gdk}, the gdk owner); XUserCompare = {c}: one signed-in user has two handles depending on which DLL copy answered"))?;
            Ok(format!("one handle {user:#x} ({gdk} == {net})"))
        });

        step("XNetworkingQuerySecurityInformationForUrl", || unsafe {
            let xa = XA::get()?;
            let n = query(XNET, XNET_V2)?;
            let gdk = with(|c| c.gdk_owner.clone());
            let vm = mod_name(slot(n, 14));
            check(vm.eq_ignore_ascii_case(&gdk), || format!("XNetworkingVerifyServerCertificate lives in {vm}, gdk owner is {gdk}"))?;
            VOBJ.store(n as usize, Ordering::SeqCst);
            VFN.store(slot(n, 14), Ordering::SeqCst);
            let sec: unsafe extern "system" fn(*mut c_void, *const u16, *mut c_void) -> i32 = core::mem::transmute(slot(n, 11));
            let res: unsafe extern "system" fn(*mut c_void, *mut c_void, usize, *mut usize, *mut u8, *mut *mut u8) -> i32 = core::mem::transmute(slot(n, 13));
            let url = w("https://datarouter.ol.epicgames.com/datarouter/api/v1/public/data");
            let mut b = Block::boxed(0);
            let hr = sec(n, url.as_ptr(), b.ptr());
            check(hr == S_OK && xa.poll(b.ptr(), wait_ms()) == S_OK, || format!("Utf16Async hr={hr:#010x}"))?;
            let (_, sz) = xa.size(b.ptr());
            let mut info = vec![0u8; sz];
            let (mut used, mut ptr) = (0usize, core::ptr::null_mut());
            let hr = res(n, b.ptr(), info.len(), &mut used, info.as_mut_ptr(), &mut ptr);
            check(hr == S_OK && used == 24 && ptr == info.as_mut_ptr(), || format!("Utf16Result hr={hr:#010x} used={used}"))?;
            let flags = u32::from_le_bytes(info[0..4].try_into().unwrap());
            check(flags == 0x2800, || format!("enabled protocols {flags:#x}, want TLS1.2|TLS1.3 0x2800"))?;
            std::mem::forget(b);
            Ok(format!("TLS flags {flags:#x}, no pins; verifier in {vm}"))
        });

        step("XNetworkingVerifyServerCertificate rejects non-backend", || unsafe {
            let (v, f) = (VOBJ.load(Ordering::SeqCst), VFN.load(Ordering::SeqCst));
            check(v != 0 && f != 0, || "verifier not resolved".into())?;
            let s = session() as *mut c_void;
            let c = WinHttpConnect(s, w("localhost").as_ptr(), 443, 0);
            check(!c.is_null(), || format!("WinHttpConnect(localhost) err={}", GetLastError()))?;
            let r = WinHttpOpenRequest(c, w("GET").as_ptr(), w("/").as_ptr(), core::ptr::null(), core::ptr::null(), core::ptr::null(), 0x0080_0000);
            let url = request_url(r);
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *const u8) -> i32 = core::mem::transmute(f);
            let hr = f(v as *mut c_void, r, core::ptr::null());
            WinHttpCloseHandle(r);
            WinHttpCloseHandle(c);
            check(hr < 0, || format!("verify accepted unsent request '{url}' with no certificate (hr={hr:#010x})"))?;
            Ok(format!("'{url}' (never sent) rejected hr={hr:#010x}"))
        });
    }

    let net_ready = with(|c| c.gate) && VFN.load(Ordering::SeqCst) != 0;
    if net_ready {
    for it in 0..repeat {
        QUIET.store(it > 0, Ordering::Relaxed);
        step("login datarouter POST", || {
            let r = http_async("POST", "datarouter.ol.epicgames.com", "/datarouter/api/v1/public/data?SessionID=&AppID=Fortnite.Fortnite&AppVersion=1.0.0&UserID=%7C%7C%7C", "Content-Type: application/json\r\n", b"{\"Events\":[]}")?;
            hr_ok(&r, 204, "datarouter")?;
            Ok(format!("204 via {}", r.url.split('?').next().unwrap_or("")))
        });

        step("login oauth client_credentials", || {
            let hdr = format!("Content-Type: application/x-www-form-urlencoded\r\nAuthorization: basic {}\r\n", b64(FN_CLIENT.as_bytes()));
            let r = http_async("POST", "account-public-service-prod.ol.epicgames.com", "/account/api/oauth/token", &hdr, b"grant_type=client_credentials&token_type=eg1")?;
            hr_ok(&r, 200, "oauth client_credentials")?;
            let j = json(&r);
            let at = j["access_token"].as_str().unwrap_or("");
            check(at.starts_with("eg1~"), || format!("access_token '{at}'"))?;
            check(j["token_type"] == "bearer" && j["client_id"] == FN_CLIENT.split(':').next().unwrap(), || format!("token_type/client_id wrong: {j}"))?;
            with(|c| c.client_token = at.to_string());
            Ok(format!("eg1 client token, expires_in {}", j["expires_in"]))
        });

        step("login waitingroom", || {
            let at = with(|c| c.client_token.clone());
            let r = http_async("GET", "launcher-public-service-prod06.ol.epicgames.com", "/waitingroom/api/waitingroom", &format!("Authorization: bearer {at}\r\n"), b"")?;
            hr_ok(&r, 204, "waitingroom")?;
            Ok("204 (no queue)".into())
        });

        step("login oauth external_auth xbl_xtoken", || {
            let token = with(|c| c.token.clone());
            check(!token.is_empty(), || "no XUser token (GDK steps skipped or failed)".into())?;
            let hdr = format!("Content-Type: application/x-www-form-urlencoded\r\nAuthorization: basic {}\r\n", b64(FN_CLIENT.as_bytes()));
            let body = format!("grant_type=external_auth&external_auth_type=xbl_xtoken&external_auth_token={}&token_type=eg1", urlenc(&token));
            let r = http_async("POST", "account-public-service-prod.ol.epicgames.com", "/account/api/oauth/token", &hdr, body.as_bytes())?;
            hr_ok(&r, 200, "oauth external_auth")?;
            let j = json(&r);
            let at = j["access_token"].as_str().unwrap_or("").to_string();
            let id = j["account_id"].as_str().unwrap_or("").to_string();
            check(at.starts_with("eg1~") && !id.is_empty(), || format!("missing access_token/account_id: {j}"))?;
            check(j["refresh_token"].as_str().map_or(false, |t| t.starts_with("eg1~")), || format!("refresh_token: {}", j["refresh_token"]))?;
            with(|c| {
                c.access_token = at;
                c.account_id = id.clone();
            });
            Ok(format!("account_id {id}, displayName {}", j["displayName"]))
        });

        step("login oauth verify", || {
            let (at, id) = with(|c| (c.access_token.clone(), c.account_id.clone()));
            check(!at.is_empty(), || "no account token".into())?;
            let r = http_async("GET", "account-public-service-prod.ol.epicgames.com", "/account/api/oauth/verify", &format!("Authorization: bearer {at}\r\n"), b"")?;
            hr_ok(&r, 200, "oauth verify")?;
            let j = json(&r);
            check(j["token"] == at.as_str(), || "verify did not echo the bearer token".into())?;
            check(j["account_id"] == id.as_str(), || format!("verify account_id {} != login {id}", j["account_id"]))?;
            Ok("bearer token and account_id consistent".into())
        });

        step("login account lookup", || {
            let (at, id) = with(|c| (c.access_token.clone(), c.account_id.clone()));
            check(!at.is_empty(), || "no account token".into())?;
            let r = http_async("GET", "account-public-service-prod.ol.epicgames.com", &format!("/account/api/public/account/{id}"), &format!("Authorization: bearer {at}\r\n"), b"")?;
            hr_ok(&r, 200, "account lookup")?;
            let j = json(&r);
            check(j["id"] == id.as_str(), || format!("account id {} != {id}", j["id"]))?;
            Ok(format!("displayName {}", j["displayName"]))
        });

        step("login oauth sessions kill", || {
            let at = with(|c| c.access_token.clone());
            check(!at.is_empty(), || "no account token".into())?;
            let r = http_async("DELETE", "account-public-service-prod.ol.epicgames.com", "/account/api/oauth/sessions/kill?killType=OTHERS_ACCOUNT_CLIENT_SERVICE", &format!("Authorization: bearer {at}\r\n"), b"")?;
            hr_ok(&r, 204, "sessions kill")?;
            Ok("204".into())
        });
        if epic {
            epic_flow();
        }
    }
        QUIET.store(false, Ordering::Relaxed);
    } else {
        report("online flow", Out::Skip("network safety gate or XNetworking verifier unavailable; no HTTP issued".into()));
    }

    step("compat log consistency", || {
        let log = with(|c| c.log.clone());
        let text = std::fs::read_to_string(&log).map_err(|e| Out::Fail(format!("read {}: {e}", log.display())))?;
        let mut bad = Vec::new();
        let mut impls = Vec::new();
        for l in text.lines() {
            let s = l.splitn(2, ' ').nth(1).unwrap_or(l);
            if s.contains("[gdk] xasync XAsync/XTaskQueue") {
                impls.push(s.to_string());
            }
            for pat in ["NOT PATCHED", "has no known owner - dropped", "unconfirmed-slot", "falling back to deny-all", "WARNING xgameruntime.dll was already loaded", "[online] panic", "[net_guard] denied"] {
                if s.contains(pat) {
                    bad.push(s.chars().take(220).collect::<String>());
                }
            }
            if s.starts_with("[online] unhandled") {
                bad.push(s.chars().take(220).collect::<String>());
            }
        }
        impls.dedup();
        check(impls.len() <= 1, || format!("XAsync implementation decided differently: {impls:?}"))?;
        check(bad.is_empty(), || format!("{} problem line(s): {}", bad.len(), bad.join(" | ")))?;
        let gdk_user = text.lines().find(|l| l.contains("[gdk_user] install")).map(|l| l.splitn(2, ' ').nth(1).unwrap_or(l).chars().take(160).collect::<String>()).unwrap_or_default();
        Ok(format!("{} lines; {}", text.lines().count(), if gdk_user.is_empty() { "no gdk_user line".into() } else { gdk_user }))
    });

    let (g, n, a, st) = with(|c| (c.gdk_owner.clone(), c.net_owner.clone(), c.async_impl.clone(), c.stage.clone()));
    let sig = std::env::var("TOMBOX_OSHOST_SIGNAL").ok().and_then(|v| usize::from_str_radix(&v, 16).ok()).map(mod_name).unwrap_or_else(|| "-".into());
    println!("OWNERS: gdk={g} net_guard={n} os_host={sig} xasync={a} stage={}", st.display());
    finish()
}
