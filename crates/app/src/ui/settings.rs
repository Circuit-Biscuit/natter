//! Settings: your profile, appearance, notifications, voice & video, devices; and space settings.

use leptos::prelude::*;
use leptos::task::spawn_local;
use natter_proto::*;

use super::Avatar;
use super::icons::Icon;
use crate::store::{Modal, store};
use crate::{api, desktop, util, voice};

#[component]
pub fn UserSettings() -> impl IntoView {
    let tab = RwSignal::new("account");
    let tabs = [
        ("account", "My account"),
        ("profile", "Profile"),
        ("appearance", "Appearance"),
        ("notifications", "Notifications"),
        ("voice", "Voice & video"),
        ("devices", "Devices"),
        ("keys", "Keyboard shortcuts"),
    ];
    view! {
        <div class="settings">
            <nav class="settings-nav">
                <div class="settings-group">"USER SETTINGS"</div>
                {tabs.into_iter().map(|(id, label)| view! {
                    <button class:active=move || tab.get() == id on:click=move |_| {
                        if id == "keys" { store().modal.set(Some(Modal::Shortcuts)); } else { tab.set(id); }
                    }>{label}</button>
                }).collect_view()}
                <div class="settings-sep"></div>
                <button class="danger" on:click=|_| crate::store::sign_out()><Icon name="logout" size=16/>" Sign out"</button>
                <div class="settings-version">{format!("Natter {}", env!("CARGO_PKG_VERSION"))}</div>
            </nav>
            <section class="settings-body scroll">
                <button class="settings-close icon-btn" title="Close (Esc)" on:click=|_| store().modal.set(None)><Icon name="x"/></button>
                {move || match tab.get() {
                    "profile" => view! { <ProfileTab/> }.into_any(),
                    "appearance" => view! { <AppearanceTab/> }.into_any(),
                    "notifications" => view! { <NotificationsTab/> }.into_any(),
                    "voice" => view! { <VoiceTab/> }.into_any(),
                    "devices" => view! { <DevicesTab/> }.into_any(),
                    _ => view! { <AccountTab/> }.into_any(),
                }}
            </section>
        </div>
    }
}

#[component]
fn AccountTab() -> impl IntoView {
    let s = store();
    let old = RwSignal::new(String::new());
    let new = RwSignal::new(String::new());
    let msg = RwSignal::new(None::<(bool, String)>);
    let change = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let body = ChangePassword { old_password: old.get_untracked(), new_password: new.get_untracked() };
        spawn_local(async move {
            match api::call::<_, ()>("PUT", "/api/me/password", &body).await {
                Ok(()) => {
                    msg.set(Some((true, "Password changed. Your other devices were signed out.".into())));
                    old.set(String::new());
                    new.set(String::new());
                }
                Err(e) => msg.set(Some((false, e.message))),
            }
        });
    };
    view! {
        <h2>"My account"</h2>
        <div class="account-card">
            {move || view! { <Avatar user=s.me_id() size=72 status=true/> }}
            <div>
                <div class="big-name">{move || s.me.get().map(|u| u.display_name).unwrap_or_default()}</div>
                <div class="muted">{move || s.me.get().map(|u| u.username).unwrap_or_default()}</div>
                <Show when=move || s.nextcloud_session.get()>
                    <div class="muted small"><Icon name="cloud" size=14/>" Signed in with Nextcloud"</div>
                </Show>
            </div>
        </div>
        <Show when=move || !s.nextcloud_session.get() fallback=|| view! {
            <p class="muted">"Your password is managed by Nextcloud."</p>
        }>
            <h3>"Change password"</h3>
            <form class="form narrow" on:submit=change>
                <label>"Current password"<input type="password" required=true autocomplete="current-password" prop:value=move || old.get() on:input=move |e| old.set(event_target_value(&e))/></label>
                <label>"New password"<input type="password" required=true minlength="8" autocomplete="new-password" prop:value=move || new.get() on:input=move |e| new.set(event_target_value(&e))/></label>
                {move || msg.get().map(|(ok, m)| view! { <div class=if ok { "form-ok" } else { "form-error" }>{m}</div> })}
                <button class="btn primary" type="submit">"Change password"</button>
            </form>
        </Show>
        <h3>"Server"</h3>
        <p class="muted">{move || {
            let base = api::server();
            let host = if base.is_empty() { util::window().location().host().unwrap_or_default() } else { base };
            format!("Connected to {} ({})", s.info.get().map(|i| i.name).unwrap_or_default(), host)
        }}</p>
    }
}

