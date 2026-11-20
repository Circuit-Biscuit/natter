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

/// Space permissions (Discord-style bit flags). A member has the space's `everyone_permissions` plus every
/// permission of every role they hold; Administrator (and owning the space) means all of them.
pub mod perm {
    pub const ADMINISTRATOR: u64 = 1;
    pub const MANAGE_SPACE: u64 = 1 << 1;
    pub const MANAGE_ROLES: u64 = 1 << 2;
    pub const MANAGE_CHANNELS: u64 = 1 << 3;
    pub const KICK_MEMBERS: u64 = 1 << 4;
    pub const BAN_MEMBERS: u64 = 1 << 5;
    pub const CREATE_INVITES: u64 = 1 << 6;
    pub const MANAGE_NICKNAMES: u64 = 1 << 7;
    pub const MANAGE_MESSAGES: u64 = 1 << 8;
    pub const MANAGE_THREADS: u64 = 1 << 9;
    pub const MENTION_EVERYONE: u64 = 1 << 10;
    pub const SEND_MESSAGES: u64 = 1 << 11;
    pub const ATTACH_FILES: u64 = 1 << 12;
    pub const ADD_REACTIONS: u64 = 1 << 13;
    pub const CREATE_THREADS: u64 = 1 << 14;
    pub const CONNECT: u64 = 1 << 15;
    pub const VIEW_CHANNEL: u64 = 1 << 16;
    pub const ALL: u64 = (1 << 17) - 1;
    /// What `@everyone` may do in a new space.
    pub const EVERYONE_DEFAULT: u64 =
        VIEW_CHANNEL | CREATE_INVITES | SEND_MESSAGES | ATTACH_FILES | ADD_REACTIONS | CREATE_THREADS | CONNECT;
    /// What a channel's permission overrides may change.
    pub const CHANNEL: u64 = VIEW_CHANNEL
        | MANAGE_CHANNELS
        | MANAGE_ROLES
        | CREATE_INVITES
        | SEND_MESSAGES
        | ATTACH_FILES
        | ADD_REACTIONS
        | CREATE_THREADS
        | MENTION_EVERYONE
        | MANAGE_MESSAGES
        | MANAGE_THREADS
        | CONNECT;

    /// For settings screens: (flag, name, what it allows, group).
    pub const LIST: [(u64, &str, &str, &str); 17] = [
        (ADMINISTRATOR, "Administrator", "Every permission, and every channel. Hand it out with care.", "Advanced"),
        (VIEW_CHANNEL, "View channels", "See channels and read their messages. Turn it off in a channel to make it private.", "General"),
        (MANAGE_SPACE, "Manage space", "Rename the space, change its icon, see and revoke everyone's invites.", "General"),
        (MANAGE_ROLES, "Manage roles", "Create, edit and hand out roles below their own highest role.", "General"),
        (MANAGE_CHANNELS, "Manage channels", "Create, edit, reorder and delete channels and categories.", "General"),
        (CREATE_INVITES, "Create invites", "Invite new people to the space.", "Membership"),
        (MANAGE_NICKNAMES, "Manage nicknames", "Change other members' nicknames.", "Membership"),
        (KICK_MEMBERS, "Kick members", "Remove members below them; they can rejoin with an invite.", "Membership"),
        (BAN_MEMBERS, "Ban members", "Remove members below them for good, and see the ban list.", "Membership"),
        (SEND_MESSAGES, "Send messages", "Write in text channels and threads.", "Text"),
        (ATTACH_FILES, "Attach files", "Upload files and share them from Nextcloud.", "Text"),
        (ADD_REACTIONS, "Add reactions", "React to messages with emoji.", "Text"),
        (CREATE_THREADS, "Create threads", "Start threads from messages.", "Text"),
        (MENTION_EVERYONE, "Mention @everyone and @here", "Ping the whole space at once.", "Text"),
        (MANAGE_MESSAGES, "Manage messages", "Delete and pin anyone's messages, post in announcement channels.", "Text"),
        (MANAGE_THREADS, "Manage threads", "Rename, archive and delete anyone's threads.", "Text"),
        (CONNECT, "Connect", "Join voice channels.", "Voice"),
    ];

