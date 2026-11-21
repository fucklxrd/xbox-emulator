//! Runtime shader translation for PSO creation.
//!
//! Xbox shader blob (DXBC wrapper → XDAT → zlib → FSSM → RDNA2) → HLSL (tombox-xshader)
//! → DXC (`dxcompiler.dll`, SM6 DXIL; FXC/`D3DCompile` SM5 DXBC as fallback) → cached bytecode.
//! Each unique shader is translated once per process (keyed by an FNV-1a hash of its
//! bytes). Failures are cached too and reported with the reason; failing shaders are
//! dumped to `%TEMP%\tombox_shaders` so coverage can be extended offline with `xshader`.

use compat_common::record;
use core::ffi::c_void;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

type Cached = Result<Arc<Vec<u8>>, Arc<String>>;
type CacheCell = Arc<OnceLock<Cached>>;
static CACHE: OnceLock<Mutex<HashMap<u64, CacheCell>>> = OnceLock::new();
static MESH_ROOT_CACHE: OnceLock<Mutex<HashMap<u64, HashMap<Vec<u8>, CacheCell>>>> = OnceLock::new();
static OK: AtomicUsize = AtomicUsize::new(0);
static FAIL: AtomicUsize = AtomicUsize::new(0);
static DUMPED: AtomicUsize = AtomicUsize::new(0);
/// Translated code pointer -> (hash, original Xbox container); kept only for the diagnostic
/// draw trace (`rtcap`).
static ORIGIN: OnceLock<Mutex<HashMap<usize, (u64, Arc<Vec<u8>>)>>> = OnceLock::new();

static ORIGIN_HASH: OnceLock<Mutex<HashMap<usize, u64>>> = OnceLock::new();

pub fn note_origin_hash(code: usize, hash: u64) {
    ORIGIN_HASH.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner()).insert(code, hash);
}

pub fn origin_hash(code: usize) -> Option<u64> {
    if let Some(h) = ORIGIN_HASH.get().and_then(|m| m.lock().unwrap_or_else(|e| e.into_inner()).get(&code).copied()) {
        return Some(h);
    }
    origin(code).map(|(h, _)| h)
}

pub fn origin(code: usize) -> Option<(u64, Arc<Vec<u8>>)> {
    ORIGIN.get()?.lock().unwrap().get(&code).cloned()
}

fn fnv1a(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

type D3DCompileFn = unsafe extern "system" fn(
    *const c_void, usize, *const u8, *const c_void, *mut c_void, *const u8, *const u8, u32, u32,
    *mut *mut c_void, *mut *mut c_void,
) -> i32;

fn d3dcompile() -> Option<D3DCompileFn> {
    static F: OnceLock<usize> = OnceLock::new();
    let p = *F.get_or_init(|| unsafe {
        extern "system" {
            fn LoadLibraryA(name: *const u8) -> *mut c_void;
            fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
        }
        let h = LoadLibraryA(b"d3dcompiler_47.dll\0".as_ptr());
        if h.is_null() {
            return 0;
        }
        GetProcAddress(h, b"D3DCompile\0".as_ptr()) as usize
    });
    if p == 0 { None } else { Some(unsafe { core::mem::transmute::<usize, D3DCompileFn>(p) }) }
}

/// Read an ID3DBlob's bytes and release it.
unsafe fn take_blob(blob: *mut c_void) -> Vec<u8> {
    if blob.is_null() {
        return Vec::new();
    }
    let vt = *(blob as *const *const usize);
    let get_ptr: unsafe extern "system" fn(*mut c_void) -> *const u8 = core::mem::transmute(*vt.add(3));
    let get_size: unsafe extern "system" fn(*mut c_void) -> usize = core::mem::transmute(*vt.add(4));
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
    let (p, n) = (get_ptr(blob), get_size(blob));
    let v = if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, n).to_vec() };
    release(blob);
    v
}

