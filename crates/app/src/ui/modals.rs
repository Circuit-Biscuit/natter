//! Dialogs: create/join space, invites, channels, quick switcher, shortcuts, new DM, pins,
//! image viewer, Nextcloud file browser, confirmations.

use leptos::prelude::*;
use leptos::task::spawn_local;
use natter_proto::*;

use super::icons::Icon;
use super::{Avatar, chat, settings};
use crate::store::{Modal, open_channel, store};
use crate::{api, util};

#[component]
pub fn ModalHost() -> impl IntoView {
    let s = store();
    view! {
        {move || s.modal.get().map(|m| {
            let wide = matches!(m, Modal::UserSettings | Modal::SpaceSettings(_));
            let image = matches!(m, Modal::Image(_));
            let body = match m {
                Modal::CreateSpace => view! { <CreateSpace/> }.into_any(),
                Modal::Invite(sp) => view! { <InviteDialog space=sp/> }.into_any(),
                Modal::CreateChannel { space, kind, parent } => view! { <CreateChannel space=space kind=kind parent=parent/> }.into_any(),
                Modal::EditChannel(id) => view! { <EditChannel id=id/> }.into_any(),
                Modal::SpaceSettings(id) => view! { <settings::SpaceSettings space=id/> }.into_any(),
                Modal::UserSettings => view! { <settings::UserSettings/> }.into_any(),
                Modal::QuickSwitcher => view! { <QuickSwitcher/> }.into_any(),
                Modal::Shortcuts => view! { <Shortcuts/> }.into_any(),
                Modal::NewDm => view! { <NewDm/> }.into_any(),
                Modal::Image(url) => view! { <ImageViewer url=url/> }.into_any(),
                Modal::Pins(c) => view! { <Pins channel=c/> }.into_any(),
                Modal::Nextcloud => view! { <NextcloudBrowser/> }.into_any(),
                Modal::CreateThread { channel, message } => view! { <CreateThreadDialog channel=channel message=message/> }.into_any(),
                Modal::Threads(c) => view! { <ThreadBrowser channel=c/> }.into_any(),
                Modal::Confirm { title, body, ok, action } => view! { <Confirm title=title body=body ok=ok action=action/> }.into_any(),
            };
            view! {
                <div class="modal-backdrop" class:image=image on:mousedown=move |_| s.modal.set(None)>
                    <div class="modal" class:wide=wide class:image=image on:mousedown=|e| e.stop_propagation() on:click=|e| e.stop_propagation()>
                        {body}
                    </div>
                </div>
            }
        })}
    }
}

fn close() {
    store().modal.set(None);
}

#[component]
fn Confirm(title: String, body: String, ok: String, action: crate::store::Action) -> impl IntoView {
    view! {
        <h2>{title}</h2>
        <p class="modal-text">{body}</p>
        <div class="modal-foot">
            <button class="btn link" on:click=|_| close()>"Cancel"</button>
            <button class="btn danger" autofocus=true on:click=move |_| { close(); action(); }>{ok}</button>
        </div>
    }
}

/// "https://chat.example.com/invite/AbC123" or "AbC123" → "AbC123".
fn invite_code(s: &str) -> String {
    s.trim().trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string()
}

