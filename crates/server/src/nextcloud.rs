//! Nextcloud: sign-in (Login Flow v2), profiles, and file storage in the user's own Nextcloud.
//! Works with any Nextcloud; nothing here is specific to ours.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use natter_proto::{Attachment, AuthResp, Event, Id, NcFile, NcLoginPoll, NcLoginStart, NcShare};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::auth::Auth;
use crate::messages::{NewUpload, image_dims, insert_upload, new_token, store_local};
use crate::{ApiErr, R, St, bad, db, forbidden, not_found, now_ms, spaces};

/// Path segments: keep the characters WebDAV is happy with, encode the rest.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

const NC_ONLY_PASSWORD: &str = "!nextcloud"; // not a PHC string: local password sign-in can never match

pub struct Nc {
    pub base: String,
    group: Option<String>,
    folder: String,
    autojoin: Option<Id>,
    http: reqwest::Client,
    /// Pending sign-ins: our flow id → (Nextcloud poll token, started).
    flows: Mutex<HashMap<String, (String, Instant)>>,
}

/// A Nextcloud login: (login name, app password, Nextcloud user id).
pub struct Creds {
    login: String,
    password: String,
    uid: String,
}

pub struct Stored {
    pub url: String,
    pub link: String,
    /// The public link we created (to remove on delete); `None` when an existing link was reused.
    pub share_id: Option<String>,
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, SEGMENT).to_string()
}

fn enc_path(path: &str) -> String {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(enc)
        .collect::<Vec<_>>()
        .join("/")
}

fn nc_err(e: impl std::fmt::Display) -> ApiErr {
    tracing::warn!("nextcloud: {e}");
    ApiErr::new(
        StatusCode::BAD_GATEWAY,
        "Nextcloud didn't answer as expected",
    )
}

impl Nc {
    pub fn new(
        base: &str,
        group: Option<String>,
        folder: String,
        autojoin: Option<Id>,
    ) -> anyhow::Result<Nc> {
        let http = reqwest::Client::builder()
            .user_agent("Natter")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Nc {
            base: base.trim_end_matches('/').to_string(),
            group,
            folder,
            autojoin,
            http,
            flows: Mutex::default(),
        })
    }

    fn ocs(&self, method: Method, path: &str, c: &Creds) -> RequestBuilder {
        self.http
            .request(method, format!("{}/ocs/v2.php/{path}", self.base))
            .basic_auth(&c.login, Some(&c.password))
            .header("OCS-APIRequest", "true")
            .header("Accept", "application/json")
    }

    fn dav(&self, method: Method, path: &str, c: &Creds) -> RequestBuilder {
        self.http
            .request(
                method,
                format!(
                    "{}/remote.php/dav/files/{}/{}",
                    self.base,
                    enc(&c.uid),
                    enc_path(path)
                ),
            )
            .basic_auth(&c.login, Some(&c.password))
    }

    /// `ocs.data` of an OCS JSON reply, or an error.
    async fn ocs_data(rb: RequestBuilder) -> Result<Value, String> {
        let res = rb.send().await.map_err(|e| e.to_string())?;
        let status = res.status();
        let body: Value = res.json().await.map_err(|e| format!("{status}: {e}"))?;
        if !status.is_success() {
            return Err(format!("{status}: {}", body["ocs"]["meta"]["message"]));
        }
        Ok(body["ocs"]["data"].clone())
    }

    async fn mkcol(&self, path: &str, c: &Creds) -> Result<(), String> {
        let res = self
            .dav(Method::from_bytes(b"MKCOL").unwrap(), path, c)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        match res.status().as_u16() {
            201 | 405 => Ok(()), // created, or already there
            s => Err(format!("MKCOL {path}: {s}")),
        }
    }

    /// PUT without overwriting: `name.ext`, then `name (2).ext`, ... Returns the path used.
    async fn put_new(
        &self,
        dir: &str,
        filename: &str,
        body: &[u8],
        c: &Creds,
    ) -> Result<String, String> {
        let (stem, ext) = match filename.rfind('.') {
            Some(i) if i > 0 => (&filename[..i], &filename[i..]),
            _ => (filename, ""),
        };
        for n in 1..=20 {
            let name = if n == 1 {
                filename.to_string()
            } else {
                format!("{stem} ({n}){ext}")
            };
            let path = format!("{dir}/{name}");
            let res = self
                .dav(Method::PUT, &path, c)
                .header("If-None-Match", "*")
                .timeout(Duration::from_secs(600))
                .body(body.to_vec())
                .send()
                .await
                .map_err(|e| e.to_string())?;
            match res.status().as_u16() {
                201 | 204 => return Ok(path),
                412 => continue, // taken
                s => return Err(format!("PUT {path}: {s}")),
            }
        }
        Err(format!("no free name for {filename} in {dir}"))
    }

    /// A read-only public link for `path`, reusing one that exists. Returns (share page, share id if we made it).
    async fn public_link(&self, path: &str, c: &Creds) -> Result<(String, Option<String>), String> {
        let existing = Self::ocs_data(
            self.ocs(Method::GET, "apps/files_sharing/api/v1/shares", c)
                .query(&[("format", "json"), ("path", path), ("reshares", "false")]),
        )
        .await
        .unwrap_or(Value::Null);
        if let Some(url) = existing
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["share_type"].as_i64() == Some(3) && s["password"].is_null())
            .and_then(|s| s["url"].as_str())
        {
            return Ok((url.to_string(), None));
        }
        let data = Self::ocs_data(
            self.ocs(
                Method::POST,
                "apps/files_sharing/api/v1/shares?format=json",
                c,
            )
            .form(&[("path", path), ("shareType", "3"), ("permissions", "1")]),
        )
        .await?;
        let url = data["url"].as_str().ok_or("share without url")?.to_string();
        let id = match &data["id"] {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        Ok((url, Some(id)))
    }

    async fn delete_share(&self, id: &str, c: &Creds) {
        let res = self
            .ocs(
                Method::DELETE,
                &format!("apps/files_sharing/api/v1/shares/{}", enc(id)),
                c,
            )
            .send()
            .await;
        if let Err(e) = res {
            tracing::warn!("nextcloud unshare {id}: {e}");
        }
    }
}

