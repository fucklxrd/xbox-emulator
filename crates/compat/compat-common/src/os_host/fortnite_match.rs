use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

#[link(name = "kernel32")]
extern "system" {
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn FlushInstructionCache(p: *mut c_void, base: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
}

pub const TITLE_EXE: &str = "fortniteclient-xsx-shipping.exe";

pub const START_GAME_INSTANCE_SITE: usize = 0xB46800;
pub const START_GAME_INSTANCE_ORIGINAL: [u8; 7] = [0x4C, 0x8D, 0x3D, 0xB9, 0x85, 0x6D, 0x05];
pub const START_GAME_INSTANCE_PATCHED: [u8; 7] = [0x49, 0x89, 0xC7, 0x90, 0x90, 0x90, 0x90];

pub const BROWSE_RVA: usize = 0xB1D51C;
pub const BROWSE_PROLOGUE: [u8; 13] = [0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57];
pub const BROWSE_VTABLE_SLOTS: [usize; 3] = [0x624D688 + 0x440, 0x624DDF0 + 0x440, 0x624E3D0 + 0x440];
pub const FURL_CTOR_RVA: usize = 0xBFA6B4;
pub const FURL_CTOR_PROLOGUE_RVA_CHECK: usize = 0xB468F0;
pub const FURL_DTOR_RVA: usize = 0xD005C0;
pub const FURL_DTOR_PROLOGUE: [u8; 13] = [0x40, 0x53, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0xD9, 0x48, 0x8B, 0x49, 0x58];
pub const FURL_SIZE: usize = 0x68;
pub const TRAVEL_PARTIAL: u32 = 1;

pub const DEFAULT_MAP: &str = "/Game/Athena/Apollo/Maps/Apollo_Terrain";
pub const DEFAULT_GAMEMODE: &str = "/Game/Athena/Athena_GameMode.Athena_GameMode_C";
pub const FALLBACK_GAMEMODE: &str = "/Script/FortniteGame.FortGameModeAthena";
pub const DEFAULT_PLAYLIST: &str = "Playlist_DefaultSquad";
pub const DEFAULT_SENTINEL: &str = "127.0.0.1:7777";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    Replay,
    Map,
    Play,
}

pub const DEFAULT_MODE: Mode = Mode::Play;

pub fn parse_mode(v: Option<&str>) -> Mode {
    match v.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("replay") => Mode::Replay,
        Some("map") => Mode::Map,
        Some("off") | Some("0") | Some("none") | Some("false") | Some("disabled") => Mode::Off,
        _ => DEFAULT_MODE,
    }
}

pub fn mode() -> Mode {
    parse_mode(std::env::var("TOMBOX_FN_MATCH").ok().as_deref())
}

pub fn parse_endpoint(v: &str) -> Option<(String, u16)> {
    let (h, p) = v.trim().rsplit_once(':')?;
    let port = p.trim().parse::<u16>().ok()?;
    let host = h.trim().trim_start_matches('[').trim_end_matches(']').to_string();
    if host.is_empty() {
        return None;
    }
    Some((host, port))
}

pub fn sentinel() -> (String, u16) {
    std::env::var("TOMBOX_FN_GAMESERVER")
        .ok()
        .and_then(|v| parse_endpoint(&v))
        .unwrap_or_else(|| parse_endpoint(DEFAULT_SENTINEL).unwrap())
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_string())
}

pub fn playlist() -> String {
    env_or("TOMBOX_FN_PLAYLIST", DEFAULT_PLAYLIST)
}

pub fn gamemode() -> String {
    env_or("TOMBOX_FN_GAMEMODE", DEFAULT_GAMEMODE)
}

