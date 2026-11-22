#![allow(non_snake_case)]

use crate::console::{self, CONSOLE};
use crate::getters::*;
use crate::result::Handle;
use core::ffi::CStr;
use std::path::PathBuf;
use std::sync::OnceLock;

// Only tests that exercise the staged console compiler need the DXIL fixtures.
fn fixture(name: &str) -> Vec<u8> {
    let dir = std::env::var_os("TOMBOX_XBSC_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"));
    let path = dir.join(name);
    std::fs::read(&path).unwrap_or_else(|err| {
        panic!("read {}: {err}. Set TOMBOX_XBSC_FIXTURES to the directory with the local XBSC test shaders", path.display())
    })
}

fn empty_cs() -> &'static [u8] {
    static DXIL: OnceLock<Vec<u8>> = OnceLock::new();
    DXIL.get_or_init(|| fixture("empty.dxil"))
}

fn add_cs() -> &'static [u8] {
    static DXIL: OnceLock<Vec<u8>> = OnceLock::new();
    DXIL.get_or_init(|| fixture("add_od.dxil"))
}

fn up() -> bool {
    CONSOLE.module(console::own()) != 0
}

macro_rules! c {
    ($f:ident $(, $a:expr)*) => {{
        let v: &[usize] = &[$($a as usize),*];
        let mut a = [0usize; 12];
        a[..v.len()].copy_from_slice(v);
        console::$f(a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[9], a[10], a[11])
    }};
}

unsafe fn code(h: usize) -> u32 {
    crate::XBSC_GetResultCode(h as *const Handle)
}

unsafe fn message(h: usize) -> String {
    CStr::from_ptr(crate::XBSC_GetErrorMessage(h as *const Handle)).to_string_lossy().into_owned()
}

unsafe fn destroy(h: usize) {
    crate::XBSC_DestroyResult(h as *mut Handle)
}

struct Pipeline {
    input: Box<[u8; 0xc8]>,
    _cs: Box<[u8; 0x1000]>,
    _rs: Box<[u8; 0x1000]>,
}

fn compute(dxil: &'static [u8]) -> Pipeline {
    let mut input = Box::new([0u8; 0xc8]);
    let mut cs = Box::new([0u8; 0x1000]);
    let mut rs = Box::new([0u8; 0x1000]);
    input[0] = 6;
    input[0x62] = 3;
    rs[0] = 2;
    cs[..8].copy_from_slice(&(dxil.as_ptr() as u64).to_le_bytes());
    cs[8..16].copy_from_slice(&(rs.as_ptr() as u64).to_le_bytes());
    input[0x40..0x48].copy_from_slice(&(cs.as_ptr() as u64).to_le_bytes());
    Pipeline { input, _cs: cs, _rs: rs }
}

unsafe fn compiled() -> (Pipeline, usize) {
    let p = compute(empty_cs());
    let h = c!(XBSC_PipelineCompile, p.input.as_ptr(), 0);
    (p, h)
}

#[test]
fn XBSC_PipelineCompile_compiles_a_compute_shader_through_newbe() {
    if !up() {
        return;
    }
    unsafe {
        let (_p, h) = compiled();
        assert_ne!(h, 0);
        assert_eq!(code(h), 0);
        assert_eq!((*(h as *const Handle)).tag, TAG_PIPELINE_OUTPUT);
        assert!(!XBSC_GetPipelineCompileOutput(h as *const u8).is_null());
        destroy(h);
        let p = compute(add_cs());
        let h = c!(XBSC_PipelineCompile, p.input.as_ptr(), 0);
        assert_eq!(code(h), 1);
        assert!(message(h).starts_with("Failed to find resource in root sig: \"<Unnamed>\" UAV:RawBuffer(u0, space=0)"), "{}", message(h));
        destroy(h);
        let mut v5 = compute(empty_cs());
        v5.input[0] = 5;
        let h = c!(XBSC_PipelineCompile, v5.input.as_ptr(), 0);
        assert_eq!(code(h), 1);
        assert_eq!(message(h), "unexpected api version: 5 expected version: 6");
        destroy(h);
    }
}

