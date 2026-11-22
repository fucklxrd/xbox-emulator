use crate::computer::E_INVALIDARG;
use crate::dcc_codec::run::{self, Call};
use compat_common::record;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct DccParams {
    pub block_shift: u32,
    pub mode: u32,
}

fn args_ok(format: i32, tile_mode: i32) -> bool {
    format != 0 && format <= 0x78 && tile_mode < 0x1f
}

fn negative(name: &str, format: i32) -> i32 {
    record(crate::M, name, &format!("format {format:#x} is negative; the console sign-extends it into its format table and reads outside it, native rejects"));
    E_INVALIDARG
}

#[no_mangle]
pub unsafe extern "system" fn XGEncodeDCC(count: u32, src: *const u8, dst: *mut u8, meta: *mut u8, sizes: *mut u32, format: i32, tile_mode: i32, params: *const DccParams) -> i32 {
    if params.is_null() || src.is_null() || dst.is_null() || meta.is_null() || !args_ok(format, tile_mode) {
        return E_INVALIDARG;
    }
    if format < 0 {
        return negative("XGEncodeDCC", format);
    }
    let p = *params;
    let n = count as usize;
    let src = core::slice::from_raw_parts(src, n * 256);
    let dst = core::slice::from_raw_parts_mut(dst, n * 256);
    let meta = core::slice::from_raw_parts_mut(meta, n);
    let sizes = (!sizes.is_null()).then(|| core::slice::from_raw_parts_mut(sizes, n));
    let c = Call { format, tile_mode, params: [p.block_shift, p.mode], name: "XGEncodeDCC" };
    run::encode(&c, src, dst, meta, sizes)
}

#[no_mangle]
pub unsafe extern "system" fn XGDecodeDCC(count: u32, src: *const u8, meta: *const u8, dst: *mut u8, format: i32, tile_mode: i32, params: *const DccParams) -> i32 {
    if params.is_null() || src.is_null() || dst.is_null() || meta.is_null() || !args_ok(format, tile_mode) {
        return E_INVALIDARG;
    }
    if format < 0 {
        return negative("XGDecodeDCC", format);
    }
    let p = *params;
    let n = count as usize;
    let src = core::slice::from_raw_parts(src, n * 256);
    let meta = core::slice::from_raw_parts(meta, n);
    let dst = core::slice::from_raw_parts_mut(dst, n * 256);
    let c = Call { format, tile_mode, params: [p.block_shift, p.mode], name: "XGDecodeDCC" };
    run::decode(&c, src, meta, dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_reject_like_console() {
        let (src, mut dst, mut meta, p) = ([0u8; 256], [0u8; 256], [0u8; 1], DccParams::default());
        let mut e = |f: i32, m: i32, pp: *const DccParams| unsafe { XGEncodeDCC(1, src.as_ptr(), dst.as_mut_ptr(), meta.as_mut_ptr(), core::ptr::null_mut(), f, m, pp) };
        assert_eq!(e(28, 14, core::ptr::null()), E_INVALIDARG);
        assert_eq!(e(0, 14, &p), E_INVALIDARG);
        assert_eq!(e(0x79, 14, &p), E_INVALIDARG);
        assert_eq!(e(-3, 14, &p), E_INVALIDARG);
        assert_eq!(e(28, 0x1b, &p), E_INVALIDARG);
        assert_eq!(e(28, -1, &p), E_INVALIDARG);
        assert_eq!(e(19, 14, &p), E_INVALIDARG);
        assert_eq!(unsafe { XGEncodeDCC(1, core::ptr::null(), dst.as_mut_ptr(), meta.as_mut_ptr(), core::ptr::null_mut(), 28, 14, &p) }, E_INVALIDARG);
        let mut dd = [0u8; 256];
        assert_eq!(unsafe { XGDecodeDCC(1, src.as_ptr(), meta.as_ptr(), dd.as_mut_ptr(), 105, 14, &p) }, E_INVALIDARG);
        assert_eq!(unsafe { XGDecodeDCC(1, src.as_ptr(), meta.as_ptr(), dd.as_mut_ptr(), 2, 0x1e, &p) }, E_INVALIDARG);
        assert_eq!(unsafe { XGDecodeDCC(1, src.as_ptr(), core::ptr::null(), dd.as_mut_ptr(), 71, 14, &p) }, E_INVALIDARG);
    }

    #[test]
    fn encode_then_decode_round_trips_a_flat_block() {
        let p = DccParams::default();
        let src = [0x40u8; 256];
        let (mut enc, mut meta, mut size, mut out) = ([0u8; 256], [0u8; 1], [0u32; 1], [0u8; 256]);
        assert_eq!(unsafe { XGEncodeDCC(1, src.as_ptr(), enc.as_mut_ptr(), meta.as_mut_ptr(), size.as_mut_ptr(), 28, 14, &p) }, 0);
        assert!(size[0] <= 256);
        assert_eq!(unsafe { XGDecodeDCC(1, enc.as_ptr(), meta.as_ptr(), out.as_mut_ptr(), 28, 14, &p) }, 0);
        assert_eq!(out, src);
    }
}
