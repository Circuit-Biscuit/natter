//! The message box: autocomplete for @people, #channels and :emoji:, replies, uploads (drop, paste,
//! picker, Nextcloud), drafts per channel, and Discord's keyboard habits.

use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use natter_proto::*;

use super::emoji;
use super::icons::Icon;
use crate::store::{MenuItem, Modal, store};
use crate::{api, gateway, util};

#[derive(Clone, PartialEq)]
pub enum UpState {
    Uploading,
    Done(Attachment),
    Failed(String),
}

#[derive(Clone, PartialEq)]
pub struct Upload {
    pub key: u32,
    pub name: String,
    pub size: u64,
    pub preview: Option<String>,
    pub state: UpState,
}

#[derive(Clone, Copy)]
pub struct Uploads(pub RwSignal<Vec<Upload>>);

impl Uploads {
    pub fn new() -> Self {
        Uploads(RwSignal::new(vec![]))
    }
}

pub fn add_files(up: Uploads, channel: Id, files: web_sys::FileList) {
    let s = store();
    let max = s.info.with_untracked(|i| i.as_ref().map(|i| i.max_upload_bytes).unwrap_or(u64::MAX));
    for i in 0..files.length() {
        let Some(f) = files.get(i) else { continue };
        let size = f.size() as u64;
        if size > max {
            s.toast(format!("{} is too large (limit {}).", f.name(), util::file_size(max)), true);
            continue;
        }
        if up.0.with_untracked(|v| v.len()) >= MAX_ATTACHMENTS {
            s.toast(format!("You can attach up to {MAX_ATTACHMENTS} files per message."), true);
            break;
        }
        let key = (js_sys::Math::random() * 1e9) as u32;
        let preview = f.type_().starts_with("image/").then(|| web_sys::Url::create_object_url_with_blob(&f).ok()).flatten();
        let name = f.name();
        up.0.update(|v| v.push(Upload { key, name: name.clone(), size, preview, state: UpState::Uploading }));
        spawn_local(async move {
            let r = api::upload(&f, &name, Some(channel)).await;
            up.0.update(|v| {
                if let Some(u) = v.iter_mut().find(|u| u.key == key) {
                    u.state = match r {
                        Ok(a) => UpState::Done(a),
                        Err(e) => UpState::Failed(e.message),
                    };
                }
            });
        });
    }
}

/// Attachments picked outside the composer (the Nextcloud browser) wait here for the open composer.
pub fn external() -> RwSignal<Vec<Attachment>> {
    thread_local! { static E: RwSignal<Vec<Attachment>> = RwSignal::new(vec![]); }
    E.with(|e| *e)
}

/// Grow the textarea with its content (up to half the window).
pub fn autosize(t: &web_sys::HtmlTextAreaElement) {
    let el: &web_sys::HtmlElement = t;
    let style = el.style();
    // Hidden (a phone showing another screen): nothing to measure yet.
    if el.offset_parent().is_none() {
        let _ = style.remove_property("height");
        return;
    }
    let _ = style.set_property("height", "auto");
    let max = util::window().inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(800.0) * 0.5;
    let h = (t.scroll_height() as f64).min(max);
    let _ = style.set_property("height", &format!("{h}px"));
}

#[derive(Clone, PartialEq)]
struct Suggestion {
    /// Inserted text.
    insert: String,
    label: String,
    detail: String,
    user: Option<Id>,
}

/// The `@foo`, `#foo` or `:foo` being typed at the caret: (start byte, token).
fn token_at(text: &str, caret: usize) -> Option<(usize, String)> {
    let before = &text[..caret.min(text.len())];
    let start = before.rfind(|c: char| c.is_whitespace()).map(|i| i + 1).unwrap_or(0);
    let tok = &before[start..];
    let first = tok.chars().next()?;
    match first {
        '@' | '#' => Some((start, tok.to_string())),
        ':' if tok.chars().count() >= 3 && !tok[1..].contains(':') => Some((start, tok.to_string())),
        _ => None,
    }
}

