//! Load generator for a Laklak server. See `loadtest/README.md`.
//!
//! Chat: every user keeps a gateway WebSocket open. Messages go out at `--msg-rate` per second in total, each to
//! its sender's space, and every member's socket clocks how long it took to arrive.
//!
//! Calls: the first `--callers` users also join calls of `--room-size` through the SFU, each with a real str0m
//! peer. The media is fake: Opus- and VP8-sized frames that carry their send time and a sequence number, so the
//! receivers measure delay and loss through the server without encoding anything.

use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt, stream};
use laklak_proto::{AuthResp, Channel, ChannelKind, Id, Invite, Op, Space, VoiceAnswer};
use reqwest::{Client, Method};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use str0m::change::{SdpAnswer, SdpOffer};
use str0m::format::Codec;
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use tokio::net::UdpSocket;
use tokio_tungstenite::tungstenite::Message as WsMsg;

const PASSWORD: &str = "load-test-password";
const USAGE: &str = "laklak-loadtest [--server http://host:port] [--users N] [--space-size N] [--msg-rate PER_SEC]
    [--callers N] [--room-size N] [--video SHARE] [--audio-kbps N] [--video-kbps N] [--fps N]
    [--ramp SECS] [--duration SECS]";

struct Opts {
    server: String,
    /// Chat users, each with a gateway connection.
    users: usize,
    space_size: usize,
    /// Messages per second, all users together.
    msg_rate: f64,
    /// The first this many users also join calls.
    callers: usize,
    room_size: usize,
    /// Share of each room that sends a camera (0..1).
    video: f64,
    audio_kbps: u64,
    video_kbps: u64,
    fps: u64,
    ramp: Duration,
    duration: Duration,
}

impl Opts {
    fn from_args() -> Result<Opts> {
        const NAMES: [&str; 12] = [
            "--server", "--users", "--space-size", "--msg-rate", "--callers", "--room-size", "--video",
            "--audio-kbps", "--video-kbps", "--fps", "--ramp", "--duration",
        ];
        let args: Vec<String> = std::env::args().skip(1).collect();
        for (i, a) in args.iter().enumerate().step_by(2) {
            if !NAMES.contains(&a.as_str()) || i + 1 >= args.len() {
                bail!("usage: {USAGE}");
            }
        }
        fn arg<T: FromStr>(args: &[String], name: &str, default: T) -> Result<T> {
            match args.iter().position(|a| a == name) {
                Some(i) => args[i + 1].parse().map_err(|_| anyhow!("{name}: can't read {:?}", args[i + 1])),
                None => Ok(default),
            }
        }
        let o = Opts {
            server: arg(&args, "--server", "http://127.0.0.1:8080".to_string())?.trim_end_matches('/').into(),
            users: arg(&args, "--users", 200)?,
            space_size: arg(&args, "--space-size", 100)?,
            msg_rate: arg(&args, "--msg-rate", 10.0)?,
            callers: arg(&args, "--callers", 50)?,
            room_size: arg(&args, "--room-size", 5)?,
            video: arg(&args, "--video", 0.2)?,
            audio_kbps: arg(&args, "--audio-kbps", 40)?,
            video_kbps: arg(&args, "--video-kbps", 1000)?,
            fps: arg(&args, "--fps", 30)?,
            ramp: Duration::from_secs(arg(&args, "--ramp", 30)?),
            duration: Duration::from_secs(arg(&args, "--duration", 120)?),
        };
        if o.users == 0 || o.space_size == 0 || o.room_size == 0 || o.fps == 0 || o.callers > o.users {
            bail!("users, space size, room size and fps must be over 0, and callers at most users");
        }
        Ok(o)
    }
}

// ---------------------------------------------------------------- numbers

/// Delay buckets (upper bounds, microseconds).
const BOUNDS_US: [u64; 12] =
    [500, 1_000, 2_000, 5_000, 10_000, 20_000, 50_000, 100_000, 200_000, 500_000, 1_000_000, u64::MAX];

#[derive(Default)]
struct Hist([AtomicU64; 12]);

impl Hist {
    fn add(&self, us: u64) {
        let i = BOUNDS_US.iter().position(|&b| us <= b).expect("last bound is u64::MAX");
        self.0[i].fetch_add(1, Relaxed);
    }
    fn take(&self) -> [u64; 12] {
        std::array::from_fn(|i| self.0[i].swap(0, Relaxed))
    }
}

