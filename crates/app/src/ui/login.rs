//! Sign-in: pick a server, then Nextcloud, an existing Laklak account, or a new one.

use leptos::prelude::*;
use leptos::task::spawn_local;
use laklak_proto::*;

use super::icons::{Icon, Wordmark};
use crate::{api, desktop, gateway, util};

fn device_name() -> String {
    let ua = util::window().navigator().user_agent().unwrap_or_default();
    let os = if ua.contains("Android") {
        "Android"
    } else if ua.contains("iPhone") || ua.contains("iPad") {
        "iOS"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Mac") {
        "macOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        "Unknown OS"
    };
    let app = match (desktop::is_tauri(), ua.contains("Android")) {
        (true, true) => "Laklak for Android",
        (true, false) => "Laklak Desktop",
        _ => "Laklak Web",
    };
    format!("{app} on {os}")
}

/// Signed in: remember it, accept a pending invite (Nextcloud sign-ins skip the sign-up form), go online.
fn finish(auth: AuthResp, on_done: Callback<()>, invite: String) {
    util::storage_set("laklak.token", &auth.token);
    util::storage_set("laklak.server", &api::server());
    api::set_token(Some(auth.token));
    desktop::request_notification_permission();
    spawn_local(async move {
        if !invite.is_empty() {
            if let Ok(sp) = api::call::<_, Space>("POST", &format!("/api/invites/{}", api::enc(&invite)), &serde_json::json!({})).await {
                util::storage_set("laklak.space", &sp.id.to_string());
            }
            let base = format!("{}/", api::default_server());
            let _ = util::window().history().map(|h| h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&base)));
        }
        gateway::connect();
        on_done.run(());
    });
}

/// "https://chat.example.com/invite/AbC123" or just "AbC123".
fn invite_code(s: &str) -> String {
    s.trim().trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string()
}

