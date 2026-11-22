use super::*;
use std::collections::VecDeque;
use std::ffi::CStr;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;


struct Pending {
    buf: usize,
    len: u32,
    ov: usize,
}

struct Ep {
    id: usize,
    port: Option<u32>,
    inbox: VecDeque<u8>,
    eof: bool,
    pending: Option<Pending>,
}

struct Fake {
    eps: Vec<Ep>,
    next_id: usize,
    next_port: u32,
    reserve_ov: usize,
}

static FAKE: Mutex<Fake> = Mutex::new(Fake { eps: Vec::new(), next_id: 0x5000, next_port: 0x9000, reserve_ov: 0 });
static FAIL_OPEN: AtomicU32 = AtomicU32::new(0);

fn fake() -> MutexGuard<'static, Fake> {
    FAKE.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe fn complete(p: Pending, n: u32, status: usize) {
    let ov = p.ov as *mut OVERLAPPED;
    (*ov).internal_high = n as usize;
    (*ov).internal = status;
    SetEvent(((*ov).event as usize & !1) as HANDLE);
}

unsafe fn deliver(ep: &mut Ep, data: &[u8]) {
    ep.inbox.extend(data.iter().copied());
    if let Some(p) = ep.pending.take() {
        let n = (p.len as usize).min(ep.inbox.len());
        for i in 0..n {
            *(p.buf as *mut u8).add(i) = ep.inbox.pop_front().unwrap();
        }
        complete(p, n as u32, 0);
    }
}

unsafe fn finish(ep: &mut Ep) {
    ep.eof = true;
    if ep.inbox.is_empty() {
        if let Some(p) = ep.pending.take() {
            complete(p, 0, 0);
        }
    }
}

unsafe fn fake_open() -> HANDLE {
    if FAIL_OPEN.load(Ordering::SeqCst) != 0 {
        return INVALID_HANDLE_VALUE;
    }
    let mut f = fake();
    let id = f.next_id;
    f.next_id += 4;
    f.eps.push(Ep { id, port: None, inbox: VecDeque::new(), eof: false, pending: None });
    id as HANDLE
}

unsafe fn fake_ioctl(h: HANDLE, code: u32, i: *const c_void, il: u32, o: *mut c_void, ol: u32, r: *mut u32, ov: *mut OVERLAPPED) -> i32 {
    let mut f = fake();
    let id = h as usize;
    if code == IOCTL_RESERVE {
        assert!(i.is_null() && il == 0 && ol == 0x18);
        let out = o as *mut u64;
        *out.add(1) = 0x7ff0_0000_0000;
        *out.add(2) = 0x20_0000;
        (*ov).internal = 0x103;
        f.reserve_ov = ov as usize;
        SetLastError(0x3e5);
        return 0;
    }
    let me = match f.eps.iter().position(|e| e.id == id) {
        Some(k) => k,
        None => {
            SetLastError(6);
            return 0;
        }
    };
    if code == IOCTL_RECV {
        assert_eq!(il, 4);
        assert_eq!(ol, 0x10);
        let wb = *(o as *const WSABUF);
        let ep = &mut f.eps[me];
        if !ep.inbox.is_empty() {
            let n = (wb.len as usize).min(ep.inbox.len());
            for k in 0..n {
                *wb.buf.add(k) = ep.inbox.pop_front().unwrap();
            }
            *r = n as u32;
            (*ov).internal = 0;
            (*ov).internal_high = n;
            return 1;
        }
        if ep.eof {
            *r = 0;
            (*ov).internal = 0;
            (*ov).internal_high = 0;
            return 1;
        }
        (*ov).internal = 0x103;
        ep.pending = Some(Pending { buf: wb.buf as usize, len: wb.len, ov: ov as usize });
        SetLastError(0x3e5);
        return 0;
    }
    assert_eq!(code, IOCTL_SEND);
    assert_eq!(*(i as *const u32), 0);
    let n = (ol >> 4) as usize;
    let arr = core::slice::from_raw_parts(o as *const WSABUF, n);
    assert_eq!(arr[0].len, 0x80);
    let hdr = core::slice::from_raw_parts(arr[0].buf, 0x80).to_vec();
    let dst = u32::from_le_bytes([hdr[0x24], hdr[0x25], hdr[0x26], hdr[0x27]]);
    let mut payload = Vec::new();
    for b in &arr[1..] {
        payload.extend_from_slice(core::slice::from_raw_parts(b.buf, b.len as usize));
    }
    let peer = f.eps.iter().position(|e| e.port == Some(dst) && e.port.is_some());
    match hdr[0] {
        0x80 => {
            let svc = u32::from_le_bytes([hdr[0x10], hdr[0x11], hdr[0x12], hdr[0x13]]);
            let port = if svc != 0 {
                svc
            } else {
                f.next_port += 1;
                f.next_port
            };
            f.eps[me].port = Some(port);
            let mut rep = hdr.clone();
            rep[0x10..0x14].copy_from_slice(&port.to_le_bytes());
            rep[0x14..0x24].fill(0);
            rep[0x23] = 1;
            deliver(&mut f.eps[me], &rep);
        }
        2 | 3 => {
            if let Some(p) = peer {
                deliver(&mut f.eps[p], &hdr);
            }
        }
        1 => {
            assert_eq!(u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize, payload.len());
            if let Some(p) = peer {
                deliver(&mut f.eps[p], &payload);
            }
        }
        5 => {
            finish(&mut f.eps[me]);
            if let Some(p) = peer {
                finish(&mut f.eps[p]);
            }
        }
        _ => panic!("unexpected link message {:#x}", hdr[0]),
    }
    *r = (0x80 + payload.len()) as u32;
    1
}

unsafe fn fake_cancel_io(h: HANDLE) -> i32 {
    let mut f = fake();
    if let Some(ep) = f.eps.iter_mut().find(|e| e.id == h as usize) {
        if let Some(p) = ep.pending.take() {
            complete(p, 0, 0xc000_0120);
        }
    }
    1
}

