#![cfg(tombox_backend_data)]

use compat_common::online::{fortnite::Fortnite, http, util};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::OnceLock;

fn port() -> u16 {
    static P: OnceLock<u16> = OnceLock::new();
    *P.get_or_init(|| http::serve(Box::new(Fortnite::with_account("XboxEmulator", "XboxEmulator"))).expect("server start"))
}

struct Client {
    w: TcpStream,
    r: BufReader<TcpStream>,
}

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("json body")
    }
    fn header(&self, k: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
}

impl Client {
    fn new() -> Client {
        let s = TcpStream::connect(("127.0.0.1", port())).expect("connect");
        Client { w: s.try_clone().unwrap(), r: BufReader::new(s) }
    }

    fn send_raw(&mut self, raw: &[u8]) -> Resp {
        self.w.write_all(raw).unwrap();
        let mut line = String::new();
        self.r.read_line(&mut line).unwrap();
        let status: u16 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
        let mut headers = Vec::new();
        loop {
            let mut l = String::new();
            self.r.read_line(&mut l).unwrap();
            let l = l.trim_end().to_string();
            if l.is_empty() {
                break;
            }
            let (k, v) = l.split_once(':').unwrap();
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
        let len = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).map(|(_, v)| v.parse::<usize>().unwrap()).unwrap_or(0);
        let mut body = vec![0u8; len];
        self.r.read_exact(&mut body).unwrap();
        Resp { status, headers, body }
    }

