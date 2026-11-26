//! The call stage: a tile per person (camera or avatar), screen shares, focus mode and controls.

use std::time::Duration;

use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use laklak_proto::{Id, VoiceState};

use super::Avatar;
use super::icons::Icon;
use crate::store::{StreamPrefs, store};
use crate::{util, voice};

#[derive(Clone, PartialEq)]
struct Tile {
    key: String,
    user: Id,
    /// "cam", "screen" or "" (avatar only).
    kind: &'static str,
    muted: bool,
}

fn tiles(people: &[VoiceState], me: Id) -> Vec<Tile> {
    let mut v = vec![];
    for p in people {
        if p.screen {
            v.push(Tile { key: format!("{}:screen", p.user_id), user: p.user_id, kind: "screen", muted: false });
        }
        let kind = if p.video { "cam" } else { "" };
        v.push(Tile { key: format!("{}:{}", p.user_id, if kind.is_empty() { "avatar" } else { kind }), user: p.user_id, kind, muted: p.muted || p.deafened });
    }
    // Your own tile first, like a mirror.
    v.sort_by_key(|t| (t.user != me, t.kind != "screen"));
    v
}

#[component]
pub fn CallView(channel: Id) -> impl IntoView {
    let s = store();
    let stage = NodeRef::<html::Div>::new();
    let list = Memo::new(move |_| {
        let me = s.me_id();
        let mut people = s.voice_in(channel);
        // Show yourself right away, before the server's voice state arrives.
        if !people.iter().any(|p| p.user_id == me) {
            let c = s.call.get();
            people.insert(0, VoiceState { user_id: me, channel_id: channel, muted: c.muted, deafened: c.deafened, video: c.video, screen: c.screen, joined_at: 0 });
        } else {
            let c = s.call.get();
            if let Some(p) = people.iter_mut().find(|p| p.user_id == me) {
                p.video = c.video;
                p.screen = c.screen;
                p.muted = c.muted;
                p.deafened = c.deafened;
            }
        }
        tiles(&people, me)
    });
    let focus = Memo::new(move |_| {
        let f = s.call.with(|c| c.focus.clone());
        f.filter(|k| list.with(|l| l.iter().any(|t| &t.key == k)))
    });
    let others = move || -> Vec<Tile> {
        let f = focus.get();
        list.get().into_iter().filter(|t| Some(&t.key) != f.as_ref()).collect()
    };
    // Full screen follows the browser's own state, so leaving it with Esc (or the OS) flips the button back too.
    let full = RwSignal::new(document().fullscreen_element().is_some());
    let fs_listener = window_event_listener_untyped("fullscreenchange", move |_| full.set(document().fullscreen_element().is_some()));
    on_cleanup(move || fs_listener.remove());
    let fullscreen = move |_| {
        if document().fullscreen_element().is_some() {
            document().exit_fullscreen();
        } else if let Some(el) = stage.get_untracked() {
            let _ = el.request_fullscreen();
        }
    };
    // Fill window: the call covers the whole window (sidebars and all) without going full screen. Esc goes back.
    let theater = RwSignal::new(false);
    let esc = window_event_listener(leptos::ev::keydown, move |e| {
        if e.key() == "Escape" && theater.get_untracked() {
            theater.set(false);
        }
    });
    on_cleanup(move || esc.remove());
    view! {
        <div class="call" class:theater=move || theater.get() node_ref=stage>
            <div class="call-stage" class:focused=move || focus.get().is_some()>
                {move || focus.get().and_then(|k| list.get().into_iter().find(|t| t.key == k)).map(|t| view! {
                    <div class="call-focus"><TileView t=t big=true/></div>
                })}
                <div class="call-grid" class:strip=move || focus.get().is_some() data-count=move || list.with(|l| l.len()).to_string()>
                    <For each=others key=|t| (t.key.clone(), t.muted) let:t>
                        <TileView t=t big=false/>
                    </For>
                </div>
            </div>
            <div class="call-controls">
                <button class="round" class:off=move || s.call.with(|c| c.muted) title="Mute (Ctrl+Shift+M)" on:click=|_| voice::toggle_mute()>
                    {move || view! { <Icon name=if s.call.with(|c| c.muted) { "mic-off" } else { "mic" }/> }}
                </button>
                <button class="round" class:off=move || s.call.with(|c| c.deafened) title="Deafen (Ctrl+Shift+D)" on:click=|_| voice::toggle_deafen()>
                    {move || view! { <Icon name=if s.call.with(|c| c.deafened) { "headphones-off" } else { "headphones" }/> }}
                </button>
                <button class="round" class:on=move || s.call.with(|c| c.video) title="Camera" on:click=|_| voice::toggle_camera()><Icon name="video"/></button>
                <button class="round" class:on=move || s.call.with(|c| c.screen)
                    title=move || if s.call.with(|c| c.screen) { "Stop sharing" } else { "Share your screen" } on:click=|_| voice::screen_button()><Icon name="screen"/></button>
                <Show when=move || s.call.with(|c| c.screen)>
                    <button class="round" title="Stream settings: quality, frame rate, bitrate" on:click=move |_| s.modal.set(Some(crate::store::Modal::GoLive))><Icon name="gear"/></button>
                </Show>
                <button class="round danger" title="Leave call" on:click=|_| voice::leave()><Icon name="hangup"/></button>
                <button class="round ghost" title=move || if theater.get() { "Exit fill window (Esc)" } else { "Fill window" }
                    on:click=move |_| theater.update(|t| *t = !*t)>
                    {move || view! { <Icon name=if theater.get() { "shrink" } else { "expand" }/> }}
                </button>
                <button class="round ghost" title=move || if full.get() { "Exit full screen" } else { "Full screen" } on:click=fullscreen>
                    {move || view! { <Icon name=if full.get() { "minimize" } else { "maximize" }/> }}
                </button>
            </div>
            <Show when=move || s.call.with(|c| c.connecting)>
                <div class="call-connecting"><div class="spinner"></div>"Connecting…"</div>
            </Show>
            <Show when=move || store().prefs.with(|p| p.push_to_talk)>
                <div class="ptt-hint" class:live=move || s.call.with(|c| c.ptt_down)>
                    {move || if s.call.with(|c| c.ptt_down) { "Talking".to_string() } else { format!("Push to talk: hold {}", store().prefs.with(|p| p.ptt_key.clone())) }}
                </div>
            </Show>
        </div>
    }
}