    /// What a member may do: everyone's permissions plus their roles'; owners and administrators may do it all.
    pub fn effective(owner: bool, everyone: u64, roles: impl IntoIterator<Item = u64>) -> u64 {
        if owner {
            return ALL;
        }
        let p = roles.into_iter().fold(everyone, |a, r| a | r);
        if p & ADMINISTRATOR != 0 { ALL } else { p & ALL }
    }

    pub fn default_everyone() -> u64 {
        EVERYONE_DEFAULT
    }
}

/// A role in a space. Higher `position` = higher in the list = more senior; members show the colour of
/// their highest coloured role. `@everyone` isn't one of these: it's the space's `everyone_permissions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleDef {
    pub id: Id,
    pub name: String,
    /// `#rrggbb`, or empty for none.
    #[serde(default)]
    pub color: String,
    /// One emoji shown next to the name, or empty.
    #[serde(default)]
    pub emoji: String,
    pub position: i64,
    pub permissions: u64,
    /// Show members with this role in their own group in the member list.
    #[serde(default)]
    pub hoist: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub user_id: Id,
    /// Owner, Admin (has Administrator through a role) or Member. Kept for older apps; `roles` is the truth.
    pub role: Role,
    pub nickname: Option<String>,
    pub joined_at: Millis,
    /// Role ids (see `Space::roles`).
    #[serde(default)]
    pub roles: Vec<Id>,
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
    /// Highest position first.
    #[serde(default)]
    pub roles: Vec<RoleDef>,
    /// What every member may do (`@everyone`).
    #[serde(default = "perm::default_everyone")]
    pub everyone_permissions: u64,
}

impl Space {
    /// What `user` may do here (0 if not a member).
    pub fn permissions(&self, user: Id) -> u64 {
        let Some(m) = self.members.iter().find(|m| m.user_id == user) else { return 0 };
        perm::effective(
            self.owner_id == user,
            self.everyone_permissions,
            self.roles.iter().filter(|r| m.roles.contains(&r.id)).map(|r| r.permissions),
        )
    }

    /// Position of `user`'s highest role (0 = none); the owner outranks everyone.
    pub fn rank(&self, user: Id) -> i64 {
        if self.owner_id == user {
            return i64::MAX;
        }
        let Some(m) = self.members.iter().find(|m| m.user_id == user) else { return -1 };
        self.roles.iter().filter(|r| m.roles.contains(&r.id)).map(|r| r.position).max().unwrap_or(0)
    }

    /// `user`'s roles, highest first.
    pub fn roles_of(&self, user: Id) -> Vec<&RoleDef> {
        let Some(m) = self.members.iter().find(|m| m.user_id == user) else { return vec![] };
        self.roles.iter().filter(|r| m.roles.contains(&r.id)).collect()
    }

    /// The colour of `user`'s highest coloured role.
    pub fn color_of(&self, user: Id) -> Option<&str> {
        self.roles_of(user).into_iter().find(|r| !r.color.is_empty()).map(|r| r.color.as_str())
    }

    /// The category, then the channel itself, whose overrides apply to `ch` (threads go by their parent).
    fn override_chain<'a>(&'a self, ch: &'a Channel) -> Vec<&'a Channel> {
        let find = |id: Option<Id>| id.and_then(|id| self.channels.iter().find(|c| c.id == id));
        let ch = if ch.kind == ChannelKind::Thread { find(ch.parent_id).unwrap_or(ch) } else { ch };
        let mut chain: Vec<&Channel> = find(ch.parent_id).filter(|c| c.kind == ChannelKind::Category).into_iter().collect();
        chain.push(ch);
        chain
    }

    /// What `user` may do in `ch` (a channel of this space): their space permissions with the channel's
    /// overrides applied. 0 means they can't see it.
    pub fn channel_permissions(&self, user: Id, ch: &Channel) -> u64 {
        let base = self.permissions(user);
        if base & perm::ADMINISTRATOR != 0 || base == 0 {
            return base;
        }
        let held: Vec<Id> = self.members.iter().find(|m| m.user_id == user).map(|m| m.roles.clone()).unwrap_or_default();
        let mut p = base;
        for c in self.override_chain(ch) {
            p = apply_overwrites(p, &c.overwrites, self.id, |o| match o.kind {
                OverwriteKind::Role => held.contains(&o.id),
                OverwriteKind::Member => o.id == user,
            });
        }
        if p & perm::VIEW_CHANNEL == 0 { 0 } else { p }
    }

