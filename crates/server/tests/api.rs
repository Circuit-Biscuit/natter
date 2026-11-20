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
use natter_proto::{Event, Id, Registration};
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
    let data = std::env::temp_dir().join(format!("natter-test-{}", rand::random::<u64>()));
    let (nc_base, nc) = fake_nextcloud().await;
    let cfg = natter_server::Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        data: data.clone(),
        name: "Test".into(),
        registration: Registration::Invite,
        max_upload: 10 * 1024 * 1024,
        media_port: 0,
        public_ip: Some("127.0.0.1".parse().unwrap()),
        web_dir: None,
        nextcloud_url: Some(nc_base.clone()),
        nextcloud_group: Some("staff".into()),
        nextcloud_folder: "Natter".into(),
        autojoin_space: None,
    };
    let (app, _st) = natter_server::build(cfg).await.unwrap();
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
    assert_eq!(ready.spaces.len(), 1);
    assert!(ready.users.iter().any(|u| u.id == u1));
    assert!(!ready.nextcloud);

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

    // Its uploads go to Nextcloud and come back as a public link; deleting the message removes the share.
    let req = Request::post("/api/uploads")
        .header("authorization", format!("Bearer {t3}"))
        .header("content-type", "image/png")
        .header("x-filename", "dot%20one.png")
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
            .contains(&"PUT alice/Natter/Uploads/dot one.png".to_string())
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

    let _ = std::fs::remove_dir_all(data);
}
