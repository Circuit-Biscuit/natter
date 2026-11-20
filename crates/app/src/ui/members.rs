//! Member list (right column) and the profile popout.

use leptos::prelude::*;
use natter_proto::*;

use super::icons::Icon;
use super::{Avatar, menu, status_label};
use crate::store::{Modal, open_dm, store};
use crate::util;

#[component]
pub fn MemberList() -> impl IntoView {
    let s = store();
    let groups = move || {
        let Some(c) = s.nav.get().channel.and_then(|c| s.channel(c)) else { return vec![] };
        let entries: Vec<(Id, Option<Role>)> = match c.space_id.and_then(|sp| s.space(sp)) {
            Some(sp) => sp.members.iter().map(|m| (m.user_id, Some(m.role))).collect(),
            None => c.recipients.iter().map(|&u| (u, None)).collect(),
        };
        let space = c.space_id;
        let mut admins = vec![];
        let mut online = vec![];
        let mut offline = vec![];
        for (u, role) in entries {
            let on = s.status(u) != Status::Offline;
            match (on, role) {
                (true, Some(Role::Owner | Role::Admin)) => admins.push(u),
                (true, _) => online.push(u),
                (false, _) => offline.push(u),
            }
        }
        let sort = |v: &mut Vec<Id>| v.sort_by_key(|&u| s.name(u, space).to_lowercase());
        sort(&mut admins);
        sort(&mut online);
        sort(&mut offline);
        let mut out = vec![];
        for (label, v) in [("Admins", admins), ("Online", online), ("Offline", offline)] {
            if !v.is_empty() {
                out.push((format!("{label} — {}", v.len()), v, space));
            }
        }
        out
    };
    view! {
        <aside class="members scroll">
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
    let role_class = move || match space.and_then(|sp| s.member(sp, user)).map(|m| m.role) {
        Some(Role::Owner) => "name owner",
        Some(Role::Admin) => "name admin",
        _ => "name",
    };
    view! {
        <div class="member" class:offline=move || s.status(user) == Status::Offline
            on:click=move |e| s.open_profile(&e, user)
            on:contextmenu=move |e| s.open_menu(&e, menu::user_menu(user, space))>
            <Avatar user=user size=32 status=true/>
            <div class="member-text">
                <span class=move || format!("{} ellipsis", role_class())>
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
                {member.as_ref().filter(|m| m.role != Role::Member).map(|m| view! {
                    <div class="profile-section">
                        <div class="profile-label">"Role"</div>
                        <span class="role-chip">{if m.role == Role::Owner { "Owner" } else { "Admin" }}</span>
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
