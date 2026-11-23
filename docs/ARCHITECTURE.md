# Natter architecture

```
 ┌──────────── natter-desktop (Tauri 2, Rust) ────────────┐        ┌────────── natter-server (Rust) ──────────┐
 │ tray · notifications · push-to-talk · single instance │        │ axum HTTP API ── SQLite (WAL, FTS5)       │
 │  ┌────────── natter-app (Leptos → WASM) ───────────┐  │ HTTPS  │      │                                    │
 │  │ UI · store · REST client · gateway client       │──┼───────►│ /api/*        REST, JSON, Bearer token     │
 │  │ glue.js: WebRTC in the system webview           │──┼─ WSS ─►│ /gateway      live events (WebSocket)      │
 │  └─────────────────────────────────────────────────┘  │        │ /files/*      uploads (capability URLs)    │
 └────────────────────────────────────────────────────────┘ UDP   │ SFU (str0m)   one UDP port, all calls      │
                         ▲───────────────── SRTP ─────────┴──────►└──────────────────────────────────────────┘
```

| Crate | Path | What |
|---|---|---|
| `natter-proto` | `crates/proto` | Every wire type (REST bodies, gateway events, ids). Compiled into server and app, so they cannot drift. |
| `natter-server` | `crates/server` | One binary: HTTP API, WebSocket gateway, SQLite storage, file store, SFU for calls. Optionally serves the web build of the app. |
| `natter-app` | `crates/app` | The whole UI, in Rust (Leptos, client-side rendered, compiled to WASM by `trunk`). Runs in the desktop shell or any browser. |
| `natter-desktop` | `crates/desktop` | Tauri 2 shell around the app: native window, tray, notifications, global push-to-talk, single instance, installers. |

## Server

### Configuration (environment)

| Variable | Default | Meaning |
|---|---|---|
| `NATTER_BIND` | `0.0.0.0:8080` | HTTP listen address (put TLS in front: Caddy / nginx) |
| `NATTER_DATA` | `./natter-data` | Data directory: `natter.db` + `files/` |
| `NATTER_NAME` | `Natter` | Server name shown on the sign-in screen |
| `NATTER_REGISTRATION` | `invite` | `open` · `invite` (a valid invite code is needed) · `closed`. The first account can always be created and becomes admin. |
| `NATTER_MAX_UPLOAD_MB` | `100` | Upload size limit |
| `NATTER_MEDIA_PORT` | `50000` | UDP port for all call media |
| `NATTER_CAMERA_KBPS` | `1000` | Most a camera sends in a call, kbit/s (`b=AS`/`b=TIAS` in the SDP the server hands out); `0` = no cap |
| `NATTER_GAME_UTC_OFFSET` | `60` | Daily Word's time zone, minutes ahead of UTC: when the day's word changes and when the 09:00 summary goes out |
| `NATTER_PUBLIC_IP` | auto (outbound interface) | IP announced to call clients; set it to the public IP on a server |
| `NATTER_WEB_DIR` | unset | Serve the built app (`crates/app/dist`) at `/` |
| `NATTER_NEXTCLOUD_URL` | unset | Nextcloud to sign in with and store files in, e.g. `https://cloud.example.com` |
| `NATTER_NEXTCLOUD_GROUP` | unset | Only members of this Nextcloud group may sign in with Nextcloud |
| `NATTER_NEXTCLOUD_FOLDER` | `Natter` | Folder in each user's Nextcloud that receives their uploads |
| `NATTER_AUTOJOIN_SPACE` | unset | Space id every Nextcloud user joins on first sign-in (the company space) |
| `RUST_LOG` | `natter_server=info` | Log filter |

### Storage

SQLite in WAL mode via `sqlx`, migrations embedded in the binary (`crates/server/migrations`). Message search is an FTS5
external-content table kept in sync by triggers. Uploads live in `files/<token>`; metadata in `uploads`.

Ids are `natter_proto::Id`: `(ms since 2026-01-01) << 10 | seq`, generated in-process, time-ordered (pagination and
"unread since" are id comparisons), and below 2^53.

### Auth

Argon2id password hashes. A session token is 32 random bytes (hex); only its SHA-256 is stored. Every device has its
own session (listed and revocable under Settings › Devices). Failed logins are throttled per username (10 per 10 min).
CORS allows any origin: tokens travel in the `Authorization` header, never in cookies.

### Permissions

