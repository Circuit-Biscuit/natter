//! Member list (right column) and the profile popout.

use leptos::prelude::*;
use leptos::task::spawn_local;
use natter_proto::*;

use super::icons::Icon;
use super::{Avatar, menu, status_label};
use crate::store::{Modal, open_dm, store};
use crate::{api, util};

#[component]
pub fn MemberList() -> impl IntoView {
    let s = store();
    // Like Discord: online members under their highest "display separately" role, then Online, then Offline.
    let groups = move || {
        let Some(c) = s.nav.get().channel.and_then(|c| s.channel(c)) else { return vec![] };
        let space = c.space_id;
        let sp = space.and_then(|id| s.space(id));
        let users: Vec<Id> = match &sp {
            Some(sp) => sp.members.iter().map(|m| m.user_id).collect(),
            None => c.recipients.clone(),
        };
        let mut hoisted: Vec<(i64, String, Vec<Id>)> = vec![];
        let mut online = vec![];
        let mut offline = vec![];
        for u in users {
            if s.status(u) == Status::Offline {
                offline.push(u);
                continue;
            }
            match sp.as_ref().and_then(|sp| sp.roles_of(u).into_iter().find(|r| r.hoist).cloned()) {
                Some(r) => match hoisted.iter_mut().find(|(pos, ..)| *pos == r.position) {
                    Some(g) => g.2.push(u),
                    None => hoisted.push((r.position, r.name.clone(), vec![u])),
                },
                None => online.push(u),
            }
        }
        hoisted.sort_by_key(|(pos, ..)| std::cmp::Reverse(*pos));
        let mut out = vec![];
        for (label, mut v) in hoisted.into_iter().map(|(_, n, v)| (n, v)).chain([("Online".to_string(), online), ("Offline".to_string(), offline)]) {
            if !v.is_empty() {
                v.sort_by_key(|&u| s.name(u, space).to_lowercase());
                out.push((format!("{label} — {}", v.len()), v, space));
            }
        }
        out
    };
    view! {
        <aside class="members scroll">
            <Show when=move || s.mobile.get()>
                <header class="chat-head">
                    <button class="icon-btn back" title="Back" on:click=|_| crate::store::go_back()><Icon name="arrow-left"/></button>
                    <span class="title">"Members"</span>
                </header>
            </Show>
            <For each=groups key=|(l, v, _)| (l.clone(), v.clone()) let:g>
                {
                    let (label, users, space) = g;
                    view! {
                        <div class="members-label">{label}</div>
                        {users.into_iter().map(|u| view! { <MemberRow user=u space=space/> }).collect_view()}
                    }
                }
            </For>
        </aside>
    }
}

#[component]
fn MemberRow(user: Id, space: Option<Id>) -> impl IntoView {
    let s = store();
    let color = move || s.role_color(space, user);
    view! {
        <div class="member" class:offline=move || s.status(user) == Status::Offline
            on:click=move |e| s.open_profile(&e, user)
            on:contextmenu=move |e| s.open_menu(&e, menu::user_menu(user, space))>
            <Avatar user=user size=32 status=true/>
            <div class="member-text">
                <span class="name ellipsis" style:color=color>
                    {move || s.name(user, space)}
                    {move || (space.and_then(|sp| s.space(sp)).map(|sp| sp.owner_id) == Some(user)).then(|| view! { <span class="crown" title="Space owner">"👑"</span> })}
                </span>
                {move || s.user(user).and_then(|u| u.custom_status).map(|c| view! { <span class="sub ellipsis">{c}</span> })}
            </div>
            {move || s.voice.with(|v| v.get(&user).map(|v| v.channel_id)).map(|_| view! { <span class="in-call" title="In a call"><Icon name="speaker" size=14/></span> })}
        </div>
    }
}

#[component]
pub fn PopoutHost() -> impl IntoView {
    let s = store();
    view! {
        {move || s.popout.get().map(|p| view! { <Profile user=p.user x=p.x y=p.y/> })}
        <super::emoji::PickerHost/>
    }
}

