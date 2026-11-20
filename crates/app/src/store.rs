//! All app state, as signals, plus the actions that change it.
//!
//! One `Store` (a bundle of `Copy` signal handles) lives for the whole app. Gateway events land in
//! `apply`, user actions go through the functions at the bottom, which update the UI first
//! (optimistically) and then tell the server.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

/// A shared action. Plain `Arc`s (not arena `Callback`s) so menus and dialogs can never outlive them.
pub type Action = Arc<dyn Fn() + Send + Sync>;

use leptos::prelude::*;
use leptos::task::spawn_local;
use natter_proto::*;
use serde::{Deserialize, Serialize};

use crate::{api, util};

// ---------------------------------------------------------------- types

/// Settings synced between your devices (stored on the server as JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "dark", "light" or "system".
    pub theme: String,
    pub compact: bool,
    pub font_size: u8,
    pub notifications: bool,
    pub sounds: bool,
    /// Muted spaces and channels: no unread marks, no notifications.
    pub muted: Vec<Id>,
    /// Collapsed categories.
    pub collapsed: Vec<Id>,
    pub show_members: bool,
    /// Recently used reactions, most recent first.
    pub recent_emoji: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "dark".into(),
            compact: false,
            font_size: 16,
            notifications: true,
            sounds: true,
            muted: vec![],
            collapsed: vec![],
            show_members: true,
            recent_emoji: vec!["👍".into(), "❤️".into(), "😂".into(), "🎉".into(), "👀".into()],
        }
    }
}

/// Per-device voice preferences (never synced: devices differ per machine).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoicePrefs {
    pub input: String,
    pub output: String,
    /// Camera `deviceId`; empty = the system default.
    pub camera: String,
    pub push_to_talk: bool,
    /// `KeyboardEvent.code`, e.g. "Backquote".
    pub ptt_key: String,
    pub noise_suppression: bool,
}

