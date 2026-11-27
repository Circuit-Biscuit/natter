# Load test

Puts one Laklak server under chat and call load on your own machine and shows where it gives out.

- **Chat:** every fake user keeps a gateway WebSocket open, like the app. Messages go out at a fixed rate in total,
  each into its sender's space, and every member's socket measures how long the message took to arrive.
- **Calls:** some users also join calls with a real WebRTC peer ([str0m](https://github.com/algesten/str0m)), the
  same library the server's SFU uses. The audio and video are fake, frames sized like Opus and VP8 that carry their
  send time and a sequence number. So the receivers measure delay and loss through the SFU without encoding
  anything.

The server and the load generator run in Docker on one bridge network. The server gets 4 CPUs (like a 4-vCPU cloud
server) and keeps its data in RAM. Both containers run the binaries from `target/release` in a plain Debian image,
so no image is built and the disk footprint stays at the Debian image (about 80 MB) plus the logs under
`loadtest/results/`.

## Run

On Linux (or WSL) with Docker and Compose:

```sh
cargo build --release -p laklak-server -p laklak-loadtest
loadtest/run.sh --users 2000 --space-size 200 --msg-rate 50 --callers 500 --room-size 5 --video 0.2
```

Every 5 seconds it prints a line like:

```
  45s  gateway 2000/2000  msgs 50/s → 10000 deliveries/s, delay p50 ≤2ms p99 ≤10ms  |  calls 500/500  media in 230.1 Mbit/s, 47000 frames/s, lost 0.00%, delay p50 ≤1ms p99 ≤5ms  |  errors 0
```

and a summary of the time after the ramp at the end. `results/<time>/server.log` has the server's CPU every few
seconds: `sfu` is the call threads together (100% = one core full) with the busiest one, `other` is everything else.

| Option | Default | |
|---|---|---|
| `--users` | 200 | fake users, each with a gateway connection |
| `--space-size` | 100 | users per space (a message reaches everyone in its space) |
| `--msg-rate` | 10 | messages per second, all users together |
| `--callers` | 50 | the first this many users also join calls |
| `--room-size` | 5 | people per call |
| `--video` | 0.2 | share of each call that sends a camera |
| `--audio-kbps` / `--video-kbps` / `--fps` | 40 / 1000 / 30 | fake media rates |
| `--ramp` | 30 | seconds over which users connect |
| `--duration` | 120 | seconds of full load after the ramp |

`SERVER_CPUS` and `LOADGEN_CPUS` (cpuset lists, default `0-3` and `4-15`) pick the cores for each container, and
`MEDIA_THREADS` (default 4) sets the server's `LAKLAK_MEDIA_THREADS`.

## What it can't tell

- The network is the machine itself, so bandwidth is effectively unlimited and there is no real loss or jitter. Real
  links are tested on real servers.
- Cameras send one layer, not simulcast, so the SFU forwards everything it gets and never switches layers.
- If the load generator's own CPU is full (see `server.log`), it is the bottleneck, not the server.
