use core::ffi::c_void;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use compat_common::record;

use super::{EXT_PS_OPTIONS, EXT_STRIDE};

pub type PsOptions = [u32; 8];

pub const PER_SAMPLE: usize = 0;
pub const Z_ORDER: usize = 1;
pub const TEX_KILL: usize = 2;
pub const CONSERVATIVE_Z: usize = 6;

const DSS: usize = 0x1f0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PsEdit {
    pub per_sample: bool,
    pub early_z: bool,
    pub ds_write: bool,
    pub cons_z: u32,
}

impl PsEdit {
    pub fn from_options(o: &PsOptions, ds_write: bool) -> PsEdit {
        let early_z = matches!(o[Z_ORDER], 2 | 4);
        PsEdit {
            per_sample: o[PER_SAMPLE] == 1,
            early_z,
            ds_write: early_z && ds_write,
            cons_z: if matches!(o[CONSERVATIVE_Z], 2 | 3) { o[CONSERVATIVE_Z] } else { 0 },
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.per_sample && !self.early_z && self.cons_z == 0
    }
}

fn u32at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

pub fn options_of(payload: &[u8]) -> PsOptions {
    core::array::from_fn(|i| u32at(payload, i * 4))
}

pub fn merge(base: &PsOptions, patch: &PsOptions) -> PsOptions {
    core::array::from_fn(|i| if patch[i] != 0 { patch[i] } else { base[i] })
}

pub fn options_in_entries(entries: &[u8], count: usize) -> Option<PsOptions> {
    entries.chunks_exact(EXT_STRIDE).take(count).filter(|e| u32at(e, 0) == EXT_PS_OPTIONS).last().map(|e| options_of(&e[4..0x24]))
}

pub fn depth_stencil_writes(desc: &[u8]) -> bool {
    let depth = u32at(desc, DSS) != 0 && u32at(desc, DSS + 4) != 0;
    let stencil = u32at(desc, DSS + 0xc) != 0 && desc[DSS + 0x11] != 0;
    depth || stencil
}

fn sin_range(h: &str) -> Result<(usize, usize), String> {
    let a = h.find("struct SIn {").ok_or("no `struct SIn {` in the pixel shader")?;
    let b = a + h[a..].find("\n};").ok_or("unterminated SIn")?;
    Ok((a, b))
}

fn main_line(h: &str) -> Result<usize, String> {
    ["\nSOut main(SIn i) {", "\nvoid main(SIn i) {"].iter().filter_map(|p| h.find(p)).min().map(|i| i + 1).ok_or_else(|| "no `main(SIn i)` entry in the pixel shader".to_string())
}

pub fn exports_depth(h: &str) -> bool {
    h.contains(" : SV_Depth;") || h.contains(" : SV_DepthLessEqual;") || h.contains(" : SV_DepthGreaterEqual;")
}

pub fn may_kill(h: &str) -> bool {
    h.contains("discard")
}

fn is_plain_interpolant(line: &str) -> bool {
    let t = line.trim();
    let Some((decl, sem)) = t.strip_suffix(';').and_then(|s| s.split_once(" : ")) else { return false };
    let mut w = decl.split_whitespace();
    let (Some(ty), Some(_name), None) = (w.next(), w.next(), w.next()) else { return false };
    (ty.starts_with("float") || ty.starts_with("half") || ty.starts_with("min16float")) && !sem.starts_with("SV_")
}

pub fn plan(h: &str, want: PsEdit) -> (PsEdit, Vec<String>) {
    let mut e = want;
    let mut gaps = Vec::new();
    if e.early_z && exports_depth(h) {
        e.early_z = false;
        gaps.push("ZOrder early-Z with a depth-exporting shader: the console also tests late (Z export forces late Z); PC default kept".to_string());
    }
    if e.early_z && e.ds_write && may_kill(h) {
        e.early_z = false;
        gaps.push("ZOrder early-Z with a shader that can discard and a PSO that writes depth/stencil: console tests early and writes late; [earlydepthstencil] would write discarded pixels, so not applied".to_string());
    }
    if e.cons_z != 0 && !exports_depth(h) {
        e.cons_z = 0;
    }
    (e, gaps)
}

pub fn apply(h: &str, e: PsEdit) -> Result<String, String> {
    let mut out = h.to_string();
    if e.cons_z != 0 {
        let to = if e.cons_z == 2 { "SV_DepthLessEqual" } else { "SV_DepthGreaterEqual" };
        let mut found = false;
        for from in [" : SV_Depth;", " : SV_DepthLessEqual;", " : SV_DepthGreaterEqual;"] {
            if out.contains(from) {
                out = out.replace(from, &format!(" : {to};"));
                found = true;
            }
        }
        if !found {
            return Err("conservative depth requested but no depth output line".to_string());
        }
        let (a, b) = sin_range(&out)?;
        let body: Vec<String> = out[a..b]
            .lines()
            .map(|l| {
                if l.trim_end().ends_with(": SV_Position;") {
                    let t = l.trim().trim_start_matches("linear ").trim_start_matches("noperspective ").trim_start_matches("centroid ").trim_start_matches("sample ");
                    format!("    linear noperspective centroid {t}")
                } else {
                    l.to_string()
                }
            })
            .collect();
        out = format!("{}{}{}", &out[..a], body.join("\n"), &out[b..]);
    }
    if e.per_sample {
        let (a, b) = sin_range(&out)?;
        let mut edited = 0;
        let mut body: Vec<String> = out[a..b]
            .lines()
            .map(|l| {
                if is_plain_interpolant(l) {
                    edited += 1;
                    format!("    sample {}", l.trim())
                } else {
                    l.to_string()
                }
            })
            .collect();
        if edited == 0 && !out[a..b].contains("SV_SampleIndex") {
            body.push("    uint tb_sample_rate : SV_SampleIndex;".to_string());
        }
        out = format!("{}{}{}", &out[..a], body.join("\n"), &out[b..]);
    }
    if e.early_z {
        let m = main_line(&out)?;
        out.insert_str(m, "[earlydepthstencil]\n");
    }
    Ok(out)
}

fn fnv1a(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[derive(Clone)]
pub struct PsOrigin {
    pub blob: Arc<Vec<u8>>,
    pub hash: u64,
    pub rtfmt: [u8; 8],
    pub options: PsOptions,
}

static ORIGINS: Mutex<Option<HashMap<usize, PsOrigin>>> = Mutex::new(None);
type VariantCache = HashMap<(u64, [u8; 8], PsEdit), Result<Option<Arc<Vec<u8>>>, String>>;
static VARIANTS: Mutex<Option<VariantCache>> = Mutex::new(None);

pub fn note_pso(pso: *mut c_void, origin: Option<PsOrigin>) {
    if pso.is_null() {
        return;
    }
    let mut g = ORIGINS.lock().unwrap_or_else(|p| p.into_inner());
    let m = g.get_or_insert_with(Default::default);
    match origin {
        Some(o) => {
            m.insert(pso as usize, o);
        }
        None => {
            m.remove(&(pso as usize));
        }
    }
}

pub fn origin_of(pso: *mut c_void) -> Option<PsOrigin> {
    ORIGINS.lock().unwrap_or_else(|p| p.into_inner()).as_ref().and_then(|m| m.get(&(pso as usize)).cloned())
}

pub fn origin_from_blob(p: *const u8, n: usize, rtfmt: [u8; 8], options: PsOptions) -> Option<PsOrigin> {
    if p.is_null() || n == 0 {
        return None;
    }
    let blob = unsafe { core::slice::from_raw_parts(p, n) }.to_vec();
    Some(PsOrigin { hash: fnv1a(&blob), blob: Arc::new(blob), rtfmt, options })
}

fn record_once(msg: String) {
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut g = SEEN.lock().unwrap_or_else(|p| p.into_inner());
    if g.len() < 64 && !g.contains(&msg) {
        g.push(msg.clone());
        drop(g);
        record("d3d12_xs", "PixelShaderOptions", &msg);
    }
}

pub fn variant_with(o: &PsOrigin, ds_write: bool, translate: &dyn Fn(&[u8], u64, &[u8; 8], &dyn Fn(&str) -> Result<String, String>) -> Result<Vec<u8>, String>) -> Option<Arc<Vec<u8>>> {
    let want = PsEdit::from_options(&o.options, ds_write);
    if want.is_empty() {
        return None;
    }
    let key = (o.hash, o.rtfmt, want);
    if let Some(hit) = VARIANTS.lock().unwrap_or_else(|p| p.into_inner()).as_ref().and_then(|m| m.get(&key).cloned()) {
        return hit.ok().flatten();
    }
    let gaps = Mutex::new(Vec::new());
    let applied = Mutex::new(PsEdit::default());
    let edit = |h: &str| -> Result<String, String> {
        let (e, g) = plan(h, want);
        *gaps.lock().unwrap() = g;
        *applied.lock().unwrap() = e;
        if e.is_empty() {
            return Err(String::new());
        }
        apply(h, e)
    };
    let r = translate(&o.blob, o.hash, &o.rtfmt, &edit);
    for g in gaps.into_inner().unwrap() {
        record_once(format!("PS {:016x}: {g}", o.hash));
    }
    let e = applied.into_inner().unwrap();
    let res = match r {
        Ok(v) => {
            record_once(format!("PS {:016x}: variant compiled for {e:?} (options {:?})", o.hash, o.options));
            Ok(Some(Arc::new(v)))
        }
        Err(s) if s.is_empty() => Ok(None),
        Err(s) => {
            record_once(format!("PS {:016x}: variant for {e:?} failed ({s}); base shader kept", o.hash));
            Err(s)
        }
    };
    VARIANTS.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(Default::default).insert(key, res.clone());
    res.ok().flatten()
}

pub fn variant(o: &PsOrigin, ds_write: bool) -> Option<Arc<Vec<u8>>> {
    variant_with(o, ds_write, &|b, h, r, e| crate::shader_xlat::translate_edited(b, h, r, e))
}

#[cfg(test)]
#[path = "pso_ext_ps_tests.rs"]
mod tests;
