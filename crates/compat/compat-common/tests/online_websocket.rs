#[cfg(not(tombox_backend_data))]
#[test]
#[ignore = "needs crates/compat/compat-common/backend_data"]
fn online_websocket_needs_backend_data() {}

#[cfg(tombox_backend_data)]
mod backend {
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn port() -> u16 {
    std::env::set_var("TOMBOX_NET_BACKEND", "builtin");
    std::env::set_var("TOMBOX_NET_TITLE", "fortnite");
    std::env::set_var("TOMBOX_ONLINE_ACCOUNT", "XboxEmulator");
    compat_common::online::start_for_config().expect("server")
}

fn send_text(s: &mut TcpStream, text: &str) {
    let p = text.as_bytes();
    let mask = [0x11u8, 0x22, 0x33, 0x44];
    let mut f = vec![0x81u8];
    if p.len() < 126 {
        f.push(0x80 | p.len() as u8);
    } else {
        f.push(0x80 | 126);
        f.extend_from_slice(&(p.len() as u16).to_be_bytes());
    }
    f.extend_from_slice(&mask);
    f.extend(p.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    s.write_all(&f).unwrap();
}

fn recv(r: &mut BufReader<TcpStream>) -> (u8, String) {
    let (_, op, data) = compat_common::online::ws::read_frame(r).unwrap();
    (op, String::from_utf8_lossy(&data).into_owned())
}

#[test]
fn xmpp_over_websocket_end_to_end() {
    let port = port();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(s, "GET / HTTP/1.1\r\nHost: xmpp-service-prod.ol.epicgames.com\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: xmpp\r\n\r\n").unwrap();
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut head = Vec::new();
    loop {
        let mut l = String::new();
        r.read_line(&mut l).unwrap();
        if l == "\r\n" || l.is_empty() {
            break;
        }
        head.push(l);
    }
    assert!(head[0].starts_with("HTTP/1.1 101"), "{head:?}");
    assert!(head.iter().any(|h| h.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")));
    assert!(head.iter().any(|h| h.to_ascii_lowercase().starts_with("sec-websocket-protocol: xmpp")));
    send_text(&mut s, "<open xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\" to=\"prod.ol.epicgames.com\" version=\"1.0\"/>");
    assert!(recv(&mut r).1.starts_with("<open "));
    assert!(recv(&mut r).1.contains("PLAIN"));
    send_text(&mut s, "<auth xmlns=\"urn:ietf:params:xml:ns:xmpp-sasl\" mechanism=\"PLAIN\">AFhib3hFbXVsYXRvcgBlZzF+eA==</auth>");
    assert!(recv(&mut r).1.contains("<success"));
    let ping = [0x89u8, 0x80, 1, 2, 3, 4];
    s.write_all(&ping).unwrap();
    assert_eq!(recv(&mut r).0, 10);
    send_text(&mut s, "<close xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\"/>");
    assert!(recv(&mut r).1.contains("<close"));
    assert_eq!(recv(&mut r).0, 8);
    let mut rest = Vec::new();
    let _ = r.read_to_end(&mut rest);
}

#[test]
fn plain_http_still_works_and_extra_routes() {
    let port = port();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(s, "POST /presence/api/v1/_/XboxEmulator/subscriptions/broadcast HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 204"), "{out}");
}
}
