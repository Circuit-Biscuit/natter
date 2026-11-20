//! Sign-in: pick a server, then Nextcloud, an existing Natter account, or a new one.

use leptos::prelude::*;
use leptos::task::spawn_local;
use natter_proto::*;

use super::icons::{Icon, Logo};
use crate::{api, desktop, gateway, util};

fn device_name() -> String {
    let ua = util::window().navigator().user_agent().unwrap_or_default();
    let os = if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Mac") {
        "macOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        "Unknown OS"
    };
    let app = if desktop::is_tauri() { "Natter Desktop" } else { "Natter Web" };
    format!("{app} on {os}")
}

fn finish(auth: AuthResp, on_done: Callback<()>) {
    util::storage_set("natter.token", &auth.token);
    util::storage_set("natter.server", &api::server());
    api::set_token(Some(auth.token));
    gateway::connect();
    desktop::request_notification_permission();
    on_done.run(());
}

/// "https://chat.example.com/invite/AbC123" or just "AbC123".
fn invite_code(s: &str) -> String {
    s.trim().trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string()
}

#[component]
pub fn Login(on_done: Callback<()>) -> impl IntoView {
    let in_app = desktop::is_tauri();
    let server = RwSignal::new(util::storage_get("natter.server").unwrap_or_default());
    let editing_server = RwSignal::new(in_app && server.get_untracked().is_empty());
    let info = RwSignal::new(None::<ServerInfo>);
    let info_error = RwSignal::new(None::<String>);
    let mode_register = RwSignal::new(false);
    let username = RwSignal::new(String::new());
    let display = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let invite = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let nc_waiting = RwSignal::new(None::<NcLoginStart>);
    let show_local = RwSignal::new(false);

    // Accept an invite link from the address bar (web: /invite/CODE).
    let path = util::window().location().pathname().unwrap_or_default();
    if let Some(code) = path.strip_prefix("/invite/") {
        invite.set(code.to_string());
        mode_register.set(true);
    }

    let load_info = move || {
        let base = server.get_untracked();
        api::set_server(&base);
        info.set(None);
        info_error.set(None);
        spawn_local(async move {
            match api::get::<ServerInfo>("/api/info").await {
                Ok(i) => {
                    if i.first_run {
                        mode_register.set(true);
                    }
                    info.set(Some(i));
                    editing_server.set(false);
                }
                Err(e) => {
                    info_error.set(Some(if e.status == 0 { "No Natter server answered at that address.".into() } else { e.message }));
                    editing_server.set(true);
                }
            }
        });
    };
    if !editing_server.get_untracked() {
        load_info();
    }

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if busy.get_untracked() {
            return;
        }
        error.set(None);
        busy.set(true);
        let register = mode_register.get_untracked();
        spawn_local(async move {
            let r: Result<AuthResp, api::Error> = if register {
                let body = RegisterReq {
                    username: username.get_untracked().trim().to_lowercase(),
                    password: password.get_untracked(),
                    display_name: Some(display.get_untracked()).filter(|d| !d.trim().is_empty()),
                    invite: Some(invite_code(&invite.get_untracked())).filter(|c| !c.is_empty()),
                    device: Some(device_name()),
                };
                api::call("POST", "/api/auth/register", &body).await
            } else {
                let body = LoginReq {
                    username: username.get_untracked().trim().to_lowercase(),
                    password: password.get_untracked(),
                    device: Some(device_name()),
                };
                api::call("POST", "/api/auth/login", &body).await
            };
            busy.set(false);
            match r {
                Ok(auth) => {
                    finish(auth, on_done);
                    // Joining through an invite link drops you into that space.
                    let code = invite_code(&invite.get_untracked());
                    if register && !code.is_empty() {
                        let _ = util::window().history().map(|h| h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/")));
                    }
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };

    let nextcloud = move |_| {
        error.set(None);
        busy.set(true);
        spawn_local(async move {
            match api::call::<_, NcLoginStart>("POST", "/api/auth/nextcloud", &serde_json::json!({})).await {
                Ok(start) => {
                    desktop::open_url(&start.login_url);
                    let flow = start.flow.clone();
                    nc_waiting.set(Some(start));
                    busy.set(false);
                    // Poll until approved, cancelled, or the 20-minute flow expires.
                    for _ in 0..600 {
                        gloo_timers::future::TimeoutFuture::new(2000).await;
                        if nc_waiting.get_untracked().is_none_or(|w| w.flow != flow) {
                            return;
                        }
                        let body = NcLoginPoll { flow: flow.clone(), device: Some(device_name()) };
                        match api::call::<_, Option<AuthResp>>("POST", "/api/auth/nextcloud/poll", &body).await {
                            Ok(Some(auth)) => {
                                nc_waiting.set(None);
                                finish(auth, on_done);
                                return;
                            }
                            Ok(None) => {}
                            Err(e) if e.status != 0 => {
                                nc_waiting.set(None);
                                error.set(Some(e.message));
                                return;
                            }
                            Err(_) => {}
                        }
                    }
                    nc_waiting.set(None);
                    error.set(Some("The Nextcloud sign-in expired. Please try again.".into()));
                }
                Err(e) => {
                    busy.set(false);
                    error.set(Some(e.message));
                }
            }
        });
    };

    view! {
        <div class="login">
            <div class="login-card">
                <div class="login-brand"><Logo size=56/><span>"Natter"</span></div>
                <Show when=move || editing_server.get() fallback=move || view! {
                    <div class="login-server">
                        <div>
                            <div class="login-title">
                                {move || info.get().map(|i| if mode_register.get() && i.first_run {
                                    format!("Set up {}", i.name)
                                } else if mode_register.get() {
                                    format!("Join {}", i.name)
                                } else {
                                    format!("Welcome back to {}", i.name)
                                })}
                            </div>
                            <div class="login-sub">
                                {move || {
                                    let s = server.get();
                                    if s.is_empty() { util::window().location().host().unwrap_or_default() } else { s.replace("https://", "").replace("http://", "") }
                                }}
                                " · "
                                <a href="#" on:click=move |e| { e.prevent_default(); editing_server.set(true); }>"Change server"</a>
                            </div>
                        </div>
                    </div>
                }>
                    <form class="form" on:submit=move |e| { e.prevent_default(); load_info(); }>
                        <div class="login-title">"Connect to your Natter server"</div>
                        <label>"Server address"
                            <input type="url" placeholder="https://chat.example.com" required=true autofocus=true
                                prop:value=move || server.get()
                                on:input=move |e| server.set(event_target_value(&e))/>
                        </label>
                        {move || info_error.get().map(|e| view! { <div class="form-error">{e}</div> })}
                        <button class="btn primary wide" type="submit">"Continue"</button>
                    </form>
                </Show>

                <Show when=move || info.get().is_some() && !editing_server.get()>
                    {move || nc_waiting.get().map(|w| {
                        let url = w.login_url.clone();
                        view! {
                            <div class="nc-wait">
                                <div class="spinner"></div>
                                <div>"Finish signing in to Nextcloud in your browser. This window continues by itself."</div>
                                <div class="row">
                                    <button class="btn" on:click=move |_| desktop::open_url(&url)>"Open the page again"</button>
                                    <button class="btn link" on:click=move |_| nc_waiting.set(None)>"Cancel"</button>
                                </div>
                            </div>
                        }
                    })}
                    <Show when=move || nc_waiting.get().is_none()>
                        {move || info.get().and_then(|i| i.nextcloud).map(|nc| {
                            let host = nc.replace("https://", "").replace("http://", "");
                            view! {
                                <button class="btn nextcloud wide" disabled=move || busy.get() on:click=nextcloud>
                                    <Icon name="cloud"/>"Sign in with Nextcloud"
                                </button>
                                <div class="login-hint">{format!("Uses your account at {host}. Your files stay in your Nextcloud.")}</div>
                                <button class="btn link" on:click=move |_| show_local.update(|v| *v = !*v)>
                                    {move || if show_local.get() { "Hide Natter account sign-in" } else { "Use a Natter account instead" }}
                                </button>
                            }
                        })}
                        <Show when=move || show_local.get() || info.get().is_some_and(|i| i.nextcloud.is_none())>
                            <form class="form" on:submit=submit>
                                <div class="tabs">
                                    <button type="button" class:active=move || !mode_register.get() on:click=move |_| mode_register.set(false)>"Sign in"</button>
                                    <button type="button" class:active=move || mode_register.get()
                                        disabled=move || info.get().is_some_and(|i| i.registration == Registration::Closed && !i.first_run)
                                        on:click=move |_| mode_register.set(true)>"Create account"</button>
                                </div>
                                <label>"Username"
                                    <input autocomplete="username" required=true minlength="2" maxlength="32"
                                        prop:value=move || username.get() on:input=move |e| username.set(event_target_value(&e))/>
                                </label>
                                <Show when=move || mode_register.get()>
                                    <label><span>"Display name " <span class="muted">"(optional)"</span></span>
                                        <input maxlength="100" prop:value=move || display.get() on:input=move |e| display.set(event_target_value(&e))/>
                                    </label>
                                </Show>
                                <label>"Password"
                                    <input type="password" required=true minlength=move || if mode_register.get() { "8" } else { "1" }
                                        autocomplete=move || if mode_register.get() { "new-password" } else { "current-password" }
                                        prop:value=move || password.get() on:input=move |e| password.set(event_target_value(&e))/>
                                </label>
                                <Show when=move || mode_register.get() && info.get().is_some_and(|i| i.registration == Registration::Invite && !i.first_run)>
                                    <label>"Invite link or code"
                                        <input required=true placeholder="https://…/invite/AbC123" prop:value=move || invite.get()
                                            on:input=move |e| invite.set(event_target_value(&e))/>
                                    </label>
                                </Show>
                                <Show when=move || mode_register.get() && info.get().is_some_and(|i| i.first_run)>
                                    <div class="login-hint">"This is a new server: the first account becomes its administrator."</div>
                                </Show>
                                {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}
                                <button class="btn primary wide" type="submit" disabled=move || busy.get()>
                                    {move || if busy.get() { "One moment…" } else if mode_register.get() { "Create account" } else { "Sign in" }}
                                </button>
                            </form>
                        </Show>
                        {move || (!show_local.get() && info.get().is_some_and(|i| i.nextcloud.is_some())).then(|| error.get().map(|e| view! { <div class="form-error">{e}</div> }))}
                    </Show>
                </Show>
            </div>
        </div>
    }
}