fn nc_of(st: &St) -> R<&Nc> {
    st.nc
        .as_ref()
        .ok_or_else(|| not_found("Nextcloud sign-in (not set up on this server)"))
}

async fn session_creds(st: &St, session: Id) -> R<Option<Creds>> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT s.nc_login, s.nc_app_password, u.nc_user_id FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.id = ?",
    )
    .bind(session)
    .fetch_optional(&st.db)
    .await?;
    Ok(match row {
        Some((Some(login), Some(password), Some(uid))) => Some(Creds {
            login,
            password,
            uid,
        }),
        _ => None,
    })
}

async fn any_creds_of(st: &St, user: Id) -> Option<Creds> {
    let session: Option<Id> =
        sqlx::query_scalar("SELECT id FROM sessions WHERE user_id = ? AND nc_app_password IS NOT NULL ORDER BY last_seen DESC")
            .bind(user)
            .fetch_optional(&st.db)
            .await
            .ok()
            .flatten();
    session_creds(st, session?).await.ok().flatten()
}

async fn require_creds<'a>(st: &'a St, auth: &Auth) -> R<(&'a Nc, Creds)> {
    let nc = nc_of(st)?;
    let creds = session_creds(st, auth.session)
        .await?
        .ok_or_else(|| forbidden("sign in with Nextcloud to use your Nextcloud files"))?;
    Ok((nc, creds))
}

/// Does this session hold a Nextcloud login?
pub async fn is_nc_session(st: &St, session: Id) -> bool {
    st.nc.is_some() && matches!(session_creds(st, session).await, Ok(Some(_)))
}

// ---------------------------------------------------------------- sign-in

pub async fn login_start(State(st): State<St>) -> R<Json<NcLoginStart>> {
    let nc = nc_of(&st)?;
    {
        let mut flows = nc.flows.lock().unwrap();
        flows.retain(|_, (_, at)| at.elapsed() < Duration::from_secs(1200));
        if flows.len() >= 1000 {
            return Err(ApiErr::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too many sign-ins in progress, try again soon",
            ));
        }
    }
    let res = nc
        .http
        .post(format!("{}/index.php/login/v2", nc.base))
        .send()
        .await
        .map_err(nc_err)?;
    let body: Value = res
        .error_for_status()
        .map_err(nc_err)?
        .json()
        .await
        .map_err(nc_err)?;
    let (Some(token), Some(login_url)) = (body["poll"]["token"].as_str(), body["login"].as_str())
    else {
        return Err(nc_err("login/v2 reply without poll token or login url"));
    };
    let flow = hex::encode(rand::random::<[u8; 16]>());
    nc.flows
        .lock()
        .unwrap()
        .insert(flow.clone(), (token.to_string(), Instant::now()));
    Ok(Json(NcLoginStart {
        login_url: login_url.to_string(),
        flow,
    }))
}