/// The bucket the `p` quantile falls in, as "≤5ms".
fn quantile(h: &[u64; 12], p: f64) -> String {
    let n: u64 = h.iter().sum();
    if n == 0 {
        return "-".into();
    }
    let want = ((n as f64 * p).ceil() as u64).max(1);
    let mut acc = 0;
    for (i, c) in h.iter().enumerate() {
        acc += c;
        if acc >= want {
            return match BOUNDS_US[i] {
                u64::MAX => ">1s".into(),
                us => format!("≤{}ms", us as f64 / 1000.0),
            };
        }
    }
    unreachable!()
}

#[derive(Default)]
struct Stats {
    ws_up: AtomicU64,
    ws_down: AtomicU64,
    sent: AtomicU64,
    delivered: AtomicU64,
    msg_delay: Hist,
    calls_up: AtomicU64,
    calls_down: AtomicU64,
    frames: AtomicU64,
    bytes: AtomicU64,
    lost: AtomicU64,
    media_delay: Hist,
    /// Failures of any kind; the first few are printed.
    errors: AtomicU64,
}

impl Stats {
    fn error(&self, what: &str, e: impl std::fmt::Display) {
        if self.errors.fetch_add(1, Relaxed) < 20 {
            eprintln!("error: {what}: {e:#}");
        }
    }
}

fn now_us() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("clock after 1970").as_micros() as u64
}

// ---------------------------------------------------------------- server API

