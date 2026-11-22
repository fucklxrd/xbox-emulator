//! Regression tests against real dumped Fortnite XSX shaders (committed in analysis/).
//!
//! These assert the two properties that make the extractor+decoder trustworthy:
//!   1. the DXBC→XDAT→zlib→FSSM→ISA chain recovers an s_endpgm-terminated RDNA2 program;
//!   2. the length-accurate decoder consumes the whole ISA and lands exactly on s_endpgm.
//! A wrong instruction length (e.g. mishandling a 2-dword VOP3 or an inline literal) would
//! desynchronize the stream and fail property 2, so this is a strong correctness check.

use std::path::PathBuf;
use tombox_xshader::{extract, rdna2, Stage};

fn fixture(name: &str) -> Vec<u8> {
    // tests run with CWD = crate dir; fixtures live at <repo>/analysis/shaders.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../analysis/shaders");
    p.push(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("missing fixture {}: {e}", p.display()))
}

fn check(name: &str, want_stage: Stage) {
    let blob = fixture(name);
    let ex = extract(&blob).expect("extract");
    assert_eq!(ex.stage, want_stage, "{name}: stage");
    assert_eq!(ex.shader_model, (6, 0), "{name}: SM6.0");
    assert!(ex.bitcode_stripped, "{name}: DXIL should be NOBC-stripped");
    assert!(!ex.isa.is_empty(), "{name}: non-empty ISA");
    // ISA must end on s_endpgm (0xBF810000).
    let tail = &ex.isa[ex.isa.len() - 4..];
    assert_eq!(
        u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]),
        0xBF81_0000,
        "{name}: ISA must end on s_endpgm"
    );
    // Decoder must consume the whole ISA and terminate on s_endpgm.
    let insns = rdna2::disassemble(&ex.isa);
    let consumed: usize = insns.iter().map(|i| i.dwords * 4).sum();
    assert_eq!(consumed, ex.isa.len(), "{name}: decoder must consume all ISA bytes");
    let last = insns.last().expect("at least one insn");
    assert_eq!(last.mnemonic, "s_endpgm", "{name}: must terminate on s_endpgm");
}

#[test]
#[cfg_attr(not(tombox_fixtures_shaders), ignore = "needs analysis/shaders fixtures extracted from local title dumps")]
fn vertex_shader_extracts_and_decodes() {
    check("fortnite_vs_sample.dxbc", Stage::Vertex);
}

#[test]
#[cfg_attr(not(tombox_fixtures_shaders), ignore = "needs analysis/shaders fixtures extracted from local title dumps")]
fn pixel_shader_extracts_and_decodes() {
    check("fortnite_ps_sample.dxbc", Stage::Pixel);
}

#[test]
#[cfg_attr(not(tombox_fixtures_shaders), ignore = "needs analysis/shaders fixtures extracted from local title dumps")]
fn compute_shader_extracts_and_decodes() {
    check("fortnite_cs_clear.dxbc", Stage::Compute);
    let ex = extract(&fixture("fortnite_cs_clear.dxbc")).unwrap();
    // Thread-group size lives in the ISA preamble.
    assert_eq!(&ex.isa_preamble[1..], &[4, 4, 4]);
}

/// The HLSL emitter must accept all three real shaders and bind what they actually use.
#[test]
#[cfg_attr(not(tombox_fixtures_shaders), ignore = "needs analysis/shaders fixtures extracted from local title dumps")]
fn fixtures_translate_to_hlsl() {
    use tombox_xshader::hlsl::translate;
    let vs = translate(&extract(&fixture("fortnite_vs_sample.dxbc")).unwrap()).expect("VS");
    assert_eq!(vs.profile, "vs_5_0");
    assert!(vs.hlsl.contains("cbuffer CB0 : register(b0)"));
    assert!(vs.hlsl.contains("SV_Position"));
    let ps = translate(&extract(&fixture("fortnite_ps_sample.dxbc")).unwrap()).expect("PS");
    assert_eq!(ps.profile, "ps_5_0");
    assert!(ps.hlsl.contains("tex0.Sample(samp0"));
    assert!(ps.hlsl.contains("SV_Target0"));
    let cs = translate(&extract(&fixture("fortnite_cs_clear.dxbc")).unwrap()).expect("CS");
    assert_eq!(cs.profile, "cs_5_0");
    assert!(cs.hlsl.contains("[numthreads(4, 4, 4)]"));
    assert!(cs.hlsl.contains("RWTexture3D<float4> uav0 : register(u0)"));
}

#[test]
#[cfg_attr(not(tombox_fixtures_shaders), ignore = "needs analysis/shaders fixtures extracted from local title dumps")]
fn xdefiant_vs_swappc_s4_binds_root_tables() {
    let ex = extract(&fixture("xdefiant_vs_swappc_s4.dxbc")).expect("extract");
    let t = tombox_xshader::hlsl::translate(&ex).expect("translate");
    for want in ["register(b1720)", "register(b1721)", "register(b1722)", "register(b1760)"] {
        assert!(t.hlsl.contains(want), "missing {want} in\n{}", t.hlsl);
    }
    assert!(!t.hlsl.contains("cbuffer CB0 : register(b0)"), "table CBVs fell back to b0:\n{}", t.hlsl);
}
#[test]
#[cfg_attr(not(tombox_fixtures_shaders), ignore = "needs analysis/shaders fixtures extracted from local title dumps")]
fn xdefiant_ps_position_precedes_front_face() {
    let ex = extract(&fixture("xdefiant_ps_frontface_quad.dxbc")).expect("extract");
    let t = tombox_xshader::hlsl::translate(&ex).expect("translate");
    let pos = t.hlsl.find(": SV_Position").expect("SV_Position input");
    let ff = t.hlsl.find(": SV_IsFrontFace").expect("SV_IsFrontFace input");
    assert!(pos < ff, "SV_Position must come before SV_IsFrontFace so its register matches the VS output:\n{}", t.hlsl);
}
