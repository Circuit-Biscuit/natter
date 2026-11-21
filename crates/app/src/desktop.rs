//! Native extras from the Tauri shell, with browser fallbacks so the same app runs on the web.

use leptos::prelude::*;
use natter_proto::Id;
use wasm_bindgen::prelude::*;

use crate::store::store;
use crate::{util, voice};

#[wasm_bindgen(module = "/glue.js")]
extern "C" {
    #[wasm_bindgen(js_name = isTauri)]
    pub fn is_tauri() -> bool;
    #[wasm_bindgen(js_name = isAndroid)]
    pub fn is_android() -> bool;
    #[wasm_bindgen(js_name = tauriInvoke)]
    async fn invoke(cmd: &str, args: &str) -> JsValue;
    #[wasm_bindgen(js_name = tauriListen)]
    fn listen(event: &str, cb: &Closure<dyn FnMut(String)>);
    #[wasm_bindgen(js_name = showBadge)]
    async fn js_badge(icon: &str, count: u32, dot: bool);
    #[wasm_bindgen(js_name = showAppIcon)]
    async fn js_app_icon(icon: &str);
    #[wasm_bindgen(js_name = appIconUrl)]
    pub fn app_icon_url(icon: &str) -> String;
    /// "min", "max" (toggle) or "close", for our own title bar.
    #[wasm_bindgen(js_name = windowAction)]
    pub fn window_action(action: &str);
}

/// The desktop app (Windows, macOS, Linux) — not the browser, not Android.
pub fn is_desktop() -> bool {
    is_tauri() && !is_android()
}

fn call(cmd: &'static str, args: serde_json::Value) {
    wasm_bindgen_futures::spawn_local(async move {
        invoke(cmd, &args.to_string()).await;
    });
}

pub fn notify(title: &str, body: &str, channel: Option<Id>) {
    if is_tauri() {
        let flash = store().settings.with_untracked(|s| s.flash);
        call("notify", serde_json::json!({ "title": title, "body": body, "flash": flash }));
        return;
    }
    use web_sys::{Notification, NotificationOptions, NotificationPermission};
    if Notification::permission() != NotificationPermission::Granted {
        return;
    }
    let opts = NotificationOptions::new();
    opts.set_body(body);
    opts.set_icon("logo.svg");
    // Our own sound plays instead (Settings › Notifications), so the OS one doesn't double it.
    opts.set_silent(Some(true));
    if let Ok(n) = Notification::new_with_options(title, &opts) {
        let onclick = Closure::<dyn FnMut()>::new(move || {
            let _ = util::window().focus();
            if let Some(c) = channel {
                crate::store::open_channel(c);
            }
        });
        n.set_onclick(Some(onclick.as_ref().unchecked_ref()));
        onclick.forget();
    }
}

/// Browsers need a click before they may ask for notification permission.
pub fn request_notification_permission() {
    if !is_tauri() && web_sys::Notification::permission() == web_sys::NotificationPermission::Default {
        let _ = web_sys::Notification::request_permission();
    }
}

/// Unread badge: red number (mentions + DMs) or red dot (anything else unread) on the tray icon and taskbar,
/// the tab icon in a browser; the count also goes in the window title.
pub fn set_badge(icon: &str, count: u32, dot: bool) {
    if let Some(doc) = util::window().document() {
        doc.set_title(&if count > 0 { format!("({count}) Natter") } else { "Natter".into() });
    }
    if is_android() {
        return;
    }
    let icon = icon.to_string();
    wasm_bindgen_futures::spawn_local(async move { js_badge(&icon, count, dot).await });
}

/// The window / taskbar icon variant (desktop app).
pub fn set_app_icon(icon: &str) {
    if is_desktop() {
        let icon = icon.to_string();
        wasm_bindgen_futures::spawn_local(async move { js_app_icon(&icon).await });
    }
}

pub fn open_url(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    if is_tauri() {
        call("open_url", serde_json::json!({ "url": url }));
    } else {
        let _ = util::window().open_with_url_and_target_and_features(url, "_blank", "noopener,noreferrer");
    }
}