fn compile(hlsl: &str, profile: &str) -> Result<Vec<u8>, String> {
    if profile.split('_').nth(1) == Some("6") {
        return Err(format!("FXC cannot compile profile {profile}: shader model 6.x needs DXC"));
    }
    let f = d3dcompile().ok_or("d3dcompiler_47.dll / D3DCompile unavailable")?;
    let target = format!("{profile}\0");
    let mut code: *mut c_void = core::ptr::null_mut();
    let mut errs: *mut c_void = core::ptr::null_mut();
    let hr = unsafe {
        f(
            hlsl.as_ptr() as *const c_void,
            hlsl.len(),
            b"tombox\0".as_ptr(),
            core::ptr::null(),
            core::ptr::null_mut(),
            b"main\0".as_ptr(),
            target.as_ptr(),
            0,
            0,
            &mut code,
            &mut errs,
        )
    };
    let err_text = String::from_utf8_lossy(&unsafe { take_blob(errs) }).into_owned();
    let bytes = unsafe { take_blob(code) };
    if hr < 0 || bytes.is_empty() {
        return Err(format!("D3DCompile hr={hr:#010x}: {}", err_text.trim()));
    }
    Ok(bytes)
}

fn dump_failure(hash: u64, blob: &[u8], hlsl: Option<&str>, err: &str) {
    if DUMPED.fetch_add(1, Ordering::Relaxed) >= 400 {
        return;
    }
    let dir = std::env::temp_dir().join("tombox_shaders");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!("{hash:016x}.dxbc")), blob);
    let _ = std::fs::write(dir.join(format!("{hash:016x}.err.txt")), err);
    if let Some(h) = hlsl {
        let _ = std::fs::write(dir.join(format!("{hash:016x}.hlsl")), h);
    }
}

/// On-disk cache of compiled shaders, keyed by the *generated HLSL* (plus profile): a
/// translator change that alters a shader's HLSL misses the cache, anything else — including
/// rebuilding this DLL — reuses the compiled bytecode. Compiling is the expensive part
/// (seconds for the large dispatcher-style compute shaders; translation takes milliseconds).
fn disk_cache_dir() -> Option<&'static std::path::PathBuf> {
    static DIR: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join("tombox_shader_cache").join("by_hlsl");
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    })
    .as_ref()
}

pub fn canonical_decls(hlsl: &str) -> String {
    let is_decl = |l: &str| {
        let s = l.trim();
        s.starts_with("bool tb_c") && s.ends_with(" = false;") && s["bool tb_c".len()..s.len() - " = false;".len()].bytes().all(|b| b.is_ascii_digit())
    };
    let lines: Vec<&str> = hlsl.split('\n').collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if is_decl(lines[i]) {
            let mut j = i;
            while j < lines.len() && is_decl(lines[j]) {
                j += 1;
            }
            let mut run: Vec<&str> = lines[i..j].to_vec();
            run.sort_by_key(|l| l.trim()["bool tb_c".len()..].split(' ').next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0));
            out.extend(run);
            i = j;
        } else {
            out.push(lines[i]);
            i += 1;
        }
    }
    out.join("\n")
}

fn loop_grad() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| {
        let on = std::env::var("TOMBOX_LOOP_GRAD").map(|v| v != "0").unwrap_or(true);
        record("d3d12_xs", "shader_xlat", if on { "in-loop pixel samples use quad-lane gradients (TOMBOX_LOOP_GRAD=0 reverts to LOD 0)" } else { "TOMBOX_LOOP_GRAD=0: in-loop pixel samples use LOD 0 (pre-06:00 translator behaviour)" });
        on
    })
}

pub fn disable_loop_grad(hlsl: &str) -> String {
    hlsl.split('\n')
        .map(|l| if l.starts_with(char::is_whitespace) && l.trim() == "#if defined(__SHADER_TARGET_MAJOR) && __SHADER_TARGET_MAJOR >= 6" { "    #if 0" } else { l })
        .collect::<Vec<_>>()
        .join("\n")
}