async fn api<T: DeserializeOwned>(
    http: &Client,
    o: &Opts,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Result<T> {
    let mut req = http.request(method, format!("{}{path}", o.server));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req.send().await?;
    let status = res.status();
    let bytes = res.bytes().await?;
    if !status.is_success() {
        bail!("{path}: {status} {}", String::from_utf8_lossy(&bytes));
    }
    Ok(serde_json::from_slice(if bytes.is_empty() { b"null" } else { &bytes })?)
}

/// Sign up (into the invite's space), or sign in and join it when the account exists from an earlier run.
async fn account(http: &Client, o: &Opts, name: &str, invite: Option<&str>) -> Result<AuthResp> {
    let body = json!({"username": name, "password": PASSWORD, "invite": invite, "device": "loadtest"});
    if let Ok(a) = api(http, o, Method::POST, "/api/auth/register", None, Some(body)).await {
        return Ok(a);
    }
    let body = json!({"username": name, "password": PASSWORD, "device": "loadtest"});
    let a: AuthResp = api(http, o, Method::POST, "/api/auth/login", None, Some(body)).await?;
    if let Some(code) = invite {
        let _ = api::<Value>(http, o, Method::POST, &format!("/api/invites/{code}"), Some(&a.token), None).await;
    }
    Ok(a)
}

struct User {
    token: String,
    /// The text channel of the user's space.
    text: Id,
    /// The call to join, if a caller.
    room: Option<Id>,
    video: bool,
}

/// Spaces of `space_size` with a text channel each; callers fill voice channels of `room_size` in their space.
async fn setup(http: &Client, o: &Opts) -> Result<Vec<User>> {
    let admin_token = account(http, o, "lt_admin", None).await?.token;
    let admin = Some(admin_token.as_str());
    let per_room_video = (o.room_size as f64 * o.video).round() as usize;
    let mut users = Vec::with_capacity(o.users);
    for s in 0..o.users.div_ceil(o.space_size) {
        let first = s * o.space_size;
        let space: Space =
            api(http, o, Method::POST, "/api/spaces", admin, Some(json!({"name": format!("Load {s}")}))).await?;
        let text = space.channels.iter().find(|c| c.kind == ChannelKind::Text).context("space has no text channel")?.id;
        let invite: Invite =
            api(http, o, Method::POST, &format!("/api/spaces/{}/invites", space.id), admin, Some(json!({}))).await?;
        let members = (o.users - first).min(o.space_size);
        let callers = o.callers.saturating_sub(first).min(members);
        let mut rooms = vec![];
        for r in 0..callers.div_ceil(o.room_size) {
            let body = json!({"name": format!("room {r}"), "kind": "voice"});
            let ch: Channel = api(http, o, Method::POST, &format!("/api/spaces/{}/channels", space.id), admin, Some(body)).await?;
            rooms.push(ch.id);
        }
        let made: Vec<Result<AuthResp>> = stream::iter(0..members)
            .map(|m| {
                let code = invite.code.clone();
                async move { account(http, o, &format!("lt{}", first + m), Some(&code)).await }
            })
            .buffered(64)
            .collect()
            .await;
        for (m, a) in made.into_iter().enumerate() {
            let caller = m < callers;
            users.push(User {
                token: a?.token,
                text,
                room: caller.then(|| rooms[m / o.room_size]),
                video: caller && m % o.room_size < per_room_video,
            });
        }
    }
    Ok(users)
}

// ---------------------------------------------------------------- chat

const MARK: &str = "\"content\":\"lt ";

/// The send time in a test message's `MessageCreate`, found without parsing the whole event.
fn sent_at(text: &str) -> Option<u64> {
    let i = text.find(MARK)? + MARK.len();
    text[i..].split('"').next()?.parse().ok()
}

async fn chat(o: Arc<Opts>, st: Arc<Stats>, token: String, end: Instant) {
    let url = format!("{}/gateway?token={token}", o.server.replacen("http", "ws", 1));
    let mut ws = match tokio_tungstenite::connect_async(&url).await {
        Ok((ws, _)) => ws,
        Err(e) => return st.error("gateway", e),
    };
    st.ws_up.fetch_add(1, Relaxed);
    let heartbeat = serde_json::to_string(&Op::Heartbeat).expect("ops serialize");
    let mut beat = tokio::time::interval(Duration::from_secs(15));
    let stop = tokio::time::sleep_until(end.into());
    tokio::pin!(stop);
    loop {
        tokio::select! {
            m = ws.next() => match m {
                Some(Ok(WsMsg::Text(t))) => if let Some(sent) = sent_at(&t) {
                    st.delivered.fetch_add(1, Relaxed);
                    st.msg_delay.add(now_us().saturating_sub(sent));
                },
                Some(Ok(_)) => {}
                Some(Err(e)) => { st.error("gateway", e); break }
                None => { st.error("gateway", "closed by the server"); break }
            },
            _ = beat.tick() => if ws.send(WsMsg::Text(heartbeat.clone().into())).await.is_err() { break },
            _ = &mut stop => { let _ = ws.close(None).await; break }
        }
    }
    st.ws_down.fetch_add(1, Relaxed);
}

/// `--msg-rate` messages a second from users in turn, each into its own space.
async fn send_messages(http: Client, o: Arc<Opts>, st: Arc<Stats>, users: Arc<Vec<User>>, end: Instant) {
    if o.msg_rate <= 0.0 {
        return;
    }
    let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / o.msg_rate));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    for k in 0usize.. {
        tick.tick().await;
        if Instant::now() >= end {
            return;
        }
        // A stride coprime with most user counts spreads senders over the spaces.
        let u = &users[k.wrapping_mul(7919) % users.len()];
        let (http, o, st, token, ch) = (http.clone(), o.clone(), st.clone(), u.token.clone(), u.text);
        tokio::spawn(async move {
            let body = json!({"content": format!("lt {}", now_us())});
            match api::<Value>(&http, &o, Method::POST, &format!("/api/channels/{ch}/messages"), Some(&token), Some(body)).await {
                Ok(_) => _ = st.sent.fetch_add(1, Relaxed),
                Err(e) => st.error("send", e),
            }
        });
    }
}

// ---------------------------------------------------------------- calls

/// A fake frame: an optional codec header byte, the send time, a sequence number, then padding to `size`.
fn frame(head: Option<u8>, seq: u32, size: usize) -> Vec<u8> {
    let mut f = Vec::with_capacity(size.max(13));
    f.extend(head);
    f.extend_from_slice(&now_us().to_le_bytes());
    f.extend_from_slice(&seq.to_le_bytes());
    f.resize(size.max(f.len()), 0);
    f
}

/// The server's SDP minus its bandwidth lines: it caps cameras with `b=AS`/`b=TIAS` (browsers honour them), which
/// str0m's parser doesn't take. The fake camera keeps its own rate anyway.
fn strip_bandwidth(sdp: &str) -> String {
    sdp.split_inclusive('\n').filter(|l| !l.starts_with("b=")).collect()
}

