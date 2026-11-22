use super::*;
use std::sync::{Mutex, Once};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn init() -> std::sync::MutexGuard<'static, ()> {
    static I: Once = Once::new();
    I.call_once(|| {
        let d = std::env::temp_dir().join(format!("tombox-sandbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::env::set_var("TOMBOX_GDK_ROOT", &d);
    });
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn w(s: &str) -> Vec<u16> {
    wstr::wide(s)
}

#[test]
fn paths_stay_inside_the_sandbox() {
    let _g = init();
    let (c, h) = fs::resolve(r"c:\windows\..\..\..\..\x\y.txt").unwrap();
    assert_eq!(c, r"C:\x\y.txt");
    assert!(h.starts_with(fs::fs_root()));
    assert_eq!(fs::resolve(r"boot\a.bsc").unwrap().0, r"C:\Windows\System32\boot\a.bsc");
    assert!(fs::resolve(r"\\server\share\f").is_none());
    assert!(fs::resolve(r"\\.\PhysicalDrive0").is_none());
}

#[test]
fn reg_get_value_reports_sizes_types_and_missing_values() {
    let _g = init();
    let key = r"\Registry\Machine\SOFTWARE\TomBoxSandboxTest";
    registry::set_string(key, "Name", "abc");
    registry::set(key, "Num", registry::REG_DWORD, &7u32.to_le_bytes());
    let mut h = 0usize;
    unsafe {
        assert_eq!(registry::reg_open_key_ex_w(registry::HKEY_LOCAL_MACHINE, w(r"SOFTWARE\TomBoxSandboxTest").as_ptr(), 0, 0x20019, &mut h), 0);
        let mut size = 0u32;
        let mut kind = 0u32;
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Name").as_ptr(), 2, &mut kind, core::ptr::null_mut(), &mut size), 0);
        assert_eq!((kind, size), (registry::REG_SZ, 8));
        let mut small = [0u8; 4];
        size = 4;
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Name").as_ptr(), 2, &mut kind, small.as_mut_ptr(), &mut size), registry::ERROR_MORE_DATA);
        assert_eq!(size, 8);
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Num").as_ptr(), 2, &mut kind, core::ptr::null_mut(), &mut size), registry::ERROR_UNSUPPORTED_TYPE);
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Gone").as_ptr(), 0xffff, &mut kind, core::ptr::null_mut(), &mut size), registry::ERROR_FILE_NOT_FOUND);
        let mut name = [0u16; 16];
        let mut nlen = 16u32;
        assert_eq!(registry::reg_enum_value_w(h, 1, name.as_mut_ptr(), &mut nlen, core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), core::ptr::null_mut()), 0);
        assert_eq!(String::from_utf16_lossy(&name[..nlen as usize]), "Num");
        nlen = 16;
        assert_eq!(registry::reg_enum_value_w(h, 2, name.as_mut_ptr(), &mut nlen, core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), core::ptr::null_mut()), registry::ERROR_NO_MORE_ITEMS);
        assert_eq!(registry::reg_close_key(h), 0);
        assert_eq!(registry::reg_close_key(h), registry::ERROR_INVALID_HANDLE);
    }
}

#[test]
fn scm_queries_answer_from_the_in_process_host() {
    let _g = init();
    host::add_service("TomBoxSvc");
    unsafe {
        let scm = host::open_sc_manager_w(core::ptr::null(), core::ptr::null(), 1);
        assert_ne!(scm, 0);
        assert_eq!(host::open_service_w(scm, w("NoSuchSvc").as_ptr(), 4), 0);
        let s = host::open_service_w(scm, w("TomBoxSvc").as_ptr(), 4);
        assert_ne!(s, 0);
        let mut buf = [0u32; 9];
        let mut need = 0u32;
        assert_eq!(host::query_service_status_ex(s, 0, buf.as_mut_ptr() as *mut u8, 8, &mut need), 0);
        assert_eq!(need, 36);
        assert_eq!(host::query_service_status_ex(s, 0, buf.as_mut_ptr() as *mut u8, 36, &mut need), 1);
        assert_eq!(buf[1], host::SERVICE_STOPPED);
        assert_eq!(host::close_service_handle(s), 1);
        assert_eq!(host::close_service_handle(s), 0);
    }
}

