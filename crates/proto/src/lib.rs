//! Natter wire protocol: every type the server and the app exchange.
//!
//! One crate, compiled into both sides, so the two can never drift apart.
//! REST bodies are JSON; gateway frames are JSON text over a WebSocket.
//! See `docs/ARCHITECTURE.md` for the route table.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Time-ordered id: `(ms since 2026-01-01) << 10 | seq`.
/// Stays below 2^53 for ~278 years, so JavaScript bots can use it as a number too.
pub type Id = i64;

/// Unix milliseconds.
pub type Millis = i64;

pub const EPOCH_MS: Millis = 1_767_225_600_000; // 2026-01-01T00:00:00Z

/// Creation time of any id.
pub fn id_time(id: Id) -> Millis {
    (id >> 10) + EPOCH_MS
}

pub const MAX_MESSAGE_LEN: usize = 4000;
pub const MAX_NAME_LEN: usize = 100;
pub const MAX_ABOUT_LEN: usize = 190;
pub const MAX_TOPIC_LEN: usize = 1024;
pub const MAX_ATTACHMENTS: usize = 10;

// ---------------------------------------------------------------- users

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Online,
    Idle,
    Dnd,
    /// Chosen by the user: looks offline to everyone else.
    Invisible,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: Id,
    pub username: String,
    pub display_name: String,
    /// URL path of the avatar image, relative to the server (`/files/...`).
    pub avatar: Option<String>,
    pub about: String,
    pub custom_status: Option<String>,
    /// Server administrator.
    pub admin: bool,
    pub created_at: Millis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    pub user_id: Id,
    pub status: Status,
}

// ---------------------------------------------------------------- spaces

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Admin,
    Member,
}

