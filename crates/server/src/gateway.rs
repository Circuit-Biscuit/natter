//! Live events: the connection hub, presence, and the `/gateway` WebSocket.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::ws::{Message as WsMsg, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use natter_proto::{Event, Id, Op, Presence, Ready, ServerInfo, Status};
use serde::Deserialize;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::{ApiErr, R, St, auth, db, nextcloud};

pub const HEARTBEAT_MS: u32 = 20_000;

enum Out {
    Text(Utf8Bytes),
    Close,
}

struct Conn {
    id: u64,
    session: Id,
    tx: UnboundedSender<Out>,
    idle: bool,
}

#[derive(Default)]
struct Inner {
    conns: HashMap<Id, Vec<Conn>>,
    /// Status each user picked.
    chosen: HashMap<Id, Status>,
    /// Presence last broadcast, to only announce changes.
    shown: HashMap<Id, Status>,
    next: u64,
}

/// Every open gateway connection, by user.
#[derive(Default)]
pub struct Hub {
    inner: Mutex<Inner>,
}

impl Hub {
    pub fn send(&self, users: &[Id], ev: &Event) {
        self.send_where(users, |_| true, ev);
    }

    pub fn send_except(&self, users: &[Id], except: Id, ev: &Event) {
        self.send_where(users, |u| u != except, ev);
    }

    fn send_where(&self, users: &[Id], keep: impl Fn(Id) -> bool, ev: &Event) {
        let text = Utf8Bytes::from(serde_json::to_string(ev).expect("events serialize"));
        let g = self.inner.lock().unwrap();
        for &u in users.iter().filter(|&&u| keep(u)) {
            for c in g.conns.get(&u).into_iter().flatten() {
                let _ = c.tx.send(Out::Text(text.clone()));
            }
        }
    }

    fn add(&self, user: Id, session: Id, tx: UnboundedSender<Out>) -> u64 {
        let mut g = self.inner.lock().unwrap();
        g.next += 1;
        let id = g.next;
        g.conns.entry(user).or_default().push(Conn {
            id,
            session,
            tx,
            idle: false,
        });
        id
    }

    fn remove(&self, user: Id, conn: u64) {
        let mut g = self.inner.lock().unwrap();
        if let Some(list) = g.conns.get_mut(&user) {
            list.retain(|c| c.id != conn);
            if list.is_empty() {
                g.conns.remove(&user);
            }
        }
    }

    fn set_idle(&self, user: Id, conn: u64, idle: bool) {
        let mut g = self.inner.lock().unwrap();
        for c in g
            .conns
            .get_mut(&user)
            .into_iter()
            .flatten()
            .filter(|c| c.id == conn)
        {
            c.idle = idle;
        }
    }

    pub fn set_chosen(&self, user: Id, status: Status) {
        self.inner.lock().unwrap().chosen.insert(user, status);
    }

    /// What others see: offline without a connection or when invisible, idle when every device is idle.
    pub fn presence(&self, user: Id) -> Status {
        Self::compute(&self.inner.lock().unwrap(), user)
    }

    fn compute(g: &Inner, user: Id) -> Status {
        let Some(conns) = g.conns.get(&user).filter(|c| !c.is_empty()) else {
            return Status::Offline;
        };
        match g.chosen.get(&user).copied().unwrap_or_default() {
            Status::Invisible | Status::Offline => Status::Offline,
            Status::Online if conns.iter().all(|c| c.idle) => Status::Idle,
            s => s,
        }
    }

    /// The new presence if it changed since it was last announced.
    fn refresh(&self, user: Id) -> Option<Status> {
        let mut g = self.inner.lock().unwrap();
        let now = Self::compute(&g, user);
        let before = g.shown.insert(user, now).unwrap_or(Status::Offline);
        (now != before).then_some(now)
    }

    /// Sign out one device: tell it, then close its connections.
    fn kick(&self, user: Id, session: Id) {
        let text = Utf8Bytes::from(serde_json::to_string(&Event::InvalidSession).unwrap());
        let mut g = self.inner.lock().unwrap();
        if let Some(list) = g.conns.get_mut(&user) {
            list.retain(|c| {
                if c.session != session {
                    return true;
                }
                let _ = c.tx.send(Out::Text(text.clone()));
                let _ = c.tx.send(Out::Close);
                false
            });
            if list.is_empty() {
                g.conns.remove(&user);
            }
        }
    }
}

pub async fn broadcast_presence(st: &St, user: Id) -> R<()> {
    if let Some(status) = st.hub.refresh(user) {
        let audience = db::user_audience(&st.db, user).await?;
        st.hub.send(
            &audience,
            &Event::PresenceUpdate(Presence {
                user_id: user,
                status,
            }),
        );
    }
    Ok(())
}

pub async fn kick_session(st: &St, user: Id, session: Id) {
    st.hub.kick(user, session);
    if let Err(e) = broadcast_presence(st, user).await {
        tracing::warn!("presence after kick: {e:?}");
    }
}