#[test]
fn hostname_size_contract_matches_get_computer_name_ex() {
    let _g = init();
    registry::set_string(system::COMPUTER_NAME, "ComputerName", "SANDBOXPC");
    let mut b = [0u16; 4];
    let mut n = 4u32;
    unsafe {
        assert_eq!(system::get_computer_name_ex_w(4, b.as_mut_ptr(), &mut n), 0);
        assert_eq!(n, 10);
        let mut big = [0u16; 16];
        n = 16;
        assert_eq!(system::get_computer_name_ex_w(4, big.as_mut_ptr(), &mut n), 1);
        assert_eq!(String::from_utf16_lossy(&big[..n as usize]), "SANDBOXPC");
        assert_eq!(system::get_computer_name_ex_w(9, big.as_mut_ptr(), &mut n), 0);
    }
}
#[test]
fn import_walk_includes_delay_loads() {
    let _g = init();
    let m = console_image::load_path(std::path::Path::new(r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\netprofmsvc_c.dll"));
    if m == 0 {
        return;
    }
    let imps = unsafe { imports(m) };
    assert!(imps.iter().any(|(d, n)| d.eq_ignore_ascii_case("NSI.dll") && n == "NsiSetAllParameters"), "delay import listed");
}
#[test]
fn reg_query_enum_and_delete_stay_in_the_emulated_hive() {
    let _g = init();
    let key = r"\Registry\Machine\SOFTWARE\TomBoxRegW";
    registry::set(key, "Bin", registry::REG_BINARY, &[1, 2, 3]);
    registry::create_key(&format!(r"{key}\Beta"));
    registry::create_key(&format!(r"{key}\Alpha"));
    let mut h = 0usize;
    unsafe {
        assert_eq!(registry::reg_open_key_ex_w(registry::HKEY_LOCAL_MACHINE, w(r"SOFTWARE\TomBoxRegW").as_ptr(), 0, 0x20019, &mut h), 0);
        let (mut kind, mut size) = (0u32, 0u32);
        assert_eq!(regw::reg_query_value_ex_w(h, w("bin").as_ptr(), core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), &mut size), 0);
        assert_eq!((kind, size), (registry::REG_BINARY, 3));
        let mut buf = [0u8; 2];
        size = 2;
        assert_eq!(regw::reg_query_value_ex_w(h, w("Bin").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), buf.as_mut_ptr(), &mut size), registry::ERROR_MORE_DATA);
        assert_eq!(size, 3);
        let mut big = [0u8; 8];
        size = 8;
        assert_eq!(regw::reg_query_value_ex_w(h, w("Bin").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), big.as_mut_ptr(), &mut size), 0);
        assert_eq!(&big[..3], &[1, 2, 3]);
        assert_eq!(regw::reg_query_value_ex_w(h, w("Nope").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), &mut size), registry::ERROR_FILE_NOT_FOUND);
        assert_eq!(regw::reg_query_value_ex_w(h, w("Bin").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), big.as_mut_ptr(), core::ptr::null_mut()), registry::ERROR_INVALID_PARAMETER);
        let mut name = [0u16; 16];
        let mut n = 16u32;
        assert_eq!(regw::reg_enum_key_ex_w(h, 0, name.as_mut_ptr(), &mut n, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()), 0);
        assert_eq!(String::from_utf16_lossy(&name[..n as usize]), "Alpha");
        let mut small = [0u16; 3];
        n = 3;
        assert_eq!(regw::reg_enum_key_ex_w(h, 1, small.as_mut_ptr(), &mut n, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()), registry::ERROR_MORE_DATA);
        assert_eq!(regw::reg_enum_key_w(h, 1, name.as_mut_ptr(), 16), 0);
        assert_eq!(String::from_utf16_lossy(&name[..4]), "Beta");
        assert_eq!(regw::reg_enum_key_w(h, 2, name.as_mut_ptr(), 16), registry::ERROR_NO_MORE_ITEMS);
        assert_eq!(regw::reg_delete_value_w(h, w("Bin").as_ptr()), 0);
        assert_eq!(regw::reg_delete_value_w(h, w("Bin").as_ptr()), registry::ERROR_FILE_NOT_FOUND);
        assert!(registry::get(key, "Bin").is_none());
        assert_eq!(regw::reg_delete_key_w(registry::HKEY_LOCAL_MACHINE, w(r"SOFTWARE\TomBoxRegW").as_ptr()), regw::ERROR_ACCESS_DENIED);
        assert_eq!(regw::reg_delete_key_w(h, w("Alpha").as_ptr()), 0);
        assert!(!registry::key_exists(&format!(r"{key}\Alpha")));
        assert_eq!(regw::reg_delete_key_w(h, w("Alpha").as_ptr()), registry::ERROR_FILE_NOT_FOUND);
        assert_eq!(regw::reg_enum_key_w(h, 0, name.as_mut_ptr(), 16), 0);
        assert_eq!(String::from_utf16_lossy(&name[..4]), "Beta");
        registry::reg_close_key(h);
    }
}

