//! The app shell: sign-in gate, the four-column layout, global shortcuts and overlays.

mod call;
mod chat;
mod composer;
mod emoji;
pub mod icons;
mod login;
mod members;
mod menu;
mod modals;
mod rail;
mod settings;
mod sidebar;

use leptos::ev;
use leptos::prelude::*;
use natter_proto::{ChannelKind, Id, Status};

use crate::store::{Conn, Modal, Store, store};
use crate::{api, desktop, gateway, util, voice};
use icons::Icon;

#[component]
pub fn App() -> impl IntoView {
    let s = Store::init();
    // App-wide signals must be born under this root owner, or they die with the first component that touches them.
    voice::tracks_version();
    emoji::init();
    composer::external();
    api::set_server(&util::storage_get("natter.server").unwrap_or_else(api::default_server));
    let token = util::storage_get("natter.token");
    api::set_token(token.clone());
    voice::init();
    desktop::init();
    gateway::start_background_timers();
    if token.is_some() {
        gateway::connect();
    }
    let signed_in = RwSignal::new(token.is_some());
    // Sign-out (or an expired token) clears `me` without a `ready`; follow it back to the sign-in screen.
    Effect::new(move |_| {
        if api::token().is_none() && !s.ready.get() {
            signed_in.set(false);
        }
    });
    global_handlers();

    // Taskbar badge = unread mentions.
    Effect::new(move |_| {
        if s.ready.get() {
            desktop::set_badge(s.total_mentions());
        }
    });

    let theme = move || {
        let t = s.settings.with(|st| st.theme.clone());
        let light = t == "light"
            || (t == "system"
                && util::window()
                    .match_media("(prefers-color-scheme: light)")
                    .ok()
                    .flatten()
                    .is_some_and(|m| m.matches()));
        if light { "app light" } else { "app dark" }
    };

    view! {
        <div class=theme class:compact=move || s.settings.with(|st| st.compact)
            style:font-size=move || format!("{}px", s.settings.with(|st| st.font_size))>
            {move || {
                if !signed_in.get() {
                    view! { <login::Login on_done=Callback::new(move |_| signed_in.set(true))/> }.into_any()
                } else if !s.ready.get() {
                    view! { <Loading/> }.into_any()
                } else {
                    view! { <Shell/> }.into_any()
                }
            }}
            <Toasts/>
        </div>
    }
}

#[component]
fn Loading() -> impl IntoView {
    let s = store();
    view! {
        <div class="loading">
            <div class="loading-mark"><icons::Logo size=72/></div>
            <div class="loading-text">
                {move || match s.conn.get() {
                    Conn::Retrying(_) => "Can't reach the server — retrying…",
                    _ => "Connecting…",
                }}
            </div>
            <button class="btn link" on:click=|_| crate::store::sign_out()>"Sign out"</button>
        </div>
    }
}

#[component]
fn Shell() -> impl IntoView {
    let s = store();
    let show_members = move || {
        let Some(c) = s.nav.get().channel.and_then(|c| s.channel(c)) else { return false };
        s.settings.with(|st| st.show_members) && s.search.with(|r| r.is_none()) && s.thread.with(|t| t.is_none()) && (c.space_id.is_some() || c.kind == ChannelKind::GroupDm) && c.kind != ChannelKind::Voice
    };
    view! {
        <div class="shell" on:click=move |_| {
            s.menu.set(None);
            s.popout.set(None);
        }>
            <rail::Rail/>
            <sidebar::Sidebar/>
            <main class="main">
                <ConnBanner/>
                <chat::MainView/>
            </main>
            <Show when=show_members>
                <members::MemberList/>
            </Show>
        </div>
        <modals::ModalHost/>
        <menu::MenuHost/>
        <members::PopoutHost/>
        <Ringing/>
    }
}

