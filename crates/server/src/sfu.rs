//! The call SFU: one UDP socket, one thread, many str0m `Rtc`s demultiplexed with `rtc.accepts()`.
//! Adapted from str0m's `examples/chat.rs`, plus rooms, user ids, track kinds and no panics on bad input.
//!
//! Renegotiation runs over each peer's data channel: the server sends `{"type":"offer","sdp":..}` when tracks
//! of others appear or go, the app answers `{"type":"answer","sdp":..}` (and may send its own offers).
//!
//! Simulcast: apps send cameras and screens as layers "h" (as set), "m" (720p) and "l" (360p). Each viewer gets
//! one layer per stream: the one it picked (`{"type":"layer",..}` on the data channel), or by its bandwidth.

use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use laklak_proto::{Id, TrackKind};
use serde::Deserialize;
use str0m::bwe::{Bitrate, BweKind};
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::channel::{ChannelData, ChannelId};
use str0m::media::{
    Direction, KeyframeRequest, KeyframeRequestKind, MediaData, MediaKind, MediaTime, Mid, Rid,
};
use str0m::net::{Protocol, Receive};
use str0m::{Event, IceConnectionState, Input, Output, Rtc};
use tokio::sync::mpsc::UnboundedSender;

pub enum SfuCmd {
    Join {
        client: u64,
        user: Id,
        room: Id,
        rtc: Box<Rtc>,
        kinds: HashMap<Mid, TrackKind>,
        /// Cap on what this app sends from its camera, in kbit/s (0 = none).
        camera_kbps: u32,
    },
    Leave {
        client: u64,
    },
}

pub enum SfuEvent {
    /// The connection died or was closed.
    Gone { client: u64 },
}