#[component]
fn CreateSpace() -> impl IntoView {
    let s = store();
    let name = RwSignal::new(format!("{}'s space", s.me.get_untracked().map(|u| u.display_name).unwrap_or_default()));
    let invite = RwSignal::new(String::new());
    let preview = RwSignal::new(None::<InvitePreview>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let create = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        busy.set(true);
        spawn_local(async move {
            match api::call::<_, Space>("POST", "/api/spaces", &CreateSpace { name: name.get_untracked().trim().to_string() }).await {
                Ok(sp) => {
                    let id = sp.id;
                    crate::store::apply(Event::SpaceCreate(sp));
                    close();
                    util::storage_set("natter.space", &id.to_string());
                    crate::store::open_space(Some(id));
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    };
    let look_up = move || {
        let code = invite_code(&invite.get_untracked());
        if code.is_empty() {
            return;
        }
        spawn_local(async move {
            match api::get::<InvitePreview>(&format!("/api/invites/{}", api::enc(&code))).await {
                Ok(p) => {
                    preview.set(Some(p));
                    error.set(None);
                }
                Err(e) => {
                    preview.set(None);
                    error.set(Some(e.message));
                }
            }
        });
    };
    let join = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let code = invite_code(&invite.get_untracked());
        busy.set(true);
        spawn_local(async move {
            match api::call::<_, Space>("POST", &format!("/api/invites/{}", api::enc(&code)), &serde_json::json!({})).await {
                Ok(sp) => {
                    let id = sp.id;
                    crate::store::apply(Event::SpaceCreate(sp));
                    close();
                    util::storage_set("natter.space", &id.to_string());
                    crate::store::open_space(Some(id));
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    };
    view! {
        <h2>"Create a space"</h2>
        <p class="modal-text">"A space is where your team talks: text channels, voice channels, and everyone in one place."</p>
        <form class="form" on:submit=create>
            <label>"Space name"
                <input required=true maxlength="100" autofocus=true prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e))/>
            </label>
            <button class="btn primary wide" type="submit" disabled=move || busy.get()>"Create"</button>
        </form>
        <div class="or"><span>"or join one"</span></div>
        <form class="form" on:submit=join>
            <label>"Invite link"
                <input placeholder="https://chat.example.com/invite/AbC123" prop:value=move || invite.get()
                    on:input=move |e| invite.set(event_target_value(&e)) on:blur=move |_| look_up()/>
            </label>
            {move || preview.get().map(|p| view! {
                <div class="invite-preview">
                    <div class="space-initials big">{util::initials(&p.space_name)}</div>
                    <div><b>{p.space_name}</b><div class="muted">{format!("{} online · {} members", p.online_count, p.member_count)}</div></div>
                </div>
            })}
            {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}
            <button class="btn wide" type="submit" disabled=move || busy.get() || invite.with(|i| i.trim().is_empty())>"Join space"</button>
        </form>
    }
}

#[component]
fn InviteDialog(space: Id) -> impl IntoView {
    let s = store();
    let expiry = RwSignal::new(7 * 86_400u64);
    let uses = RwSignal::new(0u32);
    let invite = RwSignal::new(None::<Invite>);
    let copied = RwSignal::new(false);
    let make = move || {
        let body = CreateInvite {
            max_uses: Some(uses.get_untracked()).filter(|&u| u > 0),
            expires_in_secs: Some(expiry.get_untracked()).filter(|&e| e > 0),
        };
        spawn_local(async move {
            match api::call::<_, Invite>("POST", &format!("/api/spaces/{space}/invites"), &body).await {
                Ok(i) => invite.set(Some(i)),
                Err(e) => s.error(e),
            }
        });
    };
    make();
    let link = move || {
        invite.get().map(|i| {
            let base = api::server();
            let base = if base.is_empty() { util::window().location().origin().unwrap_or_default() } else { base };
            format!("{base}/invite/{}", i.code)
        })
    };
    view! {
        <h2>{move || format!("Invite people to {}", s.space(space).map(|s| s.name).unwrap_or_default())}</h2>
        <p class="modal-text">"Send this link. New people can create an account with it (or sign in with Nextcloud) and land right here."</p>
        <div class="copy-row">
            <input readonly=true prop:value=move || link().unwrap_or_else(|| "Creating link…".into())/>
            <button class="btn primary" on:click=move |_| {
                if let Some(l) = link() {
                    util::copy_text(&l);
                    copied.set(true);
                    set_timeout(move || copied.set(false), std::time::Duration::from_millis(1500));
                }
            }>{move || if copied.get() { "Copied" } else { "Copy" }}</button>
        </div>
        <div class="row gap">
            <label class="grow">"Expires after"
                <select on:change=move |e| { expiry.set(event_target_value(&e).parse().unwrap_or(0)); make(); }>
                    {[(1800u64, "30 minutes"), (3600, "1 hour"), (86_400, "1 day"), (604_800, "7 days"), (2_592_000, "30 days"), (0, "Never")].into_iter().map(|(v, l)| view! {
                        <option value=v.to_string() selected=v == 604_800>{l}</option>
                    }).collect_view()}
                </select>
            </label>
            <label class="grow">"Max number of uses"
                <select on:change=move |e| { uses.set(event_target_value(&e).parse().unwrap_or(0)); make(); }>
                    {[(0u32, "No limit"), (1, "1 use"), (5, "5 uses"), (10, "10 uses"), (25, "25 uses"), (100, "100 uses")].into_iter().map(|(v, l)| view! {
                        <option value=v.to_string()>{l}</option>
                    }).collect_view()}
                </select>
            </label>
        </div>
    }
}

#[component]
fn CreateChannel(space: Id, kind: ChannelKind, parent: Option<Id>) -> impl IntoView {
    let s = store();
    let kind_sig = RwSignal::new(kind);
    let name = RwSignal::new(String::new());
    let read_only = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let category = kind == ChannelKind::Category;
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let body = CreateChannel { name: name.get_untracked(), kind: kind_sig.get_untracked(), parent_id: parent, topic: String::new() };
        let ro = read_only.get_untracked();
        spawn_local(async move {
            match api::call::<_, Channel>("POST", &format!("/api/spaces/{space}/channels"), &body).await {
                Ok(c) => {
                    let id = c.id;
                    let k = c.kind;
                    crate::store::apply(Event::ChannelCreate(c));
                    if ro {
                        let _ = api::call::<_, Channel>("PATCH", &format!("/api/channels/{id}"), &UpdateChannel { read_only: Some(true), ..Default::default() }).await;
                    }
                    close();
                    if k == ChannelKind::Text {
                        open_channel(id);
                    }
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    let shown = move || {
        let n = name.get();
        if kind_sig.get() == ChannelKind::Text { channel_slug(&n) } else { n }
    };
    view! {
        <h2>{if category { "Create category" } else { "Create channel" }}</h2>
        {parent.and_then(|p| s.channel(p)).map(|p| view! { <p class="modal-text">{format!("in {}", p.name)}</p> })}
        <form class="form" on:submit=submit>
            {(!category).then(|| view! {
                <div class="kind-pick">
                    <label class="kind" class:active=move || kind_sig.get() == ChannelKind::Text>
                        <input type="radio" name="kind" checked=kind == ChannelKind::Text on:change=move |_| kind_sig.set(ChannelKind::Text)/>
                        <Icon name="hash" size=24/><div><b>"Text"</b><div class="muted">"Messages, images, files, opinions and puns"</div></div>
                    </label>
                    <label class="kind" class:active=move || kind_sig.get() == ChannelKind::Voice>
                        <input type="radio" name="kind" checked=kind == ChannelKind::Voice on:change=move |_| kind_sig.set(ChannelKind::Voice)/>
                        <Icon name="speaker" size=24/><div><b>"Voice"</b><div class="muted">"Hang out with voice, video and screen sharing"</div></div>
                    </label>
                </div>
            })}
            <label>{if category { "Category name" } else { "Channel name" }}
                <input required=true maxlength="100" autofocus=true placeholder=if category { "Projects" } else { "new-channel" }
                    prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e))/>
            </label>
            <Show when=move || kind_sig.get() == ChannelKind::Text && !name.with(|n| n.is_empty())>
                <div class="muted small">{move || format!("Will be shown as #{}", shown())}</div>
            </Show>
            <Show when=move || kind_sig.get() == ChannelKind::Text>
                <label class="check"><input type="checkbox" prop:checked=move || read_only.get() on:change=move |e| read_only.set(event_target_checked(&e))/>
                    "Announcement channel — only admins can post"</label>
            </Show>
            {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}
            <div class="modal-foot">
                <button type="button" class="btn link" on:click=|_| close()>"Cancel"</button>
                <button type="submit" class="btn primary">"Create"</button>
            </div>
        </form>
    }
}

#[component]
fn EditChannel(id: Id) -> impl IntoView {
    let s = store();
    let Some(c) = s.channel(id) else { return view! { <p>"Channel not found."</p> }.into_any() };
    let name = RwSignal::new(c.name.clone());
    let topic = RwSignal::new(c.topic.clone());
    let read_only = RwSignal::new(c.read_only);
    let parent = RwSignal::new(c.parent_id.unwrap_or(0));
    let error = RwSignal::new(None::<String>);
    let category = c.kind == ChannelKind::Category;
    let cats: Vec<Channel> = c.space_id.and_then(|sp| s.space(sp)).map(|sp| sp.channels.into_iter().filter(|x| x.kind == ChannelKind::Category).collect()).unwrap_or_default();
    let save = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let body = UpdateChannel {
            name: Some(name.get_untracked()),
            topic: Some(topic.get_untracked()),
            read_only: Some(read_only.get_untracked()),
            parent_id: (!category).then(|| parent.get_untracked()),
            position: None,
            archived: None,
        };
        spawn_local(async move {
            match api::call::<_, Channel>("PATCH", &format!("/api/channels/{id}"), &body).await {
                Ok(c) => {
                    crate::store::apply(Event::ChannelUpdate(c));
                    close();
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    let move_by = move |delta: i32| {
        let Some(c) = store().channel(id) else { return };
        let body = UpdateChannel { position: Some(c.position + delta), ..Default::default() };
        spawn_local(async move {
            if let Ok(c) = api::call::<_, Channel>("PATCH", &format!("/api/channels/{id}"), &body).await {
                crate::store::apply(Event::ChannelUpdate(c));
            }
        });
    };
    view! {
        <h2>{if category { "Edit category" } else { "Edit channel" }}</h2>
        <form class="form" on:submit=save>
            <label>"Name"<input required=true maxlength="100" prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e))/></label>
            {(!category).then(|| view! {
                <label>"Topic"<textarea rows="3" maxlength="1024" prop:value=move || topic.get() on:input=move |e| topic.set(event_target_value(&e))></textarea></label>
                <label>"Category"
                    <select on:change=move |e| parent.set(event_target_value(&e).parse().unwrap_or(0))>
                        <option value="0" selected=c.parent_id.is_none()>"No category"</option>
                        {cats.iter().map(|cat| view! { <option value=cat.id.to_string() selected=c.parent_id == Some(cat.id)>{cat.name.clone()}</option> }).collect_view()}
                    </select>
                </label>
            })}
            {(c.kind == ChannelKind::Text).then(|| view! {
                <label class="check"><input type="checkbox" prop:checked=move || read_only.get() on:change=move |e| read_only.set(event_target_checked(&e))/>
                    "Announcement channel — only admins can post"</label>
            })}
            <div class="row gap">
                <span class="muted small">"Order:"</span>
                <button type="button" class="btn small" on:click=move |_| move_by(-1)>"Move up"</button>
                <button type="button" class="btn small" on:click=move |_| move_by(1)>"Move down"</button>
            </div>
            {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}
            <div class="modal-foot">
                <button type="button" class="btn danger-link" on:click=move |e| {
                    let items = super::menu::channel_menu(id);
                    if let Some(Some(del)) = items.into_iter().last() { let _ = e; (del.action)(); }
                }>"Delete"</button>
                <span class="spacer"></span>
                <button type="button" class="btn link" on:click=|_| close()>"Cancel"</button>
                <button type="submit" class="btn primary">"Save"</button>
            </div>
        </form>
    }
    .into_any()
}

#[derive(Clone, PartialEq)]
enum Hit {
    Channel(Channel),
    User(User),
}

#[component]
fn QuickSwitcher() -> impl IntoView {
    let s = store();
    let q = RwSignal::new(String::new());
    let sel = RwSignal::new(0usize);
    let hits = move || -> Vec<Hit> {
        let query = q.get().to_lowercase();
        let (kind, query) = match query.chars().next() {
            Some('#') => (Some('#'), query[1..].to_string()),
            Some('@') => (Some('@'), query[1..].to_string()),
            _ => (None, query),
        };
        let mut chans: Vec<Channel> = s.spaces.with(|v| v.iter().flat_map(|sp| sp.channels.iter().filter(|c| c.kind.has_messages()).cloned()).collect());
        chans.extend(s.dms.get());
        let mut out: Vec<(i32, Hit)> = vec![];
        if kind != Some('@') {
            for c in chans {
                let title = s.channel_title(&c).to_lowercase();
                let score = if query.is_empty() {
                    if s.unread(&c) { 2 + s.mentions(c.id) as i32 * 10 } else { 0 }
                } else if title.starts_with(&query) {
                    50
                } else if title.contains(&query) {
                    20
                } else {
                    continue;
                };
                if query.is_empty() && score == 0 {
                    continue;
                }
                out.push((score, Hit::Channel(c)));
            }
        }
        if kind != Some('#') && !query.is_empty() {
            let me = s.me_id();
            for u in s.users.with(|u| u.values().filter(|u| u.id != me).cloned().collect::<Vec<_>>()) {
                let n = u.display_name.to_lowercase();
                let score = if u.username.starts_with(&query) || n.starts_with(&query) { 45 } else if n.contains(&query) || u.username.contains(&query) { 15 } else { continue };
                out.push((score, Hit::User(u)));
            }
        }
        out.sort_by_key(|(sc, _)| -sc);
        out.into_iter().take(12).map(|(_, h)| h).collect()
    };
    let go = move |h: Hit| {
        close();
        match h {
            Hit::Channel(c) => open_channel(c.id),
            Hit::User(u) => crate::store::open_dm(u.id),
        }
    };
    view! {
        <div class="switcher">
            <input class="switcher-input" autofocus=true placeholder="Where would you like to go?"
                prop:value=move || q.get()
                on:input=move |e| { q.set(event_target_value(&e)); sel.set(0); }
                on:keydown=move |e| {
                    let n = hits().len().max(1);
                    match e.key().as_str() {
                        "ArrowDown" => { e.prevent_default(); sel.update(|i| *i = (*i + 1) % n); }
                        "ArrowUp" => { e.prevent_default(); sel.update(|i| *i = (*i + n - 1) % n); }
                        "Enter" => { if let Some(h) = hits().get(sel.get_untracked()).cloned() { go(h); } }
                        _ => {}
                    }
                }/>
            <div class="switcher-label">{move || if q.with(|q| q.is_empty()) { "UNREAD" } else { "RESULTS" }}</div>
            <div class="switcher-list">
                {move || hits().into_iter().enumerate().map(|(i, h)| {
                    let h2 = h.clone();
                    let (icon, label, sub) = match &h {
                        Hit::Channel(c) if c.kind.is_dm() => ("at", s.channel_title(c), "Direct message".to_string()),
                        Hit::Channel(c) => (match c.kind { ChannelKind::Voice => "speaker", ChannelKind::Thread => "thread", _ => "hash" }, c.name.clone(),
                            c.space_id.and_then(|sp| s.space(sp)).map(|sp| sp.name).unwrap_or_default()),
                        Hit::User(u) => ("at", u.display_name.clone(), u.username.clone()),
                    };
                    let mentions = match &h { Hit::Channel(c) => s.mentions(c.id), _ => 0 };
                    view! {
                        <div class="switcher-item" class:selected=move || sel.get() == i on:mouseenter=move |_| sel.set(i)
                            on:click=move |_| go(h2.clone())>
                            {match &h { Hit::User(u) => view! { <Avatar user=u.id size=20 status=true/> }.into_any(), _ => view! { <Icon name=icon size=18/> }.into_any() }}
                            <span class="ellipsis">{label}</span>
                            <span class="muted small">{sub}</span>
                            <span class="spacer"></span>
                            {(mentions > 0).then(|| view! { <span class="badge">{mentions}</span> })}
                        </div>
                    }
                }).collect_view()}
            </div>
            <div class="muted small switcher-tip">"Tip: start with # for channels or @ for people. Ctrl+K opens this anywhere."</div>
        </div>
    }
}

#[component]
fn Shortcuts() -> impl IntoView {
    let rows = [
        ("Ctrl + K", "Quick switcher: jump to any channel, DM or person"),
        ("Alt + ↑ / ↓", "Previous / next channel"),
        ("Alt + Shift + ↑ / ↓", "Previous / next unread channel"),
        ("Esc (in the message box)", "Mark channel as read / cancel reply"),
        ("Shift + Esc", "Mark the whole space as read"),
        ("↑ (empty message box)", "Edit your last message"),
        ("Enter / Shift + Enter", "Send / new line"),
        ("Tab", "Accept @mention, #channel or :emoji: suggestion"),
        ("Alt + click a message", "Mark unread from there"),
        ("Hold Shift on a message", "Quick pin and delete (no confirmation)"),
        ("Shift + click an emoji", "Pick several emoji at once"),
        ("Ctrl + Shift + M", "Mute / unmute"),
        ("Ctrl + Shift + D", "Deafen / undeafen"),
        ("Ctrl + P", "Pinned messages"),
        ("Ctrl + F", "Search"),
        ("Ctrl + ,", "Settings"),
        ("Ctrl + /", "This list"),
        ("@silent at the start", "Send without notifying anyone"),
    ];
    view! {
        <h2>"Keyboard shortcuts"</h2>
        <div class="shortcuts">
            {rows.into_iter().map(|(k, d)| view! { <div class="sc-row"><kbd>{k}</kbd><span>{d}</span></div> }).collect_view()}
        </div>
    }
}

#[component]
fn NewDm() -> impl IntoView {
    let s = store();
    let q = RwSignal::new(String::new());
    let picked = RwSignal::new(Vec::<Id>::new());
    let people = move || {
        let me = s.me_id();
        let query = q.get().to_lowercase();
        let mut v: Vec<User> = s.users.with(|u| u.values().filter(|u| u.id != me && (u.display_name.to_lowercase().contains(&query) || u.username.contains(&query))).cloned().collect());
        v.sort_by_key(|u| u.display_name.to_lowercase());
        v
    };
    let create = move |_| {
        let recipients = picked.get_untracked();
        if recipients.len() == 1 {
            close();
            crate::store::open_dm(recipients[0]);
            return;
        }
        spawn_local(async move {
            match api::call::<_, Channel>("POST", "/api/dms", &CreateDm { recipients }).await {
                Ok(c) => {
                    let id = c.id;
                    crate::store::apply(Event::ChannelCreate(c));
                    close();
                    open_channel(id);
                }
                Err(e) => store().error(e),
            }
        });
    };
    view! {
        <h2>"New message"</h2>
        <p class="modal-text">"Pick one person for a direct message, or several for a group (up to 10)."</p>
        <input class="wide-input" autofocus=true placeholder="Type a name" prop:value=move || q.get() on:input=move |e| q.set(event_target_value(&e))/>
        <div class="pick-list scroll">
            <For each=people key=|u| u.id let:u>
                {
                    let id = u.id;
                    view! {
                        <label class="pick">
                            <Avatar user=id size=32 status=true/>
                            <span class="grow">{u.display_name.clone()}<span class="muted small">{format!("  {}", u.username)}</span></span>
                            <input type="checkbox" prop:checked=move || picked.with(|p| p.contains(&id)) on:change=move |e| {
                                let on = event_target_checked(&e);
                                picked.update(|p| if on { if p.len() < 10 { p.push(id) } } else { p.retain(|&x| x != id) });
                            }/>
                        </label>
                    }
                }
            </For>
        </div>
        <div class="modal-foot">
            <button class="btn link" on:click=|_| close()>"Cancel"</button>
            <button class="btn primary" disabled=move || picked.with(|p| p.is_empty()) on:click=create>
                {move || if picked.with(|p| p.len() > 1) { "Create group" } else { "Message" }}
            </button>
        </div>
    }
}

#[component]
fn ImageViewer(url: String) -> impl IntoView {
    let open = url.clone();
    view! {
        <img class="lightbox" src=url alt=""/>
        <a class="lightbox-link" href="#" on:click=move |e| { e.prevent_default(); crate::desktop::open_url(&open); }>"Open original"</a>
    }
}

#[component]
fn Pins(channel: Id) -> impl IntoView {
    let s = store();
    let pins = RwSignal::new(None::<Vec<Message>>);
    spawn_local(async move {
        match api::get::<Vec<Message>>(&format!("/api/channels/{channel}/pins")).await {
            Ok(v) => pins.set(Some(v)),
            Err(e) => {
                s.error(e);
                pins.set(Some(vec![]));
            }
        }
    });
    let space = s.channel(channel).and_then(|c| c.space_id);
    view! {
        <h2>"Pinned messages"</h2>
        <div class="pins scroll">
            {move || match pins.get() {
                None => view! { <div class="spinner"></div> }.into_any(),
                Some(v) if v.is_empty() => view! { <div class="empty-hint">"This channel doesn't have any pinned messages yet. Right-click a message and choose Pin Message."</div> }.into_any(),
                Some(v) => v.into_iter().map(|m| {
                    let id = m.id;
                    view! {
                        <div class="pin">
                            <chat::MessageItem m=m head=true space=space/>
                            <div class="pin-actions">
                                <button class="btn small" on:click=move |_| { close(); crate::store::jump_to(channel, id); }>"Jump"</button>
                                <Show when=move || s.is_mod(space)>
                                    <button class="btn small" on:click=move |_| {
                                        crate::store::set_pinned(channel, id, false);
                                        pins.update(|p| if let Some(p) = p { p.retain(|m| m.id != id) });
                                    }>"Unpin"</button>
                                </Show>
                            </div>
                        </div>
                    }
                }).collect_view().into_any(),
            }}
        </div>
    }
}

#[component]
fn NextcloudBrowser() -> impl IntoView {
    let s = store();
    let path = RwSignal::new("/".to_string());
    let entries = RwSignal::new(None::<Result<Vec<NcFile>, String>>);
    let busy = RwSignal::new(false);
    Effect::new(move |_| {
        let p = path.get();
        entries.set(None);
        spawn_local(async move {
            let r = api::get::<Vec<NcFile>>(&format!("/api/nextcloud/files?path={}", api::enc(&p))).await;
            entries.set(Some(r.map_err(|e| e.message)));
        });
    });
    let crumbs = move || {
        let p = path.get();
        let mut acc = String::new();
        let mut v = vec![("Home".to_string(), "/".to_string())];
        for part in p.split('/').filter(|x| !x.is_empty()) {
            acc.push('/');
            acc.push_str(part);
            v.push((part.to_string(), acc.clone()));
        }
        v
    };
    let share = move |f: NcFile| {
        busy.set(true);
        spawn_local(async move {
            match api::call::<_, Attachment>("POST", "/api/nextcloud/share", &NcShare { path: f.path }).await {
                Ok(a) => {
                    super::composer::external().update(|v| v.push(a));
                    close();
                }
                Err(e) => s.error(e),
            }
            busy.set(false);
        });
    };
    view! {
        <h2><Icon name="cloud"/>" Share from Nextcloud"</h2>
        <div class="crumbs">
            {move || crumbs().into_iter().map(|(label, p)| view! {
                <button class="crumb" on:click=move |_| path.set(p.clone())>{label}</button><span class="sep">"/"</span>
            }).collect_view()}
        </div>
        <div class="nc-list scroll">
            {move || match entries.get() {
                None => view! { <div class="spinner"></div> }.into_any(),
                Some(Err(e)) => view! { <div class="form-error">{e}</div> }.into_any(),
                Some(Ok(v)) if v.is_empty() => view! { <div class="empty-hint">"This folder is empty."</div> }.into_any(),
                Some(Ok(mut v)) => {
                    v.sort_by_key(|f| (!f.dir, f.name.to_lowercase()));
                    v.into_iter().map(|f| {
                        let f2 = f.clone();
                        let dir = f.dir;
                        view! {
                            <div class="nc-item" on:click=move |_| if dir { path.set(f2.path.clone()) } else if !busy.get_untracked() { share(f2.clone()) }>
                                <Icon name=if dir { "folder" } else { "file" }/>
                                <span class="ellipsis grow">{f.name.clone()}</span>
                                <span class="muted small">{if dir { String::new() } else { util::file_size(f.size) }}</span>
                            </div>
                        }
                    }).collect_view().into_any()
                }
            }}
        </div>
        <p class="muted small">"Click a file to attach it. It's shared as a read-only link; the file stays in your Nextcloud."</p>
    }
}

#[component]
fn CreateThreadDialog(channel: Id, message: Option<Id>) -> impl IntoView {
    let s = store();
    let space = s.channel(channel).and_then(|c| c.space_id);
    let starter = message.and_then(|m| s.message(channel, m));
    // Suggest a name from the message, like Discord does.
    let suggested: String = starter
        .as_ref()
        .map(|m| crate::markdown::plain(&m.content, &|u| s.name(u, space)))
        .unwrap_or_default()
        .chars()
        .take(40)
        .collect();
    let name = RwSignal::new(suggested.trim().to_string());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        busy.set(true);
        let body = CreateThread { name: name.get_untracked().trim().to_string(), message_id: message };
        spawn_local(async move {
            match api::call::<_, Channel>("POST", &format!("/api/channels/{channel}/threads"), &body).await {
                Ok(t) => {
                    let id = t.id;
                    crate::store::apply(Event::ChannelCreate(t));
                    close();
                    open_channel(id);
                    util::focus(&format!("composer-{id}"));
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    };
    view! {
        <h2><Icon name="thread"/>" Create thread"</h2>
        {starter.map(|m| view! {
            <div class="thread-starter"><chat::MessageItem m=m head=true space=space/></div>
        })}
        <form class="form" on:submit=submit>
            <label>"Thread name"
                <input required=true maxlength="100" autofocus=true placeholder="What's this thread about?"
                    prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e))/>
            </label>
            {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}
            <div class="modal-foot">
                <button type="button" class="btn link" on:click=|_| close()>"Cancel"</button>
                <button type="submit" class="btn primary" disabled=move || busy.get() || name.with(|n| n.trim().is_empty())>"Create thread"</button>
            </div>
        </form>
    }
}

#[component]
fn ThreadBrowser(channel: Id) -> impl IntoView {
    let s = store();
    let list = RwSignal::new(None::<Vec<Channel>>);
    let show_archived = RwSignal::new(false);
    spawn_local(async move {
        match api::get::<Vec<Channel>>(&format!("/api/channels/{channel}/threads")).await {
            Ok(v) => list.set(Some(v)),
            Err(e) => {
                s.error(e);
                list.set(Some(vec![]));
            }
        }
    });
    let space = s.channel(channel).and_then(|c| c.space_id);
    let shown = move || {
        let archived = show_archived.get();
        list.get().map(|v| v.into_iter().filter(|t| t.thread.as_ref().is_some_and(|i| i.archived) == archived).collect::<Vec<_>>())
    };
    view! {
        <h2><Icon name="thread"/>{move || format!(" Threads in #{}", s.channel(channel).map(|c| c.name).unwrap_or_default())}</h2>
        <div class="row gap">
            <div class="tabs grow">
                <button class:active=move || !show_archived.get() on:click=move |_| show_archived.set(false)>"Active"</button>
                <button class:active=move || show_archived.get() on:click=move |_| show_archived.set(true)>"Archived"</button>
            </div>
            <button class="btn primary small" on:click=move |_| s.modal.set(Some(Modal::CreateThread { channel, message: None }))>"Create"</button>
        </div>
        <div class="thread-list scroll">
            {move || match shown() {
                None => view! { <div class="spinner"></div> }.into_any(),
                Some(v) if v.is_empty() => view! {
                    <div class="empty-hint">{if show_archived.get() { "No archived threads." } else { "No active threads. Hover a message and pick the thread icon to start one." }}</div>
                }.into_any(),
                Some(v) => v.into_iter().map(|t| {
                    let id = t.id;
                    let info = t.thread.clone();
                    let count = info.as_ref().map(|i| i.message_count).unwrap_or(0);
                    let owner = info.as_ref().map(|i| i.owner_id).unwrap_or(0);
                    let when = t.last_message_id.unwrap_or(t.id);
                    view! {
                        <div class="thread-item" on:click=move |_| { close(); open_channel(id); }>
                            <Icon name="thread"/>
                            <div class="grow">
                                <div class="thread-name">{t.name.clone()}</div>
                                <div class="muted small">{format!("{} message{} · started by {} · active {}", count, if count == 1 { "" } else { "s" },
                                    s.name(owner, space), util::timestamp(natter_proto::id_time(when)))}</div>
                            </div>
                        </div>
                    }
                }).collect_view().into_any(),
            }}
        </div>
    }
}
