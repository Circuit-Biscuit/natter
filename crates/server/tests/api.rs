//! End to end: real server on an ephemeral port, REST via the router, live events over a real WebSocket,
//! plus a fake Nextcloud for sign-in and file storage.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Method, Request, StatusCode};
use axum::routing::{any, delete, get, post};
use futures_util::StreamExt;
use laklak_proto::{Event, Id, Registration, perm};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tower::ServiceExt;

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req
            .header("content-type", "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next_event(ws: &mut Ws, want: impl Fn(&Event) -> bool) -> Event {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let WsMessage::Text(t) = ws.next().await.expect("socket open").expect("frame") {
                let ev: Event = serde_json::from_str(t.as_str()).expect("event json");
                if want(&ev) {
                    return ev;
                }
            }
        }
    })
    .await
    .expect("event in time")
}

/// Just enough Nextcloud: login flow v2, cloud/user, WebDAV MKCOL/PUT, shares, app password delete.
#[derive(Default)]
struct FakeNc {
    polls: Mutex<u32>,
    log: Mutex<Vec<String>>,
}

async fn fake_nextcloud() -> (String, Arc<FakeNc>) {
    let nc = Arc::new(FakeNc::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let b = base.clone();
    let app = Router::new()
        .route("/index.php/login/v2", post(move || async move { axum::Json(json!({"poll": {"token": "tok", "endpoint": "x"}, "login": format!("{b}/login")})) }))
        .route(
            "/index.php/login/v2/poll",
            post(|State(nc): State<Arc<FakeNc>>| async move {
                let mut n = nc.polls.lock().unwrap();
                *n += 1;
                if *n == 1 {
                    return (StatusCode::NOT_FOUND, axum::Json(json!([])));
                }
                (StatusCode::OK, axum::Json(json!({"server": "x", "loginName": "alice@example.test", "appPassword": "app-pw"})))
            }),
        )
        .route(
            "/ocs/v2.php/cloud/user",
            get(|| async { axum::Json(json!({"ocs": {"meta": {}, "data": {"id": "alice", "display-name": "Alice K", "groups": ["staff"]}}})) }),
        )
        .route("/index.php/avatar/{id}/{size}", get(|| async { StatusCode::NOT_FOUND }))
        .route(
            "/remote.php/dav/files/{*path}",
            any(|State(nc): State<Arc<FakeNc>>, method: Method, Path(path): Path<String>| async move {
                nc.log.lock().unwrap().push(format!("{method} {path}"));
                StatusCode::CREATED
            }),
        )
        .route(
            "/ocs/v2.php/apps/files_sharing/api/v1/shares",
            get(|| async { axum::Json(json!({"ocs": {"meta": {}, "data": []}})) }).post({
                let b = base.clone();
                move || async move { axum::Json(json!({"ocs": {"meta": {}, "data": {"id": 7, "url": format!("{b}/s/abc")}}})) }
            }),
        )
        .route(
            "/ocs/v2.php/apps/files_sharing/api/v1/shares/{id}",
            delete(|State(nc): State<Arc<FakeNc>>, Path(id): Path<String>| async move {
                nc.log.lock().unwrap().push(format!("UNSHARE {id}"));
                axum::Json(json!({"ocs": {"meta": {}, "data": []}}))
            }),
        )
        .route(
            "/ocs/v2.php/core/apppassword",
            delete(|State(nc): State<Arc<FakeNc>>| async move {
                nc.log.lock().unwrap().push("FORGET app-pw".into());
                axum::Json(json!({"ocs": {"meta": {}, "data": []}}))
            }),
        )
        .with_state(nc.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, nc)
}

async fn eventually(nc: &FakeNc, line: &str) {
    for _ in 0..50 {
        if nc.log.lock().unwrap().iter().any(|l| l == line) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "fake Nextcloud never saw {line:?}; log: {:?}",
        nc.log.lock().unwrap()
    );
}

#[tokio::test]
async fn full_flow() {
    let data = std::env::temp_dir().join(format!("laklak-test-{}", rand::random::<u64>()));
    let (nc_base, nc) = fake_nextcloud().await;
    let cfg = laklak_server::Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        data: data.clone(),
        name: "Test".into(),
        registration: Registration::Invite,
        max_upload: 10 * 1024 * 1024,
        media_port: 0,
        camera_kbps: 1000,
        game_utc_offset_min: 60,
        public_ip: Some("127.0.0.1".parse().unwrap()),
        web_dir: None,
        nextcloud_url: Some(nc_base.clone()),
        nextcloud_group: Some("staff".into()),
        nextcloud_folder: "Laklak".into(),
        autojoin_space: None,
        klipy_key: Some("klipy-test".into()),
        // Reset e-mails land in a folder as .eml files.
        smtp_url: Some(format!("file://{}", data.join("mail").display())),
        smtp_from: Some("Laklak <laklak@example.org>".into()),
    };
    let (app, _st) = laklak_server::build(cfg).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn({
        let app = app.clone();
        async move { axum::serve(listener, app).await.unwrap() }
    });

    // First account becomes admin.
    let (s, info) = call(&app, "GET", "/api/info", None, None).await;
    assert_eq!(
        (s, info["first_run"].as_bool()),
        (StatusCode::OK, Some(true))
    );
    assert_eq!(info["nextcloud"].as_str(), Some(nc_base.as_str()));
    assert!(info["klipy_key"].is_null(), "the GIF key is for signed-in users only");
    assert_eq!(info["camera_kbps"], 1000, "apps show the camera limit");
    assert_eq!(info["email_reset"], true, "apps offer Forgot password?");
    let (s, a) = call(
        &app,
        "POST",
        "/api/auth/register",
        None,
        Some(json!({"username": "alice", "password": "hunter2hunter2"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a}");
    let t1 = a["token"].as_str().unwrap().to_string();
    let u1 = a["user"]["id"].as_i64().unwrap();
    assert_eq!(a["user"]["admin"], json!(true));
    // Case doesn't matter, also from a Turkish keyboard (capital i = "İ").
    let (s, _) = call(&app, "POST", "/api/auth/login", None, Some(json!({"username": " ALİCE", "password": "hunter2hunter2"}))).await;
    assert_eq!(s, StatusCode::OK);

    // Space with default channels.
    let (s, space) = call(
        &app,
        "POST",
        "/api/spaces",
        Some(&t1),
        Some(json!({"name": "Circuit Biscuit"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{space}");
    let sid = space["id"].as_i64().unwrap();
    assert_eq!(space["users"].as_array().unwrap().len(), 1);
    let general: Id = space["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "general")
        .unwrap()["id"]
        .as_i64()
        .unwrap();

    // Invite-only registration.
    let (s, _) = call(
        &app,
        "POST",
        "/api/auth/register",
        None,
        Some(json!({"username": "bob", "password": "hunter2hunter2"})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, inv) = call(
        &app,
        "POST",
        &format!("/api/spaces/{sid}/invites"),
        Some(&t1),
        Some(json!({})),
    )
    .await;
    let code = inv["code"].as_str().unwrap().to_string();
    let (s, b) = call(
        &app,
        "POST",
        "/api/auth/register",
        None,
        Some(json!({"username": "bob", "password": "hunter2hunter2", "invite": code})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let t2 = b["token"].as_str().unwrap().to_string();
    let u2 = b["user"]["id"].as_i64().unwrap();
    let (_, preview) = call(&app, "GET", &format!("/api/invites/{code}"), None, None).await;
    assert_eq!(preview["member_count"], json!(2));

    // User 2 goes live.
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/gateway?token={t2}"))
        .await
        .unwrap();
    next_event(&mut ws, |e| matches!(e, Event::Hello { .. })).await;
    let Event::Ready(ready) = next_event(&mut ws, |e| matches!(e, Event::Ready(_))).await else {
        unreachable!()
    };
    assert_eq!(ready.user.id, u2);
    assert_eq!(ready.server.klipy_key.as_deref(), Some("klipy-test"));
    assert_eq!(ready.spaces.len(), 1);
    assert!(ready.users.iter().any(|u| u.id == u1));
    assert!(!ready.nextcloud);
    // A Laklak account on a server next to a Nextcloud can't browse or share Nextcloud files.
    let (s, _) = call(&app, "GET", "/api/nextcloud/files?path=/", Some(&t2), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(&app, "POST", "/api/nextcloud/share", Some(&t2), Some(json!({"path": "/Laklak/x.txt"}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // A mention, sent twice with the same nonce, lands once and pings once.
    let msg = json!({"content": format!("hi <@{u2}>"), "nonce": "n1"});
    let (s, m) = call(
        &app,
        "POST",
        &format!("/api/channels/{general}/messages"),
        Some(&t1),
        Some(msg.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{m}");
    let mid = m["id"].as_i64().unwrap();
    let Event::MessageCreate(got) = next_event(
        &mut ws,
        |e| matches!(e, Event::MessageCreate(m) if m.content.starts_with("hi")),
    )
    .await
    else {
        unreachable!()
    };
    assert_eq!((got.id, got.mentions.clone()), (mid, vec![u2]));
    let Event::ReadStateUpdate(rs) = next_event(
        &mut ws,
        |e| matches!(e, Event::ReadStateUpdate(r) if r.channel_id == general),
    )
    .await
    else {
        unreachable!()
    };
    assert_eq!(rs.mention_count, 1);
    let (_, again) = call(
        &app,
        "POST",
        &format!("/api/channels/{general}/messages"),
        Some(&t1),
        Some(msg),
    )
    .await;
    assert_eq!(again["id"].as_i64(), Some(mid));

    // Reading clears the ping.
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/channels/{general}/ack"),
        Some(&t2),
        Some(json!({"message_id": mid})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let Event::ReadStateUpdate(rs) =
        next_event(&mut ws, |e| matches!(e, Event::ReadStateUpdate(_))).await
    else {
        unreachable!()
    };
    assert_eq!((rs.last_read_id, rs.mention_count), (mid, 0));

    // Search.
    let (s, found) = call(&app, "GET", "/api/search?q=hi", Some(&t2), None).await;
    assert_eq!(s, StatusCode::OK, "{found}");
    assert!(
        found["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"].as_i64() == Some(mid))
    );

    // Reactions.
    let thumbs = "/reactions/%F0%9F%91%8D";
    let (s, _) = call(
        &app,
        "PUT",
        &format!("/api/channels/{general}/messages/{mid}{thumbs}"),
        Some(&t2),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    next_event(&mut ws, |e| matches!(e, Event::ReactionAdd { emoji, user_id, .. } if emoji == "👍" && *user_id == u2)).await;
    let (_, list) = call(
        &app,
        "GET",
        &format!("/api/channels/{general}/messages?limit=10"),
        Some(&t2),
        None,
    )
    .await;
    let listed = list
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"].as_i64() == Some(mid))
        .unwrap()
        .clone();
    assert_eq!(listed["reactions"], json!([{"emoji": "👍", "users": [u2]}]));
    call(
        &app,
        "DELETE",
        &format!("/api/channels/{general}/messages/{mid}{thumbs}"),
        Some(&t2),
        None,
    )
    .await;
    next_event(&mut ws, |e| matches!(e, Event::ReactionRemove { .. })).await;

    // Edit: author only.
    let (s, _) = call(
        &app,
        "PATCH",
        &format!("/api/channels/{general}/messages/{mid}"),
        Some(&t2),
        Some(json!({"content": "x"})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(
        &app,
        "PATCH",
        &format!("/api/channels/{general}/messages/{mid}"),
        Some(&t1),
        Some(json!({"content": "hello all"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    next_event(&mut ws, |e| matches!(e, Event::MessageUpdate(m) if m.content == "hello all" && m.edited_at.is_some())).await;

    // A reply with "@ ON" mentions (and pings) the author it answers.
    let (s, reply) = call(
        &app,
        "POST",
        &format!("/api/channels/{general}/messages"),
        Some(&t2),
        Some(json!({"content": "yo", "reply_to": mid, "reply_ping": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{reply}");
    assert_eq!(
        (
            reply["mentions"].clone(),
            reply["reply"]["author_id"].as_i64()
        ),
        (json!([u1]), Some(u1))
    );

    // Threads: started from a message, which then carries a preview.
    let (s, th) = call(
        &app,
        "POST",
        &format!("/api/channels/{general}/threads"),
        Some(&t1),
        Some(json!({"message_id": mid})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{th}");
    let tid = th["id"].as_i64().unwrap();
    assert_eq!(
        (
            th["kind"].as_str(),
            th["name"].as_str(),
            th["thread"]["starter_id"].as_i64()
        ),
        (Some("thread"), Some("hello all"), Some(mid))
    );
    next_event(
        &mut ws,
        |e| matches!(e, Event::ChannelCreate(c) if c.id == tid),
    )
    .await;
    next_event(&mut ws, |e| matches!(e, Event::MessageUpdate(m) if m.id == mid && m.thread.as_ref().is_some_and(|t| t.id == tid))).await;
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/channels/{general}/threads"),
        Some(&t2),
        Some(json!({"message_id": mid})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/api/channels/{tid}/messages"),
        Some(&t2),
        Some(json!({"content": "in thread"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, threads) = call(
        &app,
        "GET",
        &format!("/api/channels/{general}/threads"),
        Some(&t2),
        None,
    )
    .await;
    assert_eq!(threads[0]["thread"]["message_count"], json!(1));
    // Archive: the starter or an admin, not anyone.
    let (s, _) = call(
        &app,
        "PATCH",
        &format!("/api/channels/{tid}"),
        Some(&t2),
        Some(json!({"archived": true})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(
        &app,
        "PATCH",
        &format!("/api/channels/{tid}"),
        Some(&t1),
        Some(json!({"archived": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    next_event(&mut ws, |e| matches!(e, Event::ChannelUpdate(c) if c.id == tid && c.thread.as_ref().is_some_and(|t| t.archived))).await;
    // Posting brings it back.
    call(
        &app,
        "POST",
        &format!("/api/channels/{tid}/messages"),
        Some(&t2),
        Some(json!({"content": "again"})),
    )
    .await;
    next_event(&mut ws, |e| matches!(e, Event::ChannelUpdate(c) if c.id == tid && c.thread.as_ref().is_some_and(|t| !t.archived))).await;
    let (_, threads) = call(
        &app,
        "GET",
        &format!("/api/channels/{general}/threads"),
        Some(&t2),
        None,
    )
    .await;
    assert_eq!(
        (
            threads[0]["thread"]["message_count"].clone(),
            threads[0]["thread"]["archived"].clone()
        ),
        (json!(2), json!(false))
    );

    // Delete.
    let (s, _) = call(
        &app,
        "DELETE",
        &format!("/api/channels/{general}/messages/{mid}"),
        Some(&t1),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    next_event(
        &mut ws,
        |e| matches!(e, Event::MessageDelete { id, .. } if *id == mid),
    )
    .await;

    // Roles: every space starts with an Admin role; @everyone has the defaults.
    let admin_role = space["roles"][0]["id"].as_i64().unwrap();
    assert_eq!(space["roles"][0]["name"], json!("Admin"));
    assert_eq!(space["roles"][0]["permissions"], json!(perm::ADMINISTRATOR));
    assert_eq!(space["everyone_permissions"], json!(perm::EVERYONE_DEFAULT));
    let new_channel = json!({"name": "ops", "kind": "text"});
    let (s, _) = call(&app, "POST", &format!("/api/spaces/{sid}/channels"), Some(&t2), Some(new_channel.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // A "Mods" role that can manage channels and roles; everyone hears about it.
    let (s, mods) = call(
        &app,
        "POST",
        &format!("/api/spaces/{sid}/roles"),
        Some(&t1),
        Some(json!({"name": "Mods", "color": "#23A55A", "emoji": "🛠️", "permissions": perm::MANAGE_CHANNELS | perm::MANAGE_ROLES, "hoist": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{mods}");
    assert_eq!(mods["color"], json!("#23a55a"));
    let mods = mods["id"].as_i64().unwrap();
    let Event::RolesUpdate { roles, .. } = next_event(&mut ws, |e| matches!(e, Event::RolesUpdate { .. })).await else {
        unreachable!()
    };
    assert_eq!(roles.iter().map(|r| (r.id, r.position)).collect::<Vec<_>>(), vec![(admin_role, 2), (mods, 1)]);
    let role_path = |user: Id, role: Id| format!("/api/spaces/{sid}/members/{user}/roles/{role}");
    let (s, _) = call(&app, "PUT", &role_path(u2, mods), Some(&t1), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let Event::MemberUpdate { member, .. } = next_event(&mut ws, |e| matches!(e, Event::MemberUpdate { .. })).await else {
        unreachable!()
    };
    assert_eq!((member.user_id, member.roles.clone()), (u2, vec![mods]));
    // Now the member can do what the role allows…
    let (s, _) = call(&app, "POST", &format!("/api/spaces/{sid}/channels"), Some(&t2), Some(new_channel)).await;
    assert_eq!(s, StatusCode::OK);
    // …but nothing above their own highest role, and no permissions they don't have.
    let (s, _) = call(&app, "PATCH", &format!("/api/spaces/{sid}/roles/{admin_role}"), Some(&t2), Some(json!({"name": "Mine"}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(&app, "PUT", &role_path(u2, admin_role), Some(&t2), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(&app, "POST", &format!("/api/spaces/{sid}/roles"), Some(&t2), Some(json!({"name": "Root", "permissions": perm::ADMINISTRATOR}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, helpers) = call(&app, "POST", &format!("/api/spaces/{sid}/roles"), Some(&t2), Some(json!({"name": "Helpers"}))).await;
    assert_eq!(s, StatusCode::OK);
    let helpers = helpers["id"].as_i64().unwrap();
    let reorder = |ids: Vec<Id>| Some(json!({ "ids": ids }));
    let (s, _) = call(&app, "PUT", &format!("/api/spaces/{sid}/roles"), Some(&t2), reorder(vec![admin_role, helpers, mods])).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "can't lift a role above your own");
    let (s, _) = call(&app, "PUT", &format!("/api/spaces/{sid}/roles"), Some(&t1), reorder(vec![admin_role, helpers, mods])).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "the owner can");
    // @everyone without Send messages can't post.
    let without_send = perm::EVERYONE_DEFAULT & !perm::SEND_MESSAGES;
    let (s, _) = call(&app, "PATCH", &format!("/api/spaces/{sid}"), Some(&t1), Some(json!({"everyone_permissions": without_send}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let post = || Some(json!({"content": "still here?"}));
    let (s, _) = call(&app, "POST", &format!("/api/channels/{general}/messages"), Some(&t2), post()).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(&app, "PATCH", &format!("/api/spaces/{sid}"), Some(&t1), Some(json!({"everyone_permissions": perm::EVERYONE_DEFAULT}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // Making someone admin is giving them the Admin role; older apps see role "admin".
    let (s, _) = call(&app, "PUT", &role_path(u2, admin_role), Some(&t1), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let Event::MemberUpdate { member, .. } = next_event(&mut ws, |e| matches!(e, Event::MemberUpdate { member, .. } if member.roles.contains(&admin_role))).await else {
        unreachable!()
    };
    assert_eq!(serde_json::to_value(member.role).unwrap(), json!("admin"));
    let (s, _) = call(&app, "DELETE", &format!("/api/spaces/{sid}/roles/{helpers}"), Some(&t2), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "admins manage the roles below theirs");
    let (s, _) = call(&app, "PATCH", &format!("/api/spaces/{sid}/members/{u2}"), Some(&t1), Some(json!({"role": "member"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "the old role field is gone");
    let (s, _) = call(&app, "DELETE", &role_path(u2, admin_role), Some(&t1), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Private channels: hidden from members without access, everywhere.
    let (s, _) = call(&app, "DELETE", &role_path(u2, mods), Some(&t1), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let hide = json!({"id": sid, "kind": "role", "allow": 0, "deny": perm::VIEW_CHANNEL});
    let (s, secret) = call(&app, "POST", &format!("/api/spaces/{sid}/channels"), Some(&t1),
        Some(json!({"name": "secret", "kind": "text", "overwrites": [hide.clone()]}))).await;
    assert_eq!(s, StatusCode::OK, "{secret}");
    let secret = secret["id"].as_i64().unwrap();
    let msgs = |ch: Id| format!("/api/channels/{ch}/messages");
    let (s, _) = call(&app, "GET", &msgs(secret), Some(&t2), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a private channel looks like no channel");
    let (s, _) = call(&app, "POST", &msgs(secret), Some(&t2), Some(json!({"content": "let me in"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // Its messages never reach them: the next message they hear about is the one in #general.
    call(&app, "POST", &msgs(secret), Some(&t1), Some(json!({"content": "classified plans @everyone"}))).await;
    call(&app, "POST", &msgs(general), Some(&t1), Some(json!({"content": "public note"}))).await;
    let Event::MessageCreate(m) = next_event(&mut ws, |e| matches!(e, Event::MessageCreate(_))).await else { unreachable!() };
    assert_eq!((m.channel_id, m.content.as_str()), (general, "public note"));
    let found = |t: &str| {
        let t = t.to_string();
        let app = app.clone();
        async move { call(&app, "GET", "/api/search?q=classified", Some(&t), None).await.1["total"].as_i64() }
    };
    assert_eq!((found(&t1).await, found(&t2).await), (Some(1), Some(0)), "search skips what you can't see");
    // A member without Manage roles can't change who sees it.
    let me_in = json!({"kind": "member", "allow": perm::VIEW_CHANNEL, "deny": 0});
    let (s, _) = call(&app, "PUT", &format!("/api/channels/{secret}/permissions/{u2}"), Some(&t2), Some(me_in.clone())).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "they can't even see it");
    // Letting them in sends them the space again, now with the channel; taking it back removes it.
    let (s, _) = call(&app, "PUT", &format!("/api/channels/{secret}/permissions/{u2}"), Some(&t1), Some(me_in)).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let Event::SpaceCreate(seen) = next_event(&mut ws, |e| matches!(e, Event::SpaceCreate(_))).await else { unreachable!() };
    assert!(seen.channels.iter().any(|c| c.id == secret));
    let (s, list) = call(&app, "GET", &msgs(secret), Some(&t2), None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let (s, _) = call(&app, "DELETE", &format!("/api/channels/{secret}/permissions/{u2}"), Some(&t1), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let Event::SpaceCreate(seen) = next_event(&mut ws, |e| matches!(e, Event::SpaceCreate(_))).await else { unreachable!() };
    assert!(!seen.channels.iter().any(|c| c.id == secret));
    // A private category hides the channels in it.
    let (_, cat) = call(&app, "POST", &format!("/api/spaces/{sid}/channels"), Some(&t1),
        Some(json!({"name": "Staff", "kind": "category", "overwrites": [hide]}))).await;
    let (_, inside) = call(&app, "POST", &format!("/api/spaces/{sid}/channels"), Some(&t1),
        Some(json!({"name": "payroll", "kind": "text", "parent_id": cat["id"]}))).await;
    let (s, _) = call(&app, "GET", &msgs(inside["id"].as_i64().unwrap()), Some(&t2), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, "GET", &msgs(inside["id"].as_i64().unwrap()), Some(&t1), None).await;
    assert_eq!(s, StatusCode::OK, "the owner sees everything");

    // Daily Word: guesses are checked on the server; the result posts (spoiler-free) to the space's game channel.
    let (s, _) = call(&app, "PATCH", &format!("/api/spaces/{sid}"), Some(&t1), Some(json!({"game_channel_id": general}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let guess = |w: &str| Some(json!({"lang": "en", "word": w, "space_id": sid}));
    let (s, e) = call(&app, "POST", "/api/games/word/guess", Some(&t1), guess("qzxvj")).await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::BAD_REQUEST, Some("not in the word list")));
    let (_, ov) = call(&app, "GET", &format!("/api/games/word?lang=en&space_id={sid}"), Some(&t1), None).await;
    assert_eq!((ov["today"]["status"].as_str(), ov["today"]["answer"].is_null()), (Some("playing"), true), "{ov}");
    let mut last = Value::Null;
    for w in ["house", "water", "light", "music", "plant", "table"] {
        let (s, ov) = call(&app, "POST", "/api/games/word/guess", Some(&t1), guess(w)).await;
        assert_eq!(s, StatusCode::OK, "{ov}");
        assert_eq!(ov["today"]["guesses"].as_array().unwrap().last().unwrap()["marks"].as_array().unwrap().len(), 5);
        last = ov;
        if last["today"]["status"] != "playing" {
            break;
        }
    }
    assert_ne!(last["today"]["status"], "playing");
    assert!(last["today"]["answer"].as_str().is_some_and(|a| a.len() == 5), "the answer once you've finished");
    let (s, _) = call(&app, "POST", "/api/games/word/guess", Some(&t1), guess("house")).await;
    assert_eq!(s, StatusCode::CONFLICT, "one game a day");
    let Event::MessageCreate(result) = next_event(&mut ws, |e| matches!(e, Event::MessageCreate(m) if m.content.starts_with("Daily Word"))).await else {
        unreachable!()
    };
    assert_eq!((result.channel_id, result.author_id), (general, u1));
    assert!(result.content.contains("🟩") || result.content.contains("⬛"), "{}", result.content);
    let grid = result.content.split_once('\n').map_or("", |(_, g)| g).to_lowercase();
    assert!(!grid.contains(last["today"]["answer"].as_str().unwrap()), "no spoilers");
    assert_eq!(last["players"][0]["user_id"], json!(u1));
    assert_eq!(last["all_time"][0]["user_id"], json!(u1));

    // Space profile: your own nickname and status in one space; nobody else sets your status.
    let me2 = format!("/api/spaces/{sid}/members/{u2}");
    let (s, _) = call(&app, "PATCH", &me2, Some(&t2), Some(json!({"nickname": "Bee", "custom_status": "On holiday"}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let Event::MemberUpdate { member, .. } = next_event(&mut ws, |e| matches!(e, Event::MemberUpdate { member, .. } if member.user_id == u2)).await else {
        unreachable!()
    };
    assert_eq!((member.nickname.as_deref(), member.custom_status.as_deref()), (Some("Bee"), Some("On holiday")));
    let (s, _) = call(&app, "PATCH", &me2, Some(&t1), Some(json!({"custom_status": "Fired"}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(&app, "PATCH", &me2, Some(&t2), Some(json!({"custom_status": ""}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let Event::MemberUpdate { member, .. } = next_event(&mut ws, |e| matches!(e, Event::MemberUpdate { member, .. } if member.user_id == u2)).await else {
        unreachable!()
    };
    assert_eq!((member.nickname.as_deref(), member.custom_status), (Some("Bee"), None), "back to their usual status");

    // Local upload, served back with safe headers.
    let png: &[u8] =
        b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x02\0\0\0\x03\x08\x06\0\0\0\x9d\x07\xab\x0f";
    let req = Request::post("/api/uploads")
        .header("authorization", format!("Bearer {t1}"))
        .header("content-type", "image/png")
        .header("x-filename", "dot.png")
        .body(Body::from(png))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let att: Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (att["width"].as_i64(), att["height"].as_i64()),
        (Some(2), Some(3))
    );
    let res = app
        .clone()
        .oneshot(
            Request::get(att["url"].as_str().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "image/png");
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");

    // Calls: a str0m peer offers mic + data channel; the SFU answers and everyone sees the voice state.
    let voice: Id = space["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "voice")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let mut peer = str0m::Rtc::builder().build(std::time::Instant::now());
    let mut change = peer.sdp_api();
    let mic = change.add_media(
        str0m::media::MediaKind::Audio,
        str0m::media::Direction::SendOnly,
        None,
        None,
        None,
    );
    change.add_channel("signal".into());
    let (offer, pending) = change.apply().unwrap();
    let join = json!({"sdp": offer.to_sdp_string(), "tracks": {mic.to_string(): "mic"}, "muted": false, "deafened": false});
    let (s, ans) = call(
        &app,
        "POST",
        &format!("/api/channels/{voice}/voice"),
        Some(&t1),
        Some(join),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{ans}");
    let answer = str0m::change::SdpAnswer::from_sdp_string(ans["sdp"].as_str().unwrap()).unwrap();
    peer.sdp_api().accept_answer(pending, answer).unwrap();
    next_event(
        &mut ws,
        |e| matches!(e, Event::VoiceStateUpdate(v) if v.user_id == u1 && v.channel_id == voice),
    )
    .await;
    call(
        &app,
        "PATCH",
        "/api/voice",
        Some(&t1),
        Some(json!({"muted": true})),
    )
    .await;
    next_event(
        &mut ws,
        |e| matches!(e, Event::VoiceStateUpdate(v) if v.muted),
    )
    .await;
    call(&app, "DELETE", "/api/voice", Some(&t1), None).await;
    next_event(
        &mut ws,
        |e| matches!(e, Event::VoiceLeave { user_id, .. } if *user_id == u1),
    )
    .await;

    // Sign in with Nextcloud: pending, then approved; the username "alice" is taken so it gets a suffix.
    let (s, start) = call(&app, "POST", "/api/auth/nextcloud", None, None).await;
    assert_eq!(s, StatusCode::OK, "{start}");
    let poll = json!({"flow": start["flow"], "device": "test"});
    let (_, pending) = call(
        &app,
        "POST",
        "/api/auth/nextcloud/poll",
        None,
        Some(poll.clone()),
    )
    .await;
    assert_eq!(pending, Value::Null);
    let (s, nca) = call(&app, "POST", "/api/auth/nextcloud/poll", None, Some(poll)).await;
    assert_eq!(s, StatusCode::OK, "{nca}");
    assert_eq!(
        (
            nca["user"]["username"].as_str(),
            nca["user"]["display_name"].as_str()
        ),
        (Some("alice2"), Some("Alice K"))
    );
    let t3 = nca["token"].as_str().unwrap().to_string();

    // Avatars and space icons (uploaded without a channel) stay on the server, even for Nextcloud sessions.
    let req = Request::post("/api/uploads")
        .header("authorization", format!("Bearer {t3}"))
        .header("content-type", "image/png")
        .header("x-filename", "me.png")
        .body(Body::from(png))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let local: Value = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert!(local["url"].as_str().unwrap().starts_with("/files/"), "{local}");
    assert!(local["link"].is_null());

    // Its attachments go to Nextcloud and come back as a public link; deleting the message removes the share.
    let req = Request::post("/api/uploads")
        .header("authorization", format!("Bearer {t3}"))
        .header("content-type", "image/png")
        .header("x-filename", "dot%20one.png")
        .header("x-channel-id", general.to_string())
        .body(Body::from(png))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let att: Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        att["url"].as_str().unwrap(),
        format!("{nc_base}/s/abc/download/dot%20one.png")
    );
    assert_eq!(att["link"].as_str().unwrap(), format!("{nc_base}/s/abc"));
    assert!(
        nc.log
            .lock()
            .unwrap()
            .contains(&"PUT alice/Laklak/Uploads/dot one.png".to_string())
    );
    let (_, dm) = call(
        &app,
        "POST",
        "/api/dms",
        Some(&t3),
        Some(json!({"recipients": [u1]})),
    )
    .await;
    assert_eq!(
        dm.get("error").map(|_| ()),
        Some(()),
        "strangers can't be DMed: {dm}"
    );
    let (_, inv) = call(
        &app,
        "POST",
        &format!("/api/spaces/{sid}/invites"),
        Some(&t1),
        Some(json!({})),
    )
    .await;
    call(
        &app,
        "POST",
        &format!("/api/invites/{}", inv["code"].as_str().unwrap()),
        Some(&t3),
        None,
    )
    .await;
    let (s, m) = call(
        &app,
        "POST",
        &format!("/api/channels/{general}/messages"),
        Some(&t3),
        Some(json!({"content": "", "attachments": [att["id"]]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{m}");
    let (s, _) = call(
        &app,
        "DELETE",
        &format!("/api/channels/{general}/messages/{}", m["id"]),
        Some(&t3),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    eventually(&nc, "UNSHARE 7").await;

    // Signing out of a Nextcloud session drops its app password there.
    call(&app, "POST", "/api/auth/logout", Some(&t3), None).await;
    eventually(&nc, "FORGET app-pw").await;
    let (s, _) = call(&app, "GET", "/api/me/sessions", Some(&t3), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Deleting your account, Nextcloud first: sign in again, share a file, then a confirm is enough.
    let nc_sign_in = || async {
        let (_, start) = call(&app, "POST", "/api/auth/nextcloud", None, None).await;
        let (s, a) = call(&app, "POST", "/api/auth/nextcloud/poll", None, Some(json!({"flow": start["flow"], "device": "test"}))).await;
        assert_eq!(s, StatusCode::OK, "{a}");
        a
    };
    let nca = nc_sign_in().await;
    let (t4, u3) = (nca["token"].as_str().unwrap().to_string(), nca["user"]["id"].as_i64().unwrap());
    let req = Request::post("/api/uploads")
        .header("authorization", format!("Bearer {t4}"))
        .header("content-type", "image/png")
        .header("x-filename", "bye.png")
        .header("x-channel-id", general.to_string())
        .body(Body::from(png))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let att: Value = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
    let (s, shared) = call(&app, "POST", &msgs(general), Some(&t4), Some(json!({"content": "", "attachments": [att["id"]]}))).await;
    assert_eq!(s, StatusCode::OK, "{shared}");
    nc.log.lock().unwrap().clear();
    let (s, _) = call(&app, "DELETE", "/api/me", Some(&t4), Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "nothing happens without the confirm");
    let (s, _) = call(&app, "DELETE", "/api/me", Some(&t4), Some(json!({"confirm": true}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(nc.log.lock().unwrap().contains(&"UNSHARE 7".to_string()), "links go while the app password still works");
    eventually(&nc, "FORGET app-pw").await;
    let (s, _) = call(&app, "GET", "/api/me/sessions", Some(&t4), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Others see the message go, the member leave, and the name turn into "Deleted User".
    next_event(&mut ws, |e| matches!(e, Event::MessageDelete { id, .. } if Some(*id) == shared["id"].as_i64())).await;
    next_event(&mut ws, |e| matches!(e, Event::MemberRemove { user_id, .. } if *user_id == u3)).await;
    let Event::UserUpdate(gone) = next_event(&mut ws, |e| matches!(e, Event::UserUpdate(u) if u.id == u3)).await else {
        unreachable!()
    };
    assert_eq!((gone.display_name.as_str(), gone.avatar, gone.about.as_str()), ("Deleted User", None, ""));
    // The Nextcloud login is let go: signing in again makes a new account (and "alice2" is free again).
    let again = nc_sign_in().await;
    assert_ne!(again["user"]["id"].as_i64(), Some(u3));
    assert_eq!(again["user"]["username"].as_str(), Some("alice2"));

    // A local account confirms with its password, and nobody can leave their spaces without an owner.
    let delete_me = |t: &str, pw: &str| {
        let (t, pw, app) = (t.to_string(), pw.to_string(), app.clone());
        async move { call(&app, "DELETE", "/api/me", Some(&t), Some(json!({"password": pw}))).await }
    };
    let (s, e) = delete_me(&t1, "hunter2hunter2").await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::CONFLICT, Some("transfer or delete your spaces first")));
    let (s, _) = delete_me(&t2, "not-my-password").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (mut ws1, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/gateway?token={t1}")).await.unwrap();
    next_event(&mut ws1, |e| matches!(e, Event::Ready(_))).await;
    let (_, bye) = call(&app, "POST", &msgs(general), Some(&t1), Some(json!({"content": "bye bob"}))).await;
    let bye = bye["id"].as_i64().unwrap();
    let (s, _) = call(&app, "PUT", &format!("/api/channels/{general}/messages/{bye}{thumbs}"), Some(&t2), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = delete_me(&t2, "hunter2hunter2").await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    next_event(&mut ws, |e| matches!(e, Event::InvalidSession)).await;
    next_event(&mut ws1, |e| matches!(e, Event::MemberRemove { space_id, user_id } if *space_id == sid && *user_id == u2)).await;
    next_event(&mut ws1, |e| matches!(e, Event::UserUpdate(u) if u.id == u2 && u.display_name == "Deleted User")).await;
    // Their messages (threads too) and reactions are gone, and the password no longer signs in.
    let (_, list) = call(&app, "GET", &format!("{}?limit=100", msgs(general)), Some(&t1), None).await;
    let list = list.as_array().unwrap();
    assert!(list.iter().all(|m| m["author_id"].as_i64() != Some(u2)), "{list:?}");
    assert_eq!(list.iter().find(|m| m["id"].as_i64() == Some(bye)).unwrap()["reactions"], json!([]));
    let (_, in_thread) = call(&app, "GET", &msgs(tid), Some(&t1), None).await;
    assert_eq!(in_thread, json!([]));
    let (s, _) = call(&app, "POST", "/api/auth/login", None, Some(json!({"username": "bob", "password": "hunter2hunter2"}))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Lost password: a server admin gives a local account a temporary one, and it's signed out everywhere.
    let (s, c) = call(&app, "POST", "/api/auth/register", None, Some(json!({"username": "carol", "password": "hunter2hunter2", "invite": code}))).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    let (t5, u5) = (c["token"].as_str().unwrap().to_string(), c["user"]["id"].as_i64().unwrap());
    let reset = |t: &str, u: i64| {
        let (t, app) = (t.to_string(), app.clone());
        async move { call(&app, "POST", &format!("/api/users/{u}/password"), Some(&t), None).await }
    };
    assert_eq!(reset(&t5, u1).await.0, StatusCode::FORBIDDEN, "server admins only");
    let (s, r) = reset(&t1, u5).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let temp = r["password"].as_str().unwrap().to_string();
    assert_eq!(temp.len(), 14, "{temp}");
    let (s, _) = call(&app, "GET", "/api/me/sessions", Some(&t5), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let login = |pw: &str| {
        let (pw, app) = (pw.to_string(), app.clone());
        async move { call(&app, "POST", "/api/auth/login", None, Some(json!({"username": "carol", "password": pw}))).await.0 }
    };
    assert_eq!(login("hunter2hunter2").await, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&temp).await, StatusCode::OK);
    // Not yourself, not a Nextcloud account, not a deleted one.
    assert_eq!(reset(&t1, u1).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(reset(&t1, again["user"]["id"].as_i64().unwrap()).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(reset(&t1, u2).await.0, StatusCode::NOT_FOUND);

    // Forgot password: an e-mail address (confirmed with the password) gets a code that resets it.
    let (_, c) = call(&app, "POST", "/api/auth/login", None, Some(json!({"username": "carol", "password": temp}))).await;
    let t6 = c["token"].as_str().unwrap().to_string();
    let set_email = |pw: &str| {
        let (pw, t, app) = (pw.to_string(), t6.clone(), app.clone());
        async move { call(&app, "PUT", "/api/me/email", Some(&t), Some(json!({"email": " Carol@Example.org", "password": pw}))).await.0 }
    };
    assert_eq!(set_email("hunter2hunter2").await, StatusCode::FORBIDDEN);
    assert_eq!(set_email(&temp).await, StatusCode::NO_CONTENT);
    let forgot = |login: &str| {
        let (login, app) = (login.to_string(), app.clone());
        async move { call(&app, "POST", "/api/auth/forgot", None, Some(json!({"login": login}))).await.0 }
    };
    assert_eq!(forgot("nobody@example.org").await, StatusCode::NO_CONTENT, "same answer for unknown accounts");
    assert_eq!(forgot("carol@example.org").await, StatusCode::NO_CONTENT);
    let mail_dir = data.join("mail");
    let mut mail = String::new();
    for _ in 0..50 {
        if let Some(f) = std::fs::read_dir(&mail_dir).ok().and_then(|mut d| d.next()) {
            mail = std::fs::read_to_string(f.unwrap().path()).unwrap();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(mail.contains("To: carol@example.org"), "{mail}");
    let code = mail.lines().find_map(|l| l.trim().strip_prefix("Subject: Your Laklak reset code: ")).expect("code in the subject").to_string();
    assert_eq!(code.len(), 9, "{code}");
    let reset_code = |c: &str, pw: &str| {
        let (c, pw, app) = (c.to_string(), pw.to_string(), app.clone());
        async move {
            call(&app, "POST", "/api/auth/reset", None, Some(json!({"login": "Carol", "code": c, "new_password": pw}))).await
        }
    };
    assert_eq!(reset_code("AAAA-AAAA", "carols-new-one").await.0, StatusCode::BAD_REQUEST);
    let (s, r) = reset_code(&format!(" {} ", code.to_lowercase()), "carols-new-one").await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(r["token"].is_string());
    assert_eq!(reset_code(&code, "another-one-1").await.0, StatusCode::BAD_REQUEST, "a code works once");
    assert_eq!(login(&temp).await, StatusCode::UNAUTHORIZED);
    assert_eq!(login("carols-new-one").await, StatusCode::OK);
    let (s, _) = call(&app, "GET", "/api/me/sessions", Some(&t6), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "the reset signed the others out");

    // The e-mail address signs in too, in any case; a wrong password or an unknown address doesn't.
    let by_email = |e: &str, pw: &str| {
        let (e, pw, app) = (e.to_string(), pw.to_string(), app.clone());
        async move { call(&app, "POST", "/api/auth/login", None, Some(json!({"username": e, "password": pw}))).await.0 }
    };
    assert_eq!(by_email(" CAROL@example.org", "carols-new-one").await, StatusCode::OK);
    assert_eq!(by_email("carol@example.org", "not-her-password").await, StatusCode::UNAUTHORIZED);
    assert_eq!(by_email("nobody@example.org", "carols-new-one").await, StatusCode::UNAUTHORIZED);
    assert_eq!(by_email("carol@", "carols-new-one").await, StatusCode::UNAUTHORIZED);

    let _ = std::fs::remove_dir_all(data);
}