#[component]
fn ConnBanner() -> impl IntoView {
    let s = store();
    view! {
        <Show when=move || s.conn.get() != Conn::Online>
            <div class="banner warn">
                {move || match s.conn.get() {
                    Conn::Retrying(at) => {
                        let secs = ((at - util::now()) / 1000).max(0);
                        format!("Connection lost. Retrying in {secs}s… Messages you send are kept and sent when we're back.")
                    }
                    _ => "Reconnecting…".to_string(),
                }}
                <button class="btn small" on:click=|_| gateway::reconnect_now()>"Reconnect now"</button>
            </div>
        </Show>
    }
}

#[component]
fn Toasts() -> impl IntoView {
    let s = store();
    view! {
        <div class="toasts">
            <For each=move || s.toasts.get() key=|t| t.id let:t>
                <div class="toast" class:error=t.error>{t.text}</div>
            </For>
        </div>
    }
}

#[component]
fn Ringing() -> impl IntoView {
    let s = store();
    view! {
        {move || s.ringing.get().map(|(channel, caller)| {
            view! {
                <div class="ringing">
                    <Avatar user=caller size=80 ring=true/>
                    <div class="ringing-name">{s.name(caller, None)}</div>
                    <div class="ringing-sub">"Incoming call…"</div>
                    <div class="ringing-actions">
                        <button class="round danger" title="Decline" on:click=move |_| {
                            s.ringing.set(None);
                            crate::sound::ring(false);
                        }><Icon name="hangup"/></button>
                        <button class="round ok" title="Answer" on:click=move |_| {
                            crate::store::open_channel(channel);
                            voice::join(channel);
                        }><Icon name="phone"/></button>
                    </div>
                </div>
            }
        })}
    }
}

/// Round avatar with an optional status dot.
#[component]
pub fn Avatar(
    user: Id,
    #[prop(optional)] size: Option<u32>,
    #[prop(optional)] status: bool,
    #[prop(optional)] ring: bool,
) -> impl IntoView {
    let s = store();
    let size = size.unwrap_or(40);
    let pic = move || s.user(user).and_then(|u| u.avatar).map(|a| api::url(&a));
    let name = move || s.user(user).map(|u| u.display_name).unwrap_or_default();
    let speaking = move || ring && s.call.with(|c| c.speaking.contains(&user));
    view! {
        <div class="avatar" class:speaking=speaking style:width=format!("{size}px") style:height=format!("{size}px")
            style:font-size=format!("{}px", size * 2 / 5)>
            {move || match pic() {
                Some(src) => view! { <img src=src alt="" draggable="false"/> }.into_any(),
                None => view! {
                    <div class="initials" style:background=util::avatar_color(user)>{util::initials(&name())}</div>
                }.into_any(),
            }}
            {status.then(|| view! { <StatusDot user=user/> })}
        </div>
    }
}

#[component]
pub fn StatusDot(user: Id) -> impl IntoView {
    let s = store();
    view! {
        <span class=move || format!("status-dot {}", status_class(s.status(user)))
            title=move || status_label(s.status(user))></span>
    }
}

pub fn status_class(st: Status) -> &'static str {
    match st {
        Status::Online => "online",
        Status::Idle => "idle",
        Status::Dnd => "dnd",
        Status::Invisible | Status::Offline => "offline",
    }
}

pub fn status_label(st: Status) -> &'static str {
    match st {
        Status::Online => "Online",
        Status::Idle => "Idle",
        Status::Dnd => "Do Not Disturb",
        Status::Invisible => "Invisible",
        Status::Offline => "Offline",
    }
}

fn typing_target(e: &web_sys::KeyboardEvent) -> bool {
    use wasm_bindgen::JsCast;
    e.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok())
        .is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT") || el.is_content_editable())
}