#[test]
fn XBSC_GetPipelineCompileOutput_reads_tag_6_only() {
    let mut h = [0u8; 0x40];
    h[0x18] = 6;
    h[0x28..0x30].copy_from_slice(&0x1234u64.to_le_bytes());
    unsafe {
        assert_eq!(XBSC_GetPipelineCompileOutput(h.as_ptr()) as usize, 0x1234);
        h[0x18] = 7;
        assert!(XBSC_GetPipelineCompileOutput(h.as_ptr()).is_null());
        assert!(XBSC_GetPipelineCompileOutput(core::ptr::null()).is_null());
    }
}

unsafe fn serialized_src(input: *const u8) -> (usize, Vec<u8>) {
    let h = c!(XBSC_SerializeSrcPipeline, input, 0);
    let mut d: *const u8 = core::ptr::null();
    let mut n = 0usize;
    assert_eq!(XBSC_GetSerializedSrcPipeline(h as *const u8, &mut d, &mut n), 0);
    (h, core::slice::from_raw_parts(d, n).to_vec())
}

#[test]
fn XBSC_SerializeSrcPipeline_writes_the_fssm_image() {
    if !up() {
        return;
    }
    let mut input = [0u8; 0xc8];
    input[0] = 6;
    unsafe {
        let (h, b) = serialized_src(input.as_ptr());
        assert_eq!(code(h), 0);
        assert_eq!(b.len(), 0xd8);
        assert_eq!(&b[..12], &[0x46, 0x53, 0x53, 0x4d, 1, 2, 0, 0, 0xc8, 0, 0, 0]);
        assert_eq!(b[0x10], 6);
        for off in [0x08usize, 0x10, 0x18, 0x20, 0x28, 0x40, 0x48, 0x50, 0x58, 0xb8, 0xc0] {
            assert_eq!(&b[0x10 + off..0x18 + off], &0xdead_beef_dead_beefu64.to_le_bytes(), "{off:#x}");
        }
        assert!(b[0x10 + 0x60..0x10 + 0xb8].iter().all(|&x| x == 0));
        destroy(h);
        let h = c!(XBSC_SerializeSrcPipeline, 0, 0);
        assert_eq!(code(h), 1);
        destroy(h);
    }
}

#[test]
fn XBSC_GetSerializedSrcPipeline_checks_tag_and_outputs() {
    let blob = [0x46u8, 0x53, 0x53, 0x4d, 1, 2, 0, 0, 0x20, 0, 0, 0, 0, 0, 0, 0];
    let mut h = [0u8; 0x40];
    h[0x18] = 0xc;
    h[0x28..0x30].copy_from_slice(&(blob.as_ptr() as u64).to_le_bytes());
    let mut d: *const u8 = core::ptr::null();
    let mut n = 0usize;
    unsafe {
        assert_eq!(XBSC_GetSerializedSrcPipeline(h.as_ptr(), &mut d, &mut n), 0);
        assert_eq!((d, n), (blob.as_ptr(), 0x30));
        assert_eq!(XBSC_GetSerializedSrcPipeline(h.as_ptr(), core::ptr::null_mut(), &mut n), 1);
        h[0x18] = 8;
        assert_eq!(XBSC_GetSerializedSrcPipeline(h.as_ptr(), &mut d, &mut n), 1);
        assert_eq!(XBSC_GetSerializedSrcPipeline(core::ptr::null(), &mut d, &mut n), 1);
    }
}

#[test]
fn XBSC_DeserializeSrcPipeline_round_trips() {
    if !up() {
        return;
    }
    let mut input = [0u8; 0xc8];
    input[0] = 6;
    input[0x62] = 3;
    unsafe {
        let (s, b) = serialized_src(input.as_ptr());
        let h = c!(XBSC_DeserializeSrcPipeline, b.as_ptr(), b.len(), 0);
        assert_eq!(code(h), 0);
        let mut p: *const u8 = core::ptr::null();
        assert_eq!(XBSC_GetDeserializedSrcPipeline(h as *const u8, &mut p), 0);
        assert_eq!(*p, 6);
        assert_eq!(*p.add(0x62), 3);
        destroy(h);
        destroy(s);
        let h = c!(XBSC_DeserializeSrcPipeline, 0, 0, 0);
        assert_eq!(code(h), 1);
        destroy(h);
    }
}

