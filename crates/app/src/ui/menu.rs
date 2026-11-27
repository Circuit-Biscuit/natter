//! Right-click menus for messages, channels, people and spaces.

use leptos::prelude::*;
use leptos::task::spawn_local;
use laklak_proto::*;

use crate::store::{MenuItem, Modal, store};
use crate::{api, util, voice};

#[component]
pub fn MenuHost() -> impl IntoView {
    let s = store();
    view! {
        {move || s.menu.get().map(|m| {
            let win = util::window();
            let vw = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1200.0) as i32;
            let vh = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(800.0) as i32;
            let h = m.items.iter().map(|i| if i.is_some() { 34 } else { 9 }).sum::<i32>() + 12;
            let x = m.x.min(vw - 236).max(4);
            let y = if m.y + h > vh { (m.y - h).max(4) } else { m.y };
            view! {
                <div class="menu" style:left=format!("{x}px") style:top=format!("{y}px") on:click=|e| e.stop_propagation()
                    on:contextmenu=|e| e.prevent_default()>
                    {m.items.into_iter().map(|it| match it {
                        None => view! { <div class="menu-sep"></div> }.into_any(),
                        Some(it) => {
                            let action = it.action;
                            view! {
                                <button class="menu-item" class:danger=it.danger on:click=move |_| {
                                    s.menu.set(None);
                                    action();
                                }>
                                    {it.dot.map(|d| view! { <span class=format!("status-dot {d}")></span> })}
                                    {it.label}
                                </button>
                            }.into_any()
                        }
                    }).collect_view()}
                </div>
            }
        })}
    }
}

pub fn message_menu(m: &Message) -> Vec<Option<MenuItem>> {
    let s = store();
    let (channel, id, author) = (m.channel_id, m.id, m.author_id);
    let space = s.channel(channel).and_then(|c| c.space_id);
    let mine = author == s.me_id();
    let can_pin = s.can_pin_in(channel);
    let can_delete = space.is_some() && s.can_in(channel, perm::MANAGE_MESSAGES);
    let pinned = m.pinned;
    let text = m.content.clone();
    let menu_pos = s.menu;
    let mut v = vec![
        Some(MenuItem::new("Add Reaction", move || {
            let (x, y) = menu_pos.with_untracked(|m| m.as_ref().map(|m| (m.x, m.y)).unwrap_or((400, 400)));
            super::emoji::open_picker_at(x, y, move |em: String| crate::store::toggle_reaction(channel, id, em));
        })),
        Some(MenuItem::new("Reply", move || {
            s.reply_to.set(Some(id));
            util::focus(&format!("composer-{channel}"));
        })),
    ];
    if mine && m.kind == MessageKind::Default {
        v.push(Some(MenuItem::new("Edit Message", move || s.editing.set(Some(id)))));
    }
    let threadable = m.thread.is_none() && m.kind == MessageKind::Default
        && s.channel(channel).is_some_and(|c| c.kind == ChannelKind::Text && c.space_id.is_some());
    if threadable {
        v.push(Some(MenuItem::new("Create Thread", move || s.modal.set(Some(Modal::CreateThread { channel, message: Some(id) })))));
    }
    if let Some(t) = m.thread.as_ref().map(|t| t.id) {
        v.push(Some(MenuItem::new("Open Thread", move || crate::store::open_channel(t))));
    }
    if can_pin {
        v.push(Some(MenuItem::new(if pinned { "Unpin Message" } else { "Pin Message" }, move || crate::store::set_pinned(channel, id, !pinned))));
    }
    v.push(None);
    v.push(Some(MenuItem::new("Copy Text", move || util::copy_text(&text))));
    v.push(Some(MenuItem::new("Mark Unread", move || crate::store::mark_unread(channel, id))));
    v.push(Some(MenuItem::new("Copy Message ID", move || util::copy_text(&id.to_string()))));
    let moderation = s.info.with_untracked(|i| i.as_ref().is_some_and(|i| i.moderation));
    if moderation && !mine && s.user(author).is_some_and(|u| !u.bot) {
        v.push(None);
        v.push(Some(MenuItem::new("Report Message…", move || s.modal.set(Some(Modal::Report { channel, message: id }))).danger()));
    }
    if mine || can_delete {
        v.push(None);
        v.push(Some(MenuItem::new("Delete Message", move || crate::store::delete_with_confirm(channel, id)).danger()));
    }
    v
}

