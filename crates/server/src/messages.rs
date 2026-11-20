//! Messages, reactions, pins, read markers, search, uploads and file serving.

use std::collections::HashSet;
use std::time::Duration;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use natter_proto::{
    Ack, Attachment, Channel, EditMessage, Event, Id, MAX_ATTACHMENTS, MAX_MESSAGE_LEN, Message,
    MessageKind, ReadState, SearchQuery, SearchResult, SendMessage, Status, parse_user_mentions, perm,
};
use serde::Deserialize;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::auth::Auth;
use crate::db::{self, MsgRow, msg_select};
use crate::{ApiErr, R, St, bad, conflict, forbidden, nextcloud, not_found, now_ms};

// ---------------------------------------------------------------- helpers

pub fn delete_files(st: &St, tokens: &[String]) {
    for t in tokens {
        let _ = std::fs::remove_file(st.cfg.data.join("files").join(t));
    }
}

/// Post a system message (join, pin, call, thread started) and broadcast it.
pub async fn post_system(
    st: &St,
    ch: &Channel,
    author: Id,
    kind: MessageKind,
    reply_to: Option<Id>,
    content: &str,
) -> R<Message> {
    let id = st.id();
    sqlx::query("INSERT INTO messages (id, channel_id, author_id, kind, content, reply_to) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(id)
        .bind(ch.id)
        .bind(author)
        .bind(db::to_s(kind))
        .bind(content)
        .bind(reply_to)
        .execute(&st.db)
        .await?;
    sqlx::query("UPDATE channels SET last_message_id = ? WHERE id = ?")
        .bind(id)
        .bind(ch.id)
        .execute(&st.db)
        .await?;
    let msg = db::message(&st.db, ch.id, id).await?;
    st.hub.send(
        &db::audience(&st.db, ch).await?,
        &Event::MessageCreate(msg.clone()),
    );
    Ok(msg)
}

/// Delete messages of one channel: their files and Nextcloud shares, the rows, then tell everyone.
pub async fn purge(st: &St, ch: &Channel, ids: &[Id]) -> R<()> {
    let json = db::json_ids(ids);
    let tokens: Vec<String> = sqlx::query_scalar(
        "SELECT token FROM uploads WHERE message_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&json)
    .fetch_all(&st.db)
    .await?;
    let shares: Vec<(Id, String)> = sqlx::query_as(
        "SELECT owner_id, nc_share_id FROM uploads WHERE nc_share_id IS NOT NULL AND message_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&json)
    .fetch_all(&st.db)
    .await?;
    let pinged: Vec<Id> = sqlx::query_scalar(
        "SELECT DISTINCT user_id FROM pings WHERE message_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&json)
    .fetch_all(&st.db)
    .await?;
    sqlx::query(
        "DELETE FROM messages WHERE channel_id = ? AND id IN (SELECT value FROM json_each(?))",
    )
    .bind(ch.id)
    .bind(&json)
    .execute(&st.db)
    .await?;
    sqlx::query("UPDATE channels SET last_message_id = (SELECT MAX(id) FROM messages WHERE channel_id = ?1) WHERE id = ?1")
        .bind(ch.id)
        .execute(&st.db)
        .await?;
    delete_files(st, &tokens);
    for (owner, share) in shares {
        nextcloud::unshare_later(st, owner, share);
    }
    let audience = db::audience(&st.db, ch).await?;
    for &id in ids {
        st.hub.send(
            &audience,
            &Event::MessageDelete {
                id,
                channel_id: ch.id,
            },
        );
    }
    for u in pinged {
        st.hub.send(
            &[u],
            &Event::ReadStateUpdate(db::read_state(&st.db, u, ch.id).await?),
        );
    }
    Ok(())
}

fn check_content(content: &str, has_files: bool) -> R<String> {
    let content = content.trim();
    if content.chars().count() > MAX_MESSAGE_LEN {
        return Err(bad(format!(
            "messages are at most {MAX_MESSAGE_LEN} characters"
        )));
    }
    if content.is_empty() && !has_files {
        return Err(bad("empty message"));
    }
    Ok(content.to_string())
}

async fn messaging_access(st: &St, user: Id, channel: Id) -> R<db::Access> {
    let acc = db::access(&st.db, user, channel).await?;
    if !acc.ch.kind.has_messages() {
        return Err(bad("categories have no messages"));
    }
    Ok(acc)
}

// ---------------------------------------------------------------- messages

#[derive(Deserialize)]
pub struct ListQuery {
    before: Option<Id>,
    after: Option<Id>,
    around: Option<Id>,
    limit: Option<u32>,
}

pub async fn list(
    State(st): State<St>,
    auth: Auth,
    Path(ch): Path<Id>,
    Query(q): Query<ListQuery>,
) -> R<Json<Vec<Message>>> {
    messaging_access(&st, auth.user, ch).await?;
    let limit = q.limit.unwrap_or(50).clamp(1, 100) as i64;
    let older = |before: Id, n: i64| {
        sqlx::query_as::<_, MsgRow>(msg_select!(
            "WHERE channel_id = ? AND id < ? ORDER BY id DESC LIMIT ?"
        ))
        .bind(ch)
        .bind(before)
        .bind(n)
        .fetch_all(&st.db)
    };
    let newer = |from: Id, n: i64| {
        sqlx::query_as::<_, MsgRow>(msg_select!(
            "WHERE channel_id = ? AND id >= ? ORDER BY id ASC LIMIT ?"
        ))
        .bind(ch)
        .bind(from)
        .bind(n)
        .fetch_all(&st.db)
    };
    let rows = if let Some(around) = q.around {
        let mut rows = older(around, limit / 2).await?;
        rows.reverse();
        rows.extend(newer(around, limit - limit / 2).await?);
        rows
    } else if let Some(after) = q.after {
        newer(after.saturating_add(1), limit).await?
    } else {
        let mut rows = older(q.before.unwrap_or(Id::MAX), limit).await?;
        rows.reverse();
        rows
    };
    Ok(Json(db::hydrate(&st.db, rows).await?))
}

pub async fn send(
    State(st): State<St>,
    auth: Auth,
    Path(ch): Path<Id>,
    Json(req): Json<SendMessage>,
) -> R<Json<Message>> {
    let me = auth.user;
    let acc = messaging_access(&st, me, ch).await?;
    if !acc.can_post() {
        return Err(forbidden(if acc.has(perm::SEND_MESSAGES) {
            "only members with Manage messages can post here"
        } else {
            "you don't have permission to send messages here"
        }));
    }
    if !req.attachments.is_empty() && !acc.has(perm::ATTACH_FILES) {
        return Err(forbidden("you don't have permission to attach files here"));
    }
    let mut files = req.attachments.clone();
    files.sort_unstable();
    files.dedup();
    if files.len() > MAX_ATTACHMENTS {
        return Err(bad(format!("at most {MAX_ATTACHMENTS} files per message")));
    }
    let content = check_content(&req.content, !files.is_empty())?;
    let nonce = req
        .nonce
        .as_deref()
        .map(|n| n.chars().take(64).collect::<String>())
        .filter(|n| !n.is_empty());

    // A retry of something we already stored: hand back the original, post nothing new.
    if let Some(n) = &nonce {
        let existing =
            sqlx::query_as::<_, MsgRow>(msg_select!("WHERE author_id = ? AND nonce = ?"))
                .bind(me)
                .bind(n)
                .fetch_optional(&st.db)
                .await?;
        if let Some(row) = existing {
            return Ok(Json(db::hydrate(&st.db, vec![row]).await?.remove(0)));
        }
    }
    st.take_send_budget(me).map_err(|ms| ApiErr {
        status: StatusCode::TOO_MANY_REQUESTS,
        msg: "you're sending messages too fast".into(),
        retry_after_ms: Some(ms),
    })?;
    let replied_author: Option<Id> = match req.reply_to {
        Some(r) => Some(
            sqlx::query_scalar("SELECT author_id FROM messages WHERE id = ? AND channel_id = ?")
                .bind(r)
                .bind(ch)
                .fetch_optional(&st.db)
                .await?
                .ok_or_else(|| bad("can't reply to that message"))?,
        ),
        None => None,
    };
    let reply_ping = req.reply_ping && replied_author.is_some_and(|a| a != me);

    let audience = db::audience(&st.db, &acc.ch).await?;
    let in_channel: HashSet<Id> = audience.iter().copied().collect();
    let everyone = content.contains("@everyone");
    let here = content.contains("@here");
    let mention_everyone = (everyone || here) && acc.has(perm::MENTION_EVERYONE);
    let mut pinged: Vec<Id> = vec![];
    if !req.silent {
        pinged = parse_user_mentions(&content)
            .into_iter()
            .filter(|u| in_channel.contains(u))
            .collect();
        if reply_ping {
            pinged.extend(replied_author.filter(|a| in_channel.contains(a)));
        }
        if mention_everyone {
            pinged.extend(
                audience
                    .iter()
                    .filter(|&&u| everyone || st.hub.presence(u) != Status::Offline),
            );
        }
        pinged.retain(|&u| u != me);
        pinged.sort_unstable();
        pinged.dedup();
    }

    let id = st.id();
    let mut tx = st.db.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, content, reply_to, reply_ping, mention_everyone, nonce)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(ch)
    .bind(me)
    .bind(&content)
    .bind(req.reply_to)
    .bind(reply_ping)
    .bind(mention_everyone)
    .bind(&nonce)
    .execute(&mut *tx)
    .await;
    if let Err(sqlx::Error::Database(e)) = &inserted
        && e.is_unique_violation()
    {
        // The same nonce raced in twice: the first one won.
        drop(tx);
        let row = sqlx::query_as::<_, MsgRow>(msg_select!("WHERE author_id = ? AND nonce = ?"))
            .bind(me)
            .bind(&nonce)
            .fetch_one(&st.db)
            .await?;
        return Ok(Json(db::hydrate(&st.db, vec![row]).await?.remove(0)));
    }
    inserted?;
    for f in &files {
        let claimed = sqlx::query(
            "UPDATE uploads SET message_id = ? WHERE id = ? AND owner_id = ? AND message_id IS NULL AND used = 0",
        )
        .bind(id)
        .bind(f)
        .bind(me)
        .execute(&mut *tx)
        .await?;
        if claimed.rows_affected() != 1 {
            return Err(bad("one of the files is unknown or already sent"));
        }
    }
    for u in &pinged {
        sqlx::query("INSERT INTO pings (message_id, user_id) VALUES (?, ?)")
            .bind(id)
            .bind(u)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE channels SET last_message_id = ? WHERE id = ?")
        .bind(id)
        .bind(ch)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if acc.ch.kind == natter_proto::ChannelKind::Thread {
        crate::spaces::unarchive_on_post(&st, &acc.ch).await?;
    }
    // Sending marks the channel read for you.
    db::set_read(&st.db, me, ch, id).await?;

    let msg = db::message(&st.db, ch, id).await?;
    st.hub.send(&audience, &Event::MessageCreate(msg.clone()));
    st.hub.send(
        &[me],
        &Event::ReadStateUpdate(db::read_state(&st.db, me, ch).await?),
    );
    for u in pinged {
        st.hub.send(
            &[u],
            &Event::ReadStateUpdate(db::read_state(&st.db, u, ch).await?),
        );
    }
    Ok(Json(msg))
}

pub async fn edit(
    State(st): State<St>,
    auth: Auth,
    Path((ch, mid)): Path<(Id, Id)>,
    Json(req): Json<EditMessage>,
) -> R<Json<Message>> {
    let acc = messaging_access(&st, auth.user, ch).await?;
    let msg = db::message(&st.db, ch, mid).await?;
    if msg.author_id != auth.user || msg.kind != MessageKind::Default {
        return Err(forbidden("you can only edit your own messages"));
    }
    let content = check_content(&req.content, !msg.attachments.is_empty())?;
    if content != msg.content {
        sqlx::query("UPDATE messages SET content = ?, edited_at = ? WHERE id = ?")
            .bind(&content)
            .bind(now_ms())
            .bind(mid)
            .execute(&st.db)
            .await?;
    }
    let msg = db::message(&st.db, ch, mid).await?;
    st.hub.send(
        &db::audience(&st.db, &acc.ch).await?,
        &Event::MessageUpdate(msg.clone()),
    );
    Ok(Json(msg))
}

pub async fn delete(
    State(st): State<St>,
    auth: Auth,
    Path((ch, mid)): Path<(Id, Id)>,
) -> R<StatusCode> {
    let acc = messaging_access(&st, auth.user, ch).await?;
    let author: Id =
        sqlx::query_scalar("SELECT author_id FROM messages WHERE id = ? AND channel_id = ?")
            .bind(mid)
            .bind(ch)
            .fetch_optional(&st.db)
            .await?
            .ok_or_else(|| not_found("message"))?;
    if author != auth.user && !acc.has(perm::MANAGE_MESSAGES) {
        return Err(forbidden("you can only delete your own messages"));
    }
    purge(&st, &acc.ch, &[mid]).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- reactions & pins

fn check_emoji(e: &str) -> R<()> {
    let ok = !e.is_empty()
        && e.len() <= 64
        && e.chars().count() <= 16
        && !e.chars().any(|c| c.is_whitespace() || c.is_control())
        && !e.is_ascii();
    if !ok {
        return Err(bad("that's not an emoji"));
    }
    Ok(())
}

async fn message_in(st: &St, ch: Id, mid: Id) -> R<()> {
    let ok: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages WHERE id = ? AND channel_id = ?)")
            .bind(mid)
            .bind(ch)
            .fetch_one(&st.db)
            .await?;
    if !ok {
        return Err(not_found("message"));
    }
    Ok(())
}

pub async fn react(
    State(st): State<St>,
    auth: Auth,
    Path((ch, mid, emoji)): Path<(Id, Id, String)>,
) -> R<StatusCode> {
    check_emoji(&emoji)?;
    let acc = messaging_access(&st, auth.user, ch).await?;
    if !acc.has(perm::ADD_REACTIONS) {
        return Err(forbidden("you don't have permission to add reactions here"));
    }
    message_in(&st, ch, mid).await?;
    let (distinct, has): (i64, bool) = sqlx::query_as(
        "SELECT COUNT(DISTINCT emoji), COALESCE(MAX(emoji = ?), 0) FROM reactions WHERE message_id = ?",
    )
    .bind(&emoji)
    .bind(mid)
    .fetch_one(&st.db)
    .await?;
    if distinct >= 20 && !has {
        return Err(conflict(
            "a message can have at most 20 different reactions",
        ));
    }
    let done = sqlx::query("INSERT OR IGNORE INTO reactions (message_id, user_id, emoji, created_at) VALUES (?, ?, ?, ?)")
        .bind(mid)
        .bind(auth.user)
        .bind(&emoji)
        .bind(now_ms())
        .execute(&st.db)
        .await?;
    if done.rows_affected() == 1 {
        let ev = Event::ReactionAdd {
            channel_id: ch,
            message_id: mid,
            user_id: auth.user,
            emoji,
        };
        st.hub.send(&db::audience(&st.db, &acc.ch).await?, &ev);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unreact(
    State(st): State<St>,
    auth: Auth,
    Path((ch, mid, emoji)): Path<(Id, Id, String)>,
) -> R<StatusCode> {
    let acc = messaging_access(&st, auth.user, ch).await?;
    message_in(&st, ch, mid).await?;
    let done =
        sqlx::query("DELETE FROM reactions WHERE message_id = ? AND user_id = ? AND emoji = ?")
            .bind(mid)
            .bind(auth.user)
            .bind(&emoji)
            .execute(&st.db)
            .await?;
    if done.rows_affected() == 1 {
        let ev = Event::ReactionRemove {
            channel_id: ch,
            message_id: mid,
            user_id: auth.user,
            emoji,
        };
        st.hub.send(&db::audience(&st.db, &acc.ch).await?, &ev);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pins(State(st): State<St>, auth: Auth, Path(ch): Path<Id>) -> R<Json<Vec<Message>>> {
    messaging_access(&st, auth.user, ch).await?;
    let rows = sqlx::query_as::<_, MsgRow>(msg_select!(
        "WHERE channel_id = ? AND pinned_at IS NOT NULL ORDER BY pinned_at DESC"
    ))
    .bind(ch)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(db::hydrate(&st.db, rows).await?))
}

async fn set_pinned(st: &St, auth: &Auth, ch: Id, mid: Id, pin: bool) -> R<StatusCode> {
    let acc = messaging_access(st, auth.user, ch).await?;
    if !acc.can_pin() {
        return Err(forbidden("pinning takes the Manage messages permission"));
    }
    let msg = db::message(&st.db, ch, mid).await?;
    if msg.kind != MessageKind::Default {
        return Err(bad("system messages can't be pinned"));
    }
    if msg.pinned == pin {
        return Ok(StatusCode::NO_CONTENT);
    }
    if pin {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE channel_id = ? AND pinned_at IS NOT NULL",
        )
        .bind(ch)
        .fetch_one(&st.db)
        .await?;
        if count >= 50 {
            return Err(conflict("a channel can have at most 50 pins"));
        }
    }
    sqlx::query("UPDATE messages SET pinned_at = ? WHERE id = ?")
        .bind(pin.then(now_ms))
        .bind(mid)
        .execute(&st.db)
        .await?;
    let msg = db::message(&st.db, ch, mid).await?;
    st.hub.send(
        &db::audience(&st.db, &acc.ch).await?,
        &Event::MessageUpdate(msg),
    );
    if pin {
        post_system(st, &acc.ch, auth.user, MessageKind::Pin, Some(mid), "").await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pin(
    State(st): State<St>,
    auth: Auth,
    Path((ch, mid)): Path<(Id, Id)>,
) -> R<StatusCode> {
    set_pinned(&st, &auth, ch, mid, true).await
}

pub async fn unpin(
    State(st): State<St>,
    auth: Auth,
    Path((ch, mid)): Path<(Id, Id)>,
) -> R<StatusCode> {
    set_pinned(&st, &auth, ch, mid, false).await
}

pub async fn ack(
    State(st): State<St>,
    auth: Auth,
    Path(ch): Path<Id>,
    Json(req): Json<Ack>,
) -> R<StatusCode> {
    messaging_access(&st, auth.user, ch).await?;
    if req.message_id < 0 {
        return Err(bad("bad message id"));
    }
    db::set_read(&st.db, auth.user, ch, req.message_id).await?;
    let state: ReadState = db::read_state(&st.db, auth.user, ch).await?;
    st.hub.send(&[auth.user], &Event::ReadStateUpdate(state));
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- search

/// Words → FTS5 query: every word must appear, each as a literal prefix.
fn fts_query(q: &str) -> Option<String> {
    let terms: Vec<String> = q
        .split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| !w.is_empty())
        .take(16)
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

macro_rules! search_where {
    () => {
        "FROM messages m JOIN channels c ON c.id = m.channel_id
         WHERE m.kind = 'default'
           AND m.channel_id IN (SELECT c2.id FROM channels c2 JOIN members mb ON mb.space_id = c2.space_id AND mb.user_id = ?1
                                UNION SELECT channel_id FROM dm_members WHERE user_id = ?1)
           AND (?2 IS NULL OR m.id IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?2))
           AND (?3 IS NULL OR c.space_id = ?3)
           AND (?4 IS NULL OR m.channel_id = ?4)
           AND (?5 IS NULL OR m.author_id = ?5)
           AND (?6 IS NULL OR ?6 = 0 OR EXISTS(SELECT 1 FROM uploads u WHERE u.message_id = m.id))
           AND COALESCE(CASE WHEN c.kind = 'thread' THEN c.parent_id END, m.channel_id) NOT IN (SELECT value FROM json_each(?7)) "
    };
}

pub async fn search(
    State(st): State<St>,
    auth: Auth,
    Query(q): Query<SearchQuery>,
) -> R<Json<SearchResult>> {
    let fts = fts_query(&q.q);
    if fts.is_none() && q.author_id.is_none() && q.has_file != Some(true) {
        return Err(bad("type something to search for"));
    }
    let limit = q.limit.unwrap_or(25).clamp(1, 50) as i64;
    // Private channels you can't see (and their threads) stay out of the results.
    let hidden = serde_json::to_string(&db::hidden_channels(&st.db, auth.user).await?).unwrap_or_else(|_| "[]".into());
    let rows = sqlx::query_as::<_, MsgRow>(concat!(
        "SELECT m.id, m.channel_id, m.author_id, m.kind, m.content, m.edited_at, m.reply_to, m.reply_ping, m.mention_everyone, ",
        "m.pinned_at, m.nonce ",
        search_where!(),
        "AND (?8 IS NULL OR m.id < ?8) ORDER BY m.id DESC LIMIT ?9"
    ))
    .bind(auth.user)
    .bind(&fts)
    .bind(q.space_id)
    .bind(q.channel_id)
    .bind(q.author_id)
    .bind(q.has_file)
    .bind(&hidden)
    .bind(q.before)
    .bind(limit)
    .fetch_all(&st.db)
    .await?;
    let total: i64 = sqlx::query_scalar(concat!("SELECT COUNT(*) ", search_where!()))
        .bind(auth.user)
        .bind(&fts)
        .bind(q.space_id)
        .bind(q.channel_id)
        .bind(q.author_id)
        .bind(q.has_file)
        .bind(&hidden)
        .fetch_one(&st.db)
        .await?;
    Ok(Json(SearchResult {
        messages: db::hydrate(&st.db, rows).await?,
        total: total as u32,
    }))
}

// ---------------------------------------------------------------- uploads & files

fn header_str<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

/// `X-Filename` (percent-encoded UTF-8) → a safe display name.
fn clean_filename(raw: Option<&str>) -> String {
    let decoded = raw
        .map(|r| {
            percent_encoding::percent_decode_str(r)
                .decode_utf8_lossy()
                .into_owned()
        })
        .unwrap_or_default();
    let base = decoded.rsplit(['/', '\\']).next().unwrap_or("");
    let name: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    let name = name.trim().trim_start_matches('.').to_string();
    if name.is_empty() { "file".into() } else { name }
}

fn clean_content_type(raw: Option<&str>) -> String {
    let ct = raw
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let ok = ct.split_once('/').is_some_and(|(a, b)| {
        let part = |s: &str| {
            !s.is_empty()
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".+-_".contains(&c))
        };
        part(a) && part(b)
    });
    if ok {
        ct
    } else {
        "application/octet-stream".into()
    }
}

/// Image dimensions, read from the file's header.
pub fn image_dims(content_type: &str, bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    if !content_type.starts_with("image/") || content_type == "image/svg+xml" {
        return (None, None);
    }
    match imagesize::blob_size(bytes) {
        Ok(s) if s.width > 0 && s.height > 0 => (Some(s.width as u32), Some(s.height as u32)),
        _ => (None, None),
    }
}

pub struct NewUpload {
    pub owner: Id,
    pub filename: String,
    pub content_type: String,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub url: Option<String>,
    pub link: Option<String>,
    pub nc_share_id: Option<String>,
    pub used: bool,
}

/// Record an upload (its bytes are already stored under `token`, or in Nextcloud).
pub async fn insert_upload(st: &St, token: &str, u: NewUpload) -> R<Attachment> {
    let id = st.id();
    sqlx::query(
        "INSERT INTO uploads (id, token, owner_id, filename, content_type, size, width, height, url, link, nc_share_id, used, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(token)
    .bind(u.owner)
    .bind(&u.filename)
    .bind(&u.content_type)
    .bind(u.size as i64)
    .bind(u.width)
    .bind(u.height)
    .bind(&u.url)
    .bind(&u.link)
    .bind(&u.nc_share_id)
    .bind(u.used)
    .bind(now_ms())
    .execute(&st.db)
    .await?;
    Ok(Attachment {
        id,
        url: u.url.unwrap_or_else(|| db::file_url(token, &u.filename)),
        filename: u.filename,
        content_type: u.content_type,
        size: u.size,
        width: u.width,
        height: u.height,
        link: u.link,
    })
}

pub fn new_token() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

/// Store a file locally. Returns its token.
pub async fn store_local(st: &St, bytes: &[u8]) -> R<String> {
    let token = new_token();
    tokio::fs::write(st.cfg.data.join("files").join(&token), bytes).await?;
    Ok(token)
}

// ponytail: the whole upload is held in memory (≤ NATTER_MAX_UPLOAD_MB); stream to disk if that limit grows past a few hundred MB.
pub async fn upload(
    State(st): State<St>,
    auth: Auth,
    headers: HeaderMap,
    body: Bytes,
) -> R<Json<Attachment>> {
    if body.is_empty() {
        return Err(bad("empty file"));
    }
    let filename = clean_filename(header_str(&headers, "x-filename"));
    let content_type = clean_content_type(header_str(&headers, "content-type"));
    let (width, height) = image_dims(&content_type, &body);

    // Nextcloud sessions keep their files in their own Nextcloud, in a folder per space.
    let channel = header_str(&headers, "x-channel-id").and_then(|v| v.parse::<Id>().ok());
    if let Some(stored) =
        nextcloud::store_upload(&st, &auth, channel, &filename, &content_type, &body).await
    {
        let up = NewUpload {
            owner: auth.user,
            filename,
            content_type,
            size: body.len() as u64,
            width,
            height,
            url: Some(stored.url),
            link: Some(stored.link),
            nc_share_id: stored.share_id,
            used: false,
        };
        return Ok(Json(insert_upload(&st, &new_token(), up).await?));
    }

    let token = store_local(&st, &body).await?;
    let up = NewUpload {
        owner: auth.user,
        filename,
        content_type,
        size: body.len() as u64,
        width,
        height,
        url: None,
        link: None,
        nc_share_id: None,
        used: false,
    };
    Ok(Json(insert_upload(&st, &token, up).await?))
}

/// Types a browser may render in place. Everything else downloads, and SVG never renders (it can carry script).
fn inline_ok(ct: &str) -> bool {
    matches!(
        ct,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "image/bmp"
            | "video/mp4"
            | "video/webm"
            | "video/ogg"
            | "video/quicktime"
            | "text/plain"
            | "application/pdf"
    ) || ct.starts_with("audio/")
}

pub async fn file(
    State(st): State<St>,
    Path((token, _name)): Path<(String, String)>,
    req: Request,
) -> R<Response> {
    if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(not_found("file"));
    }
    let (filename, ct): (String, String) = sqlx::query_as(
        "SELECT filename, content_type FROM uploads WHERE token = ? AND url IS NULL",
    )
    .bind(&token)
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(|| not_found("file"))?;
    let path = st.cfg.data.join("files").join(&token);
    // ServeFile handles Range requests, so videos seek.
    let res = ServeFile::new(&path)
        .oneshot(req)
        .await
        .map_err(crate::internal)?;
    let mut res = res.map(Body::new);
    if res.status().is_success() {
        let inline = inline_ok(&ct);
        let mime = match (inline, ct.as_str()) {
            (true, "text/plain") => "text/plain; charset=utf-8".to_string(),
            (true, _) => ct,
            (false, _) => "application/octet-stream".into(),
        };
        let encoded =
            percent_encoding::utf8_percent_encode(&filename, percent_encoding::NON_ALPHANUMERIC);
        let disposition = format!(
            "{}; filename*=UTF-8''{encoded}",
            if inline { "inline" } else { "attachment" }
        );
        let h = res.headers_mut();
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(&mime).map_err(crate::internal)?,
        );
        h.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&disposition).map_err(crate::internal)?,
        );
        h.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; sandbox"),
        );
        h.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, max-age=31536000, immutable"),
        );
    }
    Ok(res)
}

/// Hourly: drop uploads nobody sent within a day (and replaced avatars and icons).
pub async fn gc_uploads(st: St) {
    let mut every = tokio::time::interval(Duration::from_secs(3600));
    loop {
        every.tick().await;
        let res: Result<Vec<(String, Id, Option<String>)>, _> = sqlx::query_as(
            "DELETE FROM uploads WHERE message_id IS NULL AND used = 0 AND created_at < ? RETURNING token, owner_id, nc_share_id",
        )
        .bind(now_ms() - 86_400_000)
        .fetch_all(&st.db)
        .await;
        match res {
            Ok(rows) => {
                for (token, owner, share) in rows {
                    delete_files(&st, &[token]);
                    if let Some(share) = share {
                        nextcloud::unshare_later(&st, owner, share);
                    }
                }
            }
            Err(e) => tracing::warn!("upload gc: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_names_and_queries() {
        assert_eq!(clean_filename(Some("..%2F..%2Fetc%2Fpasswd")), "passwd");
        assert_eq!(
            clean_filename(Some("C:%5Cx%5Cr%C3%A9sum%C3%A9.pdf")),
            "résumé.pdf"
        );
        assert_eq!(clean_filename(Some(".env")), "env");
        assert_eq!(clean_filename(None), "file");
        assert_eq!(
            clean_content_type(Some("image/PNG; charset=x")),
            "image/png"
        );
        assert_eq!(
            clean_content_type(Some("text/html<script>")),
            "application/octet-stream"
        );
        assert_eq!(
            fts_query(r#"hello "wor"ld  "#).as_deref(),
            Some(r#""hello"* "world"*"#)
        );
        assert_eq!(fts_query("   "), None);
        assert!(!inline_ok("image/svg+xml") && !inline_ok("text/html") && inline_ok("audio/ogg"));
    }
}
