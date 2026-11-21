#[cfg(tombox_backend_data)]
pub mod http;
pub mod util;
#[cfg(tombox_backend_data)]
pub mod fortnite;
#[cfg(tombox_backend_data)]
pub mod xdefiant;
#[cfg(tombox_backend_data)]
pub mod ws;
#[cfg(tombox_backend_data)]
pub mod xmpp;
#[cfg(tombox_backend_data)]
pub mod epic_extra;
#[cfg(tombox_backend_data)]
pub mod xboxlive;
#[cfg(tombox_backend_data)]
pub mod rta;
#[cfg(tombox_backend_data)]
pub mod trackmania;
#[cfg(tombox_backend_data)]
pub mod playfab;
#[cfg(tombox_backend_data)]
pub mod lan;
#[cfg(tombox_backend_data)]
pub mod xbl_title;
#[cfg(tombox_backend_data)]
pub mod mpsd;

#[cfg(tombox_backend_data)]
pub fn websocket_session(title: &str, req: &Request) -> Option<(Box<dyn ws::WsSession>, Option<String>)> {
    let protos = ws::requested_protocols(req);
    if fortnite::matchmaking::is_mms(req) { return fortnite::matchmaking::websocket(req); }
    if rta::is_rta(req) {
        let proto = protos.iter().find(|p| p.eq_ignore_ascii_case(rta::PROTOCOL)).cloned();
        return Some((Box::new(rta::RtaSession::new()), proto));
    }
    match title {
        "fortnite" => {
            let proto = protos.iter().find(|p| p.eq_ignore_ascii_case("xmpp")).cloned();
            Some((Box::new(xmpp::XmppSession::new()), proto))
        }
        "xdefiant" | "trackmania" => Some((Box::new(xdefiant::Notifications::new()), protos.first().cloned())),
        _ => None,
    }
}

#[cfg(tombox_backend_data)]
fn host_exe_is_xdefiant() -> bool {
    std::env::current_exe().ok().and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_ascii_lowercase())).map_or(false, |f| f.starts_with("rogue_"))
}