fn translate_uncached(blob: &[u8], hash: u64, rtfmt: &[u8; 8], root: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut ex = tombox_xshader::extract(blob).map_err(|e| {
        dump_failure(hash, blob, None, &e);
        format!("extract: {e}")
    })?;
    if let Some(root) = root.filter(|_| !ex.has_root_sig) {
        ex.root_sig = Some(tombox_xshader::parse_root_signature(root).map_err(|e| format!("pipeline root signature: {e}"))?);
    }
    let mut t = tombox_xshader::hlsl::translate(&ex).map_err(|e| {
        dump_failure(hash, blob, None, &e);
        format!("translate: {e}")
    })?;
    let needs = crate::mip_bind::Needs::of(&t);
    crate::mip_bind::note(hash, &needs);
    if let Some(why) = needs.refusal() {
        dump_failure(hash, blob, None, &why);
        return Err(format!("bind: {why}"));
    }
    if let Some(why) = crate::global_bind::refusal(t.needs_global) {
        dump_failure(hash, blob, None, &why);
        return Err(format!("bind: {why}"));
    }
    // Development aid: `%TEMP%	ombox_shader_override<hash>.hlsl` replaces the generated
    // HLSL for that shader (to instrument a translation while debugging). Logged every time.
    if let Ok(h) = std::fs::read_to_string(std::env::temp_dir().join("tombox_shader_override").join(format!("{hash:016x}.hlsl"))) {
        record("d3d12_xs", "shader_xlat", &format!("OVERRIDE: hash={hash:016x} uses hand-edited HLSL ({} bytes)", h.len()));
        t.hlsl = h;
    }
    t.hlsl = canonical_decls(&t.hlsl);
    if !loop_grad() {
        t.hlsl = disable_loop_grad(&t.hlsl);
    }
    if rtfmt.iter().any(|&k| k != 0) {
        let defs: String = rtfmt.iter().enumerate().filter(|(_, k)| **k != 0).map(|(i, k)| format!("#define TB_RTFMT{i} {k}u\n")).collect();
        t.hlsl = defs + &t.hlsl;
    }
    // Compiler: DXC (SM6 DXIL) when dxcompiler.dll + dxil.dll are available, otherwise FXC
    // (SM5 DXBC). FXC needs minutes for some of the generated compute shaders; DXC about a
    // second. `TOMBOX_SHADER_COMPILER=fxc` forces the old path.
    let sm6_only = t.profile.split('_').nth(1) == Some("6");
    let use_dxc = tombox_xshader::dxc::available()
        && (sm6_only || std::env::var("TOMBOX_SHADER_COMPILER").map(|v| v != "fxc").unwrap_or(true));
    if sm6_only && !use_dxc {
        return Err(format!("shader needs profile {} (shader model 6.x features); FXC compiles only shader model 5 and dxcompiler.dll/dxil.dll are not available", t.profile));
    }
    static ANNOUNCED: std::sync::Once = std::sync::Once::new();
    ANNOUNCED.call_once(|| {
        record("d3d12_xs", "shader_xlat", if use_dxc { "compiler: DXC (shader model 6.0, DXIL)" } else { "compiler: FXC (shader model 5, DXBC) - DXC not available or disabled" });
    });
    let profile = if use_dxc { tombox_xshader::dxc::profile_for(t.profile) } else { t.profile.to_string() };
    let _ = disk_cache_dir(); // creates the cache directory
    let cached = Some(tombox_xshader::dxc::cache_path(&t.hlsl, &profile));
    if let Some(p) = &cached {
        if let Ok(v) = std::fs::read(p) {
            if !v.is_empty() {
                return Ok(v);
            }
        }
    }
    // Leave the input on disk while it is being compiled: if FXC hangs or the title's
    // watchdog fires first, the file that remains identifies the shader.
    let dir = std::env::temp_dir().join("tombox_shaders");
    let _ = std::fs::create_dir_all(&dir);
    let inflight = dir.join(format!("inflight_{hash:016x}.dxbc"));
    let _ = std::fs::write(&inflight, blob);
    let t0 = std::time::Instant::now();
    let r = if use_dxc { tombox_xshader::dxc::compile(&t.hlsl, &profile) } else { compile(&t.hlsl, &profile) };
    let _ = std::fs::remove_file(&inflight);
    let ms = t0.elapsed().as_millis();
    if ms > 1500 {
        record("d3d12_xs", "shader_xlat", &format!("SLOW: hash={hash:016x} took {ms} ms to compile ({} bytes in, {} HLSL lines)", blob.len(), t.hlsl.lines().count()));
        if ms > 10_000 {
            let _ = std::fs::write(dir.join(format!("slow_{hash:016x}.dxbc")), blob);
        }
    }
    match r {
        Ok(v) => {
            if let Some(p) = &cached {
                let _ = std::fs::write(p, &v);
            }
            Ok(v)
        }
        Err(e) => {
            dump_failure(hash, blob, Some(&t.hlsl), &e);
            Err(e)
        }
    }
}

/// Translate an Xbox shader blob to PC bytecode (cached). `ptr/len` come from a
/// D3D12_SHADER_BYTECODE in the title's PSO desc.
pub unsafe fn translate(ptr: *const u8, len: usize) -> Cached {
    translate_for_targets(ptr, len, [0; 8])
}