#[component]
fn ProfileTab() -> impl IntoView {
    let s = store();
    let me = s.me.get_untracked().unwrap_or_else(|| User {
        id: 0, username: String::new(), display_name: String::new(), avatar: None, about: String::new(),
        custom_status: None, admin: false, created_at: 0,
    });
    let display = RwSignal::new(me.display_name.clone());
    let about = RwSignal::new(me.about.clone());
    let custom = RwSignal::new(me.custom_status.clone().unwrap_or_default());
    let saved = RwSignal::new(false);
    let nc = s.nextcloud_session.get_untracked();
    let save = move |body: UpdateMe| {
        spawn_local(async move {
            match api::call::<_, User>("PATCH", "/api/me", &body).await {
                Ok(u) => {
                    store().me.set(Some(u.clone()));
                    store().users.update(|m| { m.insert(u.id, u); });
                    saved.set(true);
                    set_timeout(move || saved.set(false), std::time::Duration::from_millis(2000));
                }
                Err(e) => store().error(e),
            }
        });
    };
    let pick_avatar = move |e: leptos::ev::Event| {
        let input: web_sys::HtmlInputElement = event_target(&e);
        let Some(f) = input.files().and_then(|l| l.get(0)) else { return };
        spawn_local(async move {
            match api::upload(&f, &f.name(), None).await {
                Ok(a) => save(UpdateMe { avatar: Some(a.id), ..Default::default() }),
                Err(e) => store().error(e),
            }
        });
    };
    view! {
        <h2>"Profile"</h2>
        <div class="profile-edit">
            <div class="avatar-edit">
                {move || view! { <Avatar user=s.me_id() size=96/> }}
                <label class="btn small">"Change avatar"<input type="file" accept="image/*" class="hidden" on:change=pick_avatar/></label>
                <button class="btn link small" on:click=move |_| save(UpdateMe { avatar: Some(0), ..Default::default() })>"Remove"</button>
                {nc.then(|| view! { <div class="muted small">"Your Nextcloud picture is used again at your next sign-in."</div> })}
            </div>
            <form class="form grow" on:submit=move |e| {
                e.prevent_default();
                save(UpdateMe {
                    display_name: Some(display.get_untracked()),
                    about: Some(about.get_untracked()),
                    custom_status: Some(custom.get_untracked()),
                    ..Default::default()
                });
            }>
                <label>"Display name"<input maxlength="100" required=true prop:value=move || display.get() on:input=move |e| display.set(event_target_value(&e))/></label>
                {nc.then(|| view! { <div class="muted small">"Comes from Nextcloud and is refreshed each time you sign in."</div> })}
                <label>"Custom status"<input maxlength="128" placeholder="In a meeting until 3" prop:value=move || custom.get() on:input=move |e| custom.set(event_target_value(&e))/></label>
                <label>"About me"<textarea rows="4" maxlength="190" prop:value=move || about.get() on:input=move |e| about.set(event_target_value(&e))></textarea></label>
                <div class="row gap">
                    <button class="btn primary" type="submit">"Save"</button>
                    <Show when=move || saved.get()><span class="form-ok">"Saved"</span></Show>
                </div>
            </form>
        </div>
    }
}