/// The local address that reaches the server, for the ICE host candidate.
async fn local_ip(o: &Opts) -> Result<IpAddr> {
    let url = reqwest::Url::parse(&o.server)?;
    let host = url.host_str().context("--server has no host")?;
    let addr = tokio::net::lookup_host((host, 9)).await?.find(|a| a.is_ipv4()).context("server has no IPv4 address")?;
    let probe = std::net::UdpSocket::bind("0.0.0.0:0")?;
    probe.connect(addr)?;
    Ok(probe.local_addr()?.ip())
}

#[allow(clippy::too_many_arguments)]
async fn call(
    http: Client,
    o: Arc<Opts>,
    st: Arc<Stats>,
    token: String,
    room: Id,
    video: bool,
    ip: IpAddr,
    end: Instant,
) -> Result<()> {
    let sock = UdpSocket::bind((ip, 0)).await?;
    let local = sock.local_addr()?;
    let mut rtc = Rtc::builder().build(Instant::now());
    rtc.add_local_candidate(Candidate::host(local, "udp").map_err(|e| anyhow!("candidate: {e:?}"))?);
    let mut change = rtc.sdp_api();
    let mic = change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
    let cam = video.then(|| change.add_media(MediaKind::Video, Direction::SendOnly, None, None, None));
    change.add_channel("signal".into());
    let (offer, pending) = change.apply().context("nothing to offer")?;
    let mut tracks = serde_json::Map::new();
    tracks.insert(mic.to_string(), "mic".into());
    if let Some(c) = cam {
        tracks.insert(c.to_string(), "cam".into());
    }
    let body = json!({"sdp": offer.to_sdp_string(), "tracks": tracks, "muted": false, "deafened": false});
    let ans: VoiceAnswer = api(&http, &o, Method::POST, &format!("/api/channels/{room}/voice"), Some(&token), Some(body)).await?;
    rtc.sdp_api().accept_answer(pending, SdpAnswer::from_sdp_string(&strip_bandwidth(&ans.sdp))?)?;

    let pt = |rtc: &mut Rtc, mid: Mid, codec: Codec| {
        rtc.writer(mid).and_then(|w| w.payload_params().find(|p| p.spec().codec == codec).map(|p| p.pt()))
    };
    let audio_pt = pt(&mut rtc, mic, Codec::Opus).context("no Opus in the answer")?;
    let cam = match cam {
        Some(m) => Some((m, pt(&mut rtc, m, Codec::Vp8).context("no VP8 in the answer")?)),
        None => None,
    };
    let audio_size = (o.audio_kbps * 1000 / 8 / 50) as usize; // 20 ms frames
    let video_size = (o.video_kbps * 1000 / 8 / o.fps) as usize;
    let (audio_every, video_every) = (Duration::from_millis(20), Duration::from_secs(1) / o.fps as u32);

    let mut connected = false;
    let result = async {
        let mut keyframe = true;
        let (mut seq_a, mut seq_v, mut rtp_a, mut rtp_v) = (0u32, 0u32, 0u64, 0u64);
        let (mut next_a, mut next_v) = (Instant::now(), Instant::now());
        let mut last_seq: HashMap<Mid, u32> = HashMap::new();
        let mut buf = vec![0u8; 2000];
        loop {
            let timeout = loop {
                match rtc.poll_output()? {
                    Output::Timeout(t) => break t,
                    Output::Transmit(t) => {
                        let _ = sock.send_to(&t.contents, t.destination).await;
                    }
                    Output::Event(Event::Connected) => {
                        connected = true;
                        st.calls_up.fetch_add(1, Relaxed);
                    }
                    Output::Event(Event::IceConnectionStateChange(IceConnectionState::Disconnected)) => {
                        bail!("call connection lost")
                    }
                    // The server adds the others' tracks with an offer on the data channel.
                    Output::Event(Event::ChannelData(d)) => {
                        if let Ok(offer) = serde_json::from_slice::<Value>(&d.data)
                            && offer["type"] == "offer"
                            && let Some(sdp) = offer["sdp"].as_str()
                        {
                            let answer = rtc.sdp_api().accept_offer(SdpOffer::from_sdp_string(&strip_bandwidth(sdp))?)?;
                            let msg = json!({"type": "answer", "sdp": answer.to_sdp_string()}).to_string();
                            rtc.channel(d.id).context("data channel gone")?.write(false, msg.as_bytes())?;
                        }
                    }
                    Output::Event(Event::KeyframeRequest(_)) => keyframe = true,
                    Output::Event(Event::MediaData(d)) => {
                        let at = usize::from(d.params.spec().codec == Codec::Vp8);
                        if let Some(b) = d.data.get(at..at + 12) {
                            let sent = u64::from_le_bytes(b[..8].try_into().expect("8 bytes"));
                            let seq = u32::from_le_bytes(b[8..].try_into().expect("4 bytes"));
                            st.media_delay.add(now_us().saturating_sub(sent));
                            if let Some(prev) = last_seq.insert(d.mid, seq) {
                                st.lost.fetch_add(u64::from(seq.saturating_sub(prev.wrapping_add(1))), Relaxed);
                            }
                            st.frames.fetch_add(1, Relaxed);
                            st.bytes.fetch_add(d.data.len() as u64, Relaxed);
                        }
                    }
                    Output::Event(_) => {}
                }
            };
            if Instant::now() >= end {
                return Ok(());
            }
            let mut wake = timeout;
            if connected {
                wake = wake.min(next_a);
                if cam.is_some() {
                    wake = wake.min(next_v);
                }
            }
            tokio::select! {
                r = sock.recv_from(&mut buf) => {
                    let (n, source) = r?;
                    if let Ok(contents) = buf[..n].try_into() {
                        let input = Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: local, contents });
                        rtc.handle_input(input)?;
                    }
                }
                _ = tokio::time::sleep_until(wake.into()) => {
                    let now = Instant::now();
                    if connected && now >= next_a {
                        let f = frame(None, seq_a, audio_size);
                        rtc.writer(mic).context("mic gone")?.write(audio_pt, now, MediaTime::new(rtp_a, Frequency::FORTY_EIGHT_KHZ), f)?;
                        (seq_a, rtp_a, next_a) = (seq_a + 1, rtp_a + 960, (next_a + audio_every).max(now));
                    }
                    if let Some((mid, pt)) = cam && connected && now >= next_v {
                        // VP8's first byte: bit 0 clear marks a keyframe (what the SFU looks for).
                        keyframe |= seq_v as u64 % (o.fps * 2) == 0;
                        let f = frame(Some(if keyframe { 0x10 } else { 0x11 }), seq_v, video_size);
                        rtc.writer(mid).context("camera gone")?.write(pt, now, MediaTime::new(rtp_v, Frequency::NINETY_KHZ), f)?;
                        keyframe = false;
                        (seq_v, rtp_v, next_v) = (seq_v + 1, rtp_v + 90_000 / o.fps, (next_v + video_every).max(now));
                    }
                    rtc.handle_input(Input::Timeout(now))?;
                }
            }
        }
    }
    .await;
    if connected {
        st.calls_down.fetch_add(1, Relaxed);
    }
    let _ = api::<Value>(&http, &o, Method::DELETE, "/api/voice", Some(&token), None).await;
    result
}