pub fn map() -> String {
    env_or("TOMBOX_FN_MAP", DEFAULT_MAP)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SiteState {
    Original,
    AlreadyPatched,
    Mismatch,
}

pub fn classify_site(call_byte: u8, bytes: &[u8; 7]) -> SiteState {
    if call_byte != 0xE8 {
        return SiteState::Mismatch;
    }
    if *bytes == START_GAME_INSTANCE_ORIGINAL {
        SiteState::Original
    } else if *bytes == START_GAME_INSTANCE_PATCHED {
        SiteState::AlreadyPatched
    } else {
        SiteState::Mismatch
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

unsafe fn protected_write(dst: *mut u8, src: &[u8]) -> bool {
    let mut old = 0u32;
    if VirtualProtect(dst as *mut c_void, src.len(), 0x40, &mut old) == 0 {
        return false;
    }
    core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
    let mut tmp = 0u32;
    VirtualProtect(dst as *mut c_void, src.len(), old, &mut tmp);
    FlushInstructionCache(GetCurrentProcess(), dst as *const c_void, src.len());
    true
}

pub unsafe fn patch_cmdline_at(site: *mut u8) -> Result<SiteState, String> {
    let call_byte = *site.sub(5);
    let mut cur = [0u8; 7];
    core::ptr::copy_nonoverlapping(site, cur.as_mut_ptr(), 7);
    match classify_site(call_byte, &cur) {
        SiteState::Original => {
            if protected_write(site, &START_GAME_INSTANCE_PATCHED) {
                Ok(SiteState::Original)
            } else {
                Err(format!("VirtualProtect failed at {:#x}", site as usize))
            }
        }
        SiteState::AlreadyPatched => Ok(SiteState::AlreadyPatched),
        SiteState::Mismatch => Err(format!("{:02X} | {}", call_byte, hex(&cur))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlView {
    pub protocol: String,
    pub host: String,
    pub port: i32,
    pub map: String,
    pub ops: Vec<String>,
}

impl UrlView {
    pub fn describe(&self) -> String {
        let mut s = String::new();
        if !self.host.is_empty() {
            s.push_str(&format!("{}:{}", self.host, self.port));
        }
        s.push_str(&self.map);
        for o in &self.ops {
            s.push('?');
            s.push_str(o);
        }
        s
    }
}

pub fn is_sentinel(url: &UrlView, sentinel: &(String, u16)) -> bool {
    !url.host.is_empty() && url.host.eq_ignore_ascii_case(&sentinel.0) && url.port == sentinel.1 as i32
}

pub fn rewrite_text(url: &UrlView, sentinel: &(String, u16), map: &str, gamemode: &str, playlist: &str) -> Option<String> {
    if !is_sentinel(url, sentinel) {
        return None;
    }
    let mut t = format!("{map}?game={gamemode}");
    if !playlist.is_empty() {
        t.push_str(&format!("?Playlist={playlist}"));
    }
    Some(t)
}

unsafe fn read_fstring(p: *const u8) -> String {
    let data = core::ptr::read_unaligned(p as *const *const u16);
    let num = core::ptr::read_unaligned(p.add(8) as *const i32);
    if data.is_null() || num <= 0 || num > 0x10000 {
        return String::new();
    }
    let mut n = num as usize;
    let s = core::slice::from_raw_parts(data, n);
    while n > 0 && s[n - 1] == 0 {
        n -= 1;
    }
    String::from_utf16_lossy(&s[..n])
}

unsafe fn read_url(p: *const u8) -> UrlView {
    let ops_data = core::ptr::read_unaligned(p.add(0x48) as *const *const u8);
    let ops_num = core::ptr::read_unaligned(p.add(0x50) as *const i32);
    let mut ops = Vec::new();
    if !ops_data.is_null() && ops_num > 0 && ops_num < 256 {
        for i in 0..ops_num as usize {
            ops.push(read_fstring(ops_data.add(i * 16)));
        }
    }
    UrlView {
        protocol: read_fstring(p),
        host: read_fstring(p.add(0x10)),
        port: core::ptr::read_unaligned(p.add(0x20) as *const i32),
        map: read_fstring(p.add(0x28)),
        ops,
    }
}

type BrowseFn = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u8, *mut c_void) -> usize;
type UrlCtorFn = unsafe extern "system" fn(*mut u8, *const u8, *const u16, u32) -> *mut u8;
type UrlDtorFn = unsafe extern "system" fn(*mut u8);

static BASE: AtomicUsize = AtomicUsize::new(0);
static ORIG_BROWSE: AtomicUsize = AtomicUsize::new(0);
static BROWSE_LOGS: AtomicUsize = AtomicUsize::new(0);

#[repr(C, align(16))]
struct UrlBuf([u8; 0x100]);

unsafe extern "system" fn browse_hook(this: *mut c_void, ctx: *mut c_void, url: *mut u8, err: *mut c_void) -> usize {
    let orig: BrowseFn = core::mem::transmute(ORIG_BROWSE.load(Ordering::Acquire));
    let base = BASE.load(Ordering::Acquire);
    if url.is_null() || base == 0 {
        return orig(this, ctx, url, err);
    }
    let view = read_url(url);
    let sent = sentinel();
    match rewrite_text(&view, &sent, &map(), &gamemode(), &playlist()) {
        Some(text) => {
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let ctor: UrlCtorFn = core::mem::transmute(base + FURL_CTOR_RVA);
            let dtor: UrlDtorFn = core::mem::transmute(base + FURL_DTOR_RVA);
            let mut tmp = UrlBuf([0u8; 0x100]);
            ctor(tmp.0.as_mut_ptr(), url, wide.as_ptr(), TRAVEL_PARTIAL);
            let new_view = read_url(tmp.0.as_ptr());
            dtor(url);
            core::ptr::copy_nonoverlapping(tmp.0.as_ptr(), url, FURL_SIZE);
            crate::record_raw("fn-match", "browse", &format!("rewrite {} -> {}", view.describe(), new_view.describe()));
            orig(this, ctx, url, err)
        }
        None => {
            if BROWSE_LOGS.fetch_add(1, Ordering::Relaxed) < 64 {
                crate::record_raw("fn-match", "browse", &format!("pass {} (sentinel {}:{})", view.describe(), sent.0, sent.1));
            }
            orig(this, ctx, url, err)
        }
    }
}

unsafe fn bytes_at(p: *const u8, want: &[u8]) -> bool {
    core::slice::from_raw_parts(p, want.len()) == want
}

unsafe fn ctor_call_target_ok(base: usize) -> bool {
    let site = (base + FURL_CTOR_PROLOGUE_RVA_CHECK) as *const u8;
    if *site != 0xE8 {
        return false;
    }
    let rel = core::ptr::read_unaligned(site.add(1) as *const i32) as isize;
    (site as isize + 5 + rel) as usize == base + FURL_CTOR_RVA
}

unsafe fn install_browse(base: usize) -> Result<usize, String> {
    let f = (base + BROWSE_RVA) as *const u8;
    if !bytes_at(f, &BROWSE_PROLOGUE) {
        return Err(format!("Browse prologue mismatch {}", hex(core::slice::from_raw_parts(f, 13))));
    }
    if !bytes_at((base + FURL_DTOR_RVA) as *const u8, &FURL_DTOR_PROLOGUE) {
        return Err("FURL destructor bytes mismatch".into());
    }
    if !ctor_call_target_ok(base) {
        return Err("FURL constructor call site mismatch".into());
    }
    BASE.store(base, Ordering::Release);
    ORIG_BROWSE.store(base + BROWSE_RVA, Ordering::Release);
    let hook = browse_hook as *const () as usize;
    let mut n = 0;
    for rva in BROWSE_VTABLE_SLOTS {
        let slot = (base + rva) as *mut usize;
        let cur = core::ptr::read_volatile(slot);
        if cur == hook {
            n += 1;
            continue;
        }
        if cur != base + BROWSE_RVA {
            return Err(format!("vtable slot {rva:#x} holds {cur:#x}, expected {:#x}", base + BROWSE_RVA));
        }
        if !protected_write(slot as *mut u8, &hook.to_le_bytes()) {
            return Err(format!("VirtualProtect failed on vtable slot {rva:#x}"));
        }
        n += 1;
    }
    Ok(n)
}

unsafe fn title_base() -> Option<usize> {
    for m in super::patch::loaded_modules() {
        let path = super::patch::module_path(m);
        let name = path.rsplit('\\').next().unwrap_or(&path).to_ascii_lowercase();
        if name == TITLE_EXE {
            return Some(m as usize);
        }
    }
    None
}

pub fn apply() {
    let m = mode();
    if m == Mode::Off {
        return;
    }
    let base = match unsafe { title_base() } {
        Some(b) => b,
        None => return,
    };
    let site = (base + START_GAME_INSTANCE_SITE) as *mut u8;
    match unsafe { patch_cmdline_at(site) } {
        Ok(SiteState::AlreadyPatched) => crate::record_raw("fn-match", "patched", &format!("StartGameInstance cmdline at {:#x} (already in place)", site as usize)),
        Ok(_) => crate::record_raw("fn-match", "patched", &format!("StartGameInstance cmdline at {:#x}", site as usize)),
        Err(e) => crate::record_raw("fn-match", "StartGameInstance", &format!("bytes mismatch {e}")),
    }
    let (sh, sp) = sentinel();
    let raw = std::env::var("TOMBOX_FN_MATCH").unwrap_or_else(|_| "<unset>".into());
    crate::record_raw("fn-match", "mode", &format!("{m:?} (TOMBOX_FN_MATCH={raw}) map={} game={} playlist={} sentinel={sh}:{sp}", map(), gamemode(), playlist()));
    if m == Mode::Play {
        match unsafe { install_browse(base) } {
            Ok(n) => crate::record_raw("fn-match", "browse", &format!("hooked UEngine::Browse at {:#x} in {n} vtables", base + BROWSE_RVA)),
            Err(e) => crate::record_raw("fn-match", "browse", &format!("hook NOT installed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site_buf(call: u8, bytes: &[u8; 7]) -> Vec<u8> {
        let mut v = vec![0u8; 32];
        v[8] = call;
        v[13..20].copy_from_slice(bytes);
        v
    }

    #[test]
    fn patch_only_on_exact_bytes() {
        let mut good = site_buf(0xE8, &START_GAME_INSTANCE_ORIGINAL);
        let r = unsafe { patch_cmdline_at(good.as_mut_ptr().add(13)) };
        assert_eq!(r, Ok(SiteState::Original));
        assert_eq!(&good[13..20], &START_GAME_INSTANCE_PATCHED);
        let again = unsafe { patch_cmdline_at(good.as_mut_ptr().add(13)) };
        assert_eq!(again, Ok(SiteState::AlreadyPatched));
        assert_eq!(&good[13..20], &START_GAME_INSTANCE_PATCHED);

        let wrong = [0x4C, 0x8D, 0x3D, 0x00, 0x00, 0x00, 0x00];
        let mut bad = site_buf(0xE8, &wrong);
        let r = unsafe { patch_cmdline_at(bad.as_mut_ptr().add(13)) };
        assert!(r.is_err());
        assert_eq!(&bad[13..20], &wrong);

        let mut nocall = site_buf(0x90, &START_GAME_INSTANCE_ORIGINAL);
        let r = unsafe { patch_cmdline_at(nocall.as_mut_ptr().add(13)) };
        assert!(r.is_err());
        assert_eq!(&nocall[13..20], &START_GAME_INSTANCE_ORIGINAL);
    }

    fn url(host: &str, port: i32, map: &str) -> UrlView {
        UrlView { protocol: "unreal".into(), host: host.into(), port, map: map.into(), ops: vec!["Name=XboxEmulator".into()] }
    }

    #[test]
    fn furl_rewrite_only_for_sentinel() {
        let s = parse_endpoint("127.0.0.1:7777").unwrap();
        let hit = rewrite_text(&url("127.0.0.1", 7777, "/Game/Maps/Frontend"), &s, DEFAULT_MAP, DEFAULT_GAMEMODE, "Playlist_DefaultSquad");
        assert_eq!(
            hit.as_deref(),
            Some("/Game/Athena/Apollo/Maps/Apollo_Terrain?game=/Game/Athena/Athena_GameMode.Athena_GameMode_C?Playlist=Playlist_DefaultSquad")
        );
        assert!(rewrite_text(&url("127.0.0.1", 7778, "/Game/Maps/Frontend"), &s, DEFAULT_MAP, DEFAULT_GAMEMODE, "p").is_none());
        assert!(rewrite_text(&url("10.0.0.5", 7777, "/Game/Maps/Frontend"), &s, DEFAULT_MAP, DEFAULT_GAMEMODE, "p").is_none());
        assert!(rewrite_text(&url("", 7777, "/Game/Maps/Frontend"), &s, DEFAULT_MAP, DEFAULT_GAMEMODE, "p").is_none());
        let no_pl = rewrite_text(&url("127.0.0.1", 7777, ""), &s, DEFAULT_MAP, FALLBACK_GAMEMODE, "");
        assert_eq!(no_pl.as_deref(), Some("/Game/Athena/Apollo/Maps/Apollo_Terrain?game=/Script/FortniteGame.FortGameModeAthena"));
    }

    #[test]
    fn endpoint_and_mode_parsing() {
        assert_eq!(parse_endpoint("127.0.0.1:7777"), Some(("127.0.0.1".into(), 7777)));
        assert_eq!(parse_endpoint("[::1]:15000"), Some(("::1".into(), 15000)));
        assert_eq!(parse_endpoint("nohost"), None);
        assert_eq!(parse_endpoint(":7777"), None);
        assert_eq!(parse_mode(Some("Play")), Mode::Play);
        assert_eq!(parse_mode(Some("replay")), Mode::Replay);
        assert_eq!(parse_mode(Some("MAP")), Mode::Map);
        assert_eq!(parse_mode(Some("off")), Mode::Off);
        assert_eq!(parse_mode(Some(" OFF ")), Mode::Off);
        assert_eq!(parse_mode(Some("0")), Mode::Off);
        assert_eq!(parse_mode(None), Mode::Play);
        assert_eq!(parse_mode(Some("")), Mode::Play);
        assert_eq!(DEFAULT_MODE, Mode::Play);
    }

    struct Pe {
        f: std::fs::File,
        base: u64,
        secs: Vec<(u32, u32, u32)>,
    }

    impl Pe {
        fn open(p: &str) -> Option<Pe> {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(p).ok()?;
            let mut h = vec![0u8; 0x1000];
            f.read_exact(&mut h).ok()?;
            let pe = u32::from_le_bytes(h[0x3c..0x40].try_into().ok()?) as usize;
            let nsec = u16::from_le_bytes(h[pe + 6..pe + 8].try_into().ok()?) as usize;
            let opt = u16::from_le_bytes(h[pe + 20..pe + 22].try_into().ok()?) as usize;
            let base = u64::from_le_bytes(h[pe + 24 + 24..pe + 24 + 32].try_into().ok()?);
            let st = pe + 24 + opt;
            let mut secs = Vec::new();
            for i in 0..nsec {
                let s = &h[st + i * 40..st + i * 40 + 40];
                let vsz = u32::from_le_bytes(s[8..12].try_into().ok()?);
                let va = u32::from_le_bytes(s[12..16].try_into().ok()?);
                let raw = u32::from_le_bytes(s[20..24].try_into().ok()?);
                secs.push((va, vsz, raw));
            }
            f.seek(SeekFrom::Start(0)).ok()?;
            Some(Pe { f, base, secs })
        }
        fn read(&mut self, rva: usize, n: usize) -> Vec<u8> {
            use std::io::{Read, Seek, SeekFrom};
            let (va, _, raw) = *self.secs.iter().find(|(va, vsz, _)| rva >= *va as usize && rva < (*va + *vsz) as usize).unwrap();
            let mut b = vec![0u8; n];
            self.f.seek(SeekFrom::Start((rva - va as usize + raw as usize) as u64)).unwrap();
            self.f.read_exact(&mut b).unwrap();
            b
        }
    }

    #[test]
    fn dump_constants_match() {
        let path = std::env::var("TOMBOX_FN_DUMP_EXE").unwrap_or_else(|_| r"D:\xbox-pc-port\games\Fortnite\extracted\Mount\FortniteGame\Binaries\XSX\FortniteClient-XSX-Shipping.exe".into());
        let Some(mut pe) = Pe::open(&path) else {
            eprintln!("dump exe not present at {path}; constant check not run");
            return;
        };
        let call = pe.read(START_GAME_INSTANCE_SITE - 5, 12);
        assert_eq!(classify_site(call[0], &call[5..12].try_into().unwrap()), SiteState::Original);
        let rel = i32::from_le_bytes(call[1..5].try_into().unwrap()) as isize;
        let getter = (START_GAME_INSTANCE_SITE as isize + rel) as usize;
        let g = pe.read(getter, 0x1d);
        assert_eq!(&g[0x11..0x14], &[0x48, 0x8D, 0x05]);
        assert_eq!(g[0x1c], 0xC3);
        assert_eq!(pe.read(BROWSE_RVA, 13), BROWSE_PROLOGUE.to_vec());
        assert_eq!(pe.read(FURL_DTOR_RVA, 13), FURL_DTOR_PROLOGUE.to_vec());
        let c = pe.read(FURL_CTOR_PROLOGUE_RVA_CHECK, 5);
        assert_eq!(c[0], 0xE8);
        let rel = i32::from_le_bytes(c[1..5].try_into().unwrap()) as isize;
        assert_eq!((FURL_CTOR_PROLOGUE_RVA_CHECK as isize + 5 + rel) as usize, FURL_CTOR_RVA);
        let vcall = pe.read(0xB46923, 7);
        assert_eq!(vcall, vec![0x41, 0xFF, 0x92, 0x40, 0x04, 0x00, 0x00]);
        for slot in BROWSE_VTABLE_SLOTS {
            let v = u64::from_le_bytes(pe.read(slot, 8).try_into().unwrap());
            assert_eq!(v, pe.base + BROWSE_RVA as u64, "slot {slot:#x}");
        }
    }

    #[test]
    fn url_describe_matches_ue_shape() {
        let u = url("127.0.0.1", 7777, "/Game/Maps/Frontend");
        assert_eq!(u.describe(), "127.0.0.1:7777/Game/Maps/Frontend?Name=XboxEmulator");
    }
}
