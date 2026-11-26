//! Laklak app: the whole UI, in Rust, compiled to WebAssembly. Runs in the desktop shell or a browser.

mod api;
mod desktop;
mod gateway;
mod markdown;
mod sound;
mod store;
mod ui;
mod util;
mod voice;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(ui::App);
}