unsafe fn fake_cancel_io_ex(_h: HANDLE, ov: *mut OVERLAPPED) -> i32 {
    let mut f = fake();
    if f.reserve_ov == ov as usize {
        (*ov).internal = 0xc000_0120;
        f.reserve_ov = 0;
    }
    1
}

unsafe fn fake_close(h: HANDLE) -> i32 {
    let mut f = fake();
    if let Some(k) = f.eps.iter().position(|e| e.id == h as usize) {
        let mut ep = f.eps.remove(k);
        if let Some(p) = ep.pending.take() {
            complete(p, 0, 0xc000_0120);
        }
    }
    1
}

static FAKE_DEVICE: DeviceOps = DeviceOps {
    open: fake_open,
    ioctl: fake_ioctl,
    cancel_io: fake_cancel_io,
    cancel_io_ex: fake_cancel_io_ex,
    close: fake_close,
};

struct Env {
    _g: MutexGuard<'static, ()>,
}

impl Drop for Env {
    fn drop(&mut self) {
        set_device_ops(&REAL_DEVICE);
        FAIL_OPEN.store(0, Ordering::SeqCst);
        reset_reserved_device();
    }
}

fn env(ops: &'static DeviceOps) -> Env {
    let g = crate::conn::tests::NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    {
        let mut f = fake();
        f.eps.clear();
        f.reserve_ov = 0;
    }
    FAIL_OPEN.store(0, Ordering::SeqCst);
    reset_reserved_device();
    set_device_ops(ops);
    Env { _g: g }
}

struct Send2(*mut u8);
unsafe impl Send for Send2 {}

fn sockaddr_in6(port: u16, last: u8) -> [u8; 0x1c] {
    let mut a = [0u8; 0x1c];
    a[0..2].copy_from_slice(&0x17u16.to_le_bytes());
    a[2..4].copy_from_slice(&port.to_be_bytes());
    a[0x17] = last;
    a
}

fn sockaddr_in(port: u16, ip: [u8; 4]) -> [u8; 0x10] {
    let mut a = [0u8; 0x10];
    a[0..2].copy_from_slice(&2u16.to_le_bytes());
    a[2..4].copy_from_slice(&port.to_be_bytes());
    a[4..8].copy_from_slice(&ip);
    a
}

unsafe fn flags_of(h: *mut u8) -> u32 {
    *at::<u32>(h, OFF_FLAGS)
}

unsafe fn link_send(h: *mut u8, data: &[u8]) -> (HRESULT, u32) {
    let b = WSABUF { len: data.len() as u32, buf: data.as_ptr() as *mut u8 };
    let mut sent = 0u32;
    let hr = XbtpLinkSend(h, &b, 1, &mut sent, 0, null_mut());
    (hr, sent)
}

unsafe fn link_recv(h: *mut u8, len: usize) -> (HRESULT, Vec<u8>) {
    let mut buf = vec![0u8; len];
    let mut got = 0u32;
    let hr = XbtpLinkRecv(h, buf.as_mut_ptr(), len as u32, &mut got, 0, null_mut());
    buf.truncate(got as usize);
    (hr, buf)
}

unsafe fn listen_and_connect(service: u16) -> (*mut u8, *mut u8, *mut u8) {
    let l = XbtpLinkCreateHandle();
    assert!(!l.is_null());
    let mut ready: HANDLE = null_mut();
    assert_eq!(XbtpLinkListen(l, null_mut(), service, &mut ready), S_OK);
    assert!(!ready.is_null());
    assert_eq!(flags_of(l) & (FLAG_BOUND | FLAG_LISTENING | MODE_MASK), FLAG_BOUND | FLAG_LISTENING | MODE_DEVICE);
    let ls = Send2(l);
    let acc = std::thread::spawn(move || {
        let ls = ls;
        let mut out: *mut u8 = null_mut();
        let hr = XbtpLinkAccept(ls.0, &mut out, 5000);
        (hr, Send2(out))
    });
    let c = XbtpLinkCreateHandle();
    let addr = sockaddr_in6(service, 1);
    assert_eq!(XbtpLinkConnect(c, addr.as_ptr(), addr.len() as u32, 3000), S_OK);
    let (hr, a) = acc.join().unwrap();
    assert_eq!(hr, S_OK);
    assert!(!a.0.is_null());
    (l, c, a.0)
}