impl Role {
    /// Owners and admins manage channels, invites, members and others' messages.
    pub fn is_mod(self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub user_id: Id,
    pub role: Role,
    pub nickname: Option<String>,
    pub joined_at: Millis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub id: Id,
    pub name: String,
    pub icon: Option<String>,
    pub owner_id: Id,
    pub channels: Vec<Channel>,
    pub members: Vec<Member>,
    /// The members' profiles. Only filled where the receiver may not know them yet
    /// (`SpaceCreate`, the reply to creating or joining a space); empty inside `Ready`.
    #[serde(default)]
    pub users: Vec<User>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Text,
    Voice,
    Category,
    Dm,
    GroupDm,
    /// A thread hanging off a message in a text channel (`parent_id` = that channel).
    Thread,
}

impl ChannelKind {
    pub fn is_dm(self) -> bool {
        matches!(self, ChannelKind::Dm | ChannelKind::GroupDm)
    }
    /// Channels that have a call room.
    pub fn has_voice(self) -> bool {
        matches!(self, ChannelKind::Voice | ChannelKind::Dm | ChannelKind::GroupDm)
    }
    /// Channels that have a message history (voice channels have a side chat too).
    pub fn has_messages(self) -> bool {
        !matches!(self, ChannelKind::Category)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    pub id: Id,
    /// `None` for direct messages.
    pub space_id: Option<Id>,
    pub kind: ChannelKind,
    pub name: String,
    pub topic: String,
    pub position: i32,
    /// Category this channel sits in.
    pub parent_id: Option<Id>,
    /// Only owners and admins may post (announcements).
    pub read_only: bool,
    pub last_message_id: Option<Id>,
    /// DM participants (including yourself); empty for space channels.
    pub recipients: Vec<Id>,
    /// Set for threads.
    #[serde(default)]
    pub thread: Option<ThreadInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadInfo {
    /// The message in the parent channel the thread was started from.
    pub starter_id: Id,
    /// Who started it (may rename or archive it, like admins).
    pub owner_id: Id,
    /// Archived threads leave the channel list; posting in one brings it back.
    pub archived: bool,
    pub message_count: u32,
}

/// Summary of a thread, attached to the message it was started from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadPreview {
    pub id: Id,
    pub name: String,
    pub message_count: u32,
    pub last_message_id: Option<Id>,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invite {
    pub code: String,
    pub space_id: Id,
    pub creator_id: Id,
    pub uses: u32,
    pub max_uses: Option<u32>,
    pub expires_at: Option<Millis>,
    pub created_at: Millis,
}

/// Public preview of an invite, readable before signing in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvitePreview {
    pub code: String,
    pub space_name: String,
    pub space_icon: Option<String>,
    pub member_count: u32,
    pub online_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ban {
    pub user_id: Id,
    pub reason: String,
    pub created_at: Millis,
}

// ---------------------------------------------------------------- messages

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    #[default]
    Default,
    /// "X joined the space." Content is empty.
    MemberJoin,
    /// "X pinned a message." `reply` points at the pinned message.
    Pin,
    /// "X started a call." Content holds the call length in seconds once it ended.
    Call,
    /// "X started a thread: <name>." Posted when a thread is started without a message; content = name.
    ThreadCreated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Id,
    pub filename: String,
    pub content_type: String,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// URL path relative to the server (`/files/<token>/<filename>`), or an absolute URL
    /// (a Nextcloud public link) for files stored in Nextcloud.
    pub url: String,
    /// Page to open the file in Nextcloud, when it lives there.
    #[serde(default)]
    pub link: Option<String>,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.content_type.starts_with("image/") && self.width.is_some()
    }
    pub fn is_video(&self) -> bool {
        self.content_type.starts_with("video/")
    }
    pub fn is_audio(&self) -> bool {
        self.content_type.starts_with("audio/")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    /// Who reacted, oldest first.
    pub users: Vec<Id>,
}

/// Short form of the message being replied to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplyPreview {
    pub id: Id,
    pub author_id: Id,
    /// First 200 characters; empty when the original was deleted.
    pub content: String,
    pub has_attachments: bool,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: Id,
    pub channel_id: Id,
    pub author_id: Id,
    pub kind: MessageKind,
    pub content: String,
    pub created_at: Millis,
    pub edited_at: Option<Millis>,
    pub reply: Option<ReplyPreview>,
    pub attachments: Vec<Attachment>,
    pub reactions: Vec<Reaction>,
    /// Users mentioned with `<@id>`.
    pub mentions: Vec<Id>,
    /// `@everyone` or `@here` was used by someone allowed to.
    pub mention_everyone: bool,
    pub pinned: bool,
    /// Echo of the client's nonce, so it can swap its optimistic copy.
    pub nonce: Option<String>,
    /// The thread started from this message, if any.
    #[serde(default)]
    pub thread: Option<ThreadPreview>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadState {
    pub channel_id: Id,
    /// Everything up to and including this id has been read (0 = nothing).
    pub last_read_id: Id,
    /// Unread messages that mention you.
    pub mention_count: u32,
}

// ---------------------------------------------------------------- voice

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceState {
    pub user_id: Id,
    pub channel_id: Id,
    pub muted: bool,
    pub deafened: bool,
    pub video: bool,
    pub screen: bool,
    pub joined_at: Millis,
}

/// What each published m-line carries. The app opens its three transceivers in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Mic,
    Cam,
    Screen,
    /// Audio of a shared screen or window.
    ScreenAudio,
}

impl TrackKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TrackKind::Mic => "mic",
            TrackKind::Cam => "cam",
            TrackKind::Screen => "screen",
            TrackKind::ScreenAudio => "screen_audio",
        }
    }
}

// ---------------------------------------------------------------- gateway

