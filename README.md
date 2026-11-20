<div align="center">

<img src="assets/logo.svg" width="112" alt="Natter logo">

# Natter

**Team chat, voice and video — fast like Discord, at home on your own Nextcloud.**

A self-hosted chat server with apps for Windows, macOS, Linux and Android — built in Rust.

[![Rust](https://img.shields.io/badge/pure-Rust-b7410e?logo=rust&logoColor=white)](#how-its-built)
[![Nextcloud](https://img.shields.io/badge/works%20with-Nextcloud-0082c9?logo=nextcloud&logoColor=white)](#made-for-nextcloud)
[![Calls](https://img.shields.io/badge/calls-WebRTC%20SFU-4b56d2)](#voice-and-video-that-scale)
[![Platforms](https://img.shields.io/badge/platforms-Windows%20·%20macOS%20·%20Linux%20·%20Android%20·%20Web-444)](#downloads)
[![Status](https://img.shields.io/badge/status-alpha-orange)](#feature-status)

<img src="docs/screenshot-chat.png" width="100%" alt="Natter: space rail, channel list, a conversation with mentions, replies and reactions, and the member list">

<sub><i>A space with text and voice channels: mentions, replies, reactions, Markdown and live presence.</i></sub>

[Chat](#chat-that-feels-instant) · [Calls](#voice-and-video-that-scale) · [Nextcloud](#made-for-nextcloud) · [Shortcuts](#the-little-things) · [vs. Talk](#natter-and-nextcloud-talk) · [Downloads](#downloads) · [Run a server](#run-a-server) · [Build](#quick-start) · [Changelog](CHANGELOG.md)

</div>

---

## Chat that feels instant

Messages appear the moment you press Enter, before the server has even replied, and quietly retry if the network
hiccups. A retried message can never post twice.

- **Spaces, channels, categories** — text channels, voice channels and announcement channels, grouped in collapsible categories.
- **Direct messages and groups** — one-to-one or up to ten people, with calls built in.
- **Threads** — spin a side conversation off any message; it opens beside the channel, shows a live card under the original message, sits under its channel in the sidebar, and archives itself after a quiet week.
- **Replies, reactions, edits, pins** — reply with or without a ping, react with any emoji, edit with ↑, pin what matters.
- **Mentions that work** — `@person`, `@everyone`, `@here` and `#channel` autocomplete as you type; mentions light up and count on every unread badge.
- **Markdown** — **bold**, *italic*, __underline__, ~~strike~~, `code`, code blocks with a copy button, quotes, headings, lists, ||spoilers||, masked links, jumbo emoji.
- **Unreads done right** — a red **NEW** line where you left off, "jump to present", mark as read with Esc, mark unread with Alt+click; read state syncs across your devices.
- **Search** — full-text search across a space or a conversation, results beside the chat.
- **Files** — drag and drop, paste screenshots, or pick from Nextcloud; images, video and audio play inline.
- **Presence** — online, idle (automatic), do not disturb, invisible, plus a custom status. "Ada is typing…".

## Voice and video that scale

<a href="docs/screenshot-call.png"><img src="docs/screenshot-call.png" width="100%" alt="A call in a voice channel with a live camera tile and speaking indicators"></a>

<sub><i>A voice channel: camera tiles, speaking rings, and who's connected — also shown in the channel list.</i></sub>

Calls go through Natter's own **SFU** (selective forwarding unit) built into the server. Everyone sends their audio
and video once, and the server forwards it to everyone else. Nextcloud Talk without its separate "high-performance
backend" connects every participant to every other, which falls apart beyond four or five people.

- **Voice channels** you drop into and out of, Discord-style, and **calls in any DM** with ringing.
- **Camera, screen sharing (with sound), mute, deafen**, speaking indicators, focus any tile, full screen.
- **Push to talk** — even while Natter is in the background (desktop app) — or voice activity with noise suppression, echo cancellation and gain control.
- **One UDP port** for all call media, encrypted in transit (DTLS-SRTP). Calls reconnect by themselves after a network change.

## Made for Nextcloud

Natter runs next to your Nextcloud and uses it for **who people are** and **where files live**. Chat, presence and
calls stay on Natter's own server, which is what keeps them fast.

- **Sign in with Nextcloud** — one click opens your Nextcloud's login page (SSO, 2FA, everything you already use), then you're in. Names and pictures come from Nextcloud.
- **Your files stay yours** — attachments are saved to your own Nextcloud (`Natter/<space>/`) and shared by read-only link. Delete the message and the share goes too; the file stays in your cloud.
- **Share from Nextcloud** — browse your Nextcloud from the message box (**+ › Share from Nextcloud**) and attach any file.
- **Any Nextcloud** — point `NATTER_NEXTCLOUD_URL` at your own Nextcloud; nothing has to be installed on the Nextcloud side.
- **Company space for everyone** — new Nextcloud users can join your main space automatically; restrict sign-in to a Nextcloud group.
- **No Nextcloud? No problem** — Natter has its own accounts and file storage too.

## The little things

| | |
|---|---|
| **Ctrl + K** | Jump to any channel, DM or person |
| **Alt + ↑ / ↓** · **Alt + Shift + ↑ / ↓** | Previous / next channel · next unread channel |
| **↑** in an empty box | Edit your last message |
| **Esc** · **Shift + Esc** | Mark channel read · mark whole space read |
| **Alt + click** | Mark unread from this message |
| **Hold Shift** on a message | Pin and delete without confirmation |
| **Shift + click** emoji | Pick several at once |
| **Ctrl + Shift + M / D** | Mute / deafen |
| **`@silent` …** | Send without notifying anyone |

On phones Natter switches to one screen at a time, like Discord's app: spaces and channels, then the chat, with
threads, members and search on top. Android's back button steps back through them; long-press a message for its menu.

Also: drafts kept per channel; an unread badge with a real number on the taskbar, tray and browser tab; a choice of
notification sounds; light, dark, darker and midnight themes plus 14 colour themes, accent colours and app icons;
compact mode, text size, zoom, reduced motion; your own global keybinds; start with the computer, close to the tray;
devices list with remote sign-out; and right-click menus everywhere.

## Natter and Nextcloud Talk

Natter started from a full read of [Nextcloud Talk](https://github.com/nextcloud/spreed)'s features (v26) — see
[docs/FEATURES.md](docs/FEATURES.md) for the side-by-side. In short: Natter keeps Talk's essentials (conversations,
mentions, replies, reactions, edits, pins, read markers, notification levels, calls with screen sharing,
permissions, Nextcloud identity and files) and changes how they're delivered:

| | Nextcloud Talk | Natter |
|---|---|---|
| Live updates | polling, or a separate Go signaling server | built-in WebSocket gateway, push only |
| Calls | peer-to-peer mesh, or a separate Janus server | SFU built into the server, one UDP port |
| Desktop app | Electron wrapper around the web app | native Rust shell; the UI is Rust too |
| Install | PHP app inside Nextcloud | one binary + SQLite, next to Nextcloud |
| Layout | conversation list | Discord-style spaces, channels, categories, voice channels |

## Feature status

| Area | Status |
|---|---|
| Spaces, text / voice / announcement channels, categories, DMs and groups | ✅ |
| Messages: send, edit, delete, reply, reactions, pins, mentions, Markdown, search | ✅ |
| Threads: from any message or the channel, side panel, sidebar, auto-archive, archived list | ✅ |
| Unread markers, mention counts, read sync across devices | ✅ |
| Presence, idle, DND, invisible, custom status, typing | ✅ |
| Invites (expiry, max uses), kick, ban | ✅ |
| Roles: colours, icons, 16 permissions, @everyone defaults, drag to reorder, hierarchy, one-click admin | ✅ |
| Voice & video calls via SFU, screen share, push to talk | ✅ |
| Sign in with Nextcloud, files in Nextcloud, share from Nextcloud | ✅ |
| Desktop app: tray, notifications, numbered badge, global keybinds, start with the computer | ✅ Windows · 🚧 macOS / Linux untested |
| Android app: phone layout, back button, calls, notifications while open | ✅ (APK, sideload) |
| Background push notifications on Android (app closed) | ⬜ |
| Private channels and categories, per-channel permissions (allow / inherit / deny per role or member) | ✅ |
| Link previews: YouTube plays inline, page cards, pictures and videos (SSRF-guarded, cached) | ✅ |
| Polls, message scheduling | ⬜ |
| Simulcast (adaptive video quality for big calls), TURN / TCP fallback | ⬜ |
| iOS app | ⬜ (the web app works in mobile browsers) |
| Signed / notarised installers | ⬜ |

✅ done · 🚧 in progress · ⬜ not yet

## Downloads

> **Get the latest version:** open **[Releases › Latest](../../releases/latest)** and pick your platform.

| Platform | File | Notes |
|---|---|---|
| **Windows** | `Natter_<version>_x64-setup.exe` | One click, no admin rights. Not code-signed yet: SmartScreen › **More info › Run anyway**. |
| **macOS** (Apple Silicon + Intel) | `Natter_<version>_universal.dmg` | Drag to Applications. Not notarised yet: right-click › **Open** the first time. macOS 11+. |
| **Linux** | `Natter_<version>_amd64.AppImage` / `.deb` | `chmod +x` the AppImage and run it. |
| **Android** 7+ | `Natter_<version>.apk` | Allow "install unknown apps" for your browser once, then open the APK. |
| **Server** | `natter-server-<version>-linux-x86_64.tar.gz` | Static binary plus the web app. |

On first start the app asks for your server's address (for example `https://chat.example.com`).

## Run a server

```sh
NATTER_NEXTCLOUD_URL=https://cloud.example.com \
NATTER_PUBLIC_IP=203.0.113.10 \
./natter-server
```

Put HTTPS in front (Caddy or nginx), open UDP port 50000 for calls, and you're done. The first account becomes
the admin. Every setting is an environment variable — see [docs/DEPLOY.md](docs/DEPLOY.md) for a complete,
copy-pasteable setup next to an existing Nextcloud, including the systemd unit and backups.

## Quick start

Build from source with Rust 1.85 or newer plus `rustup target add wasm32-unknown-unknown` and
`cargo install trunk tauri-cli`:

```sh
git clone <this repository>
cd natter
cd crates/app && trunk build --release && cd ../..   # the UI (Rust → WebAssembly)
powershell -File scripts/dev.ps1                       # server + web app on http://localhost:8080
cd crates/desktop && cargo tauri dev                   # the desktop app against it
```

On Linux install the webview libraries first: `sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev`.

Useful commands:

```sh
cargo test -p natter-proto -p natter-server -p natter-app   # protocol, Markdown, full server test (HTTP, gateway, SFU join, fake Nextcloud)
cd crates/desktop && cargo tauri build        # installers for this platform
cd crates/desktop && cargo tauri android build --apk --target aarch64 --target armv7   # Android APK (needs the Android SDK + NDK)
```

## How it's built

| Layer | Where | What |
|---|---|---|
| Protocol | `crates/proto` | every type the app and server exchange — one crate compiled into both, so they can't drift |
| Server | `crates/server` | axum HTTP API, WebSocket gateway, SQLite (WAL + FTS5), file store, Nextcloud bridge |
| Calls | `crates/server/src/sfu.rs` | SFU on [str0m](https://github.com/algesten/str0m): one UDP socket, rooms, renegotiation, keyframe routing |
| App | `crates/app` | the whole UI in Rust with [Leptos](https://leptos.dev), compiled to WebAssembly by trunk |
| Media glue | `crates/app/glue.js` | the only JavaScript: WebRTC, synthesized sounds, desktop bridge |
| Desktop & Android | `crates/desktop` | [Tauri 2](https://tauri.app) shell: tray, notifications, numbered badge, global keybinds, autostart, installers; the same crate builds the Android app (`gen/android`) |

Design notes, the full API and the gateway protocol: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Releasing a new version

Bump the version and push:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\bump.ps1 0.2.0
```

(or edit `version` under `[workspace.package]` in `Cargo.toml` and push to `main`). First add a `## [0.2.0]` section to
[CHANGELOG.md](CHANGELOG.md): it becomes the release notes. The **Release** workflow notices the new version, builds
the Windows installer, the macOS universal `.dmg`, the Linux AppImage and `.deb`, and the Linux server bundle, and
publishes them as release `v0.2.0`. Pushes that don't change the version are built and tested by **CI**.

The Android APK is signed with a key that never goes to GitHub, so it's built locally (command above) and attached to
the draft while the workflow runs: `gh release upload v0.2.0 Natter_0.2.0.apk` — it then lands in the checksums too.

First-time setup of the repository is a single command: `scripts\publish.ps1` (creates the private repository under
the organization and pushes).

## License and credits

© Circuit Biscuit. All rights reserved — private repository.

Built with Rust, axum, tokio, sqlx/SQLite, str0m, Leptos, Tauri, reqwest and argon2. Feature research from
Nextcloud Talk (AGPL-3.0; no Talk code is used). Emoji data from the `emojis` crate (Unicode / gemoji).
