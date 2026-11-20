//! UI sounds, synthesized in `glue.js` (no audio files to ship).

use leptos::prelude::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/glue.js")]
extern "C" {
    #[wasm_bindgen(js_name = playSound)]
    fn js_play(name: &str);
    #[wasm_bindgen(js_name = ring)]
    fn js_ring(on: bool);
}

#[derive(Clone, Copy)]
pub enum Sound {
    Message,
    Join,
    Leave,
    Mute,
    Unmute,
    Connect,
    Disconnect,
}

/// Message sounds to pick from in Settings › Notifications: (id, label).
pub const MESSAGE_SOUNDS: [(&str, &str); 4] = [("chime", "Chime"), ("pop", "Pop"), ("soft", "Soft"), ("ping", "Ping")];

pub fn play(s: Sound, enabled: bool) {
    if !enabled {
        return;
    }
    let name = match s {
        Sound::Message => return preview_message(&crate::store::store().settings.with_untracked(|st| st.message_sound.clone())),
        Sound::Join => "join",
        Sound::Leave => "leave",
        Sound::Mute => "mute",
        Sound::Unmute => "unmute",
        Sound::Connect => "connect",
        Sound::Disconnect => "disconnect",
    };
    js_play(name);
}

/// A message sound by id (also the Settings preview).
pub fn preview_message(name: &str) {
    js_play(&format!("message-{name}"));
}

pub fn ring(on: bool) {
    js_ring(on);
}
