//! The main column: channel header, message list (grouped, with unread marker and infinite scroll),
//! composer, search results and the call view.

use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use natter_proto::*;

use super::icons::Icon;
use super::{Avatar, call, composer, emoji, menu};
use crate::store::{Modal, PENDING_BASE, ack, jump_to, store};
use crate::{api, markdown, util, voice};

#[component]
pub fn MainView() -> impl IntoView {
    let s = store();
    // Only the channel's identity decides what to mount, so new messages never remount the view.
    let current = Memo::new(move |_| s.nav.get().channel.and_then(|c| s.channel(c)).map(|c| (c.id, c.kind)));
    view! {
        {move || match current.get() {
            None => view! { <Home/> }.into_any(),
            Some((id, ChannelKind::Voice)) => view! { <VoiceChannelView channel=id/> }.into_any(),
            Some((id, _)) => view! { <ChannelView channel=id/> }.into_any(),
        }}
    }
}

/// Nothing open: a calm landing page with people to talk to.
#[component]
fn Home() -> impl IntoView {
    let s = store();
    let people = move || {
        let me = s.me_id();
        let mut v: Vec<User> = s.users.with(|u| u.values().filter(|u| u.id != me).cloned().collect());
        v.sort_by_key(|u| (s.status(u.id) == Status::Offline, u.display_name.to_lowercase()));
        v
    };
    view! {
        <header class="chat-head"><Icon name="members"/><span class="title">"People"</span></header>
        <div class="home scroll">
            <Show when=move || s.spaces.with(|v| v.is_empty())>
                <div class="welcome">
                    <h2>"Welcome to Natter"</h2>
                    <p>"Create a space for your team, or join one with an invite link."</p>
                    <button class="btn primary" on:click=move |_| s.modal.set(Some(Modal::CreateSpace))>"Create or join a space"</button>
                </div>
            </Show>
            <div class="people">
                <For each=people key=|u| u.id let:u>
                    {
                        let id = u.id;
                        view! {
                            <div class="person" on:click=move |_| crate::store::open_dm(id)>
                                <Avatar user=id size=36 status=true/>
                                <div class="person-text">
                                    <span class="name">{u.display_name.clone()}</span>
                                    <span class="sub">{move || s.user(id).and_then(|u| u.custom_status).unwrap_or_else(|| super::status_label(s.status(id)).to_string())}</span>
                                </div>
                                <button class="icon-btn" title="Message" on:click=move |e| { e.stop_propagation(); crate::store::open_dm(id); }><Icon name="send" size=18/></button>
                            </div>
                        }
                    }
                </For>
            </div>
        </div>
    }
}

#[component]
fn VoiceChannelView(channel: Id) -> impl IntoView {
    let s = store();
    let show_chat = RwSignal::new(false);
    let in_call = Memo::new(move |_| s.call.with(|c| c.channel == Some(channel)));
    view! {
        <ChannelHeader channel=channel show_chat=Some(show_chat)/>
        <div class="voice-view">
            {move || if in_call.get() {
                view! { <call::CallView channel=channel/> }.into_any()
            } else {
                let people = s.voice_in(channel);
                view! {
                    <div class="call-idle">
                        <div class="call-idle-icon"><Icon name="speaker" size=48/></div>
                        <h2>{move || s.channel(channel).map(|c| c.name).unwrap_or_default()}</h2>
                        <p>{if people.is_empty() { "No one is here yet.".to_string() } else { format!("{} in the channel", people.len()) }}</p>
                        <button class="btn primary" on:click=move |_| voice::join(channel)>"Join Voice"</button>
                    </div>
                }.into_any()
            }}
            <Show when=move || show_chat.get()>
                <div class="voice-chat"><ChatPane channel=channel/></div>
            </Show>
        </div>
    }
}

#[component]
fn ChannelView(channel: Id) -> impl IntoView {
    let s = store();
    let dm_call = move || s.channel(channel).is_some_and(|c| c.kind.is_dm()) && s.call.with(|c| c.channel == Some(channel));
    let thread = Memo::new(move |_| s.thread.get().filter(|t| s.channel(*t).is_some_and(|c| c.parent_id == Some(channel))));
    view! {
        <ChannelHeader channel=channel show_chat=None/>
        <div class="chat-body">
            <div class="chat-col">
                <Show when=dm_call>
                    <div class="dm-call"><call::CallView channel=channel/></div>
                </Show>
                <ChatPane channel=channel/>
            </div>
            {move || s.search.get().map(|_| view! { <SearchPanel/> })}
            {move || thread.get().map(|t| view! { <ThreadPanel thread=t/> })}
        </div>
    }
}

