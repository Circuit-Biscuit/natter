#!/usr/bin/env bash
# Install or upgrade a Laklak server on Ubuntu 24.04 or Debian 12+ (x86_64): the release from GitHub (checksum
# checked), Caddy for HTTPS, a firewall, big call socket buffers, logs kept two weeks, nightly backups and a
# watchdog that alerts you. Run it again to upgrade; your settings and data stay. See docs/DEPLOY.md › "One script".
#
#   curl -fsSLO https://raw.githubusercontent.com/Circuit-Biscuit/laklak/main/deploy/install.sh
#   sudo DOMAIN=chat.example.com bash install.sh
#
#   DOMAIN         the HTTPS host name; point its DNS at this machine first (Caddy fetches the certificate)
#   VERSION        release to install, e.g. 0.13.1 (default: the latest)
#   ACME_EMAIL     optional: where Let's Encrypt may write about the certificate
#   MEDIA_THREADS  call threads on a first install (default: one per core); UDP ports 50000 and up, one each
#   ALERT_URL      optional: where the watchdog posts problems (an ntfy.sh topic, a Discord or Slack webhook)
#   TARBALL        optional: install this local build (laid out like a release's tarball) instead of a release
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
apt-get install -y -q curl ca-certificates sqlite3 restic caddy coturn ufw tcpdump unattended-upgrades jq openssl >/dev/null
# Security updates install themselves (Ubuntu's default; Debian needs it switched on).
printf 'APT::Periodic::Update-Package-Lists "1";\nAPT::Periodic::Unattended-Upgrade "1";\n' > /etc/apt/apt.conf.d/20auto-upgrades

say "Release"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
installed=$([ -x $OPT/laklak-server ] && $OPT/laklak-server --version | awk '{print $2}' || echo none)
if [ -n "${TARBALL:-}" ]; then
  # A build of your own, laid out like a release's (testing before a release): there's no checksum to check, and it
  # is always swapped in.
  [ -f "$TARBALL" ] || die "TARBALL=$TARBALL doesn't exist"
  cp "$TARBALL" "$tmp/local.tar.gz"
  tar -xzf "$tmp/local.tar.gz" -C "$tmp"
  VERSION=$("$tmp/laklak/laklak-server" --version | awk '{print $2}')
  installed="$installed (replaced by a local build)"