pub fn channel_menu(id: Id) -> Vec<Option<MenuItem>> {
    let s = store();
    let Some(c) = s.channel(id) else { return vec![] };
    let muted = s.muted(id);
    let thread_owner = c.thread.as_ref().is_some_and(|t| t.owner_id == s.me_id());
    let need = if c.kind == ChannelKind::Thread { perm::MANAGE_THREADS } else { perm::MANAGE_CHANNELS };
    let can_mod = c.space_id.is_some() && (s.can_in(id, need) || thread_owner);
    let can_permissions = c.space_id.is_some() && c.kind != ChannelKind::Thread && s.can_in(id, perm::MANAGE_ROLES);
    let mut v = Vec::new();
    if c.kind == ChannelKind::Text && c.space_id.is_some() {
        v.push(Some(MenuItem::new("Create Thread", move || s.modal.set(Some(Modal::CreateThread { channel: id, message: None })))));
        v.push(Some(MenuItem::new("Browse Threads", move || s.modal.set(Some(Modal::Threads(id))))));
    }
    if c.kind != ChannelKind::Category {
        v.push(Some(MenuItem::new("Mark As Read", move || crate::store::ack(id, None))));
        v.push(Some(MenuItem::new(if muted { "Unmute" } else { "Mute" }, move || {
            s.update_settings(|st| {
                if muted { st.muted.retain(|&x| x != id) } else { st.muted.push(id) }
            })
        })));
    }
    if c.kind.is_dm() {
        v.push(Some(MenuItem::new("Start Call", move || {
            crate::store::open_channel(id);
            voice::call_dm(id);
        })));
    }
    if let Some(sp) = c.space_id.filter(|&sp| s.can(Some(sp), perm::CREATE_INVITES)) {
        v.push(Some(MenuItem::new("Invite People", move || s.modal.set(Some(Modal::Invite(sp))))));
    }
    v.push(None);
    if can_mod {
        let sp = c.space_id.unwrap_or(0);
        if c.kind == ChannelKind::Category {
            v.push(Some(MenuItem::new("Create Channel", move || s.modal.set(Some(Modal::CreateChannel { space: sp, kind: ChannelKind::Text, parent: Some(id) })))));
        }
        if c.kind != ChannelKind::Thread {
            v.push(Some(MenuItem::new(if c.kind == ChannelKind::Category { "Edit Category" } else { "Edit Channel" }, move || s.modal.set(Some(Modal::EditChannel(id))))));
        }
    }
    if can_permissions {
        v.push(Some(MenuItem::new("Permissions", move || s.modal.set(Some(Modal::ChannelPermissions(id))))));
    }
    if let Some(sp) = c.space_id.filter(|_| c.kind != ChannelKind::Category) {
        v.push(Some(MenuItem::new("Copy Channel Link", move || {
            util::copy_text(&format!("{}/channels/{sp}/{id}", util::link_base()));
            store().toast("Channel link copied", false);
        })));
    }
    v.push(Some(MenuItem::new("Copy ID", move || util::copy_text(&id.to_string()))));
    if can_mod {
        let name = c.name.clone();
        v.push(None);
        let what = match c.kind { ChannelKind::Category => "Delete Category", ChannelKind::Thread => "Delete Thread", _ => "Delete Channel" };
        v.push(Some(MenuItem::new(what, move || {
            let n = name.clone();
            s.confirm(&format!("Delete '{n}'"), "This can't be undone. All its messages are deleted for everyone.", "Delete", move || {
                spawn_local(async move {
                    if let Err(e) = api::act("DELETE", &format!("/api/channels/{id}")).await {
                        store().error(e);
                    }
                });
            });
        }).danger()));
    }
    v
}

