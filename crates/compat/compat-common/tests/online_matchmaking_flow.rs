#[cfg(not(tombox_backend_data))]
#[test]
#[ignore = "needs crates/compat/compat-common/backend_data"]
fn online_matchmaking_flow_needs_backend_data() {}

#[cfg(tombox_backend_data)]
mod backend {
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const GAMESERVER: &str = "127.0.0.1:17777";

fn port() -> u16 {
    std::env::set_var("TOMBOX_NET_BACKEND", "builtin");
    std::env::set_var("TOMBOX_NET_TITLE", "fortnite");
    std::env::set_var("TOMBOX_ONLINE_ACCOUNT", "XboxEmulator");
    std::env::set_var("TOMBOX_FN_GAMESERVER", GAMESERVER);
    compat_common::online::start_for_config().expect("server")
}

struct Http {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Http {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    fn header(&self, k: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
}

fn http(method: &str, host: &str, path: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Http {
    let mut s = TcpStream::connect(("127.0.0.1", port())).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let data = body.map(|b| b.to_string()).unwrap_or_default();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Forwarded-Host: {host}\r\nConnection: close\r\nContent-Length: {}\r\n", data.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(data.as_bytes()).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let text = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = text.lines();
    let status = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    Http { status, headers, body: raw[split + 4..].to_vec() }
}

fn send_text(s: &mut TcpStream, text: &str) {
    let p = text.as_bytes();
    let mask = [0x5au8, 0x13, 0xc4, 0x07];
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

fn upgrade(host: &str, extra: &[(&str, &str)], proto: Option<&str>) -> (TcpStream, BufReader<TcpStream>, Vec<String>) {
    let mut s = TcpStream::connect(("127.0.0.1", port())).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut head = format!("GET / HTTP/1.1\r\nHost: {host}\r\nX-Forwarded-Host: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n");
    if let Some(p) = proto {
        head.push_str(&format!("Sec-WebSocket-Protocol: {p}\r\n"));
    }
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).unwrap();
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut lines = Vec::new();
    loop {
        let mut l = String::new();
        r.read_line(&mut l).unwrap();
        if l == "\r\n" || l.is_empty() {
            break;
        }
        lines.push(l.trim_end().to_string());
    }
    (s, r, lines)
}

fn xmpp_login(acct: &str) -> (TcpStream, BufReader<TcpStream>) {
    let (mut s, mut r, head) = upgrade("xmpp-service-prod.ol.epicgames.com", &[], Some("xmpp"));
    assert!(head[0].starts_with("HTTP/1.1 101"), "{head:?}");
    let open = "<open xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\" to=\"prod.ol.epicgames.com\" version=\"1.0\"/>";
    send_text(&mut s, open);
    assert!(recv(&mut r).1.starts_with("<open "));
    assert!(recv(&mut r).1.contains("PLAIN"));
    let token = compat_common::online::util::b64(format!("\0{acct}\0eg1~token").as_bytes());
    send_text(&mut s, &format!("<auth xmlns=\"urn:ietf:params:xml:ns:xmpp-sasl\" mechanism=\"PLAIN\">{token}</auth>"));
    assert!(recv(&mut r).1.contains("<success"));
    send_text(&mut s, open);
    assert!(recv(&mut r).1.starts_with("<open "));
    assert!(recv(&mut r).1.contains("xmpp-bind"));
    send_text(&mut s, "<iq type=\"set\" id=\"_xmpp_bind1\" xmlns=\"jabber:client\"><bind xmlns=\"urn:ietf:params:xml:ns:xmpp-bind\"><resource>V2:Fortnite:XSX::HARNESS</resource></bind></iq>");
    assert!(recv(&mut r).1.contains(&format!("<jid>{acct}@prod.ol.epicgames.com/V2:Fortnite:XSX::HARNESS</jid>")));
    send_text(&mut s, "<iq type=\"set\" id=\"_xmpp_session1\"><session xmlns=\"urn:ietf:params:xml:ns:xmpp-session\"/></iq>");
    assert!(recv(&mut r).1.contains("type=\"result\""));
    (s, r)
}

fn notification(r: &mut BufReader<TcpStream>, ty: &str, party: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let (op, text) = recv(r);
        if op != 1 || !text.starts_with("<message") {
            continue;
        }
        let a = text.find("<body>").unwrap() + 6;
        let b = text.find("</body>").unwrap();
        let v: Value = serde_json::from_str(&compat_common::online::xmpp::unescape(&text[a..b])).unwrap();
        if v["type"] == json!(format!("com.epicgames.social.party.notification.v0.{ty}")) && v["party_id"] == json!(party) {
            assert!(text.contains("from=\"xmpp-admin@prod.ol.epicgames.com\""));
            return v;
        }
    }
    panic!("no {ty} for {party}");
}

const PARTY_HOST: &str = "party-service-prod.ol.epicgames.com";

#[test]
fn party_ready_to_play_sequence() {
    let acct = "MmFlowParty";
    let (_s, mut r) = xmpp_login(acct);
    let created = http("POST", PARTY_HOST, "/party/api/v1/Fortnite/parties", &[("Content-Type", "application/json")], Some(&json!({
        "config": {"join_confirmation": false, "joinability": "OPEN", "max_size": 16},
        "join_info": {"connection": {"id": format!("{acct}@prod.ol.epicgames.com/V2:Fortnite:XSX::HARNESS"), "meta": {"urn:epic:conn:platform_s": "XSX", "urn:epic:conn:type_s": "game"}}, "meta": {"urn:epic:member:dn_s": acct}},
        "meta": {"urn:epic:cfg:party-type-id_s": "default", "urn:epic:cfg:build-id_s": "1:3:14457890"},
    })));
    assert_eq!(created.status, 200);
    let party = created.json()["id"].as_str().unwrap().to_string();
    let joined = notification(&mut r, "MEMBER_JOINED", &party);
    assert_eq!(joined["account_id"], acct);
    assert_eq!(joined["connection"]["meta"]["urn:epic:conn:platform_s"], "XSX");
    let p = http("PATCH", PARTY_HOST, &format!("/party/api/v1/Fortnite/parties/{party}"), &[], Some(&json!({
        "config": {"join_confirmation": false, "joinability": "OPEN", "max_size": 16},
        "meta": {"delete": [], "update": {"Default:PartyState_s": "BattleRoyaleView", "Default:PlaylistData_j": "{\"PlaylistData\":{\"playlistName\":\"Playlist_DefaultSquad\",\"regionId\":\"NAE\"}}"}},
        "party_state_overridden": {}, "party_privacy_type": "PUBLIC", "party_type": "DEFAULT", "party_sub_type": "default", "max_number_of_members": 16, "invite_ttl_seconds": 14400, "revision": 0,
    })));
    assert_eq!(p.status, 204);
    let up = notification(&mut r, "PARTY_UPDATED", &party);
    assert_eq!(up["revision"], 1);
    assert_eq!(up["captain_id"], acct);
    assert_eq!(up["party_state_updated"]["Default:PartyState_s"], "BattleRoyaleView");
    let m = http("PATCH", PARTY_HOST, &format!("/party/api/v1/Fortnite/parties/{party}/members/{acct}/meta"), &[], Some(&json!({
        "delete": [], "update": {"Default:LobbyState_j": "{\"LobbyState\":{\"inGameReadyCheckStatus\":\"None\",\"gameReadiness\":\"Ready\",\"readyInputType\":\"Controller\"}}"}, "revision": 0,
    })));
    assert_eq!(m.status, 204);
    let ms = notification(&mut r, "MEMBER_STATE_UPDATED", &party);
    assert_eq!(ms["revision"], 1);
    assert!(ms["member_state_updated"]["Default:LobbyState_j"].as_str().unwrap().contains("\"gameReadiness\":\"Ready\""));
    let stale = http("PATCH", PARTY_HOST, &format!("/party/api/v1/Fortnite/parties/{party}"), &[], Some(&json!({"meta": {"delete": [], "update": {}}, "revision": 0})));
    assert_eq!(stale.status, 409);
    assert_eq!(stale.json()["errorCode"], "errors.com.epicgames.social.party.stale_revision");
    assert_eq!(stale.json()["messageVars"], json!(["0", "1"]));
    let user = http("GET", PARTY_HOST, &format!("/party/api/v1/Fortnite/user/{acct}"), &[], None);
    assert_eq!(user.json()["current"][0]["revision"], 1);
    assert_eq!(user.json()["current"][0]["members"][0]["revision"], 1);
}

const FN_HOST: &str = "fortnite-public-service-prod11.ol.epicgames.com";
const MMS_HOST: &str = "fortnite-matchmaking-public-service-live-nae.ol.epicgames.com";

fn ticket(acct: &str) -> Value {
    let t = http("GET", FN_HOST, &format!("/fortnite/api/game/v2/matchmakingservice/ticket/player/{acct}?partyPlayerIds={acct}&bucketId=14457890:1:NAE:playlist_defaultsquad&player.platform=XSX&player.subregions=VA,OH&player.option.partyId=abc&party.XSX=true&input.KBM=false&player.input=GAMEPAD"), &[], None);
    assert_eq!(t.status, 200);
    t.json()
}

#[test]
fn ticket_ws_session_sequence() {
    let acct = "MmFlowTicket";
    let t = ticket(acct);
    let url = t["serviceUrl"].as_str().unwrap();
    assert!(url.starts_with("wss://"), "{url}");
    assert!(!url.contains("127.0.0.1") && !url.contains("localhost"));
    assert_eq!(url, format!("wss://{MMS_HOST}:443"));
    let auth = format!("Epic-Signed mms-player {} {}", t["payload"].as_str().unwrap(), t["signature"].as_str().unwrap());
    let (mut s, mut r, head) = upgrade(MMS_HOST, &[("Authorization", &auth)], None);
    assert!(head[0].starts_with("HTTP/1.1 101"), "{head:?}");
    let mut frames = Vec::new();
    send_text(&mut s, "ping");
    let mut pong = false;
    loop {
        let (op, text) = recv(&mut r);
        if op == 8 {
            break;
        }
        if text == "pong" {
            pong = true;
            continue;
        }
        frames.push(serde_json::from_str::<Value>(&text).unwrap());
    }
    assert!(pong);
    let states: Vec<String> = frames.iter().map(|f| if f["name"] == "Play" { "Play".to_string() } else { f["payload"]["state"].as_str().unwrap().to_string() }).collect();
    assert_eq!(states, ["Connecting", "Waiting", "Queued", "SessionAssignment", "Play"]);
    assert!(frames[..4].iter().all(|f| f["name"] == "StatusUpdate"));
    let ticket_id = frames[2]["payload"]["ticketId"].as_str().unwrap().to_string();
    assert_eq!(ticket_id.len(), 32);
    assert_eq!(frames[1]["payload"]["totalPlayers"], 1);
    let match_id = frames[3]["payload"]["matchId"].clone();
    assert_eq!(frames[4]["payload"]["matchId"], match_id);
    assert_eq!(frames[4]["payload"]["joinDelaySec"], 1);
    let sid = frames[4]["payload"]["sessionId"].as_str().unwrap().to_string();
    let sess = http("GET", FN_HOST, &format!("/fortnite/api/matchmaking/session/{sid}"), &[], None);
    assert_eq!(sess.status, 200);
    let b = sess.json();
    assert_eq!(b["attributes"]["PLAYLISTNAME_s"], "playlist_defaultsquad");
    assert_eq!(b["buildUniqueId"], "14457890");
    assert_eq!(format!("{}:{}", b["serverAddress"].as_str().unwrap(), b["serverPort"]), GAMESERVER);
    let key = http("GET", FN_HOST, &format!("/fortnite/api/game/v2/matchmaking/account/{acct}/session/{sid}"), &[], None);
    assert_eq!(key.status, 200);
    assert_eq!(key.json()["sessionId"], json!(sid));
    assert!(!key.json()["key"].as_str().unwrap().is_empty());
    let join = http("POST", FN_HOST, &format!("/fortnite/api/matchmaking/session/{sid}/join?accountId={acct}"), &[], None);
    assert_eq!(join.status, 204);
    let found = http("GET", FN_HOST, &format!("/fortnite/api/matchmaking/session/findPlayer/{acct}"), &[], None);
    assert!(found.json().as_array().unwrap().iter().any(|s| s["id"] == json!(sid)));
}

#[test]
fn bad_ticket_rejected() {
    let t = ticket("MmFlowBad");
    let auth = format!("Epic-Signed mms-player {} {}", t["payload"].as_str().unwrap(), "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=");
    let (_s, mut r, head) = upgrade(MMS_HOST, &[("Authorization", &auth)], None);
    assert!(head[0].starts_with("HTTP/1.1 401"), "{head:?}");
    let len: usize = head.iter().find_map(|h| h.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap())).unwrap();
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).unwrap();
    let rest = String::from_utf8_lossy(&buf).into_owned();
    assert!(rest.contains("authentication_failed"), "{rest}");
    let bad = http("GET", FN_HOST, "/fortnite/api/game/v2/matchmakingservice/ticket/player/MmFlowBad?partyPlayerIds=MmFlowBad&bucketId=0:0:NONE:0", &[], None);
    assert_eq!(bad.status, 400);
    assert_eq!(bad.json()["errorCode"], "errors.com.epicgames.fortnite.invalid_bucket_id");
}

#[test]
fn mpsd_put_get() {
    let path = "/serviceconfigs/00000000-0000-0000-0000-00006d2de1e1/sessionTemplates/PartySession/sessions/harness-mpsd";
    let body = json!({"constants": {"system": {"visibility": "open", "maxMembersCount": 16}}, "members": {"me": {"constants": {"system": {"initialize": true}}, "properties": {"system": {"active": true}}}}});
    let a = http("PUT", "sessiondirectory.xboxlive.com", path, &[("x-xbl-contract-version", "107")], Some(&body));
    assert_eq!(a.status, 201);
    let e1: u64 = a.header("ETag").unwrap().trim_matches('"').parse().unwrap();
    let g = http("GET", "sessiondirectory.xboxlive.com", path, &[], None);
    assert_eq!(g.status, 200);
    assert_eq!(g.json()["membersInfo"]["count"], 1);
    let b = http("PUT", "sessiondirectory.xboxlive.com", path, &[("If-Match", a.header("ETag").unwrap())], Some(&json!({"properties": {"custom": {"partyId": "x"}}})));
    assert_eq!(b.status, 200);
    let e2: u64 = b.header("ETag").unwrap().trim_matches('"').parse().unwrap();
    assert!(e2 > e1);
    assert_eq!(b.json()["properties"]["custom"]["partyId"], "x");
    let stale = http("PUT", "sessiondirectory.xboxlive.com", path, &[("If-Match", a.header("ETag").unwrap())], Some(&json!({"properties": {"custom": {"partyId": "y"}}})));
    assert_eq!(stale.status, 412);
    let left = http("PUT", "sessiondirectory.xboxlive.com", path, &[], Some(&json!({"members": {"me": null}})));
    assert_eq!(left.status, 204);
    assert_eq!(http("GET", "sessiondirectory.xboxlive.com", path, &[], None).status, 404);
}

#[test]
fn run36_lobby_party_replay() {
    let acct = "XboxEmulator";
    let (_s, mut r) = xmpp_login(acct);
    let before = http("GET", PARTY_HOST, &format!("/party/api/v1/Fortnite/user/{acct}"), &[], None);
    assert_eq!(before.status, 200);
    let created = http("POST", PARTY_HOST, "/party/api/v1/Fortnite/parties", &[("Content-Type", "application/json")], Some(&json!({
        "config": {"join_confirmation": false, "joinability": "OPEN", "max_size": 16},
        "join_info": {"connection": {"id": format!("{acct}@prod.ol.epicgames.com/V2:Fortnite:XSX::HARNESS"), "meta": {"urn:epic:conn:platform_s": "XSX", "urn:epic:conn:type_s": "game"}}, "meta": {"urn:epic:member:dn_s": acct}},
        "meta": {"urn:epic:cfg:party-type-id_s": "default"},
    })));
    assert_eq!(created.status, 200);
    let party = created.json()["id"].as_str().unwrap().to_string();
    notification(&mut r, "MEMBER_JOINED", &party);
    let count = http("GET", PARTY_HOST, &format!("/party/api/v1/Fortnite/user/{acct}/notifications/undelivered/count"), &[], None);
    assert_eq!(count.status, 200);
    let t = Instant::now();
    let p1 = http("PATCH", PARTY_HOST, &format!("/party/api/v1/Fortnite/parties/{party}"), &[], Some(&json!({
        "config": {"joinability": "OPEN"}, "meta": {"delete": [], "update": {"Default:PrivacySettings_j": "{\"PrivacySettings\":{\"partyType\":\"Public\"}}"}},
        "party_state_overridden": {}, "party_privacy_type": "PUBLIC", "party_type": "DEFAULT", "party_sub_type": "default", "max_number_of_members": 16, "invite_ttl_seconds": 14400, "revision": 0,
    })));
    assert_eq!(p1.status, 204);
    let m = http("PATCH", PARTY_HOST, &format!("/party/api/v1/Fortnite/parties/{party}/members/{acct}/meta"), &[], Some(&json!({
        "delete": [], "update": {"Default:Location_s": "PreLobby"}, "revision": 0,
    })));
    assert_eq!(m.status, 204);
    assert_eq!(notification(&mut r, "PARTY_UPDATED", &party)["revision"], 1);
    assert_eq!(notification(&mut r, "MEMBER_STATE_UPDATED", &party)["revision"], 1);
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    let p2 = http("PATCH", PARTY_HOST, &format!("/party/api/v1/Fortnite/parties/{party}"), &[], Some(&json!({
        "config": {"joinability": "OPEN"}, "meta": {"delete": [], "update": {"Default:PrivacySettings_j": "{\"PrivacySettings\":{\"partyType\":\"Public\",\"bOnlyLeaderFriendsCanJoin\":false}}"}},
        "party_privacy_type": "PUBLIC", "revision": 1,
    })));
    assert_eq!(p2.status, 204);
    assert_eq!(notification(&mut r, "PARTY_UPDATED", &party)["revision"], 2);
    let m2 = http("PATCH", PARTY_HOST, &format!("/party/api/v1/Fortnite/parties/{party}/members/{acct}/meta"), &[], Some(&json!({
        "delete": [], "update": {"Default:Location_s": "Lobby"}, "revision": 1,
    })));
    assert_eq!(m2.status, 204);
    assert_eq!(notification(&mut r, "MEMBER_STATE_UPDATED", &party)["revision"], 2);
    let got = http("GET", PARTY_HOST, &format!("/party/api/v1/Fortnite/parties/{party}"), &[], None).json();
    assert_eq!(got["revision"], 2);
    assert_eq!(got["members"][0]["meta"]["Default:Location_s"], "Lobby");
    assert!(got["meta"]["Default:PrivacySettings_j"].as_str().unwrap().contains("bOnlyLeaderFriendsCanJoin"));
    let user = http("GET", PARTY_HOST, &format!("/party/api/v1/Fortnite/user/{acct}"), &[], None).json();
    assert!(user["current"].as_array().unwrap().iter().any(|p| p["id"] == json!(party)));
}

const RUN42_TICKET: &str = "/fortnite/api/game/v2/matchmakingservice/ticket/player/XboxEmulator?partyPlayerIds=XboxEmulator&bucketId=14457890%3A0%3ANONE%3Aplaylist_defaultsquad&player.platform=XboxOne&player.subregions=&player.option.fillTeam=true&player.option.crossplayOptOut=false&player.option.partyId=165b3e31867daeb612f7c7a48c870aaa&player.option.splitScreen=false&party.XSX=true&input.KBM=true&player.input=KBM&player.option.uiLanguage=en";

#[test]
fn run42_play_ticket_to_local_match() {
    let acct = "XboxEmulator";
    let t = http("GET", FN_HOST, RUN42_TICKET, &[], None);
    assert_eq!(t.status, 200, "{}", String::from_utf8_lossy(&t.body));
    let t = t.json();
    let url = t["serviceUrl"].as_str().unwrap().to_string();
    assert!(url.starts_with("wss://fortnite-matchmaking-public-service-live-"), "{url}");
    let host = url.trim_start_matches("wss://").split(':').next().unwrap().to_string();
    let auth = format!("Epic-Signed mms-player {} {}", t["payload"].as_str().unwrap(), t["signature"].as_str().unwrap());
    let (_s, mut r, head) = upgrade(&host, &[("Authorization", &auth)], None);
    assert!(head[0].starts_with("HTTP/1.1 101"), "{head:?}");
    let mut frames = Vec::new();
    loop {
        let (op, text) = recv(&mut r);
        if op == 8 {
            break;
        }
        frames.push(serde_json::from_str::<Value>(&text).unwrap());
    }
    let play = frames.last().unwrap();
    assert_eq!(play["name"], "Play");
    let sid = play["payload"]["sessionId"].as_str().unwrap().to_string();
    let b = http("GET", FN_HOST, &format!("/fortnite/api/matchmaking/session/{sid}"), &[], None).json();
    assert_eq!(b["attributes"]["PLAYLISTNAME_s"], "playlist_defaultsquad");
    assert_eq!(b["buildUniqueId"], "14457890");
    let addr = b["serverAddress"].as_str().unwrap().to_string();
    let port = b["serverPort"].as_i64().unwrap() as i32;
    assert_eq!(format!("{addr}:{port}"), GAMESERVER);
    assert_eq!(http("GET", FN_HOST, &format!("/fortnite/api/game/v2/matchmaking/account/{acct}/session/{sid}"), &[], None).status, 200);
    assert_eq!(http("POST", FN_HOST, &format!("/fortnite/api/matchmaking/session/{sid}/join?accountId={acct}"), &[], None).status, 204);
    assert_eq!(std::env::var("TOMBOX_FN_PLAYLIST").unwrap(), "playlist_defaultsquad");
    use compat_common::os_host::fortnite_match as fm;
    let travel = fm::UrlView { protocol: "unreal".into(), host: addr, port, map: "/Game/Maps/Frontend".into(), ops: vec!["EncryptionToken=x".into()] };
    let sentinel = fm::parse_endpoint(GAMESERVER).unwrap();
    let local = fm::rewrite_text(&travel, &sentinel, fm::DEFAULT_MAP, fm::DEFAULT_GAMEMODE, &fm::playlist()).unwrap();
    assert_eq!(local, "/Game/Athena/Apollo/Maps/Apollo_Terrain?game=/Game/Athena/Athena_GameMode.Athena_GameMode_C?Playlist=playlist_defaultsquad");
}
}