#[test]
fn XBSC_GetDeserializedSrcPipeline_checks_tag_and_object() {
    let mut h = [0u8; 0x40];
    h[0x18] = 0xd;
    let mut p: *const u8 = core::ptr::null();
    unsafe {
        assert_eq!(XBSC_GetDeserializedSrcPipeline(h.as_ptr(), &mut p), 1);
        h[0x28] = 0x40;
        assert_eq!(XBSC_GetDeserializedSrcPipeline(h.as_ptr(), &mut p), 0);
        assert_eq!(p as usize, 0x40);
        h[0x18] = 9;
        assert_eq!(XBSC_GetDeserializedSrcPipeline(h.as_ptr(), &mut p), 1);
        assert_eq!(XBSC_GetDeserializedSrcPipeline(h.as_ptr(), core::ptr::null_mut()), 1);
    }
}

#[test]
fn XBSC_SerializePipelineCompileInput_copies_into_a_caller_buffer() {
    if !up() {
        return;
    }
    let mut input = [0u8; 0xc8];
    input[0] = 6;
    let (_s, want) = unsafe { serialized_src(input.as_ptr()) };
    let a = crate::alloc::XbscAllocator { alloc: Some(heap_alloc), free: Some(heap_free), ctx: core::ptr::null_mut() };
    let mut out: *mut u8 = core::ptr::null_mut();
    let mut n = 0u32;
    unsafe {
        c!(XBSC_SerializePipelineCompileInput, input.as_ptr(), &a as *const _, &mut out as *mut _, &mut n as *mut u32);
        assert_eq!(n as usize, want.len());
        assert_eq!(core::slice::from_raw_parts(out, n as usize), &want[..]);
        heap_free(core::ptr::null_mut(), out as *mut _);
    }
}

unsafe extern "system" fn heap_alloc(_: *mut core::ffi::c_void, n: usize) -> *mut core::ffi::c_void {
    std::alloc::alloc(std::alloc::Layout::from_size_align(n + 16, 16).unwrap()).add(16) as *mut _ as *mut core::ffi::c_void
}

unsafe extern "system" fn heap_free(_: *mut core::ffi::c_void, _p: *mut core::ffi::c_void) {}

#[test]
fn XBSC_DeserializePipelineCompileInput_is_unsupported_on_console() {
    assert_eq!(unsafe { crate::XBSC_DeserializePipelineCompileInput(core::ptr::null(), 0, core::ptr::null()) }, -1);
}

unsafe fn serialized_output() -> (Pipeline, usize, usize, Vec<u8>) {
    let (p, h) = compiled();
    let po = XBSC_GetPipelineCompileOutput(h as *const u8);
    let s = c!(XBSC_SerializePipelineCompileOutput, po, 0);
    let mut d: *const u8 = core::ptr::null();
    let mut n = 0usize;
    assert_eq!(XBSC_GetSerializedPipelineCompileOutput(s as *const u8, &mut d, &mut n), 0);
    (p, h, s, core::slice::from_raw_parts(d, n).to_vec())
}

#[test]
fn XBSC_SerializePipelineCompileOutput_writes_the_compiled_pipeline() {
    if !up() {
        return;
    }
    unsafe {
        let (_p, h, s, b) = serialized_output();
        assert_eq!(code(s), 0);
        assert_eq!(&b[..5], b"FSSM\x01");
        assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize + 0x10, b.len());
        assert_eq!(b[0x10], 6);
        destroy(s);
        destroy(h);
        let s = c!(XBSC_SerializePipelineCompileOutput, 0, 0);
        assert_eq!(code(s), 1);
        destroy(s);
    }
}

