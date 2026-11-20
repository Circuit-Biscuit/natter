//! Second column: the space's channels (or your DMs), the call panel and your user panel.

use leptos::prelude::*;
use natter_proto::{Channel, ChannelKind, Id, Status, VoiceState, perm};

use super::icons::Icon;
use super::{Avatar, menu};
use crate::store::{MenuItem, Modal, open_channel, set_status, store};
use crate::voice;

/// Channels in display order: loose channels first, then each category followed by its children.
pub fn ordered_channels(chans: &[Channel]) -> Vec<Channel> {
    let mut loose: Vec<&Channel> =
        chans.iter().filter(|c| c.kind != ChannelKind::Category && c.parent_id.is_none_or(|p| !chans.iter().any(|x| x.id == p))).collect();
    loose.sort_by_key(|c| (c.kind == ChannelKind::Voice, c.position, c.id));
    let mut cats: Vec<&Channel> = chans.iter().filter(|c| c.kind == ChannelKind::Category).collect();
    cats.sort_by_key(|c| (c.position, c.id));
    let mut out: Vec<Channel> = loose.into_iter().cloned().collect();
    for cat in cats {
        out.push(cat.clone());
        let mut kids: Vec<&Channel> = chans.iter().filter(|c| c.parent_id == Some(cat.id) && c.kind != ChannelKind::Category).collect();
        kids.sort_by_key(|c| (c.kind == ChannelKind::Voice, c.position, c.id));
        out.extend(kids.into_iter().cloned());
    }
    out
}

pub fn sorted_dms() -> Vec<Channel> {
    let mut v = store().dms.get();
    v.sort_by_key(|c| std::cmp::Reverse(c.last_message_id.unwrap_or(c.id)));
    v
}

#[component]
pub fn Sidebar() -> impl IntoView {
    let s = store();
    let space = Memo::new(move |_| s.nav.get().space);
    view! {
        <aside class="sidebar">
            {move || match space.get() {
                Some(sp) => view! { <SpaceChannels space=sp/> }.into_any(),
                None => view! { <DmList/> }.into_any(),
            }}
            <VoicePanel/>
            <UserPanel/>
        </aside>
    }
}

#[component]
fn SpaceChannels(space: Id) -> impl IntoView {
    let s = store();
    let name = move || s.space(space).map(|sp| sp.name).unwrap_or_default();
    let is_mod = move || s.can(Some(space), perm::MANAGE_CHANNELS);
    let rows = move || ordered_channels(&s.space(space).map(|sp| sp.channels).unwrap_or_default());
    let collapsed = move |cat: Id| s.settings.with(|st| st.collapsed.contains(&cat));
    view! {
        <header class="sidebar-head clickable" on:click=move |e| {
            let el: web_sys::Element = event_target(&e);
            let r = el.get_bounding_client_rect();
            e.stop_propagation();
            s.menu.set(Some(crate::store::Menu { x: r.left() as i32 + 8, y: r.bottom() as i32 + 4, items: menu::space_menu(space) }));
        }>
            <span class="ellipsis">{name}</span>
            <Icon name="chevron" size=18/>
        </header>
        <div class="channel-list scroll">
            <For each=rows key=|c| (c.id, c.name.clone(), c.kind, c.parent_id, c.position, c.read_only, c.last_message_id) let:c>
                {
                    let id = c.id;
                    if c.kind == ChannelKind::Category {
                        let cname = c.name.clone();
                        view! {
                            <div class="category" on:click=move |_| s.update_settings(|st| {
                                if let Some(i) = st.collapsed.iter().position(|&x| x == id) { st.collapsed.remove(i); } else { st.collapsed.push(id); }
                            }) on:contextmenu=move |e| s.open_menu(&e, menu::channel_menu(id))>
                                <span class="chev" class:closed=move || collapsed(id)><Icon name="chevron" size=12/></span>
                                <span class="ellipsis">{cname.to_uppercase()}</span>
                                <Show when=is_mod>
                                    <button class="icon-btn" title="Create channel" on:click=move |e| {
                                        e.stop_propagation();
                                        s.modal.set(Some(Modal::CreateChannel { space, kind: ChannelKind::Text, parent: Some(id) }));
                                    }><Icon name="plus" size=16/></button>
                                </Show>
                            </div>
                        }.into_any()
                    } else {
                        let parent = c.parent_id;
                        let hidden = move || parent.is_some_and(|p| collapsed(p))
                            && s.nav.get().channel != Some(id)
                            && !s.channel(id).is_some_and(|c| s.unread(&c) && !s.muted(id))
                            && s.voice_in(id).is_empty();
                        view! { <Show when=move || !hidden()><ChannelRow channel=c.clone()/></Show> }.into_any()
                    }
                }
            </For>
        </div>
    }
}