#[cfg(tombox_backend_data)]
use std::sync::OnceLock;

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn query(&self, key: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn query_all(&self, key: &str) -> Vec<&str> {
        self.query.iter().filter(|(k, _)| k == key).map(|(_, v)| v.as_str()).collect()
    }

    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    pub fn body_str(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn form(&self, key: &str) -> Option<String> {
        util::parse_pairs(&self.body_str()).into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }

    pub fn segments(&self) -> Vec<&str> {
        self.path.split('/').filter(|s| !s.is_empty()).collect()
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(v: &serde_json::Value) -> Response {
        Response::raw_json(200, serde_json::to_vec(v).unwrap_or_default())
    }

    pub fn json_status(status: u16, v: &serde_json::Value) -> Response {
        Response::raw_json(status, serde_json::to_vec(v).unwrap_or_default())
    }

    pub fn json_str(s: &str) -> Response {
        Response::raw_json(200, s.as_bytes().to_vec())
    }

    pub fn raw_json(status: u16, body: Vec<u8>) -> Response {
        Response { status, content_type: "application/json", headers: Vec::new(), body }
    }

    pub fn bytes(status: u16, content_type: &'static str, body: Vec<u8>) -> Response {
        Response { status, content_type, headers: Vec::new(), body }
    }

    pub fn empty(status: u16) -> Response {
        Response { status, content_type: "", headers: Vec::new(), body: Vec::new() }
    }
}

pub trait TitleService: Send + Sync {
    fn name(&self) -> &'static str;
    fn route(&self, req: &Request) -> Option<Response>;
    fn not_found(&self, req: &Request) -> Response {
        Response::json_status(404, &serde_json::json!({
            "error": "not_found",
            "method": req.method,
            "path": req.path,
        }))
    }
}

#[cfg(tombox_backend_data)]
pub fn title_for(name: &str) -> Option<Box<dyn TitleService>> {
    let mut n = name.trim().to_ascii_lowercase();
    if n.is_empty() && host_exe_is_xdefiant() {
        n = "xdefiant".into();
    }
    if n.is_empty() && trackmania::host_exe_is_trackmania() {
        n = "trackmania".into();
    }
    if n.is_empty() && playfab::host_exe_is_terraria() {
        n = "terraria".into();
    }
    match n.as_str() {
        "terraria" | "playfab" => Some(Box::new(playfab::Terraria)),
        "trackmania" | "tm" | "nadeo" => Some(Box::new(trackmania::Trackmania::new())),
        "xdefiant" | "rogue" | "ubisoft" => Some(Box::new(xdefiant::XDefiant::new())),
        "" | "fortnite" | "epic" => Some(Box::new(fortnite::Fortnite::new())),
        _ => None,
    }
}

#[cfg(tombox_backend_data)]
pub fn dispatch(title: &dyn TitleService, req: &Request) -> Response {
    if req.header("upgrade").map_or(false, |u| u.eq_ignore_ascii_case("websocket")) {
        crate::record_raw("online", "websocket", &format!("[{}] {} {} (websocket upgrade not emulated)", title.name(), req.method, req.path));
    }
    let routed = if xboxlive::wants(req) { xboxlive::route(req) } else if playfab::wants(req) { playfab::route(req) } else { title.route(req).or_else(|| if title.name() == "fortnite" { epic_extra::route(req) } else { None }) };
    match routed {
        Some(r) if r.status == 200 && r.body.is_empty() && xboxlive::wants(req) => {
            crate::record_raw("online", "empty_body", &format!("[{}] {} {} produced 200 with no body", title.name(), req.method, req.path));
            Response::json_status(500, &serde_json::json!({"code": "InternalServerError", "description": "the emulated service produced an empty 200 response"}))
        }
        Some(r) => r,
        None => {
            crate::record_raw("online", "unhandled", &format!("[{}] {} {}", if playfab::wants(req) { "playfab" } else { title.name() }, req.method, req.path));
            if playfab::wants(req) { playfab::Terraria.not_found(req) } else { title.not_found(req) }
        }
    }
}

#[cfg(tombox_backend_data)]
static PORT: OnceLock<u16> = OnceLock::new();

pub fn requested() -> bool {
    let v = std::env::var("TOMBOX_NET_BACKEND").ok().or_else(|| crate::cfg("net_backend")).unwrap_or_default();
    v.trim().eq_ignore_ascii_case("builtin")
}

#[cfg(tombox_backend_data)]
pub fn start_for_config() -> std::io::Result<u16> {
    if let Some(p) = PORT.get() {
        return Ok(*p);
    }
    let name = std::env::var("TOMBOX_NET_TITLE").ok().or_else(|| crate::cfg("net_title")).unwrap_or_default();
    let title = title_for(&name).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("no online service module for title '{name}'")))?;
    let port = http::serve(title)?;
    let _ = PORT.set(port);
    let _ = std::thread::Builder::new().name("tombox-lan-init".into()).spawn(|| {
        let r = lan::role();
        crate::record_raw("online", "lan", &format!("LAN role {r:?}"));
    });
    Ok(port)
}

#[cfg(tombox_backend_data)]
pub fn port() -> Option<u16> {
    PORT.get().copied()
}

#[cfg(not(tombox_backend_data))]
pub fn start_for_config() -> std::io::Result<u16> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "built-in title services require compat-common/backend_data at build time",
    ))
}

#[cfg(not(tombox_backend_data))]
pub fn port() -> Option<u16> {
    None
}

#[cfg(all(test, not(tombox_backend_data)))]
#[test]
fn source_only_build_cannot_start_a_title_service() {
    assert_eq!(start_for_config().unwrap_err().kind(), std::io::ErrorKind::Unsupported);
    assert_eq!(port(), None);
}
