//! Calls: Rust side. Media itself runs in `glue.js` (WebRTC lives in the webview).

use leptos::prelude::*;
use leptos::task::spawn_local;
use natter_proto::{Id, VoiceUpdate};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::sound::{self, Sound};
use crate::store::{Call, store};
use crate::{api, util};

#[wasm_bindgen(module = "/glue.js")]
extern "C" {
    #[wasm_bindgen(js_name = voiceSetHandler)]
    fn set_handler(cb: &Closure<dyn FnMut(String)>);
    #[wasm_bindgen(js_name = voiceJoin, catch)]
    async fn js_join(base: &str, token: &str, channel: f64, me: f64, opts: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = voiceLeave)]
    fn js_leave();
    #[wasm_bindgen(js_name = voiceSetMuted)]
    fn js_muted(on: bool);
    #[wasm_bindgen(js_name = voiceSetDeafened)]
    fn js_deafened(on: bool);
    #[wasm_bindgen(js_name = voiceSetPtt)]
    fn js_ptt_mode(on: bool);
    #[wasm_bindgen(js_name = voicePtt)]
    fn js_ptt(down: bool);
    #[wasm_bindgen(js_name = voiceSetCamera, catch)]
    async fn js_camera(on: bool) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = voiceSetScreen, catch)]
    async fn js_screen(on: bool) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = voiceAttach)]
    pub fn attach(el: &web_sys::HtmlVideoElement, key: &str);
    #[wasm_bindgen(js_name = voiceHas)]
    pub fn has(key: &str) -> bool;
    #[wasm_bindgen(js_name = voiceDevices)]
    async fn js_devices() -> JsValue;
    #[wasm_bindgen(js_name = voiceSetDevices)]
    async fn js_set_devices(input: &str, output: &str, noise: bool);
    #[wasm_bindgen(js_name = voiceSetCameraDevice, catch)]
    async fn js_set_camera_device(id: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = cameraPreview, catch)]
    pub async fn camera_preview(el: &web_sys::HtmlVideoElement, id: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = cameraPreviewStop)]
    pub fn camera_preview_stop();
    #[wasm_bindgen(js_name = voiceSetStream)]
    async fn js_set_stream(json: &str);
    #[wasm_bindgen(js_name = voiceSetCameraQuality)]
    async fn js_set_camera_quality(json: &str);
    #[wasm_bindgen(js_name = voiceStats)]
    async fn js_stats(key: &str) -> JsValue;
    #[wasm_bindgen(js_name = voiceSetLayer)]
    fn js_set_layer(key: &str, want: &str);
}

/// Pick the quality of someone's camera or screen you receive ("<user>:<kind>"): "auto", "h" (as sent), "m"
/// (720p) or "l" (360p). The server switches at the stream's next keyframe; kept across reconnects.
pub fn set_layer(key: &str, want: &str) {
    store().call.update(|c| {
        if want == "auto" {
            c.layers.remove(key);
        } else {
            c.layers.insert(key.to_string(), want.to_string());
        }
    });
    js_set_layer(key, want);
}

/// What a video stream is doing right now (see `voiceStats` in glue.js).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Stats {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub kbps: u32,
    pub codec: String,
    /// "none", "bandwidth", "cpu" or "other" (sending only).
    pub limit: String,
}

/// `local:screen`, `local:cam`, `<user>:screen`, `<user>:cam`.
pub async fn stats(key: &str) -> Option<Stats> {
    serde_json::from_str(&js_stats(key).await.as_string()?).ok()
}

/// Hand the saved stream settings to the media code: a share in progress changes at once (no need to pick the
/// window again), the next one starts with them.
pub fn apply_stream() {
    let json = serde_json::to_string(&store().prefs.get_untracked().stream).unwrap_or_default();
    spawn_local(async move { js_set_stream(&json).await });
}

/// Apply the saved camera quality (a camera that's on changes in place).
pub fn apply_camera_quality() {
    let json = serde_json::to_string(&store().prefs.get_untracked().cam).unwrap_or_default();
    spawn_local(async move { js_set_camera_quality(&json).await });
}