pub fn user_menu(user: Id, space: Option<Id>) -> Vec<Option<MenuItem>> {
    let s = store();
    let me = s.me_id();
    let Some(u) = s.user(user) else { return vec![] };
    let username = u.username.clone();
    let username2 = u.username.clone();
    let mut v = vec![
        Some(MenuItem::new("Profile", move || {
            let (x, y) = s.menu.with_untracked(|m| m.as_ref().map(|m| (m.x, m.y)).unwrap_or((300, 200)));
            s.popout.set(Some(crate::store::Popout { user, x, y }));
        })),
    ];
    if user != me {
        v.push(Some(MenuItem::new("Message", move || crate::store::open_dm(user))));
        v.push(Some(MenuItem::new("Call", move || {
            let s = store();
            let existing = s.dms.with_untracked(|d| d.iter().find(|c| c.kind == ChannelKind::Dm && c.recipients.contains(&user)).map(|c| c.id));
            match existing {
                Some(c) => {
                    crate::store::open_channel(c);
                    voice::call_dm(c);
                }
                None => {
                    crate::store::open_dm(user);
                    s.toast("Opened your conversation — press the call button to ring.", false);
                }
            }
        })));
    }
    v.push(Some(MenuItem::new("Mention", move || {
        let s = store();
        if let Some(ch) = s.nav.get_untracked().channel {
            s.drafts.update(|d| {
                let t = d.entry(ch).or_default();
                if !t.is_empty() && !t.ends_with(' ') {
                    t.push(' ');
                }
                t.push_str(&format!("@{username} "));
            });
            // Re-open the channel so the composer picks the draft up.
            crate::store::open_channel(ch);
            util::focus(&format!("composer-{ch}"));
        }
    })));
    v.push(None);
    v.push(Some(MenuItem::new("Copy Username", move || util::copy_text(&username2))));
    v.push(Some(MenuItem::new("Copy User ID", move || util::copy_text(&user.to_string()))));
    if user != me && !u.bot && s.info.with_untracked(|i| i.as_ref().is_some_and(|i| i.moderation)) {
        let blocked = s.blocked.with_untracked(|b| b.contains(&user));
        let name = u.display_name.clone();
        v.push(Some(if blocked {
            MenuItem::new("Unblock", move || crate::store::set_blocked(user, false))
        } else {
            MenuItem::new("Block…", move || {
                store().confirm(
                    &format!("Block {name}?"),
                    "Their messages are hidden for you (you can still open each one), and they can't message or ring you. They aren't told.",
                    "Block",
                    move || crate::store::set_blocked(user, true),
                )
            })
            .danger()
        }));
    }
    // Server admins can give someone a temporary password (for a lost one).
    if user != me && !u.bot && s.me.with_untracked(|m| m.as_ref().is_some_and(|m| m.admin)) {
        v.push(Some(MenuItem::new("Reset Password…", move || store().modal.set(Some(Modal::ResetPassword(user)))).danger()));
    }
    if let Some(sp) = space {
        let member = s.member(sp, user);
        let outranks = s.outranks(sp, user);
        if user != me && member.is_some() {
            // Making someone admin = giving them a role with Administrator (every space starts with "Admin").
            let space_info = s.space(sp);
            let my_rank = s.rank(sp, me);
            let admin_roles: Vec<RoleDef> = space_info.map(|x| x.roles).unwrap_or_default().into_iter()
                .filter(|r| r.permissions & perm::ADMINISTRATOR != 0 && r.position < my_rank).collect();
            if s.can(Some(sp), perm::MANAGE_ROLES) && s.can(Some(sp), perm::ADMINISTRATOR) && !admin_roles.is_empty() {
                let held: Vec<Id> = admin_roles.iter().map(|r| r.id).filter(|r| member.as_ref().is_some_and(|m| m.roles.contains(r))).collect();
                let first = admin_roles[0].id;
                v.push(None);
                v.push(Some(MenuItem::new(if held.is_empty() { "Make Admin" } else { "Remove Admin" }, move || {
                    let held = held.clone();
                    spawn_local(async move {
                        let r = if held.is_empty() {
                            api::act("PUT", &format!("/api/spaces/{sp}/members/{user}/roles/{first}")).await
                        } else {
                            let mut r = Ok(());
                            for role in held {
                                r = r.and(api::act("DELETE", &format!("/api/spaces/{sp}/members/{user}/roles/{role}")).await);
                            }
                            r
                        };
                        if let Err(e) = r {
                            store().error(e);
                        }
                    });
                })));
            }
            if s.can(Some(sp), perm::MANAGE_ROLES) {
                v.push(Some(MenuItem::new("Manage Roles", move || {
                    let (x, y) = store().menu.with_untracked(|m| m.as_ref().map(|m| (m.x, m.y)).unwrap_or((400, 300)));
                    store().popout.set(Some(crate::store::Popout { user, x, y }));
                })));
            }
            let can_kick = outranks && s.can(Some(sp), perm::KICK_MEMBERS);
            let can_ban = outranks && s.can(Some(sp), perm::BAN_MEMBERS);
            if can_kick || can_ban {
                let name = s.name(user, Some(sp));
                let name2 = name.clone();
                v.push(None);
                if can_kick {
                v.push(Some(MenuItem::new(format!("Kick {name}"), move || {
                    let n = name.clone();
                    store().confirm(&format!("Kick {n}"), "They can come back with a new invite.", "Kick", move || {
                        spawn_local(async move {
                            if let Err(e) = api::act("DELETE", &format!("/api/spaces/{sp}/members/{user}")).await {
                                store().error(e);
                            }
                        });
                    });
                }).danger()));
                }
                if can_ban {
                v.push(Some(MenuItem::new(format!("Ban {name2}"), move || {
                    let n = name2.clone();
                    store().confirm(&format!("Ban {n}"), "They are removed and can't rejoin. Their messages from the last hour are deleted.", "Ban", move || {
                        spawn_local(async move {
                            let body = BanReq { reason: String::new(), delete_message_secs: 3600 };
                            if let Err(e) = api::call::<_, ()>("PUT", &format!("/api/spaces/{sp}/bans/{user}"), &body).await {
                                store().error(e);
                            }
                        });
                    });
                }).danger()));
                }
            }
        }
    }
    v
}