| Action | Owner | Admin | Member | DM participant |
|---|---|---|---|---|
| Read & post (not in read-only channels) | ✓ | ✓ | ✓ | ✓ |
| Post in read-only (announcement) channels | ✓ | ✓ | – | – |
| React, reply, join calls, create invites | ✓ | ✓ | ✓ | ✓ |
| Edit own messages / delete own messages | ✓ | ✓ | ✓ | ✓ |
| Delete others' messages, pin, `@everyone`/`@here` pings | ✓ | ✓ | – | ✓ (pins, @everyone) |
| Manage channels, space name & icon, invites of others | ✓ | ✓ | – | – |
| Kick / ban (not owner; admins only members) | ✓ | ✓ | – | – |
| Promote / demote admins, delete space, transfer ownership | ✓ | – | – | – |

### Gateway

`GET /gateway?token=…` upgrades to a WebSocket. Frames are JSON text: server events `{"t": "...", "d": ...}`
(`natter_proto::Event`), client ops `{"op": "...", "d": ...}` (`natter_proto::Op`).

1. Server sends `Hello { heartbeat_ms }`, then `Ready` with the complete state the app needs (you, every user you share a
   space or DM with, spaces with channels and members, DMs, read states, presences, voice states, settings).
2. The app sends `Heartbeat` every `heartbeat_ms`; the server answers `HeartbeatAck` and closes connections silent for
   2 × `heartbeat_ms`.
3. **Reconnects always get a fresh `Ready`** and the app refetches the open channel's newest messages and merges them.
   No resume protocol: `Ready` is small for team-sized servers, and a fresh snapshot can never be subtly wrong.

Who receives what:

| Event | Audience |
|---|---|
| Message*, Reaction*, Channel*, Member*, Space*, VoiceState*/VoiceLeave | members of the space, or the DM's recipients |
| TypingStart | same, minus the typist |
| PresenceUpdate, UserUpdate | everyone who shares a space or DM with the user |
| ReadStateUpdate, SettingsUpdate | the user's own sessions (keeps devices in sync) |
| CallRing | DM recipients except the caller |

Presence is computed: offline without a gateway connection; otherwise the chosen status (`invisible` shows as offline),
with `online` turning `idle` while the app reports `Idle(true)`.

### Messages, unreads, mentions

* `POST …/messages` with a `nonce`: a retry with the same nonce returns the first message (unique index on
  `author_id, nonce`), so flaky networks never double-post. The created message is broadcast with the nonce echoed back
  so the sender swaps its optimistic copy.
* `<@id>` mentions are parsed server-side; `@everyone` / `@here` count only for mods (spaces) and anyone (DMs);
  `silent: true` skips pings. Each ping bumps the recipient's `mention_count` in `read_states`.
* `POST …/ack { message_id }` sets the read marker. Moving it back marks the channel unread from there; the mention count
  is recomputed. Sending a message acks your own channel.
* Rate limit: 5 messages/s per user, burst 10 (HTTP 429 with `retry_after_ms`).

### Files

