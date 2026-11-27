//! Reports, blocks and suspended accounts.
//!
//! Anyone can report a message to the server's admins; the report keeps a copy of the message as it was. Admins
//! dismiss it, delete the message or suspend the author (signed out everywhere, can't sign in again until
//! unsuspended). Blocking someone stops their direct messages and rings to you; the apps also hide their messages.

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use laklak_proto::{
    ChannelKind, Event, Id, Report, ReportAction, ReportList, ReportReq, ResolveReport, SetSuspended,
};

use crate::auth::Auth;
use crate::{ApiErr, R, St, auth, bad, db, forbidden, messages, not_found, now_ms, voice};

/// Reports a person may file in an hour.
const REPORTS_PER_HOUR: i64 = 20;
/// Resolved reports, with their copy of the message, go after this long.
const KEEP_RESOLVED: Duration = Duration::from_secs(90 * 24 * 3600);

async fn require_admin(st: &St, user: Id) -> R<()> {
    let admin: bool = sqlx::query_scalar("SELECT admin FROM users WHERE id = ?").bind(user).fetch_one(&st.db).await?;
    if !admin {
        return Err(forbidden("only server admins can do that"));
    }
    Ok(())
}

pub async fn open_reports(st: &St) -> R<u32> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE resolved_at IS NULL").fetch_one(&st.db).await?;
    Ok(n as u32)
}

async fn notify_admins(st: &St) -> R<()> {
    let admins: Vec<Id> = sqlx::query_scalar("SELECT id FROM users WHERE admin = 1").fetch_all(&st.db).await?;
    st.hub.send(&admins, &Event::ReportsChanged { open: open_reports(st).await? });
    Ok(())
}

