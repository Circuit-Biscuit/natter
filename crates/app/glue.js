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

function tone(freqs, dur = 0.12, gap = 0.08, vol = 0.12) {
  try {
    const ctx = ensureAudioContext();
    let t = ctx.currentTime + 0.01;
    for (const f of freqs) {
      const o = ctx.createOscillator();
      const g = ctx.createGain();
      o.type = "sine";
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

const SOUNDS = {
  message: () => tone([880, 1320], 0.14, 0.07, 0.08),
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