#[component]
fn TileView(t: Tile, big: bool) -> impl IntoView {
    let s = store();
    let me = s.me_id();
    let video = NodeRef::<html::Video>::new();
    let user = t.user;
    let kind = t.kind;
    let key = t.key.clone();
    let src_key = if user == me && !kind.is_empty() { format!("local:{kind}") } else { format!("{user}:{kind}") };
    let has_video = !kind.is_empty();
    let live = RwSignal::new(false);
    let k2 = src_key.clone();
    Effect::new(move |_| {
        voice::tracks_version().track();
        if let Some(el) = video.get() {
            voice::attach(&el, &k2);
            live.set(voice::has(&k2));
        }
    });
    let speaking = move || s.call.with(|c| c.speaking.contains(&user));
    // Hovering a camera or screen shows what it's doing (what you receive, or for your own what you send).
    let hovered = RwSignal::new(false);
    let info = RwSignal::new(None::<voice::Stats>);
    if has_video {
        let k3 = src_key.clone();
        if let Ok(h) = set_interval_with_handle(move || {
            if hovered.get_untracked() {
                let k = k3.clone();
                spawn_local(async move {
                    let st = voice::stats(&k).await;
                    info.try_set(st);
                });
            }
        }, Duration::from_millis(1000)) {
            on_cleanup(move || h.clear());
        }
    }
    let toggle_focus = move |_| {
        let k = key.clone();
        s.call.update(|c| c.focus = if c.focus.as_deref() == Some(&k) { None } else { Some(k) });
    };
    view! {
        <div class="tile" class:big=big class:speaking=speaking class:screen=kind == "screen" class:mirror=user == me && kind == "cam"
            on:click=toggle_focus title="Click to focus"
            on:mouseenter=move |_| hovered.set(true) on:mouseleave=move |_| hovered.set(false)>
            <Show when=move || hovered.get() && info.with(|i| i.as_ref().is_some_and(|i| i.height > 0))>
                <div class="tile-stats">{move || info.get().map(|st| stats_line(&st))}</div>
            </Show>
            {(user != me && has_video).then(|| {
                let (k, k2) = (src_key.clone(), src_key.clone());
                let picked = Memo::new(move |_| s.call.with(|c| c.layers.get(&k).cloned().unwrap_or_else(|| "auto".into())));
                view! {
                    <select class="tile-quality" title="Quality you receive" on:click=|e| e.stop_propagation()
                        on:change=move |e| voice::set_layer(&k2, &event_target_value(&e))>
                        {QUALITIES.iter().map(|&(v, label)| view! {
                            <option value=v prop:selected=move || picked.get() == v>{label}</option>
                        }).collect_view()}
                    </select>
                }
            })}
            {has_video.then(|| view! {
                <video node_ref=video autoplay=true playsinline=true prop:muted=true class:hidden=move || !live.get()></video>
            })}
            <Show when=move || !has_video || !live.get()>
                <div class="tile-avatar"><Avatar user=user size=if big { 120 } else { 72 }/></div>
            </Show>
            <div class="tile-name">
                {t.muted.then(|| view! { <Icon name="mic-off" size=14/> })}
                <span>{move || s.name(user, s.nav.get().space)}</span>
                {(kind == "screen").then(|| view! { <span class="live">"LIVE"</span> })}
            </div>
        </div>
    }
}

