# Natter vs. Nextcloud Talk — feature map

Source: `nextcloud/spreed` at commit `f240551` (Talk 26.0.0-dev, 2026-11-20): `docs/capabilities.md`,
`docs/constants.md` and the API docs (`conversation.md`, `chat.md`, `call.md`, `participant.md`, `reaction.md`,
`poll.md`, `breakout-rooms.md`, `webinar.md`, `bots.md`). Talk's code was read for features only; none of it is used.

✅ in Natter · 🔁 in Natter, done differently · ⬜ not yet · ➖ deliberately left out

## Conversations

| Talk | Natter | Notes |
|---|---|---|
| One-to-one conversations | ✅ DMs | reused, never duplicated |
| Group conversations | 🔁 group DMs (up to 10) **and** spaces with channels | Discord model: a space holds many channels |
| Public conversations (link, guests, password) | 🔁 invite links with expiry and max uses | no anonymous guests yet ⬜ |
| Note to self | ⬜ | a DM with yourself works (open your own profile › Message) |
| Rename, description | ✅ channel name and topic (Markdown) | |
| Read-only conversations | ✅ announcement channels | only owners/admins post |
| Lobby / webinar mode | ⬜ | |
| Listable / open conversations | ✅ a space's channels are open to its members; private channels and categories for chosen roles and members | |
| Favorites, archive, important, sensitive, tags | 🔁 mute per channel/space; categories | favorites/archive ⬜ |
| Breakout rooms | ⬜ | voice channels cover the everyday case |
| Federation (other Nextclouds), Matrix, Matterbridge | ➖ for now | |
| Retention / message expiration | ⬜ | |
| Conversation presets (announcement, classified…) | 🔁 announcement channels | |

## Chat

| Talk | Natter | Notes |
|---|---|---|
| Send, Markdown messages | ✅ Discord-flavoured Markdown, rendered without HTML injection | |
| Mentions (`@user`, `@all`, mention permissions) | ✅ `@user`, `@everyone`, `@here`, `#channel`; only admins ping everyone | |
| Replies (`chat-replies`, `private-reply`) | ✅ replies with optional ping | private reply ⬜ |
| Reactions (+ react permission) | ✅ any emoji, see who reacted | |
| Edit, delete (unlimited) | ✅ edit own; delete own or as admin; Shift skips the confirmation | |
| Read markers, read status, mark unread | ✅ NEW line, Alt+click mark unread, synced across devices | read receipts for others ⬜ |
| Unread / mention counters | ✅ per channel, space, DM, taskbar | |
| Typing indicators (+ typing privacy) | ✅ | privacy toggle ⬜ |
| Silent send | ✅ `@silent` prefix | |
| Pinned messages | ✅ | |
| Search (unified search) | ✅ full-text (SQLite FTS5) per space or DM | |
| Shared items overview (media / files tab) | ⬜ | search with `has_file` exists in the API |
| File sharing (from Files / upload) | ✅ upload to the sender's Nextcloud, or share from Nextcloud | |
| Voice messages, location, Deck cards, rich objects | ⬜ / ➖ | |
| Polls | ⬜ | |
| Threads | ✅ Discord-style: from a message or the channel, side panel, sidebar, auto-archive after 7 days | |
| Reminders ("remind me later"), scheduled messages | ⬜ | |
| Link previews (reference provider) | ⬜ | server-side fetch needs SSRF protection first |
| Translations, AI summary | ➖ | |
| Bots (webhook bots), commands | ⬜ | the REST API is bot-friendly (ids fit in JS numbers) |
| Clear history | ➖ | delete the channel instead |

## Calls

| Talk | Natter | Notes |
|---|---|---|
| Audio / video calls | ✅ | |
| Screen sharing | ✅ with screen audio | |
| Signaling | 🔁 built-in gateway + data-channel renegotiation | Talk: polling or separate HPB |
| Media topology | 🔁 SFU in the server | Talk: mesh, or Janus via HPB |
| Raise hand, emoji reactions in calls | ⬜ | |
| Call permissions (start/join/publish audio/video/screen) | 🔁 anyone in the channel may join and publish | ⬜ granular |
| Force mute | ⬜ | |
| Recording, live transcription | ➖ | |
| SIP dial-in/out | ➖ | |
| Call notifications / ringing | ✅ DMs ring | |
| Background blur | ⬜ | |
| End-to-end encryption | ⬜ | media is encrypted in transit; the SFU sees it |
| Push to talk | ✅ (Talk has it in the desktop client) | global hotkey in the desktop app |

## Participants and permissions

| Talk | Natter | Notes |
|---|---|---|
| Owner / moderator / user / guest | ✅ Discord-style roles with 17 permissions, @everyone defaults, hierarchy, private channels and per-channel overrides | guests ⬜ |
| Promote, demote, transfer ownership | ✅ | |
| Remove, ban | ✅ kick, ban (with message cleanup) | |
| Notification levels (always / mention / never) | 🔁 mute per channel or space; DMs and mentions notify | |
| User status (online, away, DND, custom) | ✅ + invisible, automatic idle | |
| Avatars | ✅ from Nextcloud or uploaded | |

## Platform

| Talk | Natter | Notes |
|---|---|---|
| Web app inside Nextcloud | 🔁 web app served by the Natter server | |
| Desktop (Electron "Talk Desktop") | 🔁 Tauri shell + Rust UI | far smaller, native tray/notifications/PTT |
| Mobile apps | ⬜ | web app works in mobile browsers |
| Nextcloud login | ✅ Login Flow v2 (any Nextcloud, any SSO behind it) | |
| Files integration | ✅ | |
| Calendar integration, dashboard widgets, Flow | ➖ | |