#[component]
fn AppearanceTab() -> impl IntoView {
    let s = store();
    let theme = move || s.settings.with(|st| st.theme.clone());
    view! {
        <h2>"Appearance"</h2>
        <h3>"Theme"</h3>
        <div class="theme-pick">
            {[("dark", "Dark"), ("light", "Light"), ("system", "Match system")].into_iter().map(|(id, label)| view! {
                <button class=format!("theme-swatch {id}") class:active=move || theme() == id
                    on:click=move |_| s.update_settings(|st| st.theme = id.into())>{label}</button>
            }).collect_view()}
        </div>
        <h3>"Message display"</h3>
        <label class="check"><input type="checkbox" prop:checked=move || s.settings.with(|st| st.compact)
            on:change=move |e| { let v = event_target_checked(&e); s.update_settings(|st| st.compact = v) }/>
            "Compact mode — fit more messages on screen"</label>
        <h3>"Text size"</h3>
        <div class="row gap">
            <input type="range" min="12" max="20" step="1" prop:value=move || s.settings.with(|st| st.font_size.to_string())
                on:input=move |e| { let v = event_target_value(&e).parse().unwrap_or(16); s.update_settings(|st| st.font_size = v) }/>
            <span>{move || format!("{}px", s.settings.with(|st| st.font_size))}</span>
        </div>
    }
}

#[component]
fn NotificationsTab() -> impl IntoView {
    let s = store();
    view! {
        <h2>"Notifications"</h2>
        <label class="check"><input type="checkbox" prop:checked=move || s.settings.with(|st| st.notifications)
            on:change=move |e| {
                let v = event_target_checked(&e);
                if v { desktop::request_notification_permission(); }
                s.update_settings(|st| st.notifications = v)
            }/>
            "Desktop notifications for direct messages and mentions"</label>
        <label class="check"><input type="checkbox" prop:checked=move || s.settings.with(|st| st.sounds)
            on:change=move |e| { let v = event_target_checked(&e); s.update_settings(|st| st.sounds = v) }/>
            "Sounds (messages, joining and leaving calls, mute)"</label>
        <p class="muted">"Mute a channel or a whole space from its right-click menu. Do Not Disturb (click your avatar) silences everything."</p>
        <button class="btn" on:click=move |_| desktop::notify("Natter", "This is what a notification looks like.", None)>"Send a test notification"</button>
    }
}

#[component]
fn VoiceTab() -> impl IntoView {
    let s = store();
    let devices = RwSignal::new(Vec::<voice::Device>::new());
    spawn_local(async move { devices.set(voice::devices().await) });
    let prefs = s.prefs;
    let set = move |f: Box<dyn FnOnce(&mut crate::store::VoicePrefs)>| {
        prefs.update(|p| f(p));
        s.save_prefs();
        voice::apply_devices();
        let p = prefs.get_untracked();
        voice::set_ptt_mode(p.push_to_talk);
        desktop::set_ptt_key(p.push_to_talk.then_some(p.ptt_key.as_str()));
    };
    let recording = RwSignal::new(false);
    let opts = move |kind: &'static str, current: String| {
        let list: Vec<voice::Device> = devices.get().into_iter().filter(|d| d.kind == kind).collect();
        let mut out = vec![view! { <option value="" selected=current.is_empty()>"Default"</option> }.into_any()];
        out.extend(list.into_iter().filter(|d| d.id != "default" && !d.id.is_empty()).map(|d| {
            let sel = d.id == current;
            view! { <option value=d.id.clone() selected=sel>{d.label.clone()}</option> }.into_any()
        }));
        out
    };
    view! {
        <h2>"Voice & video"</h2>
        <div class="row gap">
            <label class="grow">"Input device"
                <select on:change=move |e| { let v = event_target_value(&e); set(Box::new(move |p| p.input = v)) }>
                    {move || opts("audioinput", prefs.with(|p| p.input.clone()))}
                </select>
            </label>
            <label class="grow">"Output device"
                <select on:change=move |e| { let v = event_target_value(&e); set(Box::new(move |p| p.output = v)) }>
                    {move || opts("audiooutput", prefs.with(|p| p.output.clone()))}
                </select>
            </label>
        </div>
        <p class="muted small">"Device names appear after you've allowed microphone access once (join a call)."</p>
        <h3>"Input mode"</h3>
        <label class="check"><input type="radio" name="mode" prop:checked=move || !prefs.with(|p| p.push_to_talk)
            on:change=move |_| set(Box::new(|p| p.push_to_talk = false))/>"Voice activity"</label>
        <label class="check"><input type="radio" name="mode" prop:checked=move || prefs.with(|p| p.push_to_talk)
            on:change=move |_| set(Box::new(|p| p.push_to_talk = true))/>"Push to talk"</label>
        <Show when=move || prefs.with(|p| p.push_to_talk)>
            <div class="row gap">
                <span>"Shortcut:"</span>
                <button class="btn keybind" class:recording=move || recording.get()
                    on:click=move |_| recording.set(true)
                    on:keydown=move |e| {
                        if recording.get_untracked() {
                            e.prevent_default();
                            e.stop_propagation();
                            let code = e.code();
                            recording.set(false);
                            set(Box::new(move |p| p.ptt_key = code));
                        }
                    }>
                    {move || if recording.get() { "Press a key…".to_string() } else { prefs.with(|p| p.ptt_key.clone()) }}
                </button>
                <span class="muted small">{if desktop::is_tauri() { "Works even when Natter is in the background." } else { "In the browser it only works while Natter has focus." }}</span>
            </div>
        </Show>
        <h3>"Processing"</h3>
        <label class="check"><input type="checkbox" prop:checked=move || prefs.with(|p| p.noise_suppression)
            on:change=move |e| { let v = event_target_checked(&e); set(Box::new(move |p| p.noise_suppression = v)) }/>
            "Noise suppression"</label>
        <p class="muted small">"Echo cancellation and automatic gain control are always on."</p>
    }
}