/// The Screen buttons: choose how to share first (Go Live), or stop sharing.
pub fn screen_button() {
    let s = store();
    if s.call.with_untracked(|c| c.screen) {
        toggle_screen();
    } else {
        s.modal.set(Some(crate::store::Modal::GoLive));
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum JsEvent {
    Speaking { user: Id, on: bool },
    Tracks,
    State { state: String },
    Warn { text: String },
    ScreenEnded,
}

/// Bumped whenever remote or local tracks change, so video tiles re-attach.
pub fn tracks_version() -> RwSignal<u32> {
    thread_local! { static V: RwSignal<u32> = RwSignal::new(0); }
    V.with(|v| *v)
}

pub fn init() {
    let cb = Closure::<dyn FnMut(String)>::new(|json: String| {
        let Ok(ev) = serde_json::from_str::<JsEvent>(&json) else { return };
        let s = store();
        match ev {
            JsEvent::Speaking { user, on } => s.call.update(|c| {
                if on {
                    c.speaking.insert(user);
                } else {
                    c.speaking.remove(&user);
                }
            }),
            JsEvent::Tracks => tracks_version().update(|v| *v += 1),
            JsEvent::State { state } => match state.as_str() {
                "connected" => s.call.update(|c| c.connecting = false),
                "failed" => {
                    // Network changed or the server restarted: reconnect the call by itself.
                    if s.call.with_untracked(|c| c.channel.is_some()) {
                        s.toast("Call connection lost — reconnecting…", true);
                        rejoin();
                    }
                }
                _ => {}
            },
            JsEvent::Warn { text } => s.toast(text, true),
            JsEvent::ScreenEnded => {
                s.call.update(|c| c.screen = false);
                push_state();
                tracks_version().update(|v| *v += 1);
            }
        }
    });
    set_handler(&cb);
    cb.forget();
}

fn opts_json() -> String {
    let s = store();
    let p = s.prefs.get_untracked();
    let c = s.call.get_untracked();
    serde_json::json!({
        "muted": c.muted, "deafened": c.deafened, "input": p.input, "output": p.output, "camera": p.camera,
        "ptt": p.push_to_talk, "noise": p.noise_suppression, "stream": p.stream, "cam": p.cam, "layers": c.layers,
    })
    .to_string()
}

/// Join a voice channel or DM call (leaving any other call).
pub fn join(channel: Id) {
    let s = store();
    if s.call.with_untracked(|c| c.channel == Some(channel)) {
        return;
    }
    s.ringing.set(None);
    sound::ring(false);
    let (muted, deafened) = s.call.with_untracked(|c| (c.muted, c.deafened));
    s.call.set(Call { channel: Some(channel), connecting: true, muted, deafened, ..Default::default() });
    connect(channel, true);
}

fn connect(channel: Id, announce: bool) {
    let s = store();
    let me = s.me_id();
    spawn_local(async move {
        let base = {
            let b = api::server();
            if b.is_empty() { util::window().location().origin().unwrap_or_default() } else { b }
        };
        let r = js_join(&base, &api::token().unwrap_or_default(), channel as f64, me as f64, &opts_json()).await;
        if s.call.with_untracked(|c| c.channel != Some(channel)) {
            return; // left meanwhile
        }
        match r {
            Ok(_) => {
                if announce {
                    sound::play(Sound::Connect, s.settings.get_untracked().sounds);
                }
                // Stay "connecting" until ICE says connected (or give up after 15 s).
                set_timeout(
                    move || {
                        if s.call.with_untracked(|c| c.channel == Some(channel) && c.connecting) {
                            s.toast("Can't reach the call server's media port (UDP). Check the firewall.", true);
                        }
                    },
                    std::time::Duration::from_secs(15),
                );
            }
            Err(e) => {
                let msg = e.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message())).unwrap_or_else(|| "Could not join the call".into());
                s.toast(msg, true);
                s.call.update(|c| {
                    c.channel = None;
                    c.connecting = false;
                });
            }
        }
    });
}

/// Reconnect the current call (after a network blip or server restart).
pub fn rejoin() {
    let s = store();
    let Some(channel) = s.call.with_untracked(|c| c.channel) else { return };
    s.call.update(|c| {
        c.connecting = true;
        c.video = false;
        c.screen = false;
    });
    set_timeout(move || connect(channel, false), std::time::Duration::from_millis(800));
}

