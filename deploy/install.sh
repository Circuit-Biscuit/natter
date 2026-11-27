#!/usr/bin/env bash
# Install or upgrade a Laklak server on Ubuntu 24.04 or Debian 12+ (x86_64): the release from GitHub (checksum
# checked), Caddy for HTTPS, a firewall, big call socket buffers, logs kept two weeks, and nightly backups.
# Run it again to upgrade; your settings and data stay. See docs/DEPLOY.md › "One script".
#
#   curl -fsSLO https://raw.githubusercontent.com/Circuit-Biscuit/laklak/main/deploy/install.sh
#   sudo DOMAIN=chat.example.com bash install.sh
#
#   DOMAIN         the HTTPS host name; point its DNS at this machine first (Caddy fetches the certificate)
#   VERSION        release to install, e.g. 0.13.1 (default: the latest)
#   ACME_EMAIL     optional: where Let's Encrypt may write about the certificate
#   MEDIA_THREADS  call threads on a first install (default: one per core); UDP ports 50000 and up, one each
#   FORCE=1        restart even while a call is running
set -euo pipefail

REPO=Circuit-Biscuit/laklak
OPT=/opt/laklak
DATA=/var/lib/laklak
ENV=/etc/laklak.env
BACKUPS=/var/backups/laklak
PORT=8080