#[test]
fn XBSC_GetSerializedPipelineCompileOutput_checks_tag_8() {
    let blob = [0u8; 16];
    let mut h = [0u8; 0x40];
    h[0x18] = 8;
    h[0x28..0x30].copy_from_slice(&(blob.as_ptr() as u64).to_le_bytes());
    let mut d: *const u8 = core::ptr::null();
    let mut n = 0usize;
    unsafe {
        assert_eq!(XBSC_GetSerializedPipelineCompileOutput(h.as_ptr(), &mut d, &mut n), 0);
        assert_eq!(n, 0x10);
        h[0x18] = 0xc;
        assert_eq!(XBSC_GetSerializedPipelineCompileOutput(h.as_ptr(), &mut d, &mut n), 1);
    }
}

#[test]
fn XBSC_DeserializePipelineCompileOutput_round_trips() {
    if !up() {
        return;
    }
    unsafe {
        let (_p, h, s, b) = serialized_output();
        let d = c!(XBSC_DeserializePipelineCompileOutput, b.as_ptr(), b.len(), 0);
        assert_eq!(code(d), 0);
        let mut p: *const u8 = core::ptr::null();
        assert_eq!(XBSC_GetDeserializedPipelineCompileOutput(d as *const u8, &mut p), 0);
        assert!(!p.is_null());
        destroy(d);
        let bad = b"FSSM\x01\x02\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        let d = c!(XBSC_DeserializePipelineCompileOutput, bad.as_ptr(), bad.len(), 0);
        assert_eq!(code(d), 1);
        destroy(d);
        destroy(s);
        destroy(h);
    }
}

#[test]
fn XBSC_GetDeserializedPipelineCompileOutput_checks_tag_9() {
    let mut h = [0u8; 0x40];
    h[0x18] = 9;
    h[0x28] = 8;
    let mut p: *const u8 = core::ptr::null();
    unsafe {
        assert_eq!(XBSC_GetDeserializedPipelineCompileOutput(h.as_ptr(), &mut p), 0);
        assert_eq!(p as usize, 8);
        h[0x18] = 0xd;
        assert_eq!(XBSC_GetDeserializedPipelineCompileOutput(h.as_ptr(), &mut p), 1);
        assert_eq!(XBSC_GetDeserializedPipelineCompileOutput(core::ptr::null(), &mut p), 1);
    }
}

#[test]
fn XBSC_CompileRootSignature_compiles_versioned_descs() {
    if !up() {
        return;
    }
    for (ver, ok) in [(1u32, true), (2, true), (3, false)] {
        let mut desc = [0u8; 0x40];
        desc[..4].copy_from_slice(&ver.to_le_bytes());
        unsafe {
            let h = c!(XBSC_CompileRootSignature, desc.as_ptr(), 0, 0, 0, 0, 0);
            assert_ne!(h, 0);
            let mut out = CompiledRootSignature { a: 0, b: 0, c: 0 };
            assert_eq!(XBSC_GetCompiledRootSignature(&mut out, h as *const u8), 0);
            if ok {
                assert_eq!(code(h), 0, "{ver}");
                let s = core::slice::from_raw_parts(out.a as *const u8, 12);
                assert_eq!(s, &[2, 0, 0, 0, 1, 0, 0, 0, 0, 2, 7, 0]);
            } else {
                assert_eq!(code(h), 1);
                assert_eq!(out.a, 0);
            }
            destroy(h);
        }
    }
}

#[test]
fn XBSC_GetCompiledRootSignature_copies_three_fields() {
    let mut h = [0u8; 0x40];
    h[0x18] = 7;
    h[0x20] = 1;
    h[0x28] = 2;
    h[0x30] = 3;
    let mut out = CompiledRootSignature { a: 0, b: 0, c: 0 };
    unsafe {
        assert_eq!(XBSC_GetCompiledRootSignature(&mut out, h.as_ptr()), 0);
        assert_eq!((out.a, out.b, out.c), (1, 2, 3));
        h[0x18] = 6;
        assert_eq!(XBSC_GetCompiledRootSignature(&mut out, h.as_ptr()), 1);
        assert_eq!(XBSC_GetCompiledRootSignature(&mut out, core::ptr::null()), 1);
    }
}

unsafe fn text(h: usize) -> String {
    let mut t: *const u8 = core::ptr::null();
    let mut n = 0usize;
    assert_eq!(XBSC_GetDisassembly(h as *const u8, &mut t, &mut n), 0);
    String::from_utf8_lossy(core::slice::from_raw_parts(t, n)).into_owned()
}