#[component]
fn ChannelHeader(channel: Id, show_chat: Option<RwSignal<bool>>) -> impl IntoView {
    let s = store();
    let c = move || s.channel(channel);
    let query = RwSignal::new(String::new());
    let run_search = move || {
        let q = query.get_untracked();
        if q.trim().is_empty() {
            s.search.set(None);
            return;
        }
        let Some(ch) = c() else { return };
        let mut url = format!("/api/search?q={}&limit=50", api::enc(q.trim()));
        match ch.space_id {
            Some(sp) => url.push_str(&format!("&space_id={sp}")),
            None => url.push_str(&format!("&channel_id={}", ch.id)),
        }
        spawn_local(async move {
            match api::get::<SearchResult>(&url).await {
                Ok(r) => {
                    if s.search.with_untracked(|x| x.is_none()) {
                        crate::store::push_layer();
                    }
                    s.search.set(Some((q, r)));
                }
                Err(e) => s.error(e),
            }
        });
    };
    let icon = move || match c() {
        Some(c) if c.kind == ChannelKind::Voice => "speaker",
        Some(c) if c.kind.is_dm() => "at",
        Some(c) if c.read_only => "megaphone",
        _ => "hash",
    };
    view! {
        <header class="chat-head">
            <Show when=move || s.mobile.get()>
                <button class="icon-btn back" title="Back" on:click=|_| crate::store::go_back()><Icon name="arrow-left"/></button>
            </Show>
            {move || view! { <Icon name=icon()/> }}
            <span class="title ellipsis">{move || c().map(|c| s.channel_title(&c)).unwrap_or_default()}</span>
            {move || c().and_then(|c| s.dm_partner(&c)).map(|u| view! { <super::StatusDot user=u/> })}
            {move || c().filter(|c| !c.topic.is_empty()).map(|c| view! {
                <span class="divider"></span>
                <span class="topic ellipsis" title=c.topic.clone()>{markdown::render(&c.topic, c.space_id)}</span>
            })}
            <span class="spacer"></span>
            {move || c().filter(|c| c.kind.is_dm()).map(|c| {
                let id = c.id;
                view! {
                    <button class="icon-btn" title="Start voice call" on:click=move |_| voice::call_dm(id)><Icon name="phone"/></button>
                    <button class="icon-btn" title="Start video call" on:click=move |_| {
                        voice::call_dm(id);
                        set_timeout(voice::toggle_camera, std::time::Duration::from_millis(1500));
                    }><Icon name="video"/></button>
                }
            })}
            {show_chat.map(|sc| view! {
                <button class="icon-btn" class:on=move || sc.get() title="Show chat" on:click=move |_| sc.update(|v| *v = !*v)><Icon name="send"/></button>
            })}
            <Show when=move || c().is_some_and(|c| c.kind == ChannelKind::Text && c.space_id.is_some())>
                <button class="icon-btn" title="Threads" on:click=move |_| s.modal.set(Some(Modal::Threads(channel)))><Icon name="thread"/></button>
            </Show>
            <button class="icon-btn" title="Pinned messages (Ctrl+P)" on:click=move |_| s.modal.set(Some(Modal::Pins(channel)))><Icon name="pin"/></button>
            <Show when=move || c().is_some_and(|c| c.space_id.is_some() || c.kind == ChannelKind::GroupDm)>
                <button class="icon-btn" class:on=move || s.settings.with(|st| st.show_members) title="Member list"
                    on:click=move |_| if s.mobile.get_untracked() {
                        crate::store::push_layer();
                        s.pane.set(crate::store::Pane::Members);
                    } else {
                        s.update_settings(|st| st.show_members = !st.show_members)
                    }><Icon name="members"/></button>
            </Show>
            <div class="search-box">
                <input id="search-input" placeholder="Search" prop:value=move || query.get()
                    on:input=move |e| query.set(event_target_value(&e))
                    on:keydown=move |e| {
                        if e.key() == "Enter" { run_search(); }
                        if e.key() == "Escape" { query.set(String::new()); s.search.set(None); }
                    }/>
                <Icon name="search" size=16/>
            </div>
        </header>
    }
}

#[derive(Clone, PartialEq)]
enum Row {
    Start,
    Loading,
    Day(Millis),
    New,
    Msg { m: Message, head: bool },
}

impl Row {
    fn key(&self) -> String {
        match self {
            Row::Start => "start".into(),
            Row::Loading => "loading".into(),
            Row::Day(t) => format!("day{}", util::day_label(*t)),
            Row::New => "new".into(),
            Row::Msg { m, head } => {
                let reacts: usize = m.reactions.iter().map(|r| r.users.len() * 31 + r.emoji.len()).sum();
                let deleted = m.reply.as_ref().is_some_and(|r| r.deleted);
                let thread = m.thread.as_ref().map(|t| (t.id, t.message_count, t.archived, t.name.len()));
                format!("{}:{:?}:{}:{}:{}:{}:{}:{:?}", m.id, m.edited_at, reacts, m.reactions.len(), m.pinned, head, deleted, thread)
            }
        }
    }
}