#[component]
fn DevicesTab() -> impl IntoView {
    let s = store();
    let list = RwSignal::new(None::<Vec<SessionInfo>>);
    let load = move || spawn_local(async move {
        match api::get::<Vec<SessionInfo>>("/api/me/sessions").await {
            Ok(v) => list.set(Some(v)),
            Err(e) => s.error(e),
        }
    });
    load();
    view! {
        <h2>"Devices"</h2>
        <p class="muted">"Everywhere you're signed in. Sign out of any device you don't recognise."</p>
        {move || list.get().map(|v| v.into_iter().map(|d| {
            let id = d.id;
            view! {
                <div class="device">
                    <Icon name=if d.device.contains("Desktop") { "screen" } else { "link" }/>
                    <div class="grow">
                        <div>{d.device.clone()} {d.current.then(|| view! { <span class="tag">"THIS DEVICE"</span> })}</div>
                        <div class="muted small">{format!("Last active {} · signed in {}", util::timestamp(d.last_seen), util::day_label(d.created_at))}</div>
                    </div>
                    {(!d.current).then(|| view! {
                        <button class="btn small danger" on:click=move |_| spawn_local(async move {
                            match api::act("DELETE", &format!("/api/me/sessions/{id}")).await {
                                Ok(()) => load(),
                                Err(e) => store().error(e),
                            }
                        })>"Sign out"</button>
                    })}
                </div>
            }
        }).collect_view())}
    }
}