#[test]
fn XBSC_DumpShaderStats_prints_the_compiled_shader_stats() {
    if !up() {
        return;
    }
    unsafe {
        let (_p, h) = compiled();
        let po = XBSC_GetPipelineCompileOutput(h as *const u8);
        let s = c!(XBSC_DumpShaderStats, po, 0);
        assert_eq!(code(s), 0);
        assert_eq!((*(s as *const Handle)).tag, TAG_DISASSEMBLY);
        assert!(!text(s).is_empty());
        destroy(s);
        let s = c!(XBSC_DumpShaderStats, 0, 0);
        assert_eq!(code(s), 0);
        assert_eq!(text(s), "");
        destroy(s);
        destroy(h);
    }
}

#[test]
fn XBSC_DumpProgramShaderStats_has_no_program_for_a_pipeline_output() {
    if !up() {
        return;
    }
    unsafe {
        let (_p, h) = compiled();
        let po = XBSC_GetPipelineCompileOutput(h as *const u8);
        assert_eq!(c!(XBSC_DumpProgramShaderStats, po, 0), 0);
        destroy(h);
    }
}

#[test]
fn XBSC_GetDisassembly_reads_the_string_payload() {
    let mut h = [0u8; 0x40];
    h[0x18] = 0xe;
    h[0x20..0x23].copy_from_slice(b"abc");
    h[0x30] = 3;
    h[0x38] = 0xf;
    let mut t: *const u8 = core::ptr::null();
    let mut n = 0usize;
    unsafe {
        assert_eq!(XBSC_GetDisassembly(h.as_ptr(), &mut t, &mut n), 0);
        assert_eq!(core::slice::from_raw_parts(t, n), b"abc");
        let long = b"0123456789abcdefXYZ";
        h[0x20..0x28].copy_from_slice(&(long.as_ptr() as u64).to_le_bytes());
        h[0x30] = long.len() as u8;
        h[0x38] = 0x1f;
        assert_eq!(XBSC_GetDisassembly(h.as_ptr(), &mut t, &mut n), 0);
        assert_eq!(core::slice::from_raw_parts(t, n), long);
        h[8] = 1;
        assert_eq!(XBSC_GetDisassembly(h.as_ptr(), &mut t, &mut n), 1);
        h[8] = 0;
        h[0x18] = 0xf;
        n = 77;
        assert_eq!(XBSC_GetDisassembly(h.as_ptr(), &mut t, &mut n), 0);
        assert_eq!(n, 77);
        assert_eq!(XBSC_GetDisassembly(core::ptr::null(), &mut t, &mut n), 1);
    }
}

#[test]
fn XBSC_DisassembleHwPipeline_of_no_pipeline_is_empty() {
    if !up() {
        return;
    }
    unsafe {
        let h = c!(XBSC_DisassembleHwPipeline, 0, 0);
        assert_eq!(code(h), 0);
        assert_eq!(text(h), "");
        destroy(h);
    }
}

#[test]
fn XBSC_DisassembleHwPipelineWithCallback_of_no_pipeline_is_empty() {
    if !up() {
        return;
    }
    unsafe {
        let h = c!(XBSC_DisassembleHwPipelineWithCallback, 0, 0, 0);
        assert_eq!(code(h), 0);
        assert_eq!(text(h), "");
        destroy(h);
    }
}

#[test]
fn XBSC_DisassembleRawShader_needs_options() {
    if !up() {
        return;
    }
    let dxil = empty_cs();
    unsafe { assert_eq!(c!(XBSC_DisassembleRawShader, dxil.as_ptr(), dxil.len(), 0, 0), 0) }
}

#[test]
fn XBSC_GetShaderStats_fails_without_a_shader() {
    if !up() {
        return;
    }
    let z = 0u32;
    unsafe { assert_eq!(c!(XBSC_GetShaderStats, 0, 0, &z as *const u32, 0), 1) }
}

#[test]
fn XBSC_GetRegPressure_fails_without_a_shader() {
    if !up() {
        return;
    }
    unsafe { assert_eq!(c!(XBSC_GetRegPressure, 0, 0, 0), 1) }
}

