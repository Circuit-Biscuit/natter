//! Small helpers: time, formatting, storage, colors.

use natter_proto::{Id, Millis};
use wasm_bindgen::JsCast;

pub fn now() -> Millis {
    js_sys::Date::now() as Millis
}

pub fn window() -> web_sys::Window {
    web_sys::window().expect("window")
}

/// GET a JSON document from another site (it must allow CORS).
pub async fn fetch_json<T: serde::de::DeserializeOwned>(url: &str) -> Option<T> {
    let resp: web_sys::Response = wasm_bindgen_futures::JsFuture::from(window().fetch_with_str(url)).await.ok()?.dyn_into().ok()?;
    if !resp.ok() {
        return None;
    }
    let text = wasm_bindgen_futures::JsFuture::from(resp.text().ok()?).await.ok()?.as_string()?;
    serde_json::from_str(&text).ok()
}

pub fn storage_get(key: &str) -> Option<String> {
    window().local_storage().ok()??.get_item(key).ok()?
}

pub fn storage_set(key: &str, value: &str) {
    if let Ok(Some(s)) = window().local_storage() {
        let _ = s.set_item(key, value);
    }
}

pub fn storage_remove(key: &str) {
    if let Ok(Some(s)) = window().local_storage() {
        let _ = s.remove_item(key);
    }
}

/// Unique enough for message nonces and element ids.
pub fn nonce() -> String {
    format!("{:x}{:08x}", now(), (js_sys::Math::random() * u32::MAX as f64) as u32)
}

fn date_parts(ms: Millis) -> js_sys::Date {
    js_sys::Date::new(&(ms as f64).into())
}

fn hm(d: &js_sys::Date) -> String {
    format!("{:02}:{:02}", d.get_hours(), d.get_minutes())
}

fn day_key(d: &js_sys::Date) -> (u32, u32, u32) {
    (d.get_full_year(), d.get_month(), d.get_date())
}

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
    "November", "December",
];

/// "Today at 14:03", "Yesterday at 09:12", "03/11/2026 17:40".
pub fn timestamp(ms: Millis) -> String {
    let d = date_parts(ms);
    let today = date_parts(now());
    let yesterday = date_parts(now() - 86_400_000);
    if day_key(&d) == day_key(&today) {
        format!("Today at {}", hm(&d))
    } else if day_key(&d) == day_key(&yesterday) {
        format!("Yesterday at {}", hm(&d))
    } else {
        format!("{:02}/{:02}/{} {}", d.get_date(), d.get_month() + 1, d.get_full_year(), hm(&d))
    }
}

/// "14:03" for grouped messages.
pub fn short_time(ms: Millis) -> String {
    hm(&date_parts(ms))
}

/// "Friday, 20 November 2026 at 14:03" for tooltips.
pub fn long_time(ms: Millis) -> String {
    let d = date_parts(ms);
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    format!(
        "{}, {} {} {} at {}",
        DAYS[d.get_day() as usize],
        d.get_date(),
        MONTHS[d.get_month() as usize],
        d.get_full_year(),
        hm(&d)
    )
}

/// "20 November 2026" for day dividers.
pub fn day_label(ms: Millis) -> String {
    let d = date_parts(ms);
    format!("{} {} {}", d.get_date(), MONTHS[d.get_month() as usize], d.get_full_year())
}

pub fn same_day(a: Millis, b: Millis) -> bool {
    day_key(&date_parts(a)) == day_key(&date_parts(b))
}

pub fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

/// Stable, pleasant color per user for avatars without a picture.
pub fn avatar_color(id: Id) -> &'static str {
    const COLORS: [&str; 8] =
        ["#5865f2", "#3ba55c", "#faa61a", "#ed4245", "#eb459e", "#4b56d2", "#1abc9c", "#e67e22"];
    COLORS[(id.unsigned_abs() as usize / 7) % COLORS.len()]
}

pub fn initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    let a = words.next().and_then(|w| w.chars().next());
    let b = words.next().and_then(|w| w.chars().next());
    a.into_iter().chain(b).flat_map(char::to_uppercase).collect()
}

pub fn copy_text(text: &str) {
    let _ = window().navigator().clipboard().write_text(text);
}

pub fn element_by_id<T: JsCast>(id: &str) -> Option<T> {
    window().document()?.get_element_by_id(id)?.dyn_into().ok()
}

pub fn focus(id: &str) {
    if let Some(el) = element_by_id::<web_sys::HtmlElement>(id) {
        let _ = el.focus();
    }
}

/// Is this text nothing but a few emoji? (Shown jumbo, like Discord.)
pub fn only_emoji(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > 30 {
        return false;
    }
    t.chars().all(|c| {
        c.is_whitespace()
            || matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0xFE0F | 0x200D | 0x2190..=0x21FF | 0x2300..=0x23FF | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0x00A9 | 0x00AE | 0x2122)
    })
}

/// Replace `:shortcode:` with the emoji, leaving code spans alone.
pub fn expand_shortcodes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    let mut rest = text;
    while let Some(i) = rest.find([':', '`']) {
        out.push_str(&rest[..i]);
        let c = rest.as_bytes()[i];
        rest = &rest[i..];
        if c == b'`' {
            in_code = !in_code;
            out.push('`');
            rest = &rest[1..];
            continue;
        }
        if !in_code {
            if let Some(end) = rest[1..].find(':') {
                let code = &rest[1..1 + end];
                if !code.is_empty() && !code.contains(char::is_whitespace) {
                    if let Some(e) = emojis::get_by_shortcode(code) {
                        out.push_str(e.as_str());
                        rest = &rest[end + 2..];
                        continue;
                    }
                }
            }
        }
        out.push(':');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}
