//! Line icons (24×24, stroked with the text color).

use leptos::prelude::*;

fn paths(name: &str) -> &'static [&'static str] {
    match name {
        "hash" => &["M4 9h16", "M4 15h16", "M10 3 8 21", "M16 3l-2 18"],
        "speaker" => &["M11 5 6 9H2v6h4l5 4V5z", "M15.5 8.5a5 5 0 0 1 0 7", "M19 5a10 10 0 0 1 0 14"],
        "megaphone" => &["M3 11v2a1 1 0 0 0 1 1h2l5 4V6L6 10H4a1 1 0 0 0-1 1z", "M15 8a5 5 0 0 1 0 8", "M18 5a9 9 0 0 1 0 14"],
        "plus" => &["M12 5v14", "M5 12h14"],
        "gear" => &["M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z", "M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"],
        "mic" => &["M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3z", "M19 10v2a7 7 0 0 1-14 0v-2", "M12 19v3"],
        "mic-off" => &["M2 2l20 20", "M18.9 13.4A7 7 0 0 0 19 12v-2", "M5 10v2a7 7 0 0 0 12 5", "M15 9.3V5a3 3 0 0 0-5.7-1.3", "M9 9v3a3 3 0 0 0 5.1 2.1", "M12 19v3"],
        "headphones" => &["M3 18v-6a9 9 0 0 1 18 0v6", "M21 19a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3z", "M3 19a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3z"],
        "headphones-off" => &["M2 2l20 20", "M3 18v-6a9 9 0 0 1 3-6.7", "M9.3 3.3A9 9 0 0 1 21 12v5", "M3 19a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3z", "M21 21a2 2 0 0 1-2 0"],
        "video" => &["M23 7l-7 5 7 5V7z", "M14 5H3a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2z"],
        "screen" => &["M3 4h18a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z", "M8 21h8", "M12 17v4", "M12 13V8", "M9.5 10.5 12 8l2.5 2.5"],
        "hangup" => &["M10.7 13.3a16 16 0 0 0 3.4 2.6l1.3-1.3a2 2 0 0 1 2.1-.4 12.8 12.8 0 0 0 2.8.7 2 2 0 0 1 1.7 2v3a2 2 0 0 1-2.2 2 19.8 19.8 0 0 1-8.6-3.1 19.4 19.4 0 0 1-6-6A19.8 19.8 0 0 1 2.1 4.2 2 2 0 0 1 4.1 2h3a2 2 0 0 1 2 1.7c.1.9.4 1.9.7 2.8a2 2 0 0 1-.4 2.1L8.1 9.9", "M22 2 2 22"],
        "phone" => &["M22 16.9v3a2 2 0 0 1-2.2 2 19.8 19.8 0 0 1-8.6-3.1 19.5 19.5 0 0 1-6-6A19.8 19.8 0 0 1 2.1 4.2 2 2 0 0 1 4.1 2h3a2 2 0 0 1 2 1.7c.1.9.4 1.9.7 2.8a2 2 0 0 1-.4 2.1L8.1 9.9a16 16 0 0 0 6 6l1.3-1.3a2 2 0 0 1 2.1-.4c.9.3 1.8.6 2.8.7a2 2 0 0 1 1.7 2z"],
        "members" => &["M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2", "M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8z", "M23 21v-2a4 4 0 0 0-3-3.9", "M16 3.1a4 4 0 0 1 0 7.8"],
        "pin" => &["M12 17v5", "M9 10.8a2 2 0 0 1-1.1 1.8l-1.8.9A2 2 0 0 0 5 15.2V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.8a2 2 0 0 0-1.1-1.8l-1.8-.9a2 2 0 0 1-1.1-1.8V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"],
        "search" => &["M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16z", "M21 21l-4.3-4.3"],
        "reply" => &["M9 17l-5-5 5-5", "M20 18v-2a4 4 0 0 0-4-4H4"],
        "edit" => &["M12 20h9", "M16.5 3.5a2.1 2.1 0 1 1 3 3L7 19l-4 1 1-4z"],
        "trash" => &["M3 6h18", "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6", "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2", "M10 11v6", "M14 11v6"],
        "smile" => &["M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z", "M8 14s1.5 2 4 2 4-2 4-2", "M9 9h.01", "M15 9h.01"],
        "more" => &["M12 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2z", "M19 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2z", "M5 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2z"],
        "x" => &["M18 6 6 18", "M6 6l12 12"],
        "chevron" => &["M6 9l6 6 6-6"],
        "bell" => &["M18 8a6 6 0 0 0-12 0c0 7-3 9-3 9h18s-3-2-3-9", "M13.7 21a2 2 0 0 1-3.4 0"],
        "bell-off" => &["M13.7 21a2 2 0 0 1-3.4 0", "M18.6 13A17.9 17.9 0 0 1 18 8", "M6.3 6.3A6 6 0 0 0 6 8c0 7-3 9-3 9h14", "M18 8a6 6 0 0 0-9.3-5", "M1 1l22 22"],
        "upload" => &["M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4", "M17 8l-5-5-5 5", "M12 3v12"],
        "file" => &["M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z", "M14 2v6h6"],
        "download" => &["M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4", "M7 10l5 5 5-5", "M12 15V3"],
        "cloud" => &["M18 10h-1.3A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"],
        "folder" => &["M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"],
        "link" => &["M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7", "M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7"],
        "check" => &["M20 6 9 17l-5-5"],
        "star" => &["M12 2l3.1 6.3 6.9 1-5 4.9 1.2 6.8-6.2-3.2-6.2 3.2L7 14.2 2 9.3l6.9-1z"],
        "trending" => &["M23 6l-9.5 9.5-5-5L1 18", "M17 6h6v6"],
        "at" => &["M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8z", "M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-3.9 7.9"],
        "inbox" => &["M22 12h-6l-2 3h-4l-2-3H2", "M5.5 5.1 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.5-6.9A2 2 0 0 0 16.8 4H7.2a2 2 0 0 0-1.7 1.1z"],
        "maximize" => &["M8 3H5a2 2 0 0 0-2 2v3", "M21 8V5a2 2 0 0 0-2-2h-3", "M3 16v3a2 2 0 0 0 2 2h3", "M16 21h3a2 2 0 0 0 2-2v-3"],
        "minimize" => &["M8 3v3a2 2 0 0 1-2 2H3", "M21 8h-3a2 2 0 0 1-2-2V3", "M3 16h3a2 2 0 0 1 2 2v3", "M16 21v-3a2 2 0 0 1 2-2h3"],
        "expand" => &["m21 21-6-6m6 6v-4.8m0 4.8h-4.8", "M3 16.2V21m0 0h4.8M3 21l6-6", "M21 7.8V3m0 0h-4.8M21 3l-6 6", "M3 7.8V3m0 0h4.8M3 3l6 6"],
        "shrink" => &["m15 15 6 6m-6-6v4.8m0-4.8h4.8", "M9 19.8V15m0 0H4.2M9 15l-6 6", "M15 4.2V9m0 0h4.8M15 9l6-6", "M9 4.2V9m0 0H4.2M9 9 3 3"],
        "signal" => &["M2 20h.01", "M7 20v-4", "M12 20v-8", "M17 20V8", "M22 4v16"],
        "logout" => &["M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4", "M16 17l5-5-5-5", "M21 12H9"],
        "keyboard" => &["M20 5H4a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2z", "M6 9h.01", "M10 9h.01", "M14 9h.01", "M18 9h.01", "M7 15h10"],
        "user-plus" => &["M16 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2", "M8.5 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8z", "M20 8v6", "M23 11h-6"],
        "arrow-left" => &["M19 12H5", "M12 19l-7-7 7-7"],
        "arrow-down" => &["M12 5v14", "M19 12l-7 7-7-7"],
        "send" => &["M22 2 11 13", "M22 2 15 22l-4-9-9-4z"],
        "thread" => &["M14 9a2 2 0 0 1-2 2H6l-4 4V4a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2z", "M18 9h2a2 2 0 0 1 2 2v11l-4-4h-6a2 2 0 0 1-2-2v-1"],
        "archive" => &["M21 8v13H3V8", "M1 3h22v5H1z", "M10 12h4"],
        "eye" => &["M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z", "M12 9a3 3 0 1 0 0 6 3 3 0 1 0 0-6z"],
        "eye-off" => &["M9.9 4.24A9 9 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19", "M6.6 6.6A18.4 18.4 0 0 0 1 12s4 8 11 8a9.7 9.7 0 0 0 5.4-1.6", "M14.12 14.12a3 3 0 1 1-4.24-4.24", "M1 1l22 22"],
        "lock" => &["M19 11H5a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7a2 2 0 0 0-2-2z", "M7 11V7a5 5 0 0 1 10 0v4"],
        "shield" => &["M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"],
        "grid" => &["M3 3h7v7H3z", "M14 3h7v7h-7z", "M14 14h7v7h-7z", "M3 14h7v7H3z"],
        "grip" => &["M9 5h.01", "M9 12h.01", "M9 19h.01", "M15 5h.01", "M15 12h.01", "M15 19h.01"],
        _ => &[],
    }
}

