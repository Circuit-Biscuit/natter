//! Storage: pool setup, row → wire conversion, access checks and audiences.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use natter_proto::{
    Attachment, Channel, Id, Member, Message, Reaction, ReadState, ReplyPreview, Role, Space,
    ThreadInfo, ThreadPreview, User, id_time, parse_user_mentions,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{FromRow, SqlitePool};

use crate::{R, not_found};

pub async fn open(path: &Path) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(10));
    let db = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&db).await?;
    Ok(db)
}

/// Enums are stored as their serde names (`"text"`, `"owner"`, ...).
pub fn to_s<T: Serialize>(v: T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        other => unreachable!("not a unit enum: {other:?}"),
    }
}

pub fn from_s<T: DeserializeOwned>(s: &str) -> T {
    serde_json::from_value(serde_json::Value::String(s.into()))
        .unwrap_or_else(|_| panic!("bad enum value in database: {s}"))
}

/// JSON array for `IN (SELECT value FROM json_each(?))`, which keeps every query a static string.
pub fn json_ids(ids: &[Id]) -> String {
    serde_json::to_string(ids).unwrap()
}

pub fn file_url(token: &str, filename: &str) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    format!(
        "/files/{token}/{}",
        utf8_percent_encode(filename, NON_ALPHANUMERIC)
    )
}

// ---------------------------------------------------------------- users

macro_rules! user_select {
    ($tail:literal) => {
        concat!(
            "SELECT u.id, u.username, u.display_name, u.about, u.custom_status, u.admin, u.created_at, ",
            "up.token AS av_token, up.filename AS av_name ",
            "FROM users u LEFT JOIN uploads up ON up.id = u.avatar_id ",
            $tail
        )
    };
}

#[derive(FromRow)]
struct UserRow {
    id: Id,
    username: String,
    display_name: String,
    about: String,
    custom_status: Option<String>,
    admin: bool,
    created_at: i64,
    av_token: Option<String>,
    av_name: Option<String>,
}

impl From<UserRow> for User {
    fn from(r: UserRow) -> User {
        User {
            id: r.id,
            username: r.username,
            display_name: r.display_name,
            avatar: r.av_token.zip(r.av_name).map(|(t, n)| file_url(&t, &n)),
            about: r.about,
            custom_status: r.custom_status,
            admin: r.admin,
            created_at: r.created_at,
        }
    }
}

pub async fn user(db: &SqlitePool, id: Id) -> R<User> {
    sqlx::query_as::<_, UserRow>(user_select!("WHERE u.id = ?"))
        .bind(id)
        .fetch_optional(db)
        .await?
        .map(User::from)
        .ok_or_else(|| not_found("user"))
}

pub async fn users(db: &SqlitePool, ids: &[Id]) -> R<Vec<User>> {
    let rows = sqlx::query_as::<_, UserRow>(user_select!(
        "WHERE u.id IN (SELECT value FROM json_each(?)) ORDER BY u.id"
    ))
    .bind(json_ids(ids))
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(User::from).collect())
}

/// Everyone who shares a space or a DM with `user`, including `user`.
pub async fn user_audience(db: &SqlitePool, user: Id) -> R<Vec<Id>> {
    Ok(sqlx::query_scalar(
        "SELECT m2.user_id FROM members m1 JOIN members m2 ON m2.space_id = m1.space_id WHERE m1.user_id = ?1
         UNION
         SELECT d2.user_id FROM dm_members d1 JOIN dm_members d2 ON d2.channel_id = d1.channel_id WHERE d1.user_id = ?1
         UNION SELECT ?1",
    )
    .bind(user)
    .fetch_all(db)
    .await?)
}

// ---------------------------------------------------------------- spaces & channels

macro_rules! channel_select {
    ($tail:literal) => {
        concat!(
            "SELECT id, space_id, kind, name, topic, position, parent_id, read_only, last_message_id, ",
            "starter_id, owner_id, archived, ",
            "CASE WHEN kind = 'thread' THEN (SELECT COUNT(*) FROM messages m WHERE m.channel_id = channels.id) ELSE 0 END ",
            "AS message_count FROM channels ",
            $tail
        )
    };
}
pub(crate) use channel_select;