pub async fn login_poll(
    State(st): State<St>,
    Json(req): Json<NcLoginPoll>,
) -> R<Json<Option<AuthResp>>> {
    let nc = nc_of(&st)?;
    let token = match nc.flows.lock().unwrap().get(&req.flow) {
        Some((t, at)) if at.elapsed() < Duration::from_secs(1200) => t.clone(),
        _ => return Err(not_found("sign-in (it expired, start again)")),
    };
    let res = nc
        .http
        .post(format!("{}/index.php/login/v2/poll", nc.base))
        .form(&[("token", token.as_str())])
        .send()
        .await
        .map_err(nc_err)?;
    if res.status() == StatusCode::NOT_FOUND {
        return Ok(Json(None)); // not approved yet
    }
    let body: Value = res
        .error_for_status()
        .map_err(nc_err)?
        .json()
        .await
        .map_err(nc_err)?;
    nc.flows.lock().unwrap().remove(&req.flow);
    let (Some(login), Some(password)) = (body["loginName"].as_str(), body["appPassword"].as_str())
    else {
        return Err(nc_err("poll reply without loginName/appPassword"));
    };
    let mut creds = Creds {
        login: login.into(),
        password: password.into(),
        uid: String::new(),
    };

    let me = Nc::ocs_data(nc.ocs(Method::GET, "cloud/user?format=json", &creds))
        .await
        .map_err(nc_err)?;
    let Some(uid) = me["id"].as_str().map(str::to_string) else {
        return Err(nc_err("cloud/user without id"));
    };
    creds.uid = uid.clone();
    if let Some(group) = &nc.group {
        let member = me["groups"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|g| g.as_str() == Some(group.as_str()));
        if !member {
            forget_app_password(&st, creds.login.clone(), creds.password.clone());
            return Err(forbidden(format!(
                "your Nextcloud account isn't in the group \"{group}\""
            )));
        }
    }
    let display = me["display-name"]
        .as_str()
        .or(me["displayname"].as_str())
        .unwrap_or(&uid)
        .to_string();
    let email = me["email"].as_str().map(str::to_string);
    let user = upsert_user(&st, nc, &creds, &display, email.as_deref()).await?;
    let token =
        crate::auth::create_session(&st, user, req.device, Some((&creds.login, &creds.password)))
            .await?;
    Ok(Json(Some(AuthResp {
        token,
        user: db::user(&st.db, user).await?,
    })))
}

/// Natter username for a Nextcloud account: its id, or (for OIDC hash ids) the e-mail's local part.
fn username_base(uid: &str, email: Option<&str>, display: &str) -> String {
    let hashy = uid.len() >= 32 && uid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
    let src = if !hashy {
        uid
    } else if let Some(local) = email
        .and_then(|e| e.split('@').next())
        .filter(|l| !l.is_empty())
    {
        local
    } else {
        display
    };
    let mut s: String = src
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .take(28)
        .collect();
    while s.len() < 2 {
        s.push('_');
    }
    s
}

async fn upsert_user(st: &St, nc: &Nc, creds: &Creds, display: &str, email: Option<&str>) -> R<Id> {
    let display: String = display
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(32)
        .collect();
    let display = if display.is_empty() {
        creds.uid.clone()
    } else {
        display
    };
    let existing: Option<(Id, String)> =
        sqlx::query_as("SELECT id, display_name FROM users WHERE nc_user_id = ?")
            .bind(&creds.uid)
            .fetch_optional(&st.db)
            .await?;
    let (id, is_new, mut changed) = match existing {
        Some((id, name)) => {
            let changed = name != display;
            if changed {
                sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
                    .bind(&display)
                    .bind(id)
                    .execute(&st.db)
                    .await?;
            }
            (id, false, changed)
        }
        None => {
            let base = username_base(&creds.uid, email, &display);
            let id = st.id();
            let mut n = 1;
            loop {
                let username = if n == 1 {
                    base.clone()
                } else {
                    format!("{base}{n}")
                };
                let res = sqlx::query(
                    "INSERT INTO users (id, username, display_name, password_hash, nc_user_id, admin, created_at)
                     VALUES (?, ?, ?, ?, ?, (SELECT COUNT(*) = 0 FROM users), ?)",
                )
                .bind(id)
                .bind(&username)
                .bind(&display)
                .bind(NC_ONLY_PASSWORD)
                .bind(&creds.uid)
                .bind(now_ms())
                .execute(&st.db)
                .await;
                match res {
                    Err(sqlx::Error::Database(e)) if e.is_unique_violation() && n < 1000 => n += 1,
                    r => {
                        r?;
                        break;
                    }
                }
            }
            (id, true, false)
        }
    };
    changed |= sync_avatar(st, nc, creds, id).await.unwrap_or_else(|e| {
        tracing::warn!("nextcloud avatar of {}: {e:?}", creds.uid);
        false
    });
    if is_new {
        if let Some(space) = nc.autojoin
            && let Err(e) = spaces::admit(st, id, space).await
        {
            tracing::warn!("autojoin space {space}: {e:?}");
        }
    } else if changed {
        let user = db::user(&st.db, id).await?;
        st.hub.send(
            &db::user_audience(&st.db, id).await?,
            &Event::UserUpdate(user),
        );
    }
    Ok(id)
}