fn starts_group(prev: Option<&Message>, m: &Message) -> bool {
    let Some(p) = prev else { return true };
    p.author_id != m.author_id
        || p.kind != MessageKind::Default
        || m.kind != MessageKind::Default
        || m.reply.is_some()
        || m.created_at - p.created_at > 7 * 60_000
        || !util::same_day(p.created_at, m.created_at)
}

#[component]
pub fn ChatPane(channel: Id) -> impl IntoView {
    let s = store();
    let list_ref = NodeRef::<html::Div>::new();
    let at_bottom = RwSignal::new(true);
    let last_height = StoredValue::new(0i32);
    let drag_over = RwSignal::new(false);
    let uploads = composer::Uploads::new();
    // Where "NEW" goes: the read marker as it was when you opened the channel.
    let marker = StoredValue::new(s.read.with_untracked(|r| r.get(&channel).map(|r| r.last_read_id).unwrap_or(0)));
    let space = s.channel(channel).and_then(|c| c.space_id);

    let rows = Memo::new(move |_| {
        let log = s.log(channel);
        let me = s.me_id();
        let marker = marker.get_value();
        let mut rows = Vec::with_capacity(log.msgs.len() + 4);
        if log.loading && log.msgs.is_empty() {
            rows.push(Row::Loading);
        } else if !log.more_before && log.loaded {
            rows.push(Row::Start);
        } else if log.more_before {
            rows.push(Row::Loading);
        }
        let mut prev: Option<&Message> = None;
        let mut new_done = false;
        for m in &log.msgs {
            if prev.is_none_or(|p| !util::same_day(p.created_at, m.created_at)) {
                rows.push(Row::Day(m.created_at));
            }
            let is_new = !new_done && marker > 0 && m.id > marker && m.id < PENDING_BASE && m.author_id != me;
            if is_new {
                rows.push(Row::New);
                new_done = true;
            }
            let head = starts_group(prev, m) || is_new;
            rows.push(Row::Msg { m: m.clone(), head });
            prev = Some(m);
        }
        rows
    });

    let scroll_to_bottom = move || {
        if let Some(el) = list_ref.get_untracked() {
            el.set_scroll_top(el.scroll_height());
        }
    };

    // Keep pinned to the bottom as messages arrive; keep your place when older ones load above.
    let first_id = Memo::new(move |_| s.logs.with(|l| l.get(&channel).and_then(|l| l.msgs.first().map(|m| m.id))));
    let count = Memo::new(move |_| rows.with(|r| r.len()));
    Effect::new(move |prev_first: Option<Option<Id>>| {
        let first = first_id.get();
        count.track();
        let prepended = prev_first.is_some_and(|p| p.is_some() && first < p);
        request_animation_frame(move || {
            let Some(el) = list_ref.get_untracked() else { return };
            if prepended && !at_bottom.get_untracked() {
                let delta = el.scroll_height() - last_height.get_value();
                el.set_scroll_top(el.scroll_top() + delta);
            } else if at_bottom.get_untracked() {
                el.set_scroll_top(el.scroll_height());
            }
            last_height.set_value(el.scroll_height());
        });
        first
    });

    // A phone switching to this screen: it was hidden while messages arrived, so land on the newest.
    Effect::new(move |_| {
        s.pane.track();
        if at_bottom.get_untracked() {
            request_animation_frame(scroll_to_bottom);
        }
    });

    // Read what you see: at the bottom of a focused window means read.
    Effect::new(move |_| {
        count.track();
        let log_newest = s.logs.with(|l| l.get(&channel).map(|l| (l.more_after, l.loaded)));
        if at_bottom.get() && s.focused.get() && log_newest.is_some_and(|(after, loaded)| !after && loaded) {
            ack(channel, None);
        }
    });

    // Jump-to-message: scroll it into the middle and flash it.
    Effect::new(move |_| {
        let Some(id) = s.highlight.get() else { return };
        count.track();
        request_animation_frame(move || {
            if let Some(el) = util::element_by_id::<web_sys::Element>(&format!("msg-{id}")) {
                let opts = web_sys::ScrollIntoViewOptions::new();
                opts.set_block(web_sys::ScrollLogicalPosition::Center);
                el.scroll_into_view_with_scroll_into_view_options(&opts);
                at_bottom.set(false);
                set_timeout(move || s.highlight.set(None), std::time::Duration::from_millis(2000));
            }
        });
    });

    let on_scroll = move |_| {
        let Some(el) = list_ref.get_untracked() else { return };
        let bottom = el.scroll_height() - el.scroll_top() - el.client_height() < 40;
        at_bottom.set(bottom);
        last_height.set_value(el.scroll_height());
        if el.scroll_top() < 400 {
            crate::store::load_older(channel);
        }
        if bottom {
            crate::store::load_newer(channel);
        }
    };

    let jump_present = move |_| {
        if s.logs.with_untracked(|l| l.get(&channel).is_some_and(|l| l.more_after)) {
            crate::store::fetch_latest(channel);
        }
        at_bottom.set(true);
        request_animation_frame(scroll_to_bottom);
    };
    let unread_count = move || {
        let last_read = s.read.with(|r| r.get(&channel).map(|r| r.last_read_id).unwrap_or(0));
        s.logs.with(|l| l.get(&channel).map(|l| l.msgs.iter().filter(|m| m.id > last_read && m.id < PENDING_BASE).count()).unwrap_or(0))
    };

    view! {
        <div class="chat-pane"
            on:dragover=move |e: web_sys::DragEvent| {
                if e.data_transfer().is_some_and(|d| d.types().includes(&"Files".into(), 0)) {
                    e.prevent_default();
                    drag_over.set(true);
                }
            }
            on:dragleave=move |e: web_sys::DragEvent| {
                let leaving = e.related_target().is_none();
                if leaving { drag_over.set(false); }
            }
            on:drop=move |e: web_sys::DragEvent| {
                e.prevent_default();
                drag_over.set(false);
                if let Some(files) = e.data_transfer().and_then(|d| d.files()) {
                    composer::add_files(uploads, channel, files);
                }
            }>
            <Show when=move || { !at_bottom.get() && unread_count() > 0 }>
                <div class="new-bar">
                    <span>{move || format!("{} new message{} — ", unread_count(), if unread_count() == 1 { "" } else { "s" })}</span>
                    <button on:click=move |_| ack(channel, None)>"Mark as read"</button>
                </div>
            </Show>
            <div class="messages scroll" node_ref=list_ref on:scroll=on_scroll>
                <For each=move || rows.get() key=|r| r.key() let:row>
                    {match row {
                        Row::Start => view! { <ChannelStart channel=channel/> }.into_any(),
                        Row::Loading => view! { <div class="loading-more"><div class="spinner"></div></div> }.into_any(),
                        Row::Day(t) => view! { <div class="day-divider"><span>{util::day_label(t)}</span></div> }.into_any(),
                        Row::New => view! { <div class="new-divider"><span>"NEW"</span></div> }.into_any(),
                        Row::Msg { m, head } => view! { <MessageItem m=m head=head space=space/> }.into_any(),
                    }}
                </For>
                <div class="messages-end"></div>
            </div>
            <Show when=move || !at_bottom.get() || s.logs.with(|l| l.get(&channel).is_some_and(|l| l.more_after))>
                <button class="jump-present" on:click=jump_present>"Jump to present" <Icon name="arrow-down" size=16/></button>
            </Show>
            <composer::Composer channel=channel uploads=uploads/>
            <Show when=move || drag_over.get()>
                <div class="drop-overlay">
                    <div class="drop-card">
                        <Icon name="upload" size=48/>
                        <div class="drop-title">{move || format!("Upload to {}", s.channel(channel).map(|c| s.channel_title(&c)).unwrap_or_default())}</div>
                        <div class="drop-sub">{move || if s.nextcloud_session.get() { "Files are saved to your Nextcloud and shared by link." } else { "Drop files to attach them." }}</div>
                    </div>
                </div>
            </Show>
        </div>
    }
}