#[derive(FromRow)]
pub struct ChannelRow {
    pub id: Id,
    pub space_id: Option<Id>,
    pub kind: String,
    pub name: String,
    pub topic: String,
    pub position: i32,
    pub parent_id: Option<Id>,
    pub read_only: bool,
    pub last_message_id: Option<Id>,
    pub starter_id: Option<Id>,
    pub owner_id: Option<Id>,
    pub archived: bool,
    pub message_count: i64,
}

impl ChannelRow {
    pub fn into_channel(self, recipients: Vec<Id>) -> Channel {
        Channel {
            id: self.id,
            space_id: self.space_id,
            kind: from_s(&self.kind),
            name: self.name,
            topic: self.topic,
            position: self.position,
            parent_id: self.parent_id,
            read_only: self.read_only,
            last_message_id: self.last_message_id,
            recipients,
            thread: (self.kind == "thread").then(|| ThreadInfo {
                starter_id: self.starter_id.unwrap_or_default(),
                owner_id: self.owner_id.unwrap_or_default(),
                archived: self.archived,
                message_count: self.message_count as u32,
            }),
        }
    }
}

pub async fn dm_recipients(db: &SqlitePool, channel: Id) -> R<Vec<Id>> {
    Ok(
        sqlx::query_scalar("SELECT user_id FROM dm_members WHERE channel_id = ? ORDER BY user_id")
            .bind(channel)
            .fetch_all(db)
            .await?,
    )
}

pub async fn channel(db: &SqlitePool, id: Id) -> R<Channel> {
    let row = sqlx::query_as::<_, ChannelRow>(channel_select!("WHERE id = ?"))
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| not_found("channel"))?;
    let recipients = if row.space_id.is_none() {
        dm_recipients(db, id).await?
    } else {
        vec![]
    };
    Ok(row.into_channel(recipients))
}

pub async fn dms_of(db: &SqlitePool, user: Id) -> R<Vec<Channel>> {
    let rows = sqlx::query_as::<_, ChannelRow>(channel_select!(
        "WHERE id IN (SELECT channel_id FROM dm_members WHERE user_id = ?) ORDER BY id"
    ))
    .bind(user)
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let rec = dm_recipients(db, r.id).await?;
        out.push(r.into_channel(rec));
    }
    Ok(out)
}

pub async fn space(db: &SqlitePool, id: Id) -> R<Space> {
    let (name, owner_id, token, filename): (String, Id, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT s.name, s.owner_id, up.token, up.filename FROM spaces s LEFT JOIN uploads up ON up.id = s.icon_id
         WHERE s.id = ?",
    )
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| not_found("space"))?;
    // Archived threads stay out of the channel list; GET /api/channels/{id}/threads has them.
    let channels = sqlx::query_as::<_, ChannelRow>(channel_select!(
        "WHERE space_id = ? AND NOT (kind = 'thread' AND archived = 1) ORDER BY position, id"
    ))
    .bind(id)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| r.into_channel(vec![]))
    .collect();
    let members = sqlx::query_as::<_, (Id, String, Option<String>, i64)>(
        "SELECT user_id, role, nickname, joined_at FROM members WHERE space_id = ? ORDER BY joined_at, user_id",
    )
    .bind(id)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|(user_id, role, nickname, joined_at)| Member { user_id, role: from_s(&role), nickname, joined_at })
    .collect();
    Ok(Space {
        id,
        name,
        icon: token.zip(filename).map(|(t, n)| file_url(&t, &n)),
        owner_id,
        channels,
        members,
        users: vec![],
    })
}

/// A space with its members' profiles filled in (for `SpaceCreate` and join/create replies).
pub async fn space_with_users(db: &SqlitePool, id: Id) -> R<Space> {
    let mut s = space(db, id).await?;
    let ids: Vec<Id> = s.members.iter().map(|m| m.user_id).collect();
    s.users = users(db, &ids).await?;
    Ok(s)
}

pub async fn spaces_of(db: &SqlitePool, user: Id) -> R<Vec<Space>> {
    let ids: Vec<Id> =
        sqlx::query_scalar("SELECT space_id FROM members WHERE user_id = ? ORDER BY joined_at")
            .bind(user)
            .fetch_all(db)
            .await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(space(db, id).await?);
    }
    Ok(out)
}