pub fn retranslate(blob: &Arc<Vec<u8>>, hash: u64, rtfmt: [u8; 8]) -> Result<Arc<Vec<u8>>, String> {
    let v = std::panic::catch_unwind(|| translate_uncached(blob, hash, &rtfmt, None)).unwrap_or_else(|_| Err("translator panicked".to_string()))?;
    let v = Arc::new(v);
    ORIGIN.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap().insert(v.as_ptr() as usize, (hash, blob.clone()));
    Ok(v)
}

pub fn export_formats(num_rt: u32, formats: &[u32; 8]) -> [u8; 8] {
    static SW: crate::live_sw::Sw = crate::live_sw::Sw::new("TOMBOX_EXPORT_FMT", "export_fmt", true);
    let mut k = [0u8; 8];
    if !SW.on() {
        return k;
    }
    for i in 0..(num_rt as usize).min(8) {
        k[i] = match formats[i] {
            10 | 34 | 54 => 1,
            26 => 2,
            _ => 0,
        };
    }
    k
}

pub fn cache_key(hash: u64, rtfmt: &[u8; 8]) -> u64 {
    if rtfmt.iter().any(|&k| k != 0) { hash ^ fnv1a(rtfmt).rotate_left(17) } else { hash }
}

pub fn rtfmt_hex(rtfmt: &[u8; 8]) -> String {
    rtfmt.iter().map(|k| format!("{k:02x}")).collect()
}

pub fn parse_src_name(name: &str) -> Option<(u64, [u8; 8])> {
    let stem = name.strip_suffix(".xbox")?;
    let (h, f) = stem.split_once('_')?;
    if h.len() != 16 || f.len() != 16 {
        return None;
    }
    let hash = u64::from_str_radix(h, 16).ok()?;
    let mut rt = [0u8; 8];
    for i in 0..8 {
        rt[i] = u8::from_str_radix(&f[2 * i..2 * i + 2], 16).ok()?;
    }
    Some((hash, rt))
}

fn cache_root() -> std::path::PathBuf {
    std::env::temp_dir().join("tombox_shader_cache")
}

fn use_dxc() -> bool {
    tombox_xshader::dxc::available() && std::env::var("TOMBOX_SHADER_COMPILER").map(|v| v != "fxc").unwrap_or(true)
}

fn build_stamp() -> u64 {
    static S: OnceLock<u64> = OnceLock::new();
    *S.get_or_init(|| {
        extern "system" {
            fn GetModuleHandleExW(flags: u32, addr: *const c_void, out: *mut *mut c_void) -> i32;
            fn GetModuleFileNameW(h: *mut c_void, buf: *mut u16, n: u32) -> u32;
        }
        let mut h: *mut c_void = core::ptr::null_mut();
        let mut buf = [0u16; 1024];
        let n = unsafe {
            if GetModuleHandleExW(6, build_stamp as *const c_void, &mut h) == 0 {
                0
            } else {
                GetModuleFileNameW(h, buf.as_mut_ptr(), buf.len() as u32)
            }
        };
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        let md = std::fs::metadata(&path).ok();
        let len = md.as_ref().map(|m| m.len()).unwrap_or(0);
        let mt = md.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0);
        let mut k = Vec::new();
        k.extend_from_slice(path.as_bytes());
        k.extend_from_slice(&len.to_le_bytes());
        k.extend_from_slice(&mt.to_le_bytes());
        k.push(use_dxc() as u8);
        k.push(loop_grad() as u8);
        k.push(2);
        fnv1a(&k)
    })
}

fn blob_cache_path(key: u64) -> std::path::PathBuf {
    cache_root().join("by_blob").join(format!("{:016x}", build_stamp())).join(format!("{key:016x}.bin"))
}

fn src_dir() -> std::path::PathBuf {
    let exe = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_else(|| "title".into());
    cache_root().join("src").join(exe)
}

fn src_path(hash: u64, rtfmt: &[u8; 8]) -> std::path::PathBuf {
    src_dir().join(format!("{hash:016x}_{}.xbox", rtfmt_hex(rtfmt)))
}

fn has_override(hash: u64) -> bool {
    std::env::temp_dir().join("tombox_shader_override").join(format!("{hash:016x}.hlsl")).exists()
}

fn write_atomic(p: &std::path::Path, data: &[u8]) {
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, data).is_ok() && std::fs::rename(&tmp, p).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

