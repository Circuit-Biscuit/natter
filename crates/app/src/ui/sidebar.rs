//! Second column: the space's channels (or your DMs), the call panel and your user panel.

use leptos::prelude::*;
use leptos::task::spawn_local;
use natter_proto::{Channel, ChannelKind, Event, Id, Status, UpdateChannel, VoiceState, perm};
use wasm_bindgen::JsCast;

use super::icons::Icon;
use super::{Avatar, menu};
use crate::store::{MenuItem, Modal, SIDEBAR_WIDTH, open_channel, set_status, store};
use crate::{api, voice};

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

/// Where a drop puts `src` (before or after `target`; onto a category: at its top): the new category and the
/// new order of its neighbours (for a category: of all categories).
fn plan_move(chans: &[Channel], src: Id, target: Id, after: bool) -> Option<(Option<Id>, Vec<Id>)> {
    let get = |id: Id| chans.iter().find(|c| c.id == id);
    let is_cat = |c: &Channel| c.kind == ChannelKind::Category;
    let cat_of = |c: &Channel| c.parent_id.filter(|&p| get(p).is_some_and(is_cat));
    let (a, b) = (get(src)?, get(target)?);
    if src == target || a.kind == ChannelKind::Thread || b.kind == ChannelKind::Thread {
        return None;
    }
    let in_cat = |p: Option<Id>| chans.iter().filter(move |c| !is_cat(c) && c.kind != ChannelKind::Thread && cat_of(c) == p);
    let (parent, mut group, at): (Option<Id>, Vec<&Channel>, Option<Id>) = match (is_cat(a), is_cat(b)) {
        (true, true) => (None, chans.iter().filter(|c| is_cat(c)).collect(), Some(target)),
        (true, false) => return None,
        (false, true) => (Some(target), in_cat(Some(target)).collect(), None),
        (false, false) => (cat_of(b), in_cat(cat_of(b)).collect(), Some(target)),
    };
    group.sort_by_key(|c| (c.kind == ChannelKind::Voice, c.position, c.id));
    let mut ids: Vec<Id> = group.iter().map(|c| c.id).filter(|&id| id != src).collect();
    let i = match at {
        Some(t) => ids.iter().position(|&x| x == t)? + usize::from(after),
        None => 0,
    };
    ids.insert(i, src);
    Some((parent, ids))
}

/// Move a channel or category (drag and drop, or Move up / down in its settings): shown at once, then saved,
/// renumbering its neighbours with the existing channel update call.
pub fn move_channel(src: Id, target: Id, after: bool) {
    let s = store();
    let Some(chans) = s.channel(src).and_then(|c| c.space_id).and_then(|sp| s.space(sp)).map(|sp| sp.channels) else { return };
    let Some((parent, order)) = plan_move(&chans, src, target, after) else { return };
    let mut changed: Vec<(Channel, UpdateChannel)> = Vec::new();
    for (i, id) in order.into_iter().enumerate() {
        let Some(c) = chans.iter().find(|c| c.id == id) else { continue };
        let moved = c.kind != ChannelKind::Category && c.parent_id != parent;
        if c.position != i as i32 || moved {
            changed.push((c.clone(), UpdateChannel { position: Some(i as i32), parent_id: moved.then(|| parent.unwrap_or(0)), ..Default::default() }));
        }
    }
    for (c, u) in &changed {
        let parent_id = if u.parent_id.is_some() { parent } else { c.parent_id };
        crate::store::apply(Event::ChannelUpdate(Channel { position: u.position.unwrap_or(c.position), parent_id, ..c.clone() }));
    }
    spawn_local(async move {
        for (i, (c, u)) in changed.iter().enumerate() {
            if let Err(e) = api::call::<_, Channel>("PATCH", &format!("/api/channels/{}", c.id), u).await {
                store().error(e);
                // Put back what didn't get saved.
                for (c, _) in &changed[i..] {
                    crate::store::apply(Event::ChannelUpdate(c.clone()));
                }
                return;
            }
        }
    });
}

/// Drag and drop in the channel list: the row in hand, and where it would land (row, after it?).
#[derive(Clone, Copy)]
struct Dnd {
    dragging: RwSignal<Option<Id>>,
    over: RwSignal<Option<(Id, bool)>>,
}