pub async fn role(db: &SqlitePool, space: Id, user: Id) -> R<Option<Role>> {
    let r: Option<String> =
        sqlx::query_scalar("SELECT role FROM members WHERE space_id = ? AND user_id = ?")
            .bind(space)
            .bind(user)
            .fetch_optional(db)
            .await?;
    Ok(r.map(|r| from_s(&r)))
}

/// Your role in a space, or 404 when you are not a member (so spaces can't be probed).
pub async fn member_role(db: &SqlitePool, space: Id, user: Id) -> R<Role> {
    role(db, space, user)
        .await?
        .ok_or_else(|| not_found("space"))
}

pub async fn space_members(db: &SqlitePool, space: Id) -> R<Vec<Id>> {
    Ok(
        sqlx::query_scalar("SELECT user_id FROM members WHERE space_id = ?")
            .bind(space)
            .fetch_all(db)
            .await?,
    )
}

/// What a user may do in a channel they can see.
pub struct Access {
    pub ch: Channel,
    /// `None` in DMs.
    pub role: Option<Role>,
}

impl Access {
    pub fn dm(&self) -> bool {
        self.role.is_none()
    }
    pub fn is_mod(&self) -> bool {
        self.role.is_some_and(Role::is_mod)
    }
    /// Pin, and ping @everyone: mods in spaces, anyone in DMs.
    pub fn can_moderate_chat(&self) -> bool {
        self.dm() || self.is_mod()
    }
    pub fn can_post(&self) -> bool {
        self.dm() || !self.ch.read_only || self.is_mod()
    }
}

/// The channel as `user` may see it, or 404.
pub async fn access(db: &SqlitePool, user: Id, channel_id: Id) -> R<Access> {
    let ch = channel(db, channel_id).await?;
    let role = match ch.space_id {
        Some(space) => Some(
            role(db, space, user)
                .await?
                .ok_or_else(|| not_found("channel"))?,
        ),
        None if ch.recipients.contains(&user) => None,
        None => return Err(not_found("channel")),
    };
    Ok(Access { ch, role })
}

/// Who receives events about a channel.
pub async fn audience(db: &SqlitePool, ch: &Channel) -> R<Vec<Id>> {
    match ch.space_id {
        Some(space) => space_members(db, space).await,
        None => Ok(ch.recipients.clone()),
    }
}

/// Upload tokens of every attachment in the given channels (to delete files before a cascade).
pub async fn attachment_tokens_in_channels(db: &SqlitePool, channels: &[Id]) -> R<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT up.token FROM uploads up JOIN messages m ON m.id = up.message_id
         WHERE m.channel_id IN (SELECT value FROM json_each(?))",
    )
    .bind(json_ids(channels))
    .fetch_all(db)
    .await?)
}

// ---------------------------------------------------------------- messages

macro_rules! msg_select {
    ($tail:literal) => {
        concat!(
            "SELECT id, channel_id, author_id, kind, content, edited_at, reply_to, reply_ping, mention_everyone, pinned_at, nonce ",
            "FROM messages ",
            $tail
        )
    };
}
pub(crate) use msg_select;

#[derive(FromRow)]
pub struct MsgRow {
    pub id: Id,
    pub channel_id: Id,
    pub author_id: Id,
    pub kind: String,
    pub content: String,
    pub edited_at: Option<i64>,
    pub reply_to: Option<Id>,
    pub reply_ping: bool,
    pub mention_everyone: bool,
    pub pinned_at: Option<i64>,
    pub nonce: Option<String>,
}

#[derive(FromRow)]
pub struct UploadRow {
    pub id: Id,
    pub message_id: Option<Id>,
    pub token: String,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// Set when the file lives in Nextcloud.
    pub url: Option<String>,
    pub link: Option<String>,
}

impl UploadRow {
    pub fn attachment(&self) -> Attachment {
        Attachment {
            id: self.id,
            filename: self.filename.clone(),
            content_type: self.content_type.clone(),
            size: self.size as u64,
            width: self.width.map(|w| w as u32),
            height: self.height.map(|h| h as u32),
            url: self
                .url
                .clone()
                .unwrap_or_else(|| file_url(&self.token, &self.filename)),
            link: self.link.clone(),
        }
    }
}