fn suggestions(channel: Id, tok: &str) -> Vec<Suggestion> {
    let s = store();
    let Some(c) = s.channel(channel) else { return vec![] };
    let q = tok[1..].to_lowercase();
    let mut out = Vec::new();
    match tok.chars().next() {
        Some('@') => {
            let ids: Vec<Id> = match c.space_id {
                Some(sp) => s.space(sp).map(|sp| sp.members.iter().map(|m| m.user_id).collect()).unwrap_or_default(),
                None => c.recipients.clone(),
            };
            for id in ids {
                let Some(u) = s.user(id) else { continue };
                let name = s.name(id, c.space_id);
                if u.username.contains(&q) || name.to_lowercase().contains(&q) {
                    out.push(Suggestion { insert: format!("@{} ", u.username), label: name, detail: u.username.clone(), user: Some(id) });
                }
            }
            out.sort_by_key(|x| (!x.detail.starts_with(&q), s.status(x.user.unwrap_or(0)) == Status::Offline, x.label.to_lowercase()));
            out.truncate(8);
            if s.can(c.space_id, perm::MENTION_EVERYONE) {
                for (w, d) in [("everyone", "Notify everyone in this channel"), ("here", "Notify everyone online")] {
                    if w.starts_with(&q) {
                        out.push(Suggestion { insert: format!("@{w} "), label: format!("@{w}"), detail: d.into(), user: None });
                    }
                }
            }
        }
        Some('#') => {
            if let Some(sp) = c.space_id.and_then(|sp| s.space(sp)) {
                for ch in sp.channels.iter().filter(|x| x.kind.has_messages() && x.name.contains(&q)) {
                    out.push(Suggestion { insert: format!("#{} ", ch.name), label: format!("#{}", ch.name), detail: String::new(), user: None });
                }
                out.truncate(8);
            }
        }
        Some(':') => {
            for e in emojis::iter() {
                if let Some(code) = e.shortcodes().find(|c| c.contains(&q)) {
                    out.push(Suggestion { insert: format!("{} ", e.as_str()), label: format!("{}  :{}:", e.as_str(), code), detail: String::new(), user: None });
                    if out.len() >= 10 {
                        break;
                    }
                }
            }
            out.sort_by_key(|x| !x.label.contains(&format!(":{q}")));
        }
        _ => {}
    }
    out
}

/// Turn typed `@username` / `#channel` into mention tokens the server understands.
pub fn encode_mentions(channel: Id, text: &str) -> String {
    let s = store();
    let Some(c) = s.channel(channel) else { return text.to_string() };
    let users: Vec<(String, Id)> = match c.space_id {
        Some(sp) => s.space(sp).map(|sp| sp.members.iter().filter_map(|m| s.user(m.user_id).map(|u| (u.username, u.id))).collect()).unwrap_or_default(),
        None => c.recipients.iter().filter_map(|&r| s.user(r).map(|u| (u.username, u.id))).collect(),
    };
    let chans: Vec<(String, Id)> = c.space_id.and_then(|sp| s.space(sp)).map(|sp| sp.channels.iter().filter(|x| x.kind.has_messages()).map(|x| (x.name.clone(), x.id)).collect()).unwrap_or_default();
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    for (i, part) in text.split('`').enumerate() {
        if i > 0 {
            out.push('`');
            in_code = !in_code;
        }
        if in_code {
            out.push_str(part);
            continue;
        }
        let mut rest = part;
        while let Some(pos) = rest.find(['@', '#']) {
            out.push_str(&rest[..pos]);
            let sigil = rest.as_bytes()[pos];
            let after = &rest[pos + 1..];
            let word_end = after.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '-')).unwrap_or(after.len());
            let word = after[..word_end].trim_end_matches('.');
            let prev_ok = pos == 0 || rest[..pos].ends_with(|c: char| c.is_whitespace() || c == '(');
            let hit = if !prev_ok || word.is_empty() {
                None
            } else if sigil == b'@' {
                users.iter().find(|(n, _)| n == &word.to_lowercase()).map(|(_, id)| format!("<@{id}>"))
            } else {
                chans.iter().find(|(n, _)| n == word).map(|(_, id)| format!("<#{id}>"))
            };
            match hit {
                Some(tok) => {
                    out.push_str(&tok);
                    rest = &after[word.len()..];
                }
                None => {
                    out.push(sigil as char);
                    rest = after;
                }
            }
        }
        out.push_str(rest);
    }
    out
}

