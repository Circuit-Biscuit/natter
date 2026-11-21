# Changelog

## [Unreleased]

- **Automatic updates for the desktop apps.** Natter looks for a new release on GitHub when it starts and installs
  it before the window opens. While it's running it checks every six hours, downloads the update in the background
  and shows a green **Update** button in the title bar. Natter never restarts by itself while open, so a call is
  never cut off; during a call the button asks first. Updates are signed, and Natter refuses any it can't verify.
  Installs from this version on update themselves; earlier ones need this version installed by hand once.
- **Android:** a banner says when a newer version is out and links to its APK. Android installs it after one tap,
  because apps from outside a store can't update themselves.
- **Android:** full-screen windows (Daily Word, Settings) no longer slide under the status bar and gesture bar; the
  incoming-call card and messages at the bottom keep clear of them too.

## [0.6.1] — 2026-11-20

- Desktop: the Windows title bar came back above Natter's own when updating from an earlier version (the saved
  window state remembered the old frame). Only size, position and maximised are remembered now.
- Natter is free software under the GNU AGPL v3.0 or later. The deploy guide covers servers without a Nextcloud.

## [0.6.0] — 2026-11-20

- **Daily Word:** a five-letter word game in English or Turkish, from the grid button at the top of any channel. One
  puzzle a day per language, the same for everyone; the answer stays on the server. Turkish keyboard with ç ğ ı İ ö ş
  ü, hard mode, high-contrast colours, flip / shake / bounce animations, a countdown to the next word.
- Finishing posts your spoiler-free grid to the space's Daily Word channel (new setting in Space Settings ›
  Overview); at 09:00 the **Daily Word bot** posts yesterday's results there, @mentioning everyone who played.
- Stats (played, win %, streaks, guess distribution), who has played today, and week / month / all-time standings.
- App accounts such as the bot show a **BOT** tag.
- **Space profiles:** "Edit Space Profile" in a space's menu sets the name and status you show in that space only
  (empty = your usual one). Shown in messages, the member list, profiles and your panel at the bottom left. Older
  apps still show the nickname; they show your usual status.
- **Desktop app without the Windows title bar:** a slim bar in Natter's own colours instead, with minimise,
  maximise and close (close still keeps Natter in the tray unless that's off in Settings › System). Drag it to move
  the window, double-click it to maximise; the edges still resize.
- **Camera bitrate cap:** cameras send at most 1 Mbit/s in calls (about half the browser default), set by the server
  (`NATTER_CAMERA_KBPS`, 0 = off) through the SDP it hands the apps — no app update needed. Screen shares are unchanged.
- **Space icons and avatars uploaded while signed in with Nextcloud** showed as broken images: they were stored in
  the uploader's Nextcloud but looked for on the server. Existing ones now load from their Nextcloud link; new ones
  stay on the server.
- Server: a page with "&" just before non-English text no longer stops the link-preview catch-up (fixed live).

## [0.5.0] — 2026-11-20

- **Link previews (embeds), like Discord's:** YouTube links get a card with the video's title, channel and
  thumbnail that plays right there; other links get the page's site, title, description and picture; links straight
  to a picture or video show it inline. Wrap a link in `<…>` to send it without a preview. Messages from before
  (and the history brought over from Talk) get their previews too. Settings › Appearance › "Show link previews" turns
  them off.
- Previews are fetched once by the server and cached, never from inside your network: only public addresses on the
  standard ports, redirects re-checked, small and quick downloads.

## [0.4.1] — 2026-11-20

- Phones: with a colour theme the app fell back to the desktop layout (channel list and chat side by side) once
  your synced settings arrived. One screen at a time again.

## [0.4.0] — 2026-11-20

- **Private channels and categories:** switch "Private" on when creating a channel or category, or later in its
  Permissions, and choose the roles and members who may see it; admins and the owner always can. A private
  category hides every channel in it. Members who can't see a channel don't get it, its messages, its calls, its
  threads or its search results, and they aren't in its member list; letting someone in or out updates their
  sidebar at once.
- **Channel permissions, like Discord's:** for @everyone, any role or any member, set each channel permission
  (view, send, attach, react, threads, @everyone, manage messages / threads / channel, invites, connect) to
  allow, inherit or deny. Channels get their category's settings first, then their own; threads follow their channel.
- New **View channels** permission (on for @everyone); private channels show a lock in the sidebar.
- Sign-in: arriving with an invite on a server that allows Natter accounts offers one straight away.

## [0.3.0] — 2026-11-20

- **Roles, like Discord's:** create roles with a name, colour and emoji; switch 16 permissions on or off (manage
  space, roles, channels, nicknames, messages, threads; kick, ban, invite; send, attach, react, start threads,
  mention @everyone; join calls; Administrator); set what @everyone may do; drag to reorder; show a role's members as
  their own group in the member list; names take the colour of their highest role. People with Manage roles can only
  touch roles below their own and can't hand out permissions they don't have.
- **Make anyone admin:** every space has an Admin role; "Make Admin" / "Remove Admin" in a member's menu, and roles
  can be added or removed right from a profile. Existing admins were moved onto the Admin role automatically.
- **Notifications that get noticed:** a red number (mentions and DMs) or red dot (anything else unread) on the
  taskbar and tray icon and on the browser tab; our own notification sound, also for the test notification (the
  pop-ups themselves are silent so nothing doubles up); choose between four sounds; notify for all messages (new
  default) or only DMs and mentions; flash the taskbar or not.
- **Appearance:** light, dark, darker and midnight themes or follow the computer; 14 colour themes (gradients);
  nine accent colours; ten app icons (taskbar, tray, tab); cozy / compact, space between message groups, font
  scaling, zoom (desktop).
- **Accessibility:** saturation, reduce motion, always underline links.
- **Keybinds:** add your own (mute, deafen, camera, screen share, leave call, show / hide Natter); global in the
  desktop app. Push to talk now also works right after start-up, not only after visiting Settings.
- **System (desktop):** open Natter when the computer starts, start minimized to the tray, choose whether the close
  button minimizes to the tray.

## [0.2.0] — 2026-11-20

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
