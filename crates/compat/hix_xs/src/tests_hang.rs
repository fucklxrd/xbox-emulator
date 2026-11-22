use super::*;
use std::sync::Mutex;

static LOG: Mutex<String> = Mutex::new(String::new());

unsafe extern "system" fn sink(s: *const u8) {
    let c = core::ffi::CStr::from_ptr(s as *const core::ffi::c_char);
    LOG.lock().unwrap().push_str(&c.to_string_lossy());
}

#[test]
fn summary_stream_reports_scarlett_with_clean_gcea() {
    let s = hang_stream::summary_stream();
    assert_eq!(s.len(), hang_stream::STREAM_LEN);
    assert_eq!(s[0], hang_stream::CONSOLE_SCARLETT);
    for i in 0..hang_stream::GCEA_COUNT {
        let o = hang_stream::GCEA_STATUS_OFFSET + i * hang_stream::GCEA_STRIDE;
        assert_eq!(u32::from_le_bytes(s[o..o + 4].try_into().unwrap()), 0x300);
    }
    assert_eq!(s.iter().filter(|b| **b != 0).count(), 1 + hang_stream::GCEA_COUNT);
}

#[test]
fn doorbell_answers_hang_summary_with_versioned_stream() {
    let _g = crate::tests::LOCK.lock().unwrap();
    unsafe {
        let mb = kmd::region1();
        let data = kmd::region0();
        let seq = ((mb + kmd::MAILBOX_SEQ) as *const u32).read_volatile();
        ((mb + kmd::MAILBOX_CMD) as *mut u32).write_volatile(hang_stream::CMD_HANG_SUMMARY);
        ((mb + kmd::MAILBOX_SEQ) as *mut u32).write_volatile(seq + 1);
        let ring = [kmd::DOORBELL_MAGIC, 1];
        let mut ret = 0u32;
        assert_eq!(kmd::ioctl(kmd::IOCTL_DOORBELL, ring.as_ptr() as *const u8, 8, core::ptr::null_mut(), 0, &mut ret), 1);
        assert_eq!(((data + kmd::DATA_ACK) as *const u32).read_volatile(), seq + 1);
        assert_eq!(((data + kmd::DATA_SIZE) as *const u32).read_volatile() as usize, hang_stream::STREAM_LEN);
        assert_eq!(((data + hang_stream::VERSION_OFFSET) as *const u32).read_volatile(), hang_stream::STREAM_VERSION);
        assert_eq!(((data + hang_stream::STREAM_OFFSET) as *const u8).read_volatile(), 1);
        ((mb + kmd::MAILBOX_CMD) as *mut u32).write_volatile(0x17);
        ((mb + kmd::MAILBOX_SEQ) as *mut u32).write_volatile(seq + 2);
        kmd::ioctl(kmd::IOCTL_DOORBELL, ring.as_ptr() as *const u8, 8, core::ptr::null_mut(), 0, &mut ret);
        assert_eq!(((data + kmd::DATA_SIZE) as *const u32).read_volatile(), 0);
    }
}

#[test]
fn console_collect_hang_summary_reports_no_faults() {
    let _g = crate::tests::LOCK.lock().unwrap();
    if console::module() == 0 {
        return;
    }
    unsafe {
        crate::tests::init_hix(sink as *const () as usize);
        LOG.lock().unwrap().clear();
        let ctx = [0u8; 0x60];
        let mut report = vec![0u8; 0x1000];
        assert_eq!(CollectHangSummary(ctx.as_ptr(), report.as_mut_ptr()), 0);
        assert_eq!(kmd::last_command(), Some(hang_stream::CMD_HANG_SUMMARY));
        let l = LOG.lock().unwrap().clone();
        assert!(l.contains("Capture was taken from an Xbox Project Scarlett console.\n"), "{}", l);
        for line in ["No GCVM protection faults", "No MMVM protection faults", "No DCN VM protection faults", "No ATC faults", "No GCEA errors", "No CP*_UTCL1_STATUS errors", "No CP_HPD_UTCL1_ERROR errors"] {
            assert!(l.contains(line), "{}", line);
        }
        assert_eq!(TerminateHix(), 0);
    }
}