else
  if [ -z "${VERSION:-}" ]; then
    # /releases/latest redirects to /releases/tag/vX.Y.Z: no API call, no rate limit.
    VERSION=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" | sed -E 's#.*/tag/v##')
  fi
  [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "can't tell which version to install (got '$VERSION')"
  tarball="laklak-server-$VERSION-linux-x86_64.tar.gz"
  curl -fsSL -o "$tmp/$tarball" "https://github.com/$REPO/releases/download/v$VERSION/$tarball"
  curl -fsSL -o "$tmp/SHA256SUMS.txt" "https://github.com/$REPO/releases/download/v$VERSION/SHA256SUMS.txt"
  (cd "$tmp" && grep -E " \*?$tarball\$" SHA256SUMS.txt | sha256sum -c --quiet -) || die "checksum mismatch for $tarball"
  tar -xzf "$tmp/$tarball" -C "$tmp"
fi
echo "installing $VERSION (installed: $installed)"

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

say "Call relay (coturn)"
# For people whose network blocks UDP to the call ports: their apps reach the call through this relay, over UDP or
# TCP 3478 or TLS 5349. It relays to this server's own address and nowhere else, so it can't be used to reach other
# machines. Laklak signs day-long credentials with the shared secret; coturn needs no user list.
secret=$(sed -n 's/^LAKLAK_TURN_SECRET=//p' $ENV)
if [ -z "$secret" ]; then
  secret=$(openssl rand -hex 32)
  cat >> $ENV <<EOF
# Call relay (coturn on this machine, /etc/turnserver.conf) for networks that block UDP to the call ports.
LAKLAK_TURN_URLS=turn:$DOMAIN:3478?transport=udp,turn:$DOMAIN:3478?transport=tcp,turns:$DOMAIN:5349?transport=tcp
LAKLAK_TURN_SECRET=$secret
EOF
  settings_changed=1
fi
public_ip=$(sed -n 's/^LAKLAK_PUBLIC_IP=//p' $ENV)
cat > /etc/turnserver.conf.new <<EOF
# laklak: written by deploy/install.sh, which rewrites it on every run.
listening-port=3478
tls-listening-port=5349
realm=$DOMAIN
server-name=$DOMAIN
use-auth-secret
static-auth-secret=$secret
fingerprint
no-cli
no-multicast-peers
no-tlsv1
no-tlsv1_1
denied-peer-ip=0.0.0.0-255.255.255.255
denied-peer-ip=::-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff
allowed-peer-ip=$public_ip
min-port=49152
max-port=49999
cert=/etc/coturn/certs/cert.pem
pkey=/etc/coturn/certs/key.pem
syslog
simple-log
EOF
chown root:turnserver /etc/turnserver.conf.new
chmod 640 /etc/turnserver.conf.new
if cmp -s /etc/turnserver.conf.new /etc/turnserver.conf; then rm /etc/turnserver.conf.new; else mv /etc/turnserver.conf.new /etc/turnserver.conf; turn_changed=1; fi
[ ! -f /etc/default/coturn ] || sed -i 's/^#\?TURNSERVER_ENABLED=.*/TURNSERVER_ENABLED=1/' /etc/default/coturn
# coturn gets Caddy's certificate for TLS on 5349 (copied when it changes; until there is one, UDP/TCP 3478 work).
cat > /usr/local/sbin/laklak-turn-certs <<EOF
#!/bin/bash
set -eu
dir=\$(ls -d /var/lib/caddy/.local/share/caddy/certificates/*/$DOMAIN 2>/dev/null | head -1)
[ -n "\$dir" ] && [ -f "\$dir/$DOMAIN.crt" ] || exit 0
install -d -o root -g turnserver -m 750 /etc/coturn/certs
if ! cmp -s "\$dir/$DOMAIN.crt" /etc/coturn/certs/cert.pem; then
  install -o root -g turnserver -m 640 "\$dir/$DOMAIN.crt" /etc/coturn/certs/cert.pem
  install -o root -g turnserver -m 640 "\$dir/$DOMAIN.key" /etc/coturn/certs/key.pem
  systemctl restart coturn
  echo "coturn has the certificate for $DOMAIN"
fi
EOF
chmod 750 /usr/local/sbin/laklak-turn-certs
cat > /etc/systemd/system/laklak-turn-certs.service <<'EOF'
[Unit]
Description=Copy the HTTPS certificate to coturn

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/laklak-turn-certs
EOF
cat > /etc/systemd/system/laklak-turn-certs.timer <<'EOF'
[Unit]
Description=Copy the HTTPS certificate to coturn every hour

[Timer]
OnBootSec=2min
OnUnitActiveSec=1h

[Install]
WantedBy=timers.target
EOF

say "Firewall"
ufw allow 22/tcp comment 'SSH' >/dev/null
ufw allow 80/tcp >/dev/null
ufw allow 443 >/dev/null
ufw allow "$media_port:$media_last/udp" comment 'Laklak call media' >/dev/null
ufw allow 3478 comment 'Laklak call relay' >/dev/null
ufw allow 5349/tcp comment 'Laklak call relay (TLS)' >/dev/null
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

say "Watchdog"
install -d -m 700 /var/lib/laklak-watch
[ -f /var/lib/laklak-watch/installed ] || date +%s > /var/lib/laklak-watch/installed
if [ ! -f /etc/laklak-alerts.env ]; then
  cat > /etc/laklak-alerts.env <<EOF
# Read by laklak-watch. ALERT_URL: an ntfy.sh topic (pick a long random name), a Discord or Slack webhook, or
# anything that takes a POST. Test it: laklak-watch --test
DOMAIN=$DOMAIN
ALERT_URL=${ALERT_URL:-}
EOF
  chmod 600 /etc/laklak-alerts.env
fi
cat > /usr/local/sbin/laklak-watch <<'WATCH'
#!/bin/bash
# Every 5 minutes (laklak-watch.timer): does Laklak answer, here and over HTTPS, with a certificate that isn't about
# to run out; are its services running, the disk not full, the backups fine? Problems go to the journal
# (journalctl -u laklak-watch) and, with ALERT_URL in /etc/laklak-alerts.env, there: when they start, once a day
# while they last, and when they're gone. A dead machine can't alert: watch https://<domain>/health from outside too.
set -u
. /etc/laklak-alerts.env
state=/var/lib/laklak-watch

send() {
  echo "$1"
  [ -n "${ALERT_URL:-}" ] || return 0
  case "$ALERT_URL" in
    *discord.com/api/webhooks*) body=$(jq -n --arg m "$1" '{content: $m}'); type=application/json ;;
    *hooks.slack.com*) body=$(jq -n --arg m "$1" '{text: $m}'); type=application/json ;;
    *) body=$1; type=text/plain ;;
  esac
  curl -fsS -m 15 -H "Content-Type: $type" --data-binary "$body" "$ALERT_URL" >/dev/null || echo "could not post the alert"
}
if [ "${1:-}" = --test ]; then send "Laklak on ${DOMAIN:-$(hostname)}: alerts work"; exit; fi

problems=()
curl -fsS -m 5 -o /dev/null http://127.0.0.1:8080/health || problems+=("Laklak doesn't answer on the server (journalctl -u laklak)")
if [ -n "${DOMAIN:-}" ]; then
  curl -fsS -m 10 -o /dev/null "https://$DOMAIN/health" || problems+=("https://$DOMAIN/health doesn't answer (Caddy, DNS or the certificate)")
  end=$(timeout 10 openssl s_client -connect "$DOMAIN:443" -servername "$DOMAIN" </dev/null 2>/dev/null | openssl x509 -noout -enddate 2>/dev/null | cut -d= -f2)
  if [ -n "$end" ]; then
    left=$(( ($(date -d "$end" +%s) - $(date +%s)) / 86400 ))
    [ "$left" -ge 14 ] || problems+=("the HTTPS certificate runs out in $left days")
  fi
fi
for unit in laklak caddy coturn; do
  systemctl cat "$unit.service" >/dev/null 2>&1 || continue
  systemctl is-active -q "$unit" || problems+=("$unit isn't running (journalctl -u $unit)")
done
use=$(df --output=pcent / | tail -1 | tr -dc 0-9)
[ "$use" -lt 85 ] || problems+=("the disk is $use% full")
systemctl is-failed -q laklak-backup && problems+=("the last backup failed (journalctl -u laklak-backup)")
# A new install has no backup until its first night.
if [ $(( $(date +%s) - $(cat $state/installed 2>/dev/null || date +%s) )) -gt 129600 ]; then
  newest=$(ls -t /var/backups/laklak/laklak-*.db 2>/dev/null | head -1)
  { [ -n "$newest" ] && [ $(( $(date +%s) - $(stat -c %Y "$newest") )) -lt 129600 ]; } || problems+=("no backup in the last 36 hours")
fi

now=$(date +%s)
current=$(printf '%s\n' "${problems[@]+"${problems[@]}"}" | sed '/^$/d')
previous=$(cat $state/problems 2>/dev/null)
sent=$(cat $state/sent 2>/dev/null || echo 0)
[ -z "$current" ] || echo "$current"
if [ "$current" != "$previous" ] || { [ -n "$current" ] && [ $((now - sent)) -gt 86400 ]; }; then
  if [ -n "$current" ]; then send "Laklak on ${DOMAIN:-$(hostname)}: $current"; else send "Laklak on ${DOMAIN:-$(hostname)}: all fine again"; fi
  printf '%s' "$current" > $state/problems
  echo "$now" > $state/sent
fi
WATCH
chmod 750 /usr/local/sbin/laklak-watch
cat > /etc/systemd/system/laklak-watch.service <<'UNIT'
[Unit]
Description=Laklak watchdog

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/laklak-watch
UNIT
cat > /etc/systemd/system/laklak-watch.timer <<'UNIT'
[Unit]
Description=Laklak watchdog every 5 minutes

[Timer]
OnBootSec=5min
OnUnitActiveSec=5min

[Install]
WantedBy=timers.target
UNIT

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
systemctl enable -q --now laklak-backup.timer laklak-watch.timer laklak-turn-certs.timer
systemctl enable -q laklak caddy coturn
# Same version and settings as before: leave a running server alone (a restart drops calls).
if [ "$installed" != "$VERSION" ] || [ -n "${settings_changed:-}" ]; then systemctl restart laklak; else systemctl start laklak; fi
[ "$caddyfile" != /etc/caddy/Caddyfile ] || systemctl reload-or-restart caddy
if [ -n "${turn_changed:-}" ]; then systemctl restart coturn; else systemctl start coturn; fi
/usr/local/sbin/laklak-turn-certs || true
for _ in $(seq 30); do curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && break; sleep 1; done
curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null || die "laklak didn't come up: journalctl -u laklak"

say "Done: Laklak $VERSION"
cat <<EOF
  App:        https://$DOMAIN  (the certificate needs DNS pointing here; the first account you create is the admin)
  Settings:   $ENV  (then: systemctl restart laklak)
  Calls:      UDP $media_port-$media_last, $threads thread(s); relay (coturn) on 3478 UDP/TCP and 5349 TLS
  Logs:       journalctl -u laklak, /var/log/caddy/laklak.log (two weeks)
  Backups:    every night to $BACKUPS; add /etc/laklak-backup.env for an off-site copy (docs/DEPLOY.md)
  Alerts:     laklak-watch every 5 minutes; $(grep -q '^ALERT_URL=.' /etc/laklak-alerts.env && echo "to ALERT_URL (test: laklak-watch --test)" || echo "journal only until ALERT_URL is set in /etc/laklak-alerts.env")
  Rollback:   $( [ -d $OPT.old ] && echo "systemctl stop laklak; mv $OPT $OPT.bad; mv $OPT.old $OPT; systemctl start laklak" || echo "nothing older here")
EOF
