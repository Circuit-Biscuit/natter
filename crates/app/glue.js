// Natter's only JavaScript: WebRTC media, synthesized sounds, and the Tauri bridge.
// Everything else — state, UI, networking — is Rust (src/). Rust calls these through wasm-bindgen.

// ------------------------------------------------------------------ calls

let pc = null;
let dc = null;
let handler = null;
let me = 0;
let opts = { muted: false, deafened: false, input: "", output: "", camera: "", ptt: false, noise: true };
let pttDown = false;
const local = { mic: null, cam: null, screen: null, screenAudio: null };
const senders = {};
const remote = new Map(); // "uid:kind" -> { stream, track }
const audioEls = new Map(); // "uid:kind" -> <audio>
let actx = null;
const meters = new Map(); // "uid:kind" or "local" -> { analyser, buf, speaking, lastLoud }
let meterTimer = null;

function emit(obj) {
  if (handler) handler(JSON.stringify(obj));
}

export function voiceSetHandler(cb) {
  handler = cb;
}

function micConstraints() {
  return {
    deviceId: opts.input ? { exact: opts.input } : undefined,
    echoCancellation: true,
    noiseSuppression: opts.noise,
    autoGainControl: true,
  };
}

async function getMic() {
  try {
    const s = await navigator.mediaDevices.getUserMedia({ audio: micConstraints() });
    return s.getAudioTracks()[0];
  } catch (e) {
    if (opts.input) {
      // The saved device is gone (unplugged headset): fall back to the default one.
      opts.input = "";
      try {
        const s = await navigator.mediaDevices.getUserMedia({ audio: micConstraints() });
        return s.getAudioTracks()[0];
      } catch (_) {}
    }
    emit({ type: "warn", text: "No microphone available — you joined listen-only." });
    return null;
  }
}

function applyMicEnabled() {
  if (!local.mic) return;
  const live = !opts.muted && !opts.deafened && (!opts.ptt || pttDown);
  local.mic.enabled = live;
}

function ensureAudioContext() {
  if (!actx) actx = new AudioContext();
  if (actx.state === "suspended") actx.resume();
  return actx;
}

function addMeter(key, track) {
  try {
    const ctx = ensureAudioContext();
    const src = ctx.createMediaStreamSource(new MediaStream([track]));
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 512;
    src.connect(analyser);
    meters.set(key, { analyser, src, buf: new Float32Array(analyser.fftSize), speaking: false, lastLoud: 0 });
  } catch (_) {}
}

function startMeters() {
  if (meterTimer) return;
  meterTimer = setInterval(() => {
    const now = performance.now();
    for (const [key, m] of meters) {
      m.analyser.getFloatTimeDomainData(m.buf);
      let sum = 0;
      for (let i = 0; i < m.buf.length; i++) sum += m.buf[i] * m.buf[i];
      const rms = Math.sqrt(sum / m.buf.length);
      const enabled = key !== "local" || (local.mic && local.mic.enabled);
      if (rms > 0.015 && enabled) m.lastLoud = now;
      const speaking = now - m.lastLoud < 300;
      if (speaking !== m.speaking) {
        m.speaking = speaking;
        const user = key === "local" ? me : Number(key.split(":")[0]);
        emit({ type: "speaking", user, on: speaking });
      }
    }
  }, 80);
}

function onRemoteTrack(e) {
  const stream = e.streams[0];
  // The SFU names streams "<user>-<kind>"; we key them "<user>:<kind>".
  const m = stream && stream.id.match(/^(\d+)[-:]?(mic|cam|screen_audio|screen)$/);
  const key = m ? m[1] + ":" + m[2] : stream ? stream.id : e.track.id;
  const track = e.track;
  remote.set(key, { stream: new MediaStream([track]), track });
  if (track.kind === "audio") {
    const el = document.createElement("audio");
    el.autoplay = true;
    el.srcObject = new MediaStream([track]);
    el.muted = opts.deafened;
    if (opts.output && el.setSinkId) el.setSinkId(opts.output).catch(() => {});
    el.style.display = "none";
    document.body.appendChild(el);
    audioEls.get(key)?.remove();
    audioEls.set(key, el);
    if (key.endsWith(":mic")) addMeter(key, track);
  }
  const changed = () => emit({ type: "tracks" });
  track.onmute = changed;
  track.onunmute = changed;
  track.onended = () => {
    remote.delete(key);
    audioEls.get(key)?.remove();
    audioEls.delete(key);
    meters.delete(key);
    changed();
  };
  changed();
}

