#![cfg(tombox_backend_data)]

use compat_common::online::lan::{self, Role};
use serde_json::{json, Value};
use std::net::UdpSocket;
use std::process::Command;
use std::time::Duration;

fn lobby(api: &str, id: &str, body: Value) -> Value {
    let r = lan::invoke("playfab.lobby", &json!({"api": api, "entity": {"Id": id, "Type": "title_player_account"}, "body": body})).expect("hub call");
    assert_eq!(r["status"], 200, "{api}: {r}");
    r["body"]["data"].clone()
}

fn child() {
    let role = lan::role().clone();
    assert!(matches!(role, Role::Client { .. }), "child role {role:?}");
    if std::env::var("CHILD_DISCOVERY_ONLY").is_ok() {
        println!("CHILD-DISCOVERED {role:?}");
        return;
    }
    let found = lobby("Lobby/FindLobbies", "BBBB000000000002", json!({"Filter": "string_key1 eq 'lan'"}));
    assert_eq!(found["Lobbies"].as_array().unwrap().len(), 1, "{found}");
    let cs = std::env::var("CS").unwrap();
    let joined = lobby("Lobby/JoinLobby", "BBBB000000000002", json!({"ConnectionString": cs, "MemberEntity": {"Id": "BBBB000000000002", "Type": "title_player_account"}}));
    assert!(joined["LobbyId"].is_string());
    assert_eq!(lan::publish("lan.test", &json!({"hi": 1})), 1);
    assert!(lan::send_datagram(&std::env::var("HOST_DEVICE").unwrap(), b"ping"));
    let devs = lan::invoke("lan.devices", &json!({})).unwrap();
    assert_eq!(devs["devices"].as_array().unwrap().len(), 2, "{devs}");
    println!("CHILD-OK");
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("0.0.0.0:0").unwrap().local_addr().unwrap().port()
}

#[test]
fn two_process_lan_hub() {
    if std::env::var("TOMBOX_LAN_CHILD").is_ok() {
        child();
        return;
    }
    let dp = free_udp_port();
    std::env::set_var("TOMBOX_LAN", "host");
    std::env::set_var("TOMBOX_LAN_PORT", "0");
    std::env::set_var("TOMBOX_LAN_DISCOVERY_PORT", dp.to_string());
    let Role::Host { port } = lan::role().clone() else { panic!("parent did not become the hub: {:?}", lan::role()) };
    let pushes = lan::subscribe("lan.test");
    let dgrams = lan::datagrams();
    let created = lobby("Lobby/CreateLobby", "AAAA000000000001", json!({"MaxPlayers": 4, "AccessPolicy": "Public", "SearchData": {"string_key1": "lan"}}));
    let lobby_id = created["LobbyId"].as_str().unwrap().to_string();
    let exe = std::env::current_exe().unwrap();
    let run = |extra: &[(&str, String)]| {
        let mut c = Command::new(&exe);
        c.args(["--exact", "two_process_lan_hub", "--nocapture", "--test-threads=1"]).env("TOMBOX_LAN_CHILD", "1").env("TOMBOX_LAN", "join").env("TOMBOX_LAN_DISCOVERY_PORT", dp.to_string()).env_remove("TOMBOX_LAN_PORT").env_remove("TOMBOX_LAN_HOST");
        for (k, v) in extra {
            c.env(k, v);
        }
        c.output().unwrap()
    };
    let out = run(&[("TOMBOX_LAN_HOST", format!("127.0.0.1:{port}")), ("CS", created["ConnectionString"].as_str().unwrap().to_string()), ("HOST_DEVICE", lan::device_id().to_string())]);
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && text.contains("CHILD-OK"), "child failed:\n{text}");
    let p = pushes.recv_timeout(Duration::from_secs(3)).expect("push from the child");
    assert_eq!(p["data"]["hi"], 1);
    let (from, data) = dgrams.recv_timeout(Duration::from_secs(3)).expect("datagram from the child");
    assert_eq!(data, b"ping");
    assert!(!from.is_empty());
    let got = lobby("Lobby/GetLobby", "AAAA000000000001", json!({"LobbyId": lobby_id}));
    assert_eq!(got["Lobby"]["Members"].as_array().unwrap().len(), 2, "{got}");
    let out = run(&[("CHILD_DISCOVERY_ONLY", "1".into())]);
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && text.contains("CHILD-DISCOVERED"), "discovery child failed:\n{text}");
}