impl Dnd {
    fn start(self, id: Id) -> impl Fn(web_sys::DragEvent) + 'static {
        move |e| {
            if let Some(dt) = e.data_transfer() {
                // Firefox only starts a drag that carries data.
                let _ = dt.set_data("application/x-natter-channel", &id.to_string());
                dt.set_effect_allowed("move");
            }
            self.dragging.set(Some(id));
        }
    }

    fn over(self, id: Id) -> impl Fn(web_sys::DragEvent) + 'static {
        move |e| {
            let s = store();
            let Some(src) = self.dragging.get_untracked().and_then(|x| s.channel(x)) else { return };
            let Some(target) = s.channel(id) else { return };
            let to_cat = target.kind == ChannelKind::Category;
            // Categories only go between categories.
            if src.id == id || (src.kind == ChannelKind::Category && !to_cat) {
                return;
            }
            e.prevent_default();
            let after = (to_cat && src.kind != ChannelKind::Category)
                || e.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).is_some_and(|el| {
                    let r = el.get_bounding_client_rect();
                    f64::from(e.client_y()) > r.top() + r.height() / 2.0
                });
            if self.over.get_untracked() != Some((id, after)) {
                self.over.set(Some((id, after)));
            }
        }
    }

    fn drop(self, id: Id) -> impl Fn(web_sys::DragEvent) + 'static {
        move |e| {
            e.prevent_default();
            let (src, over) = (self.dragging.get_untracked(), self.over.get_untracked());
            self.end();
            if let (Some(src), Some((target, after))) = (src, over) && target == id {
                move_channel(src, target, after);
            }
        }
    }

    fn end(self) {
        self.dragging.set(None);
        self.over.set(None);
    }

    fn line(self, id: Id, after: bool) -> impl Fn() -> bool + Copy + 'static {
        move || self.over.get() == Some((id, after))
    }
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
    // Width: dragged at the right edge (or arrow keys on the handle), saved per device; double-click resets.
    let (default_w, min_w, max_w) = SIDEBAR_WIDTH;
    let width = RwSignal::new(s.prefs.with_untracked(|p| p.sidebar_width).clamp(min_w, max_w));
    let save = move || {
        s.prefs.update(|p| p.sidebar_width = width.get_untracked());
        s.save_prefs();
    };
    let set = move |w: i32| width.set(w.clamp(i32::from(min_w), i32::from(max_w)) as u16);
    // (pointer x, width) when the drag started
    let drag = RwSignal::new(None::<(i32, i32)>);
    let mv = window_event_listener(leptos::ev::mousemove, move |e| {
        if let Some((x0, w0)) = drag.get_untracked() {
            set(w0 + e.client_x() - x0);
        }
    });
    let up = window_event_listener(leptos::ev::mouseup, move |_| {
        if drag.get_untracked().is_some() {
            drag.set(None);
            save();
        }
    });
    on_cleanup(move || {
        mv.remove();
        up.remove();
    });
    view! {
        <aside class="sidebar" style:--sidebar-w=move || format!("{}px", width.get())>
            {move || match space.get() {
                Some(sp) => view! { <SpaceChannels space=sp/> }.into_any(),
                None => view! { <DmList/> }.into_any(),
            }}
            <VoicePanel/>
            <UserPanel/>
        </aside>
        <div class="sidebar-resize" class:active=move || drag.get().is_some() tabindex="0" role="separator"
            aria-orientation="vertical" aria-label="Resize the channel list" title="Drag to resize, double-click to reset"
            aria-valuenow=move || width.get().to_string() aria-valuemin=min_w.to_string() aria-valuemax=max_w.to_string()
            on:mousedown=move |e| {
                e.prevent_default();
                drag.set(Some((e.client_x(), i32::from(width.get_untracked()))));
            }
            on:dblclick=move |_| { width.set(default_w); save(); }
            on:keydown=move |e| {
                let step = match e.key().as_str() { "ArrowLeft" => -16, "ArrowRight" => 16, _ => return };
                e.prevent_default();
                set(i32::from(width.get_untracked()) + step);
                save();
            }></div>
        // Over everything while dragging: keeps the cursor and stops videos and text from taking the mouse.
        <Show when=move || drag.get().is_some()>
            <div class="resize-cover"></div>
        </Show>
    }
}