#[component]
fn Profile(user: Id, x: i32, y: i32) -> impl IntoView {
    let s = store();
    let me = s.me_id();
    let space = s.nav.get_untracked().space;
    let win = util::window();
    let vw = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1200.0) as i32;
    let vh = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(800.0) as i32;
    let (w, h) = (320, 420);
    let left = if x + w + 16 > vw { (x - w - 16).max(8) } else { x + 16 };
    let top = (y - 60).clamp(8, (vh - h - 8).max(8));
    let u = move || s.user(user);
    let member = space.and_then(|sp| s.member(sp, user));
    let note = RwSignal::new(String::new());
    let send = move || {
        let text = note.get_untracked();
        if text.trim().is_empty() {
            return;
        }
        open_dm(user);
        // Wait for the DM to open, then send.
        set_timeout(
            move || {
                if let Some(c) = store().nav.get_untracked().channel {
                    crate::store::send_message(c, text, vec![], false, false);
                }
            },
            std::time::Duration::from_millis(400),
        );
    };
    view! {
        <div class="profile" style:left=format!("{left}px") style:top=format!("{top}px") on:click=|e| e.stop_propagation()>
            <div class="profile-banner" style:background=util::avatar_color(user)></div>
            <div class="profile-avatar"><Avatar user=user size=80 status=true/></div>
            <div class="profile-body">
                <div class="profile-name">{move || u().map(|u| u.display_name).unwrap_or_default()}</div>
                <div class="profile-username">{move || u().map(|u| u.username).unwrap_or_default()}
                    {move || u().filter(|u| u.admin).map(|_| view! { <span class="tag">"SERVER ADMIN"</span> })}
                </div>
                {move || u().and_then(|u| u.custom_status).map(|c| view! { <div class="profile-status">{c}</div> })}
                <div class="profile-section">
                    <div class="profile-label">"Status"</div>
                    <div>{move || status_label(s.status(user))}</div>
                </div>
                {move || u().filter(|u| !u.about.is_empty()).map(|u| view! {
                    <div class="profile-section">
                        <div class="profile-label">"About me"</div>
                        <div class="profile-about">{crate::markdown::render(&u.about, None)}</div>
                    </div>
                })}
                <div class="profile-section">
                    <div class="profile-label">"Member since"</div>
                    <div class="muted">
                        {move || u().map(|u| util::day_label(u.created_at)).unwrap_or_default()}
                        {member.as_ref().map(|m| format!(" · joined this space {}", util::day_label(m.joined_at)))}
                    </div>
                </div>
                {space.filter(|_| member.is_some()).map(|sp| view! {
                    <div class="profile-section">
                        <div class="profile-label">"Roles"</div>
                        <RoleChips space=sp user=user/>
                    </div>
                })}
                {if user == me {
                    view! {
                        <button class="btn wide" on:click=move |_| { s.popout.set(None); s.modal.set(Some(Modal::UserSettings)); }>
                            <Icon name="edit" size=16/>"Edit profile"
                        </button>
                    }.into_any()
                } else {
                    view! {
                        <input class="profile-note" placeholder=move || format!("Message @{}", u().map(|u| u.username).unwrap_or_default())
                            prop:value=move || note.get() on:input=move |e| note.set(event_target_value(&e))
                            on:keydown=move |e| if e.key() == "Enter" { send(); }/>
                    }.into_any()
                }}
            </div>
        </div>
    }
}

/// A member's roles as chips; people with Manage roles can add roles below their own and remove them.
#[component]
pub fn RoleChips(space: Id, user: Id) -> impl IntoView {
    let s = store();
    let picking = RwSignal::new(false);
    let can_manage = move || s.can(Some(space), perm::MANAGE_ROLES);
    let my_rank = move || s.rank(space, s.me_id());
    let held = move || s.space(space).map(|sp| sp.roles_of(user).into_iter().cloned().collect::<Vec<_>>()).unwrap_or_default();
    let assignable = move || {
        let mine = s.perms(Some(space));
        let have: Vec<Id> = held().iter().map(|r| r.id).collect();
        s.space(space)
            .map(|sp| sp.roles.into_iter().filter(|r| r.position < my_rank() && r.permissions & !mine == 0 && !have.contains(&r.id)).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let set = move |role: Id, add: bool| {
        spawn_local(async move {
            let path = format!("/api/spaces/{space}/members/{user}/roles/{role}");
            if let Err(e) = api::act(if add { "PUT" } else { "DELETE" }, &path).await {
                store().error(e);
            }
        })
    };
    view! {
        <div class="role-chips">
            {move || held().into_iter().map(|r| {
                let id = r.id;
                let removable = can_manage() && r.position < my_rank();
                view! {
                    <span class="role-chip">
                        <span class="role-dot" style:background=role_swatch(&r.color)></span>
                        {(!r.emoji.is_empty()).then(|| view! { <span class="role-emoji">{r.emoji.clone()}</span> })}
                        <span>{r.name.clone()}</span>
                        {removable.then(|| view! { <button class="chip-x" title="Remove role" on:click=move |_| set(id, false)>"×"</button> })}
                    </span>
                }
            }).collect_view()}
            <Show when=move || can_manage() && !assignable().is_empty()>
                <button class="role-chip add" title="Add role" on:click=move |_| picking.update(|p| *p = !*p)>"+"</button>
            </Show>
            {move || (held().is_empty() && !can_manage()).then(|| view! { <span class="muted small">"No roles"</span> })}
        </div>
        <Show when=move || picking.get()>
            <div class="role-pick">
                {move || assignable().into_iter().map(|r| {
                    let id = r.id;
                    view! {
                        <button on:click=move |_| { picking.set(false); set(id, true); }>
                            <span class="role-dot" style:background=role_swatch(&r.color)></span>
                            {(!r.emoji.is_empty()).then(|| r.emoji.clone())}" "{r.name.clone()}
                        </button>
                    }
                }).collect_view()}
            </div>
        </Show>
    }
}

/// A role's colour for dots and swatches (grey when it has none).
pub fn role_swatch(color: &str) -> String {
    if color.is_empty() { "#99aab5".into() } else { color.into() }
}