// ---------------------------------------------------------------- run

#[derive(Clone, Copy)]
struct Totals {
    sent: u64,
    delivered: u64,
    frames: u64,
    bytes: u64,
    lost: u64,
}

fn totals(st: &Stats) -> Totals {
    Totals {
        sent: st.sent.load(Relaxed),
        delivered: st.delivered.load(Relaxed),
        frames: st.frames.load(Relaxed),
        bytes: st.bytes.load(Relaxed),
        lost: st.lost.load(Relaxed),
    }
}

fn loss(frames: u64, lost: u64) -> f64 {
    if frames + lost == 0 { 0.0 } else { lost as f64 * 100.0 / (frames + lost) as f64 }
}

/// A line every 5 seconds; then a summary of the time after the ramp.
async fn report(st: &Stats, users: usize, callers: usize, t0: Instant, live: Instant, end: Instant) {
    let every = Duration::from_secs(5);
    let mut tick = tokio::time::interval_at((t0 + every).into(), every);
    let (mut last, mut at_live) = (totals(st), None);
    let (mut msg_all, mut media_all) = ([0u64; 12], [0u64; 12]);
    loop {
        tick.tick().await;
        let now = Instant::now();
        let t = totals(st);
        let (md, ad) = (st.msg_delay.take(), st.media_delay.take());
        let secs = every.as_secs_f64();
        println!(
            "{:>4}s  gateway {}/{users}  msgs {:.0}/s → {:.0} deliveries/s, delay p50 {} p99 {}  |  calls {}/{callers}  media in {:.1} Mbit/s, {:.0} frames/s, lost {:.2}%, delay p50 {} p99 {}  |  errors {}",
            (now - t0).as_secs(),
            st.ws_up.load(Relaxed) - st.ws_down.load(Relaxed),
            (t.sent - last.sent) as f64 / secs,
            (t.delivered - last.delivered) as f64 / secs,
            quantile(&md, 0.5),
            quantile(&md, 0.99),
            st.calls_up.load(Relaxed) - st.calls_down.load(Relaxed),
            (t.bytes - last.bytes) as f64 * 8.0 / secs / 1e6,
            (t.frames - last.frames) as f64 / secs,
            loss(t.frames - last.frames, t.lost - last.lost),
            quantile(&ad, 0.5),
            quantile(&ad, 0.99),
            st.errors.load(Relaxed),
        );
        // Intervals that started after the ramp (give or take the timer's slack) count for the summary.
        if now - every + Duration::from_millis(500) >= live {
            at_live.get_or_insert((last, now - every));
            for i in 0..12 {
                msg_all[i] += md[i];
                media_all[i] += ad[i];
            }
        }
        last = t;
        if now >= end {
            break;
        }
    }
    let (a, from) = at_live.unwrap_or((last, Instant::now()));
    let secs = from.elapsed().as_secs_f64().max(1.0);
    println!(
        "\nsummary after the ramp ({secs:.0}s): msgs {:.1}/s → {:.0} deliveries/s, delay p50 {} p95 {} p99 {}; media in {:.1} Mbit/s, {:.0} frames/s, lost {:.2}%, delay p50 {} p95 {} p99 {}; errors {}",
        (last.sent - a.sent) as f64 / secs,
        (last.delivered - a.delivered) as f64 / secs,
        quantile(&msg_all, 0.5),
        quantile(&msg_all, 0.95),
        quantile(&msg_all, 0.99),
        (last.bytes - a.bytes) as f64 * 8.0 / secs / 1e6,
        (last.frames - a.frames) as f64 / secs,
        loss(last.frames - a.frames, last.lost - a.lost),
        quantile(&media_all, 0.5),
        quantile(&media_all, 0.95),
        quantile(&media_all, 0.99),
        st.errors.load(Relaxed),
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let o = Arc::new(Opts::from_args()?);
    // Built in one workspace with the server, reqwest gets its `rustls-no-provider` feature: install ring the same way.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = Client::builder().pool_max_idle_per_host(256).timeout(Duration::from_secs(30)).build()?;
    let mut up = false;
    for _ in 0..100 {
        if http.get(format!("{}/health", o.server)).send().await.is_ok_and(|r| r.status().is_success()) {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    if !up {
        bail!("{} doesn't answer /health", o.server);
    }

    let started = Instant::now();
    let users = Arc::new(setup(&http, &o).await?);
    let video = users.iter().filter(|u| u.video).count();
    println!(
        "setup {:.0}s: {} users in {} spaces of {}, {} callers in rooms of {} ({video} with a camera), {:.0} msgs/s",
        started.elapsed().as_secs_f64(),
        o.users,
        o.users.div_ceil(o.space_size),
        o.space_size,
        o.callers,
        o.room_size,
        o.msg_rate,
    );
    let ip = local_ip(&o).await?;
    let st = Arc::new(Stats::default());
    let t0 = Instant::now();
    let (live, end) = (t0 + o.ramp, t0 + o.ramp + o.duration);
    for (i, u) in users.iter().enumerate() {
        let at = t0 + o.ramp.mul_f64(i as f64 / users.len() as f64);
        let (o, st, token) = (o.clone(), st.clone(), u.token.clone());
        let caller = u.room.map(|room| (room, u.video, http.clone()));
        tokio::spawn(async move {
            tokio::time::sleep_until(at.into()).await;
            if let Some((room, video, http)) = caller {
                let (o, st, token) = (o.clone(), st.clone(), token.clone());
                tokio::spawn(async move {
                    if let Err(e) = call(http, o, st.clone(), token, room, video, ip, end).await {
                        st.error("call", e);
                    }
                });
            }
            chat(o, st, token, end).await;
        });
    }
    tokio::spawn(send_messages(http.clone(), o.clone(), st.clone(), users.clone(), end));
    report(&st, o.users, o.callers, t0, live, end).await;
    tokio::time::sleep(Duration::from_secs(2)).await; // callers hang up
    Ok(())
}