/// Keyboard shortcuts, focus tracking, idle detection, push-to-talk inside the window.
fn global_handlers() {
    let s = store();
    let _ = window_event_listener(ev::keydown, move |e| {
        gateway::activity();
        s.shift.set(e.shift_key());
        let ctrl = e.ctrl_key() || e.meta_key();
        let key = e.key();
        // In-window push-to-talk (the desktop app also has a global one).
        let prefs = s.prefs.get_untracked();
        if prefs.push_to_talk && e.code() == prefs.ptt_key && !typing_target(&e) {
            voice::ptt(true);
            e.prevent_default();
            return;
        }
        if !s.ready.get_untracked() {
            return;
        }
        match (ctrl, e.shift_key(), e.alt_key(), key.as_str()) {
            (true, false, false, "k") | (true, false, false, "t") => {
                e.prevent_default();
                s.menu.set(None);
                s.modal.set(Some(Modal::QuickSwitcher));
            }
            (true, false, false, "/") => {
                e.prevent_default();
                s.modal.set(Some(Modal::Shortcuts));
            }
            (true, true, false, "M") | (true, true, false, "m") => {
                e.prevent_default();
                voice::toggle_mute();
            }
            (true, true, false, "D") | (true, true, false, "d") => {
                e.prevent_default();
                voice::toggle_deafen();
            }
            (true, false, false, ",") => {
                e.prevent_default();
                s.modal.set(Some(Modal::UserSettings));
            }
            (true, false, false, "f") => {
                e.prevent_default();
                util::focus("search-input");
            }
            (true, false, false, "p") => {
                e.prevent_default();
                if let Some(c) = s.nav.get_untracked().channel {
                    s.modal.set(Some(Modal::Pins(c)));
                }
            }
            (false, false, true, "ArrowUp") | (false, false, true, "ArrowDown") => {
                e.prevent_default();
                step_channel(key == "ArrowDown", false);
            }
            (false, true, true, "ArrowUp") | (false, true, true, "ArrowDown") => {
                e.prevent_default();
                step_channel(key == "ArrowDown", true);
            }
            (false, true, false, "Escape") => {
                if let Some(sp) = s.nav.get_untracked().space {
                    crate::store::mark_space_read(sp);
                }
            }
            (false, false, false, "Escape") => {
                if s.menu.get_untracked().is_some() || s.popout.get_untracked().is_some() {
                    s.menu.set(None);
                    s.popout.set(None);
                } else if s.modal.get_untracked().is_some() {
                    s.modal.set(None);
                } else if s.search.get_untracked().is_some() {
                    s.search.set(None);
                }
            }
            _ => {}
        }
    });
    let _ = window_event_listener(ev::keyup, move |e| {
        s.shift.set(e.shift_key());
        let prefs = s.prefs.get_untracked();
        if prefs.push_to_talk && e.code() == prefs.ptt_key {
            voice::ptt(false);
        }
    });
    let _ = window_event_listener(ev::mousedown, |_| gateway::activity());
    let _ = window_event_listener(ev::focus, move |_| {
        s.focused.set(true);
        gateway::activity();
    });
    let _ = window_event_listener(ev::blur, move |_| {
        s.focused.set(false);
        s.shift.set(false);
    });
    let _ = window_event_listener(ev::online, |_| gateway::reconnect_now());
    // Leaving the page (closing the browser tab) ends the call cleanly.
    let _ = window_event_listener(ev::beforeunload, |_| voice::hang_up_local());
}

/// Alt+↑/↓ moves through channels; with Shift it jumps to the next unread one.
fn step_channel(down: bool, unread_only: bool) {
    let s = store();
    let nav = s.nav.get_untracked();
    let list: Vec<natter_proto::Channel> = match nav.space {
        Some(sp) => sidebar::ordered_channels(&s.space(sp).map(|x| x.channels).unwrap_or_default())
            .into_iter()
            .filter(|c| c.kind == ChannelKind::Text)
            .collect(),
        None => sidebar::sorted_dms(),
    };
    if list.is_empty() {
        return;
    }
    let pos = list.iter().position(|c| Some(c.id) == nav.channel);
    let n = list.len();
    let mut i = pos.unwrap_or(if down { n - 1 } else { 0 });
    for _ in 0..n {
        i = if down { (i + 1) % n } else { (i + n - 1) % n };
        let c = &list[i];
        if !unread_only || s.unread(c) || s.mentions(c.id) > 0 {
            crate::store::open_channel(c.id);
            return;
        }
    }
}