static NOTIFIED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe extern "system" fn on_notify(buffer: *mut core::ffi::c_void) {
    let b = buffer as *const u8;
    let flag = (b.add(0x10) as *const *mut u8).read_unaligned();
    *flag = 1;
    NOTIFIED.store(((b.add(0x40) as *const u32).read_unaligned() as usize) << 8 | (b.add(0x20) as *const u32).read_unaligned() as usize, std::sync::atomic::Ordering::SeqCst);
}

#[link(name = "kernel32")]
extern "system" {
    fn SleepEx(ms: u32, alertable: i32) -> u32;
}

unsafe extern "system" fn no_handler(_c: u32, _t: u32, _d: *mut core::ffi::c_void, _x: *mut core::ffi::c_void) -> u32 {
    0
}

#[test]
fn scm_a_and_status_notifications_follow_win32() {
    let _g = init();
    unsafe {
        let scm = notify::open_sc_manager_a(core::ptr::null(), core::ptr::null(), 4);
        assert_ne!(scm, 0);
        assert_eq!(notify::open_service_a(scm, b"No/Such\0".as_ptr(), 5), 0);
        let rpcss = notify::open_service_a(scm, b"RpcSs\0".as_ptr(), 5);
        assert_ne!(rpcss, 0, "RpcSs is the live in-process RPC runtime");
        let mut flag = 0u8;
        let mut buf = [0u8; 0x50];
        (buf.as_mut_ptr() as *mut u32).write_unaligned(2);
        (buf.as_mut_ptr().add(8) as *mut usize).write_unaligned(on_notify as *const () as usize);
        (buf.as_mut_ptr().add(0x10) as *mut usize).write_unaligned(&mut flag as *mut u8 as usize);
        assert_eq!(notify::notify_service_status_change(rpcss, 0, buf.as_mut_ptr().cast()), host::ERROR_INVALID_PARAMETER);
        assert_eq!(notify::notify_service_status_change(0x1234, 8, buf.as_mut_ptr().cast()), host::ERROR_INVALID_HANDLE);
        assert_eq!(notify::notify_service_status_change(rpcss, 8, buf.as_mut_ptr().cast()), 0);
        assert_eq!(flag, 0, "the callback is an APC, not a direct call");
        assert_eq!(SleepEx(5000, 1), 0xc0, "WAIT_IO_COMPLETION");
        assert_eq!(flag, 1);
        assert_eq!(NOTIFIED.load(std::sync::atomic::Ordering::SeqCst), (8 << 8) | host::SERVICE_RUNNING as usize);
        assert_eq!(&buf[0x18..0x1c], &[0, 0, 0, 0]);

        host::add_service("TomBoxNotifySvc");
        let svc = notify::open_service_a(scm, b"TomBoxNotifySvc\0".as_ptr(), 5);
        assert_ne!(svc, 0);
        flag = 0;
        assert_eq!(notify::notify_service_status_change(svc, 8, buf.as_mut_ptr().cast()), 0);
        assert_eq!(SleepEx(0, 1), 0, "stopped service: nothing delivered yet");
        let name = w("TomBoxNotifySvc");
        let st = host::register_service_ctrl_handler_ex_w(name.as_ptr(), no_handler as *mut core::ffi::c_void, core::ptr::null_mut());
        assert_ne!(st, 0);
        let start_pending = [0x10u32, 2, 0, 0, 0, 0, 0];
        assert_eq!(host::set_service_status(st, start_pending.as_ptr()), 1);
        assert_eq!(SleepEx(0, 1), 0, "START_PENDING is not in the RUNNING mask");
        let running = [0x10u32, 4, 1, 0, 0, 0, 0];
        assert_eq!(host::set_service_status(st, running.as_ptr()), 1);
        assert_eq!(SleepEx(5000, 1), 0xc0);
        assert_eq!(flag, 1);
        flag = 0;
        assert_eq!(notify::notify_service_status_change(svc, 1, buf.as_mut_ptr().cast()), 0);
        assert_eq!(host::close_service_handle(svc), 1);
        let stopped = [0x10u32, 1, 0, 0, 0, 0, 0];
        assert_eq!(host::set_service_status(st, stopped.as_ptr()), 1);
        assert_eq!(SleepEx(0, 1), 0, "closing the handle cancels its notification");
        assert_eq!(flag, 0);
        host::close_service_handle(rpcss);
        host::close_service_handle(scm);
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateEventW(sa: *const core::ffi::c_void, manual: i32, initial: i32, name: *const u16) -> usize;
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
    fn CloseHandle(h: usize) -> i32;
    fn OpenProcessToken(p: usize, a: u32, h: *mut usize) -> i32;
    fn GetCurrentProcess() -> usize;
}

fn open_xvcmgmt() -> usize {
    let name = w(xvcmgmt::DEVICE);
    let bytes = (name.len() - 1) * 2;
    let us: [usize; 2] = [bytes | (bytes << 16), name.as_ptr() as usize];
    let oa: [usize; 6] = [0x30, 0, us.as_ptr() as usize, 0x40, 0, 0];
    let mut h = 0usize;
    let mut io = [7usize; 2];
    assert_eq!(unsafe { xvcmgmt::nt_open_file(&mut h, 0x100003, oa.as_ptr() as *const u8, io.as_mut_ptr(), 3, 0) }, 0);
    assert_eq!(io, [0, 1], "IOSB: STATUS_SUCCESS, FILE_OPENED");
    assert!(xvcmgmt::is_device(h));
    h
}

struct Call {
    io: Box<[usize; 2]>,
    ev: usize,
}

impl Call {
    fn new() -> Call {
        Call { io: Box::new([0x55; 2]), ev: unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) } }
    }
    fn run(&mut self, h: usize, code: u32, inb: &[u8], out: &mut [u8]) -> i32 {
        let o = if out.is_empty() { core::ptr::null_mut() } else { out.as_mut_ptr() };
        unsafe { xvcmgmt::nt_device_io_control_file(h, self.ev, 0, 0, self.io.as_mut_ptr(), code, inb.as_ptr(), inb.len() as u32, o, out.len() as u32) }
    }
    fn wait(&self, ms: u32) -> Option<(i32, usize)> {
        (unsafe { WaitForSingleObject(self.ev, ms) } == 0).then(|| (self.io[0] as u32 as i32, self.io[1]))
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.ev) };
    }
}