// ---------------------------------------------------------------- keybinds

/// Actions a custom keybind can run: (id, label). `toggle_window` only exists in the desktop app.
pub const KEYBIND_ACTIONS: [(&str, &str); 6] = [
    ("toggle_mute", "Toggle mute"),
    ("toggle_deafen", "Toggle deafen"),
    ("toggle_camera", "Toggle camera"),
    ("toggle_screen", "Toggle screen share"),
    ("leave_call", "Leave call"),
    ("toggle_window", "Show / hide Natter"),
];

pub fn run_keybind(action: &str, down: bool) {
    if action == "ptt" {
        return voice::ptt(down);
    }
    if !down {
        return;
    }
    let in_call = store().call.with_untracked(|c| c.channel.is_some());
    match action {
        "toggle_mute" => voice::toggle_mute(),
        "toggle_deafen" => voice::toggle_deafen(),
        "toggle_camera" if in_call => voice::toggle_camera(),
        "toggle_screen" if in_call => voice::toggle_screen(),
        "leave_call" => voice::leave(),
        _ => {}
    }
}

/// "Ctrl+Shift+KeyM" for a key press with modifiers; `None` while only modifiers are held.
pub fn combo(e: &web_sys::KeyboardEvent) -> Option<String> {
    let code = e.code();
    if code.is_empty() || ["Control", "Shift", "Alt", "Meta", "OS"].iter().any(|m| code.starts_with(m)) {
        return None;
    }
    let mut s = String::new();
    for (on, name) in [(e.ctrl_key(), "Ctrl+"), (e.alt_key(), "Alt+"), (e.shift_key(), "Shift+"), (e.meta_key(), "Super+")] {
        if on {
            s.push_str(name);
        }
    }
    Some(s + &code)
}

/// "Ctrl+Shift+KeyM" → "Ctrl + Shift + M".
pub fn combo_label(keys: &str) -> String {
    keys.split('+').map(|k| k.strip_prefix("Key").or_else(|| k.strip_prefix("Digit")).unwrap_or(k)).collect::<Vec<_>>().join(" + ")
}

/// Hand the keybinds (and push-to-talk) to the desktop app, which makes them global.
pub fn sync_keybinds() {
    if !is_desktop() {
        return;
    }
    let p = store().prefs.get_untracked();
    let mut binds: Vec<serde_json::Value> = p
        .keybinds
        .iter()
        .filter(|b| !b.keys.is_empty())
        .map(|b| serde_json::json!({ "action": b.action, "keys": b.keys }))
        .collect();
    if p.push_to_talk && !p.ptt_key.is_empty() {
        binds.push(serde_json::json!({ "action": "ptt", "keys": p.ptt_key }));
    }
    wasm_bindgen_futures::spawn_local(async move {
        let failed = invoke("set_keybinds", &serde_json::json!({ "binds": binds }).to_string()).await;
        let failed: Vec<String> = js_sys::Array::from(&failed).iter().filter_map(|v| v.as_string()).collect();
        if !failed.is_empty() {
            let keys: Vec<String> = failed.iter().map(|k| combo_label(k)).collect();
            store().toast(format!("Another app already uses {} — pick a different key.", keys.join(", ")), true);
        }
    });
}

// ---------------------------------------------------------------- window & launch (desktop app)

pub fn set_close_to_tray(on: bool) {
    if is_desktop() {
        call("set_close_to_tray", serde_json::json!({ "on": on }));
    }
}

pub fn set_zoom(percent: u16) {
    if is_desktop() {
        call("set_zoom", serde_json::json!({ "factor": f64::from(percent) / 100.0 }));
    }
}

/// Start with the computer (and whether to stay in the tray then). False if the OS refused.
pub async fn set_launch(autostart: bool, minimized: bool) -> bool {
    let r = invoke("set_launch", &serde_json::json!({ "autostart": autostart, "minimized": minimized }).to_string()).await;
    r.as_bool().unwrap_or(false)
}