/// What you can pick to receive of someone's camera or screen: simulcast layers (glue.js `LAYERS`). Auto goes by
/// what your connection carries.
const QUALITIES: [(&str, &str); 4] = [("auto", "Auto"), ("h", "Source"), ("m", "720p"), ("l", "360p")];

fn rate(kbps: u32) -> String {
    if kbps >= 1000 { format!("{:.1} Mbit/s", f64::from(kbps) / 1000.0) } else { format!("{kbps} kbit/s") }
}

/// "1920×1080 · 30 fps · 3.1 Mbit/s · VP8"
fn stats_line(st: &voice::Stats) -> String {
    let mut v = vec![format!("{}×{}", st.width, st.height), format!("{} fps", st.fps)];
    if st.kbps > 0 {
        v.push(rate(st.kbps));
    }
    if !st.codec.is_empty() {
        v.push(st.codec.clone());
    }
    v.join(" · ")
}

/// Your screen's height in real pixels ("Source").
fn screen_height() -> u32 {
    let w = util::window();
    let h = w.screen().ok().and_then(|s| s.height().ok()).unwrap_or(1080);
    (f64::from(h) * w.device_pixel_ratio()).round() as u32
}

/// What "Auto" sends for this height and frame rate (16:9; the same rule as `autoKbps` in glue.js).
fn auto_kbps(height: u32, fps: u32) -> u32 {
    let h = f64::from(if height == 0 { screen_height() } else { height.min(screen_height()) });
    (h * h * 16.0 / 9.0 * f64::from(fps) * 0.1 / 1000.0).clamp(1000.0, 15000.0) as u32
}

/// (name, what it's for, height, fps, keep motion smooth)
const PRESETS: [(&str, &str, u32, u32, bool); 3] = [
    ("Text & detail", "Full screen size at 15 fps. Code, documents and slides stay crisp.", 0, 15, false),
    ("Balanced", "1080p at 30 fps. Right for most things.", 1080, 30, false),
    ("Smooth motion", "720p at 60 fps. Games and videos.", 720, 60, true),
];
const HEIGHTS: [(u32, &str); 5] = [(480, "480p"), (720, "720p"), (1080, "1080p"), (1440, "1440p"), (0, "Source")];
const RATES: [u32; 4] = [5, 15, 30, 60];