#[component]
fn ChannelStart(channel: Id) -> impl IntoView {
    let s = store();
    let c = s.channel(channel);
    let (title, sub) = match &c {
        Some(c) if c.kind.is_dm() => (
            s.channel_title(c),
            format!("This is the beginning of your direct message history with {}.", s.channel_title(c)),
        ),
        Some(c) if c.kind == ChannelKind::Thread => {
            let by = c.thread.as_ref().map(|t| s.name(t.owner_id, c.space_id)).unwrap_or_default();
            (c.name.clone(), format!("Started by {by}."))
        }
        Some(c) => (format!("Welcome to #{}!", c.name), format!("This is the start of the #{} channel.", c.name)),
        None => (String::new(), String::new()),
    };
    if let Some(c) = c.as_ref().filter(|c| c.kind == ChannelKind::Thread) {
        let starter = c.thread.as_ref().map(|t| t.starter_id);
        let parent = c.parent_id;
        return view! {
            <div class="channel-start thread-start">
                <div class="start-icon"><Icon name="thread" size=36/></div>
                <h1>{title}</h1>
                <p>{sub}" "
                    {starter.zip(parent).map(|(m, p)| view! {
                        <a href="#" on:click=move |e| { e.prevent_default(); jump_to(p, m); }>"Jump to the original message"</a>
                    })}
                </p>
            </div>
        }
        .into_any();
    }
    let partner = c.as_ref().and_then(|c| s.dm_partner(c));
    view! {
        <div class="channel-start">
            {match partner {
                Some(u) => view! { <Avatar user=u size=80/> }.into_any(),
                None => view! { <div class="start-icon"><Icon name="hash" size=40/></div> }.into_any(),
            }}
            <h1>{title}</h1>
            <p>{sub}</p>
        </div>
    }
    .into_any()
}

