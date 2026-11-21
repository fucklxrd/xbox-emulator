#[cfg(windows)]
use crate::msvc::*;
#[cfg(windows)]
use crate::printf::*;
#[cfg(windows)]
use crate::sys::*;

#[cfg(windows)]
#[test]
fn wstring_grows_like_msvc() {
    unsafe {
        let mut w = WString::empty();
        w.assign(&wide("abc")[..3]);
        assert!(!w.is_large());
        assert_eq!(w.as_slice(), &wide("abc")[..3]);
        let long = wide("0123456789abcdef");
        w.assign(&long[..16]);
        assert!(w.is_large());
        assert_eq!(w.cap, 23);
        assert_eq!(w.as_slice(), &long[..16]);
        w.destroy();
        assert_eq!(w.cap, 7);
    }
}

#[cfg(windows)]
#[test]
fn format_alloc_fixes_line_endings() {
    unsafe {
        let mut p = core::ptr::null_mut();
        let args = [7u64];
        let n = format_alloc(&mut p, b"v=%u\n\0".as_ptr(), args.as_ptr() as *const _, true);
        assert_eq!(n, 5);
        assert_eq!(cslice(p), b"v=7\r\n");
        free(p as *mut _);
        let n = format_alloc(&mut p, b"plain\0".as_ptr(), core::ptr::null(), false);
        assert_eq!(n, 5);
        assert_eq!(cslice(p), b"plain");
        free(p as *mut _);
        assert_eq!(format_alloc(&mut p, core::ptr::null(), core::ptr::null(), false), 0);
        assert!(p.is_null());
    }
}

#[test]
fn ordinal_only_rows_link_noname() {
    let a = crate::exports::link_args("4 #4 implemented | x\n5 Foo implemented\n6 Bar missing | why\n");
    assert_eq!(a, vec!["/EXPORT:XtfOrdinal_4,@4,NONAME".to_string(), "/EXPORT:Foo,@5".to_string()]);
}

#[test]
fn alias_rows_link_to_their_target() {
    let a = crate::exports::link_args("5 Foo implemented =Bar
6 Vt implemented data =Vx | same object
");
    assert_eq!(a, vec!["/EXPORT:Foo=Bar,@5".to_string(), "/EXPORT:Vt=Vx,@6,DATA".to_string()]);
    let e = crate::exports::parse("5 Foo implemented =Bar
");
    assert_eq!(crate::exports::internal_symbol(&e[0]), "Bar");
}

#[cfg(windows)]
#[test]
fn shared_ptr_counts() {
    unsafe {
        let mut a = SharedPtr::null();
        assert_eq!(a.use_count(), 0);
        a.release();
        assert!(a.ptr.is_null());
    }
}