/// Screen share quality, like Discord's Go Live window, but every setting can change while you're live (no
/// picking the window again) and it shows what actually goes out and what holds it back.
#[component]
pub fn GoLive() -> impl IntoView {
    let s = store();
    let live = move || s.call.with(|c| c.screen);
    let p = move || s.prefs.with(|p| p.stream.clone());
    let update = move |f: Box<dyn FnOnce(&mut StreamPrefs)>| {
        s.prefs.update(|p| f(&mut p.stream));
        s.save_prefs();
        // Also before going live: the capture itself asks for this frame rate.
        voice::apply_stream();
    };
    let screen_h = screen_height();
    let stats = RwSignal::new(None::<voice::Stats>);
    if let Ok(h) = set_interval_with_handle(move || {
        if s.call.with_untracked(|c| c.screen) {
            spawn_local(async move {
                let st = voice::stats("local:screen").await;
                stats.try_set(st);
            });
        }
    }, Duration::from_millis(1000)) {
        on_cleanup(move || h.clear());
    }
    let close = move || s.modal.set(None);
    let held_back = move || stats.get().and_then(|st| match st.limit.as_str() {
        "bandwidth" => Some("Held back by bitrate: your upload speed or the bitrate setting. Raise the bitrate, or lower the resolution or frame rate."),
        "cpu" => Some("Held back by this computer: it can't encode that much. Try a lower resolution or frame rate."),
        "other" => Some("Held back by the browser."),
        _ => None,
    });
    view! {
        <h2>{move || if live() { "Stream settings" } else { "Share your screen" }}</h2>
        <p class="modal-text">{move || if live() {
            "Changes apply right away; you don't have to pick the window again."
        } else {
            "Pick how it should look, then choose the screen or window to share."
        }}</p>
        <div class="choice-row golive-presets">
            {PRESETS.iter().map(|&(name, what, h, fps, motion)| view! {
                <button class="choice" class:active=move || { let x = p(); x.height == h && x.fps == fps && x.motion == motion && x.kbps == 0 }
                    on:click=move |_| update(Box::new(move |x| { x.height = h; x.fps = fps; x.motion = motion; x.kbps = 0; }))>
                    <b>{name}</b><span class="muted small">{what}</span>
                </button>
            }).collect_view()}
        </div>
        <div class="golive-grid">
            <span class="golive-label">"Resolution"</span>
            <div class="seg">
                {HEIGHTS.iter().map(|&(h, label)| {
                    let too_big = h != 0 && h > screen_h;
                    view! {
                        <button class:active=move || p().height == h prop:disabled=too_big
                            title=if too_big { "Bigger than your screen" } else { "" }
                            on:click=move |_| update(Box::new(move |x| x.height = h))>{label}</button>
                    }
                }).collect_view()}
            </div>
            <span class="golive-label">"Frame rate"</span>
            <div class="seg">
                {RATES.iter().map(|&f| view! {
                    <button class:active=move || p().fps == f on:click=move |_| update(Box::new(move |x| x.fps = f))>{format!("{f} fps")}</button>
                }).collect_view()}
            </div>
            <span class="golive-label">"Bitrate"</span>
            <div class="slider-row">
                <input type="range" min="0" max="20000" step="500" prop:value=move || p().kbps.to_string()
                    on:input=move |e| { let v = event_target_value(&e).parse().unwrap_or(0); update(Box::new(move |x| x.kbps = v)) }/>
                <span class="slider-value wide">{move || {
                    let x = p();
                    if x.kbps == 0 { format!("Auto · {}", rate(auto_kbps(x.height, x.fps))) } else { rate(x.kbps) }
                }}</span>
            </div>
            <span class="golive-label">"When it can't keep up"</span>
            <div class="seg">
                <button class:active=move || !p().motion on:click=move |_| update(Box::new(|x| x.motion = false))
                    title="Fewer frames, every one sharp: text stays readable">"Keep it sharp"</button>
                <button class:active=move || p().motion on:click=move |_| update(Box::new(|x| x.motion = true))
                    title="Lower resolution, frames keep coming: motion stays fluid">"Keep it smooth"</button>
            </div>
        </div>
        <label class="check"><input type="checkbox" prop:checked=move || p().audio prop:disabled=live
            on:change=move |e| { let v = event_target_checked(&e); update(Box::new(move |x| x.audio = v)) }/>
            {move || if live() { "Share audio (change it by sharing again)" } else { "Share audio: the sound of the screen or window, where your system allows it" }}</label>
        <Show when=live>
            <div class="golive-stats">
                <b>"Live now"</b>
                <span>{move || stats.get().filter(|st| st.height > 0).map(|st| format!("Sending {}", stats_line(&st))).unwrap_or_else(|| "Measuring…".into())}</span>
                {move || held_back().map(|t| view! { <span class="warn">{t}</span> })}
            </div>
        </Show>
        <div class="modal-foot">
            {move || if live() {
                view! {
                    <button class="btn danger-link" on:click=move |_| { close(); voice::toggle_screen(); }>"Stop sharing"</button>
                    <button class="btn primary" on:click=move |_| close()>"Done"</button>
                }.into_any()
            } else {
                view! {
                    <button class="btn" on:click=move |_| close()>"Cancel"</button>
                    <button class="btn primary" on:click=move |_| { close(); voice::toggle_screen(); }><Icon name="screen" size=18/>" Go Live"</button>
                }.into_any()
            }}
        </div>
    }
}