    fn req(&mut self, method: &str, target: &str, extra: &str, body: &str) -> Resp {
        let raw = format!(
            "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nUser-Agent: Fortnite/++Fortnite+Release-14.50-CL-14617919 XboxOneGDK/10.0\r\n{extra}Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        self.send_raw(raw.as_bytes())
    }
}

#[test]
fn oauth_eg1_token_decodes_with_future_exp() {
    let mut c = Client::new();
    let auth = format!("Authorization: basic {}\r\nContent-Type: application/x-www-form-urlencoded\r\n", util::b64(b"myclient:secret"));
    let r = c.req("POST", "/account/api/oauth/token", &auth, "grant_type=external_auth&external_auth_type=xbl&token_type=eg1");
    assert_eq!(r.status, 200);
    let j = r.json();
    assert_eq!(j["account_id"], "XboxEmulator");
    assert_eq!(j["displayName"], "XboxEmulator");
    let tok = j["access_token"].as_str().unwrap();
    let rest = tok.strip_prefix("eg1~").expect("eg1 prefix");
    let parts: Vec<&str> = rest.split('.').collect();
    assert_eq!(parts.len(), 3);
    let header: Value = serde_json::from_slice(&util::b64_decode(parts[0])).unwrap();
    assert_eq!(header["typ"], "JWT");
    let payload: Value = serde_json::from_slice(&util::b64_decode(parts[1])).unwrap();
    let now = util::now_secs();
    assert!(payload["exp"].as_u64().unwrap() > now);
    assert!(payload["iat"].as_u64().unwrap() <= now + 1);
    assert_eq!(payload["sub"], "XboxEmulator");
    assert_eq!(payload["clid"], "myclient");

    let r = c.req("POST", "/account/api/oauth/token", &auth, "grant_type=client_credentials&token_type=eg1");
    let j = r.json();
    assert!(j.get("account_id").is_none());
    assert_eq!(j["client_id"], "myclient");
    let p: Value = serde_json::from_slice(&util::b64_decode(j["access_token"].as_str().unwrap()[4..].split('.').nth(1).unwrap())).unwrap();
    assert_eq!(p["am"], "client_credentials");

    let r = c.req("DELETE", "/account/api/oauth/sessions/kill?killType=OTHERS_ACCOUNT_CLIENT_SERVICE", "", "");
    assert_eq!(r.status, 204);
    assert!(r.header("content-length").is_none());
    let r = c.req("GET", "/account/api/public/account?accountId=XboxEmulator&accountId=Other", "", "");
    assert_eq!(r.json().as_array().unwrap().len(), 2);
}

#[test]
fn query_profile_athena_full_update() {
    let mut c = Client::new();
    let r = c.req("POST", "/fortnite/api/game/v2/profile/XboxEmulator/client/QueryProfile?profileId=athena&rvn=-1", "Content-Type: application/json\r\n", "{}");
    assert_eq!(r.status, 200);
    let j = r.json();
    assert_eq!(j["profileId"], "athena");
    let ch = &j["profileChanges"][0];
    assert_eq!(ch["changeType"], "fullProfileUpdate");
    assert_eq!(ch["profile"]["accountId"], "XboxEmulator");
    assert_eq!(ch["profile"]["stats"]["attributes"]["season_num"], 14);
    assert!(ch["profile"]["items"].as_object().unwrap().len() > 100);
    let rvn = j["profileRevision"].as_i64().unwrap();

    let body = r#"{"lockerItem":"xe-loadout","category":"Character","itemToSlot":"AthenaCharacter:CID_009_Athena_Commando_M","slotIndex":0,"variantUpdates":[]}"#;
    let chunked = format!(
        "POST /fortnite/api/game/v2/profile/XboxEmulator/client/SetCosmeticLockerSlot?profileId=athena&rvn={rvn} HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n",
        10, &body[..10], body.len() - 10, &body[10..]
    );
    let r = c.send_raw(chunked.as_bytes());
    let j = r.json();
    assert_eq!(j["profileRevision"].as_i64().unwrap(), rvn + 1);
    let ch = &j["profileChanges"][0];
    assert_eq!(ch["changeType"], "itemAttrChanged");
    assert_eq!(ch["attributeValue"]["slots"]["Character"]["items"][0], "AthenaCharacter:CID_009_Athena_Commando_M");

    let r = c.req("POST", "/fortnite/api/game/v2/profile/XboxEmulator/client/SetMtxPlatform?profileId=common_core&rvn=-1", "", r#"{"newPlatform":"XBL"}"#);
    assert_eq!(r.json()["profileChanges"][0]["profile"]["stats"]["attributes"]["current_mtx_platform"], "XBL");

    let r = c.req("POST", "/fortnite/api/game/v2/profile/XboxEmulator/client/SomethingNew?profileId=athena&rvn=-1", "", "{}");
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["profileChanges"].as_array().unwrap().len(), 0);

    let r = c.req("POST", "/fortnite/api/game/v2/profile/XboxEmulator/client/QueryProfile", "", "{}");
    assert_eq!(r.status, 404);
}

#[test]
fn cloudstorage_system_hashes_match() {
    let mut c = Client::new();
    let r = c.req("GET", "/fortnite/api/cloudstorage/system", "", "");
    assert_eq!(r.status, 200);
    let list = r.json();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 4);
    for e in list {
        let name = e["filename"].as_str().unwrap();
        let f = c.req("GET", &format!("/fortnite/api/cloudstorage/system/{name}"), "", "");
        assert_eq!(f.status, 200);
        assert_eq!(e["length"].as_u64().unwrap() as usize, f.body.len());
        assert_eq!(e["hash"].as_str().unwrap(), util::hex(&util::sha1(&f.body)));
        assert_eq!(e["hash256"].as_str().unwrap(), util::hex(&util::sha256(&f.body)));
        assert!(!String::from_utf8_lossy(&f.body).to_ascii_lowercase().contains("lawin"));
    }
    let r = c.req("GET", "/fortnite/api/cloudstorage/user/XboxEmulator", "", "");
    assert_eq!(r.body, b"[]");
    let r = c.req("PUT", "/fortnite/api/cloudstorage/user/XboxEmulator/ClientSettings.Sav", "", "abc");
    assert_eq!(r.status, 204);
    let r = c.req("GET", "/fortnite/api/cloudstorage/user/XboxEmulator/ClientSettings.Sav", "", "");
    assert_eq!(r.body, b"abc");
}

