//! Link previews ("embeds"): YouTube cards that play in place, page cards from OpenGraph tags, pictures and
//! videos linked directly. The server fetches them once (cached per URL) and sends the message again with them.
//!
//! Fetching a URL someone typed is a server-side request forgery risk, so every hop is checked: http(s) on the
//! standard ports only, every address the host resolves to must be public, the connection is pinned to the
//! address that was checked (no DNS rebinding), redirects are followed by hand (re-checked, at most 4), bodies are
//! capped at 512 KiB and requests time out after 5 s.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use natter_proto::{Channel, Embed, EmbedKind, Event, Id};
use reqwest::Url;

use crate::{St, db, now_ms};

/// At most this many previews per message.
pub const MAX_EMBEDS: usize = 4;
const MAX_BODY: usize = 512 * 1024;
const UA: &str = "Mozilla/5.0 (compatible; NatterBot/1.0; link previews)";
/// Re-fetch a URL after a day; retry failures after an hour.
const FRESH_MS: i64 = 86_400_000;
const RETRY_MS: i64 = 3_600_000;

/// The http(s) links in a message worth previewing: not inside `code`, not wrapped in <angle brackets> (that
/// turns the preview off, like Discord), not links into our own Nextcloud (those need a sign-in).
pub fn urls(content: &str, skip_host: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut in_code = false;
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            in_code = !in_code;
            i += 1;
            continue;
        }
        let rest = &content[i..];
        if !in_code && (rest.starts_with("https://") || rest.starts_with("http://")) {
            let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`')).unwrap_or(rest.len());
            let mut url = &rest[..end];
            // Trailing punctuation and an unmatched ")" (from "[label](url)" or a sentence) aren't part of it.
            loop {
                let trimmed = url.trim_end_matches(['.', ',', '!', '?', ':', ';', '\'', '*', '_', '~']);
                let trimmed = if trimmed.ends_with(')') && trimmed.matches('(').count() < trimmed.matches(')').count() {
                    &trimmed[..trimmed.len() - 1]
                } else {
                    trimmed
                };
                if trimmed.len() == url.len() {
                    break;
                }
                url = trimmed;
            }
            let suppressed = i > 0 && bytes[i - 1] == b'<';
            let own = Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).is_some_and(|h| Some(h.as_str()) == skip_host);
            if !suppressed && !own && url.len() > 10 && !out.iter().any(|u| u == url) {
                out.push(url.to_string());
                if out.len() == MAX_EMBEDS {
                    break;
                }
            }
            i += end.max(1);
            continue;
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// Fill in (or clear) a message's previews and send it again to everyone who sees it.
pub async fn refresh(st: St, ch: Channel, id: Id) {
    if let Err(e) = refresh_inner(&st, &ch, id, true).await {
        tracing::debug!("embeds for {id}: {}", e.msg);
    }
}

async fn refresh_inner(st: &St, ch: &Channel, id: Id, announce: bool) -> crate::R<()> {
    let msg = db::message(&st.db, ch.id, id).await?;
    let skip = st.cfg.nextcloud_url.as_deref().and_then(|u| Url::parse(u).ok()).and_then(|u| u.host_str().map(str::to_string));
    let mut embeds = vec![];
    for url in urls(&msg.content, skip.as_deref()) {
        if let Some(e) = embed(st, &url).await {
            embeds.push(e);
        }
    }
    if embeds == msg.embeds && !embeds.is_empty() {
        return Ok(());
    }
    sqlx::query("INSERT INTO message_embeds (message_id, data) VALUES (?, ?) ON CONFLICT (message_id) DO UPDATE SET data = excluded.data")
        .bind(id)
        .bind(serde_json::to_string(&embeds).unwrap_or_else(|_| "[]".into()))
        .execute(&st.db)
        .await?;
    if announce && (embeds != msg.embeds) {
        let msg = db::message(&st.db, ch.id, id).await?;
        st.hub.send(&db::audience(&st.db, ch).await?, &Event::MessageUpdate(msg));
    }
    Ok(())
}

/// Previews for messages from before embeds (or imported): the newest first, once each, quietly.
pub async fn backfill(st: St) {
    let rows: Vec<(Id, Id)> = sqlx::query_as(
        "SELECT id, channel_id FROM messages
         WHERE (content LIKE '%http://%' OR content LIKE '%https://%') AND kind = 'default'
           AND id NOT IN (SELECT message_id FROM message_embeds)
         ORDER BY id DESC LIMIT 2000",
    )
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    for (id, ch) in rows {
        // Each in its own task: one odd page can't stop the rest.
        let st2 = st.clone();
        let _ = tokio::spawn(async move {
            if let Ok(ch) = db::channel(&st2.db, ch).await {
                let _ = refresh_inner(&st2, &ch, id, false).await;
            }
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// One URL's preview, from the cache when fresh.
async fn embed(st: &St, url: &str) -> Option<Embed> {
    let cached: Option<(Option<String>, i64)> = sqlx::query_as("SELECT data, fetched_at FROM embed_cache WHERE url = ?")
        .bind(url)
        .fetch_optional(&st.db)
        .await
        .ok()
        .flatten();
    if let Some((data, at)) = cached {
        let age = now_ms() - at;
        match data {
            Some(d) if age < FRESH_MS => return serde_json::from_str(&d).ok(),
            None if age < RETRY_MS => return None,
            _ => {}
        }
    }
    let e = build(url).await;
    let _ = sqlx::query(
        "INSERT INTO embed_cache (url, data, fetched_at) VALUES (?, ?, ?)
         ON CONFLICT (url) DO UPDATE SET data = excluded.data, fetched_at = excluded.fetched_at",
    )
    .bind(url)
    .bind(e.as_ref().and_then(|e| serde_json::to_string(e).ok()))
    .bind(now_ms())
    .execute(&st.db)
    .await;
    e
}

async fn build(url: &str) -> Option<Embed> {
    if let Some((id, start)) = youtube_id(url) {
        return Some(youtube(url, &id, start).await);
    }
    let page = fetch(url, false).await?;
    let blank = |kind| Embed {
        url: url.to_string(),
        kind,
        title: String::new(),
        description: String::new(),
        site: String::new(),
        author: String::new(),
        image: None,
        video: None,
        color: None,
    };
    if page.content_type.starts_with("image/") {
        return Some(Embed { image: Some(page.url.to_string()), ..blank(EmbedKind::Image) });
    }
    if page.content_type.starts_with("video/") {
        return Some(Embed { video: Some(page.url.to_string()), ..blank(EmbedKind::Video) });
    }
    let html = String::from_utf8_lossy(&page.body);
    let meta = Meta::parse(&html);
    let title = meta.get(&["og:title", "twitter:title"]).or_else(|| meta.title.clone()).unwrap_or_default();
    let description = meta.get(&["og:description", "twitter:description", "description"]).unwrap_or_default();
    let image = meta
        .get(&["og:image:secure_url", "og:image", "twitter:image", "twitter:image:src"])
        .and_then(|i| page.url.join(&i).ok())
        .filter(|u| matches!(u.scheme(), "https" | "http"))
        .map(|u| u.to_string());
    if title.is_empty() && description.is_empty() && image.is_none() {
        return None;
    }
    Some(Embed {
        title: clip(&title, 256),
        description: clip(&description, 350),
        site: clip(&meta.get(&["og:site_name"]).unwrap_or_else(|| page.url.host_str().unwrap_or_default().trim_start_matches("www.").to_string()), 100),
        image,
        color: meta.get(&["theme-color"]).filter(|c| c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit())),
        ..blank(EmbedKind::Link)
    })
}

/// A YouTube link's video id (11 characters) and start time in seconds.
pub fn youtube_id(url: &str) -> Option<(String, Option<u32>)> {
    let u = Url::parse(url).ok()?;
    let host = u.host_str()?.trim_start_matches("www.").trim_start_matches("m.").trim_start_matches("music.");
    let mut path = u.path_segments()?.filter(|s| !s.is_empty());
    let id = match host {
        "youtu.be" => path.next()?.to_string(),
        "youtube.com" | "youtube-nocookie.com" => match path.next()? {
            "watch" => u.query_pairs().find(|(k, _)| k == "v")?.1.into_owned(),
            "shorts" | "embed" | "live" | "v" => path.next()?.to_string(),
            _ => return None,
        },
        _ => return None,
    };
    if id.len() != 11 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    // t=90, t=1m30s, start=90
    let start = u.query_pairs().find(|(k, _)| k == "t" || k == "start").and_then(|(_, v)| {
        let v = v.trim_end_matches('s');
        v.parse::<u32>().ok().or_else(|| {
            let (m, s) = v.split_once('m')?;
            Some(m.parse::<u32>().ok()? * 60 + s.parse::<u32>().unwrap_or(0))
        })
    });
    Some((id, start))
}

async fn youtube(url: &str, id: &str, start: Option<u32>) -> Embed {
    // oEmbed gives the title and channel; the card works without it.
    let oembed = format!("https://www.youtube.com/oembed?format=json&url=https://www.youtube.com/watch?v={id}");
    let info: serde_json::Value = match fetch(&oembed, true).await {
        Some(p) => serde_json::from_slice(&p.body).unwrap_or_default(),
        None => serde_json::Value::Null,
    };
    let text = |k: &str| info.get(k).and_then(|v| v.as_str()).map(|s| clip(s, 256)).unwrap_or_default();
    let start = start.map(|s| format!("&start={s}")).unwrap_or_default();
    Embed {
        url: url.to_string(),
        kind: EmbedKind::Youtube,
        title: text("title"),
        description: String::new(),
        site: "YouTube".into(),
        author: text("author_name"),
        image: Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")),
        video: Some(format!("https://www.youtube-nocookie.com/embed/{id}?autoplay=1{start}")),
        color: Some("#ff0033".into()),
    }
}

fn clip(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max { s } else { s.chars().take(max - 1).collect::<String>() + "…" }
}

// ---------------------------------------------------------------- fetching, carefully

struct Page {
    url: Url,
    content_type: String,
    body: Vec<u8>,
}

/// Addresses we must never be tricked into calling: loopback, private, link-local, CGNAT, multicast, …
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            !(a.is_private()
                || a.is_loopback()
                || a.is_link_local()
                || a.is_broadcast()
                || a.is_documentation()
                || a.is_unspecified()
                || o[0] == 0
                || o[0] >= 224
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (o[1] & 0xfe) == 18))
        }
        IpAddr::V6(a) => {
            if let Some(v4) = a.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s0 = a.segments()[0];
            !(a.is_loopback() || a.is_unspecified() || a.is_multicast() || (s0 & 0xfe00) == 0xfc00 || (s0 & 0xffc0) == 0xfe80 || s0 == 0x2001 && a.segments()[1] == 0x0db8)
        }
    }
}

/// GET a page (HTML, or JSON when `json`), or learn that it's a picture / video, without ever reaching a
/// private address.
async fn fetch(url: &str, json: bool) -> Option<Page> {
    let mut url = Url::parse(url).ok()?;
    for _ in 0..5 {
        if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
            return None;
        }
        let host = url.host_str()?.to_string();
        let port = url.port_or_known_default()?;
        if port != 80 && port != 443 {
            return None;
        }
        let addrs: Vec<SocketAddr> = tokio::time::timeout(Duration::from_secs(3), tokio::net::lookup_host((host.as_str(), port)))
            .await
            .ok()?
            .ok()?
            .collect();
        if addrs.is_empty() || !addrs.iter().all(|a| is_public(a.ip())) {
            return None;
        }
        // Connect to exactly the address we checked.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .resolve(&host, addrs[0])
            .user_agent(UA)
            .build()
            .ok()?;
        let accept = if json { "application/json" } else { "text/html,application/xhtml+xml;q=0.9,image/*;q=0.8,video/*;q=0.7" };
        let mut res = client.get(url.clone()).header("Accept", accept).header("Accept-Language", "en").send().await.ok()?;
        if res.status().is_redirection() {
            let next = res.headers().get("location")?.to_str().ok()?;
            url = url.join(next).ok()?;
            continue;
        }
        if !res.status().is_success() {
            return None;
        }
        let content_type = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default().to_ascii_lowercase();
        if content_type.starts_with("image/") || content_type.starts_with("video/") {
            return Some(Page { url, content_type, body: vec![] });
        }
        let wanted = if json { content_type.contains("json") } else { content_type.contains("html") };
        if !wanted {
            return None;
        }
        let mut body = Vec::new();
        while let Ok(Some(chunk)) = res.chunk().await {
            body.extend_from_slice(&chunk);
            if body.len() >= MAX_BODY {
                body.truncate(MAX_BODY);
                break;
            }
        }
        return Some(Page { url, content_type, body });
    }
    None
}

// ---------------------------------------------------------------- <meta> tags without an HTML parser

struct Meta {
    tags: Vec<(String, String)>,
    title: Option<String>,
}

impl Meta {
    fn parse(html: &str) -> Meta {
        // Only the <head> matters; stop at <body> so a huge page isn't scanned.
        let lower = html.to_ascii_lowercase();
        let head_end = lower.find("<body").unwrap_or(lower.len());
        let (html, lower) = (&html[..head_end], &lower[..head_end]);
        let mut tags = vec![];
        let mut from = 0;
        while let Some(start) = lower[from..].find("<meta") {
            let start = from + start;
            let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
            let attrs = attributes(&html[start + 5..end]);
            let key = attrs.iter().find(|(k, _)| k == "property" || k == "name").map(|(_, v)| v.to_ascii_lowercase());
            let content = attrs.iter().find(|(k, _)| k == "content").map(|(_, v)| v.clone());
            if let (Some(k), Some(v)) = (key, content) {
                tags.push((k, decode_entities(&v)));
            }
            from = end;
        }
        let title = lower.find("<title").and_then(|s| {
            let open = s + lower[s..].find('>')? + 1;
            let close = open + lower[open..].find("</title")?;
            Some(decode_entities(html[open..close].trim()))
        });
        Meta { tags, title }
    }

    fn get(&self, keys: &[&str]) -> Option<String> {
        keys.iter().find_map(|k| self.tags.iter().find(|(t, v)| t == k && !v.trim().is_empty()).map(|(_, v)| v.trim().to_string()))
    }
}

/// `name="value" other='x' flag` → [(name, value), …], names lowercased.
fn attributes(s: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if !(c.is_ascii_alphabetic()) {
            continue;
        }
        let mut name_end = i + c.len_utf8();
        while let Some(&(j, d)) = chars.peek() {
            if d.is_ascii_alphanumeric() || d == '-' || d == ':' || d == '_' {
                name_end = j + d.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        let name = s[i..name_end].to_ascii_lowercase();
        while chars.peek().is_some_and(|&(_, d)| d.is_whitespace()) {
            chars.next();
        }
        if chars.peek().map(|&(_, d)| d) != Some('=') {
            continue;
        }
        chars.next();
        while chars.peek().is_some_and(|&(_, d)| d.is_whitespace()) {
            chars.next();
        }
        let value = match chars.peek().copied() {
            Some((j, q @ ('"' | '\''))) => {
                chars.next();
                let end = s[j + 1..].find(q).map_or(s.len(), |e| j + 1 + e);
                while chars.peek().is_some_and(|&(k, _)| k <= end) {
                    chars.next();
                }
                s[j + 1..end.min(s.len())].to_string()
            }
            Some((j, _)) => {
                let end = s[j..].find(|c: char| c.is_whitespace() || c == '/').map_or(s.len(), |e| j + e);
                while chars.peek().is_some_and(|&(k, _)| k < end) {
                    chars.next();
                }
                s[j..end].to_string()
            }
            None => String::new(),
        };
        out.push((name, value));
    }
    out
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        // Look for the ";" within a few characters (by character: a byte cut could split "—").
        let Some(semi) = tail.char_indices().take(12).find(|&(_, c)| c == ';').map(|(i, _)| i) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links() {
        assert_eq!(urls("look https://youtu.be/T7HfRLiC0nI!", None), vec!["https://youtu.be/T7HfRLiC0nI"]);
        assert_eq!(urls("[site](https://example.com/a_(b)) and https://example.com/x.", None), vec!["https://example.com/a_(b)", "https://example.com/x"]);
        assert!(urls("no preview <https://example.com> and `https://example.com/code`", None).is_empty());
        assert!(urls("https://cloud.example.com/f/12", Some("cloud.example.com")).is_empty());
    }

    #[test]
    fn youtube_links() {
        assert_eq!(youtube_id("https://youtu.be/T7HfRLiC0nI"), Some(("T7HfRLiC0nI".into(), None)));
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=T7HfRLiC0nI&t=1m30s"), Some(("T7HfRLiC0nI".into(), Some(90))));
        assert_eq!(youtube_id("https://youtube.com/shorts/T7HfRLiC0nI?si=x"), Some(("T7HfRLiC0nI".into(), None)));
        assert_eq!(youtube_id("https://www.youtube.com/@channel"), None);
        assert_eq!(youtube_id("https://example.com/watch?v=T7HfRLiC0nI"), None);
    }

    #[test]
    fn private_addresses_are_refused() {
        for ip in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fc00::1", "fe80::1", "::ffff:127.0.0.1"] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["9.9.9.9", "142.250.185.78", "2a00:1450:4001:80b::200e"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn reads_meta_tags() {
        let html = r##"<html><head><title>Fallback &amp; title</title>
            <meta property="og:title" content="The Water Co. &#8211; trailer">
            <meta name='description' content='Pool party co-op'>
            <meta property=og:image content="/img/cover.jpg"/>
            <meta name="theme-color" content="#1a8a8a"></head><body><meta property="og:title" content="ignored">"##;
        let m = Meta::parse(html);
        assert_eq!(m.get(&["og:title"]).as_deref(), Some("The Water Co. – trailer"));
        assert_eq!(m.get(&["og:description", "description"]).as_deref(), Some("Pool party co-op"));
        assert_eq!(m.get(&["og:image"]).as_deref(), Some("/img/cover.jpg"));
        assert_eq!(m.title.as_deref(), Some("Fallback & title"));
        assert_eq!(m.tags.iter().filter(|(k, _)| k == "og:title").count(), 1, "stops at <body>");
        // A bare "&" before multi-byte text (this crashed the first backfill on a real page).
        assert_eq!(decode_entities("Tom &— Jerry &amp; co &#x27;"), "Tom &— Jerry & co '");
    }
}