pub fn run(
    socket: UdpSocket,
    local: SocketAddr,
    rx: Receiver<SfuCmd>,
    events: UnboundedSender<SfuEvent>,
) {
    let mut clients: Vec<Client> = vec![];
    let mut to_propagate: VecDeque<Propagated> = VecDeque::new();
    let mut buf = vec![0; 2000];

    loop {
        clients.retain(|c| {
            let alive = c.rtc.is_alive();
            if !alive {
                let _ = events.send(SfuEvent::Gone { client: c.id });
            }
            alive
        });

        loop {
            match rx.try_recv() {
                Ok(SfuCmd::Join {
                    client,
                    user,
                    room,
                    rtc,
                    kinds,
                    camera_kbps,
                }) => {
                    let mut c = Client::new(client, user, room, *rtc, kinds, camera_kbps);
                    for track in clients
                        .iter()
                        .filter(|o| o.room == room)
                        .flat_map(|o| o.tracks_in.iter())
                    {
                        c.handle_track_open(Arc::downgrade(&track.id));
                    }
                    clients.push(c);
                }
                Ok(SfuCmd::Leave { client }) => {
                    if let Some(c) = clients.iter_mut().find(|c| c.id == client) {
                        c.rtc.disconnect();
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return, // server shutting down
            }
        }

        let mut timeout = Instant::now() + Duration::from_millis(100);
        for client in clients.iter_mut() {
            timeout = timeout.min(poll_until_timeout(client, &mut to_propagate, &socket));
        }

        if let Some(p) = to_propagate.pop_front() {
            propagate(&p, &mut clients);
            continue;
        }

        let duration = (timeout - Instant::now()).max(Duration::from_millis(1));
        let _ = socket.set_read_timeout(Some(duration));
        buf.resize(2000, 0);
        match socket.recv_from(&mut buf) {
            Ok((n, source)) => {
                if let Ok(contents) = buf[..n].try_into() {
                    let input = Input::Receive(
                        Instant::now(),
                        Receive {
                            proto: Protocol::Udp,
                            source,
                            destination: local,
                            contents,
                        },
                    );
                    match clients.iter_mut().find(|c| c.rtc.accepts(&input)) {
                        Some(c) => c.handle_input(input),
                        // Common: the browser's first STUN beats the Join command by a few ms; it retries.
                        None => tracing::trace!("no client for udp from {source}"),
                    }
                }
            }
            // WouldBlock/TimedOut: the read timeout. ConnectionReset: Windows reports an ICMP
            // "port unreachable" from an earlier send this way; it is not our socket's fault.
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::ConnectionReset
                ) => {}
            Err(e) => {
                tracing::warn!("sfu udp read: {e}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let now = Instant::now();
        for client in &mut clients {
            client.handle_input(Input::Timeout(now));
        }
    }
}

fn poll_until_timeout(
    client: &mut Client,
    queue: &mut VecDeque<Propagated>,
    socket: &UdpSocket,
) -> Instant {
    loop {
        if !client.rtc.is_alive() {
            return Instant::now();
        }
        match client.poll_output(socket) {
            Propagated::Timeout(t) => return t,
            p => queue.push_back(p),
        }
    }
}

/// Hand one event to every other client in the origin's room.
fn propagate(propagated: &Propagated, clients: &mut [Client]) {
    let Some(origin) = propagated.client_id() else {
        return;
    };
    let Some(room) = clients.iter().find(|c| c.id == origin).map(|c| c.room) else {
        return;
    };
    for client in clients
        .iter_mut()
        .filter(|c| c.id != origin && c.room == room)
    {
        match propagated {
            Propagated::TrackOpen(_, track_in) => client.handle_track_open(track_in.clone()),
            Propagated::MediaData(_, data) => client.handle_media_data_out(origin, data),
            Propagated::KeyframeRequest(_, source, mid_in, rid, kind) => {
                if *source == client.id {
                    client.request_keyframe_throttled(*mid_in, *rid, *kind)
                }
            }
            Propagated::Noop | Propagated::Timeout(_) => {}
        }
    }
}

struct Client {
    id: u64,
    user: Id,
    room: Id,
    rtc: Rtc,
    /// What the app said each of its m-lines carries.
    kinds: HashMap<Mid, TrackKind>,
    /// Camera cap written into every SDP we hand this app (0 = none).
    camera_kbps: u32,
    pending: Option<SdpPendingOffer>,
    cid: Option<ChannelId>,
    tracks_in: Vec<TrackInEntry>,
    tracks_out: Vec<TrackOut>,
    /// Layers this app picked per stream (`layer_rank`); not here = Auto.
    wants: HashMap<(Id, TrackKind), u8>,
    /// What the server can send this app, from transport-wide congestion control.
    estimate: Option<Bitrate>,
    /// Keyframes to ask publishers for: (publisher client, its mid, layer).
    keyframes: Vec<(u64, Mid, Rid)>,
}

struct TrackIn {
    origin: u64,
    user: Id,
    tag: TrackKind,
    mid: Mid,
    kind: MediaKind,
}

struct TrackInEntry {
    id: Arc<TrackIn>,
    /// Per layer (None = no simulcast).
    last_keyframe_request: HashMap<Option<Rid>, Instant>,
}

struct TrackOut {
    track_in: Weak<TrackIn>,
    state: TrackOutState,
    /// Simulcast layers seen coming in, and the one forwarded (None until its first keyframe).
    layers: Vec<Layer>,
    rid: Option<Rid>,
    /// Auto's pick (a `layer_rank`) and when it last changed.
    auto: u8,
    auto_at: Instant,
    /// Last keyframe request for a layer switch.
    asked: Option<Instant>,
    /// RTP time continuity across layer switches: added to the layer's time; last time sent and when.
    offset: u64,
    last: Option<(u64, Instant)>,
    switched: bool,
}

/// One simulcast layer of a stream, as one viewer sees it come in.
struct Layer {
    rid: Rid,
    rank: u8,
    bytes: u64,
    since: Instant,
    bps: f64,
    seen: Instant,
}

/// "l" < "m" < "h"; anything else counts as the top.
fn layer_rank(rid: &str) -> u8 {
    match rid {
        "l" => 0,
        "m" => 1,
        _ => 2,
    }
}

/// A layer that stopped while others of its stream still come is gone: the sender turned it off (it would be as
/// big as the one above), or its upload can't carry it.
const LAYER_GONE: Duration = Duration::from_secs(2);
/// Auto steps up at most this often (each switch costs a keyframe, and estimates wobble); it steps down at once.
const STEP_UP_EVERY: Duration = Duration::from_secs(4);

/// The layer for a wanted rank among live layers (smallest first): the smallest at or over it (a "720p" pick of a
/// 720p share is its top layer, "m" being off), else the biggest there is.
fn pick<'a>(live: &[&'a Layer], rank: u8) -> Option<&'a Layer> {
    live.iter().find(|l| l.rank >= rank).or(live.last()).copied()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrackOutState {
    ToOpen,
    Negotiating(Mid),
    Open(Mid),
    ToStop(Mid),
    NegotiatingStop(Mid),
}

impl TrackOut {
    fn mid(&self) -> Option<Mid> {
        match self.state {
            TrackOutState::ToOpen => None,
            TrackOutState::Negotiating(m)
            | TrackOutState::Open(m)
            | TrackOutState::ToStop(m)
            | TrackOutState::NegotiatingStop(m) => Some(m),
        }
    }

    /// Count a frame of a layer; true when its rate was just measured again (about once a second).
    fn note(&mut self, rid: Rid, bytes: usize, now: Instant) -> bool {
        let i = match self.layers.iter().position(|l| l.rid == rid) {
            Some(i) => i,
            None => {
                self.layers.push(Layer { rid, rank: layer_rank(&rid), bytes: 0, since: now, bps: 0.0, seen: now });
                self.layers.len() - 1
            }
        };
        let l = &mut self.layers[i];
        if now.saturating_duration_since(l.seen) > LAYER_GONE {
            // Back after a gap (the sender's upload dropped it): measure it afresh, not over the silence.
            (l.bytes, l.since, l.bps) = (0, now, 0.0);
        }
        l.bytes += bytes as u64;
        l.seen = now;
        let dt = now.saturating_duration_since(l.since);
        if dt >= Duration::from_secs(1) {
            l.bps = l.bytes as f64 * 8.0 / dt.as_secs_f64();
            l.bytes = 0;
            l.since = now;
            return true;
        }
        false
    }

    /// Layers still coming in, smallest first (all going quiet together is just a still screen).
    fn live(&self) -> Vec<&Layer> {
        let Some(newest) = self.layers.iter().map(|l| l.seen).max() else {
            return vec![];
        };
        let mut v: Vec<&Layer> = self.layers.iter().filter(|l| l.seen + LAYER_GONE >= newest).collect();
        v.sort_by_key(|l| l.rank);
        v
    }

    /// RTP time for a frame going out. Layers may have their own timestamp offsets, so after a switch the time
    /// continues from the last frame sent by the wall-clock gap instead of jumping.
    fn out_time(&mut self, data: &MediaData) -> MediaTime {
        let t = data.time;
        if std::mem::take(&mut self.switched)
            && let Some((last, at)) = self.last
        {
            let gap = data.network_time.saturating_duration_since(at).as_secs_f64() * f64::from(t.denom());
            self.offset = last.wrapping_add((gap as u64).max(1)).wrapping_sub(t.numer());
        }
        let n = t.numer().wrapping_add(self.offset);
        self.last = Some((n, data.network_time));
        MediaTime::new(n, t.frequency())
    }
}

impl Client {
    fn new(id: u64, user: Id, room: Id, rtc: Rtc, kinds: HashMap<Mid, TrackKind>, camera_kbps: u32) -> Client {
        Client {
            id,
            user,
            room,
            rtc,
            kinds,
            camera_kbps,
            pending: None,
            cid: None,
            tracks_in: vec![],
            tracks_out: vec![],
            wants: HashMap::new(),
            estimate: None,
            keyframes: vec![],
        }
    }

    fn handle_input(&mut self, input: Input) {
        if self.rtc.is_alive()
            && let Err(e) = self.rtc.handle_input(input)
        {
            tracing::info!(
                "call client {} (user {}) dropped: {e:?}",
                self.id,
                self.user
            );
            self.rtc.disconnect();
        }
    }

    fn poll_output(&mut self, socket: &UdpSocket) -> Propagated {
        if !self.rtc.is_alive() {
            return Propagated::Noop;
        }
        if let Some((origin, mid_in, rid)) = self.keyframes.pop() {
            return Propagated::KeyframeRequest(self.id, origin, mid_in, Some(rid), KeyframeRequestKind::Pli);
        }
        // Tracks of others waiting for an m-line need an SDP round trip first.
        if self.negotiate_if_needed() {
            return Propagated::Noop;
        }
        match self.rtc.poll_output() {
            Ok(output) => self.handle_output(output, socket),
            Err(e) => {
                tracing::info!("call client {} poll failed: {e:?}", self.id);
                self.rtc.disconnect();
                Propagated::Noop
            }
        }
    }

    fn handle_output(&mut self, output: Output, socket: &UdpSocket) -> Propagated {
        match output {
            Output::Transmit(t) => {
                if let Err(e) = socket.send_to(&t.contents, t.destination) {
                    tracing::debug!("sfu udp send to {}: {e}", t.destination);
                }
                Propagated::Noop
            }
            Output::Timeout(t) => Propagated::Timeout(t),
            Output::Event(e) => match e {
                Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                    self.rtc.disconnect();
                    Propagated::Noop
                }
                Event::MediaAdded(e) => self.handle_media_added(e.mid, e.kind),
                Event::MediaData(data) => self.handle_media_data_in(data),
                Event::KeyframeRequest(req) => self.handle_incoming_keyframe_req(req),
                Event::ChannelOpen(cid, _) => {
                    self.cid = Some(cid);
                    Propagated::Noop
                }
                Event::ChannelData(data) => self.handle_channel_data(data),
                Event::EgressBitrateEstimate(BweKind::Twcc { estimate, can_probe }) => {
                    tracing::trace!("call client {}: estimate {estimate} (probing {can_probe})", self.id);
                    self.estimate = Some(estimate);
                    self.allocate();
                    Propagated::Noop
                }
                _ => Propagated::Noop,
            },
        }
    }

    fn handle_media_added(&mut self, mid: Mid, kind: MediaKind) -> Propagated {
        let tag = self.kinds.get(&mid).copied().unwrap_or(match kind {
            MediaKind::Audio => TrackKind::Mic,
            MediaKind::Video => TrackKind::Cam,
        });
        let entry = TrackInEntry {
            id: Arc::new(TrackIn {
                origin: self.id,
                user: self.user,
                tag,
                mid,
                kind,
            }),
            last_keyframe_request: HashMap::new(),
        };
        // This client owns the strong reference; everyone else holds weak ones.
        let weak = Arc::downgrade(&entry.id);
        self.tracks_in.push(entry);
        Propagated::TrackOpen(self.id, weak)
    }

    fn handle_media_data_in(&mut self, data: MediaData) -> Propagated {
        if !data.contiguous {
            self.request_keyframe_throttled(data.mid, data.rid, KeyframeRequestKind::Fir);
        }
        Propagated::MediaData(self.id, data)
    }

    fn request_keyframe_throttled(
        &mut self,
        mid: Mid,
        rid: Option<Rid>,
        kind: KeyframeRequestKind,
    ) {
        let Some(mut writer) = self.rtc.writer(mid) else {
            return;
        };
        let Some(entry) = self.tracks_in.iter_mut().find(|t| t.id.mid == mid) else {
            return;
        };
        // One keyframe serves every viewer of the layer (and one switching to it).
        if entry
            .last_keyframe_request
            .get(&rid)
            .is_some_and(|t| t.elapsed() < Duration::from_millis(500))
        {
            return;
        }
        let _ = writer.request_keyframe(rid, kind);
        entry.last_keyframe_request.insert(rid, Instant::now());
    }

    fn handle_incoming_keyframe_req(&self, req: KeyframeRequest) -> Propagated {
        let Some(out) = self.tracks_out.iter().find(|t| t.mid() == Some(req.mid)) else {
            return Propagated::Noop;
        };
        let Some(track_in) = out.track_in.upgrade() else {
            return Propagated::Noop;
        };
        Propagated::KeyframeRequest(self.id, track_in.origin, track_in.mid, out.rid, req.kind)
    }

    /// Auto: share what the server can send this app between the streams it gets. Everything starts on its
    /// smallest layer, then screens step up before cameras while they fit. Also tells the bandwidth estimator
    /// what to probe for, so it finds room for bigger layers.
    fn allocate(&mut self) {
        let Some(est) = self.estimate else {
            return;
        };
        // Room for audio, retransmissions and an optimistic estimate.
        let mut budget = est.as_f64() * 0.85;
        let (mut used, mut desired) = (0.0, 0.0);
        let mut autos = vec![];
        for (i, t) in self.tracks_out.iter().enumerate() {
            let Some(ti) = t.track_in.upgrade() else { continue };
            let live = t.live();
            let (Some(low), Some(top)) = (live.first(), live.last()) else { continue };
            match self.wants.get(&(ti.user, ti.tag)) {
                Some(&rank) => {
                    let bps = pick(&live, rank).map_or(0.0, |l| l.bps);
                    used += bps;
                    desired += bps;
                }
                // A layer not measured yet: leave the stream where it is for now (its first second).
                None if live.iter().any(|l| l.bps == 0.0) => {}
                None => {
                    used += low.bps;
                    desired += top.bps;
                    autos.push((ti.tag != TrackKind::Screen, i));
                }
            }
        }
        budget -= used;
        autos.sort();
        let mut step = f64::INFINITY; // the smallest step up still to take
        for (_, i) in autos {
            let t = &mut self.tracks_out[i];
            let layers: Vec<(u8, f64)> = t.live().iter().map(|l| (l.rank, l.bps)).collect();
            let base = layers[0].1;
            let mut rank = layers.iter().rev().find(|l| l.1 - base <= budget).unwrap_or(&layers[0]).0;
            if rank > t.auto && t.auto_at.elapsed() < STEP_UP_EVERY {
                rank = t.auto;
            }
            let cost = layers.iter().rev().find(|l| l.0 <= rank).unwrap_or(&layers[0]).1;
            budget -= cost - base;
            used += cost - base;
            if let Some(up) = layers.iter().find(|l| l.0 > rank) {
                step = step.min(up.1 - cost);
            }
            if rank != t.auto {
                tracing::debug!("call client {}: auto layer {} -> {rank} (estimate {est}, layers {layers:?})", self.id, t.auto);
                t.auto = rank;
                t.auto_at = Instant::now();
            }
        }
        // Keep the link busy up to the next step (padding) and probe for the best everything could get.
        let min = if step.is_finite() { used + step } else { used };
        let round = |b: f64| Bitrate::bps((b / 250_000.0).ceil() as u64 * 250_000);
        self.rtc.bwe().set_bitrate(round(min), round(desired));
    }

    fn negotiate_if_needed(&mut self) -> bool {
        if self.cid.is_none() || self.pending.is_some() {
            return false;
        }
        // Tracks whose publisher left: stop their m-lines so the app releases them.
        for track in &mut self.tracks_out {
            if let TrackOutState::Open(m) = track.state
                && track.track_in.upgrade().is_none()
            {
                track.state = TrackOutState::ToStop(m);
            }
        }
        let mut change = self.rtc.sdp_api();
        for track in &mut self.tracks_out {
            match track.state {
                TrackOutState::ToOpen => {
                    if let Some(t) = track.track_in.upgrade() {
                        // msid tokens can't hold ':' (str0m drops it), so "<user>-<kind>".
                        let stream_id = format!("{}-{}", t.user, t.tag.as_str());
                        let mid = change.add_media(
                            t.kind,
                            Direction::SendOnly,
                            Some(stream_id),
                            None,
                            None,
                        );
                        track.state = TrackOutState::Negotiating(mid);
                    }
                }
                TrackOutState::ToStop(mid) => {
                    change.stop_media(mid);
                    track.state = TrackOutState::NegotiatingStop(mid);
                }
                _ => {}
            }
        }
        if !change.has_changes() {
            return false;
        }
        let Some((offer, pending)) = change.apply() else {
            return false;
        };
        let Some(mut channel) = self.cid.and_then(|id| self.rtc.channel(id)) else {
            return false;
        };
        let sdp = cap_camera(&offer.to_sdp_string(), &self.kinds, self.camera_kbps);
        let json = serde_json::json!({ "type": "offer", "sdp": sdp }).to_string();
        if channel.write(false, json.as_bytes()).is_err() {
            self.rtc.disconnect();
            return false;
        }
        self.pending = Some(pending);
        true
    }

    fn handle_channel_data(&mut self, d: ChannelData) -> Propagated {
        if let Ok(p) = serde_json::from_slice::<LayerPick>(&d.data) {
            match p.want.as_str() {
                "auto" => self.wants.remove(&(p.user, p.kind)),
                w => self.wants.insert((p.user, p.kind), layer_rank(w)),
            };
            self.allocate();
        } else if let Ok(offer) = serde_json::from_slice::<SdpOffer>(&d.data) {
            self.handle_offer(offer);
        } else if let Ok(answer) = serde_json::from_slice::<SdpAnswer>(&d.data) {
            self.handle_answer(answer);
        }
        Propagated::Noop
    }

    fn handle_offer(&mut self, offer: SdpOffer) {
        let answer = match self.rtc.sdp_api().accept_offer(offer) {
            Ok(a) => a,
            Err(e) => {
                tracing::info!("call client {}: bad offer: {e:?}", self.id);
                self.rtc.disconnect();
                return;
            }
        };
        // Our own pending offer lost the race; redo it after this one.
        for track in &mut self.tracks_out {
            match track.state {
                TrackOutState::Negotiating(_) => track.state = TrackOutState::ToOpen,
                TrackOutState::NegotiatingStop(m) => track.state = TrackOutState::ToStop(m),
                _ => {}
            }
        }
        let sdp = cap_camera(&answer.to_sdp_string(), &self.kinds, self.camera_kbps);
        let json = serde_json::json!({ "type": "answer", "sdp": sdp }).to_string();
        let sent = self
            .cid
            .and_then(|id| self.rtc.channel(id))
            .is_some_and(|mut ch| ch.write(false, json.as_bytes()).is_ok());
        if !sent {
            self.rtc.disconnect();
        }
    }

    fn handle_answer(&mut self, answer: SdpAnswer) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if let Err(e) = self.rtc.sdp_api().accept_answer(pending, answer) {
            tracing::info!("call client {}: bad answer: {e:?}", self.id);
            self.rtc.disconnect();
            return;
        }
        for track in &mut self.tracks_out {
            if let TrackOutState::Negotiating(m) = track.state {
                track.state = TrackOutState::Open(m);
            }
        }
        self.tracks_out
            .retain(|t| !matches!(t.state, TrackOutState::NegotiatingStop(_)));
    }

    fn handle_track_open(&mut self, track_in: Weak<TrackIn>) {
        self.tracks_out.push(TrackOut {
            track_in,
            state: TrackOutState::ToOpen,
            layers: vec![],
            rid: None,
            auto: 0, // start small: a quick first picture, then step up
            auto_at: Instant::now(),
            asked: None,
            offset: 0,
            last: None,
            switched: false,
        });
    }

    fn handle_media_data_out(&mut self, origin: u64, data: &MediaData) {
        let Some((i, key)) = self.tracks_out.iter().enumerate().find_map(|(i, o)| {
            let t = o.track_in.upgrade().filter(|t| t.origin == origin && t.mid == data.mid)?;
            Some((i, (t.user, t.tag)))
        }) else {
            return;
        };
        let want = self.wants.get(&key).copied();
        let Some(mid) = self.tracks_out[i].mid() else {
            return;
        };
        // Layer rates changed: Auto may fit something else (the estimate alone can stay put for a while).
        if let Some(rid) = data.rid
            && self.tracks_out[i].note(rid, data.data.len(), data.network_time)
        {
            self.allocate();
        }
        let out = &mut self.tracks_out[i];
        if let Some(rid) = data.rid {
            // Simulcast: forward one layer; move to another only on its keyframe (asking for one).
            let target = pick(&out.live(), want.unwrap_or(out.auto)).map(|l| l.rid);
            if target == Some(rid) && out.rid != target {
                if data.is_keyframe() {
                    out.rid = target;
                    out.switched = true;
                } else if out.asked.is_none_or(|t| t.elapsed() > Duration::from_millis(500)) {
                    out.asked = Some(Instant::now());
                    self.keyframes.push((origin, data.mid, rid));
                }
            }
            if out.rid != Some(rid) {
                return;
            }
        }
        let time = out.out_time(data);
        let Some(writer) = self.rtc.writer(mid) else {
            return;
        };
        let Some(pt) = writer.match_params(data.params) else {
            return;
        };
        if let Err(e) = writer.write(pt, data.network_time, time, data.data.clone()) {
            tracing::info!("call client {} write failed: {e:?}", self.id);
            self.rtc.disconnect();
        }
    }
}