static DISK_HITS: AtomicUsize = AtomicUsize::new(0);
static PREWARMED: AtomicUsize = AtomicUsize::new(0);

fn produce(blob: &[u8], hash: u64, rtfmt: &[u8; 8], key: u64) -> Result<Vec<u8>, String> {
    let ovr = has_override(hash);
    let bp = blob_cache_path(key);
    if !ovr {
        if let Ok(v) = std::fs::read(&bp) {
            if !v.is_empty() {
                DISK_HITS.fetch_add(1, Ordering::Relaxed);
                return Ok(v);
            }
        }
    }
    let r = std::panic::catch_unwind(|| translate_uncached(blob, hash, rtfmt, None)).unwrap_or_else(|_| Err("translator panicked".to_string()));
    if let Ok(v) = &r {
        if !ovr {
            write_atomic(&bp, v);
        }
        let sp = src_path(hash, rtfmt);
        if !sp.exists() {
            write_atomic(&sp, blob);
        }
    }
    r
}

fn cell(key: u64) -> CacheCell {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    cache.lock().unwrap().entry(key).or_insert_with(|| Arc::new(OnceLock::new())).clone()
}

fn mesh_root_cell(hash: u64, root: &[u8]) -> CacheCell {
    let cache = MESH_ROOT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap();
    let roots = cache.entry(hash).or_default();
    if let Some(entry) = roots.get(root) {
        return entry.clone();
    }
    let entry = Arc::new(OnceLock::new());
    roots.insert(root.to_vec(), entry.clone());
    entry
}

fn finish(res: Result<Vec<u8>, String>, hash: u64, len: usize, origin_blob: Option<&[u8]>) -> Cached {
    match res {
        Ok(v) => {
            let n = OK.fetch_add(1, Ordering::Relaxed) + 1;
            if n <= 8 || n % 50 == 0 {
                record("d3d12_xs", "shader_xlat", &format!("ok #{n} hash={hash:016x} {len}B -> {}B DXBC (disk hits {} prewarmed {})", v.len(), DISK_HITS.load(Ordering::Relaxed), PREWARMED.load(Ordering::Relaxed)));
            }
            let v = Arc::new(v);
            note_origin_hash(v.as_ptr() as usize, hash);
            if let Some(blob) = origin_blob.filter(|_| crate::rtcap::enabled() || crate::hot_reload::enabled()) {
                ORIGIN.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap().insert(v.as_ptr() as usize, (hash, Arc::new(blob.to_vec())));
            }
            Ok(v)
        }
        Err(e) => {
            let n = FAIL.fetch_add(1, Ordering::Relaxed) + 1;
            if n <= 40 || n % 50 == 0 {
                let first = e.lines().next().unwrap_or("");
                record("d3d12_xs", "shader_xlat", &format!("FAIL #{n} hash={hash:016x}: {first}"));
            }
            Err(Arc::new(e))
        }
    }
}

extern "system" {
    fn GetCurrentThread() -> *mut c_void;
    fn SetThreadPriority(h: *mut c_void, p: i32) -> i32;
}

pub fn start_prewarm() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if std::env::var("TOMBOX_SHADER_PREWARM").map(|v| v == "0").unwrap_or(false) {
            record("d3d12_xs", "shader_xlat", "prewarm disabled (TOMBOX_SHADER_PREWARM=0)");
            return;
        }
        let dir = src_dir();
        let mut todo: Vec<(std::path::PathBuf, u64, [u8; 8])> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some((h, rt)) = parse_src_name(&name) {
                    if !blob_cache_path(cache_key(h, &rt)).exists() {
                        todo.push((e.path(), h, rt));
                    }
                }
            }
        }
        record("d3d12_xs", "shader_xlat", &format!("prewarm: {} known shaders need translating for this build (stamp {:016x})", todo.len(), build_stamp()));
        if todo.is_empty() {
            return;
        }
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let workers = cores.saturating_sub(2).clamp(1, 16);
        let queue = Arc::new(Mutex::new(todo));
        for _ in 0..workers {
            let q = queue.clone();
            let _ = std::thread::Builder::new().name("tombox-shader-prewarm".into()).spawn(move || {
                unsafe { SetThreadPriority(GetCurrentThread(), -2) };
                let t0 = std::time::Instant::now();
                let mut done = 0usize;
                loop {
                    let next = q.lock().unwrap().pop();
                    let Some((p, h, rt)) = next else { break };
                    let Ok(blob) = std::fs::read(&p) else { continue };
                    if fnv1a(&blob) != h {
                        let _ = std::fs::remove_file(&p);
                        continue;
                    }
                    let key = cache_key(h, &rt);
                    let c = cell(key);
                    let mut ran = false;
                    c.get_or_init(|| {
                        ran = true;
                        finish(produce(&blob, h, &rt, key), h, blob.len(), Some(&blob))
                    });
                    if ran {
                        done += 1;
                        PREWARMED.fetch_add(1, Ordering::Relaxed);
                    }
                }
                record("d3d12_xs", "shader_xlat", &format!("prewarm worker finished: {done} shaders in {:.1}s", t0.elapsed().as_secs_f64()));
            });
        }
    });
}

