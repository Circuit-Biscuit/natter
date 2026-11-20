//! The call SFU: one UDP socket, one thread, many str0m `Rtc`s demultiplexed with `rtc.accepts()`.
//! Adapted from str0m's `examples/chat.rs`, plus rooms, user ids, track kinds and no panics on bad input.
//!
//! Renegotiation runs over each peer's data channel: the server sends `{"type":"offer","sdp":..}` when tracks
//! of others appear or go, the app answers `{"type":"answer","sdp":..}` (and may send its own offers).

use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use natter_proto::{Id, TrackKind};
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::channel::{ChannelData, ChannelId};
use str0m::media::{
    Direction, KeyframeRequest, KeyframeRequestKind, MediaData, MediaKind, Mid, Rid,
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
            Propagated::KeyframeRequest(_, req, source, mid_in) => {
                if *source == client.id {
                    client.handle_keyframe_request(*req, *mid_in)
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
    chosen_rid: Option<Rid>,
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
    last_keyframe_request: Option<Instant>,
}

struct TrackOut {
    track_in: Weak<TrackIn>,
    state: TrackOutState,
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
            chosen_rid: None,
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
            last_keyframe_request: None,
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
        if entry
            .last_keyframe_request
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        let _ = writer.request_keyframe(rid, kind);
        entry.last_keyframe_request = Some(Instant::now());
    }

    fn handle_incoming_keyframe_req(&self, mut req: KeyframeRequest) -> Propagated {
        let Some(out) = self.tracks_out.iter().find(|t| t.mid() == Some(req.mid)) else {
            return Propagated::Noop;
        };
        let Some(track_in) = out.track_in.upgrade() else {
            return Propagated::Noop;
        };
        req.rid = self.chosen_rid;
        Propagated::KeyframeRequest(self.id, req, track_in.origin, track_in.mid)
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
        if let Ok(offer) = serde_json::from_slice::<SdpOffer>(&d.data) {
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
        });
    }

    fn handle_media_data_out(&mut self, origin: u64, data: &MediaData) {
        let Some(mid) = self
            .tracks_out
            .iter()
            .find(|o| {
                o.track_in
                    .upgrade()
                    .is_some_and(|i| i.origin == origin && i.mid == data.mid)
            })
            .and_then(|o| o.mid())
        else {
            return;
        };
        // ponytail: no simulcast layer selection; forward the plain stream (or layer "h").
        if data.rid.is_some() && data.rid != Some("h".into()) {
            return;
        }
        self.chosen_rid = data.rid;
        let Some(writer) = self.rtc.writer(mid) else {
            return;
        };
        let Some(pt) = writer.match_params(data.params) else {
            return;
        };
        if let Err(e) = writer.write(pt, data.network_time, data.time, data.data.clone()) {
            tracing::info!("call client {} write failed: {e:?}", self.id);
            self.rtc.disconnect();
        }
    }

    fn handle_keyframe_request(&mut self, req: KeyframeRequest, mid_in: Mid) {
        if !self.tracks_in.iter().any(|i| i.id.mid == mid_in) {
            return;
        }
        let Some(mut writer) = self.rtc.writer(mid_in) else {
            return;
        };
        let _ = writer.request_keyframe(req.rid, req.kind);
    }
}

#[allow(clippy::large_enum_variant)]
enum Propagated {
    Noop,
    Timeout(Instant),
    TrackOpen(u64, Weak<TrackIn>),
    MediaData(u64, MediaData),
    /// (from client, request, publisher client, publisher's mid)
    KeyframeRequest(u64, KeyframeRequest, u64, Mid),
}

impl Propagated {
    fn client_id(&self) -> Option<u64> {
        match self {
            Propagated::TrackOpen(c, _)
            | Propagated::MediaData(c, _)
            | Propagated::KeyframeRequest(c, _, _, _) => Some(*c),
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
}
