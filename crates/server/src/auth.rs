//! Accounts, sessions and the `Auth` extractor.

use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use axum::Json;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use natter_proto::{
    AuthResp, ChangePassword, DeleteAccount, Event, Id, LoginReq, MAX_ABOUT_LEN, RegisterReq,
    Registration, ServerInfo, SessionInfo, Status, UpdateMe, User, valid_username,
};
use sha2::{Digest, Sha256};

use crate::{
    ApiErr, R, St, bad, conflict, db, forbidden, gateway, messages, nextcloud, not_found, now_ms,
    spaces, voice,
};

/// The signed-in user behind a request.
pub struct Auth {
    pub user: Id,
    pub session: Id,
}

fn unauthorized() -> ApiErr {
    ApiErr::new(StatusCode::UNAUTHORIZED, "sign in first")
}

impl FromRequestParts<St> for Auth {
    type Rejection = ApiErr;

    async fn from_request_parts(parts: &mut Parts, st: &St) -> R<Auth> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(unauthorized)?;
        session_for_token(st, token).await?.ok_or_else(unauthorized)
    }
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub async fn session_for_token(st: &St, token: &str) -> R<Option<Auth>> {
    let row: Option<(Id, Id, i64)> =
        sqlx::query_as("SELECT id, user_id, last_seen FROM sessions WHERE token_hash = ?")
            .bind(hash_token(token))
            .fetch_optional(&st.db)
            .await?;
    let Some((session, user, last_seen)) = row else {
        return Ok(None);
    };
    let now = now_ms();
    if now - last_seen > 60_000 {
        sqlx::query("UPDATE sessions SET last_seen = ? WHERE id = ?")
            .bind(now)
            .bind(session)
            .execute(&st.db)
            .await?;
    }
    Ok(Some(Auth { user, session }))
}

/// New session token. `nc` = (login name, app password) for Nextcloud sign-ins.
pub(crate) async fn create_session(
    st: &St,
    user: Id,
    device: Option<String>,
    nc: Option<(&str, &str)>,
) -> R<String> {
    let token = hex::encode(rand::random::<[u8; 32]>());
    let device = device
        .map(|d| d.trim().chars().take(64).collect::<String>())
        .filter(|d| !d.is_empty());
    let now = now_ms();
    sqlx::query(
        "INSERT INTO sessions (id, user_id, token_hash, device, created_at, last_seen, nc_login, nc_app_password)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
        .bind(st.id())
        .bind(user)
        .bind(hash_token(&token))
        .bind(device.unwrap_or_else(|| "Unknown device".into()))
        .bind(now)
        .bind(now)
        .bind(nc.map(|n| n.0))
        .bind(nc.map(|n| n.1))
        .execute(&st.db)
        .await?;
    Ok(token)
}

/// Trimmed, non-empty, at most `max` characters, no control characters.
pub fn clean(s: &str, max: usize, what: &str) -> R<String> {
    let s = s.trim();
    if s.is_empty() {
        return Err(bad(format!("{what} can't be empty")));
    }
    if s.chars().count() > max {
        return Err(bad(format!("{what} is longer than {max} characters")));
    }
    if s.chars().any(char::is_control) {
        return Err(bad(format!("{what} contains control characters")));
    }
    Ok(s.to_string())
}

async fn hash_password(password: String) -> R<String> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
    })
    .await
    .map_err(|e| ApiErr::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| ApiErr::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn verify_password(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).is_ok_and(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
    })
    .await
    .unwrap_or(false)
}

fn check_password(p: &str) -> R<()> {
    if !(8..=256).contains(&p.chars().count()) {
        return Err(bad("password must be 8 to 256 characters"));
    }
    Ok(())
}

async fn user_count(st: &St) -> R<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&st.db)
        .await?)
}

pub async fn info(State(st): State<St>) -> R<Json<ServerInfo>> {
    Ok(Json(ServerInfo {
        name: st.cfg.name.clone(),
        version: crate::VERSION.into(),
        registration: st.cfg.registration,
        first_run: user_count(&st).await? == 0,
        max_upload_bytes: st.cfg.max_upload,
        nextcloud: st.nc.as_ref().map(|n| n.base.clone()),
        // Only signed-in people get it (in `Ready`): this endpoint is public.
        klipy_key: None,
        camera_kbps: st.cfg.camera_kbps,
    }))
}

