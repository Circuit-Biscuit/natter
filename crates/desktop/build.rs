fn main() {
    // Android 15+ phones may use 16 KB memory pages, and Google Play refuses native libraries that aren't
    // aligned for them. (Here rather than in rustflags, which the Tauri CLI sets for Android itself.)
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
    }
    tauri_build::build()
}