/// Show stored mention tokens as `@username` again when editing.
pub fn decode_mentions(text: &str) -> String {
    let s = store();
    let mut out = String::new();
    let mut rest = text;
    while let Some(p) = rest.find('<') {
        out.push_str(&rest[..p]);
        let r = &rest[p..];
        let parsed = r.find('>').and_then(|end| {
            let inner = &r[2..end];
            let id: Id = inner.parse().ok()?;
            let text = if r.starts_with("<@") {
                format!("@{}", s.user(id)?.username)
            } else if r.starts_with("<#") {
                format!("#{}", s.channel(id)?.name)
            } else {
                return None;
            };
            Some((text, end + 1))
        });
        match parsed {
            Some((t, len)) => {
                out.push_str(&t);
                rest = &r[len..];
            }
            None => {
                out.push('<');
                rest = &r[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[component]
pub fn Composer(channel: Id, uploads: Uploads) -> impl IntoView {
    let s = store();
    let text = RwSignal::new(s.drafts.with_untracked(|d| d.get(&channel).cloned().unwrap_or_default()));
    let el = NodeRef::<html::Textarea>::new();
    let file_input = NodeRef::<html::Input>::new();
    let sugg = RwSignal::new(Vec::<Suggestion>::new());
    let sel = RwSignal::new(0usize);
    let tok_start = StoredValue::new(0usize);
    let ping_reply = RwSignal::new(true);

    let can_post = move || {
        let Some(c) = s.channel(channel) else { return false };
        s.can(c.space_id, perm::SEND_MESSAGES) && (!c.read_only || s.can(c.space_id, perm::MANAGE_MESSAGES))
    };
    let placeholder = move || match s.channel(channel) {
        Some(c) if c.kind.is_dm() => format!("Message @{}", s.channel_title(&c)),
        Some(c) => format!("Message #{}", c.name),
        None => String::new(),
    };

    Effect::new(move |_| {
        s.pane.track(); // re-measure when a phone switches to this screen
        if let Some(t) = el.get() {
            autosize(&t);
            if s.modal.with_untracked(|m| m.is_none()) {
                let _ = t.focus();
            }
        }
    });
    // Focus the box when replying.
    Effect::new(move |_| {
        if s.reply_to.get().is_some_and(|r| s.message(channel, r).is_some()) {
            util::focus(&format!("composer-{channel}"));
        }
    });

    let set_text = move |v: String| {
        text.set(v.clone());
        s.drafts.update(|d| {
            d.insert(channel, v);
        });
        if let Some(t) = el.get_untracked() {
            t.set_value(&text.get_untracked());
            autosize(&t);
        }
    };

    let refresh_suggestions = move || {
        let Some(t) = el.get_untracked() else { return };
        let v = t.value();
        let caret = t.selection_start().ok().flatten().unwrap_or(v.len() as u32) as usize;
        // selectionStart counts UTF-16 units; map to a byte index.
        let byte = v.char_indices().scan(0usize, |u16s, (b, c)| {
            let here = *u16s;
            *u16s += c.len_utf16();
            Some((here, b))
        }).find(|(u, _)| *u >= caret).map(|(_, b)| b).unwrap_or(v.len());
        match token_at(&v, byte) {
            Some((start, tok)) => {
                tok_start.set_value(start);
                sugg.set(suggestions(channel, &tok));
                sel.set(0);
            }
            None => sugg.set(vec![]),
        }
    };

    let accept = move |i: usize| {
        let Some(x) = sugg.get_untracked().get(i).cloned() else { return };
        let Some(t) = el.get_untracked() else { return };
        let v = t.value();
        let start = tok_start.get_value();
        let end = v[start..].find(char::is_whitespace).map(|e| start + e).unwrap_or(v.len());
        let new = format!("{}{}{}", &v[..start], x.insert, v[end..].trim_start());
        let caret = (start + x.insert.len()) as u32;
        set_text(new);
        let caret16 = text.get_untracked()[..caret as usize].encode_utf16().count() as u32;
        let _ = t.set_selection_range(caret16, caret16);
        sugg.set(vec![]);
    };

    let send = move || {
        let mut content = text.get_untracked();
        if uploads.0.with_untracked(|v| v.iter().any(|u| u.state == UpState::Uploading)) {
            s.toast("Still uploading — one moment.", false);
            return;
        }
        let atts: Vec<Attachment> = uploads.0.with_untracked(|v| v.iter().filter_map(|u| match &u.state { UpState::Done(a) => Some(a.clone()), _ => None }).collect());
        if content.trim().is_empty() && atts.is_empty() {
            return;
        }
        if content.chars().count() > MAX_MESSAGE_LEN {
            s.toast(format!("Messages can be up to {MAX_MESSAGE_LEN} characters."), true);
            return;
        }
        let mut silent = false;
        if let Some(rest) = content.strip_prefix("@silent ") {
            silent = true;
            content = rest.to_string();
        }
        let content = encode_mentions(channel, &content);
        crate::store::send_message(channel, content, atts, silent, ping_reply.get_untracked());
        ping_reply.set(true);
        gateway::typing_sent();
        uploads.0.update(|v| {
            for u in v.iter() {
                if let Some(p) = &u.preview {
                    let _ = web_sys::Url::revoke_object_url(p);
                }
            }
            v.clear();
        });
        set_text(String::new());
        sugg.set(vec![]);
    };

    let on_key = move |e: web_sys::KeyboardEvent| {
        let open = !sugg.with_untracked(|v| v.is_empty());
        match e.key().as_str() {
            "ArrowDown" if open => {
                e.prevent_default();
                sel.update(|i| *i = (*i + 1) % sugg.with_untracked(|v| v.len()));
            }
            "ArrowUp" if open => {
                e.prevent_default();
                let n = sugg.with_untracked(|v| v.len());
                sel.update(|i| *i = (*i + n - 1) % n);
            }
            "Tab" if open => {
                e.prevent_default();
                accept(sel.get_untracked());
            }
            "Enter" if open && !e.shift_key() => {
                e.prevent_default();
                accept(sel.get_untracked());
            }
            // On phones Enter is a new line and the send button sends (as in Discord's app).
            "Enter" if !e.shift_key() && !s.mobile.get_untracked() => {
                e.prevent_default();
                send();
            }
            "Escape" => {
                if open {
                    sugg.set(vec![]);
                } else if s.reply_to.get_untracked().is_some() {
                    s.reply_to.set(None);
                } else {
                    crate::store::ack(channel, None);
                }
            }
            "ArrowUp" if text.with_untracked(|t| t.is_empty()) => {
                // Edit your last message.
                let me = s.me_id();
                let last = s.logs.with_untracked(|l| {
                    l.get(&channel).and_then(|l| l.msgs.iter().rev().find(|m| m.author_id == me && m.id < crate::store::PENDING_BASE && m.kind == MessageKind::Default).map(|m| m.id))
                });
                if let Some(id) = last {
                    e.prevent_default();
                    s.editing.set(Some(id));
                }
            }
            _ => {}
        }
    };

    let on_paste = move |e: web_sys::ClipboardEvent| {
        if let Some(files) = e.clipboard_data().and_then(|d| d.files()) {
            if files.length() > 0 {
                e.prevent_default();
                add_files(uploads, channel, files);
            }
        }
    };

    let plus_menu = move |e: leptos::ev::MouseEvent| {
        let mut items = vec![Some(MenuItem::new("Upload a file", move || {
            if let Some(i) = file_input.get_untracked() {
                i.click();
            }
        }))];
        if s.nextcloud_session.get_untracked() {
            items.push(Some(MenuItem::new("Share from Nextcloud…", move || s.modal.set(Some(Modal::Nextcloud)))));
        }
        s.open_menu(&e, items);
    };

    // Files picked in the Nextcloud browser arrive here.
    Effect::new(move |_| {
        let picked = external().get();
        if picked.is_empty() {
            return;
        }
        external().set(vec![]);
        uploads.0.update(|v| {
            for a in picked {
                let key = (js_sys::Math::random() * 1e9) as u32;
                let preview = a.is_image().then(|| api::url(&a.url));
                v.push(Upload { key, name: a.filename.clone(), size: a.size, preview, state: UpState::Done(a) });
            }
        });
    });

    let numbered = move || -> Vec<(usize, Suggestion)> { sugg.get().into_iter().enumerate().collect() };
    let typing_line = move || {
        let who = s.typing_in(channel);
        let space = s.channel(channel).and_then(|c| c.space_id);
        let names: Vec<String> = who.iter().map(|&u| s.name(u, space)).collect();
        match names.len() {
            0 => String::new(),
            1 => format!("{} is typing…", names[0]),
            2 => format!("{} and {} are typing…", names[0], names[1]),
            3 => format!("{}, {} and {} are typing…", names[0], names[1], names[2]),
            _ => "Several people are typing…".into(),
        }
    };

    view! {
        <div class="composer-wrap">
            <Show when=move || !sugg.with(|v| v.is_empty())>
                <div class="autocomplete">
                    <For each=numbered key=|(i, x)| (*i, x.insert.clone()) let:item>
                        {
                            let (i, x) = item;
                            view! {
                                <div class="ac-item" class:selected=move || sel.get() == i
                                    on:mouseenter=move |_| sel.set(i)
                                    on:mousedown=move |e| { e.prevent_default(); accept(i); }>
                                    {x.user.map(|u| view! { <super::Avatar user=u size=24 status=true/> })}
                                    <span class="ac-label">{x.label.clone()}</span>
                                    <span class="ac-detail">{x.detail.clone()}</span>
                                </div>
                            }
                        }
                    </For>
                </div>
            </Show>
            {move || s.reply_to.get().and_then(|r| s.message(channel, r)).map(|m| {
                let space = s.channel(channel).and_then(|c| c.space_id);
                let author = m.author_id;
                view! {
                    <div class="reply-to">
                        <span>"Replying to " <b>{s.name(author, space)}</b></span>
                        <span class="spacer"></span>
                        <button class="ping-toggle" class:on=move || ping_reply.get() title="Mention the author"
                            on:click=move |_| ping_reply.update(|v| *v = !*v)>
                            {move || if ping_reply.get() { "@ ON" } else { "@ OFF" }}
                        </button>
                        <button class="icon-btn" title="Cancel reply (Esc)" on:click=move |_| s.reply_to.set(None)><Icon name="x" size=16/></button>
                    </div>
                }
            })}
            <Show when=move || !uploads.0.with(|v| v.is_empty())>
                <div class="upload-strip">
                    <For each=move || uploads.0.get() key=|u| (u.key, matches!(u.state, UpState::Uploading), matches!(u.state, UpState::Failed(_))) let:u>
                        {
                            let key = u.key;
                            view! {
                                <div class="upload" class:failed=matches!(u.state, UpState::Failed(_))>
                                    {match u.preview.clone() {
                                        Some(p) => view! { <img src=p alt=""/> }.into_any(),
                                        None => view! { <div class="upload-icon"><Icon name="file" size=40/></div> }.into_any(),
                                    }}
                                    <div class="upload-name ellipsis" title=u.name.clone()>{u.name.clone()}</div>
                                    <div class="upload-state">{match &u.state {
                                        UpState::Uploading => "Uploading…".to_string(),
                                        UpState::Done(_) => util::file_size(u.size),
                                        UpState::Failed(e) => e.clone(),
                                    }}</div>
                                    <button class="icon-btn remove" title="Remove" on:click=move |_| uploads.0.update(|v| v.retain(|x| x.key != key))><Icon name="trash" size=16/></button>
                                </div>
                            }
                        }
                    </For>
                </div>
            </Show>
            {move || if can_post() {
                view! {
                    <div class="composer" class:has-reply=move || s.reply_to.get().is_some()>
                        <button class="icon-btn plus" title="Upload a file" on:click=plus_menu><Icon name="plus" size=20/></button>
                        <textarea id=format!("composer-{channel}") node_ref=el rows="1" placeholder=placeholder
                            prop:value=move || text.get()
                            on:input=move |e| {
                                let v = event_target_value(&e);
                                if !v.is_empty() && !v.starts_with('/') { gateway::typing(channel); }
                                text.set(v.clone());
                                s.drafts.update(|d| { d.insert(channel, v); });
                                if let Some(t) = el.get_untracked() { autosize(&t); }
                                refresh_suggestions();
                            }
                            on:click=move |_| refresh_suggestions()
                            on:keydown=on_key
                            on:paste=on_paste
                            on:blur=move |_| set_timeout(move || sugg.set(vec![]), std::time::Duration::from_millis(150))></textarea>
                        <Show when=move || { text.with(|t| t.chars().count()) + 200 > MAX_MESSAGE_LEN }>
                            <span class="char-count" class:over=move || { text.with(|t| t.chars().count()) > MAX_MESSAGE_LEN }>
                                {move || MAX_MESSAGE_LEN as i64 - text.with(|t| t.chars().count()) as i64}
                            </span>
                        </Show>
                        <button class="icon-btn" title="Emoji" on:click=move |e| {
                            emoji::open_picker(&e, move |em: String| {
                                let Some(t) = el.get_untracked() else { return };
                                let v = t.value();
                                let at = t.selection_start().ok().flatten().map(|c| {
                                    let mut u = 0u32;
                                    v.char_indices().find(|(_, ch)| { let here = u; u += ch.len_utf16() as u32; here >= c }).map(|(b, _)| b).unwrap_or(v.len())
                                }).unwrap_or(v.len());
                                set_text(format!("{}{}{}", &v[..at], em, &v[at..]));
                                let _ = t.focus();
                            });
                        }><Icon name="smile" size=22/></button>
                        <Show when=move || s.mobile.get()>
                            <button class="icon-btn send" title="Send" on:mousedown=|e| e.prevent_default() on:click=move |_| send()><Icon name="send" size=20/></button>
                        </Show>
                        <input type="file" multiple=true class="hidden" node_ref=file_input on:change=move |e| {
                            let input: web_sys::HtmlInputElement = event_target(&e);
                            if let Some(files) = input.files() { add_files(uploads, channel, files); }
                            input.set_value("");
                        }/>
                    </div>
                }.into_any()
            } else {
                view! { <div class="composer locked"><Icon name="lock" size=18/>{move || if s.channel(channel).is_some_and(|c| c.read_only) { "Only members with Manage messages can post in this channel." } else { "You don't have permission to send messages here." }}</div> }.into_any()
            }}
            <div class="typing">
                <Show when=move || !typing_line().is_empty()>
                    <span class="dots"><i></i><i></i><i></i></span>
                </Show>
                <span>{typing_line}</span>
            </div>
        </div>
    }
}