pub async fn register(State(st): State<St>, Json(req): Json<RegisterReq>) -> R<Json<AuthResp>> {
    let username = req.username.trim().to_lowercase();
    if !valid_username(&username) {
        return Err(bad("usernames are 2-32 characters: a-z, 0-9, _ and ."));
    }
    check_password(&req.password)?;
    let display_name = clean(
        req.display_name.as_deref().unwrap_or(&username),
        32,
        "display name",
    )?;
    let invite = req
        .invite
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let invite_space = match invite {
        Some(code) => Some(spaces::check_invite(&st, code).await?),
        None => None,
    };
    // With Nextcloud on and sign-ups closed, even the first (admin) account must come through Nextcloud,
    // so nobody can claim a fresh public server before its owner signs in.
    if st.cfg.registration == Registration::Closed && st.nc.is_some() {
        return Err(forbidden("this server signs people in with Nextcloud only"));
    }
    if user_count(&st).await? > 0 {
        match st.cfg.registration {
            Registration::Closed => {
                return Err(forbidden("this server doesn't accept new accounts"));
            }
            Registration::Invite if invite_space.is_none() => {
                return Err(forbidden("you need an invite to create an account here"));
            }
            _ => {}
        }
    }
    let hash = hash_password(req.password).await?;
    let id = st.id();
    // The first account becomes admin; decided inside the insert so two racing sign-ups can't both win.
    let res = sqlx::query(
        "INSERT INTO users (id, username, display_name, password_hash, admin, created_at)
         VALUES (?, ?, ?, ?, (SELECT COUNT(*) = 0 FROM users), ?)",
    )
    .bind(id)
    .bind(&username)
    .bind(&display_name)
    .bind(&hash)
    .bind(now_ms())
    .execute(&st.db)
    .await;
    match res {
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(conflict("that username is taken"));
        }
        r => r?,
    };
    let token = create_session(&st, id, req.device, None).await?;
    if let Some(code) = invite {
        spaces::join_space(&st, id, code).await?;
    }
    Ok(Json(AuthResp {
        token,
        user: db::user(&st.db, id).await?,
    }))
}

pub async fn login(State(st): State<St>, Json(req): Json<LoginReq>) -> R<Json<AuthResp>> {
    let username = req.username.trim().to_lowercase();
    {
        let mut fails = st.login_fails.lock().unwrap();
        let list = fails.entry(username.clone()).or_default();
        list.retain(|t| t.elapsed() < Duration::from_secs(600));
        if list.len() >= 10 {
            return Err(ApiErr::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too many attempts, try again in a few minutes",
            ));
        }
    }
    let row: Option<(Id, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE username = ?")
            .bind(&username)
            .fetch_optional(&st.db)
            .await?;
    let ok = match &row {
        Some((_, hash)) => verify_password(req.password, hash.clone()).await,
        None => false,
    };
    let Some((id, _)) = row.filter(|_| ok) else {
        st.login_fails
            .lock()
            .unwrap()
            .entry(username)
            .or_default()
            .push(Instant::now());
        return Err(ApiErr::new(
            StatusCode::UNAUTHORIZED,
            "wrong username or password",
        ));
    };
    st.login_fails.lock().unwrap().remove(&username);
    let token = create_session(&st, id, req.device, None).await?;
    Ok(Json(AuthResp {
        token,
        user: db::user(&st.db, id).await?,
    }))
}

/// End `session` (or, with `all_but`, every other session of `user`): sign those devices out and
/// drop their Nextcloud app passwords. Returns how many ended.
async fn end_sessions(st: &St, user: Id, session: Id, all_but: bool) -> R<usize> {
    let sql = if !all_but {
        "DELETE FROM sessions WHERE user_id = ? AND id = ? RETURNING id, nc_login, nc_app_password"
    } else {
        "DELETE FROM sessions WHERE user_id = ? AND id != ? RETURNING id, nc_login, nc_app_password"
    };
    let ended: Vec<(Id, Option<String>, Option<String>)> = sqlx::query_as(sql)
        .bind(user)
        .bind(session)
        .fetch_all(&st.db)
        .await?;
    for (id, login, password) in &ended {
        gateway::kick_session(st, user, *id).await;
        if let (Some(l), Some(p)) = (login, password) {
            nextcloud::forget_app_password(st, l.clone(), p.clone());
        }
    }
    Ok(ended.len())
}

pub async fn logout(State(st): State<St>, auth: Auth) -> R<StatusCode> {
    end_sessions(&st, auth.user, auth.session, false).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn sessions(State(st): State<St>, auth: Auth) -> R<Json<Vec<SessionInfo>>> {
    let rows: Vec<(Id, String, i64, i64)> = sqlx::query_as(
        "SELECT id, device, created_at, last_seen FROM sessions WHERE user_id = ? ORDER BY last_seen DESC",
    )
    .bind(auth.user)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, device, created_at, last_seen)| SessionInfo {
                id,
                device,
                created_at,
                last_seen,
                current: id == auth.session,
            })
            .collect(),
    ))
}

