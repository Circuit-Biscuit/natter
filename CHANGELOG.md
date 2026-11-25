# Changelog

## [0.12.0] — 2026-11-25

### New

- **Forgot password?** On the sign-in screen: enter your username or e-mail and Natter e-mails you a code (it works
  for 30 minutes). Type it in with a new password and you're signed in, and signed out everywhere else. For this,
  Natter accounts can add an e-mail address when they sign up or in Settings › My account (your password confirms
  it; nobody else sees it). Servers turn it on with their own mail account: `NATTER_SMTP_URL` and
  `NATTER_SMTP_FROM` (docs/DEPLOY.md). Accounts that sign in with Nextcloud reset their password in Nextcloud.
- **Server admins can reset a password**: right-click someone › Reset Password… gives you a temporary password to
  pass on, once, and signs them out everywhere. Works on every server, mail or not.
- **Show password:** every password box has an eye that shows what you typed, so a typo at sign-up can't lock you
  out unseen.
- **AI noise suppression.** Settings › Voice & video › Noise suppression is now Off / Standard / **Strong (AI)**,
  and Strong is the default. It runs your microphone through RNNoise, a small neural network made for calls, on
  your own device (nothing leaves it): keyboard and mouse clicks, fans, traffic and background chatter are taken
  out before anyone hears you, while your voice comes through. It adds about 10 ms. Standard is the old setting,
  the system's own filter, which only takes the edge off steady hum. If you had turned noise suppression off,
  it stays off. A change applies to a call you're in right away.

### Fixed

- Switching microphone or noise suppression during a call could leave your speaking ring lit until you spoke
  again.
- Usernames typed on a Turkish keyboard didn't sign in when they had a capital i: that keyboard's "İ" turned into
  "i" plus an invisible dot, so the name never matched (and signing up with it failed). "İ" and "ı" now count as
  "i" in usernames, on every server running this version, whichever app version signs in. The username box on
  phones no longer capitalizes or spell-checks what you type.

### For people who run or build Natter

- The server's database gains an optional e-mail per account and a table for reset codes (migration 0010); it
  updates itself on start. Reset e-mails stay off until you set `NATTER_SMTP_URL` and `NATTER_SMTP_FROM`.
- Noise suppression lives in `crates/denoise` (RNNoise through nnnoiseless, BSD-3), built to its own small wasm
  module by a Trunk pre-build hook. `trunk build` needs nothing new.

## [0.11.0] — 2026-11-23

### New

- **Natter on Google Play** (in testing for now). The Google Play app updates through Google Play, so it doesn't
  show the "new version" banner; the APK from GitHub keeps it. The two are signed differently, so moving from one
  to the other means uninstalling first.
- **Delete your account** yourself, in Settings › My account › Delete account: confirm with your password (or just
  confirm, if you signed in with Nextcloud) and it's gone at once. Your messages with their files, reactions,
  profile picture, memberships, game results, settings and sign-ins are deleted, and the links Natter made to files
  in your Nextcloud are removed. Like Discord, people who still have a DM or thread with you see "Deleted User",
  and you need to hand over or delete the spaces you own first. The server side is `DELETE /api/me`, so it works
  on every Natter server once it runs this version.
- **Resizable channel list.** Drag its right edge (200–480 px), or focus the edge and use the arrow keys;
  double-click resets it. Each computer or browser remembers its own width.
- **Swipe between screens on phones.** From the chat, swipe right for the channel list and left for the member
  list; swipe left on the channel list to get back to the chat, and right on the member list. Swiping right works
  like the back arrow (it closes an open thread or search first). Swipes that start on a text field or on something
  that scrolls sideways, like a wide code block, are left alone.