#[component]
pub fn Icon(name: &'static str, #[prop(optional)] size: Option<u32>) -> impl IntoView {
    let size = size.unwrap_or(20);
    view! {
        <svg class="icon" width=size height=size viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            {paths(name).iter().map(|d| view! { <path d=*d/> }).collect_view()}
        </svg>
    }
}

/// The headset L: earcup on top, boom arm as stem and foot, mic at the foot's end (Fredoka's stem width).
/// Same geometry as `assets/logo.svg` and `LOGO_L` in glue.js. Built as markup and set with `inner_html`:
/// SVG children made by a separate `view!` would land in the HTML namespace and not draw.
fn logo_markup(gradient_id: Option<&str>) -> String {
    let (defs, fill) = match gradient_id {
        // Pink → turquoise, earcup to mic. Stops are CSS tokens so light themes get the darker set.
        Some(id) => (
            format!(
                concat!(
                    r#"<defs><linearGradient id="{id}" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="78" y2="100">"#,
                    r#"<stop offset="0" style="stop-color:var(--lk-1)"/><stop offset=".25" style="stop-color:var(--lk-2)"/>"#,
                    r#"<stop offset=".5" style="stop-color:var(--lk-3)"/><stop offset=".75" style="stop-color:var(--lk-4)"/>"#,
                    r#"<stop offset="1" style="stop-color:var(--lk-5)"/></linearGradient></defs>"#
                ),
                id = id
            ),
            format!("url(#{id})"),
        ),
        None => (String::new(), "currentColor".to_string()),
    };
    format!(
        concat!(
            r#"{defs}<rect x="0" y="0" width="38" height="46" rx="16" fill="{fill}"/>"#,
            r#"<path d="M19 38V73Q19 89 35 89H61" fill="none" stroke="{fill}" stroke-width="22" stroke-linecap="round"/>"#,
            r#"<rect x="44" y="73" width="34" height="32" rx="16" fill="{fill}"/>"#
        ),
        defs = defs,
        fill = fill
    )
}

/// Each SVG gradient needs a document-unique id.
fn gradient_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    format!("lk-grad-{}", N.fetch_add(1, Ordering::Relaxed))
}

/// The Laklak mark, centred in a square: one colour (`currentColor`) or the brand gradient.
#[component]
pub fn Logo(#[prop(optional)] size: Option<u32>, #[prop(optional)] gradient: bool) -> impl IntoView {
    let size = size.unwrap_or(28);
    let id = gradient.then(gradient_id);
    let markup = logo_markup(id.as_deref());
    view! { <svg width=size height=size viewBox="-17 -3.5 112 112" aria-hidden="true" inner_html=markup></svg> }
}

/// "Laklak" with the gradient headset L as its first letter; sized by the surrounding font-size.
#[component]
pub fn Wordmark() -> impl IntoView {
    let markup = logo_markup(Some(&gradient_id()));
    view! {
        <span class="wordmark" aria-label="Laklak">
            <svg class="wordmark-l" viewBox="-4 -4 82 112" aria-hidden="true" inner_html=markup></svg>
            <span aria-hidden="true">"aklak"</span>
        </span>
    }
}