#[component]
fn ChannelRow(channel: Channel) -> impl IntoView {
    let s = store();
    let id = channel.id;
    let kind = channel.kind;
    let icon = match (kind, channel.read_only) {
        (ChannelKind::Voice, _) => "speaker",
        (_, true) => "megaphone",
        _ => "hash",
    };
    let active = move || s.nav.get().channel == Some(id);
    let unread = move || !s.muted(id) && s.channel(id).is_some_and(|c| s.unread(&c));
    let mentions = move || s.mentions(id);
    let muted = move || s.muted(id);
    let in_call = move || s.call.with(|c| c.channel == Some(id));
    let name = channel.name.clone();
    view! {
        <div class="channel" class:active=active class:unread=unread class:muted=muted class:voice=kind == ChannelKind::Voice
            on:click=move |_| {
                if kind == ChannelKind::Voice {
                    if !in_call() { voice::join(id); }
                }
                open_channel(id);
            }
            on:contextmenu=move |e| s.open_menu(&e, menu::channel_menu(id))>
            <span class="unread-pill"></span>
            <Icon name=icon size=20/>
            <span class="ellipsis name">{name}</span>
            {move || s.is_private(id).then(|| view! { <span class="private-lock" title="Private channel"><Icon name="lock" size=12/></span> })}
            {move || (mentions() > 0).then(|| view! { <span class="badge">{mentions()}</span> })}
            <Show when=move || s.channel(id).is_some_and(|c| c.kind != ChannelKind::Thread) && s.can_in(id, perm::MANAGE_CHANNELS)>
                <button class="icon-btn hover-only" title="Edit channel" on:click=move |e| {
                    e.stop_propagation();
                    s.modal.set(Some(Modal::EditChannel(id)));
                }><Icon name="gear" size=16/></button>
            </Show>
        </div>
        {(kind == ChannelKind::Voice).then(|| view! { <VoiceMembers channel=id/> })}
        {(kind == ChannelKind::Text).then(|| view! { <ThreadRows parent=id/> })}
    }
}

/// Active threads, tucked under their channel with a little spine.
#[component]
fn ThreadRows(parent: Id) -> impl IntoView {
    let s = store();
    view! {
        <For each=move || s.threads_of(parent) key=|t| (t.id, t.name.clone()) let:t>
            {
                let id = t.id;
                let unread = move || !s.muted(id) && s.channel(id).is_some_and(|c| s.unread(&c));
                view! {
                    <div class="channel thread-row" class:active=move || s.thread.get() == Some(id) class:unread=unread
                        on:click=move |_| open_channel(id)
                        on:contextmenu=move |e| s.open_menu(&e, menu::channel_menu(id))>
                        <span class="thread-spine"></span>
                        <span class="ellipsis name">{move || s.channel(id).map(|c| c.name).unwrap_or_default()}</span>
                        {move || (s.mentions(id) > 0).then(|| view! { <span class="badge">{s.mentions(id)}</span> })}
                    </div>
                }
            }
        </For>
    }
}