/// The card under a message that started a thread.
#[component]
fn ThreadCard(t: ThreadPreview, space: Option<Id>) -> impl IntoView {
    let s = store();
    let id = t.id;
    let last = t.last_message_id;
    let n = t.message_count;
    // Fetch the thread's newest messages so the card can preview the latest reply.
    if n > 0 {
        crate::store::load_channel(id);
    }
    view! {
        <div class="thread-card" on:click=move |e| { e.stop_propagation(); crate::store::open_channel(id); }>
            <div class="thread-card-top">
                <span class="thread-name">{t.name.clone()}</span>
                <span class="thread-count">{format!("{n} Message{} ›", if n == 1 { "" } else { "s" })}</span>
                {t.archived.then(|| view! { <span class="muted small">"archived"</span> })}
            </div>
            {move || last.and_then(|l| s.message(id, l)).map(|m| {
                let author = m.author_id;
                view! {
                    <div class="thread-last">
                        <Avatar user=author size=16/>
                        <b>{s.name(author, space)}</b>
                        <span class="ellipsis">{markdown::plain(&m.content, &|u| s.name(u, space))}</span>
                    </div>
                }
            })}
            {(n == 0).then(|| view! { <div class="thread-last muted">"No replies yet. Be the first."</div> })}
        </div>
    }
}

/// The open thread, beside its channel.
#[component]
fn ThreadPanel(thread: Id) -> impl IntoView {
    let s = store();
    let c = move || s.channel(thread);
    let archived = move || c().and_then(|c| c.thread).is_some_and(|t| t.archived);
    let can_manage = move || c().is_some_and(|c| s.can_in(c.id, perm::MANAGE_THREADS) || c.thread.is_some_and(|t| t.owner_id == s.me_id()));
    let set_archived = move |a: bool| {
        leptos::task::spawn_local(async move {
            let body = UpdateChannel { archived: Some(a), ..Default::default() };
            match api::call::<_, Channel>("PATCH", &format!("/api/channels/{thread}"), &body).await {
                Ok(ch) => {
                    crate::store::apply(Event::ChannelUpdate(ch));
                    if a {
                        s.thread.set(None);
                        s.toast("Thread archived. Find it again under Threads in the channel header.", false);
                    }
                }
                Err(e) => s.error(e),
            }
        });
    };
    view! {
        <aside class="thread-panel">
            <header class="chat-head">
                <Icon name="thread"/>
                <span class="title ellipsis">{move || c().map(|c| c.name).unwrap_or_default()}</span>
                <span class="spacer"></span>
                <Show when=can_manage>
                    <button class="icon-btn" title=move || if archived() { "Unarchive thread" } else { "Archive thread" }
                        on:click=move |_| set_archived(!archived())><Icon name="archive"/></button>
                </Show>
                <button class="icon-btn" title="More" on:click=move |e| s.open_menu(&e, menu::channel_menu(thread))><Icon name="more"/></button>
                <button class="icon-btn" title="Close thread" on:click=move |_| if s.mobile.get_untracked() { crate::store::go_back() } else { s.thread.set(None) }><Icon name="x"/></button>
            </header>
            <Show when=archived>
                <div class="banner archived">"This thread is archived. Sending a message brings it back."</div>
            </Show>
            <ChatPane channel=thread/>
        </aside>
    }
}