/// Rows → full messages, with attachments, reactions and reply previews in three batched queries.
pub async fn hydrate(db: &SqlitePool, rows: Vec<MsgRow>) -> R<Vec<Message>> {
    if rows.is_empty() {
        return Ok(vec![]);
    }
    let ids: Vec<Id> = rows.iter().map(|r| r.id).collect();
    let ids_json = json_ids(&ids);

    let mut files: HashMap<Id, Vec<Attachment>> = HashMap::new();
    for up in sqlx::query_as::<_, UploadRow>(
        "SELECT id, message_id, token, filename, content_type, size, width, height, url, link FROM uploads
         WHERE message_id IN (SELECT value FROM json_each(?)) ORDER BY id",
    )
    .bind(&ids_json)
    .fetch_all(db)
    .await?
    {
        files.entry(up.message_id.unwrap_or_default()).or_default().push(up.attachment());
    }

    let mut reactions: HashMap<Id, Vec<Reaction>> = HashMap::new();
    for (mid, emoji, user) in sqlx::query_as::<_, (Id, String, Id)>(
        "SELECT message_id, emoji, user_id FROM reactions
         WHERE message_id IN (SELECT value FROM json_each(?)) ORDER BY created_at, user_id",
    )
    .bind(&ids_json)
    .fetch_all(db)
    .await?
    {
        let list = reactions.entry(mid).or_default();
        match list.iter_mut().find(|r| r.emoji == emoji) {
            Some(r) => r.users.push(user),
            None => list.push(Reaction {
                emoji,
                users: vec![user],
            }),
        }
    }

    let reply_ids: Vec<Id> = rows.iter().filter_map(|r| r.reply_to).collect();
    let mut replies: HashMap<Id, ReplyPreview> = HashMap::new();
    if !reply_ids.is_empty() {
        for (id, author_id, content, has_attachments) in
            sqlx::query_as::<_, (Id, Id, String, bool)>(
                "SELECT m.id, m.author_id, substr(m.content, 1, 200),
                    EXISTS(SELECT 1 FROM uploads u WHERE u.message_id = m.id)
             FROM messages m WHERE m.id IN (SELECT value FROM json_each(?))",
            )
            .bind(json_ids(&reply_ids))
            .fetch_all(db)
            .await?
        {
            replies.insert(
                id,
                ReplyPreview {
                    id,
                    author_id,
                    content,
                    has_attachments,
                    deleted: false,
                },
            );
        }
    }

    let mut threads: HashMap<Id, ThreadPreview> = HashMap::new();
    for (id, starter, name, message_count, last_message_id, archived) in
        sqlx::query_as::<_, (Id, Id, String, i64, Option<Id>, bool)>(
            "SELECT c.id, c.starter_id, c.name, (SELECT COUNT(*) FROM messages m WHERE m.channel_id = c.id),
                    c.last_message_id, c.archived
             FROM channels c WHERE c.starter_id IN (SELECT value FROM json_each(?))",
        )
        .bind(&ids_json)
        .fetch_all(db)
        .await?
    {
        let p = ThreadPreview { id, name, message_count: message_count as u32, last_message_id, archived };
        threads.insert(starter, p);
    }

    Ok(rows
        .into_iter()
        .map(|r| {
            let reply = r.reply_to.map(|id| {
                replies.get(&id).cloned().unwrap_or(ReplyPreview {
                    id,
                    author_id: 0,
                    content: String::new(),
                    has_attachments: false,
                    deleted: true,
                })
            });
            let mut mentions = parse_user_mentions(&r.content);
            // A reply with "@ ON" mentions the author it answers.
            if let Some(rp) = reply
                .as_ref()
                .filter(|rp| r.reply_ping && !rp.deleted && rp.author_id != r.author_id)
                && !mentions.contains(&rp.author_id)
            {
                mentions.push(rp.author_id);
            }
            Message {
                id: r.id,
                channel_id: r.channel_id,
                author_id: r.author_id,
                kind: from_s(&r.kind),
                mentions,
                created_at: id_time(r.id),
                edited_at: r.edited_at,
                reply,
                attachments: files.remove(&r.id).unwrap_or_default(),
                reactions: reactions.remove(&r.id).unwrap_or_default(),
                mention_everyone: r.mention_everyone,
                pinned: r.pinned_at.is_some(),
                thread: threads.remove(&r.id),
                nonce: r.nonce,
                content: r.content,
            }
        })
        .collect())
}

