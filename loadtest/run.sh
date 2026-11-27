#!/usr/bin/env bash
# One load test run: a fresh server (data in RAM), the load generator with your arguments, and the server's CPU
# per thread every few seconds. Build first, from the repo root:
#   cargo build --release -p laklak-server -p laklak-loadtest
# Example:
#   loadtest/run.sh --users 2000 --space-size 200 --msg-rate 50 --callers 500 --room-size 5 --video 0.2
set -euo pipefail
cd "$(dirname "$0")"
out="results/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$out"
echo "$*" > "$out/args.txt"

docker compose down --remove-orphans >/dev/null 2>&1 || true
docker compose up -d laklak
pid=$(docker inspect -f '{{.State.Pid}}' laklak-lt-server)

# sfu = the call threads together (100% = one core full) and the busiest one, other = everything else; then both
# containers' totals and UDP packets
# dropped because a socket's buffer was full (counted since start, per container).
udp_drops() {
  awk '/^Udp:/ { if (seen) { printf "udp drops: receive %s, send %s", $6, $7; exit } seen = 1 }' "/proc/$1/net/snmp" 2>/dev/null
}
(
  while [ -d "/proc/$pid" ]; do
    loadgen=$(docker ps -q --filter name=laklak-lt-loadgen | head -1)
    lpid=$([ -n "$loadgen" ] && docker inspect -f '{{.State.Pid}}' "$loadgen" || echo 0)
    echo "          server $(udp_drops "$pid")  |  loadgen $(udp_drops "$lpid")"
    rss=$(awk '/VmRSS/ {printf "%.0f", $2 / 1024}' "/proc/$pid/status" 2>/dev/null || echo "?")
    top -H -b -n 2 -d 1 -p "$pid" | awk -v t="$(date +%T)" -v rss="$rss" '
      /^top -/ { delete cpu; busiest = 0; next }
      /^ *[0-9]+ / { cpu[$12 ~ /^sfu/ ? "sfu" : "other"] += $9; if ($12 ~ /^sfu/ && $9 > busiest) busiest = $9 }
      END { printf "%s  server threads: sfu %.0f%% (busiest %.0f%%)  other %.0f%%  rss %s MB\n", t, cpu["sfu"], busiest, cpu["other"], rss }'
    docker stats --no-stream --format '          {{.Name}}: cpu {{.CPUPerc}}  mem {{.MemUsage}}' | grep laklak-lt || true
    sleep 3
  done
) > "$out/server.log" 2>&1 &
sampler=$!

status=0
docker compose run --rm loadgen "$@" 2>&1 | tee "$out/loadgen.log" || status=$?
kill "$sampler" 2>/dev/null || true
docker compose down >/dev/null 2>&1

echo
echo "busiest server moments:"
grep "server threads" "$out/server.log" | awk '{s = $5; o = $9; sub("%", "", s); sub("%", "", o); print s + o, $0}' |
  sort -n | tail -3 | cut -d' ' -f2-
echo "full logs: loadtest/$out"
exit "$status"