/// Everything the app needs to render, in one frame.
pub async fn ready(st: &St, user: Id, session: Id) -> R<Ready> {
    let me = db::user(&st.db, user).await?;
    let spaces = db::spaces_of(&st.db, user).await?;
    let dms = db::dms_of(&st.db, user).await?;
    let users = db::users(&st.db, &db::user_audience(&st.db, user).await?).await?;
    let presences = users
        .iter()
        .map(|u| Presence {
            user_id: u.id,
            status: st.hub.presence(u.id),
        })
        .collect();
    let visible: HashSet<Id> = spaces
        .iter()
        .flat_map(|s| s.channels.iter().map(|c| c.id))
        .chain(dms.iter().map(|c| c.id))
        .collect();
    let (status, settings): (String, String) =
        sqlx::query_as("SELECT status, settings FROM users WHERE id = ?")
            .bind(user)
            .fetch_one(&st.db)
            .await?;
    Ok(Ready {
        user: me,
        users,
        spaces,
        dms,
        read_states: db::read_states(&st.db, user).await?,
        presences,
        voice_states: st.voice.states_in(&visible),
        status: db::from_s(&status),
        settings: serde_json::from_str(&settings).unwrap_or_default(),
        server: ServerInfo {
            name: st.cfg.name.clone(),
            version: crate::VERSION.into(),
            registration: st.cfg.registration,
            first_run: false,
            max_upload_bytes: st.cfg.max_upload,
            nextcloud: st.nc.as_ref().map(|n| n.base.clone()),
            klipy_key: st.cfg.klipy_key.clone(),
            camera_kbps: st.cfg.camera_kbps,
        },
        nextcloud: nextcloud::is_nc_session(st, session).await,
    })
}

#[derive(Deserialize)]
pub struct WsQuery {
    token: String,
}

pub async fn ws(
    State(st): State<St>,
    Query(q): Query<WsQuery>,
    upgrade: WebSocketUpgrade,
) -> R<Response> {
    let auth = auth::session_for_token(&st, &q.token)
        .await?
        .ok_or_else(|| ApiErr::new(StatusCode::UNAUTHORIZED, "sign in first"))?;
    Ok(upgrade
        .max_message_size(64 * 1024)
        .on_upgrade(move |socket| run(st, auth, socket)))
}

async fn run(st: St, auth: auth::Auth, socket: WebSocket) {
    let user = auth.user;
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = unbounded_channel();

    let status: Option<String> = sqlx::query_scalar("SELECT status FROM users WHERE id = ?")
        .bind(user)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();
    st.hub
        .set_chosen(user, status.map(|s| db::from_s(&s)).unwrap_or_default());
    // Register before building Ready so nothing that happens in between is lost.
    // Events queued meanwhile are delivered after Ready; they are all idempotent upserts.
    let conn = st.hub.add(user, auth.session, tx.clone());

    let frame = |ev: &Event| WsMsg::Text(Utf8Bytes::from(serde_json::to_string(ev).unwrap()));
    let greeted = match ready(&st, user, auth.session).await {
        Ok(r) => {
            sink.send(frame(&Event::Hello {
                heartbeat_ms: HEARTBEAT_MS,
            }))
            .await
            .is_ok()
                && sink.send(frame(&Event::Ready(Box::new(r)))).await.is_ok()
        }
        Err(e) => {
            tracing::error!("ready for {user}: {e:?}");
            false
        }
    };
    if !greeted {
        st.hub.remove(user, conn);
        return;
    }
    let _ = broadcast_presence(&st, user).await;

    let mut writer = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            let msg = match out {
                Out::Text(t) => WsMsg::Text(t),
                Out::Close => {
                    let _ = sink.send(WsMsg::Close(None)).await;
                    break;
                }
            };
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    let ack = Utf8Bytes::from(serde_json::to_string(&Event::HeartbeatAck).unwrap());
    let silence = Duration::from_millis(2 * HEARTBEAT_MS as u64);
    loop {
        let msg = tokio::select! {
            m = tokio::time::timeout(silence, stream.next()) => match m {
                Ok(Some(Ok(m))) => m,
                _ => break, // closed, errored, or silent too long
            },
            _ = &mut writer => break, // kicked or the socket died
        };
        let WsMsg::Text(text) = msg else {
            if matches!(msg, WsMsg::Close(_)) {
                break;
            }
            continue;
        };
        match serde_json::from_str::<Op>(&text) {
            Ok(Op::Heartbeat) => {
                let _ = tx.send(Out::Text(ack.clone()));
            }
            Ok(Op::Typing { channel_id }) => typing(&st, user, channel_id).await,
            Ok(Op::Idle(idle)) => {
                st.hub.set_idle(user, conn, idle);
                let _ = broadcast_presence(&st, user).await;
            }
            Err(e) => tracing::debug!("bad op from {user}: {e}"),
        }
    }
    writer.abort();
    st.hub.remove(user, conn);
    let _ = broadcast_presence(&st, user).await;
}

async fn typing(st: &St, user: Id, channel_id: Id) {
    let Ok(acc) = db::access(&st.db, user, channel_id).await else {
        return;
    };
    if !acc.can_post() {
        return;
    }
    if let Ok(audience) = db::audience(&st.db, &acc.ch).await {
        st.hub.send_except(
            &audience,
            user,
            &Event::TypingStart {
                channel_id,
                user_id: user,
            },
        );
    }
}