#[component]
pub fn SpaceSettings(space: Id) -> impl IntoView {
    let s = store();
    let tab = RwSignal::new("overview");
    let owner = move || s.space(space).is_some_and(|sp| sp.owner_id == s.me_id());
    view! {
        <div class="settings">
            <nav class="settings-nav">
                <div class="settings-group ellipsis">{move || s.space(space).map(|sp| sp.name.to_uppercase()).unwrap_or_default()}</div>
                {[("overview", "Overview"), ("members", "Members"), ("invites", "Invites"), ("bans", "Bans")].into_iter().map(|(id, label)| view! {
                    <button class:active=move || tab.get() == id on:click=move |_| tab.set(id)>{label}</button>
                }).collect_view()}
                <Show when=owner>
                    <div class="settings-sep"></div>
                    <button class="danger" on:click=move |_| {
                        let name = s.space(space).map(|sp| sp.name).unwrap_or_default();
                        s.confirm(&format!("Delete '{name}'"), "Every channel and message in it is deleted for everyone. This can't be undone.", "Delete space", move || {
                            spawn_local(async move {
                                if let Err(e) = api::act("DELETE", &format!("/api/spaces/{space}")).await { store().error(e); }
                            });
                        });
                    }>"Delete space"</button>
                </Show>
            </nav>
            <section class="settings-body scroll">
                <button class="settings-close icon-btn" title="Close (Esc)" on:click=|_| store().modal.set(None)><Icon name="x"/></button>
                {move || match tab.get() {
                    "members" => view! { <SpaceMembers space=space/> }.into_any(),
                    "invites" => view! { <SpaceInvites space=space/> }.into_any(),
                    "bans" => view! { <SpaceBans space=space/> }.into_any(),
                    _ => view! { <SpaceOverview space=space/> }.into_any(),
                }}
            </section>
        </div>
    }
}

#[component]
fn SpaceOverview(space: Id) -> impl IntoView {
    let s = store();
    let name = RwSignal::new(s.space(space).map(|sp| sp.name).unwrap_or_default());
    let save = move |body: UpdateSpace| spawn_local(async move {
        match api::call::<_, ()>("PATCH", &format!("/api/spaces/{space}"), &body).await {
            Ok(()) => store().toast("Saved.", false),
            Err(e) => store().error(e),
        }
    });
    let pick_icon = move |e: leptos::ev::Event| {
        let input: web_sys::HtmlInputElement = event_target(&e);
        let Some(f) = input.files().and_then(|l| l.get(0)) else { return };
        spawn_local(async move {
            match api::upload(&f, &f.name(), None).await {
                Ok(a) => save(UpdateSpace { icon: Some(a.id), ..Default::default() }),
                Err(e) => store().error(e),
            }
        });
    };
    view! {
        <h2>"Overview"</h2>
        <div class="profile-edit">
            <div class="avatar-edit">
                <div class="space-icon-big">
                    {move || match s.space(space).and_then(|sp| sp.icon) {
                        Some(i) => view! { <img src=api::url(&i) alt=""/> }.into_any(),
                        None => view! { <span>{util::initials(&s.space(space).map(|sp| sp.name).unwrap_or_default())}</span> }.into_any(),
                    }}
                </div>
                <label class="btn small">"Upload icon"<input type="file" accept="image/*" class="hidden" on:change=pick_icon/></label>
                <button class="btn link small" on:click=move |_| save(UpdateSpace { icon: Some(0), ..Default::default() })>"Remove"</button>
            </div>
            <form class="form grow" on:submit=move |e| { e.prevent_default(); save(UpdateSpace { name: Some(name.get_untracked()), ..Default::default() }); }>
                <label>"Space name"<input required=true maxlength="100" prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e))/></label>
                <button class="btn primary" type="submit">"Save"</button>
            </form>
        </div>
    }
}

#[component]
fn SpaceMembers(space: Id) -> impl IntoView {
    let s = store();
    let q = RwSignal::new(String::new());
    let members = move || {
        let query = q.get().to_lowercase();
        let mut v: Vec<Member> = s.space(space).map(|sp| sp.members).unwrap_or_default();
        v.retain(|m| s.name(m.user_id, Some(space)).to_lowercase().contains(&query) || s.user(m.user_id).is_some_and(|u| u.username.contains(&query)));
        v.sort_by_key(|m| (m.role, s.name(m.user_id, Some(space)).to_lowercase()));
        v
    };
    view! {
        <h2>{move || format!("Members — {}", s.space(space).map(|sp| sp.members.len()).unwrap_or(0))}</h2>
        <input class="wide-input" placeholder="Search members" prop:value=move || q.get() on:input=move |e| q.set(event_target_value(&e))/>
        <div class="member-table">
            <For each=members key=|m| (m.user_id, m.role, m.nickname.clone()) let:m>
                {
                    let u = m.user_id;
                    view! {
                        <div class="member-line" on:contextmenu=move |e| s.open_menu(&e, super::menu::user_menu(u, Some(space)))>
                            <Avatar user=u size=32 status=true/>
                            <div class="grow">
                                <div>{move || s.name(u, Some(space))}</div>
                                <div class="muted small">{move || s.user(u).map(|x| x.username).unwrap_or_default()}{format!(" · joined {}", util::day_label(m.joined_at))}</div>
                            </div>
                            <span class="role-chip">{match m.role { Role::Owner => "Owner", Role::Admin => "Admin", Role::Member => "Member" }}</span>
                            <button class="icon-btn" title="Actions" on:click=move |e| s.open_menu(&e, super::menu::user_menu(u, Some(space)))><Icon name="more"/></button>
                        </div>
                    }
                }
            </For>
        </div>
    }
}