async function onSignal(ev) {
  const msg = JSON.parse(ev.data);
  if (msg.type === "offer") {
    await pc.setRemoteDescription(msg);
    const answer = await pc.createAnswer();
    await pc.setLocalDescription(answer);
    dc.send(JSON.stringify({ type: "answer", sdp: answer.sdp }));
  } else if (msg.type === "answer") {
    await pc.setRemoteDescription(msg);
  }
}

// Join a call. Resolves when the SFU answered; media flows once ICE connects.
export async function voiceJoin(base, token, channel, myId, optsJson) {
  voiceLeave();
  me = myId;
  opts = Object.assign(opts, JSON.parse(optsJson));
  ensureAudioContext();
  local.mic = await getMic();
  pc = new RTCPeerConnection({ bundlePolicy: "max-bundle" });
  const micStream = new MediaStream(local.mic ? [local.mic] : []);
  const order = [
    ["mic", "audio"],
    ["cam", "video"],
    ["screen", "video"],
    ["screen_audio", "audio"],
  ];
  for (const [name, kind] of order) {
    const track = name === "mic" ? local.mic : null;
    const t = pc.addTransceiver(track || kind, { direction: "sendonly", streams: [micStream] });
    senders[name] = t;
  }
  dc = pc.createDataChannel("signal");
  dc.onmessage = (ev) => onSignal(ev).catch((e) => emit({ type: "warn", text: "Call renegotiation failed: " + e }));
  pc.ontrack = onRemoteTrack;
  pc.onconnectionstatechange = () => emit({ type: "state", state: pc ? pc.connectionState : "closed" });
  applyMicEnabled();
  if (local.mic) addMeter("local", local.mic);
  startMeters();
  const offer = await pc.createOffer();
  await pc.setLocalDescription(offer);
  const tracks = {};
  for (const [name] of order) tracks[senders[name].mid] = name;
  const res = await fetch(base + "/api/channels/" + channel + "/voice", {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: "Bearer " + token },
    body: JSON.stringify({ sdp: offer.sdp, tracks, muted: opts.muted, deafened: opts.deafened }),
  });
  if (!res.ok) {
    let msg = "Could not join the call (" + res.status + ")";
    try {
      msg = (await res.json()).error || msg;
    } catch (_) {}
    voiceLeave();
    throw new Error(msg);
  }
  const answer = await res.json();
  await pc.setRemoteDescription({ type: "answer", sdp: answer.sdp });
}

export function voiceLeave() {
  if (meterTimer) clearInterval(meterTimer);
  meterTimer = null;
  meters.clear();
  for (const k of Object.keys(local)) {
    if (local[k]) local[k].stop();
    local[k] = null;
  }
  for (const el of audioEls.values()) el.remove();
  audioEls.clear();
  remote.clear();
  if (dc) dc.close();
  if (pc) pc.close();
  pc = null;
  dc = null;
}

export function voiceSetMuted(m) {
  opts.muted = m;
  applyMicEnabled();
}

export function voiceSetDeafened(d) {
  opts.deafened = d;
  for (const el of audioEls.values()) el.muted = d;
  applyMicEnabled();
}

export function voiceSetPtt(enabled) {
  opts.ptt = enabled;
  applyMicEnabled();
}

export function voicePtt(down) {
  pttDown = down;
  applyMicEnabled();
}

export async function voiceSetCamera(on) {
  if (!pc) return false;
  if (local.cam) {
    local.cam.stop();
    local.cam = null;
  }
  if (on) local.cam = await openCamera();
  await senders.cam.sender.replaceTrack(local.cam);
  emit({ type: "tracks" });
  return !!local.cam;
}

function cameraConstraints(id) {
  return { deviceId: id ? { exact: id } : undefined, width: { ideal: 1280 }, height: { ideal: 720 }, frameRate: { ideal: 30 } };
}

// The chosen camera, or the default one if it was unplugged.
async function openCamera() {
  try {
    return (await navigator.mediaDevices.getUserMedia({ video: cameraConstraints(opts.camera) })).getVideoTracks()[0];
  } catch (e) {
    if (!opts.camera) throw e;
    return (await navigator.mediaDevices.getUserMedia({ video: cameraConstraints("") })).getVideoTracks()[0];
  }
}