#[test]
fn XBSC_CreatePixDwarfApi_fails_without_a_shader() {
    if !up() {
        return;
    }
    unsafe { assert_eq!(c!(XBSC_CreatePixDwarfApi, 0, 0, 0, 0), 1) }
}

#[test]
fn XBSC_AssembleShader_reports_the_console_restriction() {
    let o = 0usize;
    unsafe {
        let h = crate::XBSC_AssembleShader(core::ptr::null(), 0, &o, core::ptr::null());
        assert_eq!(code(h as usize), 1);
        assert_eq!(message(h as usize), crate::E_ASSEMBLY);
        let mut out: *const u8 = core::ptr::null();
        assert_eq!(XBSC_GetAssembledShader(h as *const u8, &mut out), 1);
        destroy(h as usize);
        assert!(crate::XBSC_AssembleShader(core::ptr::null(), 0, core::ptr::null(), core::ptr::null()).is_null());
    }
}

#[test]
fn XBSC_GetAssembledShader_reads_tag_15() {
    let mut h = [0u8; 0x40];
    h[0x18] = 0xf;
    h[0x28] = 0x55;
    let mut out: *const u8 = core::ptr::null();
    unsafe {
        assert_eq!(XBSC_GetAssembledShader(h.as_ptr(), &mut out), 0);
        assert_eq!(out as usize, 0x55);
        assert_eq!(XBSC_GetAssembledShader(h.as_ptr(), core::ptr::null_mut()), 0);
        h[0x18] = 0xe;
        out = core::ptr::null();
        assert_eq!(XBSC_GetAssembledShader(h.as_ptr(), &mut out), 0);
        assert!(out.is_null());
        assert_eq!(XBSC_GetAssembledShader(core::ptr::null(), &mut out), 1);
    }
}

macro_rules! sc_thunk {
    ($($t:ident = $n:ident;)*) => {$(
        #[test]
        fn $t() {
            assert!(console::FORWARDED.iter().any(|f| f.0 == stringify!($n)));
            if up() {
                assert!(CONSOLE.proc(console::own(), stringify!($n)).is_some());
                assert!(sc_dll_xs_delay_import(), "console thunk target");
            }
        }
    )*};
}

fn sc_dll_xs_delay_import() -> bool {
    let h = CONSOLE.module(console::own()) as *const u8;
    unsafe {
        let pe = h.add(*(h.add(0x3c) as *const u32) as usize);
        let size = *(pe.add(24 + 56) as *const u32) as usize;
        let img = core::slice::from_raw_parts(h, size);
        let want: Vec<u8> = "sc_dll_xs".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        img.windows(want.len()).any(|w| w == &want[..])
    }
}

sc_thunk! {
    SCCompileMultiShadersXbox_forwards_to_console_thunk = SCCompileMultiShadersXbox;
    SCDumpHwShader_forwards_to_console_thunk = SCDumpHwShader;
    SCDumpHwShaderData_forwards_to_console_thunk = SCDumpHwShaderData;
    SCDumpMergedHwShader_forwards_to_console_thunk = SCDumpMergedHwShader;
    SCDumpMergedHwShaderData_forwards_to_console_thunk = SCDumpMergedHwShaderData;
    SCDumpRawShader_forwards_to_console_thunk = SCDumpRawShader;
    SC_ConvertRootSignatureToSCInputs_forwards_to_console_thunk = SC_ConvertRootSignatureToSCInputs;
    SC_OutputHandle_GetShaderObject_forwards_to_console_thunk = SC_OutputHandle_GetShaderObject;
    SC_SCDX12TableDesc_Destroy_forwards_to_console_thunk = SC_SCDX12TableDesc_Destroy;
}

#[test]
fn console_handles_route_destroy_and_message_to_the_console() {
    if !up() {
        return;
    }
    unsafe {
        let h = c!(XBSC_SerializeSrcPipeline, 0, 0);
        assert!(console::is_console_handle(h as *const Handle));
        assert_eq!(code(h), 1);
        assert_eq!(message(h), "unknown error");
        destroy(h);
    }
}