fn create(h: usize, vm: [u8; 16], svc: [u8; 16]) -> u64 {
    let mut inb = vm.to_vec();
    inb.extend_from_slice(&svc);
    let mut out = [0u8; 8];
    let mut c = Call::new();
    assert_eq!(c.run(h, xvcmgmt::IOCTL_CREATE, &inb, &mut out), 0);
    assert_eq!(c.wait(0), Some((0, 8)));
    u64::from_le_bytes(out)
}

fn pend(h: usize, code: u32, id: u64, out: &mut [u8]) -> Call {
    let mut c = Call::new();
    assert_eq!(c.run(h, code, &id.to_le_bytes(), out), xvcmgmt::STATUS_PENDING, "{code:#x} pends like the driver's worker");
    c
}

fn send(h: usize, id: u64, data: &[u8]) -> i32 {
    let mut inb = id.to_le_bytes().to_vec();
    inb.extend_from_slice(&(data.as_ptr() as u64).to_le_bytes());
    inb.extend_from_slice(&(data.len() as u32).to_le_bytes());
    Call::new().run(h, xvcmgmt::IOCTL_SEND, &inb, &mut [])
}

#[test]
fn xvcmgmt_queue_follows_the_driver() {
    let _g = init();
    let h = open_xvcmgmt();
    let svc = [0x42u8; 16];
    let mut c = Call::new();
    assert_eq!(c.run(h, xvcmgmt::IOCTL_CREATE, &[0; 0x1f], &mut [0; 8]), xvcmgmt::STATUS_INVALID_PARAMETER);
    assert_eq!(c.wait(0), None, "a synchronous failure leaves the event and IOSB alone");
    assert_eq!(c.run(h, xvcmgmt::IOCTL_RECEIVE, &7u64.to_le_bytes(), &mut []), xvcmgmt::STATUS_BUFFER_TOO_SMALL);
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_ACCEPT, &[0; 8], &mut [0; 4]), xvcmgmt::STATUS_BUFFER_TOO_SMALL);
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_SEND, &[0; 0x15], &mut []), xvcmgmt::STATUS_INVALID_PARAMETER);
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_CLOSE, &0xdeadu64.to_le_bytes(), &mut []), 0, "close of an unknown id is STATUS_SUCCESS");
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_CONNECT, &0xdeadu64.to_le_bytes(), &mut []), xvcmgmt::STATUS_INVALID_HANDLE);

    let lone = create(h, xvcmgmt::LOOPBACK_VM, [0x99; 16]);
    assert_eq!(pend(h, xvcmgmt::IOCTL_CONNECT, lone, &mut []).wait(5000), Some((xvcmgmt::STATUS_CONNECTION_REFUSED, 0)), "no listener");

    let server = create(h, xvcmgmt::LOOPBACK_VM, svc);
    let a = create(h, xvcmgmt::LOOPBACK_VM, svc);
    assert!(a > server && server >= 2);
    assert_eq!(pend(h, xvcmgmt::IOCTL_LISTEN, server, &mut []).wait(5000), Some((0, 0)));
    let mut accepted = [0u8; 8];
    let mut acc = Call::new();
    assert_eq!(acc.run(h, xvcmgmt::IOCTL_ACCEPT, &server.to_le_bytes(), &mut accepted), xvcmgmt::STATUS_PENDING);
    assert_eq!(pend(h, xvcmgmt::IOCTL_CONNECT, a, &mut []).wait(5000), Some((0, 0)));
    assert_eq!(acc.wait(5000), Some((0, 8)));
    let b = u64::from_le_bytes(accepted);
    assert!(b > a);

    let mut buf = [0u8; 4];
    let r = pend(h, xvcmgmt::IOCTL_RECEIVE, b, &mut buf);
    assert_eq!(r.wait(100), None, "receive stays pending until bytes arrive");
    assert_eq!(send(h, a, b"hello"), 0);
    assert_eq!(r.wait(5000), Some((0, 4)), "stream semantics: as many bytes as fit");
    assert_eq!(&buf, b"hell");
    let mut rest = [0u8; 16];
    assert_eq!(pend(h, xvcmgmt::IOCTL_RECEIVE, b, &mut rest).wait(5000), Some((0, 1)));
    assert_eq!(rest[0], b'o');

    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_SHUTDOWN, &a.to_le_bytes(), &mut []), 0);
    assert_eq!(pend(h, xvcmgmt::IOCTL_RECEIVE, b, &mut rest).wait(5000), Some((0, 0)), "graceful disconnect reads as 0 bytes");
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_SHUTDOWN, &a.to_le_bytes(), &mut []), xvcmgmt::STATUS_NOT_CONNECTED);

    let mut pa = Call::new();
    assert_eq!(pa.run(h, xvcmgmt::IOCTL_ACCEPT, &server.to_le_bytes(), &mut accepted), xvcmgmt::STATUS_PENDING);
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_CLOSE, &server.to_le_bytes(), &mut []), 0);
    assert_eq!(pa.wait(5000), Some((xvcmgmt::STATUS_LOCAL_DISCONNECT, 0)), "pending accepts complete with STATUS_LOCAL_DISCONNECT");

    let host = create(h, xvcmgmt::HOST_VM, claims::launch_instance());
    assert_eq!(pend(h, xvcmgmt::IOCTL_CONNECT, host, &mut []).wait(5000), Some((0, 0)), "the emulated host listens on c12a416b + XBOX://LI");
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_SHUTDOWN, &host.to_le_bytes(), &mut []), xvcmgmt::STATUS_NOT_IMPLEMENTED_PEER, "graceful disconnect is loopback-only");
    let conns = xvcmgmt::host_connections(claims::launch_instance());
    assert_eq!(conns.len(), 1);
    assert_eq!(send(h, host, b"to-host"), 0);
    assert_eq!(xvcmgmt::host_take_received(conns[0]), b"to-host");
    let mut got = [0u8; 8];
    let hr = pend(h, xvcmgmt::IOCTL_RECEIVE, host, &mut got);
    assert_eq!(xvcmgmt::host_send(conns[0], b"reply"), 0);
    assert_eq!(hr.wait(5000), Some((0, 5)));
    assert_eq!(&got[..5], b"reply");
    let hr2 = pend(h, xvcmgmt::IOCTL_RECEIVE, host, &mut got);
    xvcmgmt::host_close(conns[0]);
    assert_eq!(hr2.wait(5000), Some((xvcmgmt::STATUS_CONNECTION_RESET, 0)), "abortive peer close");
    assert_eq!(send(h, host, b"x"), xvcmgmt::STATUS_CONNECTION_INVALID);
    let cancelled = pend(h, xvcmgmt::IOCTL_RECEIVE, lone, &mut got);
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_CLOSE, &lone.to_le_bytes(), &mut []), 0);
    assert_eq!(cancelled.wait(5000), Some((xvcmgmt::STATUS_CANCELLED, 0)), "close cancels in-flight receives");
    assert_eq!(Call::new().run(h, 0x150094, &[], &mut [0; 4]), xvcmgmt::STATUS_INVALID_DEVICE_REQUEST);
    let mut dup = 0usize;
    extern "system" {
        fn DuplicateHandle(sp: usize, sh: usize, tp: usize, th: *mut usize, access: u32, inherit: i32, opts: u32) -> i32;
    }
    assert_ne!(unsafe { DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &mut dup, 0, 0, 2) }, 0);
    assert!(xvcmgmt::is_device(dup), "a duplicated device handle still reaches the device");
    assert_eq!(unsafe { nt_close(h) }, 0);
    assert!(!xvcmgmt::is_device(h), "a closed handle value is no longer the device");
    assert_eq!(Call::new().run(dup, xvcmgmt::IOCTL_CLOSE, &0u64.to_le_bytes(), &mut []), 0);
    unsafe { CloseHandle(dup) };
    assert!(!xvcmgmt::is_device(dup));
}

