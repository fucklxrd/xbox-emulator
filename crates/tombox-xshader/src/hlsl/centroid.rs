use std::fmt::Write as _;

use crate::centroid_abi::{CONSOLE_DEFAULT, TB_CENTROID_FLAG_CUSTOM, TB_CENTROID_FLAG_POSITIONS, TB_CENTROID_MAX_POSITION_SAMPLES, TB_CENTROID_PC_DEFINE, TB_CENTROID_REGISTER, TB_CENTROID_SPACE};
use crate::{ShaderExtract, Stage};

const SV_SAMPLE_INDEX: u32 = 10;

pub(super) fn lowered(ex: &ShaderExtract) -> Vec<usize> {
    if ex.stage != Stage::Pixel || ex.input_sig.iter().any(|e| e.sysval == SV_SAMPLE_INDEX) {
        return Vec::new();
    }
    ex.input_sig.iter().enumerate().filter(|(_, e)| e.sysval == 0 && e.comp_type == 3 && e.name.contains("_centroid")).map(|(i, _)| i).collect()
}

pub(super) fn qualifier(ex: &ShaderExtract, ei: usize) -> &'static str {
    if lowered(ex).contains(&ei) { "TB_CEN_Q " } else { "centroid " }
}

pub(super) fn header(ex: &ShaderExtract) -> Option<String> {
    if lowered(ex).is_empty() {
        return None;
    }
    let def: Vec<String> = CONSOLE_DEFAULT.iter().map(|t| format!("uint2({:#x}u, {:#x}u)", t[0], t[1])).collect();
    let mut h = String::new();
    let _ = writeln!(h, "#ifdef {TB_CENTROID_PC_DEFINE}\n#define TB_CEN_Q centroid\n#else\n#define TB_CEN_Q");
    let _ = writeln!(h, "cbuffer TbCentroid : register(b{TB_CENTROID_REGISTER}, space{TB_CENTROID_SPACE}) {{ uint4 tb_cen_h; uint4 tb_cen_pos[2]; }};");
    let _ = writeln!(h, "static const uint2 tb_cen_def[5] = {{ {} }};", def.join(", "));
    let _ = writeln!(
        h,
        "uint tb_centroid_sample(uint cov) {{ uint n = GetRenderTargetSampleCount(); uint full = n >= 32u ? 0xffffffffu : ((1u << n) - 1u); cov &= full; if (n <= 1u || cov == full || cov == 0u) return 0xffffffffu; uint2 t = (tb_cen_h.z & {TB_CENTROID_FLAG_CUSTOM}u) != 0u ? tb_cen_h.xy : tb_cen_def[min(firstbithigh(n), 4u)]; for (uint k = 0u; k < 16u; k++) {{ uint s = ((k < 8u ? t.x : t.y) >> ((k & 7u) * 4u)) & 15u; if (s < n && ((cov >> s) & 1u) != 0u) return s; }} return 0xffffffffu; }}"
    );
    let _ = writeln!(
        h,
        "bool tb_centroid_snapped(uint s) {{ return (tb_cen_h.z & {TB_CENTROID_FLAG_POSITIONS}u) != 0u && s < {TB_CENTROID_MAX_POSITION_SAMPLES}u; }}\nint2 tb_centroid_offset(uint s, float4 pos) {{ uint b = ((((uint)pos.x) & 1u) | ((((uint)pos.y) & 1u) << 1)) * 8u + s; uint v = (tb_cen_pos[b >> 4][(b >> 2) & 3u] >> ((b & 3u) * 8u)) & 0xffu; return int2(asint(v << 28) >> 28, asint(v << 24) >> 28); }}"
    );
    let _ = writeln!(h, "#endif");
    Some(h)
}

pub(super) fn sys_fields(ex: &ShaderExtract, needs_pos: bool) -> String {
    if lowered(ex).is_empty() {
        return String::new();
    }
    let pos = if needs_pos { "    float4 tb_pos : SV_Position;\n" } else { "" };
    format!("{pos}    uint tb_cov : SV_Coverage;\n")
}

pub(super) fn rewrite(src: &str, eis: &[usize]) -> String {
    let mut out = src.to_string();
    for &ei in eis {
        out = out.replace(&format!("i.i{ei}."), &format!("tb_ci{ei}."));
    }
    out
}

pub(super) fn prologue(ex: &ShaderExtract, pos_field: &str) -> String {
    let eis = lowered(ex);
    if eis.is_empty() {
        return String::new();
    }
    let mut p = String::new();
    for &ei in &eis {
        let _ = writeln!(p, "    {} tb_ci{ei} = i.i{ei};", super::sig_type(&ex.input_sig[ei]).0);
    }
    let _ = writeln!(p, "#ifndef {TB_CENTROID_PC_DEFINE}");
    let _ = writeln!(p, "    uint tb_cs = tb_centroid_sample(i.tb_cov);");
    let _ = writeln!(p, "    if (tb_cs != 0xffffffffu && tb_centroid_snapped(tb_cs)) {{");
    let _ = writeln!(p, "        int2 tb_co = tb_centroid_offset(tb_cs, {pos_field});");
    for &ei in &eis {
        let _ = writeln!(p, "        tb_ci{ei} = EvaluateAttributeSnapped(i.i{ei}, tb_co);");
    }
    let _ = writeln!(p, "    }} else if (tb_cs != 0xffffffffu) {{");
    for &ei in &eis {
        let _ = writeln!(p, "        tb_ci{ei} = EvaluateAttributeAtSample(i.i{ei}, tb_cs);");
    }
    let _ = writeln!(p, "    }}\n#endif");
    p
}

pub(super) fn lower(ex: &ShaderExtract, pos_field: &str, init: &str, body: &str) -> (String, String) {
    let eis = lowered(ex);
    if eis.is_empty() {
        return (init.to_string(), body.to_string());
    }
    (prologue(ex, pos_field) + &rewrite(init, &eis), rewrite(body, &eis))
}