pub async fn message(db: &SqlitePool, channel: Id, id: Id) -> R<Message> {
    let row = sqlx::query_as::<_, MsgRow>(msg_select!("WHERE id = ? AND channel_id = ?"))
        .bind(id)
        .bind(channel)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| not_found("message"))?;
    Ok(hydrate(db, vec![row]).await?.remove(0))
}

// ---------------------------------------------------------------- read states

const MENTIONS_SQL: &str =
    "SELECT m.channel_id, COUNT(*) FROM pings p JOIN messages m ON m.id = p.message_id
     LEFT JOIN read_states r ON r.user_id = p.user_id AND r.channel_id = m.channel_id
     WHERE p.user_id = ? AND m.id > COALESCE(r.last_read_id, 0) GROUP BY m.channel_id";

pub async fn read_states(db: &SqlitePool, user: Id) -> R<Vec<ReadState>> {
    let mut states: HashMap<Id, ReadState> = HashMap::new();
    for (channel_id, last_read_id) in sqlx::query_as::<_, (Id, Id)>(
        "SELECT channel_id, last_read_id FROM read_states WHERE user_id = ?",
    )
    .bind(user)
    .fetch_all(db)
    .await?
    {
        states.insert(
            channel_id,
            ReadState {
                channel_id,
                last_read_id,
                mention_count: 0,
            },
        );
    }
    for (channel_id, count) in sqlx::query_as::<_, (Id, i64)>(MENTIONS_SQL)
        .bind(user)
        .fetch_all(db)
        .await?
    {
        states
            .entry(channel_id)
            .or_insert(ReadState {
                channel_id,
                last_read_id: 0,
                mention_count: 0,
            })
            .mention_count = count as u32;
    }
    let mut out: Vec<ReadState> = states.into_values().collect();
    out.sort_by_key(|s| s.channel_id);
    Ok(out)
}

pub async fn read_state(db: &SqlitePool, user: Id, channel_id: Id) -> R<ReadState> {
    let last_read_id: Id = sqlx::query_scalar(
        "SELECT last_read_id FROM read_states WHERE user_id = ? AND channel_id = ?",
    )
    .bind(user)
    .bind(channel_id)
    .fetch_optional(db)
    .await?
    .unwrap_or(0);
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pings p JOIN messages m ON m.id = p.message_id
         WHERE p.user_id = ? AND m.channel_id = ? AND m.id > ?",
    )
    .bind(user)
    .bind(channel_id)
    .bind(last_read_id)
    .fetch_one(db)
    .await?;
    Ok(ReadState {
        channel_id,
        last_read_id,
        mention_count: count as u32,
    })
}

pub async fn set_read(db: &SqlitePool, user: Id, channel_id: Id, last_read_id: Id) -> R<()> {
    sqlx::query(
        "INSERT INTO read_states (user_id, channel_id, last_read_id) VALUES (?, ?, ?)
         ON CONFLICT (user_id, channel_id) DO UPDATE SET last_read_id = excluded.last_read_id",
    )
    .bind(user)
    .bind(channel_id)
    .bind(last_read_id)
    .execute(db)
    .await?;
    Ok(())
}

/// Mark every channel of a space read (on join), so old history doesn't show up as unread.
pub async fn mark_space_read(db: &SqlitePool, user: Id, space: Id) -> R<()> {
    sqlx::query(
        "INSERT INTO read_states (user_id, channel_id, last_read_id)
         SELECT ?, id, COALESCE(last_message_id, 0) FROM channels WHERE space_id = ?
         ON CONFLICT (user_id, channel_id) DO UPDATE SET last_read_id = excluded.last_read_id",
    )
    .bind(user)
    .bind(space)
    .execute(db)
    .await?;
    Ok(())
}