// Pick another camera; if it's on in a call, swap it in place (no renegotiation).
export async function voiceSetCameraDevice(id) {
  if (id === opts.camera) return;
  opts.camera = id;
  if (pc && local.cam) {
    const old = local.cam;
    local.cam = await openCamera();
    await senders.cam.sender.replaceTrack(local.cam);
    old.stop();
    emit({ type: "tracks" });
  }
}

// Settings preview, independent of any call.
let preview = null;
export async function cameraPreview(el, id) {
  cameraPreviewStop();
  preview = await navigator.mediaDevices.getUserMedia({ video: cameraConstraints(id) });
  el.srcObject = preview;
}

export function cameraPreviewStop() {
  if (preview) preview.getTracks().forEach((t) => t.stop());
  preview = null;
}

export async function voiceSetScreen(on) {
  if (!pc) return false;
  for (const k of ["screen", "screenAudio"]) {
    if (local[k]) {
      local[k].onended = null;
      local[k].stop();
      local[k] = null;
    }
  }
  if (on) {
    const s = await navigator.mediaDevices.getDisplayMedia({ video: { frameRate: { ideal: 30 } }, audio: true });
    local.screen = s.getVideoTracks()[0];
    local.screenAudio = s.getAudioTracks()[0] || null;
    local.screen.contentHint = "detail";
    // The browser's own "Stop sharing" button.
    local.screen.onended = () => emit({ type: "screen_ended" });
  }
  await senders.screen.sender.replaceTrack(local.screen);
  await senders.screen_audio.sender.replaceTrack(local.screenAudio);
  emit({ type: "tracks" });
  return !!local.screen;
}

// Show a stream in a <video>: "local:cam", "local:screen" or "<user>:cam" / "<user>:screen".
export function voiceAttach(el, key) {
  let stream = null;
  if (key === "local:cam" && local.cam) stream = new MediaStream([local.cam]);
  else if (key === "local:screen" && local.screen) stream = new MediaStream([local.screen]);
  else if (remote.has(key)) stream = remote.get(key).stream;
  if (el.srcObject !== stream) el.srcObject = stream;
}

export function voiceHas(key) {
  if (key === "local:cam") return !!local.cam;
  if (key === "local:screen") return !!local.screen;
  const r = remote.get(key);
  return !!r && r.track.readyState === "live" && !r.track.muted;
}

export async function voiceDevices() {
  try {
    const list = await navigator.mediaDevices.enumerateDevices();
    return JSON.stringify(list.map((d) => ({ kind: d.kind, id: d.deviceId, label: d.label || d.kind })));
  } catch (_) {
    return "[]";
  }
}

export async function voiceSetDevices(input, output, noise) {
  const changed = input !== opts.input || noise !== opts.noise;
  opts.input = input;
  opts.output = output;
  opts.noise = noise;
  for (const el of audioEls.values()) if (el.setSinkId) el.setSinkId(output).catch(() => {});
  if (pc && changed) {
    const old = local.mic;
    local.mic = await getMic();
    await senders.mic.sender.replaceTrack(local.mic);
    if (old) old.stop();
    meters.delete("local");
    if (local.mic) addMeter("local", local.mic);
    applyMicEnabled();
  }
}

// ------------------------------------------------------------------ sounds (synthesized, no assets)

function tone(freqs, dur = 0.12, gap = 0.08, vol = 0.12, type = "sine") {
  try {
    const ctx = ensureAudioContext();
    let t = ctx.currentTime + 0.01;
    for (const f of freqs) {
      const o = ctx.createOscillator();
      const g = ctx.createGain();
      o.type = type;
      o.frequency.value = f;
      g.gain.setValueAtTime(0, t);
      g.gain.linearRampToValueAtTime(vol, t + 0.01);
      g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
      o.connect(g).connect(ctx.destination);
      o.start(t);
      o.stop(t + dur + 0.02);
      t += gap;
    }
  } catch (_) {}
}

function sweep(f0, f1, dur, vol) {
  try {
    const ctx = ensureAudioContext();
    const t = ctx.currentTime + 0.01;
    const o = ctx.createOscillator();
    const g = ctx.createGain();
    o.frequency.setValueAtTime(f0, t);
    o.frequency.exponentialRampToValueAtTime(f1, t + dur);
    g.gain.setValueAtTime(0, t);
    g.gain.linearRampToValueAtTime(vol, t + 0.005);
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    o.connect(g).connect(ctx.destination);
    o.start(t);
    o.stop(t + dur + 0.02);
  } catch (_) {}
}

