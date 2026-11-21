# Deploying Natter

One binary, one data folder, one HTTPS host name, one UDP port — with or without a Nextcloud. These steps target
Ubuntu 24.04 with Apache (for example next to an existing Nextcloud); a Caddy variant follows.

## 1. Install

```sh
sudo useradd --system --home /var/lib/natter --create-home natter
sudo tar -xzf natter-server-<version>-linux-x86_64.tar.gz -C /opt   # → /opt/natter/{natter-server,web/}
```

## 2. Configure — `/etc/natter.env`

```sh
NATTER_BIND=127.0.0.1:8080
NATTER_DATA=/var/lib/natter
NATTER_WEB_DIR=/opt/natter/web
NATTER_NAME=Example Co
NATTER_REGISTRATION=closed            # people sign in with Nextcloud; no local sign-ups
NATTER_NEXTCLOUD_URL=https://cloud.example.com
NATTER_NEXTCLOUD_GROUP=staff          # optional: only this Nextcloud group may sign in
NATTER_PUBLIC_IP=203.0.113.10         # the address call media is announced on
NATTER_MEDIA_PORT=50000
# NATTER_AUTOJOIN_SPACE=<space id>    # set after creating the company space (right-click it › Copy Space ID)
# NATTER_KLIPY_KEY=<app key>          # optional: turns on the GIF picker (see below)
RUST_LOG=natter_server=info
```

**Without Nextcloud**, leave out the `NATTER_NEXTCLOUD_*` lines and set `NATTER_REGISTRATION=invite` (people need
an invite link to make an account) or `open` (anyone who reaches the page may sign up). People then use Natter
accounts with a username and password, and every file is stored in the data folder. Everything else works the same.

**GIFs:** create a free app key at [partner.klipy.com](https://partner.klipy.com) and set `NATTER_KLIPY_KEY`. A new
key is in testing mode (100 requests an hour for everyone together); ask for production access in KLIPY's panel once
it works. The apps talk to KLIPY directly (KLIPY's terms require that), so the key reaches every signed-in user, as in
any app with a GIF picker. Blocked words and single GIFs are set in KLIPY's panel.

`chmod 600 /etc/natter.env`. The data folder holds the database (with Nextcloud app passwords of signed-in sessions)
and local uploads — keep it `700`, owned by `natter`.

## 3. systemd — `/etc/systemd/system/natter.service`

```ini
[Unit]
Description=Natter chat server
After=network-online.target
Wants=network-online.target

[Service]
User=natter
EnvironmentFile=/etc/natter.env
ExecStart=/opt/natter/natter-server
Restart=always
RestartSec=2
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/natter
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

```sh
sudo systemctl daemon-reload && sudo systemctl enable --now natter
curl -s http://127.0.0.1:8080/health
sudo ufw allow 50000/udp comment 'Natter call media'
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
    # Uploads up to NATTER_MAX_UPLOAD_MB
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

Nothing to install. Natter uses Nextcloud's standard client APIs:

* **Login Flow v2** — on by default. Each Natter device shows up as an app password named "Natter" under the user's
  *Settings › Security*; signing out of Natter deletes it.
* **Files** — WebDAV into each user's `Natter/` folder.
* **Public link shares** — *Administration › Sharing › Allow users to share via link* must be on. If links require a
  password or expiry by policy, Natter falls back to storing that file on the Natter server.

## 6. First run

1. Open `https://chat.example.com` (or the desktop app) and **Sign in with Nextcloud** — or, without Nextcloud,
   **Create account** (the first account needs no invite). The first person becomes the server admin.
2. Create the company space, copy its id (right-click › Copy Space ID), set `NATTER_AUTOJOIN_SPACE`, restart.
   Everyone who signs in afterwards lands in it. Without Nextcloud, invite people from the space instead
   (right-click › Invite People) and send them the link.

## Backups

Everything lives in `NATTER_DATA`. A consistent copy while running:

```sh
sqlite3 /var/lib/natter/natter.db ".backup '/backup/natter-$(date +%F).db'"
rsync -a /var/lib/natter/files/ /backup/natter-files/
```

Files sent by Nextcloud users are in their Nextcloud and covered by its backup.

## Upgrades

Replace `/opt/natter`, `systemctl restart natter`. Database migrations run automatically at start. Connected apps
reconnect by themselves (calls in progress reconnect too).

## Troubleshooting

| Symptom | Fix |
|---|---|
| Calls stay on "Connecting…" | UDP 50000 blocked, or `NATTER_PUBLIC_IP` is not the address clients reach. Check `call media on udp …` in `journalctl -u natter`. |
| "Connection lost" banner flickers | The proxy isn't upgrading `/gateway` to a WebSocket (see the RewriteRule above). |
| Nextcloud sign-in says "not allowed" | The user isn't in `NATTER_NEXTCLOUD_GROUP`. |
| Attachments land on the Natter server, not in Nextcloud | Link sharing is off or restricted; see step 5. |
