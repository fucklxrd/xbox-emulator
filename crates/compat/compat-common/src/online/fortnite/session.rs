use super::super::{util, Request, Response};
use super::matchmaking::Ticket;
use super::party::{capture, note};
use super::{epic_error, Fortnite};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub const MAX_PUBLIC_PLAYERS: usize = 100;
pub const DEFAULT_GAMESERVER: (&str, u16) = ("127.0.0.1", 7777);

#[derive(Clone, Debug)]
pub struct Sess {
    pub id: String,
    pub match_id: String,
    pub owner_id: String,
    pub owner_name: String,
    pub playlist: String,
    pub region: String,
    pub subregion: String,
    pub net_cl: String,
    pub server_address: String,
    pub server_port: u16,
    pub key: String,
    pub session_key: String,
    pub players: Vec<String>,
    pub public_players: Vec<String>,
    pub private_players: Vec<String>,
    pub extra_attributes: Map<String, Value>,
    pub max_public: usize,
    pub max_private: usize,
    pub started: bool,
    pub last_updated: String,
}

fn sessions() -> &'static Mutex<HashMap<String, Sess>> {
    static S: OnceLock<Mutex<HashMap<String, Sess>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn gameserver() -> (String, u16) {
    if let Some(v) = std::env::var("TOMBOX_FN_GAMESERVER").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        if let Some((ip, port)) = v.rsplit_once(':') {
            if let Ok(p) = port.trim().parse::<u16>() {
                if !ip.trim().is_empty() {
                    return (ip.trim().to_string(), p);
                }
            }
        }
        note("session", &format!("TOMBOX_FN_GAMESERVER '{v}' is not ip:port; using cfg/default"));
    }
    let ip = crate::cfg("online_gameserver_ip").filter(|s| !s.is_empty()).unwrap_or_else(|| DEFAULT_GAMESERVER.0.into());
    let port = crate::cfg("online_gameserver_port").and_then(|p| p.parse::<u16>().ok()).unwrap_or(DEFAULT_GAMESERVER.1);
    (ip, port)
}

fn random_key() -> String {
    let h = format!("{}{}", util::random_hex32(), util::random_hex32());
    let bytes: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap_or(0)).collect();
    util::b64(&bytes)
}

pub fn create(sid: &str, match_id: &str, t: &Ticket) -> Sess {
    let (ip, port) = gameserver();
    let owner = util::random_hex32().to_uppercase();
    let region = t.region.clone();
    let s = Sess {
        id: sid.to_string(),
        match_id: match_id.to_string(),
        owner_name: format!("[DS]fortnite-live{}-{}", region.to_ascii_lowercase(), &owner[..12].to_ascii_lowercase()),
        owner_id: owner,
        playlist: t.playlist.clone(),
        subregion: t.subregions.first().cloned().unwrap_or_else(|| region.clone()),
        region,
        net_cl: t.net_cl.clone(),
        server_address: ip,
        server_port: port,
        key: random_key(),
        session_key: util::random_hex32().to_uppercase(),
        players: t.party_player_ids.clone(),
        public_players: Vec::new(),
        private_players: Vec::new(),
        extra_attributes: Map::new(),
        max_public: MAX_PUBLIC_PLAYERS,
        max_private: 0,
        started: false,
        last_updated: util::iso_now(),
    };
    note("session", &format!("create {sid} match {match_id} playlist {} build {} server {}:{} players {:?}", s.playlist, s.net_cl, s.server_address, s.server_port, s.players));
    sessions().lock().unwrap_or_else(|p| p.into_inner()).insert(sid.to_string(), s.clone());
    if !s.playlist.is_empty() {
        std::env::set_var("TOMBOX_FN_PLAYLIST", &s.playlist);
    }
    s
}

pub fn get(sid: &str) -> Option<Sess> {
    sessions().lock().unwrap_or_else(|p| p.into_inner()).get(sid).cloned()
}