`POST /api/uploads` takes the raw body (header `X-Filename`, `Content-Type`) and returns an `Attachment`. Messages, avatars
and space icons reference upload ids; uploads unused after 24 h are deleted. Files are served at
`/files/<random 128-bit token>/<filename>` (capability URLs, like Discord's CDN, so `<img>` tags work without headers).
Only images, video, audio and plain text are served inline; everything else is `Content-Disposition: attachment`
with `X-Content-Type-Options: nosniff`, and SVG is never inline.

### Nextcloud

Natter runs next to a Nextcloud and uses it for **who people are** and **where files live**. Chat, presence and calls stay
in Natter's own server, which is what keeps them fast.

**Sign in with Nextcloud** (any Nextcloud, using its standard [Login Flow v2](https://docs.nextcloud.com/server/latest/developer_manual/client_apis/LoginFlow/index.html)):

1. App → `POST /api/auth/nextcloud` → server starts a flow at `{nc}/index.php/login/v2` (User-Agent `Natter`, so the
   app password shows up as "Natter" under the user's Nextcloud security settings) → returns `NcLoginStart`.
2. The app opens `login_url` in the system browser. The user signs in to Nextcloud as always (for us: Stalwart).
3. App polls `POST /api/auth/nextcloud/poll { flow }` every 2 s → `null` until approved, then `AuthResp`. The server
   polls Nextcloud, gets `{loginName, appPassword}`, verifies them with `GET {nc}/ocs/v2.php/cloud/user` (checks
   `NATTER_NEXTCLOUD_GROUP`), then creates or updates the Natter user linked by Nextcloud user id: username from the id,
   display name and custom avatar from Nextcloud (refreshed at every sign-in). New users join `NATTER_AUTOJOIN_SPACE`.
   Nextcloud sign-in ignores `NATTER_REGISTRATION` (Nextcloud already vouches for the person).
4. The app password is kept with that Natter session (server DB, never sent to the app) so the server can store files
   for the user. Signing out or revoking the session deletes the app password in Nextcloud too.

**Files.** When the uploading session is a Nextcloud session, `POST /api/uploads` streams the file into
`{NATTER_NEXTCLOUD_FOLDER}/{space or "Direct messages"}/{name}` in the sender's Nextcloud (WebDAV `PUT`, never overwriting:
`name (2).ext`), creates a read-only public link share, and returns an `Attachment` whose `url` is the share's direct
download URL and `link` the share page. Images get their dimensions read before upload. If Nextcloud refuses (quota,
link sharing disabled) the file is stored locally instead, so sending never fails because of Nextcloud. Deleting the
message deletes the share (the file stays in the sender's Nextcloud). Local sessions keep using local storage.

**Share from Nextcloud.** `GET /api/nextcloud/files?path=/` lists a folder (WebDAV `PROPFIND`, depth 1);
`POST /api/nextcloud/share { path }` creates (or reuses) a public link for that file and returns an `Attachment` to send
like any upload.

### Calls (SFU)

One UDP port, one thread, many [`str0m`](https://github.com/algesten/str0m) `Rtc` instances, demultiplexed with
`rtc.accepts()` — the design of str0m's `chat` example, extended with rooms (one per voice channel or DM) and user ids.

* Join: `POST /api/channels/{id}/voice` with the app's SDP offer and a map `mid → TrackKind`. The app always opens the
  same transceivers (`mic`, `cam`, `screen`, `screen_audio`, all send-only) plus a data channel, so turning the camera
  or screen share on and off is `replaceTrack()` — no renegotiation, instant.
* Quality is per sender, also without renegotiation: screen shares take their frame rate from the capture
  (`applyConstraints`) and their size, frame rate, bitrate and `degradationPreference` from
  `RTCRtpSender.setParameters()` (`applyStream` in glue.js; "auto" bitrate = 0.1 bit per pixel per frame, 1–15
  Mbit/s). Cameras get size / frame rate constraints and an optional `maxBitrate`, still under the server's
  `b=AS` camera cap, which `ServerInfo.camera_kbps` tells the apps. Every viewer gets the same stream (no simulcast).
* The server adds a send-only m-line on every other participant's connection for each new track, with msid stream id
  `"<user_id>-<track kind>"` (msid tokens can't contain `:`), and renegotiates over the data channel (offer from server, answer from app).
* Keyframe requests (PLI/FIR) are routed back to the publisher, throttled to 1/s.
* A dead connection (ICE timeout) or `DELETE /api/voice` removes the voice state and broadcasts `VoiceLeave`.
* Speaking indicators are computed in the app from audio levels; nothing goes over the network.

### Server notes

* Code: `crates/server/src/` — `lib.rs` (config, state, router), `auth.rs`, `gateway.rs` (hub, presence, Ready),
  `spaces.rs`, `messages.rs` (also uploads/files/search), `voice.rs` + `sfu.rs`, `nextcloud.rs`, `db.rs`; schema in
  `migrations/`. Test: `cargo test -p natter-server` (one end-to-end test with a fake Nextcloud).
* `POST /api/uploads` takes an optional `X-Channel-Id` header; Nextcloud sessions use it to pick the folder
  (space name or `Direct messages`; without it, `Uploads`). `X-Filename` may be percent-encoded UTF-8.
* 429 bodies carry `retry_after_ms` (`ApiError`).
* Mention counts are never stored: they are counted from a `pings` table (who was actually pinged), so deletes and
  mark-unread can't leave them wrong.
* Threads are channel rows of kind `thread` (`parent_id` = text channel, `space_id` = its space), so messages,
  reactions, pins, acks, typing and search work in them unchanged. Starter messages carry a `ThreadPreview`.
  `Space.channels` (Ready, SpaceCreate) leaves out archived threads; posting in one unarchives it; threads quiet for
  7 days are archived hourly. Message counts are counted, not stored. Members may post in threads of read-only channels.
* SFU signalling on the data channel is str0m's JSON: `{"type":"offer","sdp":...}` / `{"type":"answer","sdp":...}`,
  the same shape as `RTCSessionDescription.toJSON()`.

### Routes

Public: `GET /api/info` · `POST /api/auth/register` · `POST /api/auth/login` · `POST /api/auth/nextcloud` ·
`POST /api/auth/nextcloud/poll` · `GET /api/invites/{code}` ·
`GET /files/{token}/{name}` · `GET /gateway?token=` · `GET /health`.

Everything else needs `Authorization: Bearer <token>`:

| Method | Path | Body → Response |
|---|---|---|
| POST | `/api/auth/logout` | – → 204 |
| GET | `/api/me/sessions` | → `[SessionInfo]` |
| DELETE | `/api/me/sessions/{id}` | → 204 (that device is signed out: `InvalidSession`) |
| PATCH | `/api/me` | `UpdateMe` → `User` |
| DELETE | `/api/me` | `DeleteAccount` (password, or `confirm` on a Nextcloud session) → 204; 409 while you own a space. Deletes your messages, uploads, memberships, reactions, pings, games, settings, sessions; the `users` row stays as an anonymous "Deleted User" |
| PUT | `/api/me/password` | `ChangePassword` → 204 (signs out your other devices) |
| PUT | `/api/me/settings` | any JSON → 204 |
| POST | `/api/uploads` | raw bytes → `Attachment` |
| POST | `/api/spaces` | `CreateSpace` → `Space` (with categories, `#general` and a `General` voice channel) |
| PATCH / DELETE | `/api/spaces/{id}` | `UpdateSpace` → 204 / → 204 |
| GET / POST | `/api/spaces/{id}/invites` | → `[Invite]` / `CreateInvite` → `Invite` |
| POST / DELETE | `/api/invites/{code}` | join → `Space` / revoke → 204 |
| PATCH / DELETE | `/api/spaces/{id}/members/{user_id}` | `UpdateMember` (nickname: yours, or below you with Manage nicknames; `custom_status`: only your own, shown in that space instead of your usual one) → 204 / leave (yourself) or kick → 204 |
| POST / PUT | `/api/spaces/{id}/roles` | `CreateRole` → `RoleDef` / `ReorderRoles` (all ids, highest first) → 204 |
| PATCH / DELETE | `/api/spaces/{id}/roles/{role_id}` | `UpdateRole` → 204 / → 204 |
| PUT / DELETE | `/api/spaces/{id}/members/{user_id}/roles/{role_id}` | give / take a role → 204 |
| PUT / DELETE | `/api/channels/{id}/permissions/{target_id}` | `SetOverwrite` (allow / deny for a role, `@everyone` = the space id, or a member) → 204 |
| GET | `/api/games/word?lang=en\|tr&space_id=` | → `word::Overview` (today's board, players today in that space, your stats, standings) |
| POST | `/api/games/word/guess` | `word::GuessReq` → `word::Overview`; 400 for a word not in the list or a hard-mode miss, 409 once finished |
| GET | `/api/spaces/{id}/bans` | → `[Ban]` |
| PUT / DELETE | `/api/spaces/{id}/bans/{user_id}` | `BanReq` → 204 / → 204 |
| POST | `/api/spaces/{id}/channels` | `CreateChannel` → `Channel` |
| PATCH / DELETE | `/api/channels/{id}` | `UpdateChannel` → `Channel` / → 204. Threads: only `name` and `archived`, by the thread's starter or an admin (who may also delete it). Deleting a text channel deletes its threads. |
| GET / POST | `/api/channels/{id}/threads` | → `[Channel]` (all threads of a text channel, archived too, latest activity first) / `CreateThread` → `Channel` (409 if the message already has one) |
| POST | `/api/dms` | `CreateDm` → `Channel` |
| GET | `/api/channels/{id}/messages?before=&after=&around=&limit=` | → `[Message]`, oldest first, limit ≤ 100 (default 50) |
| POST | `/api/channels/{id}/messages` | `SendMessage` → `Message` |
| PATCH / DELETE | `/api/channels/{id}/messages/{mid}` | `EditMessage` → `Message` / → 204 |
| PUT / DELETE | `/api/channels/{id}/messages/{mid}/reactions/{emoji}` | → 204 |
| GET | `/api/channels/{id}/pins` | → `[Message]`, newest pin first |
| PUT / DELETE | `/api/channels/{id}/pins/{mid}` | → 204 |
| POST | `/api/channels/{id}/ack` | `Ack` → 204 |
| GET | `/api/search?q=&space_id=&channel_id=&author_id=&has_file=&before=&limit=` | → `SearchResult` |
| POST | `/api/channels/{id}/voice` | `VoiceJoin` → `VoiceAnswer` |
| PATCH / DELETE | `/api/voice` | `VoiceUpdate` → 204 / leave → 204 |
| POST | `/api/channels/{id}/ring` | → 204 (DMs: ring the others) |
| GET | `/api/nextcloud/files?path=` | → `[NcFile]` (Nextcloud sessions only) |
| POST | `/api/nextcloud/share` | `NcShare` → `Attachment` |

Errors are `{"error": "..."}` with a fitting status (400 bad input, 401 no/invalid token, 403 not allowed, 404 not found
or not visible, 409 conflict, 413 too large, 429 rate limited).

## App

Leptos 0.8, client-side rendered, built by `trunk` into `crates/app/dist`.

| Module | What |
|---|---|
| `api.rs` | `fetch` wrapper: JSON in/out, Bearer token, typed errors |
| `gateway.rs` | WebSocket client: heartbeats, reconnect with jittered backoff, idle detection, dispatch into the store |
| `store.rs` | All state as signals: users, spaces, channels, messages per channel, read states, presence, voice, typing, UI |
| `markdown.rs` | Discord-flavoured Markdown → view nodes (never `innerHTML`, so messages cannot inject script into the shell) |
| `ui/*` | Rail, channel list, chat, composer, member list, popouts, menus, modals, settings, call view |
| `glue.js` + `voice.rs`, `sound.rs` | The only JavaScript: WebRTC (devices, the one peer connection to the SFU, tracks, speaking detection, push-to-talk), synthesized sounds, Tauri bridge |
| `desktop.rs` | Bridge to the Tauri shell (notifications, badge, push-to-talk), with browser fallbacks |

## Desktop shell

Tauri 2. The window loads the app's `dist`. Native extras exposed as commands/events: OS notifications, taskbar flash and
badge, tray icon with "show / quit", close-to-tray, single instance, global push-to-talk key (pressed/released events), and
remembered window size. The webview provides WebRTC (WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux).

**Updates** (`tauri-plugin-updater`, release builds only). Each release carries `latest.json`: the version, plus one
installer URL and minisign signature per platform (`windows-x86_64`, `linux-x86_64` for the AppImage,
`linux-x86_64-deb`, `darwin-*`). It is written by the publish job in `release.yml` from the `.sig` files the bundler
makes when `createUpdaterArtifacts` is on.

- At launch, before the window shows, the shell checks (6 s timeout per request). If a newer version downloads in
  time, it installs at once. Windows runs the NSIS installer in passive mode, which starts the new Natter with the
  same arguments, so a tray start stays in the tray. macOS and Linux replace the app and restart.
- While running, the app calls `check_update` a minute after start and then every 6 h. The shell downloads in the
  background (no timeout) and keeps the bytes. The app then shows **Update** in the title bar, which calls
  `install_update`. If Natter is in a call, it asks first.
- Android has no updater. The app reads `api.github.com/repos/…/releases/latest`, compares `tag_name` with its own
  version and shows a banner linking to the `.apk` asset.
- Development builds (`debug_assertions`) never update, so test copies can't replace an installed Natter.

## Roles and permissions

Discord-style. A space has `everyone_permissions` (`@everyone`) and any number of roles (`RoleDef`: name, colour,
emoji, position, permission bits, hoist). A member may do what `@everyone` may plus what each of their roles may
(`perm::effective`); Administrator, and owning the space, means everything. The bits live in `natter_proto::perm`
with names and descriptions for settings screens, and the same `Space::permissions` / `Space::rank` run on the
server (as two small queries in `db::perms`) and in the app (to show only what you may use).

Hierarchy: you manage roles below your own highest role (`rank`), never grant permissions you lack, and kick, ban
or rename only members ranked below you; the owner outranks everyone. New roles start at the bottom; reordering
must keep the roles at or above yours where they are. Every space starts with an "Admin" role (Administrator);
"Make admin" in the member menu gives it. Changes go out as `RolesUpdate` (the whole list) and `MemberUpdate`.
`Member.role` (owner / admin / member) is still filled in for apps from before roles.

Spaces from before roles were upgraded at start-up (`db::upgrade_roles`): each got the Admin role and old admins
got it. In DMs everyone may chat, share files, react, ping the group, pin and call, but not delete others' messages.

### Channel permissions and private channels

Each channel carries `overwrites`: allow / deny bits for `@everyone` (target id = the space id), a role or a
member, limited to `perm::CHANNEL`. `Space::channel_permissions` starts from the member's space permissions and
applies, in Discord's order, `@everyone`'s override, all their roles' overrides together, then their own — first
for the channel's category, then for the channel (threads use their parent channel). Administrators and the owner
skip overrides. Without View channels the result is 0: the channel doesn't exist for them.

The server enforces that everywhere a channel can reach a client: `db::access` answers 404, `db::audience` (who
gets message, thread, typing, reaction, voice and channel events) is only the members who can see it, `Ready` and
`SpaceCreate` carry only visible channels (`db::visible_to`), and search leaves out hidden channels and their
threads. When a permission change alters which channels someone can see (an override, a role's permissions, a role
given or taken, `@everyone`, moving a channel between categories), each affected member gets the space again as
they now see it (`SpaceCreate`, which apps treat as replace).

### Link previews

`embeds.rs`. After a message is sent or edited, the server picks up to four http(s) links (not in code, not
`<wrapped>`, not into the configured Nextcloud), builds a preview for each and sends the message again
(`MessageUpdate`) with `embeds`. YouTube links become a card with the oEmbed title/channel, the `i.ytimg.com`
thumbnail and a `youtube-nocookie.com` player URL (the app shows the thumbnail and swaps in the player on click);
pages become cards from OpenGraph / Twitter / `<title>` tags; links that answer with `image/*` or `video/*` are shown
as such. Previews are stored per message (`message_embeds`) and per URL (`embed_cache`: a day, failures an hour).
At start-up older messages with links get previews in the background (newest 2000, quietly).
Links to KLIPY's media servers (`static*.klipy.com`, what the GIF picker sends) are never fetched: they become
picture (or video) previews straight away. A message whose whole text is one picture or video link shows only the
picture, as in Discord.

### GIFs

`app/src/ui/gif.rs`. The server only hands out the KLIPY app key (`ServerInfo.klipy_key` from `NATTER_KLIPY_KEY`,
in `Ready`, so to signed-in people only; the public `/api/info` leaves it out);
the app calls `api.klipy.com` itself, because KLIPY's terms require requests and media loads to come from the user's
app, and keep results in their order. The picker shows category tiles (fetched once per run), Trending, Favorites
(`Settings.fav_gifs`, synced like every setting) and search (300 ms after typing stops), 24 at a time with more on
scroll, in two columns filled shortest-first. Picking a GIF sends its full-size animated WebP link as its own
message and reports the share to KLIPY (`POST …/gifs/share/{slug}`, with the search words, no user id).

Fetching typed URLs is guarded against server-side request forgery: http(s) on ports 80/443 only, no credentials in
the URL, every address the host resolves to must be public (no loopback, private, link-local, CGNAT, multicast,
documentation, IPv4-mapped private), the connection is pinned to the checked address, redirects are followed by hand
and re-checked (at most 4), 3 s to connect, 5 s in all, 512 KiB of HTML at most.

### Daily Word

`games.rs`, with the shared rules in `natter_proto::word` (scoring with repeated letters, hard mode, Turkish
case folding). Word lists are compiled in from `crates/server/words/` (`{lang}_answers.txt`, `{lang}_allowed.txt`,
`{lang}_extra_allowed.txt`). At start-up each language's answers are shuffled with a random seed made once per
server and kept in `game_meta`, so day *n* is answer *n* of that order and nobody can work it out from the repo. Day
1 is 2026-11-20 in the game's time zone.

Plays are kept per user, language and day (`word_plays`: the guesses as JSON, hard mode, solved, finished time); the
answer only leaves the server once that player has finished. A finished game posts the spoiler-free grid as the
player's own message to the space's `game_channel_id` (if set and they may post there). Every five minutes after
09:00 local, each space with a game channel that has no `game_summaries` row for yesterday gets one message from the
Daily Word bot (a `bot` user with no password, added to the space on first use) listing yesterday's players with
`<@id>` mentions. Points: 7 − tries for a win, 0 for a loss; stats and streaks (consecutive days
won) are per language, standings add both.