/// Report a message you can see (not your own) to the server's admins.
pub async fn report(
    State(st): State<St>,
    auth: Auth,
    Path((ch, mid)): Path<(Id, Id)>,
    Json(req): Json<ReportReq>,
) -> R<StatusCode> {
    db::access(&st.db, auth.user, ch).await?;
    let (author, content): (Id, String) =
        sqlx::query_as("SELECT author_id, content FROM messages WHERE id = ? AND channel_id = ?")
            .bind(mid)
            .bind(ch)
            .fetch_optional(&st.db)
            .await?
            .ok_or_else(|| not_found("message"))?;
    if author == auth.user {
        return Err(bad("that's your own message"));
    }
    let reason = req.reason.trim();
    if reason.is_empty() || reason.chars().count() > 1000 {
        return Err(bad("say what's wrong with it, in up to 1,000 characters"));
    }
    let recent: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE reporter_id = ? AND created_at > ?")
        .bind(auth.user)
        .bind(now_ms() - 3600 * 1000)
        .fetch_one(&st.db)
        .await?;
    if recent >= REPORTS_PER_HOUR {
        return Err(ApiErr::new(StatusCode::TOO_MANY_REQUESTS, "that's a lot of reports: try again in an hour"));
    }
    let added = sqlx::query(
        "INSERT OR IGNORE INTO reports (id, reporter_id, author_id, channel_id, message_id, content, reason, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(st.id())
    .bind(auth.user)
    .bind(author)
    .bind(ch)
    .bind(mid)
    .bind(&content)
    .bind(reason)
    .bind(now_ms())
    .execute(&st.db)
    .await?
    .rows_affected();
    if added == 1 {
        tracing::info!("user {} reported message {mid} by user {author}", auth.user);
        notify_admins(&st).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Server admins: open reports first (oldest first), then the latest 50 resolved ones, and who's suspended.
pub async fn list(State(st): State<St>, auth: Auth) -> R<Json<ReportList>> {
    require_admin(&st, auth.user).await?;
    sqlx::query("DELETE FROM reports WHERE resolved_at < ?")
        .bind(now_ms() - KEEP_RESOLVED.as_millis() as i64)
        .execute(&st.db)
        .await?;
    type Row = (Id, Option<Id>, Option<Id>, Id, Id, bool, String, String, i64, Option<i64>, Option<Id>, Option<String>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT r.id, r.reporter_id, r.author_id, r.channel_id, r.message_id,
                EXISTS(SELECT 1 FROM messages m WHERE m.id = r.message_id), r.content, r.reason, r.created_at,
                r.resolved_at, r.resolved_by, r.resolution
         FROM reports r
         WHERE r.resolved_at IS NULL OR r.id IN (SELECT id FROM reports WHERE resolved_at IS NOT NULL ORDER BY resolved_at DESC LIMIT 50)
         ORDER BY r.resolved_at IS NOT NULL, CASE WHEN r.resolved_at IS NULL THEN r.created_at ELSE -r.resolved_at END",
    )
    .fetch_all(&st.db)
    .await?;
    let reports: Vec<Report> = rows
        .into_iter()
        .map(|r| Report {
            id: r.0,
            reporter_id: r.1,
            author_id: r.2,
            channel_id: r.3,
            message_id: r.4,
            message_exists: r.5,
            content: r.6,
            reason: r.7,
            created_at: r.8,
            resolved_at: r.9,
            resolved_by: r.10,
            resolution: r.11,
        })
        .collect();
    let mut ids: Vec<Id> =
        reports.iter().flat_map(|r| [r.reporter_id, r.author_id, r.resolved_by]).flatten().collect();
    ids.sort_unstable();
    ids.dedup();
    let suspended: Vec<Id> = sqlx::query_scalar("SELECT id FROM users WHERE suspended = 1").fetch_all(&st.db).await?;
    Ok(Json(ReportList {
        reports,
        users: db::users(&st.db, &ids).await?,
        suspended: db::users(&st.db, &suspended).await?,
    }))
}

/// Server admins: dismiss a report, delete its message, or suspend its author.
pub async fn resolve(
    State(st): State<St>,
    auth: Auth,
    Path(id): Path<Id>,
    Json(req): Json<ResolveReport>,
) -> R<StatusCode> {
    require_admin(&st, auth.user).await?;
    let (author, ch, mid): (Option<Id>, Id, Id) =
        sqlx::query_as("SELECT author_id, channel_id, message_id FROM reports WHERE id = ?")
            .bind(id)
            .fetch_optional(&st.db)
            .await?
            .ok_or_else(|| not_found("report"))?;
    let resolution = match req.action {
        ReportAction::Dismiss => "dismissed",
        ReportAction::Delete => {
            if let Ok(channel) = db::channel(&st.db, ch).await {
                messages::purge(&st, &channel, &[mid]).await?;
            }
            "deleted"
        }
        ReportAction::Suspend => {
            let author = author.ok_or_else(|| not_found("author"))?;
            suspend(&st, auth.user, author, true).await?;
            "suspended"
        }
    };
    // Other reports of the same message are settled by this one too.
    sqlx::query(
        "UPDATE reports SET resolved_at = ?, resolved_by = ?, resolution = ? WHERE message_id = ? AND resolved_at IS NULL",
    )
    .bind(now_ms())
    .bind(auth.user)
    .bind(resolution)
    .bind(mid)
    .execute(&st.db)
    .await?;
    tracing::info!("server admin {} resolved the report on message {mid}: {resolution}", auth.user);
    notify_admins(&st).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Server admins: suspend an account (signed out everywhere, out of any call, can't sign in) or lift it.
pub async fn set_suspended(
    State(st): State<St>,
    auth: Auth,
    Path(user): Path<Id>,
    Json(req): Json<SetSuspended>,
) -> R<StatusCode> {
    require_admin(&st, auth.user).await?;
    suspend(&st, auth.user, user, req.suspended).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn suspend(st: &St, admin: Id, user: Id, on: bool) -> R<()> {
    let (username, is_admin): (String, bool) = sqlx::query_as("SELECT username, admin FROM users WHERE id = ?")
        .bind(user)
        .fetch_optional(&st.db)
        .await?
        .ok_or_else(|| not_found("user"))?;
    if on && is_admin {
        return Err(forbidden("server admins can't be suspended (take their admin rights away first)"));
    }
    sqlx::query("UPDATE users SET suspended = ? WHERE id = ?").bind(on).bind(user).execute(&st.db).await?;
    if on {
        voice::disconnect(st, user).await?;
        auth::end_sessions(st, user, 0, true).await?;
    }
    tracing::info!("server admin {admin} {} {username}", if on { "suspended" } else { "lifted the suspension of" });
    Ok(())
}

/// Block someone: no new DMs or messages in a DM with you, no rings; your apps hide their messages.
pub async fn block(State(st): State<St>, auth: Auth, Path(user): Path<Id>) -> R<StatusCode> {
    if user == auth.user {
        return Err(bad("you can't block yourself"));
    }
    db::user(&st.db, user).await?;
    sqlx::query("INSERT OR IGNORE INTO blocks (user_id, blocked_id, created_at) VALUES (?, ?, ?)")
        .bind(auth.user)
        .bind(user)
        .bind(now_ms())
        .execute(&st.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unblock(State(st): State<St>, auth: Auth, Path(user): Path<Id>) -> R<StatusCode> {
    sqlx::query("DELETE FROM blocks WHERE user_id = ? AND blocked_id = ?")
        .bind(auth.user)
        .bind(user)
        .execute(&st.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn blocked_by(st: &St, user: Id) -> R<Vec<Id>> {
    Ok(sqlx::query_scalar("SELECT blocked_id FROM blocks WHERE user_id = ?").bind(user).fetch_all(&st.db).await?)
}

/// Either of the two blocked the other.
pub async fn between(st: &St, a: Id, b: Id) -> R<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM blocks WHERE (user_id = ? AND blocked_id = ?) OR (user_id = ? AND blocked_id = ?))",
    )
    .bind(a)
    .bind(b)
    .bind(b)
    .bind(a)
    .fetch_one(&st.db)
    .await?)
}

/// A one-to-one DM where either side blocked the other takes no messages (group DMs stay open).
pub async fn check_dm(st: &St, me: Id, ch: &laklak_proto::Channel) -> R<()> {
    if ch.kind == ChannelKind::Dm
        && let Some(&other) = ch.recipients.iter().find(|&&u| u != me)
        && between(st, me, other).await?
    {
        return Err(forbidden("you can't message this person"));
    }
    Ok(())
}