#[component]
pub fn VoiceMembers(channel: Id) -> impl IntoView {
    let s = store();
    view! {
        <div class="voice-members">
            <For each=move || s.voice_in(channel) key=|v: &VoiceState| (v.user_id, v.muted, v.deafened, v.video, v.screen) let:v>
                {
                    let u = v.user_id;
                    view! {
                        <div class="voice-member" on:click=move |e| s.open_profile(&e, u)
                            on:contextmenu=move |e| s.open_menu(&e, menu::user_menu(u, s.nav.get_untracked().space))>
                            <Avatar user=u size=24 ring=true/>
                            <span class="ellipsis" class:speaking=move || s.call.with(|c| c.speaking.contains(&u))>{move || s.name(u, s.nav.get().space)}</span>
                            {v.screen.then(|| view! { <span class="live">"LIVE"</span> })}
                            {v.video.then(|| view! { <Icon name="video" size=14/> })}
                            {(v.muted && !v.deafened).then(|| view! { <span class="vm-icon"><Icon name="mic-off" size=14/></span> })}
                            {v.deafened.then(|| view! { <span class="vm-icon"><Icon name="headphones-off" size=14/></span> })}
                        </div>
                    }
                }
            </For>
        </div>
    }
}

#[component]
fn DmList() -> impl IntoView {
    let s = store();
    view! {
        <header class="sidebar-head">
            <button class="search-like" on:click=move |_| s.modal.set(Some(Modal::QuickSwitcher))>"Find or start a conversation"</button>
        </header>
        <div class="channel-list scroll">
            <div class="category static">
                <span class="ellipsis">"DIRECT MESSAGES"</span>
                <button class="icon-btn" title="New message" on:click=move |_| s.modal.set(Some(Modal::NewDm))><Icon name="plus" size=16/></button>
            </div>
            <For each=sorted_dms key=|c| (c.id, c.recipients.len(), c.name.clone()) let:c>
                {
                    let id = c.id;
                    let partner = s.dm_partner(&c);
                    let group = c.kind == ChannelKind::GroupDm;
                    let count = c.recipients.len();
                    let unread = move || !s.muted(id) && s.channel(id).is_some_and(|c| s.unread(&c));
                    view! {
                        <div class="dm" class:active=move || s.nav.get().channel == Some(id) class:unread=unread
                            on:click=move |_| open_channel(id)
                            on:contextmenu=move |e| s.open_menu(&e, menu::channel_menu(id))>
                            {match partner {
                                Some(u) => view! { <Avatar user=u size=32 status=true/> }.into_any(),
                                None => view! { <div class="group-icon"><Icon name="members" size=18/></div> }.into_any(),
                            }}
                            <div class="dm-text">
                                <span class="ellipsis name">{move || s.channel(id).map(|c| s.channel_title(&c)).unwrap_or_default()}</span>
                                {group.then(|| view! { <span class="sub">{format!("{count} members")}</span> })}
                                {partner.map(|u| view! {
                                    <span class="sub ellipsis">{move || s.user(u).and_then(|u| u.custom_status).unwrap_or_default()}</span>
                                })}
                            </div>
                            {move || (s.mentions(id) > 0).then(|| view! { <span class="badge">{s.mentions(id)}</span> })}
                            {move || (!s.voice_in(id).is_empty()).then(|| view! { <span class="in-call" title="Call in progress"><Icon name="phone" size=14/></span> })}
                        </div>
                    }
                }
            </For>
            <Show when=move || s.dms.with(|d| d.is_empty())>
                <div class="empty-hint">"No conversations yet. Click someone's name to message them, or use + above."</div>
            </Show>
        </div>
    }
}