    /// `ch` is hidden from `@everyone` (a private channel or category).
    pub fn is_private(&self, ch: &Channel) -> bool {
        let mut p = self.everyone_permissions;
        for c in self.override_chain(ch) {
            p = apply_overwrites(p, &c.overwrites, self.id, |_| false);
        }
        p & perm::VIEW_CHANNEL == 0
    }

    /// The members who can see `ch`.
    pub fn viewers(&self, ch: &Channel) -> Vec<Id> {
        self.members.iter().map(|m| m.user_id).filter(|&u| self.channel_permissions(u, ch) != 0).collect()
    }
}

/// Discord's order: `@everyone`'s override, then all of the member's roles' overrides together, then the
/// member's own. `applies` says which role / member overrides are theirs.
fn apply_overwrites(mut p: u64, ows: &[Overwrite], space: Id, applies: impl Fn(&Overwrite) -> bool) -> u64 {
    if let Some(o) = ows.iter().find(|o| o.kind == OverwriteKind::Role && o.id == space) {
        p = (p & !(o.deny & perm::CHANNEL)) | (o.allow & perm::CHANNEL);
    }
    let (mut allow, mut deny) = (0, 0);
    for o in ows.iter().filter(|o| o.kind == OverwriteKind::Role && o.id != space && applies(o)) {
        allow |= o.allow;
        deny |= o.deny;
    }
    p = (p & !(deny & perm::CHANNEL)) | (allow & perm::CHANNEL);
    if let Some(o) = ows.iter().find(|o| o.kind == OverwriteKind::Member && applies(o)) {
        p = (p & !(o.deny & perm::CHANNEL)) | (o.allow & perm::CHANNEL);
    }
    p
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
    /// Permission overrides (Discord's "channel permissions"). A channel in a category gets the category's
    /// first, then its own; threads go by their parent channel.
    #[serde(default)]
    pub overwrites: Vec<Overwrite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverwriteKind {
    /// A role, or `@everyone` when `id` is the space's id.
    Role,
    Member,
}

/// Allow or deny permissions in one channel for a role, `@everyone` or one member; anything in neither is
/// inherited.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Overwrite {
    pub id: Id,
    pub kind: OverwriteKind,
    pub allow: u64,
    pub deny: u64,
}