#[component]
fn SpaceChannels(space: Id) -> impl IntoView {
    let s = store();
    let name = move || s.space(space).map(|sp| sp.name).unwrap_or_default();
    let is_mod = move || s.can(Some(space), perm::MANAGE_CHANNELS);
    let rows = move || ordered_channels(&s.space(space).map(|sp| sp.channels).unwrap_or_default());
    let collapsed = move |cat: Id| s.settings.with(|st| st.collapsed.contains(&cat));
    let dnd = Dnd { dragging: RwSignal::new(None), over: RwSignal::new(None) };
    let draggable = move || if is_mod() { "true" } else { "false" };
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
        <div class="channel-list scroll" on:dragover=move |e: web_sys::DragEvent| {
            // Not over a place it can go: no drop line.
            if !e.default_prevented() && dnd.over.get_untracked().is_some() { dnd.over.set(None); }
        }>
            <For each=rows key=|c| (c.id, c.name.clone(), c.kind, c.parent_id, c.position, c.read_only, c.last_message_id) let:c>
                {
                    let id = c.id;
                    if c.kind == ChannelKind::Category {
                        let cname = c.name.clone();
                        view! {
                            <div class="category" draggable=draggable class:dragging=move || dnd.dragging.get() == Some(id)
                                class:drop-before=dnd.line(id, false) class:drop-after=dnd.line(id, true)
                                on:dragstart=dnd.start(id) on:dragend=move |_| dnd.end() on:dragover=dnd.over(id) on:drop=dnd.drop(id)
                                on:click=move |_| s.update_settings(|st| {
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
                        view! { <Show when=move || !hidden()><ChannelRow channel=c.clone() dnd=dnd draggable=draggable/></Show> }.into_any()
                    }
                }
            </For>
        </div>
    }
}

#[component]
fn ChannelRow(channel: Channel, dnd: Dnd, draggable: impl Fn() -> &'static str + Copy + Send + Sync + 'static) -> impl IntoView {
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
            draggable=draggable class:dragging=move || dnd.dragging.get() == Some(id)
            class:drop-before=dnd.line(id, false) class:drop-after=dnd.line(id, true)
            on:dragstart=dnd.start(id) on:dragend=move |_| dnd.end() on:dragover=dnd.over(id) on:drop=dnd.drop(id)
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
                            // Click to watch: the badge turns into "WATCH" under the pointer.
                            {v.screen.then(|| view! {
                                <button class="live watch" title="Watch stream" on:click=move |e| { e.stop_propagation(); voice::watch(channel, u, "screen"); }>
                                    <span class="live-label">"LIVE"</span><span class="watch-label">"WATCH"</span>
                                </button>
                            })}
                            {v.video.then(|| view! {
                                <button class="watch-cam" title="Watch camera" on:click=move |e| { e.stop_propagation(); voice::watch(channel, u, "cam"); }>
                                    <Icon name="video" size=14/>
                                </button>
                            })}
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
                        <button class="btn small" class:on=move || s.call.with(|c| c.screen) on:click=|_| voice::screen_button()>
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
                    <span class="ellipsis name">{move || s.name(me(), s.nav.get().space)}</span>
                    <span class="ellipsis sub">{move || s.status_text(me(), s.nav.get().space).unwrap_or_else(|| super::status_label(s.my_status.get()).to_string())}</span>
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(id: Id, kind: ChannelKind, position: i32, parent_id: Option<Id>) -> Channel {
        Channel {
            id, space_id: Some(1), kind, name: String::new(), topic: String::new(), position, parent_id, read_only: false,
            last_message_id: None, recipients: vec![], thread: None, overwrites: vec![],
        }
    }

    #[test]
    fn drops() {
        use ChannelKind::*;
        let v = [
            ch(10, Category, 0, None), ch(11, Text, 0, Some(10)), ch(12, Text, 1, Some(10)), ch(13, Voice, 2, Some(10)),
            ch(20, Category, 1, None), ch(21, Text, 0, Some(20)), ch(30, Text, 0, None), ch(31, Thread, 0, Some(11)),
        ];
        // Within a category, before / after.
        assert_eq!(plan_move(&v, 12, 11, false), Some((Some(10), vec![12, 11, 13])));
        assert_eq!(plan_move(&v, 11, 12, true), Some((Some(10), vec![12, 11, 13])));
        // Into another category: next to a channel, or onto its header (top).
        assert_eq!(plan_move(&v, 11, 21, true), Some((Some(20), vec![21, 11])));
        assert_eq!(plan_move(&v, 13, 20, true), Some((Some(20), vec![13, 21])));
        // Out of categories, next to a loose channel (the thread stays out of it).
        assert_eq!(plan_move(&v, 21, 30, false), Some((None, vec![21, 30])));
        // Categories between categories only.
        assert_eq!(plan_move(&v, 20, 10, false), Some((None, vec![20, 10])));
        assert_eq!(plan_move(&v, 20, 11, false), None);
        assert_eq!(plan_move(&v, 31, 12, false), None);
    }
}