pub async fn autostart_enabled() -> bool {
    invoke("get_autostart", "{}").await.as_bool().unwrap_or(false)
}

// ---------------------------------------------------------------- updates (desktop app and Android)

/// Natter's public releases. Desktop updates come through the shell (signed `latest.json`); Android can't
/// install apps by itself, so it links to the newest APK from here.
const LATEST_RELEASE: &str = "https://api.github.com/repos/Circuit-Biscuit/natter/releases/latest";

/// Look for a newer Natter a minute after start, then every six hours. The desktop shell downloads it in the
/// background, then the title bar shows "Update"; on Android a banner links to the APK. The web app updates
/// with its server. (At launch the desktop shell has just checked by itself, hence the minute.)
pub fn watch_updates() {
    if !is_tauri() {
        return;
    }
    set_timeout(check_updates, std::time::Duration::from_secs(60));
    set_interval(check_updates, std::time::Duration::from_secs(6 * 3600));
}

fn check_updates() {
    wasm_bindgen_futures::spawn_local(async {
        let found = if is_android() {
            android_update().await
        } else {
            invoke("check_update", "{}").await.as_string().map(|v| (v, None))
        };
        if found.is_some() {
            store().update.set(found);
        }
    });
}

async fn android_update() -> Option<(String, Option<String>)> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
        assets: Vec<Asset>,
    }
    #[derive(serde::Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    let r: Release = util::fetch_json(LATEST_RELEASE).await?;
    let version = r.tag_name.trim_start_matches('v').to_string();
    if !newer(&version, env!("CARGO_PKG_VERSION")) {
        return None;
    }
    let apk = r.assets.into_iter().find(|a| a.name.ends_with(".apk"))?.browser_download_url;
    Some((version, Some(apk)))
}

/// "0.10.0" is newer than "0.9.3".
fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| v.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    parse(a) > parse(b)
}

/// The title bar's "Update": install the downloaded update and restart (asks first during a call).
pub fn install_update() {
    let go = || {
        wasm_bindgen_futures::spawn_local(async {
            // On success Natter restarts and this never returns.
            invoke("install_update", "{}").await;
            store().update.set(None);
            store().toast("Couldn't install the update. Natter will try again later.", true);
        });
    };
    if store().call.with_untracked(|c| c.channel.is_some()) {
        store().confirm("Update now?", "Natter restarts to update, which ends your call.", "Restart and update", go);
    } else {
        go();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn newer_versions() {
        assert!(super::newer("0.7.0", "0.6.1"));
        assert!(super::newer("0.10.0", "0.9.3"));
        assert!(super::newer("1.0.0", "0.99.99"));
        assert!(!super::newer("0.6.1", "0.6.1"));
        assert!(!super::newer("0.6.0", "0.6.1"));
    }
}

/// Apply this device's preferences at startup.
pub fn apply_prefs() {
    let p = store().prefs.get_untracked();
    voice::set_ptt_mode(p.push_to_talk);
    if !is_desktop() {
        return;
    }
    sync_keybinds();
    set_close_to_tray(p.close_to_tray);
    if p.zoom != 100 {
        set_zoom(p.zoom);
    }
}

/// Events from the shell: global keybinds, and notification clicks.
pub fn init() {
    let cb = Closure::<dyn FnMut(String)>::new(|payload: String| {
        #[derive(serde::Deserialize)]
        struct Ev {
            action: String,
            down: bool,
        }
        if let Ok(ev) = serde_json::from_str::<Ev>(&payload) {
            run_keybind(&ev.action, ev.down);
        }
    });
    listen("keybind", &cb);
    cb.forget();
    let open = Closure::<dyn FnMut(String)>::new(|payload: String| {
        if let Ok(id) = payload.trim_matches('"').parse::<Id>() {
            crate::store::open_channel(id);
        }
    });
    listen("open_channel", &open);
    open.forget();
}