#[test]
fn initialize_system_is_idempotent() {
    unsafe {
        assert_eq!(XbtpLinkInitializeSystem(), S_OK);
        assert_eq!(XbtpLinkInitializeSystem(), S_OK);
        assert_eq!(INIT_STATE.load(Ordering::SeqCst), 2);
        assert_eq!(WSA_STARTED.load(Ordering::SeqCst), 1);
        assert_eq!(EPOCH_PENDING.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn create_handle_layout_and_close() {
    let _e = env(&REAL_DEVICE);
    unsafe {
        let h = XbtpLinkCreateHandle();
        assert!(!h.is_null());
        assert_eq!(*at::<usize>(h, OFF_SOCKET), INVALID_SOCKET);
        assert_eq!(*at::<u32>(h, OFF_REFS), 1);
        assert!(!(*at::<HANDLE>(h, OFF_IDLE_EVENT)).is_null());
        assert_eq!(*at::<*mut u8>(h, 0x3a0), h.add(0x2c8));
        assert_eq!(*at::<*mut u8>(h, 0x490), h.add(0x3b8));
        assert_eq!(*at::<*mut u8>(h, 0x2d8), h);
        assert_eq!(*at::<*mut u8>(h, 0x3c8), h);
        assert_eq!(flags_of(h), 0);
        XbtpLinkCloseHandle(h);
        XbtpLinkCloseHandle(null_mut());
    }
}

#[test]
fn listen_without_device_reports_no_device() {
    let _e = env(&REAL_DEVICE);
    unsafe {
        assert_eq!(XbtpLinkListen(null_mut(), null_mut(), 1, null_mut()), LINK_E_NULL_HANDLE);
        let h = XbtpLinkCreateHandle();
        let mut ready: HANDLE = 1 as HANDLE;
        assert_eq!(XbtpLinkListen(h, null_mut(), 7, &mut ready), LINK_E_NO_DEVICE);
        assert!(ready.is_null());
        assert!((*at::<HANDLE>(h, OFF_DEVICE)).is_null());
        let mut out: *mut u8 = 1 as *mut u8;
        assert_eq!(XbtpLinkAccept(h, &mut out, 0), LINK_E_NO_DEVICE);
        assert!(out.is_null());
        assert_eq!(XbtpLinkAccept(null_mut(), &mut out, 0), LINK_E_NULL_HANDLE);
        let a = sockaddr_in6(9, 1);
        assert_eq!(XbtpLinkConnect(h, a.as_ptr(), 0x1c, 0), LINK_E_NO_DEVICE);
        XbtpLinkCloseHandle(h);
    }
}

#[test]
fn listen_accept_connect_send_recv_over_device() {
    let _e = env(&FAKE_DEVICE);
    unsafe {
        let (l, c, a) = listen_and_connect(77);
        assert_eq!(flags_of(c) & FLAG_CONNECTED, FLAG_CONNECTED);
        assert_eq!(*at::<u32>(c, OFF_TARGET), 77);
        assert_eq!(*at::<u32>(c, OFF_REMOTE), *at::<u32>(a, OFF_LOCAL));
        assert_eq!(*at::<u32>(a, OFF_REMOTE), *at::<u32>(c, OFF_LOCAL));
        let mut info = [0u8; 16];
        assert_eq!(XbtpLinkQueryHandleInformation(c, 1, 16, info.as_mut_ptr()), S_OK);
        assert_eq!(info[15], 1);
        let (hr, sent) = link_send(c, b"hello link");
        assert_eq!(hr, S_OK);
        assert_eq!(sent, 0x80 + 10);
        let (hr, got) = link_recv(a, 64);
        assert_eq!(hr, S_OK);
        assert_eq!(got, b"hello link");
        let rs = Send2(c);
        let rd = std::thread::spawn(move || {
            let rs = rs;
            link_recv(rs.0, 64)
        });
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(link_send(a, b"back").0, S_OK);
        let (hr, got) = rd.join().unwrap();
        assert_eq!(hr, S_OK);
        assert_eq!(got, b"back");
        assert_eq!(XbtpLinkShutdownHandle(a), S_OK);
        assert_eq!(flags_of(a) & FLAG_SHUTDOWN, FLAG_SHUTDOWN);
        let (hr, got) = link_recv(c, 8);
        assert_eq!(hr, S_OK);
        assert!(got.is_empty());
        assert_eq!(XbtpLinkShutdownHandle(a), S_OK);
        XbtpLinkCloseHandle(a);
        XbtpLinkCloseHandle(c);
        XbtpLinkCloseHandle(l);
        assert!(fake().eps.is_empty());
    }
}

#[test]
fn connect_by_name_uses_numeric_ipv4() {
    let _e = env(&FAKE_DEVICE);
    unsafe {
        let l = XbtpLinkCreateHandle();
        assert_eq!(XbtpLinkListen(l, null_mut(), 91, null_mut()), S_OK);
        let ls = Send2(l);
        let acc = std::thread::spawn(move || {
            let ls = ls;
            let mut out: *mut u8 = null_mut();
            let hr = XbtpLinkAccept(ls.0, &mut out, 5000);
            (hr, Send2(out))
        });
        let c = XbtpLinkCreateHandle();
        let name = wide("127.0.0.1");
        assert_eq!(XbtpLinkConnectByName(c, name.as_ptr(), 91, 3000), S_OK);
        let (hr, a) = acc.join().unwrap();
        assert_eq!(hr, S_OK);
        assert_eq!(*at::<u32>(c, OFF_TARGET), 91);
        assert_eq!(*at::<u8>(c, OFF_TARGET + 0x13), 1);
        assert_eq!(*at::<u32>(c, OFF_TARGET + 0x10), 0x0100_0000);
        XbtpLinkCloseHandle(a.0);
        XbtpLinkCloseHandle(c);
        XbtpLinkCloseHandle(l);
        let c = XbtpLinkCreateHandle();
        assert_eq!(XbtpLinkConnectByName(c, null(), 1, 0) as u32, 0x8007_2af9);
        XbtpLinkCloseHandle(c);
    }
}

#[test]
fn connect_refused_and_bad_address() {
    let _e = env(&FAKE_DEVICE);
    unsafe {
        assert_eq!(XbtpLinkConnect(null_mut(), null(), 0, 0), LINK_E_NULL_HANDLE);
        let c = XbtpLinkCreateHandle();
        let mut bad = sockaddr_in6(5, 1);
        bad[0] = 0x63;
        assert_eq!(XbtpLinkConnect(c, bad.as_ptr(), 0x1c, 0), LINK_E_CONNECT_ADDRESS);
        let short = sockaddr_in6(5, 1);
        assert_eq!(XbtpLinkConnect(c, short.as_ptr(), 0x10, 0), LINK_E_CONNECT_ADDRESS);
        let a = sockaddr_in6(4444, 1);
        assert_eq!(XbtpLinkConnect(c, a.as_ptr(), 0x1c, 0), LINK_E_CONNECT_REFUSED);
        assert_eq!(flags_of(c), 0);
        XbtpLinkCloseHandle(c);
        assert!(fake().eps.is_empty());
    }
}

#[test]
fn send_rejects_oversized_and_null() {
    let _e = env(&FAKE_DEVICE);
    unsafe {
        let mut sent = 0u32;
        assert_eq!(XbtpLinkSend(null_mut(), null(), 0, &mut sent, 0, null_mut()), LINK_E_NULL_HANDLE);
        let h = XbtpLinkCreateHandle();
        let big = [WSABUF { len: 0x1000_0001, buf: null_mut() }];
        assert_eq!(XbtpLinkSend(h, big.as_ptr(), 1, &mut sent, 0, null_mut()), LINK_E_TOO_LARGE);
        let many = [WSABUF { len: 0x0100_0000, buf: null_mut() }, WSABUF { len: 1, buf: null_mut() }];
        assert_eq!(XbtpLinkSend(h, many.as_ptr(), 2, &mut sent, 0, null_mut()), LINK_E_TOO_LARGE);
        assert_eq!(XbtpLinkSend(h, null(), 0, &mut sent, 0, null_mut()), LINK_E_BAD_MODE);
        XbtpLinkCloseHandle(h);
    }
}

#[test]
fn recv_modes_and_null_handle() {
    let _e = env(&FAKE_DEVICE);
    unsafe {
        let mut got = 0u32;
        let mut b = [0u8; 4];
        assert_eq!(XbtpLinkRecv(null_mut(), b.as_mut_ptr(), 4, &mut got, 0, null_mut()), LINK_E_NULL_HANDLE);
        let h = XbtpLinkCreateHandle();
        assert_eq!(XbtpLinkRecv(h, b.as_mut_ptr(), 4, &mut got, 0, null_mut()), LINK_E_BAD_MODE);
        XbtpLinkCloseHandle(h);
        let (l, c, a) = listen_and_connect(78);
        let mut ov = OVERLAPPED::default();
        assert_eq!(XbtpLinkRecv(a, b.as_mut_ptr(), 4, &mut got, 0, &mut ov), E_PENDING);
        let mut n = 9u32;
        assert_eq!(XbtpLinkGetOverlappedResult(a, &ov, &mut n), E_PENDING);
        assert_eq!(n, 0);
        assert_eq!(link_send(c, b"abcd").0, S_OK);
        assert_eq!(XbtpLinkGetOverlappedResult(a, &ov, &mut n), S_OK);
        assert_eq!(n, 4);
        assert_eq!(&b, b"abcd");
        XbtpLinkCloseHandle(a);
        XbtpLinkCloseHandle(c);
        XbtpLinkCloseHandle(l);
    }
}

#[test]
fn get_overlapped_result_maps_status() {
    unsafe {
        let mut n = 5u32;
        let mut ov = OVERLAPPED { internal: 0, internal_high: 33, ..OVERLAPPED::default() };
        assert_eq!(XbtpLinkGetOverlappedResult(null_mut(), &ov, &mut n), S_OK);
        assert_eq!(n, 33);
        ov.internal = 0x103;
        assert_eq!(XbtpLinkGetOverlappedResult(null_mut(), &ov, &mut n), E_PENDING);
        assert_eq!(n, 0);
        let want: [(usize, u32); 10] = [
            (1, 0x8c11_051a),
            (2, 0x8c11_0514),
            (3, 0x8007_000e),
            (4, 0x8c11_0516),
            (5, 0x8c11_0517),
            (6, 0x8c11_0518),
            (7, 0x8c11_0519),
            (8, 0x8c11_051b),
            (9, 0x8c11_051c),
            (0, 0x8c11_051c),
        ];
        for (k, w) in want {
            ov.internal = 0xe078_0000 | k;
            assert_eq!(XbtpLinkGetOverlappedResult(null_mut(), &ov, &mut n) as u32, w);
        }
        ov.internal = 0xc000_0120;
        assert_eq!(XbtpLinkGetOverlappedResult(null_mut(), &ov, &mut n) as u32, 0x8c11_051c);
    }
}

#[test]
fn handle_control_toggles_tcp_path() {
    let _e = env(&REAL_DEVICE);
    unsafe {
        let h = XbtpLinkCreateHandle();
        let one = 1u32;
        let zero = 0u32;
        let two = 2u32;
        let mut out = 7u32;
        let mut ret = 0u32;
        assert_eq!(XbtpLinkHandleControl(h, 1, 3, &one, 0, null_mut(), null_mut()), E_INVALIDARG);
        assert_eq!(XbtpLinkHandleControl(h, 1, 4, null(), 0, null_mut(), null_mut()), E_INVALIDARG);
        assert_eq!(XbtpLinkHandleControl(null_mut(), 1, 4, &one, 0, null_mut(), null_mut()), E_FAIL);
        assert_eq!(XbtpLinkHandleControl(h, 1, 4, &two, 0, null_mut(), null_mut()), E_INVALIDARG);
        assert_eq!(XbtpLinkHandleControl(h, 1, 4, &one, 0, null_mut(), null_mut()), S_OK);
        assert_eq!(XbtpLinkHandleControl(h, 2, 0, null(), 4, &mut ret, &mut out), S_OK);
        assert_eq!((out, ret), (1, 4));
        assert_eq!(XbtpLinkHandleControl(h, 1, 4, &zero, 0, null_mut(), null_mut()), S_OK);
        assert_eq!(XbtpLinkHandleControl(h, 2, 0, null(), 4, &mut ret, &mut out), S_OK);
        assert_eq!(out, 0);
        assert_eq!(XbtpLinkHandleControl(null_mut(), 2, 0, null(), 4, &mut ret, &mut out), S_OK);
        assert_eq!(out, 0);
        assert_eq!(XbtpLinkHandleControl(h, 2, 0, null(), 3, &mut ret, &mut out), E_INVALIDARG);
        assert_eq!(XbtpLinkHandleControl(h, 3, 0, null(), 0, null_mut(), null_mut()), E_NOTIMPL);
        *at::<u32>(h, OFF_FLAGS) |= FLAG_CONNECTED;
        assert_eq!(XbtpLinkHandleControl(h, 1, 4, &one, 0, null_mut(), null_mut()), E_INVALIDARG);
        *at::<u32>(h, OFF_FLAGS) = 0;
        XbtpLinkCloseHandle(h);
    }
}

#[test]
fn query_handle_information_classes() {
    let _e = env(&REAL_DEVICE);
    unsafe {
        let h = XbtpLinkCreateHandle();
        for i in 0..16 {
            *at::<u8>(h, 0x88 + i) = i as u8 + 1;
        }
        *at::<u32>(h, OFF_PATH_TCP) = 3;
        *at::<u32>(h, OFF_PATH_XTD) = 4;
        let mut b = [0u8; 16];
        assert_eq!(XbtpLinkQueryHandleInformation(h, 1, 15, b.as_mut_ptr()), E_INVALIDARG);
        assert_eq!(XbtpLinkQueryHandleInformation(h, 1, 16, b.as_mut_ptr()), S_OK);
        assert_eq!(b[0], 1);
        assert_eq!(b[15], 16);
        let mut p = [0u32; 2];
        assert_eq!(XbtpLinkQueryHandleInformation(h, 2, 4, p.as_mut_ptr() as *mut u8), E_INVALIDARG);
        assert_eq!(XbtpLinkQueryHandleInformation(h, 2, 8, p.as_mut_ptr() as *mut u8), S_OK);
        assert_eq!(p, [3, 4]);
        assert_eq!(XbtpLinkQueryHandleInformation(h, 9, 0, null_mut()), S_OK);
        XbtpLinkCloseHandle(h);
    }
}

#[test]
fn set_completion_port_binds_device_handle() {
    let _e = env(&REAL_DEVICE);
    unsafe {
        assert_eq!(XbtpLinkSetCompletionPort(null_mut(), null_mut(), 0, 0), LINK_E_NULL_HANDLE);
        let h = XbtpLinkCreateHandle();
        let port = CreateIoCompletionPort(INVALID_HANDLE_VALUE, null_mut(), 0, 1);
        assert!(!port.is_null());
        let hr = XbtpLinkSetCompletionPort(h, port, 1, 1);
        assert!(hr < 0 && (hr as u32 & 0xffff_0000) == 0x8007_0000);
        let path = std::env::temp_dir().join(format!("xbtp_link_iocp_{}.bin", std::process::id()));
        let w = wide(path.to_str().unwrap());
        let f = CreateFileW(w.as_ptr(), 0xc000_0000, 0, null_mut(), 2, 0x4000_0000 | 0x0400_0000, null_mut());
        assert_ne!(f, INVALID_HANDLE_VALUE);
        *at::<HANDLE>(h, OFF_DEVICE) = f;
        assert_eq!(XbtpLinkSetCompletionPort(h, port, 1, 1), S_OK);
        *at::<HANDLE>(h, OFF_DEVICE) = null_mut();
        CloseHandle(f);
        CloseHandle(port);
        XbtpLinkCloseHandle(h);
    }
}

#[test]
fn service_queries_match_console() {
    unsafe {
        let mut buf = [0x41u16; 4];
        let mut len = 4u32;
        assert_eq!(XbtpLinkGetServiceLocation(buf.as_mut_ptr(), &mut len), E_NOTIMPL);
        assert_eq!((buf[0], len), (0, 0));
        let mut z = 0u32;
        assert_eq!(XbtpLinkGetServiceLocation(null_mut(), &mut z), E_NOTIMPL);
        let mut o = 9u32;
        assert_eq!(XbtpLinkGetServiceOptions(&mut o), S_OK);
        assert_eq!(o, 0);
        assert_eq!(XbtpLinkGetServiceVersion(), E_NOTIMPL);
        assert_eq!(XbtpLinkSetServiceOptions(), E_NOTIMPL);
        assert_eq!(XbtpLinkQueryStatistics(), E_NOTIMPL);
        assert_eq!(XbtpLinkQueryPacketLog(), E_NOTIMPL);
        assert_eq!(XbtpLinkSetupRoutingLog(), E_NOTIMPL);
        assert_eq!(XbtpLinkStartRouting(), S_OK);
        XbtpLinkStopRouting();
    }
}

#[test]
fn reserved_memory_lifecycle() {
    let _e = env(&FAKE_DEVICE);
    unsafe {
        let mut a: *mut c_void = 1 as *mut c_void;
        let mut s = 1u64;
        assert_eq!(XbtpLinkFreeReservedMemory(), 0);
        assert_eq!(XbtpLinkAllocReservedMemory(&mut a, &mut s), S_OK);
        assert_eq!(a as usize, 0x7ff0_0000_0000);
        assert_eq!(s, 0x20_0000);
        assert_eq!(XbtpLinkAllocReservedMemory(&mut a, &mut s), HR_SHARING_VIOLATION);
        assert!(a.is_null());
        assert_eq!(s, 0);
        assert_eq!(XbtpLinkFreeReservedMemory(), 3);
        assert_eq!(XbtpLinkFreeReservedMemory(), 0);
        assert_eq!(XbtpLinkAllocReservedMemory(&mut a, &mut s), S_OK);
        assert_eq!(XbtpLinkFreeReservedMemory(), 3);
    }
    drop(_e);
    let _e = env(&REAL_DEVICE);
    unsafe {
        let mut a: *mut c_void = null_mut();
        let mut s = 0u64;
        assert_eq!(XbtpLinkAllocReservedMemory(&mut a, &mut s), HR_NOT_SUPPORTED);
        assert_eq!(RESERVE_STATE.load(Ordering::SeqCst), 0);
        assert_eq!(XbtpLinkFreeReservedMemory(), 0);
    }
}

#[test]
fn routing_redirect_resolves_addresses() {
    let _e = env(&REAL_DEVICE);
    unsafe {
        let a = sockaddr_in(99, [10, 1, 2, 3]);
        assert_eq!(XbtpLinkSetRoutingRedirect(a.as_ptr(), 16), S_OK);
        let r = routing_redirect();
        assert_eq!(u32::from_le_bytes([r[0], r[1], r[2], r[3]]), 99);
        assert_eq!(&r[4..0x18], &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 10, 1, 2, 3, 0, 0, 0, 0]);
        assert_eq!(format_link_address(r.as_ptr()), "10.1.2.3");
        let lo = sockaddr_in(7, [127, 0, 0, 1]);
        assert_eq!(XbtpLinkSetRoutingRedirect(lo.as_ptr(), 16), S_OK);
        let r = routing_redirect();
        assert_eq!(r[0x13], 1);
        assert_eq!(&r[4..0x13], &[0u8; 15]);
        assert_eq!(format_link_address(r.as_ptr()), "[::1]");
        let mut v6 = sockaddr_in6(8, 0);
        v6[8] = 0xfe;
        v6[9] = 0x80;
        assert_eq!(XbtpLinkSetRoutingRedirect(v6.as_ptr(), 0x1c), S_OK);
        assert_eq!(format_link_address(routing_redirect().as_ptr()), "[fe80:0:0:0:0:0:0:0]");
        let mut out = [0xaau8; 0x18];
        assert_eq!(sockaddr_to_link(v6.as_ptr(), 0x10, out.as_mut_ptr()), 0xc000_000du32 as i32);
        assert_eq!(out, [0xaa; 0x18]);
        assert_eq!(XbtpLinkSetRoutingRedirect(v6.as_ptr(), 0x10), S_OK);
    }
}

#[test]
fn process_routing_socket_replies_with_redirect() {
    let _e = env(&REAL_DEVICE);
    unsafe {
        assert_eq!(XbtpLinkInitializeSystem(), S_OK);
        let a = sockaddr_in(1234, [192, 168, 5, 6]);
        XbtpLinkSetRoutingRedirect(a.as_ptr(), 16);
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut cl = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        cl.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (sv, _) = l.accept().unwrap();
        use std::os::windows::io::IntoRawSocket;
        let raw = sv.into_raw_socket() as SOCKET;
        assert_eq!(XbtpLinkProcessRoutingSocket(raw), S_OK);
        let mut req = [0u8; 0x80];
        req[0] = 0x81;
        req[0x40] = 0x55;
        cl.write_all(&req).unwrap();
        let mut rep = [0u8; 0x80];
        cl.read_exact(&mut rep).unwrap();
        assert_eq!(u32::from_le_bytes([rep[0], rep[1], rep[2], rep[3]]), MSG_GATEWAY);
        assert_eq!(&rep[4..0x10], &[0u8; 12]);
        assert_eq!(u16::from_le_bytes([rep[0x38], rep[0x39]]), 0x68);
        assert_eq!(&rep[0x3a..0x4a], &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 192, 168, 5, 6]);
        assert_eq!(rep[0x4a], 0);
        cl.write_all(&req).unwrap();
        let mut tail = [0u8; 1];
        assert_eq!(cl.read(&mut tail).unwrap_or(0), 0);
    }
}

fn router_session(s: &mut TcpStream, service: u16, redirect: Option<[u8; 16]>) {
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut pre = [0u8; 8];
    s.read_exact(&mut pre).unwrap();
    assert_eq!(pre, [0xa8, 0xaf, 0, 0, 0, 0, 0, 0]);
    let mut m = [0u8; 0x80];
    s.read_exact(&mut m).unwrap();
    assert_eq!(u32::from_le_bytes([m[0], m[1], m[2], m[3]]), MSG_TCP_CONNECT);
    assert_eq!(u32::from_le_bytes([m[0x24], m[0x25], m[0x26], m[0x27]]), service as u32);
    let mut rep = [0u8; 0x80];
    if let Some(r) = redirect {
        rep[0x38..0x3a].copy_from_slice(&0x68u16.to_le_bytes());
        rep[0x3a..0x4a].copy_from_slice(&r);
    }
    s.write_all(&rep).unwrap();
}

#[test]
fn tcp_path_connects_through_router_and_moves_data() {
    let _e = env(&REAL_DEVICE);
    unsafe {
        let l = TcpListener::bind("127.0.0.1:4201").expect("router port 4201");
        let srv = std::thread::spawn(move || {
            let (mut s1, _) = l.accept().unwrap();
            let mut v4 = [0u8; 16];
            v4[10] = 0xff;
            v4[11] = 0xff;
            v4[12] = 127;
            v4[15] = 1;
            router_session(&mut s1, 5555, Some(v4));
            let (mut s, _) = l.accept().unwrap();
            router_session(&mut s, 5555, None);
            let mut b = [0u8; 5];
            s.read_exact(&mut b).unwrap();
            assert_eq!(&b, b"hello");
            s.write_all(b"world!").unwrap();
            drop(s1);
            s
        });
        let h = XbtpLinkCreateHandle();
        let one = 1u32;
        assert_eq!(XbtpLinkHandleControl(h, 1, 4, &one, 0, null_mut(), null_mut()), S_OK);
        let short = sockaddr_in(5555, [127, 0, 0, 1]);
        assert_eq!(tcp_connect(h, short.as_ptr(), 8, 100), E_INVALIDARG);
        let a = sockaddr_in(5555, [127, 0, 0, 1]);
        assert_eq!(XbtpLinkConnect(h, a.as_ptr(), 16, 3000), S_OK);
        assert_eq!(flags_of(h) & MODE_MASK, MODE_TCP);
        assert_ne!(*at::<usize>(h, OFF_SOCKET), INVALID_SOCKET);
        assert_eq!(tcp_connect(h, a.as_ptr(), 16, 100), E_INVALIDARG);
        let (hr, sent) = link_send(h, b"hello");
        assert_eq!(hr, S_OK);
        assert_eq!(sent, 5);
        let mut got = 0u32;
        let mut b = [0u8; 4];
        assert_eq!(XbtpLinkRecv(h, b.as_mut_ptr(), 4, &mut got, 1, null_mut()), E_INVALIDARG);
        let mut all = Vec::new();
        while all.len() < 6 {
            let (hr, part) = link_recv(h, 16);
            assert_eq!(hr, S_OK);
            assert!(!part.is_empty());
            all.extend_from_slice(&part);
        }
        assert_eq!(all, b"world!");
        let s = srv.join().unwrap();
        let sock = *at::<usize>(h, OFF_SOCKET);
        XbtpLinkCloseHandle(h);
        closesocket(sock);
        drop(s);
    }
}

unsafe extern "C" fn collect(l: *const u8, ctx: *mut c_void) -> i32 {
    let v = &mut *(ctx as *mut Vec<String>);
    v.push(CStr::from_ptr(l as *const core::ffi::c_char).to_string_lossy().into_owned());
    1
}

unsafe extern "C" fn stop_after_one(_l: *const u8, ctx: *mut c_void) -> i32 {
    let n = &mut *(ctx as *mut u32);
    *n += 1;
    0
}

fn decode_packet(rec: &[u8]) -> (HRESULT, Vec<String>) {
    let mut v: Vec<String> = Vec::new();
    let hr = unsafe { XbtpLinkDecodePacketRecord(rec.as_ptr(), rec.len() as u32, collect, &mut v as *mut Vec<String> as *mut c_void) };
    (hr, v)
}

fn put_opt(rec: &mut [u8], off: &mut usize, ty: u16, data: &[u8]) {
    let w = (ty << 5) | ((data.len() / 2) as u16);
    let base = 0x20 + 0x38 + *off;
    rec[base..base + 2].copy_from_slice(&w.to_le_bytes());
    rec[base + 2..base + 2 + data.len()].copy_from_slice(data);
    *off += 2 + data.len();
}

#[test]
fn decode_packet_record_formats_lines() {
    let mut rec = vec![0u8; 0xa0];
    assert_eq!(unsafe { XbtpLinkDecodePacketRecord(rec.as_ptr(), 0x9f, collect, null_mut()) }, E_INVALIDARG);
    rec[0] = 2;
    rec[4..6].copy_from_slice(&0xabcu16.to_le_bytes());
    rec[0x10..0x18].copy_from_slice(&42i64.to_le_bytes());
    rec[0x20] = 1;
    rec[0x21] = 0x7;
    rec[0x24..0x28].copy_from_slice(&5i32.to_le_bytes());
    rec[0x28..0x30].copy_from_slice(&9i64.to_le_bytes());
    rec[0x30..0x34].copy_from_slice(&0x77u32.to_le_bytes());
    rec[0x43] = 1;
    rec[0x44..0x48].copy_from_slice(&0x9001u32.to_le_bytes());
    rec[0x48] = 0x20;
    rec[0x49] = 0x01;
    let mut off = 0;
    put_opt(&mut rec, &mut off, 9, &0x1234u32.to_le_bytes());
    put_opt(&mut rec, &mut off, 0xc, &[1, 2]);
    put_opt(&mut rec, &mut off, 2, &[0x41, 0, 0x42, 0]);
    put_opt(&mut rec, &mut off, 0x1f, &[0, 0]);
    put_opt(&mut rec, &mut off, 0xf, &3u16.to_le_bytes());
    put_opt(&mut rec, &mut off, 0xa, &(-4i64).to_le_bytes());
    let (hr, v) = decode_packet(&rec);
    assert_eq!(hr, S_OK);
    assert_eq!(v[0], "0ABC:0000000000000042:P:O:DATA:07 0:0:0:0:0:0:0:1:77 - 2001:0:0:0:0:0:0:0:9001 9[5]");
    assert_eq!(v[1], "0ABC:0000000000000042:O:PROT_VER 00001234");
    assert_eq!(v[2], "0ABC:0000000000000042:O:PATH_INFO TCP:1 XTD:2");
    assert_eq!(v[3], "0ABC:0000000000000042:O:XTD_REQ AB");
    assert_eq!(v[4], v[3]);
    assert_eq!(v[5], "0ABC:0000000000000042:O:XTD_PENDING_RECV: 3");
    assert_eq!(v[6], "0ABC:0000000000000042:O:LINK_INDEX -4");
    assert_eq!(v.len(), 7);
    let mut r2 = vec![0u8; 0xa0];
    r2[0] = 1;
    r2[0x20] = 3;
    let mut off = 0;
    let host: Vec<u8> = (0u8..16).collect();
    put_opt(&mut r2, &mut off, 1, &host);
    let red = [0x20u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
    put_opt(&mut r2, &mut off, 3, &red);
    let (_, v) = decode_packet(&r2);
    assert!(v[0].starts_with("0000:0000000000000000:P:I:CRES:00 "));
    assert_eq!(v[1], "0000:0000000000000000:O:HOST_ID 000102030405060708090A0B0C0D0E0F");
    assert_eq!(v[2], "0000:0000000000000000:O:REDIRECT 2001:0:0:0:0:0:0:2");
    let mut r3 = vec![0u8; 0xa0];
    r3[0x20] = 0x80;
    r3[0x58] = 1;
    let (_, v) = decode_packet(&r3);
    assert_eq!(v.len(), 1);
    assert!(v[0].starts_with("0000:0000000000000000:P:????:INT1:00 "));
    r3[0x58] = 5;
    let (_, v) = decode_packet(&r3);
    assert!(v[0].starts_with("0000:0000000000000000:P:????:INT?:05 "));
    r3[0x20] = 0x99;
    let (_, v) = decode_packet(&r3);
    assert!(v[0].contains(":0x99:00 "));
    let mut n = 0u32;
    assert_eq!(unsafe { XbtpLinkDecodePacketRecord(rec.as_ptr(), 0xa0, stop_after_one, &mut n as *mut u32 as *mut c_void) }, E_ABORT);
    assert_eq!(n, 1);
}

#[test]
fn decode_packet_record_text_kinds() {
    let mut rec = vec![0u8; 0xa0];
    rec[3] = 0x20;
    rec[0x20..0x25].copy_from_slice(b"raw!\0");
    let (hr, v) = decode_packet(&rec);
    assert_eq!(hr, S_OK);
    assert_eq!(v, vec!["raw!".to_string()]);
    rec[0x20..0xa0].fill(b'x');
    let (hr, v) = decode_packet(&rec);
    assert_eq!(hr, S_OK);
    assert!(v.is_empty());
    let mut rec = vec![0u8; 0xa0];
    rec[3] = 0x10;
    rec[4] = 0x12;
    rec[0x20..0x26].copy_from_slice(b"log me");
    let (hr, v) = decode_packet(&rec);
    assert_eq!(hr, S_OK);
    assert_eq!(v, vec!["0012:0000000000000000:L:log me".to_string()]);
    let mut n = 0u32;
    assert_eq!(unsafe { XbtpLinkDecodePacketRecord(rec.as_ptr(), 0xa0, stop_after_one, &mut n as *mut u32 as *mut c_void) }, E_ABORT);
}

fn decode_stat(rec: &[u8]) -> (HRESULT, Vec<String>) {
    let mut v: Vec<String> = Vec::new();
    let hr = unsafe { XbtpLinkDecodeStatisticRecord(rec.as_ptr(), rec.len() as u32, collect, &mut v as *mut Vec<String> as *mut c_void) };
    (hr, v)
}

#[test]
fn decode_statistic_record_formats_lines() {
    let mut rec = vec![0u8; 0x98];
    assert_eq!(unsafe { XbtpLinkDecodeStatisticRecord(rec.as_ptr(), 0x97, collect, null_mut()) }, E_INVALIDARG);
    rec[8..0x10].copy_from_slice(&(-1i64).to_le_bytes());
    assert_eq!(decode_stat(&rec).1, vec!["Default Gateway".to_string()]);
    rec[8..0x10].fill(0);
    rec[0x38..0x40].copy_from_slice(&100i64.to_le_bytes());
    rec[0x40..0x48].copy_from_slice(&2_500_000u64.to_le_bytes());
    rec[0x18..0x20].copy_from_slice(&7i64.to_le_bytes());
    rec[0x48..0x50].copy_from_slice(&1i64.to_le_bytes());
    let (_, v) = decode_stat(&rec);
    assert_eq!(v[0], "Gateway");
    assert_eq!(v[1], " TCP: Recv: 100 bytes (2.500 MB/s) Send: 7 bytes (0.000 MB/s)");
    assert_eq!(v[2], " XTD: Recv: 1 bytes (0.000 MB/s) Send: 0 bytes (0.000 MB/s)");
    let mut r = vec![0u8; 0x98];
    r[4] = 1;
    r[0x68 + 0xe] = 0xff;
    r[0x68 + 0xf] = 0xff;
    r[0x68 + 0x10..0x68 + 0x14].copy_from_slice(&[1, 2, 3, 4]);
    assert_eq!(decode_stat(&r).1, vec![" Address 1.2.3.4".to_string()]);
    r[4] = 3;
    r[0x68..0x6c].copy_from_slice(&88u32.to_le_bytes());
    assert_eq!(decode_stat(&r).1, vec![" Listening [::1]:88".to_string()]);
    r[4] = 4;
    r[0x7c..0x80].copy_from_slice(&0xabu32.to_le_bytes());
    r[0x7c + 0x13] = 1;
    r[0x90..0x94].copy_from_slice(&12u32.to_le_bytes());
    r[0x58..0x60].copy_from_slice(&1234i64.to_le_bytes());
    r[0x60..0x68].copy_from_slice(&5i64.to_le_bytes());
    let (_, v) = decode_stat(&r);
    assert_eq!(v[0], " Port [::1]:12-00000058 [::1]:0-AB IN ACTIVE");
    assert_eq!(v[1], "  Recv: 0/0 Send: 0/0 Stall: 1.234 Lifetime: 0.005");
    r[4] = 5;
    r[6] = 2;
    let (_, v) = decode_stat(&r);
    assert_eq!(v[0], " Port [::1]:0-00000058 [::1]:12-AB OUT CLOSED");
    r[4] = 2;
    assert!(decode_stat(&r).1.is_empty());
    r[4] = 9;
    assert!(decode_stat(&r).1.is_empty());
    r[4] = 1;
    let mut n = 0u32;
    assert_eq!(unsafe { XbtpLinkDecodeStatisticRecord(r.as_ptr(), 0x98, stop_after_one, &mut n as *mut u32 as *mut c_void) }, E_ABORT);
}

#[test]
fn option_parser_follows_console_bounds() {
    unsafe {
        let mut m = [0u8; 0x80];
        let mut off = 0usize;
        let mut put = |ty: u16, n: usize| {
            let w = (ty << 5) | (n as u16 / 2);
            m[0x38 + off..0x3a + off].copy_from_slice(&w.to_le_bytes());
            off += 2 + n;
        };
        put(3, 16);
        put(0xc, 2);
        put(9, 0x3e);
        let mut out = [(null::<u8>(), 0u16, 0u16); 8];
        let n = parse_options(m.as_ptr(), &mut out);
        assert_eq!(n, 2);
        assert_eq!((out[0].1, out[0].2), (3, 16));
        assert_eq!(out[0].0, m.as_ptr().add(0x3a));
        assert_eq!((out[1].1, out[1].2), (0xc, 2));
        assert_eq!((out[2].1, out[2].2), (9, 0x3e));
        assert!(out[2].0.is_null());
    }
}