// Audio starts suspended until the first click or key press (browser autoplay rules); wake it then, so the
// first notification sound isn't swallowed. The desktop app also lifts the rule for its webview.
for (const ev of ["pointerdown", "keydown"]) {
  addEventListener(ev, () => { try { ensureAudioContext(); } catch (_) {} }, { once: true, capture: true });
}

const SOUNDS = {
  "message-chime": () => tone([880, 1320], 0.14, 0.07, 0.08),
  "message-pop": () => sweep(520, 1100, 0.09, 0.14),
  "message-soft": () => tone([523, 659], 0.22, 0.1, 0.07, "triangle"),
  "message-ping": () => tone([1568], 0.25, 0, 0.06),
  join: () => tone([523, 784], 0.16, 0.09),
  leave: () => tone([784, 523], 0.16, 0.09),
  mute: () => tone([440], 0.08, 0, 0.08),
  unmute: () => tone([660], 0.08, 0, 0.08),
  connect: () => tone([523, 659, 784], 0.14, 0.08),
  disconnect: () => tone([784, 659, 523], 0.14, 0.08),
};

export function playSound(name) {
  SOUNDS[name]?.();
}

let ringTimer = null;
export function ring(on) {
  if (ringTimer) clearInterval(ringTimer);
  ringTimer = null;
  if (on) {
    const r = () => tone([660, 880, 660, 880], 0.18, 0.2, 0.1);
    r();
    ringTimer = setInterval(r, 2500);
  }
}

// ------------------------------------------------------------------ desktop (Tauri) bridge

export function isTauri() {
  return !!(window.__TAURI__ && window.__TAURI__.core);
}

export async function tauriInvoke(cmd, argsJson) {
  if (!isTauri()) return null;
  try {
    return await window.__TAURI__.core.invoke(cmd, JSON.parse(argsJson));
  } catch (e) {
    console.warn("natter desktop:", cmd, e);
    return null;
  }
}

export function tauriListen(event, cb) {
  if (isTauri()) window.__TAURI__.event.listen(event, (e) => cb(JSON.stringify(e.payload)));
}

export function isAndroid() {
  return /Android/i.test(navigator.userAgent);
}

// ------------------------------------------------------------------ app icon + unread badge
// Drawn here because a canvas has fonts: the tray and taskbar badges carry a real number.

// Gradient stops (3) and the bubble colour per icon variant; names match `APP_ICONS` in settings.rs.
const APP_ICONS = {
  classic: ["#6c76f0", "#4b56d2", "#3540a8", "#ffffff"],
  midnight: ["#3a3c44", "#1b1c20", "#000000", "#ffffff"],
  snow: ["#ffffff", "#eef0f4", "#d5d9e0", "#4b56d2"],
  sunset: ["#ffb35c", "#f0617a", "#8e3fa8", "#ffffff"],
  ocean: ["#5ee0d0", "#2b8cff", "#1f3fa8", "#ffffff"],
  forest: ["#8be09b", "#2f9e5b", "#17543a", "#ffffff"],
  candy: ["#ffa3d7", "#c86cf0", "#6c5ce7", "#ffffff"],
  gold: ["#ffe08a", "#f0a030", "#a8541a", "#ffffff"],
  crimson: ["#ff7a7a", "#da373c", "#7a161b", "#ffffff"],
  mint: ["#d6fbe9", "#86e0c4", "#3fb39a", "#0f3d33"],
};

// The logo's speech bubble with the voice wave cut out (same path as assets/logo.svg).
const BUBBLE =
  "M216,112 H296 A120,120 0 0 1 296,352 H216 A120,120 0 0 1 175,344.8 C160,362 142,382 126,395 Q108,409 111,388 " +
  "C113,362 117,334 124.1,309.1 A120,120 0 0 1 216,112 Z M154,232 a14,14 0 0 1 28,0 v0 a14,14 0 0 1 -28,0 Z " +
  "M198,204 a14,14 0 0 1 28,0 v56 a14,14 0 0 1 -28,0 Z M242,170 a14,14 0 0 1 28,0 v124 a14,14 0 0 1 -28,0 Z " +
  "M286,196 a14,14 0 0 1 28,0 v72 a14,14 0 0 1 -28,0 Z M330,220 a14,14 0 0 1 28,0 v24 a14,14 0 0 1 -28,0 Z";

