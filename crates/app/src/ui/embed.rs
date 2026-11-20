//! Link previews under messages: YouTube (thumbnail, then the player in place), page cards, pictures, videos.

use leptos::prelude::*;
use natter_proto::{Embed, EmbedKind};

use crate::desktop;
use crate::store::{Modal, store};

/// Links in previews open in the browser, never inside Natter.
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
