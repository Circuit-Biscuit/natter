//! Who is in which call, the call routes, and the bridge to the SFU thread.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use natter_proto::{
    Event, Id, MessageKind, Millis, VoiceAnswer, VoiceJoin, VoiceState, VoiceUpdate, perm,
};
use str0m::change::SdpOffer;
use str0m::media::Mid;
use str0m::{Candidate, Rtc};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::auth::Auth;
use crate::sfu::{SfuCmd, SfuEvent};
use crate::{Config, R, St, bad, db, forbidden, internal, messages, not_found, now_ms};

struct Entry {
    state: VoiceState,
    client: u64,
}

pub struct Voice {
    /// Address announced to call clients.
    pub candidate: SocketAddr,
    sfu: std::sync::mpsc::Sender<SfuCmd>,
    states: Mutex<HashMap<Id, Entry>>,
    /// DM calls in progress: channel → ("X started a call" message, start time).
    calls: Mutex<HashMap<Id, (Id, Millis)>>,
    next_client: AtomicU64,
}

/// The local address the OS would use to reach the internet. Connecting a UDP socket sends nothing.
fn outbound_ip() -> IpAddr {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("8.8.8.8:80").and_then(|_| s.local_addr()))
        .map(|a| a.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

impl Voice {
    pub fn start(cfg: &Config) -> anyhow::Result<(Voice, UnboundedReceiver<SfuEvent>)> {
        str0m::crypto::from_feature_flags().install_process_default();
        let socket = UdpSocket::bind(("0.0.0.0", cfg.media_port))?;
        let port = socket.local_addr()?.port();
        let candidate = SocketAddr::new(cfg.public_ip.unwrap_or_else(outbound_ip), port);
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let (ev_tx, ev_rx) = unbounded_channel();
        std::thread::Builder::new()
            .name("sfu".into())
            .spawn(move || crate::sfu::run(socket, candidate, cmd_rx, ev_tx))?;
        let voice = Voice {
            candidate,
            sfu: cmd_tx,
            states: Mutex::default(),
            calls: Mutex::default(),
            next_client: AtomicU64::new(1),
        };
        Ok((voice, ev_rx))
    }

    fn command(&self, cmd: SfuCmd) {
        let _ = self.sfu.send(cmd);
    }

    pub fn states_in(&self, channels: &HashSet<Id>) -> Vec<VoiceState> {
        let g = self.states.lock().unwrap();
        let mut out: Vec<VoiceState> = g
            .values()
            .filter(|e| channels.contains(&e.state.channel_id))
            .map(|e| e.state.clone())
            .collect();
        out.sort_by_key(|s| s.joined_at);
        out
    }

    pub fn users_in(&self, channels: &HashSet<Id>) -> Vec<Id> {
        self.states_in(channels)
            .into_iter()
            .map(|s| s.user_id)
            .collect()
    }

    fn room_empty(&self, channel: Id) -> bool {
        !self
            .states
            .lock()
            .unwrap()
            .values()
            .any(|e| e.state.channel_id == channel)
    }
}

async fn broadcast(st: &St, channel: Id, ev: &Event) -> R<()> {
    let ch = db::channel(&st.db, channel).await?;
    st.hub.send(&db::audience(&st.db, &ch).await?, ev);
    Ok(())
}

pub async fn join(
    State(st): State<St>,
    auth: Auth,
    Path(ch): Path<Id>,
    Json(req): Json<VoiceJoin>,
) -> R<Json<VoiceAnswer>> {
    let me = auth.user;
    let acc = db::access(&st.db, me, ch).await?;
    if !acc.ch.kind.has_voice() {
        return Err(bad("that channel has no calls"));
    }
    if !acc.has(perm::CONNECT) {
        return Err(forbidden("you don't have permission to join calls here"));
    }
    if req.tracks.len() > 8 || req.tracks.keys().any(|m| m.is_empty() || m.len() > 16) {
        return Err(bad("bad track list"));
    }
    let kinds: HashMap<Mid, _> = req
        .tracks
        .iter()
        .map(|(m, k)| (Mid::from(m.as_str()), *k))
        .collect();
    let offer =
        SdpOffer::from_sdp_string(&req.sdp).map_err(|e| bad(format!("bad SDP offer: {e}")))?;

    disconnect(&st, me).await?; // one call at a time

    let mut rtc = Rtc::builder().build(Instant::now());
    let candidate = Candidate::host(st.voice.candidate, "udp")
        .map_err(|e| internal(format!("candidate: {e:?}")))?;
    rtc.add_local_candidate(candidate);
    let answer = rtc
        .sdp_api()
        .accept_offer(offer)
        .map_err(|e| bad(format!("can't use that SDP offer: {e}")))?;

    let client = st.voice.next_client.fetch_add(1, Ordering::Relaxed);
    let first = st.voice.room_empty(ch);
    let state = VoiceState {
        user_id: me,
        channel_id: ch,
        muted: req.muted,
        deafened: req.deafened,
        video: false,
        screen: false,
        joined_at: now_ms(),
    };
    st.voice.states.lock().unwrap().insert(
        me,
        Entry {
            state: state.clone(),
            client,
        },
    );
    st.voice.command(SfuCmd::Join {
        client,
        user: me,
        room: ch,
        rtc: Box::new(rtc),
        kinds: kinds.clone(),
        camera_kbps: st.cfg.camera_kbps,
    });
    broadcast(&st, ch, &Event::VoiceStateUpdate(state)).await?;

    if first && acc.dm() {
        let msg = messages::post_system(&st, &acc.ch, me, MessageKind::Call, None, "").await?;
        st.voice
            .calls
            .lock()
            .unwrap()
            .insert(ch, (msg.id, now_ms()));
    }
    Ok(Json(VoiceAnswer {
        sdp: crate::sfu::cap_camera(&answer.to_sdp_string(), &kinds, st.cfg.camera_kbps),
    }))
}

pub async fn update(
    State(st): State<St>,
    auth: Auth,
    Json(req): Json<VoiceUpdate>,
) -> R<StatusCode> {
    let state = {
        let mut g = st.voice.states.lock().unwrap();
        let e = g
            .get_mut(&auth.user)
            .ok_or_else(|| not_found("call (you're not in one)"))?;
        let s = &mut e.state;
        s.muted = req.muted.unwrap_or(s.muted);
        s.deafened = req.deafened.unwrap_or(s.deafened);
        s.video = req.video.unwrap_or(s.video);
        s.screen = req.screen.unwrap_or(s.screen);
        s.clone()
    };
    broadcast(&st, state.channel_id, &Event::VoiceStateUpdate(state)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn leave(State(st): State<St>, auth: Auth) -> R<StatusCode> {
    disconnect(&st, auth.user).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn ring(State(st): State<St>, auth: Auth, Path(ch): Path<Id>) -> R<StatusCode> {
    let acc = db::access(&st.db, auth.user, ch).await?;
    if !acc.dm() {
        return Err(bad("only direct messages ring"));
    }
    st.hub.send_except(
        &acc.ch.recipients,
        auth.user,
        &Event::CallRing {
            channel_id: ch,
            caller_id: auth.user,
        },
    );
    Ok(StatusCode::NO_CONTENT)
}

/// Hang `user` up, if they are in a call.
pub async fn disconnect(st: &St, user: Id) -> R<()> {
    let entry = st.voice.states.lock().unwrap().remove(&user);
    if let Some(e) = entry {
        st.voice.command(SfuCmd::Leave { client: e.client });
        left(st, e.state).await?;
    }
    Ok(())
}

async fn left(st: &St, state: VoiceState) -> R<()> {
    let ch = state.channel_id;
    // The channel may be gone already (deleted with people in it).
    if db::channel(&st.db, ch).await.is_err() {
        st.voice.calls.lock().unwrap().remove(&ch);
        return Ok(());
    }
    broadcast(
        st,
        ch,
        &Event::VoiceLeave {
            user_id: state.user_id,
            channel_id: ch,
        },
    )
    .await?;
    if st.voice.room_empty(ch) {
        let call = st.voice.calls.lock().unwrap().remove(&ch);
        if let Some((msg, started)) = call {
            let secs = ((now_ms() - started) / 1000).max(1);
            sqlx::query("UPDATE messages SET content = ? WHERE id = ?")
                .bind(secs.to_string())
                .bind(msg)
                .execute(&st.db)
                .await?;
            if let Ok(m) = db::message(&st.db, ch, msg).await {
                broadcast(st, ch, &Event::MessageUpdate(m)).await?;
            }
        }
    }
    Ok(())
}

/// The SFU reports dead connections; drop their voice state unless the user already rejoined.
pub async fn watch_sfu(st: St, mut events: UnboundedReceiver<SfuEvent>) {
    while let Some(SfuEvent::Gone { client }) = events.recv().await {
        let state = {
            let mut g = st.voice.states.lock().unwrap();
            let user = g.iter().find(|(_, e)| e.client == client).map(|(u, _)| *u);
            user.and_then(|u| g.remove(&u)).map(|e| e.state)
        };
        if let Some(s) = state
            && let Err(e) = left(&st, s).await
        {
            tracing::warn!("voice leave after drop: {e:?}");
        }
    }
}