type Job = Box<dyn FnOnce() + Send>;

fn helper_pool() -> &'static Mutex<std::sync::mpsc::Sender<Job>> {
    static POOL: OnceLock<Mutex<std::sync::mpsc::Sender<Job>>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        for _ in 0..cores.saturating_sub(2).clamp(1, 8) {
            let rx = rx.clone();
            let _ = std::thread::Builder::new().name("tombox-shader-stage".into()).spawn(move || loop {
                let job = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
                match job {
                    Ok(j) => j(),
                    Err(_) => break,
                }
            });
        }
        Mutex::new(tx)
    })
}

pub unsafe fn prefetch(ptr: *const u8, len: usize, rtfmt: [u8; 8]) -> bool {
    if ptr.is_null() || len < 32 || len > 0x40_0000 {
        return false;
    }
    let blob = core::slice::from_raw_parts(ptr, len);
    let hash = fnv1a(blob);
    let key = cache_key(hash, &rtfmt);
    let c = cell(key);
    if c.get().is_some() {
        return false;
    }
    let blob = Arc::new(blob.to_vec());
    let job: Job = Box::new(move || {
        c.get_or_init(|| crate::perf::timed(crate::perf::SHADER, || finish(produce(&blob, hash, &rtfmt, key), hash, blob.len(), Some(&blob))));
    });
    helper_pool().lock().unwrap_or_else(|e| e.into_inner()).send(job).is_ok()
}

pub unsafe fn translate_for_targets(ptr: *const u8, len: usize, rtfmt: [u8; 8]) -> Cached {
    if ptr.is_null() || len < 32 || len > 0x40_0000 {
        return Err(Arc::new(format!("bad shader blob ({ptr:p}, {len})")));
    }
    start_prewarm();
    let blob = core::slice::from_raw_parts(ptr, len);
    let hash = fnv1a(blob);
    let fmt_key = rtfmt.iter().any(|&k| k != 0);
    let key = cache_key(hash, &rtfmt);
    let c = cell(key);
    if let Some(hit) = c.get() {
        return hit.clone();
    }
    if fmt_key {
        static FMTN: AtomicUsize = AtomicUsize::new(0);
        if FMTN.fetch_add(1, Ordering::Relaxed) < 12 {
            record("d3d12_xs", "shader_xlat", &format!("export-format PS hash={hash:016x} rtfmt={rtfmt:?} (1=fp16 RTZ, 2=r11g11b10)"));
        }
    }
    let t0 = std::time::Instant::now();
    let entry = c.get_or_init(|| crate::perf::timed(crate::perf::SHADER, || finish(produce(blob, hash, &rtfmt, key), hash, len, Some(blob)))).clone();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    if ms > 50.0 {
        static SL: AtomicUsize = AtomicUsize::new(0);
        if crate::slowcall::should_log(SL.fetch_add(1, Ordering::Relaxed) as u64 + 1) {
            record("d3d12_xs", "shader_xlat", &format!("caller thread blocked {ms:.0} ms on shader {hash:016x} ({len}B)"));
        }
    }
    entry
}

pub fn needs_external_root_data(blob: &[u8]) -> bool {
    (32..=0x40_0000).contains(&blob.len())
        && tombox_xshader::Dxbc::parse(blob).is_ok_and(|dxbc| dxbc.chunk(b"RTS0").is_none())
}

pub fn translate_mesh_with_root(blob: &[u8], root: &[u8]) -> Cached {
    let hash = fnv1a(blob);
    let entry = mesh_root_cell(hash, root);
    // The existing by-blob cache, replay, and prewarming cannot restore a pipeline root
    // signature. Keep these inputs separate and let the generated-HLSL disk cache handle DXC.
    entry.get_or_init(|| {
        crate::perf::timed(crate::perf::SHADER, || {
            let result = std::panic::catch_unwind(|| translate_uncached(blob, hash, &[0; 8], Some(root)))
                .unwrap_or_else(|_| Err("translator panicked".to_string()));
            finish(result, hash, blob.len(), None)
        })
    }).clone()
}