pub async fn revoke_session(State(st): State<St>, auth: Auth, Path(id): Path<Id>) -> R<StatusCode> {
    if end_sessions(&st, auth.user, id, false).await? == 0 {
        return Err(not_found("session"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn change_password(
    State(st): State<St>,
    auth: Auth,
    Json(req): Json<ChangePassword>,
) -> R<StatusCode> {
    check_password(&req.new_password)?;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(auth.user)
        .fetch_one(&st.db)
        .await?;
    if !verify_password(req.old_password, hash).await {
        return Err(forbidden("current password is wrong"));
    }
    let hash = hash_password(req.new_password).await?;
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash)
        .bind(auth.user)
        .execute(&st.db)
        .await?;
    // Every other device has to sign in again.
    end_sessions(&st, auth.user, auth.session, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete your own account, like Discord: your password confirms it (or `confirm` on a Nextcloud session), and it is
/// refused while you own a space. Your messages (with their files and Nextcloud links), other uploads, memberships,
/// reactions, pings, read markers, Daily Word games, settings and sessions go.
///
/// The `users` row is anonymised, not deleted: it becomes "Deleted User" with no name, password, picture or Nextcloud
/// link, so DMs, threads, bans and `<@id>` mentions that other people keep still point at someone
/// (`messages.author_id` and `spaces.owner_id` have no ON DELETE). Sessions end first but the row is anonymised last,
/// so a failure halfway leaves an account you can sign back into and delete again.
pub async fn delete_me(
    State(st): State<St>,
    auth: Auth,
    Json(req): Json<DeleteAccount>,
) -> R<StatusCode> {
    let me = auth.user;
    let (hash, nc_session): (String, bool) = sqlx::query_as(
        "SELECT u.password_hash, s.nc_login IS NOT NULL FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.id = ?",
    )
    .bind(auth.session)
    .fetch_one(&st.db)
    .await?;
    if nc_session {
        if !req.confirm {
            return Err(bad("confirm that you want to delete your account"));
        }
    } else if !verify_password(req.password.unwrap_or_default(), hash).await {
        return Err(forbidden("password is wrong"));
    }
    let owns: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM spaces WHERE owner_id = ?)")
        .bind(me)
        .fetch_one(&st.db)
        .await?;
    if owns {
        return Err(conflict("transfer or delete your spaces first"));
    }
    let audience = db::user_audience(&st.db, me).await?;

    // Links first: removing them takes an app password, and ending the sessions deletes those.
    nextcloud::unshare_all(&st, me).await?;
    end_sessions(&st, me, auth.session, true).await?;
    end_sessions(&st, me, auth.session, false).await?;
    voice::disconnect(&st, me).await?;

    let mine: Vec<(Id, Id)> =
        sqlx::query_as("SELECT channel_id, id FROM messages WHERE author_id = ? ORDER BY channel_id")
            .bind(me)
            .fetch_all(&st.db)
            .await?;
    for chunk in mine.chunk_by(|a, b| a.0 == b.0) {
        let ch = db::channel(&st.db, chunk[0].0).await?;
        let ids: Vec<Id> = chunk.iter().map(|m| m.1).collect();
        messages::purge(&st, &ch, &ids).await?;
    }
    let joined: Vec<Id> = sqlx::query_scalar("SELECT space_id FROM members WHERE user_id = ?")
        .bind(me)
        .fetch_all(&st.db)
        .await?;
    for space in joined {
        spaces::remove_from_space(&st, space, me).await?;
    }
    // ponytail: reactions vanish for others at their next reload (no ReactionRemove per reaction).
    for sql in [
        "DELETE FROM reactions WHERE user_id = ?",
        "DELETE FROM pings WHERE user_id = ?",
        "DELETE FROM read_states WHERE user_id = ?",
        "DELETE FROM word_plays WHERE user_id = ?",
    ] {
        sqlx::query(sql).bind(me).execute(&st.db).await?;
    }
    // What's left is the avatar and files never sent. Space icons they set stay: those belong to the space.
    let tokens: Vec<String> = sqlx::query_scalar(
        "DELETE FROM uploads WHERE owner_id = ? AND id NOT IN (SELECT icon_id FROM spaces WHERE icon_id IS NOT NULL)
         RETURNING token",
    )
    .bind(me)
    .fetch_all(&st.db)
    .await?;
    messages::delete_files(&st, &tokens);
    // "deleted-…" has a '-', which usernames can't, so nobody can take it; '!' marks a hash nothing matches.
    sqlx::query(
        "UPDATE users SET username = ?, display_name = 'Deleted User', password_hash = '!deleted', avatar_id = NULL,
         about = '', custom_status = NULL, admin = 0, settings = '{}', nc_user_id = NULL WHERE id = ?",
    )
    .bind(format!("deleted-{me}"))
    .bind(me)
    .execute(&st.db)
    .await?;
    // Space members already got MemberRemove; this renames them in DMs, mentions and threads.
    st.hub.send(&audience, &Event::UserUpdate(db::user(&st.db, me).await?));
    Ok(StatusCode::NO_CONTENT)
}

pub async fn put_settings(
    State(st): State<St>,
    auth: Auth,
    Json(v): Json<serde_json::Value>,
) -> R<StatusCode> {
    let text = v.to_string();
    if text.len() > 64 * 1024 {
        return Err(bad("settings are larger than 64 KB"));
    }
    sqlx::query("UPDATE users SET settings = ? WHERE id = ?")
        .bind(text)
        .bind(auth.user)
        .execute(&st.db)
        .await?;
    st.hub.send(&[auth.user], &Event::SettingsUpdate(v));
    Ok(StatusCode::NO_CONTENT)
}

/// An unused image upload of `user`, for avatars and icons. Returns its id.
pub async fn own_image_upload(st: &St, user: Id, upload: Id) -> R<Id> {
    let width: Option<Option<i64>> = sqlx::query_scalar(
        "SELECT width FROM uploads WHERE id = ? AND owner_id = ? AND message_id IS NULL",
    )
    .bind(upload)
    .bind(user)
    .fetch_optional(&st.db)
    .await?;
    match width {
        None => Err(bad("unknown upload")),
        Some(None) => Err(bad("that upload is not an image")),
        Some(Some(_)) => Ok(upload),
    }
}

/// Let the uploads GC take a replaced avatar or icon.
pub async fn release_upload(st: &St, upload: Option<Id>) -> R<()> {
    if let Some(id) = upload {
        sqlx::query("UPDATE uploads SET used = 0, created_at = 0 WHERE id = ?")
            .bind(id)
            .execute(&st.db)
            .await?;
    }
    Ok(())
}

pub async fn update_me(
    State(st): State<St>,
    auth: Auth,
    Json(req): Json<UpdateMe>,
) -> R<Json<User>> {
    let me = auth.user;
    if let Some(name) = &req.display_name {
        let name = clean(name, 32, "display name")?;
        sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
            .bind(name)
            .bind(me)
            .execute(&st.db)
            .await?;
    }
    if let Some(about) = &req.about {
        let about = about.trim();
        if about.chars().count() > MAX_ABOUT_LEN {
            return Err(bad(format!(
                "about me is longer than {MAX_ABOUT_LEN} characters"
            )));
        }
        sqlx::query("UPDATE users SET about = ? WHERE id = ?")
            .bind(about)
            .bind(me)
            .execute(&st.db)
            .await?;
    }
    if let Some(custom) = &req.custom_status {
        let custom = if custom.trim().is_empty() {
            None
        } else {
            Some(clean(custom, 128, "custom status")?)
        };
        sqlx::query("UPDATE users SET custom_status = ? WHERE id = ?")
            .bind(custom)
            .bind(me)
            .execute(&st.db)
            .await?;
    }
    if let Some(avatar) = req.avatar {
        let new = if avatar == 0 {
            None
        } else {
            Some(own_image_upload(&st, me, avatar).await?)
        };
        let old: Option<Id> = sqlx::query_scalar("SELECT avatar_id FROM users WHERE id = ?")
            .bind(me)
            .fetch_one(&st.db)
            .await?;
        if old != new {
            release_upload(&st, old).await?;
            if let Some(id) = new {
                sqlx::query("UPDATE uploads SET used = 1 WHERE id = ?")
                    .bind(id)
                    .execute(&st.db)
                    .await?;
            }
            sqlx::query("UPDATE users SET avatar_id = ? WHERE id = ?")
                .bind(new)
                .bind(me)
                .execute(&st.db)
                .await?;
        }
    }
    if let Some(status) = req.status {
        if status == Status::Offline {
            return Err(bad("choose invisible to appear offline"));
        }
        sqlx::query("UPDATE users SET status = ? WHERE id = ?")
            .bind(db::to_s(status))
            .bind(me)
            .execute(&st.db)
            .await?;
        st.hub.set_chosen(me, status);
        gateway::broadcast_presence(&st, me).await?;
    }
    let user = db::user(&st.db, me).await?;
    let audience = db::user_audience(&st.db, me).await?;
    st.hub.send(&audience, &Event::UserUpdate(user.clone()));
    Ok(Json(user))
}