- **Privacy policy** linked in Settings › My account; the
  [privacy policy](https://circuit-biscuit.com/privacy/natter/) says what the apps and our own server keep.

### Fixed

- Signing out (or deleting your account) went back to the sign-in screen only after a reload; it stayed on
  "Connecting…" when the app had started signed in.
- The Natter logo at the top of the server list was wider than it was tall (a style meant for another page
  stretched it), so it showed an oval or a wide rounded square when hovered or selected. It's now a circle, like
  in Discord, while spaces keep their rounded-square hover.
- The sign-in screen ran off the sides on phones narrower than the sign-in card (most of them); it now fits the
  screen.
- Android: the app's native code is now aligned for 16 KB memory pages, which newer phones (Android 15+) can use
  and Google Play requires.

### For people who build Natter

- One command for an Android release: `scripts/android-release.ps1` builds the APK and the Google Play bundle,
  attaches the APK to the GitHub draft and uploads the bundle to Google Play with fastlane (lanes `play`,
  `promote`, `listing`). The store listing lives in `fastlane/metadata`. Works with a service-account key or, for
  organisations that forbid key files, with your own gcloud sign-in. See docs/DEPLOY.md › Releasing.
- Android builds no longer run trunk themselves (`tauri.android.conf.json`): build the UI first, as before.

## [0.10.0] — 2026-11-23

### New

- **Stream quality controls for screen sharing**, like Discord's Go Live window but more. **Screen** now opens a
  window before you pick what to share:
  - Presets: **Text & detail** (full screen size at 15 fps, for code, documents and slides), **Balanced** (1080p,
    30 fps) and **Smooth motion** (720p, 60 fps, for games and videos).
  - Or set it yourself: resolution (480p up to your screen's own size), frame rate (5 to 60 fps), bitrate (Auto, or
    up to 20 Mbit/s).
  - What to give up when the connection or computer can't keep up: frames (**Keep it sharp**) or resolution
    (**Keep it smooth**).
  - Whether to share the sound.

  While you're live, the new gear button next to the screen button opens the same window: every change applies at
  once, without picking the window again. It shows what's actually going out (for example "1920×1080 · 30 fps · 4.1
  Mbit/s · VP8") and says when the upload speed or your computer is holding it back. Hover any screen share to see
  the same numbers for what you receive. Your choices are remembered on each device.
- **Camera quality:** Settings › Voice & video › Camera quality sets the resolution (360p to 1080p), frame rate (15,
  30 or 60 fps) and bitrate. A camera that's on changes right away. The server's camera limit (1 Mbit/s unless its
  owner changed `NATTER_CAMERA_KBPS`) still applies, and the settings say what it is.
- **Pick the quality you receive (simulcast).** Cameras and screen shares now go out in up to three sizes at once:
  as the sender set it, 720p and 360p. Everyone in the call gets the one that suits them, so one person on a slow
  connection no longer drags everyone else down, and a fast one isn't held back. Hover someone's camera or screen
  share and choose from the menu in its corner:
  - **Auto** (the default): the server measures what your connection can take and picks for you. It starts on
    the smallest size, so the picture comes up fast, and steps up as it finds room. With several streams it shares
    your bandwidth between them, screen shares first. It steps down at once when your connection struggles.
  - **Source**: the full stream as sent. **720p** or **360p**: save bandwidth, or keep a small tile light.

  Discord picks for you; here you choose, per stream. The switch happens on the stream's next keyframe (about a
  second). Hovering a camera now shows the same numbers as a screen share (resolution, frame rate, bitrate,
  codec), so you can see what you get. Your picks stay through a reconnect. The sender's Go Live and camera settings
  still apply: they set the top size, and the smaller ones follow with the same quality per pixel. If the sender's
  upload can't carry every size, the browser drops the biggest first and you get the next one down.

  Sending three sizes takes more upload: a 1080p, 30 fps screen share sends about 9.7 Mbit/s in total instead of
  6.2. Simulcast is tested in Chromium-based apps (Windows, Android, web in Chrome or Edge); a sender whose browser
  can't do it is forwarded as one stream, as before.

### Changed

- **Android: the app's ID is now `com.CircuitBiscuit.natter`**, the one it will have on Google Play. Android treats
  that as a different app, so this version installs next to the old Natter instead of replacing it. Install it, sign
  in, then uninstall the old one (it would keep showing the update banner). Messages and synced settings live on the
  server, so nothing is lost; only this phone's own choices (microphone, camera quality) start fresh. Desktop apps
  keep their ID and update as usual.

## [0.9.0] — 2026-11-23

### New

- **Drag and drop channels and categories in the channel list.** If you can manage channels, grab a channel and
  drop it where it should go: a line shows where it will land. Drop it on another channel to sit above or below it
  (in that channel's category), or on a category's name to put it at the top of that category. Categories drag the
  same way, between other categories. Everyone sees the new order at once. Works on desktop and the web (phones have
  no drag and drop; use the channel's settings there).
- **Channel links.** Right-click a channel › **Copy Channel Link**. Paste the link in a message and it shows as a
  card under it: **Join Voice** for a voice channel (with who's in it), **Go to Channel** for a text channel, **Open
  Thread** for a thread. Someone who can't see the channel gets a "No access" card. Inside the message the link
  shows as the channel's name, like a #channel mention; a message that's only the link shows just the card. Opening
  the link in a browser opens the web app on that channel.
- **Emoticons become emoji when you send**, as in Discord: `:)` 🙂, `:(` 🙁, `:D` 😃, `;)` 😉, `:P` 😛, `:O` 😮,
  `:|` 😐, `:/` 😕, `:'(` 😢, `>:(` 😠, `:*` 😘, `<3` ❤️ and a few more. Only a whole word converts, so links and
  times like `10:30` stay as they are, and code is never touched. Turn it off in Settings › Appearance › **Convert
  emoticons**.

### Fixed

- Radio buttons (Notifications › Notify me about, Voice & video › Input mode) sat centred above shouting upper-case
  labels. They're full-width rows now, with the picked one highlighted. The checkboxes in the create / edit channel
  windows had the same problem and line up properly again.
- **Move up / Move down** in a channel's settings sometimes did nothing (the channel ended up with the same number
  as its neighbour). They now swap it with the channel above or below.
- GitHub release pages only ever said "Natter x.y.z": the release workflow never found the changelog. Every release
  now carries its changelog section, and a version without one isn't released.

## [0.8.0] — 2026-11-21

- **GIFs, like in Discord.** A **GIF** button next to the emoji button opens a GIF picker: categories, trending
  GIFs, search, and your favourites (star a GIF; favourites follow you to every device). Picking one sends it at
  once as a message of its own, and every message that's only a picture or GIF link now shows just the picture.
  The GIFs come from KLIPY (Google shut down Tenor's API in June 2026); the server needs a free KLIPY key
  (`NATTER_KLIPY_KEY`), otherwise there's no GIF button.
- Pictures and GIFs that finish loading no longer push the newest messages out of view.
- A link preview could vanish from your own message right after you sent it.
- Settings changed quickly one after another (or GIFs starred in a row) could undo each other.

## [0.7.0] — 2026-11-21

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