pub fn counts() -> (usize, usize) {
    (OK.load(Ordering::Relaxed), FAIL.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_pipeline_root_cache_separates_layouts_and_keeps_embedded_roots() {
        let body = [0; 24];
        let embedded = crate::device_wrap::wrap_root_signature(&body);
        assert!(!needs_external_root_data(&embedded));
        let mut stripped = embedded[..32].to_vec();
        stripped[24..28].copy_from_slice(&32u32.to_le_bytes());
        stripped[28..32].fill(0);
        assert!(needs_external_root_data(&stripped));
        assert!(!needs_external_root_data(&stripped[..31]));

        let hash = fnv1a(&stripped);
        let root_a = mesh_root_cell(hash, &[1, 2, 3]);
        let root_b = mesh_root_cell(hash, &[1, 2, 4]);
        assert!(Arc::ptr_eq(&root_a, &mesh_root_cell(hash, &[1, 2, 3])));
        assert!(!Arc::ptr_eq(&root_a, &root_b));
        assert!(!Arc::ptr_eq(&root_a, &cell(cache_key(hash, &[0; 8]))));
    }

    #[test]
    fn fxc_refuses_sm6_profiles() {
        for p in ["ps_6_7", "ps_6_8", "cs_6_8"] {
            let e = compile("float4 main() : SV_Target { return 0; }", p).unwrap_err();
            assert!(e.contains("needs DXC"), "{p}: {e}");
        }
    }

    #[test]
    fn prefetch_rejects_bad_blobs_and_known_shaders() {
        assert!(!unsafe { prefetch(core::ptr::null(), 64, [0; 8]) });
        let blob = vec![7u8; 16];
        assert!(!unsafe { prefetch(blob.as_ptr(), blob.len(), [0; 8]) });
        let known = vec![0x5au8; 64];
        let key = cache_key(fnv1a(&known), &[0; 8]);
        let _ = cell(key).set(Ok(Arc::new(vec![1])));
        assert!(!unsafe { prefetch(known.as_ptr(), known.len(), [0; 8]) });
        let sent = std::sync::mpsc::channel::<u8>();
        let job: Job = Box::new(move || { let _ = sent.0.send(9); });
        assert!(helper_pool().lock().unwrap().send(job).is_ok());
        assert_eq!(sent.1.recv_timeout(std::time::Duration::from_secs(5)), Ok(9));
    }

    #[test]
    fn translated_code_keeps_console_hash_for_profiler() {
        let out = finish(Ok(vec![1, 2, 3, 4]), 0x1234_5678_9abc_def0, 4, Some(&[9, 9, 9, 9])).unwrap();
        assert_eq!(origin_hash(out.as_ptr() as usize), Some(0x1234_5678_9abc_def0));
        assert_eq!(origin_hash(1), None);
    }

    #[test]
    fn src_names_round_trip() {
        let rt = [1u8, 0, 2, 0, 0, 0, 0, 0];
        let name = format!("{:016x}_{}.xbox", 0xdead_beef_0123_4567u64, rtfmt_hex(&rt));
        assert_eq!(parse_src_name(&name), Some((0xdead_beef_0123_4567, rt)));
        assert_eq!(parse_src_name("x_y.xbox"), None);
        assert_eq!(parse_src_name(&name.replace(".xbox", ".bin")), None);
        assert_eq!(canonical_decls("a\n    bool tb_c395 = false;\n    bool tb_c72 = false;\nb"), "a\n    bool tb_c72 = false;\n    bool tb_c395 = false;\nb");
        assert_eq!(disable_loop_grad("#if defined(__SHADER_TARGET_MAJOR) && __SHADER_TARGET_MAJOR >= 6\n    #if defined(__SHADER_TARGET_MAJOR) && __SHADER_TARGET_MAJOR >= 6\nx"), "#if defined(__SHADER_TARGET_MAJOR) && __SHADER_TARGET_MAJOR >= 6\n    #if 0\nx");
        assert_eq!(cache_key(5, &[0; 8]), 5);
        assert_ne!(cache_key(5, &rt), 5);
    }
}