#[test]
fn replays_every_run15_request() {
    let form = "Content-Type: application/x-www-form-urlencoded\r\n";
    let js = "Content-Type: application/json\r\n";
    let rows: &[(&str, &str, &str, &str, u16)] = &[
        ("DELETE", "/account/api/oauth/sessions/kill", "", "", 204),
        ("GET", "/account/api/public/account?accountId=XboxEmulator", "", "", 200),
        ("GET", "/account/api/public/account/XboxEmulator", "", "", 200),
        ("GET", "/account/api/public/account/XboxEmulator/externalAuths", "", "", 200),
        ("GET", "/api/v1/Fortnite/get", "", "", 200),
        ("GET", "/api/v1/events/Fortnite/download/XboxEmulator", "", "", 200),
        ("GET", "/content-controls/XboxEmulator", "", "", 200),
        ("GET", "/content/api/pages/fortnite-game/", "", "", 200),
        ("GET", "/fortnite/api/calendar/v1/timeline", "", "", 200),
        ("GET", "/fortnite/api/cloudstorage/system", "", "", 200),
        ("GET", "/fortnite/api/cloudstorage/system/DefaultEngine.ini", "", "", 200),
        ("GET", "/fortnite/api/cloudstorage/system/DefaultGame.ini", "", "", 200),
        ("GET", "/fortnite/api/cloudstorage/system/DefaultInput.ini", "", "", 200),
        ("GET", "/fortnite/api/cloudstorage/system/DefaultRuntimeOptions.ini", "", "", 200),
        ("GET", "/fortnite/api/cloudstorage/system/config", "", "", 200),
        ("GET", "/fortnite/api/cloudstorage/user/XboxEmulator", "", "", 200),
        ("GET", "/fortnite/api/game/v2/enabled_features", "", "", 200),
        ("GET", "/fortnite/api/game/v2/privacy/account/XboxEmulator", "", "", 200),
        ("GET", "/fortnite/api/receipts/v1/account/XboxEmulator/receipts", "", "", 200),
        ("GET", "/fortnite/api/storefront/v2/catalog", "", "", 200),
        ("GET", "/fortnite/api/storefront/v2/keychain", "", "", 200),
        ("GET", "/friends/api/v1/XboxEmulator/blocklist", "", "", 200),
        ("GET", "/friends/api/v1/XboxEmulator/recent/fortnite", "", "", 200),
        ("GET", "/friends/api/v1/XboxEmulator/settings", "", "", 200),
        ("GET", "/friends/api/v1/XboxEmulator/summary", "", "", 200),
        ("GET", "/lightswitch/api/service/bulk/status?serviceId=Fortnite", "", "", 200),
        ("GET", "/party/api/v1/Fortnite/user/XboxEmulator", "", "", 200),
        ("GET", "/party/api/v1/Fortnite/user/XboxEmulator/notifications/undelivered/count", "", "", 200),
        ("GET", "/presence/api/v1/_/XboxEmulator/last-online", "", "", 200),
        ("GET", "/presence/api/v1/_/XboxEmulator/settings/subscriptions", "", "", 200),
        ("GET", "/rootDesc.xml", "", "", 404),
        ("GET", "/sdk/v1/default", "", "", 200),
        ("GET", "/sdk/v1/product/prod-fn", "", "", 200),
        ("GET", "/socialban/api/public/v1/XboxEmulator", "", "", 200),
        ("GET", "/statsproxy/api/statsv2/account/XboxEmulator", "", "", 200),
        ("GET", "/waitingroom/api/waitingroom", "", "", 204),
        ("POST", "/account/api/oauth/token", form, "grant_type=external_auth&external_auth_type=xbl&external_auth_token=x&token_type=eg1", 200),
        ("POST", "/account/api/oauth/token", form, "grant_type=client_credentials&token_type=eg1", 200),
        ("POST", "/api/v1/user/setting", js, r#"{"settings":["avatar"]}"#, 200),
        ("POST", "/auth/v1/oauth/token", form, "grant_type=client_credentials&deployment_id=x", 200),
        ("POST", "/datarouter/api/v1/public/data?SessionID=1&AppID=Fortnite", js, "{}", 204),
        ("POST", "/datarouter/api/v1/public/data/clients", js, "{}", 204),
        ("POST", "/fortnite/api/game/v2/profile/XboxEmulator/client/QueryProfile?profileId=athena&rvn=-1", js, "{}", 200),
        ("POST", "/fortnite/api/game/v2/profile/XboxEmulator/client/QueryProfile?profileId=common_core&rvn=-1", js, "{}", 200),
        ("POST", "/fortnite/api/game/v2/profile/XboxEmulator/client/QueryProfile?profileId=common_public&rvn=-1", js, "{}", 200),
        ("POST", "/fortnite/api/game/v2/profile/XboxEmulator/client/QueryProfile?profileId=creative&rvn=-1", js, "{}", 200),
        ("POST", "/fortnite/api/game/v2/profile/XboxEmulator/client/SetMtxPlatform?profileId=common_core&rvn=-1", js, r#"{"newPlatform":"XBL"}"#, 200),
    ];
    let mut c = Client::new();
    for (m, p, h, b, st) in rows {
        let r = c.req(m, p, h, b);
        assert_eq!(r.status, *st, "{m} {p}");
        if r.header("content-type").map_or(false, |t| t.starts_with("application/json")) {
            serde_json::from_slice::<Value>(&r.body).unwrap_or_else(|e| panic!("{m} {p}: bad json {e}"));
        }
    }
}

#[test]
fn unknown_route_404_and_lobby_routes() {
    let mut c = Client::new();
    let r = c.req("GET", "/definitely/not/a/route", "", "");
    assert_eq!(r.status, 404);
    assert_eq!(r.json()["errorCode"], "errors.com.epicgames.common.not_found");
    assert_eq!(c.req("GET", "/presence/api/v1/_/XboxEmulator/settings/subscriptions", "", "").body, b"[]");
    assert_eq!(c.req("GET", "/friends/api/v1/XboxEmulator/recent/fortnite", "", "").body, b"[]");
    assert_eq!(c.req("GET", "/api/v1/Fortnite/get", "", "").body, b"{}");
    assert_eq!(c.req("GET", "/content-controls/XboxEmulator", "", "").status, 200);
    assert_eq!(c.req("POST", "/datarouter/api/v1/public/data/clients", "", "{}").status, 204);
    assert_eq!(c.req("POST", "/datarouter/api/v1/public/data?SessionID=x", "", "{}").status, 204);
    assert_eq!(c.req("GET", "/waitingroom/api/waitingroom", "", "").status, 204);
    let t = c.req("GET", "/fortnite/api/calendar/v1/timeline", "", "").json();
    assert!(t["currentTime"].as_str().unwrap().starts_with("20"));
    let pages = c.req("GET", "/content/api/pages/fortnite-game/", "", "");
    let s = String::from_utf8_lossy(&pages.body).to_ascii_lowercase();
    assert!(!s.contains("lawin") && !s.contains("discord.gg"));
    assert_eq!(pages.json()["emergencynotice"]["news"]["messages"].as_array().unwrap().len(), 0);
    for p in ["/fortnite/api/storefront/v2/catalog", "/fortnite/api/storefront/v2/keychain", "/lightswitch/api/service/bulk/status",
              "/sdk/v1/default", "/friends/api/v1/XboxEmulator/summary", "/party/api/v1/Fortnite/user/XboxEmulator",
              "/fortnite/api/game/v2/privacy/account/XboxEmulator", "/api/v1/events/Fortnite/download/XboxEmulator",
              "/statsproxy/api/statsv2/account/XboxEmulator", "/account/api/public/account/XboxEmulator/externalAuths",
              "/socialban/api/public/v1/XboxEmulator", "/fortnite/api/receipts/v1/account/XboxEmulator/receipts"] {
        let r = c.req("GET", p, "", "");
        assert_eq!(r.status, 200, "{p}");
        let _ = r.json();
    }
}

#[test]
fn blurl_video_flow_serves_playable_hls() {
    let mut c = Client::new();
    for p in ["/PbgTgOjJvNqMrvYW/master.blurl", "/PbgTgOjJvNqMrvYW/master.json", "/tnxfhPXtcEQHjgsN/master-en.blurl"] {
        let r = c.req("GET", p, "X-Forwarded-Host: video.example.invalid
", "");
        assert_eq!(r.status, 200, "{p}");
        assert!(!r.body.starts_with(b"blul"));
        let j = r.json();
        let pl = j["playlists"].as_array().expect("playlists array");
        assert!(!pl.is_empty());
        assert!(pl.iter().all(Value::is_object));
        assert_eq!(pl[0]["type"], "master");
        assert_eq!(pl[0]["language"], "en");
        let id = p.split('/').nth(1).unwrap();
        assert_eq!(pl[0]["url"], format!("https://video.example.invalid/{id}/master.m3u8"));
        assert!(pl[0].get("rel_url").is_none());
        assert!(j.get("bulkregionlockdata").is_none() && j.get("ucp").is_none() && j.get("subtitles").is_none());
    }
    for base in ["/PbgTgOjJvNqMrvYW/", "/PbgTgOjJvNqMrvYW/master.blurl/", "/PbgTgOjJvNqMrvYW/master.json/"] {
        let master_url = c.req("GET", "/PbgTgOjJvNqMrvYW/master.blurl", "", "").json()["playlists"][0]["url"].as_str().unwrap().to_string();
        assert_eq!(master_url, "http://127.0.0.1/PbgTgOjJvNqMrvYW/master.m3u8");
        let master_rel = master_url.rsplit('/').next().unwrap();
        let m = c.req("GET", &format!("{base}{master_rel}"), "", "");
        assert_eq!(m.status, 200);
        assert_eq!(m.header("content-type"), Some("application/vnd.apple.mpegurl"));
        let ms = String::from_utf8(m.body.clone()).unwrap();
        assert!(ms.starts_with("#EXTM3U"));
        let inf = ms.lines().position(|l| l.starts_with("#EXT-X-STREAM-INF:")).expect("variant");
        assert!(ms.lines().nth(inf).unwrap().contains("BANDWIDTH="));
        let variant = ms.lines().nth(inf + 1).unwrap().to_string();
        let v = c.req("GET", &format!("{base}{variant}"), "", "");
        assert_eq!(v.status, 200);
        let vs = String::from_utf8(v.body.clone()).unwrap();
        assert!(vs.contains("#EXT-X-ENDLIST"));
        let map = vs.lines().find_map(|l| l.strip_prefix("#EXT-X-MAP:URI=\"")).unwrap().trim_end_matches('"').to_string();
        let init = c.req("GET", &format!("{base}{map}"), "", "");
        assert_eq!(init.status, 200);
        assert_eq!(&init.body[4..8], b"ftyp");
        assert!(init.body.windows(4).any(|w| w == b"avc1") && !init.body.windows(4).any(|w| w == b"mp4a"));
        assert_eq!(init.body.windows(4).filter(|w| *w == b"trak").count(), 1);
        let media = ms.lines().find(|l| l.starts_with("#EXT-X-MEDIA:TYPE=AUDIO")).expect("audio rendition");
        assert!(media.contains("GROUP-ID=\"aud\"") && media.contains("DEFAULT=YES"));
        assert!(ms.lines().nth(inf).unwrap().contains("AUDIO=\"aud\"") && ms.lines().nth(inf).unwrap().contains("mp4a.40.2"));
        let aurl = media.split("URI=\"").nth(1).unwrap().trim_end_matches('"').to_string();
        let a = c.req("GET", &format!("{base}{aurl}"), "", "");
        assert_eq!(a.status, 200);
        let as_ = String::from_utf8(a.body.clone()).unwrap();
        assert!(as_.contains("#EXT-X-ENDLIST"));
        let amap = as_.lines().find_map(|l| l.strip_prefix("#EXT-X-MAP:URI=\"")).unwrap().trim_end_matches('"').to_string();
        let ainit = c.req("GET", &format!("{base}{amap}"), "", "");
        assert_eq!(ainit.status, 200);
        assert_eq!(&ainit.body[4..8], b"ftyp");
        assert!(ainit.body.windows(4).any(|w| w == b"mp4a") && !ainit.body.windows(4).any(|w| w == b"avc1"));
        assert_eq!(ainit.body.windows(4).filter(|w| *w == b"trak").count(), 1);
        let asegs: Vec<&str> = as_.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        assert_eq!(asegs.len(), 1);
        let aseg = c.req("GET", &format!("{base}{}", asegs[0]), "", "");
        assert_eq!(aseg.status, 200);
        assert!(aseg.body.windows(4).any(|w| w == b"moof") && aseg.body.windows(4).any(|w| w == b"mdat"));
        assert_eq!(aseg.body.windows(4).filter(|w| *w == b"traf").count(), 1);
        let segs: Vec<&str> = vs.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        assert_eq!(segs.len(), 1);
        let s = c.req("GET", &format!("{base}{}", segs[0]), "", "");
        assert_eq!(s.status, 200);
        assert!(s.body.windows(4).any(|w| w == b"moof") && s.body.windows(4).any(|w| w == b"mdat"));
        assert_eq!(s.body.windows(4).filter(|w| *w == b"traf").count(), 1);
        let total = s.body.len();
        let part = c.req("GET", &format!("{base}{}", segs[0]), "Range: bytes=0-99\r\n", "");
        assert_eq!(part.status, 206);
        assert_eq!(part.body, s.body[..100].to_vec());
        assert_eq!(part.header("content-range").unwrap(), format!("bytes 0-99/{total}"));
        let tail = c.req("GET", &format!("{base}{}", segs[0]), "Range: bytes=100-\r\n", "");
        assert_eq!(tail.status, 206);
        assert_eq!(tail.body, s.body[100..].to_vec());
        let bad = c.req("GET", &format!("{base}{}", segs[0]), &format!("Range: bytes={total}-\r\n"), "");
        assert_eq!(bad.status, 416);
    }
    let raw = "GET /PbgTgOjJvNqMrvYW/master.blurl HTTP/1.1
Host: 127.0.0.1:54321
Content-Length: 0

";
    assert_eq!(c.send_raw(raw.as_bytes()).json()["playlists"][0]["url"], "http://127.0.0.1:54321/PbgTgOjJvNqMrvYW/master.m3u8");
    let raw = "GET /PbgTgOjJvNqMrvYW/master.blurl HTTP/1.1
Host: 127.0.0.1:54321
X-Forwarded-Host: localhost
Content-Length: 0

";
    assert_eq!(c.send_raw(raw.as_bytes()).json()["playlists"][0]["url"], "http://127.0.0.1:54321/PbgTgOjJvNqMrvYW/master.m3u8");
    assert_eq!(c.req("GET", "/PbgTgOjJvNqMrvYW/master.blurl", "X-Forwarded-Host: bad/host
", "").json()["playlists"][0]["url"], "http://127.0.0.1/PbgTgOjJvNqMrvYW/master.m3u8");
    assert_eq!(c.req("GET", "/PbgTgOjJvNqMrvYW/master.blurl/master.blurl", "", "").status, 404);
    assert_eq!(c.req("GET", "/PbgTgOjJvNqMrvYW/foo/init.mp4", "", "").status, 404);
    assert_eq!(c.req("GET", "/PbgTgOjJvNqMrvYW/other.bin", "", "").status, 404);
    assert_eq!(c.req("POST", "/PbgTgOjJvNqMrvYW/master.blurl", "", "").status, 404);
}
