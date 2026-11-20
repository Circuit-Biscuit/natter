//! Natter app shell: Tauri 2 around the Rust/WASM UI, for desktop (Windows, macOS, Linux) and Android.
//!
//! Desktop adds what a browser tab can't: tray icon with close-to-tray, OS notifications, taskbar badge,
//! global push-to-talk, one running instance, a window that remembers its size. On Android the same
//! commands exist; the desktop-only ones quietly do nothing.

use tauri::AppHandle;
#[cfg(desktop)]
use tauri::Manager;
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tauri_plugin_opener::OpenerExt;

#[cfg(desktop)]
fn main_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window("main")
}

#[cfg(desktop)]
fn show(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// OS notification for a DM or mention; on desktop also flashes the taskbar when Natter isn't focused.
#[tauri::command]
fn notify(app: AppHandle, title: String, body: String) {
    let n = app.notification();
    // Android 13+ asks once; until then notifications are silently dropped.
    if !matches!(n.permission_state(), Ok(PermissionState::Granted)) {
        let _ = n.request_permission();
    }
    let _ = n.builder().title(&title).body(&body).show();
    #[cfg(desktop)]
    if let Some(w) = main_window(&app) {
        if !w.is_focused().unwrap_or(true) {
            let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
        }
    }
}

/// Unread mentions: dock badge on macOS/Linux, a red dot on the taskbar icon on Windows.
#[tauri::command]
fn set_badge(app: AppHandle, count: u32) {
    #[cfg(desktop)]
    {
        let Some(w) = main_window(&app) else { return };
        #[cfg(target_os = "windows")]
        {
            let _ = w.set_overlay_icon(if count > 0 { Some(red_dot()) } else { None });
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = w.set_badge_count(if count > 0 { Some(count as i64) } else { None });
        }
        if let Some(tray) = app.tray_by_id("natter") {
            let tip = if count > 0 { format!("Natter — {count} unread mention(s)") } else { "Natter".into() };
            let _ = tray.set_tooltip(Some(tip));
        }
    }
    #[cfg(mobile)]
    let _ = (app, count);
}

#[cfg(target_os = "windows")]
fn red_dot() -> tauri::image::Image<'static> {
    const N: u32 = 16;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
            let alpha = (8.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            px.extend_from_slice(&[0xDA, 0x37, 0x3C, (alpha * 255.0) as u8]);
        }
    }
    tauri::image::Image::new_owned(px, N, N)
}

/// Links in messages open in the default browser — never inside the app.
#[tauri::command]
fn open_url(app: AppHandle, url: String) {
    if url.starts_with("https://") || url.starts_with("http://") {
        let _ = app.opener().open_url(url, None::<&str>);
    }
}

/// Global push-to-talk: `code` is a DOM `KeyboardEvent.code` ("Backquote", "F13", "KeyV"…), `None` turns it off.
/// The app gets `ptt` events (true = pressed, false = released) even while it is in the background.
// ponytail: an OS hotkey reserves the key system-wide while Natter runs; a raw keyboard hook would
// share it but needs Input Monitoring permission on macOS. Pick a key you don't type with.
#[tauri::command]
fn set_ptt_key(app: AppHandle, code: Option<String>) -> Result<(), String> {
    #[cfg(desktop)]
    {
        use std::str::FromStr;
        use tauri::Emitter;
        use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut, ShortcutState};
        let gs = app.global_shortcut();
        gs.unregister_all().map_err(|e| e.to_string())?;
        let Some(code) = code else { return Ok(()) };
        let key = Code::from_str(&code).map_err(|_| format!("unknown key {code}"))?;
        gs.on_shortcut(Shortcut::new(None, key), |app, _, ev| {
            let _ = app.emit("ptt", ev.state() == ShortcutState::Pressed);
        })
        .map_err(|e| e.to_string())
    }
    #[cfg(mobile)]
    {
        let _ = (app, code);
        Ok(())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder
        // A second launch just brings the running Natter forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show(app)))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build());
    builder
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![notify, set_badge, open_url, set_ptt_key])
        .setup(|_app| {
            #[cfg(desktop)]
            tray(_app)?;
            Ok(())
        })
        .on_window_event(|_window, _ev| {
            // Closing the window keeps Natter running in the tray (calls and notifications continue).
            #[cfg(desktop)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = _ev {
                if _window.label() == "main" {
                    let _ = _window.hide();
                    api.prevent_close();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Natter failed to start");
}

#[cfg(desktop)]
fn tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    let open = MenuItem::with_id(app, "open", "Open Natter", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Natter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("natter")
        .tooltip("Natter")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "open" => show(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}