/// Copy the Nextcloud avatar if the person set one there. Returns whether it changed.
async fn sync_avatar(st: &St, nc: &Nc, creds: &Creds, user: Id) -> R<bool> {
    let res = nc
        .http
        .get(format!(
            "{}/index.php/avatar/{}/512",
            nc.base,
            enc(&creds.uid)
        ))
        .basic_auth(&creds.login, Some(&creds.password))
        .send()
        .await
        .map_err(nc_err)?;
    let custom = res
        .headers()
        .get("X-NC-IsCustomAvatar")
        .is_some_and(|v| v.as_bytes() == b"1");
    if !res.status().is_success() || !custom {
        return Ok(false);
    }
    let ct = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/png")
        .to_string();
    let bytes = res.bytes().await.map_err(nc_err)?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Ok(false);
    }
    let (width, height) = image_dims(&ct, &bytes);
    if width.is_none() {
        return Ok(false);
    }
    // Same picture as last time? Then nothing to do.
    let current: Option<String> = sqlx::query_scalar(
        "SELECT up.token FROM users u JOIN uploads up ON up.id = u.avatar_id WHERE u.id = ? AND up.url IS NULL",
    )
    .bind(user)
    .fetch_optional(&st.db)
    .await?;
    if let Some(token) = &current
        && let Ok(old) = tokio::fs::read(st.cfg.data.join("files").join(token)).await
        && Sha256::digest(&old) == Sha256::digest(&bytes)
    {
        return Ok(false);
    }
    let token = store_local(st, &bytes).await?;
    let up = NewUpload {
        owner: user,
        filename: "avatar".into(),
        content_type: ct,
        size: bytes.len() as u64,
        width,
        height,
        url: None,
        link: None,
        nc_share_id: None,
        used: true,
    };
    let att = insert_upload(st, &token, up).await?;
    let old: Option<Id> = sqlx::query_scalar("SELECT avatar_id FROM users WHERE id = ?")
        .bind(user)
        .fetch_one(&st.db)
        .await?;
    crate::auth::release_upload(st, old).await?;
    sqlx::query("UPDATE users SET avatar_id = ? WHERE id = ?")
        .bind(att.id)
        .bind(user)
        .execute(&st.db)
        .await?;
    Ok(true)
}

/// Best effort: delete an app password in Nextcloud (sign-out, revoked device).
pub fn forget_app_password(st: &St, login: String, password: String) {
    let Some(nc) = &st.nc else { return };
    let rb = nc.ocs(
        Method::DELETE,
        "core/apppassword",
        &Creds {
            login,
            password,
            uid: String::new(),
        },
    );
    tokio::spawn(async move {
        if let Err(e) = rb.send().await {
            tracing::warn!("nextcloud app password delete: {e}");
        }
    });
}

/// Best effort, in the background: remove a public link we created for an upload.
pub fn unshare_later(st: &St, owner: Id, share_id: String) {
    if st.nc.is_none() {
        return;
    }
    let st = st.clone();
    tokio::spawn(async move {
        let Some(nc) = &st.nc else { return };
        match any_creds_of(&st, owner).await {
            Some(c) => nc.delete_share(&share_id, &c).await,
            None => {
                tracing::info!("share {share_id} kept: its owner has no Nextcloud session left")
            }
        }
    });
}

// ---------------------------------------------------------------- files

fn folder_name(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "Uploads".into() } else { s }
}