#[component]
fn SpaceInvites(space: Id) -> impl IntoView {
    let s = store();
    let list = RwSignal::new(None::<Vec<Invite>>);
    let load = move || spawn_local(async move {
        match api::get::<Vec<Invite>>(&format!("/api/spaces/{space}/invites")).await {
            Ok(v) => list.set(Some(v)),
            Err(e) => s.error(e),
        }
    });
    load();
    view! {
        <h2>"Invites"</h2>
        <button class="btn primary" on:click=move |_| s.modal.set(Some(Modal::Invite(space)))>"Create invite link"</button>
        {move || list.get().map(|v| if v.is_empty() {
            view! { <p class="muted">"No active invites."</p> }.into_any()
        } else {
            v.into_iter().map(|i| {
                let code = i.code.clone();
                view! {
                    <div class="device">
                        <Icon name="link"/>
                        <div class="grow">
                            <div class="mono">{i.code.clone()}</div>
                            <div class="muted small">
                                {format!("by {} · used {}{} · {}", s.name(i.creator_id, Some(space)), i.uses,
                                    i.max_uses.map(|m| format!("/{m}")).unwrap_or_default(),
                                    i.expires_at.map(|e| format!("expires {}", util::timestamp(e))).unwrap_or_else(|| "never expires".into()))}
                            </div>
                        </div>
                        <button class="btn small danger" on:click=move |_| {
                            let c = code.clone();
                            spawn_local(async move {
                                match api::act("DELETE", &format!("/api/invites/{}", api::enc(&c))).await {
                                    Ok(()) => load(),
                                    Err(e) => store().error(e),
                                }
                            });
                        }>"Revoke"</button>
                    </div>
                }
            }).collect_view().into_any()
        })}
    }
}

#[component]
fn SpaceBans(space: Id) -> impl IntoView {
    let s = store();
    let list = RwSignal::new(None::<Vec<Ban>>);
    let load = move || spawn_local(async move {
        match api::get::<Vec<Ban>>(&format!("/api/spaces/{space}/bans")).await {
            Ok(v) => list.set(Some(v)),
            Err(e) => s.error(e),
        }
    });
    load();
    view! {
        <h2>"Bans"</h2>
        {move || list.get().map(|v| if v.is_empty() {
            view! { <p class="muted">"Nobody is banned."</p> }.into_any()
        } else {
            v.into_iter().map(|b| {
                let u = b.user_id;
                view! {
                    <div class="device">
                        <Avatar user=u size=32/>
                        <div class="grow">
                            <div>{s.name(u, None)}</div>
                            <div class="muted small">{format!("{} · {}", if b.reason.is_empty() { "No reason given".to_string() } else { b.reason.clone() }, util::day_label(b.created_at))}</div>
                        </div>
                        <button class="btn small" on:click=move |_| spawn_local(async move {
                            match api::act("DELETE", &format!("/api/spaces/{space}/bans/{u}")).await {
                                Ok(()) => load(),
                                Err(e) => store().error(e),
                            }
                        })>"Unban"</button>
                    </div>
                }
            }).collect_view().into_any()
        })}
    }
}