#[test]
fn launch_claim_is_merged_into_the_process_token_attributes() {
    let _g = init();
    let mut need = 0u32;
    assert_eq!(unsafe { claims::nt_query_information_token(usize::MAX - 3, claims::TOKEN_SECURITY_ATTRIBUTES, core::ptr::null_mut(), 0, &mut need) }, 0xC000_0023u32 as i32);
    let mut buf = vec![0u64; need as usize / 8 + 1];
    assert_eq!(unsafe { claims::nt_query_information_token(usize::MAX - 3, claims::TOKEN_SECURITY_ATTRIBUTES, buf.as_mut_ptr() as *mut u8, need, &mut need) }, 0);
    let attrs = unsafe { claims::parse(buf.as_ptr() as usize) }.unwrap();
    let li: Vec<&claims::Attribute> = attrs.iter().filter(|a| String::from_utf16_lossy(&a.name) == "XBOX://LI").collect();
    assert_eq!(li.len(), 1);
    assert_eq!(li[0].kind, claims::TYPE_OCTET_STRING);
    assert_eq!(li[0].values, vec![claims::Value::Bytes(claims::launch_instance().to_vec())]);
    let real = attrs.len() - 1;
    let round = claims::merge(attrs.clone(), claims::emulated());
    assert_eq!(round.len(), real + 1, "merging again replaces, never duplicates");
    let mut again = vec![0u64; claims::size_of(&round) / 8 + 1];
    unsafe { claims::write(&round, again.as_mut_ptr() as usize) };
    assert_eq!(unsafe { claims::parse(again.as_ptr() as usize) }.unwrap(), round);
    assert_eq!(claims::parse_guid(&claims::format_guid(&claims::launch_instance())), Some(claims::launch_instance()));
    assert_eq!(claims::parse_guid("c12a416b-f347-405c-b765-2de027a6249e"), Some(xvcmgmt::HOST_VM));
    assert_eq!(claims::parse_guid("e0e16197-dd56-4a10-9195-5ee7a155a838"), Some(xvcmgmt::LOOPBACK_VM));
    assert!(unsafe { claims::is_process_token(usize::MAX - 3) });
    let mut other = 0usize;
    assert_ne!(unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut other) }, 0);
    assert!(unsafe { claims::is_process_token(other) });
    unsafe { CloseHandle(other) };
}
