//! UI sounds, synthesized in `glue.js` (no audio files to ship).

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

pub fn play(s: Sound, enabled: bool) {
    if !enabled {
        return;
    }
    js_play(match s {
        Sound::Message => "message",
        Sound::Join => "join",
        Sound::Leave => "leave",
        Sound::Mute => "mute",
        Sound::Unmute => "unmute",
        Sound::Connect => "connect",
        Sound::Disconnect => "disconnect",
    });
}

pub fn ring(on: bool) {
    js_ring(on);
}