pub fn leave() {
    let s = store();
    if s.call.with_untracked(|c| c.channel.is_none()) {
        return;
    }
    hang_up_local();
    sound::play(Sound::Disconnect, s.settings.get_untracked().sounds);
    spawn_local(async {
        let _ = api::act("DELETE", "/api/voice").await;
    });
}

/// Drop the media without telling the server (it already knows, or we're signing out).
pub fn hang_up_local() {
    js_leave();
    let s = store();
    let me = s.me_id();
    s.call.update(|c| {
        let (muted, deafened) = (c.muted, c.deafened);
        *c = Call { muted, deafened, ..Default::default() };
    });
    s.voice.update(|v| {
        v.remove(&me);
    });
    tracks_version().update(|v| *v += 1);
}

fn push_state() {
    let c = store().call.get_untracked();
    if c.channel.is_none() {
        return;
    }
    let body = VoiceUpdate { muted: Some(c.muted), deafened: Some(c.deafened), video: Some(c.video), screen: Some(c.screen) };
    spawn_local(async move {
        let _: Result<(), _> = api::call("PATCH", "/api/voice", &body).await;
    });
}

pub fn toggle_mute() {
    let s = store();
    let sounds = s.settings.get_untracked().sounds;
    s.call.update(|c| {
        if c.deafened {
            // Unmuting while deafened undeafens too, like Discord.
            c.deafened = false;
            c.muted = false;
        } else {
            c.muted = !c.muted;
        }
    });
    let c = s.call.get_untracked();
    js_deafened(c.deafened);
    js_muted(c.muted);
    sound::play(if c.muted { Sound::Mute } else { Sound::Unmute }, sounds);
    push_state();
}

pub fn toggle_deafen() {
    let s = store();
    let sounds = s.settings.get_untracked().sounds;
    s.call.update(|c| {
        c.deafened = !c.deafened;
        if c.deafened {
            c.muted = true;
        } else {
            c.muted = false;
        }
    });
    let c = s.call.get_untracked();
    js_deafened(c.deafened);
    js_muted(c.muted);
    sound::play(if c.deafened { Sound::Mute } else { Sound::Unmute }, sounds);
    push_state();
}

pub fn toggle_camera() {
    let s = store();
    let on = !s.call.with_untracked(|c| c.video);
    spawn_local(async move {
        match js_camera(on).await {
            Ok(v) => {
                s.call.update(|c| c.video = v.as_bool().unwrap_or(false));
                push_state();
            }
            Err(_) => s.toast("Couldn't start the camera. Is it in use or blocked?", true),
        }
    });
}

pub fn toggle_screen() {
    let s = store();
    let on = !s.call.with_untracked(|c| c.screen);
    spawn_local(async move {
        match js_screen(on).await {
            Ok(v) => {
                let sharing = v.as_bool().unwrap_or(false);
                s.call.update(|c| {
                    c.screen = sharing;
                    if sharing {
                        c.focus = Some(format!("{}:screen", s.me_id()));
                    }
                });
                push_state();
            }
            Err(_) => {} // picker cancelled
        }
    });
}

pub fn set_ptt_mode(on: bool) {
    js_ptt_mode(on);
}

pub fn ptt(down: bool) {
    let s = store();
    if s.call.with_untracked(|c| c.ptt_down) == down {
        return;
    }
    s.call.update(|c| c.ptt_down = down);
    js_ptt(down);
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct Device {
    pub kind: String,
    pub id: String,
    pub label: String,
}

pub async fn devices() -> Vec<Device> {
    js_devices().await.as_string().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn apply_devices() {
    let s = store();
    let p = s.prefs.get_untracked();
    spawn_local(async move {
        js_set_devices(&p.input, &p.output, p.noise_suppression).await;
        if js_set_camera_device(&p.camera).await.is_err() {
            s.toast("Couldn't switch to that camera. Is another app using it?", true);
        }
    });
}

/// Start a call in a DM: join it and ring the others.
pub fn call_dm(channel: Id) {
    join(channel);
    spawn_local(async move {
        let _ = api::act("POST", &format!("/api/channels/{channel}/ring")).await;
    });
}