#[component]
pub fn MessageItem(m: Message, head: bool, space: Option<Id>) -> impl IntoView {
    let s = store();
    let me = s.me_id();
    let id = m.id;
    let channel = m.channel_id;
    let author = m.author_id;
    let pending = id >= PENDING_BASE;
    let nonce = m.nonce.clone().unwrap_or_default();
    let nonce2 = StoredValue::new(nonce.clone());
    let failed = move || pending && s.failed.with(|f| nonce2.with_value(|n| f.contains(n)));
    let mine = author == me;
    let mentioned = m.mentions.contains(&me) || m.mention_everyone;
    let editing = move || s.editing.get() == Some(id);
    let created = m.created_at;
    let msg_for_menu = m.clone();

    let on_click = move |e: leptos::ev::MouseEvent| {
        if e.alt_key() && !pending {
            crate::store::mark_unread(channel, id);
        }
    };

    if m.kind != MessageKind::Default {
        return view! { <SystemMessage m=m space=space/> }.into_any();
    }

    let author_name = move || s.name(author, space);
    // Members show the colour of their highest coloured role.
    let author_color = move || s.role_color(space, author);
    let content = m.content.clone();
    let jumbo = util::only_emoji(&content) && m.reply.is_none();
    let edited_at = m.edited_at;

    view! {
        <div id=format!("msg-{id}") class="message" class:head=head class:mentioned=mentioned class:pending=pending
            class:failed=failed class:highlight=move || s.highlight.get() == Some(id) class:editing=editing
            on:click=on_click
            on:contextmenu=move |e| if !pending { s.open_menu(&e, menu::message_menu(&msg_for_menu)) }>
            {m.reply.clone().map(|r| view! { <ReplyBar r=r channel=channel space=space/> })}
            <div class="msg-gutter">
                {if head {
                    view! { <div class="msg-avatar" on:click=move |e| s.open_profile(&e, author)><Avatar user=author size=40/></div> }.into_any()
                } else {
                    view! { <span class="msg-time-hover" title=util::long_time(created)>{util::short_time(created)}</span> }.into_any()
                }}
            </div>
            <div class="msg-body">
                {head.then(|| view! {
                    <div class="msg-head">
                        <span class="author" style:color=author_color on:click=move |e| s.open_profile(&e, author)>{author_name}</span>
                        <span class="msg-time" title=util::long_time(created)>{util::timestamp(created)}</span>
                    </div>
                })}
                {move || if editing() {
                    view! { <InlineEditor channel=channel id=id content=s.message(channel, id).map(|m| m.content).unwrap_or_default()/> }.into_any()
                } else {
                    let content = content.clone();
                    view! {
                        <div class="msg-content" class:jumbo=jumbo>
                            {(!content.is_empty()).then(|| markdown::render(&content, space))}
                            {edited_at.map(|t| view! { <span class="edited" title=util::long_time(t)>"(edited)"</span> })}
                        </div>
                    }.into_any()
                }}
                {(!m.attachments.is_empty()).then(|| view! {
                    <div class="attachments">{m.attachments.iter().cloned().map(|a| view! { <AttachmentView a=a/> }).collect_view()}</div>
                })}
                {(!m.reactions.is_empty()).then(|| view! { <Reactions m=m.clone() space=space/> })}
                {m.thread.clone().map(|t| view! { <ThreadCard t=t space=space/> })}
                {move || failed().then(|| {
                    let n1 = nonce.clone();
                    let n2 = nonce.clone();
                    view! {
                        <div class="failed-line">
                            "Message failed to send. "
                            <a href="#" on:click=move |e| { e.prevent_default(); crate::store::retry(channel, n1.clone()); }>"Retry"</a>
                            " · "
                            <a href="#" on:click=move |e| { e.prevent_default(); crate::store::discard_failed(channel, n2.clone()); }>"Delete"</a>
                        </div>
                    }
                })}
            </div>
            {(!pending).then(|| view! { <Toolbar channel=channel id=id mine=mine space=space/> })}
        </div>
    }
    .into_any()
}

#[component]
fn SystemMessage(m: Message, space: Option<Id>) -> impl IntoView {
    let s = store();
    let author = m.author_id;
    let channel = m.channel_id;
    let created = m.created_at;
    let name = move || s.name(author, space);
    let (icon, text): (&'static str, AnyView) = match m.kind {
        MessageKind::MemberJoin => ("user-plus", view! { <span><b class="author" on:click=move |e| s.open_profile(&e, author)>{name}</b>" joined the space. Say hi!"</span> }.into_any()),
        MessageKind::Pin => {
            let target = m.reply.as_ref().map(|r| r.id);
            ("pin", view! {
                <span><b class="author">{name}</b>" pinned "
                    <a href="#" on:click=move |e| { e.prevent_default(); if let Some(t) = target { jump_to(channel, t); } }>"a message"</a>
                    " to this channel. "
                    <a href="#" on:click=move |e| { e.prevent_default(); s.modal.set(Some(Modal::Pins(channel))); }>"See all pinned messages."</a>
                </span>
            }.into_any())
        }
        MessageKind::ThreadCreated => {
            let thread = m.thread.clone();
            let tname = m.content.clone();
            ("thread", view! {
                <span>
                    <b class="author" on:click=move |e| s.open_profile(&e, author)>{name}</b>" started a thread: "<b>{tname}</b>". "
                    <a href="#" on:click=move |e| { e.prevent_default(); s.modal.set(Some(Modal::Threads(channel))); }>"See all threads."</a>
                    {thread.map(|t| view! { <ThreadCard t=t space=space/> })}
                </span>
            }.into_any())
        }
        _ => {
            let secs: u64 = m.content.parse().unwrap_or(0);
            let tail = if secs > 0 { format!(" started a call that lasted {}.", duration(secs)) } else { " started a call.".to_string() };
            ("phone", view! { <span><b class="author">{name}</b>{tail}</span> }.into_any())
        }
    };
    view! {
        <div id=format!("msg-{}", m.id) class="message system head">
            <div class="msg-gutter"><span class="sys-icon"><Icon name=icon size=18/></span></div>
            <div class="msg-body">{text}<span class="msg-time" title=util::long_time(created)>{util::timestamp(created)}</span></div>
        </div>
    }
}