/// A viewer's quality pick for a stream it gets: `want` is "auto", "h", "m" or "l".
#[derive(Deserialize)]
struct LayerPick {
    user: Id,
    kind: TrackKind,
    want: String,
}

#[allow(clippy::large_enum_variant)]
enum Propagated {
    Noop,
    Timeout(Instant),
    TrackOpen(u64, Weak<TrackIn>),
    MediaData(u64, MediaData),
    /// (from client, publisher client, publisher's mid, layer, kind)
    KeyframeRequest(u64, u64, Mid, Option<Rid>, KeyframeRequestKind),
}

impl Propagated {
    fn client_id(&self) -> Option<u64> {
        match self {
            Propagated::TrackOpen(c, _)
            | Propagated::MediaData(c, _)
            | Propagated::KeyframeRequest(c, _, _, _, _) => Some(*c),
            _ => None,
        }
    }
}

/// Cap the app's camera: a `b=AS` (and `b=TIAS`) line in its camera's m-section of SDP we hand it. Browsers keep
/// what they send under the remote description's bandwidth line, so this needs no change in the apps.
pub fn cap_camera(sdp: &str, kinds: &HashMap<Mid, TrackKind>, kbps: u32) -> String {
    if kbps == 0 {
        return sdp.to_string();
    }
    let cams: Vec<String> = kinds.iter().filter(|(_, k)| **k == TrackKind::Cam).map(|(m, _)| m.to_string()).collect();
    let mut out: Vec<String> = vec![];
    // Split into the session part and one block per m= line.
    let mut sections: Vec<Vec<&str>> = vec![vec![]];
    for line in sdp.lines() {
        if line.starts_with("m=") {
            sections.push(vec![]);
        }
        sections.last_mut().expect("one section").push(line);
    }
    for section in sections {
        let is_cam = section.first().is_some_and(|l| l.starts_with("m=video"))
            && section.iter().any(|l| l.strip_prefix("a=mid:").is_some_and(|m| cams.iter().any(|c| c == m.trim())));
        if !is_cam {
            out.extend(section.iter().map(|l| l.to_string()));
            continue;
        }
        // SDP order is m=, i=, c=, b=, then a=: the bandwidth lines go after c= (or after m= without one).
        let lines: Vec<&str> = section.into_iter().filter(|l| !l.starts_with("b=")).collect();
        let at = lines.iter().position(|l| l.starts_with("c=")).unwrap_or(0) + 1;
        for (i, l) in lines.iter().enumerate() {
            out.push(l.to_string());
            if i + 1 == at {
                out.push(format!("b=AS:{kbps}"));
                out.push(format!("b=TIAS:{}", u64::from(kbps) * 1000));
            }
        }
    }
    let mut sdp = out.join("\r\n");
    sdp.push_str("\r\n");
    sdp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_only_the_camera() {
        let sdp = "v=0\r\no=- 1 2 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n\
            m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=mid:0\r\n\
            m=video 9 UDP/TLS/RTP/SAVPF 96\r\nc=IN IP4 0.0.0.0\r\nb=AS:5000\r\na=mid:1\r\na=sendrecv\r\n\
            m=video 9 UDP/TLS/RTP/SAVPF 96\r\nc=IN IP4 0.0.0.0\r\na=mid:2\r\n";
        let kinds: HashMap<Mid, TrackKind> =
            [(Mid::from("0"), TrackKind::Mic), (Mid::from("1"), TrackKind::Cam), (Mid::from("2"), TrackKind::Screen)].into();
        let out = cap_camera(sdp, &kinds, 1000);
        let cam = out.split("m=video").nth(1).unwrap();
        let screen = out.split("m=video").nth(2).unwrap();
        assert!(cam.contains("c=IN IP4 0.0.0.0\r\nb=AS:1000\r\nb=TIAS:1000000\r\na=mid:1"), "{cam}");
        assert!(!cam.contains("b=AS:5000"), "the old limit goes");
        assert!(!screen.contains("b=") && !out.split("m=audio").nth(1).unwrap().split("m=video").next().unwrap().contains("b="));
        assert_eq!(cap_camera(sdp, &kinds, 0), sdp, "0 = no cap");
    }

    #[test]
    fn picks_live_layers() {
        let t0 = Instant::now();
        let mut out = TrackOut {
            track_in: Weak::new(),
            state: TrackOutState::ToOpen,
            layers: vec![],
            rid: None,
            auto: 2,
            auto_at: t0,
            asked: None,
            offset: 0,
            last: None,
            switched: false,
        };
        let rid = |out: &TrackOut, rank| pick(&out.live(), rank).map(|l| l.rid.to_string());
        // A 720p share: "m" would be as big as "h", so the sender keeps it off.
        out.note("l".into(), 100, t0);
        out.note("h".into(), 100, t0);
        assert_eq!(rid(&out, 1).as_deref(), Some("h"), "720p of a 720p share is its top layer");
        assert_eq!(rid(&out, 0).as_deref(), Some("l"));
        assert_eq!(rid(&out, 2).as_deref(), Some("h"));
        // "h" stops (the sender's upload can't carry it) while "l" goes on: Source falls back to "l".
        out.note("l".into(), 100, t0 + Duration::from_secs(3));
        assert_eq!(rid(&out, 2).as_deref(), Some("l"));
        // It comes back: measured afresh, not as 100 bytes over three seconds.
        out.note("h".into(), 100, t0 + Duration::from_secs(4));
        out.note("h".into(), 100, t0 + Duration::from_secs(5));
        assert_eq!(out.layers.iter().find(|l| &*l.rid == "h").map(|l| l.bps), Some(1600.0));
        // Everything quiet together (a still screen) keeps every layer.
        let mut still = TrackOut { layers: vec![], ..out };
        still.note("l".into(), 1, t0);
        still.note("h".into(), 1, t0 + Duration::from_millis(10));
        assert_eq!(still.live().len(), 2);
    }
}