/// Body of `PUT /api/channels/{id}/permissions/{target}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SetOverwrite {
    pub kind: Option<OverwriteKind>,
    pub allow: u64,
    pub deny: u64,
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
    /// A space's roles or `@everyone` permissions changed (the whole list, highest first). Members keep only
    /// role ids that are still in it.
    RolesUpdate {
        space_id: Id,
        roles: Vec<RoleDef>,
        everyone_permissions: u64,
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
    /// What `@everyone` may do (needs Manage roles).
    #[serde(default)]
    pub everyone_permissions: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateRole {
    pub name: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub emoji: String,
    #[serde(default)]
    pub permissions: u64,
    #[serde(default)]
    pub hoist: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateRole {
    pub name: Option<String>,
    /// `Some("")` clears it.
    pub color: Option<String>,
    /// `Some("")` clears it.
    pub emoji: Option<String>,
    pub permissions: Option<u64>,
    pub hoist: Option<bool>,
}

/// The new order of a space's roles, highest first; every role must be listed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReorderRoles {
    pub ids: Vec<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateChannel {
    pub name: String,
    pub kind: ChannelKind,
    pub parent_id: Option<Id>,
    #[serde(default)]
    pub topic: String,
    /// Start with these permission overrides (e.g. a private channel).
    #[serde(default)]
    pub overwrites: Vec<Overwrite>,
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
    fn permissions_rank_and_colour() {
        let role = |id, position, permissions, color: &str| RoleDef {
            id, name: format!("r{id}"), color: color.into(), emoji: String::new(), position, permissions, hoist: false,
        };
        let member = |user_id, roles: Vec<Id>| Member { user_id, role: Role::Member, nickname: None, joined_at: 0, roles };
        let space = Space {
            id: 1, name: "s".into(), icon: None, owner_id: 10, channels: vec![], users: vec![],
            members: vec![member(10, vec![]), member(11, vec![100]), member(12, vec![101, 102]), member(13, vec![])],
            roles: vec![role(100, 3, perm::ADMINISTRATOR, ""), role(101, 2, perm::KICK_MEMBERS, ""), role(102, 1, 0, "#23a55a")],
            everyone_permissions: perm::SEND_MESSAGES,
        };
        assert_eq!(space.permissions(10), perm::ALL, "owner");
        assert_eq!(space.permissions(11), perm::ALL, "administrator");
        assert_eq!(space.permissions(12), perm::SEND_MESSAGES | perm::KICK_MEMBERS);
        assert_eq!(space.permissions(13), perm::SEND_MESSAGES, "only @everyone");
        assert_eq!(space.permissions(99), 0, "not a member");
        assert_eq!((space.rank(10), space.rank(11), space.rank(12), space.rank(13)), (i64::MAX, 3, 2, 0));
        assert_eq!(space.color_of(12), Some("#23a55a"), "highest coloured role");
        assert_eq!(space.color_of(11), None);
    }

    #[test]
    fn private_channels() {
        let ch = |id, kind, parent: Option<Id>, overwrites: Vec<Overwrite>| Channel {
            id, space_id: Some(1), kind, name: format!("c{id}"), topic: String::new(), position: 0, parent_id: parent,
            read_only: false, last_message_id: None, recipients: vec![], thread: None, overwrites,
        };
        let ow = |id, kind, allow, deny| Overwrite { id, kind, allow, deny };
        let hide = ow(1, OverwriteKind::Role, 0, perm::VIEW_CHANNEL);
        let member = |user_id, roles: Vec<Id>| Member { user_id, role: Role::Member, nickname: None, joined_at: 0, roles };
        let space = Space {
            id: 1, name: "s".into(), icon: None, owner_id: 10, users: vec![],
            members: vec![member(10, vec![]), member(11, vec![100]), member(12, vec![]), member(13, vec![101])],
            roles: vec![
                RoleDef { id: 100, name: "staff".into(), color: String::new(), emoji: String::new(), position: 2, permissions: 0, hoist: false },
                RoleDef { id: 101, name: "admin".into(), color: String::new(), emoji: String::new(), position: 1, permissions: perm::ADMINISTRATOR, hoist: false },
            ],
            everyone_permissions: perm::EVERYONE_DEFAULT,
            channels: vec![
                ch(2, ChannelKind::Category, None, vec![hide.clone(), ow(100, OverwriteKind::Role, perm::VIEW_CHANNEL, 0)]),
                ch(3, ChannelKind::Text, Some(2), vec![]),
                ch(4, ChannelKind::Text, None, vec![hide, ow(12, OverwriteKind::Member, perm::VIEW_CHANNEL, perm::SEND_MESSAGES)]),
                ch(5, ChannelKind::Text, None, vec![ow(100, OverwriteKind::Role, 0, perm::SEND_MESSAGES)]),
            ],
        };
        let c = |id| space.channels.iter().find(|c| c.id == id).unwrap().clone();
        let mut thread = ch(6, ChannelKind::Thread, Some(4), vec![]);
        thread.thread = None;
        // A private category hides its channels, except from the role it lets in, admins and the owner.
        assert_eq!(space.viewers(&c(3)), vec![10, 11, 13]);
        assert!(space.is_private(&c(3)) && space.is_private(&c(2)) && !space.is_private(&c(5)));
        // A member let into a private channel sees it, and their own deny wins over @everyone's allow.
        assert_eq!(space.viewers(&c(4)), vec![10, 12, 13]);
        assert_eq!(space.channel_permissions(12, &c(4)) & perm::SEND_MESSAGES, 0);
        assert_eq!(space.viewers(&thread), vec![10, 12, 13], "threads follow their channel");
        // A role's deny takes away what @everyone may do; admins keep everything.
        assert_eq!(space.channel_permissions(11, &c(5)) & perm::SEND_MESSAGES, 0);
        assert_ne!(space.channel_permissions(12, &c(5)) & perm::SEND_MESSAGES, 0);
        assert_eq!(space.channel_permissions(13, &c(5)), perm::ALL);
    }

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
