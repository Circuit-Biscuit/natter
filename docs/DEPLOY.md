# Deploying Laklak

One binary, one data folder, one HTTPS host name, one UDP port — with or without a Nextcloud. These steps target
Ubuntu 24.04 with Apache (for example next to an existing Nextcloud); a Caddy variant follows.

## 1. Install

```sh
sudo useradd --system --home /var/lib/laklak --create-home laklak
sudo tar -xzf laklak-server-<version>-linux-x86_64.tar.gz -C /opt   # → /opt/laklak/{laklak-server,web/}
```

## 2. Configure — `/etc/laklak.env`

```sh
LAKLAK_BIND=127.0.0.1:8080
LAKLAK_DATA=/var/lib/laklak
LAKLAK_WEB_DIR=/opt/laklak/web
LAKLAK_NAME=Example Co
LAKLAK_REGISTRATION=closed            # people sign in with Nextcloud; no local sign-ups
LAKLAK_NEXTCLOUD_URL=https://cloud.example.com
LAKLAK_NEXTCLOUD_GROUP=staff          # optional: only this Nextcloud group may sign in
LAKLAK_PUBLIC_IP=203.0.113.10         # the address call media is announced on
LAKLAK_MEDIA_PORT=50000
# LAKLAK_MEDIA_THREADS=4             # more people in calls: one call thread per core, UDP ports 50000-50003
# LAKLAK_AUTOJOIN_SPACE=<space id>    # set after creating the company space (right-click it › Copy Space ID)
# LAKLAK_KLIPY_KEY=<app key>          # optional: turns on the GIF picker (see below)
# LAKLAK_SMTP_URL=smtps://chat%40example.com:<password>@mail.example.com:465   # optional: "Forgot password?" e-mails
# LAKLAK_SMTP_FROM=Laklak <chat@example.com>
RUST_LOG=laklak_server=info
```

**Without Nextcloud**, leave out the `LAKLAK_NEXTCLOUD_*` lines and set `LAKLAK_REGISTRATION=invite` (people need
an invite link to make an account) or `open` (anyone who reaches the page may sign up). People then use Laklak
accounts with a username and password, and every file is stored in the data folder. Everything else works the same.