export function appIconUrl(name) {
  const [a, b, c, fg] = APP_ICONS[name] || APP_ICONS.classic;
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512"><defs>` +
    `<linearGradient id="bg" x1="64" y1="32" x2="448" y2="480" gradientUnits="userSpaceOnUse">` +
    `<stop offset="0" stop-color="${a}"/><stop offset=".55" stop-color="${b}"/><stop offset="1" stop-color="${c}"/></linearGradient>` +
    `<radialGradient id="glow" cx="150" cy="96" r="300" gradientUnits="userSpaceOnUse">` +
    `<stop offset="0" stop-color="#fff" stop-opacity=".22"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient>` +
    `<filter id="lift" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="10" stdDeviation="12" flood-color="#000" flood-opacity=".28"/></filter></defs>` +
    `<rect width="512" height="512" rx="116" fill="url(#bg)"/><rect width="512" height="512" rx="116" fill="url(#glow)"/>` +
    `<path filter="url(#lift)" fill="${fg}" fill-rule="evenodd" d="${BUBBLE}"/></svg>`;
  return "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg);
}

const iconImages = new Map();
function iconImage(name) {
  if (!iconImages.has(name)) {
    const img = new Image();
    img.src = appIconUrl(name);
    iconImages.set(name, img.decode().then(() => img));
  }
  return iconImages.get(name);
}

// Red disc at (cx, cy), with the count in it (9+ past nine) or plain when count is 0.
function badge(g, cx, cy, r, count) {
  g.beginPath();
  g.arc(cx, cy, r, 0, Math.PI * 2);
  g.fillStyle = "#e5383d";
  g.fill();
  if (count > 0) {
    const text = count > 9 ? "9+" : String(count);
    g.fillStyle = "#fff";
    g.textAlign = "center";
    g.textBaseline = "middle";
    g.font = `700 ${Math.round(r * (text.length > 1 ? 1.2 : 1.5))}px "Segoe UI", Inter, system-ui, sans-serif`;
    g.fillText(text, cx, cy + r * 0.08);
  }
}

async function renderIcon(name, size, count, dot) {
  const c = document.createElement("canvas");
  c.width = c.height = size;
  const g = c.getContext("2d");
  g.drawImage(await iconImage(name), 0, 0, size, size);
  if (count > 0 || dot) {
    const r = size * (count > 0 ? 0.3 : 0.2);
    // Cut a ring around the badge so it reads on any icon colour.
    g.save();
    g.globalCompositeOperation = "destination-out";
    g.beginPath();
    g.arc(size - r, size - r, r + size * 0.06, 0, Math.PI * 2);
    g.fill();
    g.restore();
    badge(g, size - r, size - r, r, count);
  }
  return c;
}

// The Windows taskbar overlay: just the badge, full size (Windows shrinks it onto the icon's corner).
function overlay(count) {
  const c = document.createElement("canvas");
  c.width = c.height = 32;
  badge(c.getContext("2d"), 16, 16, count > 0 ? 16 : 10, count);
  return c;
}

const pixels = (c) => Array.from(c.getContext("2d").getImageData(0, 0, c.width, c.height).data);

function setFavicon(url) {
  let l = document.querySelector('link[rel~="icon"]');
  if (!l) {
    l = document.createElement("link");
    l.rel = "icon";
    document.head.append(l);
  }
  l.href = url;
}

// Unread badge: on the tray icon and taskbar (desktop app), or on the tab icon (browser).
// `count` = mentions + unread DMs; `dot` = something else is unread.
export async function showBadge(name, count, dot) {
  try {
    if (isTauri()) {
      const tray = await renderIcon(name, 32, count, dot);
      const over = count > 0 || dot ? { rgba: pixels(overlay(count)), size: 32 } : null;
      await window.__TAURI__.core.invoke("set_badge", { count, tray: { rgba: pixels(tray), size: 32 }, overlay: over });
    } else {
      setFavicon((await renderIcon(name, 64, count, dot)).toDataURL());
    }
  } catch (e) {
    console.warn("natter badge:", e);
  }
}

// Window / taskbar icon in the desktop app.
export async function showAppIcon(name) {
  try {
    if (isTauri()) {
      const c = await renderIcon(name, 128, 0, false);
      await window.__TAURI__.core.invoke("set_app_icon", { icon: { rgba: pixels(c), size: 128 } });
    }
  } catch (e) {
    console.warn("natter icon:", e);
  }
}
