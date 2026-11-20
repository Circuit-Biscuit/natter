//! Natter app shell: Tauri 2 around the Rust/WASM UI, for desktop (Windows, macOS, Linux) and Android.
//!
//! Desktop adds what a browser tab can't: tray icon with close-to-tray, OS notifications, an unread badge on
//! the taskbar and tray, global keybinds, starting with the computer, one running instance, a window that
//! remembers its size. On Android the same commands exist; the desktop-only ones quietly do nothing.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tauri_plugin_opener::OpenerExt;

/// Passed by the OS when Natter starts with the computer.
#[cfg(desktop)]
const AUTOSTART_ARG: &str = "--autostart";

/// The close button hides Natter to the tray (default) instead of quitting.
struct CloseToTray(AtomicBool);

/// A square RGBA image drawn by the UI (it has fonts; the badge number needs them).
#[derive(Deserialize)]
struct Icon {
    rgba: Vec<u8>,
    size: u32,
}

impl Icon {
    #[cfg(desktop)]
    fn image(self) -> Option<tauri::image::Image<'static>> {
        (self.rgba.len() == (self.size * self.size * 4) as usize).then(|| tauri::image::Image::new_owned(self.rgba, self.size, self.size))
    }
}

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

#[cfg(desktop)]
fn toggle(app: &AppHandle) {
    match main_window(app) {
        Some(w) if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) => {
            let _ = w.hide();
        }
        _ => show(app),
    }
}

/// OS notification (silent: the app plays its own sound); flashes the taskbar when Natter isn't in front.
#[tauri::command]
fn notify(app: AppHandle, title: String, body: String, flash: Option<bool>) {
    let n = app.notification();
    // Android 13+ asks once; until then notifications are silently dropped.
    if !matches!(n.permission_state(), Ok(PermissionState::Granted)) {
        let _ = n.request_permission();
    }
    let _ = n.builder().title(&title).body(&body).show();
    #[cfg(desktop)]
    if flash.unwrap_or(true)
        && let Some(w) = main_window(&app)
        && !w.is_focused().unwrap_or(true)
    {
        let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
    #[cfg(mobile)]
    let _ = flash;
}

/// Unread badge. `tray` is the tray icon with the badge drawn on; `overlay` is the badge alone for the
/// Windows taskbar (macOS and Linux show `count` on the dock / launcher instead).
#[tauri::command]
fn set_badge(app: AppHandle, count: u32, tray: Option<Icon>, overlay: Option<Icon>) {
    #[cfg(desktop)]
    {
        if let Some(w) = main_window(&app) {
            #[cfg(target_os = "windows")]
            {
                let _ = w.set_overlay_icon(overlay.and_then(Icon::image));
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = overlay;
                let _ = w.set_badge_count((count > 0).then_some(i64::from(count)));
            }
        }
        if let Some(t) = app.tray_by_id("natter") {
            if let Some(img) = tray.and_then(Icon::image) {
                let _ = t.set_icon(Some(img));
            }
            let tip = match count {
                0 => "Natter".to_string(),
                1 => "Natter — 1 unread mention".into(),
                n => format!("Natter — {n} unread mentions"),
            };
            let _ = t.set_tooltip(Some(tip));
        }
    }
    #[cfg(mobile)]
    let _ = (app, count, tray, overlay);
}

/// The window (taskbar) icon, when a different app icon is picked in Settings › Appearance.
#[tauri::command]
fn set_app_icon(app: AppHandle, icon: Icon) {
    #[cfg(desktop)]
    if let (Some(w), Some(img)) = (main_window(&app), icon.image()) {
        let _ = w.set_icon(img);
    }
    #[cfg(mobile)]
    let _ = (app, icon);
}

/// Links in messages open in the default browser — never inside the app.
#[tauri::command]
fn open_url(app: AppHandle, url: String) {
    if url.starts_with("https://") || url.starts_with("http://") {
        let _ = app.opener().open_url(url, None::<&str>);
    }
}

#[derive(Deserialize)]
#[cfg_attr(mobile, allow(dead_code))]
struct Bind {
    action: String,
    /// "Ctrl+Shift+KeyM": modifiers, then a DOM `KeyboardEvent.code`.
    keys: String,
}

/// Global keybinds (push-to-talk among them): each becomes a system-wide hotkey, and the app gets
/// `keybind` events ({action, down}) even while Natter is in the background. "Show / hide Natter" is
/// handled here. Returns the key combos that couldn't be registered (another app holds them).
// ponytail: an OS hotkey reserves the combo system-wide while Natter runs; a raw keyboard hook would
// share it but needs Input Monitoring permission on macOS.
#[tauri::command]
fn set_keybinds(app: AppHandle, binds: Vec<Bind>) -> Vec<String> {
    #[cfg(desktop)]
    {
        use std::str::FromStr;
        use tauri::Emitter;
        use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
        let gs = app.global_shortcut();
        let _ = gs.unregister_all();
        let mut failed = vec![];
        for b in binds {
            let Ok(shortcut) = Shortcut::from_str(&b.keys) else {
                failed.push(b.keys);
                continue;
            };
            let action = b.action;
            let ok = gs.on_shortcut(shortcut, move |app, _, ev| {
                let down = ev.state() == ShortcutState::Pressed;
                if action == "toggle_window" {
                    if down {
                        toggle(app);
                    }
                } else {
                    let _ = app.emit("keybind", serde_json::json!({ "action": &action, "down": down }));
                }
            });
            if ok.is_err() {
                failed.push(b.keys);
            }
        }
        failed
    }
    #[cfg(mobile)]
    {
        let _ = (app, binds);
        vec![]
    }
}

#[tauri::command]
fn set_close_to_tray(state: tauri::State<CloseToTray>, on: bool) {
    state.0.store(on, Ordering::Relaxed);
}

#[tauri::command]
fn set_zoom(app: AppHandle, factor: f64) {
    #[cfg(desktop)]
    if let Some(w) = main_window(&app) {
        let _ = w.set_zoom(factor.clamp(0.5, 2.0));
    }
    #[cfg(mobile)]
    let _ = (app, factor);
}

/// Marker file: when started with the computer, stay in the tray.
#[cfg(desktop)]
fn start_minimized_flag(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("start-minimized"))
}