#[component]
fn VoicePanel() -> impl IntoView {
    let s = store();
    let channel = Memo::new(move |_| s.call.with(|c| c.channel));
    view! {
        {move || channel.get().map(|ch| {
            let title = move || {
                let c = s.channel(ch);
                match c {
                    Some(c) if c.kind.is_dm() => s.channel_title(&c),
                    Some(c) => {
                        let sp = c.space_id.and_then(|sp| s.space(sp)).map(|sp| sp.name).unwrap_or_default();
                        format!("{} / {}", c.name, sp)
                    }
                    None => String::new(),
                }
            };
            view! {
                <div class="voice-panel">
                    <div class="vp-top">
                        <div class="vp-status" class:connecting=move || s.call.with(|c| c.connecting)>
                            <Icon name="signal" size=16/>
                            {move || if s.call.with(|c| c.connecting) { "Connecting…" } else { "Voice Connected" }}
                        </div>
                        <button class="icon-btn" title="Disconnect" on:click=|_| voice::leave()><Icon name="hangup" size=20/></button>
                    </div>
                    <a class="vp-channel ellipsis" href="#" on:click=move |e| { e.prevent_default(); open_channel(ch); }>{title}</a>
                    <div class="vp-actions">
                        <button class="btn small" class:on=move || s.call.with(|c| c.video) on:click=|_| voice::toggle_camera()>
                            <Icon name="video" size=18/>"Camera"
                        </button>
                        <button class="btn small" class:on=move || s.call.with(|c| c.screen) on:click=|_| voice::toggle_screen()>
                            <Icon name="screen" size=18/>"Screen"
                        </button>
                    </div>
                </div>
            }
        })}
    }
}

#[component]
fn UserPanel() -> impl IntoView {
    let s = store();
    let me = move || s.me_id();
    let status_menu = move |e: leptos::ev::MouseEvent| {
        let items = vec![
            Some(MenuItem::new("🟢  Online", || set_status(Status::Online))),
            Some(MenuItem::new("🌙  Idle", || set_status(Status::Idle))),
            Some(MenuItem::new("⛔  Do Not Disturb — no notifications", || set_status(Status::Dnd))),
            Some(MenuItem::new("⚪  Invisible — appear offline", || set_status(Status::Invisible))),
            None,
            Some(MenuItem::new("Edit profile…", move || s.modal.set(Some(Modal::UserSettings)))),
            Some(MenuItem::new("Copy username", move || crate::util::copy_text(&s.me.get_untracked().map(|u| u.username).unwrap_or_default()))),
        ];
        s.open_menu(&e, items);
    };
    let muted = move || s.call.with(|c| c.muted);
    let deaf = move || s.call.with(|c| c.deafened);
    view! {
        <div class="user-panel">
            <div class="up-me" on:click=status_menu>
                {move || view! { <Avatar user=me() size=32 status=true ring=true/> }}
                <div class="up-text">
                    <span class="ellipsis name">{move || s.me.get().map(|u| u.display_name).unwrap_or_default()}</span>
                    <span class="ellipsis sub">{move || s.me.get().and_then(|u| u.custom_status).unwrap_or_else(|| super::status_label(s.my_status.get()).to_string())}</span>
                </div>
            </div>
            <button class="icon-btn" class:danger=muted title=move || if muted() { "Unmute (Ctrl+Shift+M)" } else { "Mute (Ctrl+Shift+M)" }
                on:click=|_| voice::toggle_mute()>
                {move || view! { <Icon name=if muted() { "mic-off" } else { "mic" }/> }}
            </button>
            <button class="icon-btn" class:danger=deaf title=move || if deaf() { "Undeafen (Ctrl+Shift+D)" } else { "Deafen (Ctrl+Shift+D)" }
                on:click=|_| voice::toggle_deafen()>
                {move || view! { <Icon name=if deaf() { "headphones-off" } else { "headphones" }/> }}
            </button>
            <button class="icon-btn" title="User settings (Ctrl+,)" on:click=move |_| s.modal.set(Some(Modal::UserSettings))><Icon name="gear"/></button>
        </div>
    }
}