pub fn session_json(s: &Sess) -> Value {
    let open = s.max_public.saturating_sub(s.players.len().max(s.public_players.len()));
    let mut v = json!({
        "id": s.id,
        "ownerId": s.owner_id,
        "ownerName": s.owner_name,
        "serverName": s.owner_name,
        "serverAddress": s.server_address,
        "serverPort": s.server_port,
        "maxPublicPlayers": s.max_public,
        "openPublicPlayers": open,
        "maxPrivatePlayers": s.max_private,
        "openPrivatePlayers": s.max_private.saturating_sub(s.private_players.len()),
        "attributes": {
            "REGION_s": s.region,
            "GAMEMODE_s": "FORTATHENA",
            "ALLOWBROADCASTING_b": true,
            "SUBREGION_s": s.subregion,
            "DCID_s": format!("FORTNITE-LIVE{}-{}", s.region, s.subregion).to_ascii_uppercase(),
            "tenant_s": "Fortnite",
            "MATCHMAKINGPOOL_s": "Any",
            "STORMSHIELDDEFENSETYPE_i": 0,
            "HOTFIXVERSION_i": 0,
            "PLAYLISTNAME_s": s.playlist,
            "SESSIONKEY_s": s.session_key,
            "TENANT_s": "Fortnite",
            "BEACONPORT_i": 15009,
        },
        "publicPlayers": s.public_players,
        "privatePlayers": s.private_players,
        "totalPlayers": s.players.len(),
        "allowJoinInProgress": false,
        "shouldAdvertise": false,
        "isDedicated": false,
        "usesStats": false,
        "allowInvites": false,
        "usesPresence": false,
        "allowJoinViaPresence": true,
        "allowJoinViaPresenceFriendsOnly": false,
        "buildUniqueId": s.net_cl,
        "lastUpdated": s.last_updated,
        "started": s.started,
    });
    if let Some(a) = v["attributes"].as_object_mut() {
        for (k, x) in &s.extra_attributes {
            a.insert(k.clone(), x.clone());
        }
    }
    v
}

fn ids(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| e.as_str().map(str::to_string).or_else(|| e.get("accountId").or_else(|| e.get("id")).and_then(Value::as_str).map(str::to_string)))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

const CORE_ATTRIBUTES: &[&str] = &["REGION_s", "SUBREGION_s", "PLAYLISTNAME_s", "SESSIONKEY_s", "BEACONPORT_i"];

fn apply_body(s: &mut Sess, b: &Value) {
    if let Some(v) = b.get("serverAddress").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        s.server_address = v.to_string();
    }
    if let Some(p) = b.get("serverPort").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).filter(|p| *p != 0) {
        s.server_port = p;
    }
    if let Some(v) = b.get("ownerName").or_else(|| b.get("serverName")).and_then(Value::as_str).filter(|v| !v.is_empty()) {
        s.owner_name = v.to_string();
    }
    if let Some(v) = b.get("ownerId").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        s.owner_id = v.to_string();
    }
    if let Some(v) = b.get("buildUniqueId").and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_u64().map(|n| n.to_string()))) {
        s.net_cl = v;
    }
    if let Some(n) = b.get("maxPublicPlayers").and_then(Value::as_u64) {
        s.max_public = n as usize;
    }
    if let Some(n) = b.get("maxPrivatePlayers").and_then(Value::as_u64) {
        s.max_private = n as usize;
    }
    if let Some(a) = b.get("attributes").and_then(Value::as_object) {
        for (k, v) in a {
            match (k.as_str(), v.as_str()) {
                ("REGION_s", Some(x)) => s.region = x.to_string(),
                ("SUBREGION_s", Some(x)) => s.subregion = x.to_string(),
                ("PLAYLISTNAME_s", Some(x)) => s.playlist = x.to_string(),
                ("SESSIONKEY_s", Some(x)) => s.session_key = x.to_string(),
                _ if CORE_ATTRIBUTES.contains(&k.as_str()) => {}
                _ => {
                    s.extra_attributes.insert(k.clone(), v.clone());
                }
            }
        }
    }
    if b.get("publicPlayers").is_some() {
        s.public_players = ids(b.get("publicPlayers"));
    }
    if b.get("privatePlayers").is_some() {
        s.private_players = ids(b.get("privatePlayers"));
    }
    if let Some(v) = b.get("started").and_then(Value::as_bool) {
        s.started = v;
    }
    s.last_updated = util::iso_now();
}

