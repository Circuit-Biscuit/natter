//! REST client over `fetch`. The server URL and token live here, outside the reactive graph.

use std::cell::RefCell;

use laklak_proto::{ApiError, Attachment};
use serde::{Serialize, de::DeserializeOwned};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, Request, RequestInit, Response};

thread_local! {
    static CONF: RefCell<(String, Option<String>)> = const { RefCell::new((String::new(), None)) };
}

/// Server base URL without trailing slash; "" means the page's own origin.
pub fn set_server(base: &str) {
    CONF.with(|c| c.borrow_mut().0 = base.trim().trim_end_matches('/').to_string());
}

/// Where the web app was served from (works under a sub-path like `/laklak/`); empty in the desktop app,
/// which asks for a server address instead.
pub fn default_server() -> String {
    if crate::desktop::is_tauri() {
        return String::new();
    }
    crate::util::window()
        .document()
        .and_then(|d| d.base_uri().ok().flatten())
        .map(|b| b.trim_end_matches('/').to_string())
        .unwrap_or_default()
}

pub fn server() -> String {
    CONF.with(|c| c.borrow().0.clone())
}

pub fn set_token(token: Option<String>) {
    CONF.with(|c| c.borrow_mut().1 = token);
}

pub fn token() -> Option<String> {
    CONF.with(|c| c.borrow().1.clone())
}

/// Absolute URL for a server path such as `/files/...` (avatars, attachments).
pub fn url(path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("blob:") {
        path.to_string()
    } else {
        format!("{}{}", server(), path)
    }
}

/// WebSocket URL of the gateway. With `identify` the token goes in the first frame instead (servers that say
/// `gateway_identify`), so it never shows up in a URL or a proxy's log.
pub fn gateway_url(identify: bool) -> String {
    let base = server();
    let base = if base.is_empty() { crate::util::window().location().origin().unwrap_or_default() } else { base };
    let ws = base.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
    if identify { format!("{ws}/gateway") } else { format!("{ws}/gateway?token={}", token().unwrap_or_default()) }
}

#[derive(Debug, Clone)]
pub struct Error {
    pub status: u16,
    pub message: String,
    /// The server's `ApiError::code`, for errors the app acts on (`totp_required`).
    pub code: Option<String>,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn js_err(e: JsValue) -> Error {
    let message = e
        .dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .unwrap_or_else(|| "Network error".into());
    Error { status: 0, message: format!("Can't reach the server ({message})"), code: None }
}

async fn send(method: &str, path: &str, body: Option<JsValue>, headers: &[(&str, &str)]) -> Result<Response, Error> {
    let init = RequestInit::new();
    init.set_method(method);
    let h = Headers::new().map_err(js_err)?;
    if let Some(t) = token() {
        h.set("Authorization", &format!("Bearer {t}")).map_err(js_err)?;
    }
    for (k, v) in headers {
        h.set(k, v).map_err(js_err)?;
    }
    init.set_headers(&h);
    if let Some(b) = body {
        init.set_body(&b);
    }
    let req = Request::new_with_str_and_init(&url(path), &init).map_err(js_err)?;
    let resp: Response = JsFuture::from(crate::util::window().fetch_with_request(&req))
        .await
        .map_err(js_err)?
        .unchecked_into();
    if resp.ok() {
        return Ok(resp);
    }
    let status = resp.status();
    let text = match resp.text() {
        Ok(p) => JsFuture::from(p).await.ok().and_then(|v| v.as_string()).unwrap_or_default(),
        Err(_) => String::new(),
    };
    let parsed = serde_json::from_str::<ApiError>(&text).ok();
    let code = parsed.as_ref().and_then(|e| e.code.clone());
    let message = parsed.map(|e| e.error).ok_or(()).unwrap_or_else(|_| {
        match status {
            401 => "Your session has ended. Please sign in again.".into(),
            413 => "That file is too large for this server.".into(),
            429 => "Slow down a little.".into(),
            _ => format!("Server error ({status})"),
        }
    });
    if status == 401 && token().is_some() {
        crate::store::session_expired();
    }
    Err(Error { status, message, code })
}

async fn parse<R: DeserializeOwned>(resp: Response) -> Result<R, Error> {
    let text = JsFuture::from(resp.text().map_err(js_err)?).await.map_err(js_err)?.as_string().unwrap_or_default();
    let text = if text.is_empty() { "null" } else { &text };
    serde_json::from_str(text).map_err(|e| Error { status: 0, message: format!("Unexpected reply: {e}"), code: None })
}

pub async fn get<R: DeserializeOwned>(path: &str) -> Result<R, Error> {
    parse(send("GET", path, None, &[]).await?).await
}

pub async fn call<B: Serialize, R: DeserializeOwned>(method: &str, path: &str, body: &B) -> Result<R, Error> {
    let json = serde_json::to_string(body).unwrap_or_default();
    parse(send(method, path, Some(json.into()), &[("Content-Type", "application/json")]).await?).await
}

/// Body-less request whose reply we don't need (PUT reaction, DELETE ...).
pub async fn act(method: &str, path: &str) -> Result<(), Error> {
    send(method, path, None, &[]).await.map(|_| ())
}

/// Upload a file. `channel` tells the server which Nextcloud folder it belongs in.
pub async fn upload(blob: &web_sys::Blob, filename: &str, channel: Option<laklak_proto::Id>) -> Result<Attachment, Error> {
    let ty = blob.type_();
    let ty = if ty.is_empty() { "application/octet-stream".to_string() } else { ty };
    let name = js_sys::encode_uri_component(filename).as_string().unwrap_or_default();
    let ch = channel.map(|c| c.to_string()).unwrap_or_default();
    let mut headers = vec![("Content-Type", ty.as_str()), ("X-Filename", name.as_str())];
    if channel.is_some() {
        headers.push(("X-Channel-Id", ch.as_str()));
    }
    parse(send("POST", "/api/uploads", Some(blob.into()), &headers).await?).await
}

pub fn enc(s: &str) -> String {
    js_sys::encode_uri_component(s).as_string().unwrap_or_default()
}
