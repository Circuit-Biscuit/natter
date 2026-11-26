//! The far-left rail: Home (direct messages), unread DMs, spaces, and "add a space".

use leptos::prelude::*;
use laklak_proto::{Id, Space};

use super::icons::{Icon, Logo};
use super::{Avatar, menu};
use crate::store::{Modal, open_channel, open_space, store};
use crate::{api, util};

#[component]
pub fn Rail() -> impl IntoView {
    let s = store();
    let home_active = move || s.nav.get().space.is_none();
    view! {
        <nav class="rail">
            <div class="rail-item" class:active=home_active title="Direct Messages">
                <span class="pill"></span>
                <button class="rail-btn rail-home" on:click=move |_| {
                    util::storage_remove("laklak.space");
                    open_space(None);
                }><Logo size=28/></button>
            </div>
            <For each=move || s.unread_dms() key=|(c, n)| (c.id, *n) let:item>
                {
                    let (c, n) = item;
                    let id = c.id;
                    let who = s.dm_partner(&c).unwrap_or_else(|| c.recipients.first().copied().unwrap_or(0));
                    let title = s.channel_title(&c);
                    view! {
                        <div class="rail-item unread" title=title>
                            <span class="pill"></span>
                            <button class="rail-btn" on:click=move |_| open_channel(id)><Avatar user=who size=48/></button>
                            <span class="badge">{n}</span>
                        </div>
                    }
                }
            </For>
            <div class="rail-sep"></div>
            <For each=move || s.spaces.get() key=|sp| (sp.id, sp.name.clone(), sp.icon.clone()) let:sp>
                <SpaceButton space=sp/>
            </For>
            <div class="rail-item">
                <button class="rail-btn add" title="Add a space" on:click=move |_| s.modal.set(Some(Modal::CreateSpace))>
                    <Icon name="plus" size=24/>
                </button>
            </div>
        </nav>
    }
}

#[component]
fn SpaceButton(space: Space) -> impl IntoView {
    let s = store();
    let id: Id = space.id;
    let name = space.name.clone();
    let active = move || s.nav.get().space == Some(id);
    let badge = move || s.space_badge(id);
    let icon = space.icon.clone().map(|i| api::url(&i));
    let initials = util::initials(&space.name);
    view! {
        <div class="rail-item" class:active=active class:unread=move || badge().0 title=name.clone()
            on:contextmenu=move |e| s.open_menu(&e, menu::space_menu(id))>
            <span class="pill"></span>
            <button class="rail-btn space" on:click=move |_| {
                util::storage_set("laklak.space", &id.to_string());
                open_space(Some(id));
            }>
                {match icon {
                    Some(src) => view! { <img src=src alt="" draggable="false"/> }.into_any(),
                    None => view! { <span class="space-initials">{initials}</span> }.into_any(),
                }}
            </button>
            {move || (badge().1 > 0).then(|| view! { <span class="badge">{badge().1}</span> })}
        </div>
    }
}
