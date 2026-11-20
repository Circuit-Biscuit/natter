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
