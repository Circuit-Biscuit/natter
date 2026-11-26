//! Link previews under messages: YouTube (thumbnail, then the player in place), page cards, pictures, videos.

use leptos::prelude::*;
use laklak_proto::{ChannelKind, Embed, EmbedKind, Id};

use super::Avatar;
use super::icons::Icon;
use crate::store::{Modal, open_channel, store};
use crate::{desktop, voice};

/// Links in previews open in the browser, never inside Laklak.
fn open(url: String) -> impl Fn(leptos::ev::MouseEvent) + Clone {
    move |e: leptos::ev::MouseEvent| {
        e.prevent_default();
        desktop::open_url(&url);
    }
}

#[component]
pub fn EmbedView(e: Embed) -> impl IntoView {
    let s = store();
    match e.kind {
        EmbedKind::Image => {
            let src = e.image.clone().unwrap_or_else(|| e.url.clone());
            let full = src.clone();
            view! {
                <img class="embed-image" src=src alt="" loading="lazy" referrerpolicy="no-referrer"
                    on:click=move |_| s.modal.set(Some(Modal::Image(full.clone())))/>
            }
            .into_any()
        }
        EmbedKind::Video => {
            let src = e.video.clone().unwrap_or_else(|| e.url.clone());
            view! { <video class="embed-video" src=src controls=true preload="metadata"></video> }.into_any()
        }
        EmbedKind::Youtube => {
            let playing = RwSignal::new(false);
            let player = e.video.clone().unwrap_or_default();
            let thumb = e.image.clone().unwrap_or_default();
            view! {
                <div class="embed youtube" style:border-left-color=e.color.clone().unwrap_or_default()>
                    <div class="embed-site">{e.site.clone()}{(!e.author.is_empty()).then(|| format!(" · {}", e.author))}</div>
                    {(!e.title.is_empty()).then(|| view! {
                        <a class="embed-title" href=e.url.clone() on:click=open(e.url.clone())>{e.title.clone()}</a>
                    })}
                    <div class="embed-player">
                        {move || if playing.get() {
                            view! {
                                <iframe src=player.clone() title="YouTube video" referrerpolicy="strict-origin-when-cross-origin"
                                    allow="autoplay; encrypted-media; picture-in-picture; fullscreen" allowfullscreen=true></iframe>
                            }.into_any()
                        } else {
                            view! {
                                <button class="embed-thumb" title="Play" on:click=move |_| playing.set(true)>
                                    <img src=thumb.clone() alt="" loading="lazy" referrerpolicy="no-referrer"/>
                                    <span class="embed-play" aria-hidden="true"></span>
                                </button>
                            }.into_any()
                        }}
                    </div>
                </div>
            }
            .into_any()
        }
        EmbedKind::Link => {
            let image = e.image.clone();
            view! {
                <div class="embed" style:border-left-color=e.color.clone().unwrap_or_default()>
                    {(!e.site.is_empty()).then(|| view! { <div class="embed-site">{e.site.clone()}</div> })}
                    {(!e.title.is_empty()).then(|| view! {
                        <a class="embed-title" href=e.url.clone() on:click=open(e.url.clone())>{e.title.clone()}</a>
                    })}
                    {(!e.description.is_empty()).then(|| view! { <div class="embed-text">{e.description.clone()}</div> })}
                    {image.map(|src| {
                        let full = src.clone();
                        view! {
                            <img class="embed-picture" src=src alt="" loading="lazy" referrerpolicy="no-referrer"
                                on:click=move |_| s.modal.set(Some(Modal::Image(full.clone())))/>
                        }
                    })}
                </div>
            }
            .into_any()
        }
    }
}

/// Card under a message that links to one of our channels ("Copy Channel Link"), like Discord's invite cards:
/// Join Voice for a voice channel, Go to Channel otherwise.
#[component]
pub fn ChannelCard(channel: Id) -> impl IntoView {
    let s = store();
    move || {
        let Some(c) = s.channel(channel) else {
            return view! {
                <div class="embed channel-card">
                    <div class="cc-label">"Channel link"</div>
                    <div class="cc-row">
                        <span class="cc-icon"><Icon name="lock" size=22/></span>
                        <div class="cc-text"><b>"No access"</b><span>"You can't see this channel, or it was deleted."</span></div>
                    </div>
                </div>
            }
            .into_any();
        };
        let (label, icon) = match (c.kind, c.read_only) {
            (ChannelKind::Voice, _) => ("Voice channel", "speaker"),
            (ChannelKind::Thread, _) => ("Thread", "thread"),
            (_, true) => ("Announcement channel", "megaphone"),
            _ => ("Text channel", "hash"),
        };
        let voice = c.kind == ChannelKind::Voice;
        let in_call = move || s.call.with(|call| call.channel == Some(channel));
        let space = c.space_id.and_then(|sp| s.space(sp)).map(|sp| sp.name).unwrap_or_default();
        view! {
            <div class="embed channel-card">
                <div class="cc-label">{label}</div>
                <div class="cc-row">
                    <span class="cc-icon"><Icon name=icon size=22/></span>
                    <div class="cc-text">
                        <b class="ellipsis">{c.name.clone()}</b>
                        <span class="ellipsis">{space}</span>
                    </div>
                    {voice.then(|| view! {
                        <div class="cc-people">
                            {move || s.voice_in(channel).into_iter().take(5).map(|v| view! { <Avatar user=v.user_id size=24/> }).collect_view()}
                        </div>
                    })}
                    <button class="btn primary small" class:on=move || voice && in_call() on:click=move |_| {
                        if voice && !in_call() {
                            voice::join(channel);
                        }
                        open_channel(channel);
                    }>{move || match c.kind {
                        ChannelKind::Voice if in_call() => "Joined",
                        ChannelKind::Voice => "Join Voice",
                        ChannelKind::Thread => "Open Thread",
                        _ => "Go to Channel",
                    }}</button>
                </div>
            </div>
        }
        .into_any()
    }
}
