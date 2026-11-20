//! Native extras from the Tauri shell, with browser fallbacks so the same app runs on the web.

use natter_proto::Id;
use wasm_bindgen::prelude::*;

use crate::util;

#[wasm_bindgen(module = "/glue.js")]
extern "C" {
    #[wasm_bindgen(js_name = isTauri)]
    pub fn is_tauri() -> bool;
    #[wasm_bindgen(js_name = tauriInvoke)]
    async fn invoke(cmd: &str, args: &str) -> JsValue;
    #[wasm_bindgen(js_name = tauriListen)]
    fn listen(event: &str, cb: &Closure<dyn FnMut(String)>);
}

fn call(cmd: &'static str, args: serde_json::Value) {
    wasm_bindgen_futures::spawn_local(async move {
        invoke(cmd, &args.to_string()).await;
    });
}

pub fn notify(title: &str, body: &str, channel: Option<Id>) {
    if is_tauri() {
        call("notify", serde_json::json!({ "title": title, "body": body }));
        return;
    }
    use web_sys::{Notification, NotificationOptions, NotificationPermission};
    if Notification::permission() != NotificationPermission::Granted {
        return;
    }
    let opts = NotificationOptions::new();
    opts.set_body(body);
    opts.set_icon("logo.svg");
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

/// Unread mentions on the taskbar / dock (and in the browser tab title).
pub fn set_badge(count: u32) {
    if is_tauri() {
        call("set_badge", serde_json::json!({ "count": count }));
    }
    if let Some(doc) = util::window().document() {
        doc.set_title(&if count > 0 { format!("({count}) Natter") } else { "Natter".into() });
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

/// Global push-to-talk key (works while Natter is in the background). Desktop only.
pub fn set_ptt_key(code: Option<&str>) {
    if is_tauri() {
        call("set_ptt_key", serde_json::json!({ "code": code }));
    }
}

/// The shell reports global push-to-talk presses as `ptt` events with a bool payload.
pub fn init() {
    let cb = Closure::<dyn FnMut(String)>::new(|payload: String| {
        crate::voice::ptt(payload == "true");
    });
    listen("ptt", &cb);
    cb.forget();
    let open = Closure::<dyn FnMut(String)>::new(|payload: String| {
        if let Ok(id) = payload.trim_matches('"').parse::<Id>() {
            crate::store::open_channel(id);
        }
    });
    listen("open_channel", &open);
    open.forget();
}
