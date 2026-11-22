use console_sandbox::*;

fn w(s: &str) -> Vec<u16> {
    wstr::wide(s)
}

fn setup() {
    let d = std::env::temp_dir().join(format!("tombox-verify-lastwrite-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::env::set_var("TOMBOX_GDK_ROOT", &d);
}

unsafe fn written(h: usize) -> u64 {
    let mut t = 0u64;
    let n = core::ptr::null_mut();
    assert_eq!(registry::reg_query_info_key_w(h, core::ptr::null_mut(), n, n, n, n, n, n, n, n, n, &mut t), 0);
    t
}

unsafe fn create(h: usize, s: &str) -> usize {
    let mut out = 0usize;
    let sub = w(s);
    assert_eq!(registry::reg_create_key_ex_w(h, sub.as_ptr(), 0, core::ptr::null(), 0, 0, core::ptr::null(), &mut out, core::ptr::null_mut()), 0);
    out
}

fn pause() {
    std::thread::sleep(std::time::Duration::from_millis(30));
}

#[test]
fn last_write_time_moves_on_every_change() {
    setup();
    unsafe {
        let parent = create(registry::HKEY_LOCAL_MACHINE, "SOFTWARE\\TomBoxVerify\\LastWrite");
        let a = create(parent, "A");
        registry::reg_close_key(a);
        pause();
        let t0 = written(parent);
        pause();
        assert_eq!(regw::reg_delete_key_w(parent, w("A").as_ptr()), 0);
        let t1 = written(parent);
        assert!(t1 > t0, "RegDeleteKeyW bumps the parent last-write time");

        let b = create(parent, "B");
        pause();
        let t2 = written(parent);
        pause();
        assert_eq!(regnt::nt_delete_key(b), 0);
        let t3 = written(parent);
        assert!(t3 > t2, "NtDeleteKey bumps the parent last-write time ({t2} -> {t3})");
        registry::reg_close_key(b);
        registry::reg_close_key(parent);
    }
}

#[test]
fn value_delete_moves_last_write_time() {
    setup();
    unsafe {
        let parent = create(registry::HKEY_LOCAL_MACHINE, "SOFTWARE\\TomBoxVerify\\LastWriteValue");
        let one = 1u32.to_le_bytes();
        let v = w("V");
        assert_eq!(registry::reg_set_value_ex_w(parent, v.as_ptr(), 0, registry::REG_DWORD, one.as_ptr(), 4), 0);
        pause();
        let t4 = written(parent);
        pause();
        assert_eq!(regw::reg_delete_value_w(parent, v.as_ptr()), 0);
        let t5 = written(parent);
        assert!(t5 > t4, "RegDeleteValueW bumps the key last-write time ({t4} -> {t5})");
        registry::reg_close_key(parent);
    }
}
