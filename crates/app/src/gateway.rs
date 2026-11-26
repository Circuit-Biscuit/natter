//! Live connection to the server: one WebSocket, heartbeats, reconnects with backoff, idle detection.

use std::cell::{Cell, RefCell};

use leptos::prelude::*;
use laklak_proto::{Event, Id, Op};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{CloseEvent, MessageEvent, WebSocket};

use crate::store::{Conn, store};
use crate::{api, util};

struct Socket {
    ws: WebSocket,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
}

thread_local! {
    static SOCKET: RefCell<Option<Socket>> = const { RefCell::new(None) };
    /// Bumped on every connect/disconnect so stale timers and callbacks know to stand down.
    static GENERATION: Cell<u32> = const { Cell::new(0) };
    static ATTEMPTS: Cell<u32> = const { Cell::new(0) };
    static LAST_ACK: Cell<f64> = const { Cell::new(0.0) };
    static LAST_ACTIVITY: Cell<f64> = const { Cell::new(0.0) };
    static IDLE: Cell<bool> = const { Cell::new(false) };
    static LAST_TYPING: RefCell<(Id, f64)> = const { RefCell::new((0, 0.0)) };
}

fn generation() -> u32 {
    GENERATION.with(Cell::get)
}

pub fn connect() {
    disconnect();
    let generation = generation();
    let s = store();
    s.conn.set(Conn::Connecting);
    let Ok(ws) = WebSocket::new(&api::gateway_url()) else {
        schedule_reconnect(generation);
        return;
    };
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        let Some(text) = e.data().as_string() else { return };
        match serde_json::from_str::<Event>(&text) {
            Ok(Event::Hello { heartbeat_ms }) => start_heartbeat(generation, heartbeat_ms),
            Ok(Event::HeartbeatAck) => LAST_ACK.with(|c| c.set(util::now() as f64)),
            Ok(ev @ Event::Ready(_)) => {
                ATTEMPTS.with(|a| a.set(0));
                LAST_ACK.with(|c| c.set(util::now() as f64));
                store().conn.set(Conn::Online);
                crate::store::apply(ev);
                if IDLE.with(Cell::get) {
                    send(Op::Idle(true));
                }
            }
            Ok(ev) => crate::store::apply(ev),
            Err(err) => leptos::logging::warn!("gateway: unreadable event: {err}"),
        }
    });
    let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |_e: CloseEvent| {
        if generation == self::generation() {
            schedule_reconnect(generation);
        }
    });
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    SOCKET.with(|s| *s.borrow_mut() = Some(Socket { ws, _on_message: on_message, _on_close: on_close }));
}

/// Close on purpose (sign-out). No reconnect.
pub fn disconnect() {
    GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
    SOCKET.with(|s| {
        if let Some(sock) = s.borrow_mut().take() {
            sock.ws.set_onclose(None);
            sock.ws.set_onmessage(None);
            let _ = sock.ws.close();
        }
    });
}

fn schedule_reconnect(generation: u32) {
    let attempt = ATTEMPTS.with(|a| {
        a.set(a.get() + 1);
        a.get()
    });
    // 1 s, 2 s, 4 s … capped at 30 s, with jitter so a restarted server isn't stampeded.
    let base = (1000u64 << attempt.min(5).saturating_sub(1)).min(30_000) as f64;
    let delay = (base * (0.75 + js_sys::Math::random() * 0.5)) as u64;
    store().conn.set(Conn::Retrying(util::now() + delay as i64));
    set_timeout(
        move || {
            if generation == self::generation() && api::token().is_some() {
                connect();
            }
        },
        std::time::Duration::from_millis(delay),
    );
}

/// Retry right now (the "Reconnect" button, or the OS says we're back online).
pub fn reconnect_now() {
    if api::token().is_some() && !matches!(store().conn.get_untracked(), Conn::Online) {
        ATTEMPTS.with(|a| a.set(0));
        connect();
    }
}

fn start_heartbeat(generation: u32, ms: u32) {
    let ms = ms.max(5000);
    let tick = move || {
        if generation != self::generation() {
            return;
        }
        let silent_for = util::now() as f64 - LAST_ACK.with(Cell::get);
        if silent_for > 2.5 * ms as f64 {
            // Half-open connection: drop it and let the close handler reconnect.
            SOCKET.with(|s| {
                if let Some(sock) = s.borrow().as_ref() {
                    let _ = sock.ws.close();
                }
            });
            schedule_reconnect(generation);
            return;
        }
        send(Op::Heartbeat);
        start_heartbeat(generation, ms);
    };
    set_timeout(tick, std::time::Duration::from_millis(ms as u64));
}

pub fn send(op: Op) {
    let Ok(text) = serde_json::to_string(&op) else { return };
    SOCKET.with(|s| {
        if let Some(sock) = s.borrow().as_ref() {
            if sock.ws.ready_state() == WebSocket::OPEN {
                let _ = sock.ws.send_with_str(&text);
            }
        }
    });
}

/// Tell others you're typing — at most every 8 seconds per channel.
pub fn typing(channel: Id) {
    let now = util::now() as f64;
    let due = LAST_TYPING.with(|t| {
        let (c, at) = *t.borrow();
        c != channel || now - at > 8000.0
    });
    if due {
        LAST_TYPING.with(|t| *t.borrow_mut() = (channel, now));
        send(Op::Typing { channel_id: channel });
    }
}

/// Typing stops showing for you once you send.
pub fn typing_sent() {
    LAST_TYPING.with(|t| *t.borrow_mut() = (0, 0.0));
}

/// Any keyboard or mouse use: you're back.
pub fn activity() {
    LAST_ACTIVITY.with(|a| a.set(util::now() as f64));
    if IDLE.with(Cell::get) {
        IDLE.with(|i| i.set(false));
        send(Op::Idle(false));
    }
}

/// Go idle after 10 minutes without input (checked every 30 s); expire typing indicators.
pub fn start_background_timers() {
    activity();
    set_interval(
        || {
            let away = util::now() as f64 - LAST_ACTIVITY.with(Cell::get);
            if away > 600_000.0 && !IDLE.with(Cell::get) {
                IDLE.with(|i| i.set(true));
                send(Op::Idle(true));
            }
        },
        std::time::Duration::from_secs(30),
    );
    set_interval(
        || {
            let now = util::now();
            let s = store();
            if s.typing.with_untracked(|t| t.values().flatten().any(|(_, until)| *until < now)) {
                s.typing.update(|t| {
                    for v in t.values_mut() {
                        v.retain(|(_, until)| *until >= now);
                    }
                });
            }
        },
        std::time::Duration::from_secs(1),
    );
}