/// Put an upload into the sender's Nextcloud and share it. `None` (with a warning) when this isn't a
/// Nextcloud session or Nextcloud refused: the caller then stores the file locally.
pub async fn store_upload(
    st: &St,
    auth: &Auth,
    channel: Option<Id>,
    filename: &str,
    _content_type: &str,
    body: &[u8],
) -> Option<Stored> {
    let nc = st.nc.as_ref()?;
    let creds = session_creds(st, auth.session).await.ok()??;
    let sub = match channel {
        Some(c) => match db::access(&st.db, auth.user, c).await {
            Ok(acc) => match acc.ch.space_id {
                Some(space) => db::space(&st.db, space)
                    .await
                    .map(|s| folder_name(&s.name))
                    .unwrap_or("Uploads".into()),
                None => "Direct messages".into(),
            },
            Err(_) => "Uploads".into(),
        },
        None => "Uploads".into(),
    };
    let folder = folder_name(&nc.folder);
    let result = async {
        nc.mkcol(&folder, &creds).await?;
        let dir = format!("{folder}/{sub}");
        nc.mkcol(&dir, &creds).await?;
        let path = nc.put_new(&dir, filename, body, &creds).await?;
        let name = path.rsplit('/').next().unwrap_or(filename).to_string();
        let (link, made) = nc.public_link(&format!("/{path}"), &creds).await?;
        Ok::<_, String>(Stored {
            url: format!("{}/download/{}", link.trim_end_matches('/'), enc(&name)),
            link,
            share_id: made,
        })
    }
    .await;
    match result {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(
                "nextcloud upload for {}, storing locally instead: {e}",
                creds.uid
            );
            None
        }
    }
}

#[derive(Debug, Default, PartialEq)]
struct DavEntry {
    href: String,
    modified: String,
    length: Option<u64>,
    content_type: String,
    size: Option<u64>,
    dir: bool,
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Tolerant reader for WebDAV multistatus replies: matches tags by local name, whatever the namespace prefix.
fn parse_multistatus(xml: &str) -> Vec<DavEntry> {
    let mut out = vec![];
    let mut cur: Option<DavEntry> = None;
    for chunk in xml.split('<').skip(1) {
        let (tag, text) = chunk.split_once('>').unwrap_or((chunk, ""));
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name);
        if closing {
            if local == "response" {
                out.extend(cur.take());
            }
            continue;
        }
        let Some(e) = cur.as_mut() else {
            if local == "response" {
                cur = Some(DavEntry::default());
            }
            continue;
        };
        let text = unescape(text.trim());
        match local {
            "collection" => e.dir = true,
            _ if tag.ends_with('/') => {}
            "href" => e.href = text,
            "getlastmodified" => e.modified = text,
            "getcontentlength" => e.length = text.parse().ok(),
            "getcontenttype" => e.content_type = text,
            "size" => e.size = text.parse().ok(),
            _ => {}
        }
    }
    out
}

const PROPFIND: &str = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns"><d:prop><d:getlastmodified/><d:getcontentlength/><d:getcontenttype/><d:resourcetype/><oc:size/></d:prop></d:propfind>"#;

/// `/a/b`, no `..`, no control characters.
fn check_path(path: &str) -> R<String> {
    let ok = path.starts_with('/')
        && !path.split('/').any(|s| s == ".." || s == ".")
        && !path.chars().any(char::is_control)
        && path.len() <= 4096;
    if !ok {
        return Err(bad("bad path"));
    }
    Ok(path.to_string())
}