fn host_create(req: &Request) -> Response {
    let b = req.json();
    if !b.is_object() {
        return epic_error(400, "errors.com.epicgames.common.json_mapping_error", "session body must be a JSON object", 1019, &[]);
    }
    let (ip, port) = gameserver();
    let owner = util::random_hex32().to_uppercase();
    let region = super::matchmaking::local_region();
    let mut s = Sess {
        id: util::random_hex32(),
        match_id: util::random_hex32(),
        owner_name: format!("[DS]fortnite-live{}-{}", region.to_ascii_lowercase(), &owner[..12].to_ascii_lowercase()),
        owner_id: owner,
        playlist: String::new(),
        subregion: region.clone(),
        region,
        net_cl: String::new(),
        server_address: ip,
        server_port: port,
        key: random_key(),
        session_key: util::random_hex32().to_uppercase(),
        players: Vec::new(),
        public_players: Vec::new(),
        private_players: Vec::new(),
        extra_attributes: Map::new(),
        max_public: MAX_PUBLIC_PLAYERS,
        max_private: 0,
        started: false,
        last_updated: util::iso_now(),
    };
    apply_body(&mut s, &b);
    note("session", &format!("host create {} playlist {} server {}:{}", s.id, s.playlist, s.server_address, s.server_port));
    let out = session_json(&s);
    sessions().lock().unwrap_or_else(|p| p.into_inner()).insert(s.id.clone(), s);
    Response::json(&out)
}

fn host_update(req: &Request, id: &str, what: &str) -> Response {
    let b = req.json();
    if (what == "players" || what == "update") && !b.is_object() {
        return epic_error(400, "errors.com.epicgames.common.json_mapping_error", &format!("{what} body must be a JSON object"), 1019, &[]);
    }
    let mut g = sessions().lock().unwrap_or_else(|p| p.into_inner());
    let Some(s) = g.get_mut(id) else { return not_found(id) };
    match what {
        "start" => {
            s.started = true;
            s.last_updated = util::iso_now();
        }
        "stop" => {
            s.started = false;
            s.last_updated = util::iso_now();
        }
        "heartbeat" => s.last_updated = util::iso_now(),
        _ => apply_body(s, &b),
    }
    note("session", &format!("{what} {id} started={} public={} private={}", s.started, s.public_players.len(), s.private_players.len()));
    if what == "update" {
        return Response::json(&session_json(s));
    }
    Response::empty(204)
}

fn not_found(id: &str) -> Response {
    epic_error(404, "errors.com.epicgames.fortnite.session_not_found", &format!("Sorry, we couldn't find a session for id {id}"), 16101, &[id])
}

fn matches(s: &Sess, criteria: &Value) -> bool {
    let attrs = session_json(s)["attributes"].clone();
    criteria.as_array().map_or(true, |list| list.iter().all(|c| {
        let key = c.get("key").and_then(Value::as_str).unwrap_or("");
        let ty = c.get("type").and_then(Value::as_str).unwrap_or("EQUAL");
        let want = c.get("value").cloned().unwrap_or(Value::Null);
        let have = attrs.get(key).cloned().unwrap_or(Value::Null);
        let eq = match (&have, &want) {
            (Value::String(a), Value::String(b)) => a.eq_ignore_ascii_case(b),
            (a, b) => a == b,
        };
        match ty {
            "NOT_EQUAL" => !eq,
            _ => eq,
        }
    }))
}

