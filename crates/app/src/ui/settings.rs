//! Settings: your profile, appearance, notifications, voice & video, devices; and space settings.

use leptos::prelude::*;
use leptos::task::spawn_local;
use natter_proto::*;

use super::Avatar;
use super::icons::Icon;
use crate::store::{Keybind, Modal, store};
use crate::{api, desktop, sound, util, voice};

#[component]
pub fn UserSettings() -> impl IntoView {
    let tab = RwSignal::new("account");
    let item = move |id: &'static str, label: &'static str| view! {
        <button class:active=move || tab.get() == id on:click=move |_| tab.set(id)>{label}</button>
    };
    view! {
        <div class="settings">
            <nav class="settings-nav">
                <div class="settings-group">"USER SETTINGS"</div>
                {item("account", "My account")}
                {item("profile", "Profile")}
                {item("devices", "Devices")}
                <div class="settings-sep"></div>
                <div class="settings-group">"APP SETTINGS"</div>
                {item("appearance", "Appearance")}
                {item("accessibility", "Accessibility")}
                {item("notifications", "Notifications")}
                {item("voice", "Voice & video")}
                {item("keybinds", "Keybinds")}
                {desktop::is_desktop().then(|| item("system", "System"))}
                <button on:click=|_| store().modal.set(Some(Modal::Shortcuts))>"Keyboard shortcuts"</button>
                <div class="settings-sep"></div>
                <button class="danger" on:click=|_| crate::store::sign_out()><Icon name="logout" size=16/>" Sign out"</button>
                <div class="settings-version">{format!("Natter {}", env!("CARGO_PKG_VERSION"))}</div>
            </nav>
            <section class="settings-body scroll">
                <button class="settings-close icon-btn" title="Close (Esc)" on:click=|_| store().modal.set(None)><Icon name="x"/></button>
                {move || match tab.get() {
                    "profile" => view! { <ProfileTab/> }.into_any(),
                    "appearance" => view! { <AppearanceTab/> }.into_any(),
                    "accessibility" => view! { <AccessibilityTab/> }.into_any(),
                    "notifications" => view! { <NotificationsTab/> }.into_any(),
                    "voice" => view! { <VoiceTab/> }.into_any(),
                    "keybinds" => view! { <KeybindsTab/> }.into_any(),
                    "system" => view! { <SystemTab/> }.into_any(),
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

// ---------------------------------------------------------------- appearance

/// A colour theme: a gradient behind see-through panels. The colours live in style.css (`.grad-<id>`).
pub struct Gradient {
    pub id: &'static str,
    pub label: &'static str,
    /// Light panels and text on it (else dark).
    pub light: bool,
}

const fn g(id: &'static str, label: &'static str, light: bool) -> Gradient {
    Gradient { id, label, light }
}

pub const GRADIENTS: [Gradient; 14] = [
    g("mint", "Mint", true),
    g("peach", "Peach", true),
    g("lavender", "Lavender", true),
    g("candy", "Cotton candy", true),
    g("sky", "Sky", true),
    g("sand", "Sand", true),
    g("twilight", "Twilight", false),
    g("ocean", "Ocean", false),
    g("forest", "Forest", false),
    g("ember", "Ember", false),
    g("grape", "Grape", false),
    g("sunset", "Sunset", false),
    g("neon", "Neon", false),
    g("abyss", "Abyss", false),
];

pub fn gradient(id: &str) -> Option<&'static Gradient> {
    GRADIENTS.iter().find(|g| !id.is_empty() && g.id == id)
}

const BASE_THEMES: [(&str, &str); 4] = [("light", "Light"), ("dark", "Dark"), ("darker", "Darker"), ("midnight", "Midnight")];

/// (value, label, swatch); the empty value is the default blurple.
const ACCENTS: [(&str, &str, &str); 9] = [
    ("", "Blurple", "#5d68ec"),
    ("#23a55a", "Green", "#23a55a"),
    ("#13a8a0", "Teal", "#13a8a0"),
    ("#2b8cff", "Blue", "#2b8cff"),
    ("#9b59f6", "Purple", "#9b59f6"),
    ("#e94f9c", "Pink", "#e94f9c"),
    ("#e5484d", "Red", "#e5484d"),
    ("#f28c28", "Orange", "#f28c28"),
    ("#c99a06", "Gold", "#c99a06"),
];

/// Names match `APP_ICONS` in glue.js.
const APP_ICONS: [(&str, &str); 10] = [
    ("classic", "Classic"),
    ("midnight", "Midnight"),
    ("snow", "Snow"),
    ("sunset", "Sunset"),
    ("ocean", "Ocean"),
    ("forest", "Forest"),
    ("candy", "Candy"),
    ("gold", "Gold"),
    ("crimson", "Crimson"),
    ("mint", "Mint"),
];

/// A settings row with a label, a hint and a switch on the right.
fn switch(
    label: &'static str,
    hint: &'static str,
    get: impl Fn() -> bool + Send + Sync + 'static,
    set: impl Fn(bool) + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <label class="switch-row">
            <div class="grow">
                <div class="switch-label">{label}</div>
                {(!hint.is_empty()).then(|| view! { <div class="muted small">{hint}</div> })}
            </div>
            <input type="checkbox" class="switch" prop:checked=get on:change=move |e| set(event_target_checked(&e))/>
        </label>
    }
}

/// A slider that updates live and saves when let go.
fn slider(
    (min, max, step): (i32, i32, i32),
    unit: &'static str,
    get: impl Fn() -> i32 + Copy + Send + Sync + 'static,
    set: impl Fn(i32) + Send + Sync + 'static,
    save: impl Fn() + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <div class="slider-row">
            <input type="range" min=min max=max step=step prop:value=move || get().to_string()
                on:input=move |e| set(event_target_value(&e).parse().unwrap_or(min)) on:change=move |_| save()/>
            <span class="slider-value">{move || format!("{}{unit}", get())}</span>
        </div>
    }
}

#[component]
fn AppearanceTab() -> impl IntoView {
    let s = store();
    let st = s.settings;
    let prefs = s.prefs;
    let system = move || st.with(|x| x.theme == "system");
    view! {
        <h2>"Appearance"</h2>
        <h3>"Theme"</h3>
        {switch("Same as device theme", "Follow your computer's light or dark mode.", system,
            move |on| s.update_settings(|x| { x.theme = if on { "system" } else { "dark" }.into(); x.gradient.clear(); }))}
        <div class="tile-row">
            {BASE_THEMES.into_iter().map(|(id, label)| view! {
                <button class=format!("theme-tile base-{id}") title=label aria-label=label disabled=system
                    class:active=move || st.with(|x| x.gradient.is_empty() && x.theme == id)
                    on:click=move |_| s.update_settings(|x| { x.theme = id.into(); x.gradient.clear(); })></button>
            }).collect_view()}
        </div>
        <h3>"Colour themes"</h3>
        <div class="tile-row">
            {GRADIENTS.iter().map(|g| {
                let id = g.id;
                view! {
                    <button class=format!("theme-tile grad-{id}") title=g.label aria-label=g.label
                        class:active=move || st.with(|x| x.gradient == id)
                        on:click=move |_| s.update_settings(|x| x.gradient = if x.gradient == id { String::new() } else { id.into() })></button>
                }
            }).collect_view()}
        </div>
        <p class="muted small">"Click the chosen colour theme again to go back to your default theme."</p>
        <h3>"Accent colour"</h3>
        <div class="tile-row">
            {ACCENTS.into_iter().map(|(v, label, swatch)| view! {
                <button class="accent-dot" title=label aria-label=label style:background=swatch
                    class:active=move || st.with(|x| x.accent == v)
                    on:click=move |_| s.update_settings(|x| x.accent = v.into())></button>
            }).collect_view()}
        </div>
        {(!desktop::is_android()).then(|| view! {
            <h3>"App icon"</h3>
            <p class="muted small">{if desktop::is_desktop() { "Natter's icon on the taskbar and in the tray." } else { "Natter's icon on this browser tab." }}</p>
            <div class="tile-row">
                {APP_ICONS.into_iter().map(|(id, label)| view! {
                    <button class="app-icon-pick" title=label aria-label=label
                        class:active=move || st.with(|x| x.app_icon == id)
                        on:click=move |_| s.update_settings(|x| x.app_icon = id.into())>
                        <img src=desktop::app_icon_url(id) alt=""/>
                    </button>
                }).collect_view()}
            </div>
        })}
        <h3>"Message display"</h3>
        <div class="choice-row">
            <button class="choice" class:active=move || !st.with(|x| x.compact) on:click=move |_| s.update_settings(|x| x.compact = false)>
                <b>"Cozy"</b><span class="muted small">"Roomy, with avatars on every group."</span>
            </button>
            <button class="choice" class:active=move || st.with(|x| x.compact) on:click=move |_| s.update_settings(|x| x.compact = true)>
                <b>"Compact"</b><span class="muted small">"Fit more messages on screen at once."</span>
            </button>
        </div>
        {switch("Show link previews", "YouTube videos, pages and pictures under the messages that link to them.",
            move || st.with(|x| x.embeds), move |on| s.update_settings(|x| x.embeds = on))}
        <h3>"Space between message groups"</h3>
        {slider((0, 24, 4), "px", move || st.with(|x| i32::from(x.group_spacing)),
            move |v| st.update(|x| x.group_spacing = v as u8), move || s.save_settings())}
        <h3>"Chat font scaling"</h3>
        {slider((12, 20, 1), "px", move || st.with(|x| i32::from(x.font_size)),
            move |v| st.update(|x| x.font_size = v as u8), move || s.save_settings())}
        {desktop::is_desktop().then(|| view! {
            <h3>"Zoom level"</h3>
            {slider((50, 200, 10), "%", move || prefs.with(|p| i32::from(p.zoom)),
                move |v| { prefs.update(|p| p.zoom = v as u16); desktop::set_zoom(v as u16); }, move || s.save_prefs())}
        })}
    }
}

#[component]
fn AccessibilityTab() -> impl IntoView {
    let s = store();
    let st = s.settings;
    view! {
        <h2>"Accessibility"</h2>
        <h3>"Saturation"</h3>
        <p class="muted small">"Turn down the colours of the whole app."</p>
        {slider((0, 100, 10), "%", move || st.with(|x| i32::from(x.saturation)),
            move |v| st.update(|x| x.saturation = v as u8), move || s.save_settings())}
        <h3>"Motion"</h3>
        {switch("Reduce motion", "Turn off animations and transitions.", move || st.with(|x| x.reduce_motion),
            move |on| s.update_settings(|x| x.reduce_motion = on))}
        <h3>"Links"</h3>
        {switch("Always underline links", "Make links in messages stand out from the text around them.", move || st.with(|x| x.underline_links),
            move |on| s.update_settings(|x| x.underline_links = on))}
    }
}

// ---------------------------------------------------------------- notifications

#[component]
fn NotificationsTab() -> impl IntoView {
    let s = store();
    let st = s.settings;
    let test = move |_| {
        sound::play(sound::Sound::Message, st.with_untracked(|x| x.sounds));
        desktop::request_notification_permission();
        desktop::notify("Natter", "This is what a notification looks like.", None);
    };
    view! {
        <h2>"Notifications"</h2>
        {switch("Desktop notifications", "A pop-up when a message arrives while you're not looking at its channel.",
            move || st.with(|x| x.notifications),
            move |on| { if on { desktop::request_notification_permission(); } s.update_settings(|x| x.notifications = on) })}
        <h3>"Notify me about"</h3>
        <label class="check"><input type="radio" name="notify-level" prop:checked=move || st.with(|x| x.notify_all)
            on:change=move |_| s.update_settings(|x| x.notify_all = true)/>"All messages"</label>
        <label class="check"><input type="radio" name="notify-level" prop:checked=move || !st.with(|x| x.notify_all)
            on:change=move |_| s.update_settings(|x| x.notify_all = false)/>"Only direct messages and @mentions"</label>
        <p class="muted small">"Muted channels and spaces stay quiet either way (right-click one to mute it). Do Not Disturb silences everything."</p>
        <h3>"Badges"</h3>
        {switch("Unread badge", "A red number on Natter's icon for mentions and direct messages, a red dot for anything else unread.",
            move || st.with(|x| x.badge), move |on| s.update_settings(|x| x.badge = on))}
        {desktop::is_desktop().then(|| switch("Flash the taskbar", "When a notification arrives while Natter isn't in front.",
            move || st.with(|x| x.flash), move |on| s.update_settings(|x| x.flash = on)))}
        <h3>"Sounds"</h3>
        {switch("Sounds", "New messages, people joining and leaving calls, mute and unmute.",
            move || st.with(|x| x.sounds), move |on| s.update_settings(|x| x.sounds = on))}
        <div class="tile-row">
            {sound::MESSAGE_SOUNDS.into_iter().map(|(id, label)| view! {
                <button class="btn sound-pick" class:active=move || st.with(|x| x.message_sound == id)
                    on:click=move |_| { s.update_settings(|x| x.message_sound = id.into()); sound::preview_message(id); }>
                    <Icon name="bell" size=16/>{label}
                </button>
            }).collect_view()}
        </div>
        <div class="row gap test-row">
            <button class="btn primary" on:click=test>"Send a test notification"</button>
        </div>
    }
}

// ---------------------------------------------------------------- keybinds & system

#[component]
fn KeybindsTab() -> impl IntoView {
    let s = store();
    let prefs = s.prefs;
    let recording = RwSignal::new(None::<usize>);
    let save = move |f: Box<dyn FnOnce(&mut Vec<Keybind>)>| {
        prefs.update(|p| f(&mut p.keybinds));
        s.save_prefs();
        desktop::sync_keybinds();
    };
    let record = move |i: usize| {
        recording.set(Some(i));
        request_animation_frame(move || util::focus(&format!("keybind-{i}")));
    };
    let add = move |_| {
        save(Box::new(|k| k.push(Keybind { action: "toggle_mute".into(), keys: String::new() })));
        record(prefs.with_untracked(|p| p.keybinds.len() - 1));
    };
    view! {
        <h2>"Keybinds"</h2>
        <p class="muted">{if desktop::is_desktop() {
            "Your keybinds work everywhere, even while Natter is in the background or in the tray."
        } else {
            "In the browser, keybinds work while Natter has focus. In the desktop app they work everywhere."
        }}</p>
        <button class="btn primary" on:click=add><Icon name="plus" size=16/>" Add a keybind"</button>
        {move || prefs.with(|p| p.keybinds.clone()).into_iter().enumerate().map(|(i, b)| {
            let action = b.action.clone();
            let keys = b.keys.clone();
            view! {
                <div class="keybind-row">
                    <label class="grow">"Action"
                        <select on:change=move |e| { let v = event_target_value(&e); save(Box::new(move |k| if let Some(b) = k.get_mut(i) { b.action = v })) }>
                            {desktop::KEYBIND_ACTIONS.into_iter().filter(|(id, _)| *id != "toggle_window" || desktop::is_desktop()).map(|(id, label)| view! {
                                <option value=id selected=action == id>{label}</option>
                            }).collect_view()}
                        </select>
                    </label>
                    <label class="grow">"Keybind"
                        <button id=format!("keybind-{i}") class="btn keybind" class:recording=move || recording.get() == Some(i)
                            on:click=move |_| record(i)
                            // Also fires as the dialog closes, after the signal is gone: hence try_.
                            on:blur=move |_| if recording.try_get_untracked() == Some(Some(i)) { recording.set(None) }
                            on:keydown=move |e| {
                                if recording.get_untracked() != Some(i) { return; }
                                e.prevent_default();
                                e.stop_propagation();
                                if e.key() == "Escape" { recording.set(None); return; }
                                if let Some(c) = desktop::combo(&e) {
                                    recording.set(None);
                                    save(Box::new(move |k| if let Some(b) = k.get_mut(i) { b.keys = c }));
                                }
                            }>
                            {move || if recording.get() == Some(i) {
                                "Press the keys…".to_string()
                            } else if keys.is_empty() {
                                "Record keybind".into()
                            } else {
                                desktop::combo_label(&keys)
                            }}
                        </button>
                    </label>
                    <button class="icon-btn danger" title="Remove keybind" on:click=move |_| save(Box::new(move |k| { if i < k.len() { k.remove(i); } }))>
                        <Icon name="trash"/>
                    </button>
                </div>
            }
        }).collect_view()}
        <h3>"Built-in shortcuts"</h3>
        <p class="muted small">"Ctrl+K quick switcher, Ctrl+Shift+M mute, Ctrl+Shift+D deafen, Alt+↑/↓ channels, and more."</p>
        <button class="btn" on:click=move |_| s.modal.set(Some(Modal::Shortcuts))><Icon name="keyboard" size=16/>" See all shortcuts"</button>
    }
}

#[component]
fn SystemTab() -> impl IntoView {
    let s = store();
    let prefs = s.prefs;
    let autostart = RwSignal::new(false);
    spawn_local(async move { autostart.set(desktop::autostart_enabled().await) });
    let apply = move |on: bool| {
        let minimized = prefs.with_untracked(|p| p.start_minimized);
        spawn_local(async move {
            if desktop::set_launch(on, minimized).await {
                autostart.set(on);
            } else {
                store().toast("Couldn't change the startup setting.", true);
            }
        });
    };
    view! {
        <h2>"System"</h2>
        {switch("Open Natter when your computer starts", "", move || autostart.get(), apply)}
        <div class="sub-setting" class:disabled=move || !autostart.get()>
            {switch("Start minimized", "Start in the tray, out of your way.", move || prefs.with(|p| p.start_minimized), move |on| {
                prefs.update(|p| p.start_minimized = on);
                s.save_prefs();
                if autostart.get_untracked() { apply(true); }
            })}
        </div>
        {switch("Minimize to tray", "Closing the window keeps Natter running in the tray, so calls and notifications carry on.",
            move || prefs.with(|p| p.close_to_tray), move |on| {
                prefs.update(|p| p.close_to_tray = on);
                s.save_prefs();
                desktop::set_close_to_tray(on);
            })}
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
        voice::set_ptt_mode(prefs.with_untracked(|p| p.push_to_talk));
        desktop::sync_keybinds();
    };
    let recording = RwSignal::new(false);
    let previewing = RwSignal::new(false);
    let preview_el = NodeRef::<leptos::html::Video>::new();
    let start_preview = move || {
        previewing.set(true);
        // The <video> mounts on the next frame.
        request_animation_frame(move || {
            let Some(el) = preview_el.get_untracked() else { return };
            let id = prefs.with_untracked(|p| p.camera.clone());
            spawn_local(async move {
                match voice::camera_preview(&el, &id).await {
                    // Permission granted: real device names are available now.
                    Ok(_) => devices.set(voice::devices().await),
                    Err(_) => {
                        previewing.set(false);
                        store().toast("Couldn't open the camera. Is it in use or blocked?", true);
                    }
                }
            });
        });
    };
    let stop_preview = move || {
        voice::camera_preview_stop();
        previewing.set(false);
    };
    on_cleanup(voice::camera_preview_stop);
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
        <p class="muted small">"Device names appear once you've allowed microphone or camera access (join a call, or test the camera below)."</p>
        <h3>"Camera"</h3>
        <div class="row gap camera-row">
            <label class="grow">"Device"
                <select on:change=move |e| {
                    let v = event_target_value(&e);
                    set(Box::new(move |p| p.camera = v));
                    if previewing.get_untracked() { start_preview(); }
                }>
                    {move || opts("videoinput", prefs.with(|p| p.camera.clone()))}
                </select>
            </label>
            <button class="btn" on:click=move |_| if previewing.get_untracked() { stop_preview() } else { start_preview() }>
                {move || if previewing.get() { "Stop preview" } else { "Test camera" }}
            </button>
        </div>
        <Show when=move || previewing.get()>
            <video class="camera-preview" node_ref=preview_el autoplay=true playsinline=true prop:muted=true></video>
        </Show>
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
    let can = move |p: u64| s.can(Some(space), p);
    let tab = RwSignal::new(if can(perm::MANAGE_SPACE) { "overview" } else if can(perm::MANAGE_ROLES) { "roles" } else { "members" });
    let owner = move || s.space(space).is_some_and(|sp| sp.owner_id == s.me_id());
    view! {
        <div class="settings">
            <nav class="settings-nav">
                <div class="settings-group ellipsis">{move || s.space(space).map(|sp| sp.name.to_uppercase()).unwrap_or_default()}</div>
                {[("overview", "Overview", perm::MANAGE_SPACE), ("roles", "Roles", perm::MANAGE_ROLES), ("members", "Members", 0),
                  ("invites", "Invites", 0), ("bans", "Bans", perm::BAN_MEMBERS)].into_iter().map(|(id, label, need)| view! {
                    <Show when=move || can(need)>
                        <button class:active=move || tab.get() == id on:click=move |_| tab.set(id)>{label}</button>
                    </Show>
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
                    "roles" => view! { <SpaceRoles space=space/> }.into_any(),
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
        v.sort_by_key(|m| (std::cmp::Reverse(s.rank(space, m.user_id)), s.name(m.user_id, Some(space)).to_lowercase()));
        v
    };
    view! {
        <h2>{move || format!("Members — {}", s.space(space).map(|sp| sp.members.len()).unwrap_or(0))}</h2>
        <input class="wide-input" placeholder="Search members" prop:value=move || q.get() on:input=move |e| q.set(event_target_value(&e))/>
        <div class="member-table">
            <For each=members key=|m| (m.user_id, m.nickname.clone()) let:m>
                {
                    let u = m.user_id;
                    view! {
                        <div class="member-line" on:contextmenu=move |e| s.open_menu(&e, super::menu::user_menu(u, Some(space)))>
                            <Avatar user=u size=32 status=true/>
                            <div class="grow">
                                <div>{move || s.name(u, Some(space))}</div>
                                <div class="muted small">{move || s.user(u).map(|x| x.username).unwrap_or_default()}{format!(" · joined {}", util::day_label(m.joined_at))}</div>
                            </div>
                            {move || (s.space(space).map(|sp| sp.owner_id) == Some(u)).then(|| view! { <span class="role-chip owner-chip">"👑 Owner"</span> })}
                            <super::members::RoleChips space=space user=u/>
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

// ---------------------------------------------------------------- roles

/// Swatches for role colours.
const ROLE_COLORS: [&str; 20] = [
    "#1abc9c", "#2ecc71", "#3498db", "#9b59b6", "#e91e63", "#f1c40f", "#e67e22", "#e74c3c", "#95a5a6", "#607d8b",
    "#11806a", "#1f8b4c", "#206694", "#71368a", "#ad1457", "#c27c0e", "#a84300", "#992d22", "#979c9f", "#546e7a",
];

/// Quick picks for a role's icon (any emoji can be typed).
const ROLE_EMOJI: [&str; 14] = ["🛡️", "👑", "⭐", "🔧", "🛠️", "🎨", "💡", "🎮", "🎬", "📅", "🧪", "🎵", "📣", "🤝"];

/// The Roles tab: @everyone, then the roles highest first, drag to reorder; a role opens its editor.
#[component]
fn SpaceRoles(space: Id) -> impl IntoView {
    let s = store();
    // None: the list; Some(None): @everyone; Some(Some(id)): that role.
    let editing = RwSignal::new(None::<Option<Id>>);
    let q = RwSignal::new(String::new());
    let dragging = RwSignal::new(None::<Id>);
    let roles = move || s.space(space).map(|sp| sp.roles).unwrap_or_default();
    let members_with = move |role: Id| s.space(space).map_or(0, |sp| sp.members.iter().filter(|m| m.roles.contains(&role)).count());
    let my_rank = move || s.rank(space, s.me_id());
    let can_manage = move || s.can(Some(space), perm::MANAGE_ROLES);
    let create = move |_| {
        spawn_local(async move {
            let body = CreateRole { name: "new role".into(), ..Default::default() };
            match api::call::<_, RoleDef>("POST", &format!("/api/spaces/{space}/roles"), &body).await {
                Ok(r) => editing.set(Some(Some(r.id))),
                Err(e) => store().error(e),
            }
        });
    };
    // Dropping a role on another takes that one's place.
    let drop_on = move |target: Id| {
        let Some(src) = dragging.get_untracked() else { return };
        dragging.set(None);
        let mut ids: Vec<Id> = roles().iter().map(|r| r.id).collect();
        let (Some(from), Some(to)) = (ids.iter().position(|&x| x == src), ids.iter().position(|&x| x == target)) else { return };
        if from == to {
            return;
        }
        let id = ids.remove(from);
        ids.insert(to, id);
        spawn_local(async move {
            if let Err(e) = api::call::<_, ()>("PUT", &format!("/api/spaces/{space}/roles"), &ReorderRoles { ids }).await {
                store().error(e);
            }
        });
    };
    let delete = move |r: RoleDef| {
        let id = r.id;
        s.confirm(&format!("Delete '{}'", r.name), "Everyone who has it loses it. This can't be undone.", "Delete role", move || {
            spawn_local(async move {
                if let Err(e) = api::act("DELETE", &format!("/api/spaces/{space}/roles/{id}")).await {
                    store().error(e);
                }
            });
        });
    };
    view! {
        {move || match editing.get() {
            Some(role) => view! { <RoleEditor space=space role=role editing=editing/> }.into_any(),
            None => view! {
                <h2>"Roles"</h2>
                <p class="muted">"Use roles to group your space's members and decide what they may do."</p>
                <button class="default-perms" on:click=move |_| editing.set(Some(None))>
                    <Icon name="members"/>
                    <div class="grow">
                        <div class="switch-label">"Default permissions"</div>
                        <div class="muted small">"@everyone · applies to all members"</div>
                    </div>
                    <span class="chev-right"><Icon name="chevron" size=18/></span>
                </button>
                <div class="row gap roles-tools">
                    <input class="wide-input grow" placeholder="Search roles" prop:value=move || q.get() on:input=move |e| q.set(event_target_value(&e))/>
                    <Show when=can_manage><button class="btn primary" on:click=create>"Create role"</button></Show>
                </div>
                <p class="muted small">"Members use the colour of the highest role they have on this list. Drag roles to reorder them."</p>
                <div class="roles-table">
                    <div class="roles-head">
                        <span class="grow">{move || format!("Roles — {}", roles().len())}</span>
                        <span class="roles-count">"Members"</span>
                        <span class="roles-actions"></span>
                    </div>
                    {move || {
                        let query = q.get().to_lowercase();
                        roles().into_iter().filter(|r| r.name.to_lowercase().contains(&query)).map(|r| {
                            let id = r.id;
                            let locked = !can_manage() || r.position >= my_rank();
                            let r2 = r.clone();
                            view! {
                                <div class="role-row" class:dragging=move || dragging.get() == Some(id) draggable=if locked { "false" } else { "true" }
                                    on:dragstart=move |_| dragging.set(Some(id))
                                    on:dragend=move |_| dragging.set(None)
                                    on:dragover=move |e| if dragging.get_untracked().is_some() && !locked { e.prevent_default() }
                                    on:drop=move |e| { e.prevent_default(); drop_on(id) }
                                    on:click=move |_| editing.set(Some(Some(id)))>
                                    <span class="role-grip" class:hidden=locked><Icon name="grip" size=16/></span>
                                    <span class="role-shield" style:color=super::members::role_swatch(&r.color)>
                                        {if r.emoji.is_empty() { view! { <Icon name="shield" size=18/> }.into_any() } else { view! { <span>{r.emoji.clone()}</span> }.into_any() }}
                                    </span>
                                    <span class="grow ellipsis role-name">{r.name.clone()}
                                        {locked.then(|| view! { <span class="role-lock" title="Above your highest role"><Icon name="lock" size=14/></span> })}
                                    </span>
                                    <span class="roles-count">{members_with(id)}" "<Icon name="members" size=14/></span>
                                    <span class="roles-actions">
                                        <button class="icon-btn" title=if locked { "View" } else { "Edit" } on:click=move |e| { e.stop_propagation(); editing.set(Some(Some(id))) }>
                                            <Icon name=if locked { "search" } else { "edit" } size=16/>
                                        </button>
                                        {(!locked).then(|| {
                                            let r3 = r2.clone();
                                            view! {
                                                <button class="icon-btn danger" title="Delete role" on:click=move |e| { e.stop_propagation(); delete(r3.clone()) }>
                                                    <Icon name="trash" size=16/>
                                                </button>
                                            }
                                        })}
                                    </span>
                                </div>
                            }
                        }).collect_view()
                    }}
                </div>
            }.into_any(),
        }}
    }
}

/// Edit one role (display, permissions, members) or the @everyone permissions. Changes wait for Save.
#[component]
fn RoleEditor(space: Id, role: Option<Id>, editing: RwSignal<Option<Option<Id>>>) -> impl IntoView {
    let s = store();
    let current = move || role.and_then(|id| s.space(space).and_then(|sp| sp.roles.into_iter().find(|r| r.id == id)));
    let everyone = move || s.space(space).map_or(0, |sp| sp.everyone_permissions);
    let tab = RwSignal::new(if role.is_some() { "display" } else { "permissions" });
    let name = RwSignal::new(String::new());
    let color = RwSignal::new(String::new());
    let emoji = RwSignal::new(String::new());
    let hoist = RwSignal::new(false);
    let perms = RwSignal::new(0u64);
    let reset = move || {
        match current() {
            Some(r) => {
                name.set(r.name);
                color.set(r.color);
                emoji.set(r.emoji);
                hoist.set(r.hoist);
                perms.set(r.permissions);
            }
            None => perms.set(everyone()),
        }
    };
    reset();
    // The role was deleted (by someone else): back to the list.
    Effect::new(move |_| {
        if role.is_some() && current().is_none() {
            editing.set(None);
        }
    });
    let locked = move || !s.can(Some(space), perm::MANAGE_ROLES) || current().is_some_and(|r| r.position >= s.rank(space, s.me_id()));
    let mine = move || s.perms(Some(space));
    let dirty = move || match current() {
        Some(r) => r.name != name.get() || r.color != color.get() || r.emoji != emoji.get() || r.hoist != hoist.get() || r.permissions != perms.get(),
        None => role.is_none() && everyone() != perms.get(),
    };
    let saving = RwSignal::new(false);
    let save = move |_| {
        saving.set(true);
        spawn_local(async move {
            let r = match role {
                Some(id) => {
                    let body = UpdateRole {
                        name: Some(name.get_untracked()),
                        color: Some(color.get_untracked()),
                        emoji: Some(emoji.get_untracked()),
                        permissions: Some(perms.get_untracked()),
                        hoist: Some(hoist.get_untracked()),
                    };
                    api::call::<_, ()>("PATCH", &format!("/api/spaces/{space}/roles/{id}"), &body).await
                }
                None => {
                    let body = UpdateSpace { everyone_permissions: Some(perms.get_untracked()), ..Default::default() };
                    api::call::<_, ()>("PATCH", &format!("/api/spaces/{space}"), &body).await
                }
            };
            saving.set(false);
            match r {
                Ok(()) => store().toast("Saved.", false),
                Err(e) => store().error(e),
            }
        });
    };
    let holders = move || {
        let Some(id) = role else { return vec![] };
        s.space(space).map(|sp| sp.members.into_iter().filter(|m| m.roles.contains(&id)).map(|m| m.user_id).collect()).unwrap_or_default()
    };
    let others = move || {
        let Some(id) = role else { return vec![] };
        let mut v: Vec<Id> = s.space(space).map(|sp| sp.members.into_iter().filter(|m| !m.roles.contains(&id)).map(|m| m.user_id).collect()).unwrap_or_default();
        v.sort_by_key(|&u| s.name(u, Some(space)).to_lowercase());
        v
    };
    let set_member = move |user: Id, add: bool| {
        let Some(id) = role else { return };
        spawn_local(async move {
            let path = format!("/api/spaces/{space}/members/{user}/roles/{id}");
            if let Err(e) = api::act(if add { "PUT" } else { "DELETE" }, &path).await {
                store().error(e);
            }
        });
    };
    let tab_btn = move |id: &'static str, label: String| view! {
        <button class:active=move || tab.get() == id on:click=move |_| tab.set(id)>{label}</button>
    };
    view! {
        <div class="role-editor">
            <button class="btn link back-link" on:click=move |_| editing.set(None)><Icon name="arrow-left" size=16/>" Roles"</button>
            <h2>{move || match role {
                None => "@everyone".to_string(),
                Some(_) => format!("Edit role — {}", name.get()),
            }}</h2>
            {move || locked().then(|| view! { <p class="muted small"><Icon name="lock" size=14/>" You can look, but this role sits at or above your own highest role."</p> })}
            <div class="tabs editor-tabs">
                {role.is_some().then(|| tab_btn("display", "Display".into()))}
                {tab_btn("permissions", "Permissions".into())}
                {role.is_some().then(|| view! { <button class:active=move || tab.get() == "members" on:click=move |_| tab.set("members")>{move || format!("Manage members ({})", holders().len())}</button> })}
            </div>
            <fieldset class="plain" disabled=locked>
            {move || match tab.get() {
                "display" => view! {
                    <label>"Role name"<input class="wide-input" maxlength="100" prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e))/></label>
                    <h3>"Role colour"</h3>
                    <p class="muted small">"Members use the colour of their highest role."</p>
                    <div class="tile-row">
                        <button class="accent-dot no-colour" title="No colour" class:active=move || color.get().is_empty() on:click=move |_| color.set(String::new())></button>
                        {ROLE_COLORS.into_iter().map(|c| view! {
                            <button class="accent-dot" title=c style:background=c class:active=move || color.get() == c on:click=move |_| color.set(c.into())></button>
                        }).collect_view()}
                        <input type="color" class="colour-input" title="Any colour" prop:value=move || super::members::role_swatch(&color.get())
                            on:input=move |e| color.set(event_target_value(&e))/>
                    </div>
                    <h3>"Role icon"</h3>
                    <div class="tile-row">
                        <input class="wide-input emoji-input" maxlength="8" placeholder="none" prop:value=move || emoji.get() on:input=move |e| emoji.set(event_target_value(&e))/>
                        {ROLE_EMOJI.into_iter().map(|e| view! {
                            <button class="emoji-pick" class:active=move || emoji.get() == e on:click=move |_| emoji.set(if emoji.get_untracked() == e { String::new() } else { e.into() })>{e}</button>
                        }).collect_view()}
                    </div>
                    {switch("Display members separately", "Show members with this role in their own group in the member list.", move || hoist.get(), move |on| hoist.set(on))}
                }.into_any(),
                "members" => view! {
                    <div class="row gap">
                        <select class="grow" on:change=move |e| {
                            if let Ok(u) = event_target_value(&e).parse::<Id>() { set_member(u, true); }
                            reset_picker(&e);
                        }>
                            <option value="" selected=true>"Add a member…"</option>
                            {move || others().into_iter().map(|u| view! { <option value=u.to_string()>{s.name(u, Some(space))}</option> }).collect_view()}
                        </select>
                    </div>
                    {move || holders().into_iter().map(|u| view! {
                        <div class="member-line">
                            <Avatar user=u size=32/>
                            <div class="grow">{move || s.name(u, Some(space))}</div>
                            <button class="icon-btn" title="Remove from role" on:click=move |_| set_member(u, false)><Icon name="x" size=16/></button>
                        </div>
                    }).collect_view()}
                    {move || holders().is_empty().then(|| view! { <p class="muted">"Nobody has this role yet."</p> })}
                }.into_any(),
                _ => {
                    let groups = ["General", "Membership", "Text", "Voice", "Advanced"];
                    view! {
                        {role.is_none().then(|| view! { <p class="muted small">"What every member may do. Roles add to this; they never take anything away."</p> })}
                        {groups.into_iter().map(|g| view! {
                            <h3>{g}</h3>
                            {perm::LIST.into_iter().filter(|(.., group)| *group == g).filter(|(f, ..)| role.is_some() || *f != perm::ADMINISTRATOR).map(|(f, label, hint, _)| view! {
                                <label class="switch-row" class:off-limits=move || f & !mine() != 0>
                                    <div class="grow">
                                        <div class="switch-label">{label}</div>
                                        <div class="muted small">{hint}</div>
                                    </div>
                                    <input type="checkbox" class="switch" prop:checked=move || perms.get() & f != 0
                                        disabled=move || f & !mine() != 0
                                        on:change=move |e| { let on = event_target_checked(&e); perms.update(|p| if on { *p |= f } else { *p &= !f }); }/>
                                </label>
                            }).collect_view()}
                        }).collect_view()}
                    }.into_any()
                }
            }}
            </fieldset>
            <Show when=move || dirty() && tab.get() != "members">
                <div class="save-bar">
                    <span class="grow">"Careful — you have unsaved changes!"</span>
                    <button class="btn link" on:click=move |_| reset()>"Reset"</button>
                    <button class="btn primary" disabled=move || saving.get() on:click=save>"Save changes"</button>
                </div>
            </Show>
        </div>
    }
}

// ---------------------------------------------------------------- channel permissions

/// A channel's (or category's) permissions: make it private and pick who sees it, or set any channel permission
/// to allow / inherit / deny for @everyone, a role or a member. Changes apply at once.
#[component]
pub fn ChannelPermissions(channel: Id) -> impl IntoView {
    let s = store();
    let Some(space) = s.channel(channel).and_then(|c| c.space_id) else {
        return view! { <p class="muted">"Direct messages have no permissions to set."</p> }.into_any();
    };
    let category = s.channel(channel).is_some_and(|c| c.kind == ChannelKind::Category);
    let sp = move || s.space(space);
    let ch = move || s.channel(channel);
    let selected = RwSignal::new(space);
    let extra = RwSignal::new(Vec::<Id>::new());
    let mine = move || s.chan_perms(channel);
    let ow = move |target: Id| ch().and_then(|c| c.overwrites.into_iter().find(|o| o.id == target));
    let kind_of = move |target: Id| {
        if target == space || sp().is_some_and(|x| x.roles.iter().any(|r| r.id == target)) { OverwriteKind::Role } else { OverwriteKind::Member }
    };
    let label = move |target: Id| -> String {
        if target == space {
            "@everyone".into()
        } else if let Some(r) = sp().and_then(|x| x.roles.into_iter().find(|r| r.id == target)) {
            r.name
        } else {
            s.name(target, Some(space))
        }
    };
    // Sent one after another: making it private keeps your own access first, or you'd be locked out mid-way.
    let send = move |changes: Vec<(Id, u64, u64)>| {
        spawn_local(async move {
            for (target, allow, deny) in changes {
                let body = SetOverwrite { kind: Some(kind_of(target)), allow, deny };
                if let Err(e) = api::call::<_, ()>("PUT", &format!("/api/channels/{channel}/permissions/{target}"), &body).await {
                    store().error(e);
                    return;
                }
            }
        });
    };
    let bits = move |target: Id| ow(target).map_or((0, 0), |o| (o.allow, o.deny));
    let private = move || s.is_private(channel);
    let set_private = move |on: bool| {
        let (a, d) = bits(space);
        let mut changes = vec![];
        if on && !s.can(Some(space), perm::ADMINISTRATOR) {
            let me = s.me_id();
            let (ma, md) = bits(me);
            changes.push((me, ma | perm::VIEW_CHANNEL, md & !perm::VIEW_CHANNEL));
        }
        changes.push((space, a & !perm::VIEW_CHANNEL, if on { d | perm::VIEW_CHANNEL } else { d & !perm::VIEW_CHANNEL }));
        send(changes);
    };
    let let_in = move |target: Id, on: bool| {
        let (a, d) = bits(target);
        send(vec![(target, if on { a | perm::VIEW_CHANNEL } else { a & !perm::VIEW_CHANNEL }, d & !perm::VIEW_CHANNEL)]);
    };
    // Targets with an override, @everyone first, roles by rank, then members.
    let targets = move || {
        let Some(x) = sp() else { return vec![] };
        let set: Vec<Id> = ch().map(|c| c.overwrites.iter().map(|o| o.id).collect()).unwrap_or_default();
        let mut out = vec![space];
        out.extend(x.roles.iter().map(|r| r.id).filter(|r| set.contains(r) || extra.with(|e| e.contains(r))));
        out.extend(x.members.iter().map(|m| m.user_id).filter(|u| set.contains(u) || extra.with(|e| e.contains(u))));
        out
    };
    let addable = move |exclude: Vec<Id>| -> Vec<(Id, String)> {
        let Some(x) = sp() else { return vec![] };
        let mut v: Vec<(Id, String)> = x.roles.iter().filter(|r| !exclude.contains(&r.id)).map(|r| (r.id, format!("Role: {}", r.name))).collect();
        let mut people: Vec<(Id, String)> = x.members.iter().map(|m| m.user_id).filter(|u| !exclude.contains(u)).map(|u| (u, s.name(u, Some(space)))).collect();
        people.sort_by_key(|(_, n)| n.to_lowercase());
        v.extend(people);
        v
    };
    let allowed_in = move || targets().into_iter().filter(|&t| t != space && bits(t).0 & perm::VIEW_CHANNEL != 0).collect::<Vec<_>>();
    view! {
        <div class="settings">
            <nav class="settings-nav">
                <div class="settings-group ellipsis">{move || ch().map(|c| if category { c.name.to_uppercase() } else { format!("#{}", c.name).to_uppercase() }).unwrap_or_default()}</div>
                <div class="settings-group">"ROLES / MEMBERS"</div>
                {move || targets().into_iter().map(|t| view! {
                    <button class:active=move || selected.get() == t on:click=move |_| selected.set(t)>
                        {if kind_of(t) == OverwriteKind::Role { view! { <Icon name="shield" size=14/> }.into_any() } else { view! { <Icon name="members" size=14/> }.into_any() }}
                        " "{label(t)}
                    </button>
                }).collect_view()}
                <select class="add-target" on:change=move |e| {
                    if let Ok(t) = event_target_value(&e).parse::<Id>() {
                        extra.update(|x| x.push(t));
                        selected.set(t);
                    }
                    reset_picker(&e);
                }>
                    <option value="" selected=true>"+ Add a role or member"</option>
                    {move || addable(targets()).into_iter().map(|(id, n)| view! { <option value=id.to_string()>{n}</option> }).collect_view()}
                </select>
            </nav>
            <section class="settings-body scroll">
                <button class="settings-close icon-btn" title="Close (Esc)" on:click=|_| store().modal.set(None)><Icon name="x"/></button>
                <h2>"Permissions"</h2>
                <p class="muted">{if category {
                    "Channels in this category get these first, then their own."
                } else {
                    "These apply on top of what each role may do in the space."
                }}</p>
                {switch(if category { "Private category" } else { "Private channel" },
                    "Only the roles and members you let in can see it. Admins and the owner always can.",
                    private, set_private)}
                <Show when=private>
                    <h3>"Who can see it"</h3>
                    <div class="role-chips">
                        {move || allowed_in().into_iter().map(|t| view! {
                            <span class="role-chip">
                                {if kind_of(t) == OverwriteKind::Role { view! { <Icon name="shield" size=12/> }.into_any() } else { view! { <Icon name="members" size=12/> }.into_any() }}
                                <span>{label(t)}</span>
                                <button class="chip-x" title="Take away" on:click=move |_| let_in(t, false)>"×"</button>
                            </span>
                        }).collect_view()}
                        {move || allowed_in().is_empty().then(|| view! { <span class="muted small">"Nobody yet, apart from admins and the owner."</span> })}
                    </div>
                    <select class="wide-input add-viewer" on:change=move |e| {
                        if let Ok(t) = event_target_value(&e).parse::<Id>() { let_in(t, true); }
                        reset_picker(&e);
                    }>
                        <option value="" selected=true>"Let in a role or member…"</option>
                        {move || addable(allowed_in()).into_iter().map(|(id, n)| view! { <option value=id.to_string()>{n}</option> }).collect_view()}
                    </select>
                </Show>
                <h3>{move || format!("Advanced permissions — {}", label(selected.get()))}</h3>
                <p class="muted small">"✓ allows, ✗ denies, / leaves it to the roles."</p>
                {perm::LIST.into_iter().filter(|(f, ..)| f & perm::CHANNEL != 0).map(|(f, name, hint, _)| {
                    let state = move || {
                        let (a, d) = bits(selected.get());
                        if a & f != 0 { 1 } else if d & f != 0 { -1 } else { 0 }
                    };
                    let pick = move |v: i8| {
                        let t = selected.get_untracked();
                        let (a, d) = bits(t);
                        let (a, d) = (a & !f, d & !f);
                        send(vec![(t, if v == 1 { a | f } else { a }, if v == -1 { d | f } else { d })]);
                    };
                    let off_limits = move || f & !mine() != 0;
                    view! {
                        <div class="perm-row" class:off-limits=off_limits>
                            <div class="grow">
                                <div class="switch-label">{name}</div>
                                <div class="muted small">{hint}</div>
                            </div>
                            <div class="tri">
                                <button class="deny" title="Deny" class:active=move || state() == -1 disabled=off_limits on:click=move |_| pick(-1)><Icon name="x" size=16/></button>
                                <button class="inherit" title="Inherit" class:active=move || state() == 0 disabled=off_limits on:click=move |_| pick(0)>"/"</button>
                                <button class="allow" title="Allow" class:active=move || state() == 1 disabled=off_limits on:click=move |_| pick(1)><Icon name="check" size=16/></button>
                            </div>
                        </div>
                    }
                }).collect_view()}
            </section>
        </div>
    }
    .into_any()
}

/// "Add …" pickers go back to their prompt after a choice (the list under them changes).
fn reset_picker(e: &leptos::ev::Event) {
    event_target::<web_sys::HtmlSelectElement>(e).set_value("");
}