impl Default for VoicePrefs {
    fn default() -> Self {
        VoicePrefs {
            input: String::new(),
            output: String::new(),
            camera: String::new(),
            push_to_talk: false,
            ptt_key: "Backquote".into(),
            noise_suppression: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Log {
    /// Sorted by id. Unsent messages have huge ids so they sort last.
    pub msgs: Vec<Message>,
    pub more_before: bool,
    /// We are looking at an older stretch (after a jump), newer messages exist.
    pub more_after: bool,
    pub loading: bool,
    pub loaded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conn {
    Connecting,
    Online,
    /// Lost; retrying at the given time.
    Retrying(Millis),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nav {
    /// `None` = Home (direct messages).
    pub space: Option<Id>,
    pub channel: Option<Id>,
}

#[derive(Clone)]
pub enum Modal {
    CreateSpace,
    Invite(Id),
    CreateChannel { space: Id, kind: ChannelKind, parent: Option<Id> },
    EditChannel(Id),
    SpaceSettings(Id),
    UserSettings,
    QuickSwitcher,
    Shortcuts,
    NewDm,
    Image(String),
    Pins(Id),
    Nextcloud,
    CreateThread { channel: Id, message: Option<Id> },
    Threads(Id),
    Confirm { title: String, body: String, ok: String, action: Action },
}

#[derive(Clone)]
pub struct MenuItem {
    pub label: String,
    pub danger: bool,
    pub action: Action,
}

impl MenuItem {
    pub fn new(label: impl Into<String>, f: impl Fn() + Send + Sync + 'static) -> Self {
        MenuItem { label: label.into(), danger: false, action: Arc::new(f) }
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

#[derive(Clone)]
pub struct Menu {
    pub x: i32,
    pub y: i32,
    /// `None` entries draw a separator.
    pub items: Vec<Option<MenuItem>>,
}

#[derive(Clone, Copy, PartialEq)]
pub struct Popout {
    pub user: Id,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, PartialEq)]
pub struct Toast {
    pub id: u32,
    pub text: String,
    pub error: bool,
}

/// Your own side of a call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Call {
    pub channel: Option<Id>,
    pub connecting: bool,
    pub muted: bool,
    pub deafened: bool,
    pub video: bool,
    pub screen: bool,
    /// Users currently speaking (yourself included).
    pub speaking: HashSet<Id>,
    /// Push-to-talk key held.
    pub ptt_down: bool,
    /// Tile shown large ("<user>:<kind>").
    pub focus: Option<String>,
}

#[derive(Clone, Copy)]
pub struct Store {
    pub me: RwSignal<Option<User>>,
    pub my_status: RwSignal<Status>,
    pub info: RwSignal<Option<ServerInfo>>,
    pub nextcloud_session: RwSignal<bool>,
    pub users: RwSignal<HashMap<Id, User>>,
    pub presence: RwSignal<HashMap<Id, Status>>,
    pub spaces: RwSignal<Vec<Space>>,
    pub dms: RwSignal<Vec<Channel>>,
    pub read: RwSignal<HashMap<Id, ReadState>>,
    pub logs: RwSignal<HashMap<Id, Log>>,
    pub failed: RwSignal<HashSet<String>>,
    pub voice: RwSignal<HashMap<Id, VoiceState>>,
    pub typing: RwSignal<HashMap<Id, Vec<(Id, Millis)>>>,
    pub settings: RwSignal<Settings>,
    pub prefs: RwSignal<VoicePrefs>,
    pub conn: RwSignal<Conn>,
    pub ready: RwSignal<bool>,
    pub nav: RwSignal<Nav>,
    pub modal: RwSignal<Option<Modal>>,
    pub menu: RwSignal<Option<Menu>>,
    pub popout: RwSignal<Option<Popout>>,
    pub toasts: RwSignal<Vec<Toast>>,
    pub reply_to: RwSignal<Option<Id>>,
    pub editing: RwSignal<Option<Id>>,
    /// Message to scroll to and flash.
    pub highlight: RwSignal<Option<Id>>,
    pub drafts: RwSignal<HashMap<Id, String>>,
    pub call: RwSignal<Call>,
    /// Incoming DM call waiting for an answer: (channel, caller).
    pub ringing: RwSignal<Option<(Id, Id)>>,
    pub shift: RwSignal<bool>,
    pub search: RwSignal<Option<(String, SearchResult)>>,
    pub focused: RwSignal<bool>,
    /// Thread open in the side panel.
    pub thread: RwSignal<Option<Id>>,
    /// Narrow screen (phone): one column at a time.
    pub mobile: RwSignal<bool>,
    /// Which column a phone shows.
    pub pane: RwSignal<Pane>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// Spaces and channel list.
    Nav,
    Chat,
    Members,
}

/// Phones show one column; anything narrower than this counts as a phone.
pub const MOBILE_MAX_WIDTH: f64 = 760.0;

pub fn window_is_narrow() -> bool {
    util::window().inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1200.0) < MOBILE_MAX_WIDTH
}

/// On a phone every screen you open (chat, thread, members, search) is a history entry, so Android's back
/// button and the in-app back arrow close them one at a time.
pub fn push_layer() {
    if store().mobile.get_untracked() {
        if let Ok(h) = util::window().history() {
            let _ = h.push_state_with_url(&wasm_bindgen::JsValue::NULL, "", None);
        }
    }
}

/// The in-app back arrow (phones).
pub fn go_back() {
    if let Ok(h) = util::window().history() {
        let _ = h.back();
    }
}

/// Undo the top-most phone layer (called on `popstate`).
pub fn pop_layer() {
    let s = store();
    if !s.mobile.get_untracked() {
        return;
    }
    if s.thread.get_untracked().is_some() {
        s.thread.set(None);
    } else if s.pane.get_untracked() == Pane::Members {
        s.pane.set(Pane::Chat);
    } else if s.search.with_untracked(|r| r.is_some()) {
        s.search.set(None);
    } else if s.pane.get_untracked() == Pane::Chat {
        s.pane.set(Pane::Nav);
    }
}

static STORE: OnceLock<Store> = OnceLock::new();

pub fn store() -> Store {
    *STORE.get().expect("store initialised")
}

impl Store {
    pub fn init() -> Store {
        let prefs = util::storage_get("natter.voice").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let settings = util::storage_get("natter.settings").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let s = Store {
            me: RwSignal::new(None),
            my_status: RwSignal::new(Status::Online),
            info: RwSignal::new(None),
            nextcloud_session: RwSignal::new(false),
            users: RwSignal::new(HashMap::new()),
            presence: RwSignal::new(HashMap::new()),
            spaces: RwSignal::new(vec![]),
            dms: RwSignal::new(vec![]),
            read: RwSignal::new(HashMap::new()),
            logs: RwSignal::new(HashMap::new()),
            failed: RwSignal::new(HashSet::new()),
            voice: RwSignal::new(HashMap::new()),
            typing: RwSignal::new(HashMap::new()),
            settings: RwSignal::new(settings),
            prefs: RwSignal::new(prefs),
            conn: RwSignal::new(Conn::Connecting),
            ready: RwSignal::new(false),
            nav: RwSignal::new(Nav::default()),
            modal: RwSignal::new(None),
            menu: RwSignal::new(None),
            popout: RwSignal::new(None),
            toasts: RwSignal::new(vec![]),
            reply_to: RwSignal::new(None),
            editing: RwSignal::new(None),
            highlight: RwSignal::new(None),
            drafts: RwSignal::new(HashMap::new()),
            call: RwSignal::new(Call::default()),
            ringing: RwSignal::new(None),
            shift: RwSignal::new(false),
            search: RwSignal::new(None),
            focused: RwSignal::new(true),
            thread: RwSignal::new(None),
            mobile: RwSignal::new(window_is_narrow()),
            pane: RwSignal::new(Pane::Nav),
        };
        let _ = STORE.set(s);
        s
    }

    // ------------------------------------------------------------ lookups (reactive)

    pub fn me_id(&self) -> Id {
        self.me.with(|m| m.as_ref().map(|u| u.id).unwrap_or(0))
    }

    pub fn user(&self, id: Id) -> Option<User> {
        self.users.with(|u| u.get(&id).cloned())
    }

    pub fn space(&self, id: Id) -> Option<Space> {
        self.spaces.with(|s| s.iter().find(|s| s.id == id).cloned())
    }

    pub fn channel(&self, id: Id) -> Option<Channel> {
        self.spaces
            .with(|s| s.iter().flat_map(|s| s.channels.iter()).find(|c| c.id == id).cloned())
            .or_else(|| self.dms.with(|d| d.iter().find(|c| c.id == id).cloned()))
    }

    pub fn member(&self, space: Id, user: Id) -> Option<Member> {
        self.spaces.with(|s| {
            s.iter().find(|s| s.id == space).and_then(|s| s.members.iter().find(|m| m.user_id == user).cloned())
        })
    }

    pub fn my_role(&self, space: Id) -> Option<Role> {
        self.member(space, self.me_id()).map(|m| m.role)
    }

    pub fn is_mod(&self, space: Option<Id>) -> bool {
        match space {
            Some(s) => self.my_role(s).is_some_and(Role::is_mod),
            None => true,
        }
    }

    /// Nickname in the space, else display name.
    pub fn name(&self, user: Id, space: Option<Id>) -> String {
        if let Some(n) = space.and_then(|s| self.member(s, user)).and_then(|m| m.nickname) {
            return n;
        }
        self.user(user).map(|u| u.display_name).unwrap_or_else(|| "Unknown user".into())
    }

    pub fn status(&self, user: Id) -> Status {
        if user == self.me_id() {
            let s = self.my_status.get();
            return if s == Status::Invisible { Status::Offline } else { s };
        }
        self.presence.with(|p| p.get(&user).copied().unwrap_or(Status::Offline))
    }

    /// Name shown for a channel: `general`, or the other people in a DM.
    pub fn channel_title(&self, c: &Channel) -> String {
        if c.kind.is_dm() {
            let me = self.me_id();
            let names: Vec<String> =
                c.recipients.iter().filter(|&&u| u != me).map(|&u| self.name(u, None)).collect();
            if !c.name.is_empty() && c.kind == ChannelKind::GroupDm {
                return c.name.clone();
            }
            if names.is_empty() { self.name(me, None) } else { names.join(", ") }
        } else {
            c.name.clone()
        }
    }

    /// Active threads of a channel, most recently active first.
    pub fn threads_of(&self, parent: Id) -> Vec<Channel> {
        let mut v: Vec<Channel> = self.spaces.with(|s| {
            s.iter()
                .flat_map(|s| s.channels.iter())
                .filter(|c| c.kind == ChannelKind::Thread && c.parent_id == Some(parent) && !c.thread.as_ref().is_some_and(|t| t.archived))
                .cloned()
                .collect()
        });
        v.sort_by_key(|c| std::cmp::Reverse(c.last_message_id.unwrap_or(c.id)));
        v
    }

    pub fn dm_partner(&self, c: &Channel) -> Option<Id> {
        let me = self.me_id();
        (c.kind == ChannelKind::Dm).then(|| c.recipients.iter().copied().find(|&u| u != me).unwrap_or(me))
    }

    pub fn muted(&self, id: Id) -> bool {
        self.settings.with(|s| s.muted.contains(&id))
    }

    pub fn unread(&self, c: &Channel) -> bool {
        let Some(last) = c.last_message_id else { return false };
        self.read.with(|r| r.get(&c.id).map(|r| r.last_read_id).unwrap_or(0)) < last
    }

    pub fn mentions(&self, channel: Id) -> u32 {
        self.read.with(|r| r.get(&channel).map(|r| r.mention_count).unwrap_or(0))
    }

    /// (has unread, mention count) for a whole space, skipping muted channels.
    pub fn space_badge(&self, space: Id) -> (bool, u32) {
        let Some(sp) = self.space(space) else { return (false, 0) };
        let space_muted = self.muted(space);
        let mut unread = false;
        let mut mentions = 0;
        for c in sp.channels.iter().filter(|c| c.kind.has_messages()) {
            if self.muted(c.id) {
                continue;
            }
            mentions += self.mentions(c.id);
            unread |= !space_muted && self.unread(c);
        }
        (unread, mentions)
    }

    /// DMs with unread messages, newest first (shown under the Home button).
    pub fn unread_dms(&self) -> Vec<(Channel, u32)> {
        let mut v: Vec<(Channel, u32)> = self
            .dms
            .get()
            .into_iter()
            .filter(|c| !self.muted(c.id) && self.unread(c))
            .map(|c| {
                let n = self.mentions(c.id).max(1);
                (c, n)
            })
            .collect();
        v.sort_by_key(|(c, _)| std::cmp::Reverse(c.last_message_id));
        v
    }

    pub fn total_mentions(&self) -> u32 {
        let dms: u32 = self.unread_dms().iter().map(|(_, n)| n).sum();
        let spaces: u32 = self.spaces.with(|s| s.iter().map(|s| s.id).collect::<Vec<_>>()).iter().map(|&s| self.space_badge(s).1).sum();
        dms + spaces
    }

    pub fn typing_in(&self, channel: Id) -> Vec<Id> {
        let me = self.me_id();
        self.typing.with(|t| t.get(&channel).map(|v| v.iter().map(|(u, _)| *u).filter(|&u| u != me).collect()).unwrap_or_default())
    }

    pub fn voice_in(&self, channel: Id) -> Vec<VoiceState> {
        let mut v: Vec<VoiceState> = self.voice.with(|m| m.values().filter(|s| s.channel_id == channel).cloned().collect());
        v.sort_by_key(|s| s.joined_at);
        v
    }

    pub fn log(&self, channel: Id) -> Log {
        self.logs.with(|l| l.get(&channel).cloned().unwrap_or_default())
    }

    pub fn message(&self, channel: Id, id: Id) -> Option<Message> {
        self.logs.with(|l| l.get(&channel).and_then(|l| l.msgs.iter().find(|m| m.id == id).cloned()))
    }

    // ------------------------------------------------------------ small UI helpers

    pub fn toast(&self, text: impl Into<String>, error: bool) {
        let id = (js_sys::Math::random() * 1e9) as u32;
        self.toasts.update(|t| t.push(Toast { id, text: text.into(), error }));
        let toasts = self.toasts;
        set_timeout(move || toasts.update(|t| t.retain(|x| x.id != id)), std::time::Duration::from_millis(4500));
    }

    pub fn error(&self, e: api::Error) {
        self.toast(e.message, true);
    }

    pub fn confirm(&self, title: &str, body: &str, ok: &str, f: impl Fn() + Send + Sync + 'static) {
        self.modal.set(Some(Modal::Confirm {
            title: title.into(),
            body: body.into(),
            ok: ok.into(),
            action: Arc::new(f),
        }));
    }

    pub fn open_menu(&self, ev: &web_sys::MouseEvent, items: Vec<Option<MenuItem>>) {
        ev.prevent_default();
        ev.stop_propagation();
        self.popout.set(None);
        self.menu.set(Some(Menu { x: ev.client_x(), y: ev.client_y(), items }));
    }

    pub fn open_profile(&self, ev: &web_sys::MouseEvent, user: Id) {
        ev.stop_propagation();
        self.menu.set(None);
        self.popout.set(Some(Popout { user, x: ev.client_x(), y: ev.client_y() }));
    }

    pub fn save_settings(&self) {
        let s = self.settings.get_untracked();
        let json = serde_json::to_value(&s).unwrap_or_default();
        util::storage_set("natter.settings", &json.to_string());
        spawn_local(async move {
            let _: Result<(), _> = api::call("PUT", "/api/me/settings", &json).await;
        });
    }

    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) {
        self.settings.update(f);
        self.save_settings();
    }

    pub fn save_prefs(&self) {
        util::storage_set("natter.voice", &serde_json::to_string(&self.prefs.get_untracked()).unwrap_or_default());
    }
}

// ---------------------------------------------------------------- navigation

pub fn open_space(space: Option<Id>) {
    let s = store();
    let key = format!("natter.last.{}", space.unwrap_or(0));
    let remembered = util::storage_get(&key).and_then(|v| v.parse::<Id>().ok());
    let channel = match space {
        Some(sp) => {
            let chans = s.space(sp).map(|s| s.channels).unwrap_or_default();
            remembered
                .filter(|id| chans.iter().any(|c| c.id == *id && c.kind == ChannelKind::Text))
                .or_else(|| first_text_channel(&chans))
        }
        None => remembered.filter(|id| s.dms.with_untracked(|d| d.iter().any(|c| c.id == *id))),
    };
    s.nav.set(Nav { space, channel });
    if let Some(c) = channel {
        load_channel(c);
    }
}

pub fn first_text_channel(chans: &[Channel]) -> Option<Id> {
    let mut v: Vec<&Channel> = chans.iter().filter(|c| c.kind == ChannelKind::Text).collect();
    v.sort_by_key(|c| (chans.iter().find(|p| Some(p.id) == c.parent_id).map(|p| p.position).unwrap_or(-1), c.position));
    v.first().map(|c| c.id)
}

pub fn open_channel(channel: Id) {
    let s = store();
    let Some(c) = s.channel(channel) else { return };
    // A thread opens in the side panel next to its channel.
    if c.kind == ChannelKind::Thread {
        if let Some(parent) = c.parent_id {
            if s.nav.get_untracked().channel != Some(parent) {
                open_channel(parent);
            }
            s.search.set(None);
            // From a phone's channel list: chat underneath, thread on top (back closes them in turn).
            if s.mobile.get_untracked() && s.pane.get_untracked() == Pane::Nav {
                push_layer();
                s.pane.set(Pane::Chat);
            }
            if s.thread.get_untracked() != Some(channel) {
                push_layer();
            }
            s.thread.set(Some(channel));
            load_channel(channel);
            return;
        }
    }
    s.thread.set(None);
    if s.mobile.get_untracked() && s.pane.get_untracked() == Pane::Nav {
        push_layer();
        s.pane.set(Pane::Chat);
    }
    s.nav.set(Nav { space: c.space_id, channel: Some(channel) });
    s.reply_to.set(None);
    s.editing.set(None);
    s.search.set(None);
    util::storage_set(&format!("natter.last.{}", c.space_id.unwrap_or(0)), &channel.to_string());
    load_channel(channel);
}

/// Open (or create) the DM with someone.
pub fn open_dm(user: Id) {
    let s = store();
    let existing = s.dms.with_untracked(|d| {
        d.iter().find(|c| c.kind == ChannelKind::Dm && c.recipients.contains(&user) && (user != s.me_id() || c.recipients.len() == 1 || c.recipients.iter().all(|&r| r == user))).map(|c| c.id)
    });
    if let Some(id) = existing {
        s.popout.set(None);
        open_channel(id);
        return;
    }
    spawn_local(async move {
        match api::call::<_, Channel>("POST", "/api/dms", &CreateDm { recipients: vec![user] }).await {
            Ok(c) => {
                let id = c.id;
                upsert_dm(c);
                s.popout.set(None);
                open_channel(id);
            }
            Err(e) => s.error(e),
        }
    });
}

fn upsert_dm(c: Channel) {
    store().dms.update(|d| match d.iter_mut().find(|x| x.id == c.id) {
        Some(x) => *x = c,
        None => d.push(c),
    });
}

// ---------------------------------------------------------------- messages

const PAGE: usize = 50;

/// Make sure the newest messages of a channel are loaded.
pub fn load_channel(channel: Id) {
    let s = store();
    let log = s.logs.with_untracked(|l| l.get(&channel).cloned());
    if log.as_ref().is_some_and(|l| l.loaded || l.loading) {
        return;
    }
    fetch_latest(channel);
}

/// (Re)load the newest page and merge it in (also used after reconnecting).
pub fn fetch_latest(channel: Id) {
    let s = store();
    s.logs.update(|l| l.entry(channel).or_default().loading = true);
    spawn_local(async move {
        let r: Result<Vec<Message>, _> = api::get(&format!("/api/channels/{channel}/messages?limit={PAGE}")).await;
        s.logs.update(|l| {
            let log = l.entry(channel).or_default();
            log.loading = false;
            match r {
                Ok(msgs) => {
                    let full = msgs.len() >= PAGE;
                    if log.more_after || !log.loaded {
                        // Fresh start (or we were viewing an old window): replace, keep unsent ones.
                        let unsent: Vec<Message> = log.msgs.drain(..).filter(|m| m.id >= PENDING_BASE).collect();
                        log.msgs = msgs;
                        log.msgs.extend(unsent);
                        log.more_before = full;
                    } else {
                        merge(&mut log.msgs, msgs);
                    }
                    log.more_after = false;
                    log.loaded = true;
                }
                Err(_) => {}
            }
        });
    });
}

fn merge(into: &mut Vec<Message>, new: Vec<Message>) {
    for m in new {
        if let Some(n) = &m.nonce {
            into.retain(|x| !(x.id >= PENDING_BASE && x.nonce.as_ref() == Some(n)));
        }
        match into.binary_search_by_key(&m.id, |x| x.id) {
            Ok(i) => into[i] = m,
            Err(i) => into.insert(i, m),
        }
    }
}

pub fn load_older(channel: Id) {
    let s = store();
    let (busy, oldest) = s.logs.with_untracked(|l| {
        let log = l.get(&channel);
        (
            log.is_none_or(|l| l.loading || !l.more_before),
            log.and_then(|l| l.msgs.first().map(|m| m.id)),
        )
    });
    let Some(oldest) = oldest else { return };
    if busy {
        return;
    }
    s.logs.update(|l| l.entry(channel).or_default().loading = true);
    spawn_local(async move {
        let r: Result<Vec<Message>, _> = api::get(&format!("/api/channels/{channel}/messages?before={oldest}&limit={PAGE}")).await;
        s.logs.update(|l| {
            let log = l.entry(channel).or_default();
            log.loading = false;
            if let Ok(msgs) = r {
                log.more_before = msgs.len() >= PAGE;
                merge(&mut log.msgs, msgs);
            }
        });
    });
}

pub fn load_newer(channel: Id) {
    let s = store();
    let (busy, newest) = s.logs.with_untracked(|l| {
        let log = l.get(&channel);
        (
            log.is_none_or(|l| l.loading || !l.more_after),
            log.and_then(|l| l.msgs.iter().rev().find(|m| m.id < PENDING_BASE).map(|m| m.id)),
        )
    });
    let Some(newest) = newest else { return };
    if busy {
        return;
    }
    s.logs.update(|l| l.entry(channel).or_default().loading = true);
    spawn_local(async move {
        let r: Result<Vec<Message>, _> = api::get(&format!("/api/channels/{channel}/messages?after={newest}&limit={PAGE}")).await;
        s.logs.update(|l| {
            let log = l.entry(channel).or_default();
            log.loading = false;
            if let Ok(msgs) = r {
                log.more_after = msgs.len() >= PAGE;
                merge(&mut log.msgs, msgs);
            }
        });
    });
}

/// Scroll to a message, loading the stretch around it if needed.
pub fn jump_to(channel: Id, message: Id) {
    let s = store();
    if s.nav.get_untracked().channel != Some(channel) {
        open_channel(channel);
    }
    if s.message(channel, message).is_some() {
        s.highlight.set(Some(message));
        return;
    }
    s.logs.update(|l| l.entry(channel).or_default().loading = true);
    spawn_local(async move {
        let r: Result<Vec<Message>, _> =
            api::get(&format!("/api/channels/{channel}/messages?around={message}&limit={PAGE}")).await;
        s.logs.update(|l| {
            let log = l.entry(channel).or_default();
            log.loading = false;
            if let Ok(msgs) = r {
                log.more_before = true;
                log.more_after = msgs.last().map(|m| m.id) < s.channel(channel).and_then(|c| c.last_message_id);
                log.msgs = msgs;
                log.loaded = true;
            }
        });
        s.highlight.set(Some(message));
    });
}

/// Ids at or above this belong to messages not yet confirmed by the server.
pub const PENDING_BASE: Id = 1 << 52;

pub fn send_message(channel: Id, content: String, attachments: Vec<Attachment>, silent: bool, reply_ping: bool) {
    let s = store();
    let content = util::expand_shortcodes(content.trim());
    if content.is_empty() && attachments.is_empty() {
        return;
    }
    let reply_to = s.reply_to.get_untracked().filter(|r| s.message(channel, *r).is_some());
    if reply_to.is_some() {
        s.reply_to.set(None);
    }
    let nonce = util::nonce();
    let me = s.me_id();
    let pending = Message {
        id: PENDING_BASE + util::now(),
        channel_id: channel,
        author_id: me,
        kind: MessageKind::Default,
        content: content.clone(),
        created_at: util::now(),
        edited_at: None,
        reply: reply_to.and_then(|r| s.message(channel, r)).map(|m| ReplyPreview {
            id: m.id,
            author_id: m.author_id,
            content: m.content.chars().take(200).collect(),
            has_attachments: !m.attachments.is_empty(),
            deleted: false,
        }),
        attachments: attachments.clone(),
        reactions: vec![],
        mentions: parse_user_mentions(&content),
        mention_everyone: false,
        pinned: false,
        nonce: Some(nonce.clone()),
        thread: None,
    };
    s.logs.update(|l| l.entry(channel).or_default().msgs.push(pending));
    let body = SendMessage {
        content,
        reply_to,
        attachments: attachments.iter().map(|a| a.id).collect(),
        nonce: Some(nonce),
        silent,
        reply_ping: reply_ping && reply_to.is_some(),
    };
    post_message(channel, body);
}

fn post_message(channel: Id, body: SendMessage) {
    let s = store();
    let nonce = body.nonce.clone().unwrap_or_default();
    s.failed.update(|f| {
        f.remove(&nonce);
    });
    spawn_local(async move {
        // Retry transient failures; the nonce makes retries safe.
        let mut delay = 500;
        for attempt in 0..4 {
            match api::call::<_, Message>("POST", &format!("/api/channels/{channel}/messages"), &body).await {
                Ok(m) => {
                    on_message(m, false);
                    return;
                }
                Err(e) if e.status == 0 && attempt < 3 => {
                    gloo_timers::future::TimeoutFuture::new(delay).await;
                    delay *= 3;
                }
                Err(e) => {
                    s.failed.update(|f| {
                        f.insert(nonce.clone());
                    });
                    if e.status != 0 {
                        s.error(e);
                    }
                    return;
                }
            }
        }
    });
}

/// Try sending a failed message again.
pub fn retry(channel: Id, nonce: String) {
    let s = store();
    let Some(m) = s.logs.with_untracked(|l| {
        l.get(&channel).and_then(|l| l.msgs.iter().find(|m| m.nonce.as_ref() == Some(&nonce)).cloned())
    }) else {
        return;
    };
    post_message(
        channel,
        SendMessage {
            content: m.content,
            reply_to: m.reply.map(|r| r.id),
            attachments: m.attachments.iter().map(|a| a.id).collect(),
            nonce: Some(nonce),
            silent: false,
            reply_ping: false,
        },
    );
}

pub fn discard_failed(channel: Id, nonce: String) {
    store().logs.update(|l| {
        if let Some(log) = l.get_mut(&channel) {
            log.msgs.retain(|m| m.nonce.as_ref() != Some(&nonce) || m.id < PENDING_BASE);
        }
    });
}

pub fn edit_message(channel: Id, id: Id, content: String) {
    let s = store();
    let content = util::expand_shortcodes(content.trim());
    s.editing.set(None);
    if s.message(channel, id).is_some_and(|m| m.content == content) {
        return;
    }
    s.logs.update(|l| {
        if let Some(m) = l.get_mut(&channel).and_then(|l| l.msgs.iter_mut().find(|m| m.id == id)) {
            m.content = content.clone();
            m.edited_at = Some(util::now());
        }
    });
    spawn_local(async move {
        if let Err(e) =
            api::call::<_, Message>("PATCH", &format!("/api/channels/{channel}/messages/{id}"), &EditMessage { content }).await
        {
            s.error(e);
            fetch_latest(channel);
        }
    });
}

pub fn delete_message(channel: Id, id: Id) {
    let s = store();
    remove_message(channel, id);
    spawn_local(async move {
        if let Err(e) = api::act("DELETE", &format!("/api/channels/{channel}/messages/{id}")).await {
            s.error(e);
            fetch_latest(channel);
        }
    });
}

/// Ask first, unless Shift is held (Discord's shortcut).
pub fn delete_with_confirm(channel: Id, id: Id) {
    let s = store();
    if s.shift.get_untracked() {
        delete_message(channel, id);
    } else {
        s.confirm("Delete message", "Are you sure you want to delete this message? Tip: hold Shift to skip this.", "Delete", move || {
            delete_message(channel, id)
        });
    }
}

fn remove_message(channel: Id, id: Id) {
    let s = store();
    s.logs.update(|l| {
        if let Some(log) = l.get_mut(&channel) {
            log.msgs.retain(|m| m.id != id);
            for m in log.msgs.iter_mut() {
                if let Some(r) = m.reply.as_mut().filter(|r| r.id == id) {
                    r.deleted = true;
                    r.content.clear();
                }
            }
        }
    });
}

pub fn toggle_reaction(channel: Id, id: Id, emoji: String) {
    let s = store();
    let me = s.me_id();
    let had = s.message(channel, id).is_some_and(|m| m.reactions.iter().any(|r| r.emoji == emoji && r.users.contains(&me)));
    apply_reaction(channel, id, me, &emoji, !had);
    if !had {
        s.update_settings(|st| {
            st.recent_emoji.retain(|e| *e != emoji);
            st.recent_emoji.insert(0, emoji.clone());
            st.recent_emoji.truncate(12);
        });
    }
    let method = if had { "DELETE" } else { "PUT" };
    spawn_local(async move {
        if let Err(e) = api::act(method, &format!("/api/channels/{channel}/messages/{id}/reactions/{}", api::enc(&emoji))).await {
            s.error(e);
            apply_reaction(channel, id, me, &emoji, had);
        }
    });
}

fn apply_reaction(channel: Id, id: Id, user: Id, emoji: &str, add: bool) {
    store().logs.update(|l| {
        let Some(m) = l.get_mut(&channel).and_then(|l| l.msgs.iter_mut().find(|m| m.id == id)) else { return };
        match m.reactions.iter_mut().position(|r| r.emoji == emoji) {
            Some(i) => {
                let r = &mut m.reactions[i];
                if add {
                    if !r.users.contains(&user) {
                        r.users.push(user);
                    }
                } else {
                    r.users.retain(|&u| u != user);
                    if r.users.is_empty() {
                        m.reactions.remove(i);
                    }
                }
            }
            None if add => m.reactions.push(Reaction { emoji: emoji.to_string(), users: vec![user] }),
            None => {}
        }
    });
}

pub fn set_pinned(channel: Id, id: Id, pin: bool) {
    let s = store();
    spawn_local(async move {
        let method = if pin { "PUT" } else { "DELETE" };
        match api::act(method, &format!("/api/channels/{channel}/pins/{id}")).await {
            Ok(()) => s.toast(if pin { "Message pinned." } else { "Message unpinned." }, false),
            Err(e) => s.error(e),
        }
    });
}

/// Mark read up to `message` (or everything). Cheap to call often: it skips no-ops.
pub fn ack(channel: Id, message: Option<Id>) {
    let s = store();
    let Some(target) = message.or_else(|| s.channel(channel).and_then(|c| c.last_message_id)) else { return };
    let current = s.read.with_untracked(|r| r.get(&channel).copied());
    if current.is_some_and(|r| r.last_read_id >= target && r.mention_count == 0) {
        return;
    }
    set_read(channel, target);
}

/// Alt+click: mark unread from this message on.
pub fn mark_unread(channel: Id, message: Id) {
    let s = store();
    let prev = s.logs.with_untracked(|l| {
        l.get(&channel).and_then(|l| l.msgs.iter().rev().find(|m| m.id < message).map(|m| m.id))
    });
    set_read(channel, prev.unwrap_or(message - 1));
    s.toast("Marked as unread.", false);
}

fn set_read(channel: Id, id: Id) {
    let s = store();
    s.read.update(|r| {
        let e = r.entry(channel).or_insert(ReadState { channel_id: channel, last_read_id: 0, mention_count: 0 });
        if id >= e.last_read_id {
            e.mention_count = 0;
        }
        e.last_read_id = id;
    });
    spawn_local(async move {
        let _: Result<(), _> = api::call("POST", &format!("/api/channels/{channel}/ack"), &Ack { message_id: id }).await;
    });
}

pub fn mark_space_read(space: Id) {
    let s = store();
    for c in s.space(space).map(|s| s.channels).unwrap_or_default() {
        if s.unread(&c) || s.mentions(c.id) > 0 {
            ack(c.id, None);
        }
    }
}

// ---------------------------------------------------------------- gateway events

pub fn apply(ev: Event) {
    let s = store();
    match ev {
        Event::Hello { .. } | Event::HeartbeatAck => {}
        Event::Ready(r) => apply_ready(*r),
        Event::MessageCreate(m) => on_message(m, true),
        Event::MessageUpdate(m) => {
            s.logs.update(|l| {
                if let Some(x) = l.get_mut(&m.channel_id).and_then(|l| l.msgs.iter_mut().find(|x| x.id == m.id)) {
                    *x = m.clone();
                }
                // Replies quoting it show the new text.
                if let Some(log) = l.get_mut(&m.channel_id) {
                    for x in log.msgs.iter_mut() {
                        if let Some(r) = x.reply.as_mut().filter(|r| r.id == m.id) {
                            r.content = m.content.chars().take(200).collect();
                        }
                    }
                }
            });
        }
        Event::MessageDelete { id, channel_id } => remove_message(channel_id, id),
        Event::ReactionAdd { channel_id, message_id, user_id, emoji } => {
            if user_id != s.me_id() || s.message(channel_id, message_id).is_some_and(|m| !m.reactions.iter().any(|r| r.emoji == emoji && r.users.contains(&user_id))) {
                apply_reaction(channel_id, message_id, user_id, &emoji, true)
            }
        }
        Event::ReactionRemove { channel_id, message_id, user_id, emoji } => apply_reaction(channel_id, message_id, user_id, &emoji, false),
        Event::TypingStart { channel_id, user_id } => {
            let until = util::now() + 10_000;
            s.typing.update(|t| {
                let v = t.entry(channel_id).or_default();
                v.retain(|(u, _)| *u != user_id);
                v.push((user_id, until));
            });
        }
        Event::PresenceUpdate(p) => s.presence.update(|m| {
            m.insert(p.user_id, p.status);
        }),
        Event::UserUpdate(u) => {
            if s.me_id() == u.id {
                s.me.set(Some(u.clone()));
            }
            s.users.update(|m| {
                m.insert(u.id, u);
            });
        }
        Event::SpaceCreate(mut sp) => {
            let users = std::mem::take(&mut sp.users);
            s.users.update(|m| m.extend(users.into_iter().map(|u| (u.id, u))));
            s.spaces.update(|v| match v.iter_mut().find(|x| x.id == sp.id) {
                Some(x) => *x = sp,
                None => v.push(sp),
            })
        }
        Event::SpaceUpdate { id, name, icon, owner_id } => s.spaces.update(|v| {
            if let Some(x) = v.iter_mut().find(|x| x.id == id) {
                x.name = name;
                x.icon = icon;
                x.owner_id = owner_id;
            }
        }),
        Event::SpaceDelete { id } => {
            s.spaces.update(|v| v.retain(|x| x.id != id));
            if s.nav.get_untracked().space == Some(id) {
                open_space(None);
            }
        }
        Event::ChannelCreate(c) | Event::ChannelUpdate(c) => match c.space_id {
            Some(sid) => s.spaces.update(|v| {
                if let Some(sp) = v.iter_mut().find(|x| x.id == sid) {
                    match sp.channels.iter_mut().find(|x| x.id == c.id) {
                        Some(x) => *x = c,
                        None => sp.channels.push(c),
                    }
                }
            }),
            None => upsert_dm(c),
        },
        Event::ChannelDelete { id, space_id } => {
            match space_id {
                Some(sid) => s.spaces.update(|v| {
                    if let Some(sp) = v.iter_mut().find(|x| x.id == sid) {
                        sp.channels.retain(|x| x.id != id);
                    }
                }),
                None => s.dms.update(|d| d.retain(|x| x.id != id)),
            }
            if s.thread.get_untracked() == Some(id) {
                s.thread.set(None);
            }
            if s.nav.get_untracked().channel == Some(id) {
                open_space(space_id);
            }
        }
        Event::MemberAdd { space_id, member, user } => {
            s.users.update(|m| {
                m.insert(user.id, user);
            });
            s.spaces.update(|v| {
                if let Some(sp) = v.iter_mut().find(|x| x.id == space_id) {
                    sp.members.retain(|m| m.user_id != member.user_id);
                    sp.members.push(member);
                }
            });
        }
        Event::MemberUpdate { space_id, member } => s.spaces.update(|v| {
            if let Some(m) = v.iter_mut().find(|x| x.id == space_id).and_then(|sp| sp.members.iter_mut().find(|m| m.user_id == member.user_id)) {
                *m = member;
            }
        }),
        Event::MemberRemove { space_id, user_id } => {
            if user_id == s.me_id() {
                s.spaces.update(|v| v.retain(|x| x.id != space_id));
                if s.nav.get_untracked().space == Some(space_id) {
                    open_space(None);
                }
            } else {
                s.spaces.update(|v| {
                    if let Some(sp) = v.iter_mut().find(|x| x.id == space_id) {
                        sp.members.retain(|m| m.user_id != user_id);
                    }
                });
            }
        }
        Event::ReadStateUpdate(r) => s.read.update(|m| {
            m.insert(r.channel_id, r);
        }),
        Event::VoiceStateUpdate(v) => {
            let joined = s.voice.with_untracked(|m| !m.contains_key(&v.user_id));
            if joined && s.call.with_untracked(|c| c.channel == Some(v.channel_id)) && v.user_id != s.me_id() {
                crate::sound::play(crate::sound::Sound::Join, s.settings.get_untracked().sounds);
            }
            s.voice.update(|m| {
                m.insert(v.user_id, v);
            });
        }
        Event::VoiceLeave { user_id, channel_id } => {
            s.voice.update(|m| {
                if m.get(&user_id).is_some_and(|v| v.channel_id == channel_id) {
                    m.remove(&user_id);
                }
            });
            if user_id == s.me_id() {
                if s.call.with_untracked(|c| c.channel == Some(channel_id) && !c.connecting) {
                    crate::voice::hang_up_local();
                }
            } else if s.call.with_untracked(|c| c.channel == Some(channel_id)) {
                crate::sound::play(crate::sound::Sound::Leave, s.settings.get_untracked().sounds);
            }
        }
        Event::CallRing { channel_id, caller_id } => {
            if s.call.with_untracked(|c| c.channel.is_none()) {
                s.ringing.set(Some((channel_id, caller_id)));
                crate::sound::ring(true);
                let name = s.name(caller_id, None);
                crate::desktop::notify(&format!("{name} is calling"), "Open Natter to answer.", Some(channel_id));
            }
        }
        Event::SettingsUpdate(v) => {
            if let Ok(st) = serde_json::from_value::<Settings>(v) {
                util::storage_set("natter.settings", &serde_json::to_string(&st).unwrap_or_default());
                s.settings.set(st);
            }
        }
        Event::InvalidSession => session_expired(),
    }
}

fn apply_ready(r: Ready) {
    let s = store();
    s.info.set(Some(r.server));
    s.nextcloud_session.set(r.nextcloud);
    s.my_status.set(r.status);
    s.users.set(r.users.into_iter().map(|u| (u.id, u)).collect());
    s.me.set(Some(r.user));
    s.presence.set(r.presences.into_iter().map(|p| (p.user_id, p.status)).collect());
    s.spaces.set(r.spaces);
    s.dms.set(r.dms);
    s.read.set(r.read_states.into_iter().map(|r| (r.channel_id, r)).collect());
    s.voice.set(r.voice_states.into_iter().map(|v| (v.user_id, v)).collect());
    if let Ok(st) = serde_json::from_value::<Settings>(r.settings.clone()) {
        if r.settings.as_object().is_some_and(|o| !o.is_empty()) {
            s.settings.set(st);
        }
    }
    // Reconnected: refresh what's on screen, forget the rest (it reloads on demand).
    let open = s.nav.get_untracked().channel;
    s.logs.update(|l| l.retain(|id, _| Some(*id) == open));
    if !s.ready.get_untracked() {
        s.ready.set(true);
        let last_space = util::storage_get("natter.space").and_then(|v| v.parse::<Id>().ok());
        let space = last_space.filter(|id| s.spaces.with_untracked(|v| v.iter().any(|x| x.id == *id)));
        open_space(space.or_else(|| s.spaces.with_untracked(|v| v.first().map(|x| x.id))));
    } else if let Some(c) = open {
        if s.channel(c).is_some() {
            fetch_latest(c);
        } else {
            open_space(None);
        }
    }
    // A call that survived the reconnect is still ours; one the server forgot is over.
    if let Some(c) = s.call.with_untracked(|c| c.channel) {
        let me = s.me_id();
        if !s.voice.with_untracked(|v| v.get(&me).is_some_and(|v| v.channel_id == c)) && !s.call.with_untracked(|c| c.connecting) {
            crate::voice::rejoin();
        }
    }
}

fn on_message(m: Message, from_gateway: bool) {
    let s = store();
    let me = s.me_id();
    let channel = m.channel_id;
    let mine = m.author_id == me;
    // Bump the channel so unread state and DM ordering update.
    let bump = |c: &mut Channel| {
        if c.last_message_id.is_none_or(|l| l < m.id) {
            c.last_message_id = Some(m.id);
            if let Some(t) = c.thread.as_mut() {
                t.message_count += 1;
                t.archived = false;
            }
        }
    };
    // The card under the thread's first message counts along.
    if let Some(parent) = s.channel(channel).filter(|c| c.kind == ChannelKind::Thread).and_then(|c| c.parent_id) {
        s.logs.update(|l| {
            if let Some(log) = l.get_mut(&parent) {
                for x in log.msgs.iter_mut() {
                    if let Some(p) = x.thread.as_mut().filter(|p| p.id == channel && p.last_message_id.is_none_or(|l| l < m.id)) {
                        p.message_count += 1;
                        p.last_message_id = Some(m.id);
                        p.archived = false;
                    }
                }
            }
        });
    }
    let in_space = s.channel(channel).and_then(|c| c.space_id);
    match in_space {
        Some(sid) => s.spaces.update(|v| {
            if let Some(c) = v.iter_mut().find(|x| x.id == sid).and_then(|sp| sp.channels.iter_mut().find(|c| c.id == channel)) {
                bump(c);
            }
        }),
        None => s.dms.update(|d| {
            if let Some(c) = d.iter_mut().find(|c| c.id == channel) {
                bump(c);
            }
        }),
    }
    s.typing.update(|t| {
        if let Some(v) = t.get_mut(&channel) {
            v.retain(|(u, _)| *u != m.author_id);
        }
    });
    let viewing = s.nav.get_untracked().channel == Some(channel);
    s.logs.update(|l| {
        if let Some(log) = l.get_mut(&channel) {
            if log.loaded && !log.more_after {
                merge(&mut log.msgs, vec![m.clone()]);
            }
        }
    });
    if mine {
        if let Some(n) = &m.nonce {
            s.failed.update(|f| {
                f.remove(n);
            });
        }
        s.read.update(|r| {
            let e = r.entry(channel).or_insert(ReadState { channel_id: channel, last_read_id: 0, mention_count: 0 });
            e.last_read_id = e.last_read_id.max(m.id);
        });
        return;
    }
    if !from_gateway {
        return;
    }
    // Notify: DMs and mentions, unless muted, DND, or you're looking right at it.
    let c = s.channel(channel);
    let is_dm = c.as_ref().is_some_and(|c| c.kind.is_dm());
    let mentioned = m.mentions.contains(&me) || m.mention_everyone;
    let muted = s.muted(channel) || in_space.is_some_and(|sp| s.muted(sp));
    let looking = viewing && s.focused.get_untracked();
    if (is_dm || mentioned) && !looking && !(muted && !mentioned) && s.my_status.get_untracked() != Status::Dnd {
        let st = s.settings.get_untracked();
        crate::sound::play(crate::sound::Sound::Message, st.sounds);
        if st.notifications {
            let author = s.name(m.author_id, in_space);
            let title = match (&c, in_space.and_then(|sp| s.space(sp))) {
                (Some(c), Some(sp)) => format!("{author} (#{}, {})", c.name, sp.name),
                _ => author,
            };
            let mut body = crate::markdown::plain(&m.content, &|id| s.name(id, in_space));
            if body.is_empty() && !m.attachments.is_empty() {
                body = format!("Sent {} file(s)", m.attachments.len());
            }
            crate::desktop::notify(&title, &body, Some(channel));
        }
    }
}

pub fn session_expired() {
    crate::voice::hang_up_local();
    crate::gateway::disconnect();
    util::storage_remove("natter.token");
    api::set_token(None);
    let s = store();
    s.me.set(None);
    s.ready.set(false);
    s.logs.set(HashMap::new());
}

pub fn sign_out() {
    spawn_local(async {
        let _ = api::act("POST", "/api/auth/logout").await;
        session_expired();
    });
}

/// Choose your status (online / idle / dnd / invisible).
pub fn set_status(status: Status) {
    let s = store();
    s.my_status.set(status);
    spawn_local(async move {
        let body = UpdateMe { status: Some(status), ..Default::default() };
        if let Err(e) = api::call::<_, User>("PATCH", "/api/me", &body).await {
            s.error(e);
        }
    });
}