pub fn route(fnt: &Fortnite, req: &Request, m: &str, seg: &[&str]) -> Option<Response> {
    let _ = fnt;
    Some(match (m, seg) {
        ("GET", ["fortnite", "api", "matchmaking", "session", "findPlayer", acct]) => {
            let g = sessions().lock().unwrap_or_else(|p| p.into_inner());
            Response::json(&Value::Array(g.values().filter(|s| s.players.iter().any(|p| p.eq_ignore_ascii_case(acct))).map(session_json).collect()))
        }
        ("POST", ["fortnite", "api", "matchmaking", "session", "matchMakingRequest"]) => {
            capture("session", req);
            let body = req.json();
            let criteria = body.get("criteria").cloned().unwrap_or(Value::Null);
            let max = body.get("maxResults").and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
            let g = sessions().lock().unwrap_or_else(|p| p.into_inner());
            let mut v: Vec<&Sess> = g.values().filter(|s| !s.started && matches(s, &criteria)).collect();
            v.sort_by(|a, b| a.last_updated.cmp(&b.last_updated));
            Response::json(&Value::Array(v.into_iter().take(max).map(session_json).collect()))
        }
        ("POST", ["fortnite", "api", "matchmaking", "session", id, "join"]) => {
            capture("session", req);
            let mut g = sessions().lock().unwrap_or_else(|p| p.into_inner());
            let Some(s) = g.get_mut(*id) else { return Some(not_found(id)) };
            let Some(acct) = req.query("accountId").map(str::to_string).filter(|a| !a.is_empty()) else {
                return Some(epic_error(400, "errors.com.epicgames.common.missing_parameter", "accountId is required", 1040, &["accountId"]));
            };
            if !s.players.iter().any(|p| p.eq_ignore_ascii_case(&acct)) {
                if s.players.len() >= MAX_PUBLIC_PLAYERS {
                    return Some(epic_error(409, "errors.com.epicgames.fortnite.session_full", &format!("session {id} is full"), 16104, &[id]));
                }
                s.players.push(acct.clone());
            }
            s.last_updated = util::iso_now();
            note("session", &format!("join {id} {acct} -> {} players", s.players.len()));
            Response::empty(204)
        }
        ("POST", ["fortnite", "api", "matchmaking", "session"]) => {
            capture("session", req);
            host_create(req)
        }
        ("PUT", ["fortnite", "api", "matchmaking", "session", id]) => {
            capture("session", req);
            host_update(req, id, "update")
        }
        ("POST", ["fortnite", "api", "matchmaking", "session", id, what @ ("heartbeat" | "players" | "start" | "stop")]) => {
            capture("session", req);
            host_update(req, id, what)
        }
        ("POST", ["fortnite", "api", "matchmaking", "session", id, "invite"]) => {
            capture("session", req);
            match get(id) {
                Some(_) => Response::empty(204),
                None => not_found(id),
            }
        }
        ("DELETE", ["fortnite", "api", "matchmaking", "session", id]) => {
            capture("session", req);
            match sessions().lock().unwrap_or_else(|p| p.into_inner()).remove(*id) {
                Some(_) => {
                    note("session", &format!("delete {id}"));
                    Response::empty(204)
                }
                None => not_found(id),
            }
        }
        ("GET", ["fortnite", "api", "matchmaking", "session", id]) => match get(id) {
            Some(s) => Response::json(&session_json(&s)),
            None => not_found(id),
        },
        ("GET", ["fortnite", "api", "game", "v2", "matchmaking", "account", acct, "session", sid]) => match get(sid) {
            Some(s) if s.players.iter().any(|p| p.eq_ignore_ascii_case(acct)) => Response::json(&json!({"accountId": acct, "sessionId": sid, "key": s.key})),
            Some(_) => epic_error(404, "errors.com.epicgames.fortnite.player_not_in_session", &format!("{acct} is not a member of session {sid}"), 16105, &[acct, sid]),
            None => not_found(sid),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn ticket(acct: &str, playlist: &str) -> Ticket {
        Ticket {
            ticket_id: util::random_hex32(),
            account_id: acct.into(),
            party_player_ids: vec![acct.into()],
            bucket_id: format!("14457890:1:NAE:{playlist}"),
            net_cl: "14457890".into(),
            playlist_revision: 1,
            region: "NAE".into(),
            playlist: playlist.into(),
            platform: "XSX".into(),
            subregions: vec!["VA".into()],
            options: Map::new(),
            issued_at: util::now_secs(),
        }
    }

    fn call(method: &str, path: &str) -> Response {
        let (p, q) = path.split_once('?').unwrap_or((path, ""));
        let r = Request { method: method.into(), path: p.into(), query: util::parse_pairs(q), headers: Vec::new(), body: Vec::new() };
        let f = Fortnite::with_account("x", "x");
        route(&f, &r, method, &r.segments()).unwrap()
    }

    #[test]
    fn session_from_ticket_carries_playlist_and_build() {
        let sid = util::random_hex32();
        create(&sid, "m1", &ticket("SeA", "playlist_defaultsquad"));
        let r = call("GET", &format!("/fortnite/api/matchmaking/session/{sid}"));
        assert_eq!(r.status, 200);
        let b: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(b["attributes"]["PLAYLISTNAME_s"], "playlist_defaultsquad");
        assert_eq!(b["buildUniqueId"], "14457890");
        assert_eq!(b["attributes"]["REGION_s"], "NAE");
        assert_eq!(b["attributes"]["SUBREGION_s"], "VA");
        assert_eq!(b["maxPublicPlayers"], 100);
        assert_eq!(b["openPublicPlayers"], 99);
        assert!(b["serverPort"].as_u64().unwrap() > 0);
        assert_eq!(call("GET", "/fortnite/api/matchmaking/session/doesnotexist").status, 404);
    }

    #[test]
    fn key_route_requires_member() {
        let sid = util::random_hex32();
        let s = create(&sid, "m2", &ticket("SeB", "playlist_defaultsolo"));
        let r = call("GET", &format!("/fortnite/api/game/v2/matchmaking/account/SeB/session/{sid}"));
        assert_eq!(r.status, 200);
        let b: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(b["key"], json!(s.key));
        assert_eq!(util::b64_decode(&s.key).len(), 32);
        assert_eq!(call("GET", &format!("/fortnite/api/game/v2/matchmaking/account/Stranger/session/{sid}")).status, 404);
        assert_eq!(call("GET", "/fortnite/api/game/v2/matchmaking/account/SeB/session/none").status, 404);
    }

    fn call_body(method: &str, path: &str, body: Value) -> Response {
        let r = Request { method: method.into(), path: path.into(), query: Vec::new(), headers: Vec::new(), body: body.to_string().into_bytes() };
        let f = Fortnite::with_account("x", "x");
        route(&f, &r, method, &r.segments()).unwrap()
    }

    #[test]
    fn host_lifecycle_start_players_heartbeat_stop_delete() {
        let sid = util::random_hex32();
        create(&sid, "m4", &ticket("SeH", "playlist_defaultsolo"));
        let base = format!("/fortnite/api/matchmaking/session/{sid}");
        assert_eq!(call("POST", &format!("{base}/start")).status, 204);
        assert!(get(&sid).unwrap().started);
        let found = call_body("POST", "/fortnite/api/matchmaking/session/matchMakingRequest", json!({"criteria": [], "maxResults": 100}));
        let b: Value = serde_json::from_slice(&found.body).unwrap();
        assert!(!b.as_array().unwrap().iter().any(|s| s["id"] == json!(sid)));
        assert_eq!(call_body("POST", &format!("{base}/players"), json!({"publicPlayers": ["SeH", {"accountId": "SeI"}], "privatePlayers": []})).status, 204);
        let s = get(&sid).unwrap();
        assert_eq!(s.public_players, vec!["SeH".to_string(), "SeI".to_string()]);
        let b: Value = serde_json::from_slice(&call("GET", &base).body).unwrap();
        assert_eq!(b["publicPlayers"], json!(["SeH", "SeI"]));
        assert_eq!(b["openPublicPlayers"], 98);
        assert_eq!(call_body("POST", &format!("{base}/players"), json!("x")).status, 400);
        let before = get(&sid).unwrap().last_updated;
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(call("POST", &format!("{base}/heartbeat")).status, 204);
        assert!(get(&sid).unwrap().last_updated >= before);
        assert_eq!(call("POST", &format!("{base}/stop")).status, 204);
        assert!(!get(&sid).unwrap().started);
        assert_eq!(call("POST", &format!("{base}/invite")).status, 204);
        assert_eq!(call("DELETE", &base).status, 204);
        assert!(get(&sid).is_none());
        for what in ["start", "stop", "heartbeat", "invite"] {
            assert_eq!(call("POST", &format!("{base}/{what}")).status, 404, "{what}");
        }
        assert_eq!(call("DELETE", &base).status, 404);
    }

    #[test]
    fn host_create_and_update_keep_attributes() {
        let r = call_body("POST", "/fortnite/api/matchmaking/session", json!({
            "ownerName": "[DS]local", "serverAddress": "127.0.0.1", "serverPort": 7777, "maxPublicPlayers": 4, "buildUniqueId": "14457890",
            "attributes": {"PLAYLISTNAME_s": "Playlist_DefaultSolo", "REGION_s": "NAE", "CUSTOM_s": "x"},
        }));
        assert_eq!(r.status, 200);
        let b: Value = serde_json::from_slice(&r.body).unwrap();
        let id = b["id"].as_str().unwrap().to_string();
        assert_eq!(b["attributes"]["PLAYLISTNAME_s"], "Playlist_DefaultSolo");
        assert_eq!(b["attributes"]["CUSTOM_s"], "x");
        assert_eq!(b["maxPublicPlayers"], 4);
        assert_eq!(b["buildUniqueId"], "14457890");
        let u = call_body("PUT", &format!("/fortnite/api/matchmaking/session/{id}"), json!({"attributes": {"PLAYLISTNAME_s": "Playlist_DefaultDuo"}, "started": true}));
        assert_eq!(u.status, 200);
        let b: Value = serde_json::from_slice(&u.body).unwrap();
        assert_eq!(b["attributes"]["PLAYLISTNAME_s"], "Playlist_DefaultDuo");
        assert_eq!(b["attributes"]["CUSTOM_s"], "x");
        assert_eq!(b["started"], true);
        assert_eq!(call_body("POST", "/fortnite/api/matchmaking/session", json!([])).status, 400);
        assert_eq!(call_body("PUT", "/fortnite/api/matchmaking/session/none", json!({})).status, 404);
    }

    #[test]
    fn join_adds_player() {
        let sid = util::random_hex32();
        create(&sid, "m3", &ticket("SeC", "playlist_defaultduo"));
        assert_eq!(call("POST", &format!("/fortnite/api/matchmaking/session/{sid}/join?accountId=SeD")).status, 204);
        assert!(get(&sid).unwrap().players.contains(&"SeD".to_string()));
        assert_eq!(call("POST", "/fortnite/api/matchmaking/session/missing/join?accountId=SeD").status, 404);
        let r = call("GET", "/fortnite/api/matchmaking/session/findPlayer/SeD");
        let b: Value = serde_json::from_slice(&r.body).unwrap();
        assert!(b.as_array().unwrap().iter().any(|s| s["id"] == json!(sid)));
    }
}