pub fn space_menu(id: Id) -> Vec<Option<MenuItem>> {
    let s = store();
    let Some(sp) = s.space(id) else { return vec![] };
    let muted = s.muted(id);
    let p = s.perms(Some(id));
    let can_settings = p & (perm::MANAGE_SPACE | perm::MANAGE_ROLES | perm::BAN_MEMBERS | perm::KICK_MEMBERS) != 0;
    let can_channels = p & perm::MANAGE_CHANNELS != 0;
    let owner = sp.owner_id == s.me_id();
    let mut v = vec![
        Some(MenuItem::new("Mark As Read", move || crate::store::mark_space_read(id))),
        (p & perm::CREATE_INVITES != 0).then(|| MenuItem::new("Invite People", move || s.modal.set(Some(Modal::Invite(id))))),
        Some(MenuItem::new("Edit Space Profile", move || s.modal.set(Some(Modal::SpaceProfile(id))))),
        Some(MenuItem::new(if muted { "Unmute Space" } else { "Mute Space" }, move || {
            s.update_settings(|st| if muted { st.muted.retain(|&x| x != id) } else { st.muted.push(id) })
        })),
    ];
    if can_settings || can_channels {
        v.push(None);
    }
    if can_settings {
        v.push(Some(MenuItem::new("Space Settings", move || s.modal.set(Some(Modal::SpaceSettings(id))))));
    }
    if can_channels {
        v.push(Some(MenuItem::new("Create Channel", move || s.modal.set(Some(Modal::CreateChannel { space: id, kind: ChannelKind::Text, parent: None })))));
        v.push(Some(MenuItem::new("Create Category", move || s.modal.set(Some(Modal::CreateChannel { space: id, kind: ChannelKind::Category, parent: None })))));
    }
    v.push(None);
    v.push(Some(MenuItem::new("Copy Space ID", move || util::copy_text(&id.to_string()))));
    if !owner {
        let name = sp.name.clone();
        v.push(Some(MenuItem::new("Leave Space", move || {
            let me = store().me_id();
            store().confirm(&format!("Leave '{name}'"), "You won't be able to rejoin unless you are invited again.", "Leave", move || {
                spawn_local(async move {
                    if let Err(e) = api::act("DELETE", &format!("/api/spaces/{id}/members/{me}")).await {
                        store().error(e);
                    }
                });
            });
        }).danger()));
    }
    v
}