fn duration(secs: u64) -> String {
    match secs {
        0..60 => "a few seconds".into(),
        60..3600 => format!("{} minute{}", secs / 60, if secs / 60 == 1 { "" } else { "s" }),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

#[component]
fn ReplyBar(r: ReplyPreview, channel: Id, space: Option<Id>) -> impl IntoView {
    let s = store();
    let target = r.id;
    let author = r.author_id;
    let snippet = if r.deleted {
        "Original message was deleted".to_string()
    } else if r.content.is_empty() && r.has_attachments {
        "Click to see attachment".to_string()
    } else {
        markdown::plain(&r.content, &|id| s.name(id, space))
    };
    view! {
        <div class="reply-bar" on:click=move |e| { e.stop_propagation(); if !r.deleted { jump_to(channel, target); } }>
            <span class="reply-spine"></span>
            {(!r.deleted).then(|| view! { <Avatar user=author size=16/> <span class="reply-author">{move || s.name(author, space)}</span> })}
            <span class="reply-text ellipsis" class:deleted=r.deleted>{snippet}</span>
        </div>
    }
}

#[component]
fn InlineEditor(channel: Id, id: Id, content: String) -> impl IntoView {
    let s = store();
    let text = RwSignal::new(composer::decode_mentions(&content));
    let el = NodeRef::<html::Textarea>::new();
    Effect::new(move |_| {
        if let Some(t) = el.get() {
            let _ = t.focus();
            let len = t.value().len() as u32;
            let _ = t.set_selection_range(len, len);
            composer::autosize(&t);
        }
    });
    let save = move || {
        let v = text.get_untracked();
        if v.trim().is_empty() {
            s.editing.set(None);
            crate::store::delete_with_confirm(channel, id);
        } else {
            crate::store::edit_message(channel, id, composer::encode_mentions(channel, &v));
        }
    };
    view! {
        <div class="inline-editor">
            <textarea node_ref=el rows="1" prop:value=move || text.get()
                on:input=move |e| {
                    text.set(event_target_value(&e));
                    if let Some(t) = el.get_untracked() { composer::autosize(&t); }
                }
                on:keydown=move |e| {
                    if e.key() == "Escape" { e.stop_propagation(); s.editing.set(None); }
                    if e.key() == "Enter" && !e.shift_key() { e.prevent_default(); save(); }
                }></textarea>
            <div class="editor-hint">"escape to " <a href="#" on:click=move |e| { e.prevent_default(); s.editing.set(None); }>"cancel"</a>
                " • enter to " <a href="#" on:click=move |e| { e.prevent_default(); save(); }>"save"</a></div>
        </div>
    }
}

#[component]
fn AttachmentView(a: Attachment) -> impl IntoView {
    let s = store();
    let url = api::url(&a.url);
    if a.is_image() {
        let (w, h) = (a.width.unwrap_or(400) as f64, a.height.unwrap_or(300) as f64);
        let scale = (400.0 / w).min(300.0 / h).min(1.0);
        let full = url.clone();
        return view! {
            <img class="att-image" src=url alt=a.filename.clone() loading="lazy"
                width=(w * scale).round() as u32 height=(h * scale).round() as u32
                on:click=move |_| s.modal.set(Some(Modal::Image(full.clone())))/>
        }
        .into_any();
    }
    if a.is_video() {
        return view! { <video class="att-video" src=url controls=true preload="metadata"></video> }.into_any();
    }
    let audio = a.is_audio();
    let dl = url.clone();
    view! {
        <div class="att-file">
            <Icon name="file" size=32/>
            <div class="att-file-text">
                <a href=dl.clone() class="ellipsis" download=a.filename.clone() on:click=move |e| {
                    if desktop_link(&dl) { e.prevent_default(); }
                }>{a.filename.clone()}</a>
                <span class="sub">{util::file_size(a.size)}
                    {a.link.clone().map(|l| view! {
                        " · " <a href=l.clone() on:click=move |e| { e.prevent_default(); crate::desktop::open_url(&l); }>"Open in Nextcloud"</a>
                    })}
                </span>
                {audio.then(|| view! { <audio src=url.clone() controls=true preload="none"></audio> })}
            </div>
        </div>
    }
    .into_any()
}

/// In the desktop app, downloads open in the system browser (it handles saving).
fn desktop_link(url: &str) -> bool {
    if crate::desktop::is_tauri() {
        crate::desktop::open_url(url);
        true
    } else {
        false
    }
}

#[component]
fn Reactions(m: Message, space: Option<Id>) -> impl IntoView {
    let s = store();
    let me = s.me_id();
    let channel = m.channel_id;
    let id = m.id;
    view! {
        <div class="reactions">
            {m.reactions.iter().cloned().map(|r| {
                let mine = r.users.contains(&me);
                let names: Vec<String> = r.users.iter().take(6).map(|&u| s.name(u, space)).collect();
                let more = r.users.len().saturating_sub(6);
                let title = format!("{}{} reacted with {}", names.join(", "), if more > 0 { format!(" and {more} more") } else { String::new() }, r.emoji);
                let emoji = r.emoji.clone();
                view! {
                    <button class="reaction" class:me=mine title=title on:click=move |e| {
                        e.stop_propagation();
                        crate::store::toggle_reaction(channel, id, emoji.clone());
                    }>
                        <span class="emoji">{r.emoji.clone()}</span><span class="count">{r.users.len()}</span>
                    </button>
                }
            }).collect_view()}
            <button class="reaction add" title="Add reaction" on:click=move |e| {
                e.stop_propagation();
                emoji::open_picker(&e, move |em: String| crate::store::toggle_reaction(channel, id, em));
            }><Icon name="smile" size=16/></button>
        </div>
    }
}

#[component]
fn Toolbar(channel: Id, id: Id, mine: bool, space: Option<Id>) -> impl IntoView {
    let s = store();
    let can_pin = s.can_pin_in(channel);
    let can_delete = space.is_some() && s.can_in(channel, perm::MANAGE_MESSAGES);
    let quick = move || s.settings.with(|st| st.recent_emoji.iter().take(3).cloned().collect::<Vec<_>>());
    let can_thread = move || {
        s.channel(channel).is_some_and(|c| c.kind == ChannelKind::Text && c.space_id.is_some())
            && s.can_in(channel, perm::CREATE_THREADS)
            && s.message(channel, id).is_some_and(|m| m.thread.is_none() && m.kind == MessageKind::Default)
    };
    view! {
        <div class="toolbar" on:click=|e| e.stop_propagation()>
            <For each=quick key=|e| e.clone() let:em>
                {
                    let e2 = em.clone();
                    view! { <button class="tb-emoji" title=format!("React with {em}") on:click=move |_| crate::store::toggle_reaction(channel, id, e2.clone())>{em.clone()}</button> }
                }
            </For>
            <button title="Add reaction" on:click=move |e| {
                emoji::open_picker(&e, move |em: String| crate::store::toggle_reaction(channel, id, em));
            }><Icon name="smile" size=18/></button>
            <Show when=move || !s.shift.get()>
                <button title="Reply" on:click=move |_| {
                    s.reply_to.set(Some(id));
                    util::focus(&format!("composer-{channel}"));
                }><Icon name="reply" size=18/></button>
                {mine.then(|| view! { <button title="Edit" on:click=move |_| s.editing.set(Some(id))><Icon name="edit" size=18/></button> })}
                <Show when=can_thread>
                    <button title="Create Thread" on:click=move |_| s.modal.set(Some(Modal::CreateThread { channel, message: Some(id) }))><Icon name="thread" size=18/></button>
                </Show>
            </Show>
            <Show when=move || s.shift.get()>
                {can_pin.then(|| view! {
                    <button title="Pin" on:click=move |_| crate::store::set_pinned(channel, id, !s.message(channel, id).is_some_and(|m| m.pinned))><Icon name="pin" size=18/></button>
                })}
                {(mine || can_delete).then(|| view! {
                    <button class="danger" title="Delete (no confirmation while Shift is held)" on:click=move |_| crate::store::delete_message(channel, id)><Icon name="trash" size=18/></button>
                })}
            </Show>
            <button title="More" on:click=move |e| {
                if let Some(m) = s.message(channel, id) { s.open_menu(&e, menu::message_menu(&m)); }
            }><Icon name="more" size=18/></button>
        </div>
    }
}

#[component]
fn SearchPanel() -> impl IntoView {
    let s = store();
    view! {
        <aside class="search-panel">
            <header>
                <span>{move || s.search.get().map(|(q, r)| format!("{} result{} for “{}”", r.total, if r.total == 1 { "" } else { "s" }, q)).unwrap_or_default()}</span>
                <button class="icon-btn" title="Close" on:click=move |_| if s.mobile.get_untracked() { crate::store::go_back() } else { s.search.set(None) }><Icon name="x" size=18/></button>
            </header>
            <div class="scroll search-results">
                {move || s.search.get().map(|(_, r)| r.messages.into_iter().map(|m| {
                    let channel = m.channel_id;
                    let id = m.id;
                    let space = s.channel(channel).and_then(|c| c.space_id);
                    let cname = s.channel(channel).map(|c| s.channel_title(&c)).unwrap_or_default();
                    view! {
                        <div class="search-hit" on:click=move |_| jump_to(channel, id)>
                            <div class="hit-channel">{if space.is_some() { format!("#{cname}") } else { cname }}</div>
                            <MessageItem m=m head=true space=space/>
                        </div>
                    }
                }).collect_view())}
                <Show when=move || s.search.with(|r| r.as_ref().is_some_and(|(_, r)| r.messages.is_empty()))>
                    <div class="empty-hint">"Nothing found. Try fewer or different words."</div>
                </Show>
            </div>
        </aside>
    }
}