**GIFs:** create a free app key at [partner.klipy.com](https://partner.klipy.com) and set `LAKLAK_KLIPY_KEY`. A new
key is in testing mode (100 requests an hour for everyone together); ask for production access in KLIPY's panel once
it works. Every server needs its own key. The apps talk to KLIPY directly (KLIPY's terms require that), so the server
gives the key to signed-in users (never to the public `/api/info`), as in any app with a GIF picker. Blocked words and single GIFs are set in KLIPY's panel.

**Password reset e-mails:** Laklak accounts (not Nextcloud ones) can add an e-mail address, and then reset a lost
password themselves with a code sent there, once the server can send mail. Set `LAKLAK_SMTP_URL` to your mail
account: `smtps://user:password@mail.example.com:465` (TLS from the start) or
`smtp://user:password@mail.example.com:587?tls=required` (STARTTLS). Write an `@` in the user name as `%40` and any
other special character in the password URL-encoded too. `LAKLAK_SMTP_FROM` is the sender people see, like
`Laklak <chat@example.com>`; most mail servers want it to be the account you sign in with or one of its aliases.
The log says `password reset e-mail is on` at startup, or why the mail server didn't answer. Without SMTP the apps
offer no e-mail reset. Either way a server admin can give someone a temporary password: right-click them ›
**Reset Password…**.

**When someone can't sign in:** the log says why, for every try: `journalctl -u laklak | grep -E "sign-in|reset code"`
shows `no account has that username` (they're typing another name), `<username> (by e-mail): wrong password`,
`<username> signed in`, `reset code e-mailed to <username>`, `wrong code for <username>` and so on. It names an
account only when it exists, and never logs a password or what was typed for an account that doesn't.

`chmod 600 /etc/laklak.env`. The data folder holds the database (with Nextcloud app passwords of signed-in sessions)
and local uploads — keep it `700`, owned by `laklak`.

## 3. systemd — `/etc/systemd/system/laklak.service`

```ini
[Unit]
Description=Laklak chat server
After=network-online.target
Wants=network-online.target

[Service]
User=laklak
EnvironmentFile=/etc/laklak.env
ExecStart=/opt/laklak/laklak-server
Restart=always
RestartSec=2
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/laklak
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

```sh
sudo systemctl daemon-reload && sudo systemctl enable --now laklak
curl -s http://127.0.0.1:8080/health
sudo ufw allow 50000/udp comment 'Laklak call media'   # with LAKLAK_MEDIA_THREADS=4: 50000:50003/udp
# Let the call socket have 4 MB buffers: calls come in bursts, and Linux's default (~200 KB) drops packets
# with a hundred people in calls.
printf 'net.core.rmem_max = 8388608\nnet.core.wmem_max = 8388608\n' | sudo tee /etc/sysctl.d/90-laklak.conf
sudo sysctl --system
```

## 4. HTTPS with Apache (`chat.example.com`)

```sh
sudo a2enmod proxy proxy_http proxy_wstunnel headers
```

```apache
<VirtualHost *:443>
    ServerName chat.example.com
    # certificates: certbot --apache -d chat.example.com
    ProxyPreserveHost On
    ProxyRequests Off
    RewriteEngine On
    RewriteCond %{HTTP:Upgrade} =websocket [NC]
    RewriteRule ^/gateway(.*) ws://127.0.0.1:8080/gateway$1 [P,L]
    ProxyPass        / http://127.0.0.1:8080/
    ProxyPassReverse / http://127.0.0.1:8080/
    # Uploads up to LAKLAK_MAX_UPLOAD_MB
    LimitRequestBody 0
    ProxyTimeout 600
</VirtualHost>
```

The app sends a heartbeat every 20 s, so idle WebSockets stay well inside the 600 s proxy timeout.

### …or Caddy

```caddy
chat.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

## 5. Nextcloud side (only if you use one)

Nothing to install. Laklak uses Nextcloud's standard client APIs:

* **Login Flow v2** — on by default. Each Laklak device shows up as an app password named "Laklak" under the user's
  *Settings › Security*; signing out of Laklak deletes it.
* **Files** — WebDAV into each user's `Laklak/` folder.
* **Public link shares** — *Administration › Sharing › Allow users to share via link* must be on. If links require a
  password or expiry by policy, Laklak falls back to storing that file on the Laklak server.

## 6. First run

1. Open `https://chat.example.com` (or the desktop app) and **Sign in with Nextcloud** — or, without Nextcloud,
   **Create account** (the first account needs no invite). The first person becomes the server admin.
2. Create the company space, copy its id (right-click › Copy Space ID), set `LAKLAK_AUTOJOIN_SPACE`, restart.
   Everyone who signs in afterwards lands in it. Without Nextcloud, invite people from the space instead
   (right-click › Invite People) and send them the link.

## Backups

Everything lives in `LAKLAK_DATA`. A consistent copy while running:

```sh
sqlite3 /var/lib/laklak/laklak.db ".backup '/backup/laklak-$(date +%F).db'"
rsync -a /var/lib/laklak/files/ /backup/laklak-files/
```

Files sent by Nextcloud users are in their Nextcloud and covered by its backup.

## Upgrades

Replace `/opt/laklak`, `systemctl restart laklak`. Database migrations run automatically at start. Connected apps
reconnect by themselves (calls in progress reconnect too).

## Releasing

This part is for whoever publishes Laklak's apps. Desktop installers and the server bundle are built by GitHub
Actions; the Android app is built on your own computer, because its signing key never goes to GitHub.

1. Add a `## [x.y.z]` section to [CHANGELOG.md](../CHANGELOG.md), run
   `powershell -ExecutionPolicy Bypass -File scripts\bump.ps1 x.y.z` and `git push origin main`. The **Release**
   workflow builds Windows, macOS, Linux and the server into a draft release `vx.y.z`, then publishes it with
   checksums and `latest.json` once those builds are done (about 20 minutes).
2. While the draft is still open, run

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\android-release.ps1
   ```

   It builds the web UI, then the signed universal APK and the app bundle (`target\android\Laklak_x.y.z.apk` and
   `.aab`), attaches the APK to the draft (so it's in `SHA256SUMS.txt`), and uploads the bundle to Google Play's
   **internal testing** track. Play's "What's new" is made from the headlines of the changelog section.
   Options: `-Track alpha` (closed testing), `-Status draft` (upload without rolling out), `-SkipGitHub`,
   `-SkipPlay`, `-SkipBuild` (reuse what's in `target\android`).
3. Try the internal build, then move it on. Production goes out only when the owner says so:

   ```powershell
   fastlane android promote from:internal to:production rollout:0.2
   ```

   `rollout` is optional (0.2 = 20 % of users; leave it out for everyone).

The store listing (texts, icon, feature graphic, screenshots) lives in `fastlane/metadata/android/en-US/`. Change
it there and upload it with `fastlane android listing`.

### Version codes and signing

* Android's `versionCode` is `major × 1,000,000 + minor × 1,000 + patch` (Tauri's own rule), the same for the APK
  and the bundle. Google Play accepts each version code only once.
* Google Play signs the copies it installs with its own key (Play App Signing); our key is the **upload key** and
  also signs the APK on GitHub. Android won't update an app from a copy signed with another key, so switching between
  the Play app and the APK means uninstalling first. The Play app doesn't show the "new APK" banner: Google Play
  updates it.
* Google Play needs native code aligned for 16 KB memory pages; `crates/desktop/build.rs` takes care of it.

### One-time setup

* Rust targets `aarch64-linux-android` and `armv7-linux-androideabi`, `cargo install tauri-cli trunk`, the Android
  SDK with NDK 27 or newer, and a JDK 17+ (Android Studio's `jbr` works).
* Ruby 3 and `gem install fastlane`; the GitHub CLI (`gh`), signed in.
* Signing: `crates/desktop/gen/android/keystore.properties` (git-ignored) with `storeFile=<path to .jks>`,
  `password=…` and `keyAlias=…`. Keep the keystore backed up: without it, no APK can update an installed one, and
  Google Play needs a key reset before it takes uploads again.
* Google Play access: a Google Cloud service account, invited in Play Console (*Users and permissions*) with
  release, testing-track and store-presence rights for the app. Then either
  * a JSON key file for it: `SUPPLY_JSON_KEY=<path>` in `fastlane/.env`, or
  * no key file at all (for organisations that forbid them): give yourself *Service Account Token Creator* on the
    service account, run `gcloud auth application-default login` once, and set
    `PLAY_SERVICE_ACCOUNT=<its email>` in `fastlane/.env`. fastlane then acts as the service account with your own
    sign-in.
* `fastlane/.env` (git-ignored) also holds `ANDROID_HOME`, `NDK_HOME` and `JAVA_HOME` for the script:

  ```sh
  ANDROID_HOME=C:\Android\sdk
  NDK_HOME=C:\Android\sdk\ndk\27.0.12077973
  JAVA_HOME=C:\Program Files\Android\Android Studio\jbr
  PLAY_SERVICE_ACCOUNT=play-publisher@your-project.iam.gserviceaccount.com
  ```

Publishing your own fork on Google Play? Change the package name first (`crates/desktop/tauri.android.conf.json`,
`applicationId` and `namespace` in `gen/android/app/build.gradle.kts`, `fastlane/Appfile`): Play keeps each name
for one developer.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Calls stutter or drop out with many people in them | The log says `call socket buffer is … KB`: raise `net.core.rmem_max`/`wmem_max` (step 3). |
| Calls stay on "Connecting…" | UDP 50000 blocked, or `LAKLAK_PUBLIC_IP` is not the address clients reach. Check `call media on udp …` in `journalctl -u laklak`. |
| "Connection lost" banner flickers | The proxy isn't upgrading `/gateway` to a WebSocket (see the RewriteRule above). |
| Nextcloud sign-in says "not allowed" | The user isn't in `LAKLAK_NEXTCLOUD_GROUP`. |
| Attachments land on the Laklak server, not in Nextcloud | Link sharing is off or restricted; see step 5. |
