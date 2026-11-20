# Changelog

## [Unreleased]

- **Android app** (Tauri, same Rust UI): APK for Android 7+, microphone/camera/notification permissions, keyboard-aware layout.
- **Phone layout** (app and mobile web): one screen at a time — channels, chat, then threads, members and search on
  top; back arrow and Android back button step back; send button; long-press menus; safe-area padding.
- **Camera selection** in Settings › Voice & video, with a live preview; switching mid-call swaps the camera instantly.
- Web app can live under a sub-path (e.g. `/natter/`); Nextcloud sign-in accepts invite links; with Nextcloud on and
  sign-ups closed, no local accounts at all.

## [0.1.0] — 2026-11-20

First version.

- **Chat:** spaces with text, voice and announcement channels in categories; direct messages and groups; replies (with or
  without a ping), reactions, edits, deletes, pins; `@user`, `@everyone`, `@here` and `#channel` mentions with
  autocomplete; Discord-flavoured Markdown; full-text search; uploads by drag and drop, paste or picker.
- **Threads:** start one from any message (or the channel header), open it beside the channel, see a live card under
  the original message and the thread under its channel in the sidebar; archive by hand or automatically after 7 quiet
  days; browse archived threads.
- **Unreads:** NEW marker, mention badges per channel, space, DM and taskbar; mark read / unread; synced across devices.
- **Presence:** online, automatic idle, do not disturb, invisible, custom status; typing indicators.
- **Calls:** voice channels and DM calls through the built-in SFU; camera, screen sharing with audio, mute, deafen,
  push to talk, speaking indicators, focus and full screen.
- **Nextcloud:** sign in with Nextcloud (Login Flow v2), names and avatars from Nextcloud, attachments stored in the
  sender's Nextcloud and shared by link, share files from Nextcloud, optional group restriction and auto-join space.
- **Admin:** invites with expiry and use limits, owner / admin / member roles, kick, ban, space and channel settings.
- **Desktop app:** tray with close-to-tray, notifications, taskbar badge, global push to talk, single instance,
  remembered window size; Windows installer.
