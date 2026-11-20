//! The call stage: a tile per person (camera or avatar), screen shares, focus mode and controls.

use leptos::html;
use leptos::prelude::*;
use natter_proto::{Id, VoiceState};

use super::Avatar;
use super::icons::Icon;
use crate::store::store;
use crate::voice;

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
    let fullscreen = move |_| {
        if let Some(el) = stage.get_untracked() {
            let _ = el.request_fullscreen();
        }
    };
    view! {
        <div class="call" node_ref=stage>
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
                <button class="round" class:on=move || s.call.with(|c| c.screen) title="Share your screen" on:click=|_| voice::toggle_screen()><Icon name="screen"/></button>
                <button class="round danger" title="Leave call" on:click=|_| voice::leave()><Icon name="hangup"/></button>
                <button class="round ghost" title="Full screen" on:click=fullscreen><Icon name="maximize"/></button>
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
    let toggle_focus = move |_| {
        let k = key.clone();
        s.call.update(|c| c.focus = if c.focus.as_deref() == Some(&k) { None } else { Some(k) });
    };
    view! {
        <div class="tile" class:big=big class:speaking=speaking class:screen=kind == "screen" class:mirror=user == me && kind == "cam"
            on:click=toggle_focus title="Click to focus">
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