say() { printf '\n== %s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run as root (sudo)"
[ "$(uname -m)" = x86_64 ] || die "releases are built for x86_64 only"
command -v apt-get >/dev/null || die "needs apt (Ubuntu or Debian)"
: "${DOMAIN:?set DOMAIN, e.g. sudo DOMAIN=chat.example.com bash install.sh}"

say "Packages"
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get install -y -q curl ca-certificates sqlite3 restic caddy ufw tcpdump unattended-upgrades >/dev/null
# Security updates install themselves (Ubuntu's default; Debian needs it switched on).
printf 'APT::Periodic::Update-Package-Lists "1";\nAPT::Periodic::Unattended-Upgrade "1";\n' > /etc/apt/apt.conf.d/20auto-upgrades

say "Release"
if [ -z "${VERSION:-}" ]; then
  # /releases/latest redirects to /releases/tag/vX.Y.Z: no API call, no rate limit.
  VERSION=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" | sed -E 's#.*/tag/v##')
fi
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "can't tell which version to install (got '$VERSION')"
installed=$([ -x $OPT/laklak-server ] && $OPT/laklak-server --version | awk '{print $2}' || echo none)
echo "installing $VERSION (installed: $installed)"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tarball="laklak-server-$VERSION-linux-x86_64.tar.gz"
curl -fsSL -o "$tmp/$tarball" "https://github.com/$REPO/releases/download/v$VERSION/$tarball"
curl -fsSL -o "$tmp/SHA256SUMS.txt" "https://github.com/$REPO/releases/download/v$VERSION/SHA256SUMS.txt"
(cd "$tmp" && grep -E " \*?$tarball\$" SHA256SUMS.txt | sha256sum -c --quiet -) || die "checksum mismatch for $tarball"
tar -xzf "$tmp/$tarball" -C "$tmp"

say "User, folders, settings"
id laklak >/dev/null 2>&1 || useradd --system --home $DATA --shell /usr/sbin/nologin laklak
install -d -o laklak -g laklak -m 700 $DATA
install -d -m 700 $BACKUPS
if [ ! -f $ENV ]; then
  ip=$(ip -4 route get 1.1.1.1 | awk '{for (i = 1; i < NF; i++) if ($i == "src") print $(i + 1)}')
  cat > $ENV <<EOF
# Laklak settings (docs/ARCHITECTURE.md lists them all). Restart after changes: systemctl restart laklak
LAKLAK_BIND=127.0.0.1:$PORT
LAKLAK_DATA=$DATA
LAKLAK_WEB_DIR=$OPT/web
LAKLAK_NAME=$DOMAIN
LAKLAK_REGISTRATION=invite
LAKLAK_PUBLIC_IP=$ip
LAKLAK_MEDIA_PORT=50000
LAKLAK_MEDIA_THREADS=${MEDIA_THREADS:-$(nproc)}
RUST_LOG=laklak_server=info
# LAKLAK_NEXTCLOUD_URL=https://cloud.example.com
# LAKLAK_SMTP_URL=smtps://chat%40example.com:<password>@mail.example.com:465
# LAKLAK_SMTP_FROM=Laklak <chat@example.com>
# LAKLAK_KLIPY_KEY=<app key>
EOF
  chmod 600 $ENV
  echo "wrote $ENV (public IP $ip)"
fi
media_port=$(sed -n 's/^LAKLAK_MEDIA_PORT=//p' $ENV); media_port=${media_port:-50000}
threads=$(sed -n 's/^LAKLAK_MEDIA_THREADS=//p' $ENV); threads=${threads:-1}
media_last=$((media_port + threads - 1))

say "System settings"
# The call socket asks for 4 MB buffers; Linux caps them at these (calls come in bursts).
printf 'net.core.rmem_max = 8388608\nnet.core.wmem_max = 8388608\n' > /etc/sysctl.d/90-laklak.conf
sysctl -q --system || echo "warning: sysctl --system failed (container?)"
# Logs go after two weeks (privacy policy).
install -d /etc/systemd/journald.conf.d
printf '[Journal]\nMaxRetentionSec=2week\nMaxFileSec=1day\n' > /etc/systemd/journald.conf.d/90-laklak.conf
systemctl restart systemd-journald

cat > /etc/systemd/system/laklak.service <<EOF
[Unit]
Description=Laklak chat server (https://$DOMAIN)
After=network-online.target
Wants=network-online.target

[Service]
User=laklak
Group=laklak
EnvironmentFile=$ENV
ExecStart=$OPT/laklak-server
Restart=always
RestartSec=2
LimitNOFILE=1048576
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=$DATA
ProtectKernelTunables=true
ProtectControlGroups=true
RestrictSUIDSGID=true

[Install]
WantedBy=multi-user.target
EOF

say "HTTPS (Caddy)"
caddyfile=/etc/caddy/Caddyfile
if [ -f $caddyfile ] && ! grep -q '^# laklak' $caddyfile && ! grep -q '^:80 {' $caddyfile; then
  caddyfile=/etc/caddy/Caddyfile.laklak
  echo "warning: /etc/caddy/Caddyfile is your own; wrote $caddyfile for you to merge"
fi
install -d /etc/caddy/sites /var/log/caddy
chown caddy:caddy /var/log/caddy
cat > $caddyfile <<EOF
# laklak: written by deploy/install.sh, which rewrites it on every run. Other sites go in /etc/caddy/sites/.
{
${ACME_EMAIL:+	email $ACME_EMAIL}
}

$DOMAIN {
	encode zstd gzip
	header Strict-Transport-Security "max-age=31536000"
	reverse_proxy 127.0.0.1:$PORT
	# Two weeks of access logs, without the session token apps up to 0.13 put in the /gateway URL.
	log {
		output file /var/log/caddy/laklak.log {
			roll_keep_for 336h
		}
		format filter {
			wrap json
			fields {
				request>uri query {
					delete token
				}
			}
		}
	}
}

import /etc/caddy/sites/*
EOF
caddy validate --config $caddyfile --adapter caddyfile >/dev/null 2>&1 || die "Caddy rejects $caddyfile (caddy validate --config $caddyfile)"

say "Firewall"
ufw allow 22/tcp comment 'SSH' >/dev/null
ufw allow 80/tcp >/dev/null
ufw allow 443 >/dev/null
ufw allow "$media_port:$media_last/udp" comment 'Laklak call media' >/dev/null
ufw --force enable >/dev/null && ufw status | sed -n '1,20p' || echo "warning: ufw could not start (container?)"

say "Backups"
cat > /usr/local/sbin/laklak-backup <<'EOF'
#!/bin/sh
# Nightly: a consistent copy of the database (SQLite's online backup) kept 7 days on this disk, by weekday. With
# /etc/laklak-backup.env (RESTIC_REPOSITORY, RESTIC_PASSWORD, and what the repository needs) also the database,
# the uploaded files and the settings to an encrypted restic repository elsewhere: 7 daily and 4 weekly snapshots.
set -eu
db="/var/backups/laklak/laklak-$(date +%u).db"
sqlite3 /var/lib/laklak/laklak.db ".backup '$db'"
[ "$(sqlite3 "$db" 'PRAGMA integrity_check')" = ok ] || { echo "backup $db fails its integrity check" >&2; exit 1; }
if [ -f /etc/laklak-backup.env ]; then
  set -a; . /etc/laklak-backup.env; set +a
  restic cat config >/dev/null 2>&1 || restic init
  restic backup --quiet --tag laklak "$db" /var/lib/laklak/files /etc/laklak.env
  restic forget --quiet --tag laklak --keep-daily 7 --keep-weekly 4 --prune
  echo "off-site backup done"
else
  echo "local backup only: no /etc/laklak-backup.env"
fi
EOF
chmod 750 /usr/local/sbin/laklak-backup
cat > /etc/systemd/system/laklak-backup.service <<'EOF'
[Unit]
Description=Laklak backup
After=laklak.service

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/laklak-backup
EOF
cat > /etc/systemd/system/laklak-backup.timer <<'EOF'
[Unit]
Description=Laklak backup every night

[Timer]
OnCalendar=*-*-* 03:30
RandomizedDelaySec=30m
Persistent=true

[Install]
WantedBy=timers.target
EOF

say "Start"
systemctl daemon-reload
if [ "$installed" != "$VERSION" ]; then
  if systemctl is-active -q laklak && [ "${FORCE:-0}" != 1 ]; then
    n=$( (timeout 10 tcpdump -ni any udp portrange "$media_port-$media_last" -q 2>/dev/null || true) | wc -l)
    [ "$n" -le 20 ] || die "a call is running ($n UDP packets in 10 s): not restarting. Try later or FORCE=1."
  fi
  rm -rf $OPT.new && cp -a "$tmp/laklak" $OPT.new
  [ ! -d $OPT/web/download ] || cp -an $OPT/web/download $OPT.new/web/
  chown -R root:root $OPT.new
  systemctl stop laklak 2>/dev/null || true
  rm -rf $OPT.old
  [ ! -d $OPT ] || mv $OPT $OPT.old
  mv $OPT.new $OPT
fi
systemctl enable -q --now laklak-backup.timer
systemctl enable -q laklak caddy
# Same version as before: leave a running server alone (a restart drops calls); settings apply at the next restart.
if [ "$installed" != "$VERSION" ]; then systemctl restart laklak; else systemctl start laklak; fi
[ "$caddyfile" != /etc/caddy/Caddyfile ] || systemctl reload-or-restart caddy
for _ in $(seq 30); do curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && break; sleep 1; done
curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null || die "laklak didn't come up: journalctl -u laklak"

say "Done: Laklak $VERSION"
cat <<EOF
  App:        https://$DOMAIN  (the certificate needs DNS pointing here; the first account you create is the admin)
  Settings:   $ENV  (then: systemctl restart laklak)
  Calls:      UDP $media_port-$media_last, $threads thread(s)
  Logs:       journalctl -u laklak, /var/log/caddy/laklak.log (two weeks)
  Backups:    every night to $BACKUPS; add /etc/laklak-backup.env for an off-site copy (docs/DEPLOY.md)
  Rollback:   $( [ -d $OPT.old ] && echo "systemctl stop laklak; mv $OPT $OPT.bad; mv $OPT.old $OPT; systemctl start laklak" || echo "nothing older here")
EOF