async fn propfind(nc: &Nc, c: &Creds, path: &str, depth: &str) -> R<Vec<NcFile>> {
    let res = nc
        .dav(Method::from_bytes(b"PROPFIND").unwrap(), path, c)
        .header("Depth", depth)
        .header("Content-Type", "application/xml")
        .body(PROPFIND)
        .send()
        .await
        .map_err(nc_err)?;
    match res.status().as_u16() {
        207 => {}
        404 => return Err(not_found("file")),
        s => return Err(nc_err(format!("PROPFIND {path}: {s}"))),
    }
    let xml = res.text().await.map_err(nc_err)?;
    let mut out = vec![];
    for e in parse_multistatus(&xml) {
        let href = percent_encoding::percent_decode_str(&e.href)
            .decode_utf8_lossy()
            .into_owned();
        let Some((_, rest)) = href.split_once("/remote.php/dav/files/") else {
            continue;
        };
        let rel = rest
            .split_once('/')
            .map(|(_, p)| p)
            .unwrap_or("")
            .trim_end_matches('/');
        let p = format!("/{rel}");
        out.push(NcFile {
            name: rel.rsplit('/').next().unwrap_or("").to_string(),
            dir: e.dir,
            size: if e.dir {
                e.size.unwrap_or(0)
            } else {
                e.length.or(e.size).unwrap_or(0)
            },
            modified: httpdate::parse_http_date(&e.modified)
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            content_type: if e.dir {
                "inode/directory".into()
            } else {
                e.content_type
            },
            path: p,
        });
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct FilesQuery {
    path: Option<String>,
}

pub async fn files(
    State(st): State<St>,
    auth: Auth,
    Query(q): Query<FilesQuery>,
) -> R<Json<Vec<NcFile>>> {
    let (nc, creds) = require_creds(&st, &auth).await?;
    let path = check_path(q.path.as_deref().unwrap_or("/"))?;
    let want = path.trim_end_matches('/');
    let mut list: Vec<NcFile> = propfind(nc, &creds, &path, "1")
        .await?
        .into_iter()
        .filter(|f| f.path != want && f.path != "/")
        .collect();
    list.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Json(list))
}

pub async fn share(
    State(st): State<St>,
    auth: Auth,
    Json(req): Json<NcShare>,
) -> R<Json<Attachment>> {
    let (nc, creds) = require_creds(&st, &auth).await?;
    let path = check_path(&req.path)?;
    let file = propfind(nc, &creds, &path, "0")
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| not_found("file"))?;
    if file.dir {
        return Err(bad("pick a file, not a folder"));
    }
    let (link, made) = nc.public_link(&path, &creds).await.map_err(|e| {
        tracing::warn!("nextcloud share {path}: {e}");
        ApiErr::new(
            StatusCode::BAD_GATEWAY,
            "Nextcloud wouldn't share that file (is link sharing allowed?)",
        )
    })?;
    // Dimensions from the first 256 KB, enough for every common image header.
    let (mut width, mut height) = (None, None);
    if file.content_type.starts_with("image/")
        && let Ok(res) = nc
            .dav(Method::GET, &path, &creds)
            .header("Range", "bytes=0-262143")
            .send()
            .await
        && let Ok(head) = res.bytes().await
    {
        (width, height) = image_dims(&file.content_type, &head);
    }
    let up = NewUpload {
        owner: auth.user,
        url: Some(format!(
            "{}/download/{}",
            link.trim_end_matches('/'),
            enc(&file.name)
        )),
        link: Some(link),
        nc_share_id: made,
        filename: file.name,
        content_type: if file.content_type.is_empty() {
            "application/octet-stream".into()
        } else {
            file.content_type
        },
        size: file.size,
        width,
        height,
        used: false,
    };
    Ok(Json(insert_upload(&st, &new_token(), up).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dav_and_names() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns"><d:response><d:href>/remote.php/dav/files/alice/Photos/</d:href>
<d:propstat><d:prop><d:getlastmodified>Wed, 18 Nov 2026 10:00:00 GMT</d:getlastmodified><d:resourcetype><d:collection/></d:resourcetype><oc:size>42</oc:size></d:prop></d:propstat>
<d:propstat><d:prop><d:getcontentlength/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>
<D:response><D:href>/remote.php/dav/files/alice/Photos/a%20%26%20b.jpg</D:href><D:propstat><D:prop><D:getcontentlength>7</D:getcontentlength><D:getcontenttype>image/jpeg</D:getcontenttype><D:resourcetype/></D:prop></D:propstat></D:response></d:multistatus>"#;
        let e = parse_multistatus(xml);
        assert_eq!(e.len(), 2);
        assert!(e[0].dir && e[0].size == Some(42) && e[0].length.is_none());
        assert_eq!(
            (e[1].href.as_str(), e[1].length, e[1].dir),
            (
                "/remote.php/dav/files/alice/Photos/a%20%26%20b.jpg",
                Some(7),
                false
            )
        );
        assert_eq!(username_base("Alice.K", None, "x"), "alice.k");
        assert_eq!(
            username_base("alice@example.com", None, "x"),
            "alice_example.com"
        );
        assert_eq!(
            username_base(&"ab12".repeat(16), Some("bob@example.com"), "x"),
            "bob"
        );
        assert_eq!(
            enc_path("/Natter/My Space/a b.png"),
            "Natter/My%20Space/a%20b.png"
        );
        assert!(
            check_path("/a/../b").is_err()
                && check_path("x").is_err()
                && check_path("/a b/c").is_ok()
        );
    }
}