/// Server -> client. Serialized as `{"t": "...", "d": {...}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", content = "d")]
pub enum Event {
    Hello {
        heartbeat_ms: u32,
    },
    Ready(Box<Ready>),
    MessageCreate(Message),
    MessageUpdate(Message),
    MessageDelete {
        id: Id,
        channel_id: Id,
    },
    ReactionAdd {
        channel_id: Id,
        message_id: Id,
        user_id: Id,
        emoji: String,
    },
    ReactionRemove {
        channel_id: Id,
        message_id: Id,
        user_id: Id,
        emoji: String,
    },
    TypingStart {
        channel_id: Id,
        user_id: Id,
    },
    PresenceUpdate(Presence),
    UserUpdate(User),
    SpaceCreate(Space),
    SpaceUpdate {
        id: Id,
        name: String,
        icon: Option<String>,
        owner_id: Id,
    },
    SpaceDelete {
        id: Id,
    },
    ChannelCreate(Channel),
    ChannelUpdate(Channel),
    ChannelDelete {
        id: Id,
        space_id: Option<Id>,
    },
    MemberAdd {
        space_id: Id,
        member: Member,
        user: User,
    },
    MemberUpdate {
        space_id: Id,
        member: Member,
    },
    MemberRemove {
        space_id: Id,
        user_id: Id,
    },
    ReadStateUpdate(ReadState),
    VoiceStateUpdate(VoiceState),
    VoiceLeave {
        user_id: Id,
        channel_id: Id,
    },
    /// Someone started a call in a DM you are part of.
    CallRing {
        channel_id: Id,
        caller_id: Id,
    },
    SettingsUpdate(serde_json::Value),
    /// The token was revoked or the account deleted: sign out.
    InvalidSession,
    HeartbeatAck,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ready {
    pub user: User,
    /// Every user you share a space or a DM with (yourself included).
    pub users: Vec<User>,
    pub spaces: Vec<Space>,
    pub dms: Vec<Channel>,
    pub read_states: Vec<ReadState>,
    pub presences: Vec<Presence>,
    pub voice_states: Vec<VoiceState>,
    /// Your own chosen status (others see `presences`).
    pub status: Status,
    /// Free-form app settings, synced between your devices.
    pub settings: serde_json::Value,
    pub server: ServerInfo,
    /// This session signed in through Nextcloud, so it can share files from there.
    #[serde(default)]
    pub nextcloud: bool,
}

/// Client -> server. Serialized as `{"op": "...", "d": ...}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", content = "d")]
pub enum Op {
    Heartbeat,
    Typing { channel_id: Id },
    /// The app noticed you went away (true) or came back (false).
    Idle(bool),
}

// ---------------------------------------------------------------- REST bodies

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Registration {
    /// Anyone may create an account.
    Open,
    /// An invite code is needed to create an account.
    Invite,
    /// No new accounts.
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub registration: Registration,
    /// No account exists yet: the first one becomes the admin.
    pub first_run: bool,
    pub max_upload_bytes: u64,
    /// Base URL of the Nextcloud you can sign in with (and store files in), if any.
    #[serde(default)]
    pub nextcloud: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterReq {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
    pub invite: Option<String>,
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResp {
    pub token: String,
    pub user: User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: Id,
    pub device: String,
    pub created_at: Millis,
    pub last_seen: Millis,
    pub current: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateMe {
    pub display_name: Option<String>,
    pub about: Option<String>,
    /// Upload id of a new avatar; `Some(0)` removes it.
    pub avatar: Option<Id>,
    pub status: Option<Status>,
    /// `Some("")` clears it.
    pub custom_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePassword {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSpace {
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSpace {
    pub name: Option<String>,
    /// Upload id of a new icon; `Some(0)` removes it.
    pub icon: Option<Id>,
    /// Hand the space to another member (owner only).
    pub owner_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateChannel {
    pub name: String,
    pub kind: ChannelKind,
    pub parent_id: Option<Id>,
    #[serde(default)]
    pub topic: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateChannel {
    pub name: Option<String>,
    pub topic: Option<String>,
    pub position: Option<i32>,
    /// `Some(0)` moves the channel out of its category.
    pub parent_id: Option<Id>,
    pub read_only: Option<bool>,
    /// Threads only.
    #[serde(default)]
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateThread {
    /// 1–100 characters; defaults to the start of the message.
    #[serde(default)]
    pub name: String,
    /// Start it from this message in the channel; `None` posts a "started a thread" message as the starter.
    pub message_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDm {
    /// Other participants; one makes a 1:1 DM (reused if it exists), more make a group.
    pub recipients: Vec<Id>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateInvite {
    pub max_uses: Option<u32>,
    pub expires_in_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateMember {
    pub role: Option<Role>,
    /// `Some("")` clears it.
    pub nickname: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BanReq {
    #[serde(default)]
    pub reason: String,
    /// Also delete their messages from the last N seconds.
    #[serde(default)]
    pub delete_message_secs: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SendMessage {
    pub content: String,
    pub reply_to: Option<Id>,
    #[serde(default)]
    pub attachments: Vec<Id>,
    /// Client-chosen; a retry with the same nonce returns the first message instead of a duplicate.
    pub nonce: Option<String>,
    /// Don't ping mentioned users with a notification.
    #[serde(default)]
    pub silent: bool,
    /// Ping the author of `reply_to` (Discord's "@ ON" for replies).
    #[serde(default)]
    pub reply_ping: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditMessage {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ack {
    /// Read up to here. Setting it to an older id marks the channel unread from there.
    pub message_id: Id,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub space_id: Option<Id>,
    pub channel_id: Option<Id>,
    pub author_id: Option<Id>,
    pub has_file: Option<bool>,
    pub before: Option<Id>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub messages: Vec<Message>,
    pub total: u32,
}

/// Join a call: the app's SDP offer plus what each of its m-lines carries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceJoin {
    pub sdp: String,
    pub tracks: BTreeMap<String, TrackKind>,
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceAnswer {
    pub sdp: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoiceUpdate {
    pub muted: Option<bool>,
    pub deafened: Option<bool>,
    pub video: Option<bool>,
    pub screen: Option<bool>,
}

// ---------------------------------------------------------------- Nextcloud

/// Start of a Nextcloud sign-in: open `login_url` in the browser, then poll with `flow`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NcLoginStart {
    pub login_url: String,
    pub flow: String,
}

/// Poll a Nextcloud sign-in. The reply is `null` until the user approved it in the browser,
/// then an `AuthResp`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NcLoginPoll {
    pub flow: String,
    pub device: Option<String>,
}

/// One entry of your Nextcloud files (`GET /api/nextcloud/files?path=`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NcFile {
    /// Path inside your Nextcloud, starting with `/`.
    pub path: String,
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub modified: Millis,
    pub content_type: String,
}

/// Share one of your Nextcloud files into a message: returns an `Attachment` to send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NcShare {
    pub path: String,
}

/// Error body of every non-2xx response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    /// On 429: how long to wait before retrying.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

// ---------------------------------------------------------------- text helpers

/// Pull `<@123>` user mentions out of message text.
pub fn parse_user_mentions(content: &str) -> Vec<Id> {
    let mut out = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("<@") {
        rest = &rest[start + 2..];
        let end = rest.find('>').unwrap_or(0);
        if let Ok(id) = rest[..end].parse::<Id>()
            && !out.contains(&id)
        {
            out.push(id);
        }
    }
    out
}

/// Usernames: 2-32 of a-z, 0-9, `_` and `.`.
pub fn valid_username(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
}

/// Channel names are lower-case with dashes, like `general` or `release-notes`.
pub fn channel_slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
        } else if (c == '-' || c.is_whitespace()) && !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(MAX_NAME_LEN).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_and_names() {
        assert_eq!(parse_user_mentions("hi <@12> and <@7>, <@12> <@x> <@"), vec![12, 7]);
        assert!(valid_username("alice.k_2"));
        assert!(!valid_username("Alice"));
        assert!(!valid_username("a"));
        assert_eq!(channel_slug("  Release Notes!! 2 "), "release-notes-2");
        assert_eq!(channel_slug("--a  -  b--"), "a-b");
        assert_eq!(id_time(5 << 10), EPOCH_MS + 5);
    }

    #[test]
    fn event_wire_shape() {
        let e = Event::TypingStart { channel_id: 1, user_id: 2 };
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"t":"TypingStart","d":{"channel_id":1,"user_id":2}}"#
        );
        let op: Op = serde_json::from_str(r#"{"op":"Heartbeat"}"#).unwrap();
        assert!(matches!(op, Op::Heartbeat));
    }
}
