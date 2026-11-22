use crate::xgs_kmd::{self as kmd, E_FAIL, E_INVALIDARG};
use core::ptr::null_mut;

pub const IOCTL_UPLOAD_MICROCODE: u32 = 0x7900_c223;

#[repr(C)]
pub struct MicrocodeUpload {
    pub kind: u32,
    pub pad: u32,
    pub c: usize,
    pub b: usize,
    pub e: usize,
    pub d: usize,
    pub g: usize,
    pub f: usize,
}

pub unsafe fn submit(m: &MicrocodeUpload) -> i32 {
    kmd::call_opened(IOCTL_UPLOAD_MICROCODE, (m as *const MicrocodeUpload).cast(), 0x38, null_mut(), 0)
}

#[no_mangle]
pub unsafe extern "system" fn D3DUploadCustomMicrocode(kind: u32, b: usize, c: usize, d: usize, e: usize, f: usize, g: usize) -> i32 {
    if kind == 0 || b == 0 || c == 0 || d == 0 || e == 0 || f == 0 || g == 0 {
        return E_INVALIDARG;
    }
    let m = MicrocodeUpload { kind, pad: 0, c, b, e, d, g, f };
    if submit(&m) != 0 {
        E_FAIL
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_console_packet() {
        assert_eq!(core::mem::size_of::<MicrocodeUpload>(), 0x38);
        let m = MicrocodeUpload { kind: 1, pad: 0, c: 3, b: 2, e: 5, d: 4, g: 7, f: 6 };
        let w = unsafe { core::slice::from_raw_parts((&m as *const MicrocodeUpload).cast::<u64>(), 7) };
        assert_eq!(w, &[1, 3, 2, 5, 4, 7, 6]);
    }

    #[test]
    fn zero_arguments_are_rejected() {
        let all = [1usize, 2, 3, 4, 5, 6, 7];
        for i in 0..7 {
            let mut a = all;
            a[i] = 0;
            let r = unsafe { D3DUploadCustomMicrocode(a[0] as u32, a[1], a[2], a[3], a[4], a[5], a[6]) };
            assert_eq!(r, E_INVALIDARG);
        }
    }

    #[test]
    fn valid_upload_fails_on_the_emulated_kmd() {
        let _g = kmd::test_serial();
        let code = [0u32; 16];
        let p = code.as_ptr() as usize;
        assert_eq!(unsafe { D3DUploadCustomMicrocode(1, p, 64, p, 1, 1, p) }, E_FAIL);
        let h = kmd::cached();
        assert!(xgs_kmd_emu::is_device(h));
        assert_eq!(unsafe { kmd::release(h) }, 0);
        assert_eq!(kmd::cached(), kmd::INVALID);
    }
}