/// Start Natter with the computer (or not), and whether it then stays in the tray. True when it worked.
#[tauri::command]
fn set_launch(app: AppHandle, autostart: bool, minimized: bool) -> bool {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let al = app.autolaunch();
        if (if autostart { al.enable() } else { al.disable() }).is_err() {
            return false;
        }
        let Some(flag) = start_minimized_flag(&app) else { return false };
        if minimized {
            flag.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(&flag, b"").is_ok()
        } else {
            let _ = std::fs::remove_file(&flag);
            true
        }
    }
    #[cfg(mobile)]
    {
        let _ = (app, autostart, minimized);
        false
    }
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        app.autolaunch().is_enabled().unwrap_or(false)
    }
    #[cfg(mobile)]
    {
        let _ = app;
        false
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder
        // A second launch just brings the running Natter forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show(app)))
        // Size and position only: whether the window shows is decided at startup (tray start), and the frame is
        // always ours (state saved by 0.5 and older says "decorated", which would bring the Windows title bar back).
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        & !tauri_plugin_window_state::StateFlags::VISIBLE
                        & !tauri_plugin_window_state::StateFlags::DECORATIONS,
                )
                .build(),
        )
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec![AUTOSTART_ARG])));
    builder
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(CloseToTray(AtomicBool::new(true)))
        .invoke_handler(tauri::generate_handler![
            notify,
            set_badge,
            set_app_icon,
            open_url,
            set_keybinds,
            set_close_to_tray,
            set_zoom,
            set_launch,
            get_autostart
        ])
        .setup(|_app| {
            #[cfg(desktop)]
            {
                tray(_app)?;
                // The window starts hidden (tauri.conf.json); show it unless we started with the computer
                // and the user wants Natter to wait in the tray.
                let app = _app.handle();
                let quiet = std::env::args().any(|a| a == AUTOSTART_ARG) && start_minimized_flag(app).is_some_and(|f| f.exists());
                if !quiet {
                    show(app);
                }
            }
            Ok(())
        })
        .on_window_event(|window, ev| {
            // Closing the window keeps Natter running in the tray (calls and notifications continue),
            // unless that's switched off in Settings › System.
            #[cfg(desktop)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = ev
                && window.label() == "main"
            {
                if window.state::<CloseToTray>().0.load(Ordering::Relaxed) {
                    let _ = window.hide();
                    api.prevent_close();
                } else {
                    window.app_handle().exit(0);
                }
            }
            #[cfg(mobile)]
            let _ = (window, ev);
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