#[component]
pub fn Login(on_done: Callback<()>) -> impl IntoView {
    let in_app = desktop::is_tauri();
    let server = RwSignal::new(util::storage_get("laklak.server").unwrap_or_else(api::default_server));
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
    let show_pw = RwSignal::new(false);
    let email = RwSignal::new(String::new());
    // Forgot password: 0 = not now, 1 = ask for a code, 2 = enter the code and a new password.
    let forgot = RwSignal::new(0u8);
    let code = RwSignal::new(String::new());
    let new_pw = RwSignal::new(String::new());
    let show_new = RwSignal::new(false);
    let email_reset = move || info.get().is_some_and(|i| i.email_reset);
    // Came with an invite to a server that lets invited people make a Laklak account.
    let can_join_without_nc = move || {
        !invite.get().is_empty() && info.get().is_some_and(|i| i.registration != Registration::Closed)
    };

    // Accept an invite link from the address bar (web: /invite/CODE).
    let path = util::window().location().pathname().unwrap_or_default();
    if let Some(code) = path.split("/invite/").nth(1).filter(|c| !c.is_empty()) {
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
                    info_error.set(Some(if e.status == 0 { "No Laklak server answered at that address.".into() } else { e.message }));
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
                    username: laklak_proto::username_key(&username.get_untracked()),
                    password: password.get_untracked(),
                    display_name: Some(display.get_untracked()).filter(|d| !d.trim().is_empty()),
                    invite: Some(invite_code(&invite.get_untracked())).filter(|c| !c.is_empty()),
                    device: Some(device_name()),
                    email: Some(email.get_untracked()).filter(|e| !e.trim().is_empty()),
                };
                api::call("POST", "/api/auth/register", &body).await
            } else {
                let body = LoginReq {
                    username: laklak_proto::username_key(&username.get_untracked()),
                    password: password.get_untracked(),
                    device: Some(device_name()),
                };
                api::call("POST", "/api/auth/login", &body).await
            };
            busy.set(false);
            match r {
                Ok(auth) => {
                    // Sign-up already used the invite; a sign-in still has to accept it.
                    let code = if register { String::new() } else { invite_code(&invite.get_untracked()) };
                    finish(auth, on_done, code);
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };

    let forgot_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if busy.get_untracked() {
            return;
        }
        error.set(None);
        busy.set(true);
        let step = forgot.get_untracked();
        spawn_local(async move {
            let login = username.get_untracked().trim().to_string();
            if step == 1 {
                let r = api::call::<_, ()>("POST", "/api/auth/forgot", &ForgotPassword { login }).await;
                busy.set(false);
                match r {
                    Ok(()) => {
                        code.set(String::new());
                        forgot.set(2);
                    }
                    Err(e) => error.set(Some(e.message)),
                }
            } else {
                let body = ResetWithCode {
                    login,
                    code: code.get_untracked(),
                    new_password: new_pw.get_untracked(),
                    device: Some(device_name()),
                };
                let r = api::call::<_, AuthResp>("POST", "/api/auth/reset", &body).await;
                busy.set(false);
                match r {
                    Ok(auth) => {
                        forgot.set(0);
                        finish(auth, on_done, invite_code(&invite.get_untracked()));
                    }
                    Err(e) => error.set(Some(e.message)),
                }
            }
        });
    };
    let open_forgot = move |e: leptos::ev::MouseEvent| {
        e.prevent_default();
        error.set(None);
        forgot.set(1);
    };
    let back = move |e: leptos::ev::MouseEvent| {
        e.prevent_default();
        error.set(None);
        forgot.set(0);
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
                                finish(auth, on_done, invite_code(&invite.get_untracked()));
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
                <div class="login-brand"><Wordmark/></div>
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
                        <div class="login-title">"Connect to your Laklak server"</div>
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
                                // Someone arriving with an invite and no Nextcloud account goes straight to sign-up.
                                <button class="btn link" on:click=move |_| {
                                    let open = !show_local.get_untracked();
                                    show_local.set(open);
                                    if open && can_join_without_nc() { mode_register.set(true); }
                                }>
                                    {move || if show_local.get() {
                                        "Hide Laklak account sign-in"
                                    } else if can_join_without_nc() {
                                        "No Nextcloud account? Use a Laklak account"
                                    } else {
                                        "Use a Laklak account instead"
                                    }}
                                </button>
                            }
                        })}
                        <Show when=move || forgot.get() != 0 && (show_local.get() || info.get().is_some_and(|i| i.nextcloud.is_none()))>
                            <form class="form" on:submit=forgot_submit>
                                <div class="login-title small">"Reset your password"</div>
                                <Show when=email_reset fallback=|| view! {
                                    <p class="login-hint left">"This server can't send e-mail. Ask one of its admins: they can give you a temporary password."</p>
                                }>
                                    <Show when=move || forgot.get() == 1 fallback=move || view! {
                                        <p class="login-hint left">"If that account has an e-mail address, a code is on its way. It works for 30 minutes; check your spam folder too."</p>
                                        <label>"Code from the e-mail"
                                            <input required=true autocomplete="one-time-code" autocapitalize="characters" spellcheck="false" placeholder="XXXX-XXXX"
                                                prop:value=move || code.get() on:input=move |e| code.set(event_target_value(&e))/>
                                        </label>
                                        <label>"New password"
                                            <div class="pw">
                                                <input type=super::pw_type(show_new) required=true minlength="8" autocomplete="new-password"
                                                    prop:value=move || new_pw.get() on:input=move |e| new_pw.set(event_target_value(&e))/>
                                                <super::Reveal shown=show_new/>
                                            </div>
                                        </label>
                                        {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}
                                        <button class="btn primary wide" type="submit" disabled=move || busy.get()>"Set password and sign in"</button>
                                        <a href="#" class="small" on:click=move |e| { e.prevent_default(); error.set(None); forgot.set(1); }>"Send a new code"</a>
                                    }>
                                        <p class="login-hint left">"We'll e-mail you a code, if your account has an e-mail address."</p>
                                        <label>"Username or e-mail"
                                            <input required=true autocomplete="username" autocapitalize="none" spellcheck="false"
                                                prop:value=move || username.get() on:input=move |e| username.set(event_target_value(&e))/>
                                        </label>
                                        {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}
                                        <button class="btn primary wide" type="submit" disabled=move || busy.get()>"E-mail me a code"</button>
                                    </Show>
                                </Show>
                                <a href="#" class="small" on:click=back>"Back to sign in"</a>
                            </form>
                        </Show>
                        <Show when=move || forgot.get() == 0 && (show_local.get() || info.get().is_some_and(|i| i.nextcloud.is_none()))>
                            <form class="form" on:submit=submit>
                                <div class="tabs">
                                    <button type="button" class:active=move || !mode_register.get() on:click=move |_| mode_register.set(false)>"Sign in"</button>
                                    <button type="button" class:active=move || mode_register.get()
                                        disabled=move || info.get().is_some_and(|i| i.registration == Registration::Closed && !i.first_run)
                                        on:click=move |_| mode_register.set(true)>"Create account"</button>
                                </div>
                                // Accounts with an e-mail address can sign in with it too.
                                <label>{move || if !mode_register.get() && email_reset() { "Username or e-mail" } else { "Username" }}
                                    <input autocomplete="username" autocapitalize="none" spellcheck="false" required=true minlength="2"
                                        maxlength=move || if mode_register.get() { "32" } else { "254" }
                                        prop:value=move || username.get() on:input=move |e| username.set(event_target_value(&e))/>
                                </label>
                                <Show when=move || mode_register.get()>
                                    <label><span>"Display name " <span class="muted">"(optional)"</span></span>
                                        <input maxlength="100" prop:value=move || display.get() on:input=move |e| display.set(event_target_value(&e))/>
                                    </label>
                                </Show>
                                <Show when=move || mode_register.get() && email_reset()>
                                    <label><span>"E-mail " <span class="muted">"(optional: to reset a lost password, or sign in with)"</span></span>
                                        <input type="email" maxlength="254" autocomplete="email" autocapitalize="none" spellcheck="false"
                                            prop:value=move || email.get() on:input=move |e| email.set(event_target_value(&e))/>
                                    </label>
                                </Show>
                                <label>"Password"
                                    <div class="pw">
                                        <input type=super::pw_type(show_pw) required=true minlength=move || if mode_register.get() { "8" } else { "1" }
                                            autocomplete=move || if mode_register.get() { "new-password" } else { "current-password" }
                                            prop:value=move || password.get() on:input=move |e| password.set(event_target_value(&e))/>
                                        <super::Reveal shown=show_pw/>
                                    </div>
                                </label>
                                <Show when=move || !mode_register.get()>
                                    <a href="#" class="forgot-link" on:click=open_forgot>"Forgot password?"</a>
                                </Show>
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
