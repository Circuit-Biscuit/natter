//! GIF picker, like Discord's: category tiles, Trending, Favorites (starred, synced in settings) and search.
//! Picking a GIF sends its link as a message of its own, which every app shows as just the GIF.
//!
//! The GIFs come from KLIPY (Google closed Tenor's API in June 2026). KLIPY's terms shape this: the app calls KLIPY
//! itself (the server only hands out the app key, `LAKLAK_KLIPY_KEY`), results stay in KLIPY's order, the search
//! box says "Search KLIPY", and each send is reported to KLIPY's share endpoint. No user id is sent to KLIPY.

use std::cell::RefCell;
use std::sync::Arc;

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Deserialize;

use super::icons::Icon;
use crate::store::{Gif, store};
use crate::util;

const API: &str = "https://api.klipy.com/api/v1";
// ponytail: KLIPY's middle setting; their Partner Panel blocks words or single GIFs if that's ever needed.
const QUERY: &str = "per_page=24&content_filter=medium&format_filter=webp,gif";
/// Settings are capped at 64 KB on the server; 100 favourites take about 26.
const MAX_FAVS: usize = 100;
const W: i32 = 424;
const H: i32 = 480;

#[derive(Clone)]
struct Picker {
    x: i32,
    y: i32,
    on_pick: Arc<dyn Fn(Gif) + Send + Sync>,
}

fn picker() -> RwSignal<Option<Picker>> {
    thread_local! { static P: RwSignal<Option<Picker>> = RwSignal::new(None); }
    P.with(|p| *p)
}

/// Create the picker's signal under the app root.
pub fn init() {
    picker();
}

/// The server's KLIPY key; without one there's no GIF button.
pub fn key() -> Option<String> {
    store().info.with(|i| i.as_ref().and_then(|i| i.klipy_key.clone()))
}

/// Open with the picker's bottom-right corner near the click, kept on screen.
pub fn open_picker(e: &web_sys::MouseEvent, on_pick: impl Fn(Gif) + Send + Sync + 'static) {
    e.stop_propagation();
    let win = util::window();
    let vw = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1200.0) as i32;
    let vh = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(800.0) as i32;
    let x = (e.client_x() - W).clamp(8, (vw - W - 8).max(8));
    let y = (e.client_y() - H - 8).clamp(8, (vh - H - 8).max(8));
    store().menu.set(None);
    picker().set(Some(Picker { x, y, on_pick: Arc::new(on_pick) }));
}

// ---------------------------------------------------------------- KLIPY's JSON

#[derive(Deserialize)]
struct Resp<T> {
    data: T,
}

#[derive(Deserialize)]
struct Page {
    data: Vec<Item>,
    #[serde(default)]
    has_next: bool,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default)]
    slug: String,
    /// Missing on ads (which only come when ads are switched on in KLIPY's panel; we don't).
    file: Option<Sizes>,
}

#[derive(Deserialize)]
struct Sizes {
    hd: Option<Formats>,
    md: Option<Formats>,
    sm: Option<Formats>,
    xs: Option<Formats>,
}

#[derive(Deserialize)]
struct Formats {
    webp: Option<Media>,
    gif: Option<Media>,
}

#[derive(Clone, Deserialize)]
struct Media {
    url: String,
    width: f64,
    height: f64,
}

impl Item {
    fn gif(self) -> Option<Gif> {
        let f = self.file?;
        let pick = |s: &Option<Formats>| s.as_ref().and_then(|f| f.webp.clone().or_else(|| f.gif.clone()));
        let full = pick(&f.hd).or_else(|| pick(&f.md))?;
        let small = pick(&f.sm).or_else(|| pick(&f.xs)).unwrap_or_else(|| full.clone());
        Some(Gif { url: full.url, preview: small.url, width: small.width.max(1.0) as u32, height: small.height.max(1.0) as u32, slug: self.slug })
    }
}

#[derive(Deserialize)]
struct Cats {
    categories: Vec<Cat>,
}

#[derive(Clone, PartialEq, Deserialize)]
struct Cat {
    category: String,
    query: String,
    preview_url: String,
}

thread_local! {
    /// Categories change rarely: fetched once per run.
    static CATS: RefCell<Vec<Cat>> = const { RefCell::new(vec![]) };
}

fn enc(s: &str) -> String {
    js_sys::encode_uri_component(s).into()
}

/// Tell KLIPY a GIF was sent (their terms; it also tunes their ranking). `q` = the search it came from.
fn report_share(slug: String, q: String) {
    let Some(key) = key() else { return };
    if slug.is_empty() {
        return;
    }
    spawn_local(async move {
        let init = web_sys::RequestInit::new();
        init.set_method("POST");
        if let Ok(h) = web_sys::Headers::new() {
            let _ = h.set("Content-Type", "application/json");
            init.set_headers(&h);
        }
        init.set_body(&serde_json::json!({ "q": q }).to_string().into());
        let url = format!("{API}/{key}/gifs/share/{}", enc(&slug));
        let _ = wasm_bindgen_futures::JsFuture::from(util::window().fetch_with_str_and_init(&url, &init)).await;
    });
}

// ---------------------------------------------------------------- the picker

#[derive(Clone, Copy, PartialEq)]
enum View {
    Home,
    Trending,
    Favorites,
    Search,
}

#[component]
pub fn GifPickerHost() -> impl IntoView {
    view! {
        {move || picker().get().map(|p| view! { <GifPicker p=p/> })}
    }
}

#[component]
fn GifPicker(p: Picker) -> impl IntoView {
    let s = store();
    let query = RwSignal::new(String::new());
    let view = RwSignal::new(View::Home);
    let results = RwSignal::new(Vec::<Gif>::new());
    let loading = RwSignal::new(false);
    let failed = RwSignal::new(false);
    let more = StoredValue::new(false);
    let page = StoredValue::new(1u32);
    // Bumped on every new view or search, so late answers to old ones are dropped.
    let generation = StoredValue::new(0u32);
    let cats = RwSignal::new(CATS.with(|c| c.borrow().clone()));
    let on_pick = StoredValue::new(p.on_pick);

    if cats.with_untracked(|c| c.is_empty())
        && let Some(key) = key()
    {
        spawn_local(async move {
            if let Some(r) = util::fetch_json::<Resp<Cats>>(&format!("{API}/{key}/gifs/categories")).await {
                CATS.with(|c| *c.borrow_mut() = r.data.categories.clone());
                cats.set(r.data.categories);
            }
        });
    }

    let load = move |g: u32| {
        let Some(key) = key() else { return };
        let n = page.get_value();
        let url = match view.get_untracked() {
            View::Trending => format!("{API}/{key}/gifs/trending?page={n}&{QUERY}"),
            View::Search => format!("{API}/{key}/gifs/search?page={n}&q={}&{QUERY}", enc(query.get_untracked().trim())),
            _ => return,
        };
        loading.set(true);
        spawn_local(async move {
            let r = util::fetch_json::<Resp<Page>>(&url).await;
            // (`try_`: the picker may have been closed meanwhile.)
            if generation.try_get_value() != Some(g) {
                return;
            }
            loading.set(false);
            match r {
                Some(r) => {
                    more.set_value(r.data.has_next);
                    results.update(|v| {
                        for g in r.data.data.into_iter().filter_map(Item::gif) {
                            // A GIF can come again on a later page.
                            if !v.iter().any(|x| x.url == g.url) {
                                v.push(g);
                            }
                        }
                    });
                }
                None => {
                    more.set_value(false);
                    failed.set(true);
                }
            }
        });
    };

    // A new view or search starts over (typing waits a moment first).
    Effect::new(move |_| {
        let q = query.get();
        let v = view.get();
        let g = generation.get_value() + 1;
        generation.set_value(g);
        page.set_value(1);
        more.set_value(false);
        results.set(vec![]);
        loading.set(false);
        failed.set(false);
        match v {
            View::Trending => load(g),
            View::Search if !q.trim().is_empty() => {
                loading.set(true);
                set_timeout(move || if generation.try_get_value() == Some(g) { load(g) }, std::time::Duration::from_millis(300));
            }
            _ => {}
        }
    });

    let on_scroll = move |e: web_sys::Event| {
        let el: web_sys::Element = event_target(&e);
        if el.scroll_height() - el.scroll_top() - el.client_height() < 400 && more.get_value() && !loading.get_untracked() {
            page.set_value(page.get_value() + 1);
            load(generation.get_value());
        }
    };

    let pick = move |g: Gif| {
        let q = if view.get_untracked() == View::Search { query.get_untracked() } else { String::new() };
        report_share(g.slug.clone(), q);
        on_pick.with_value(|f| f(g));
        picker().set(None);
    };
    let is_fav = move |url: &str| s.settings.with(|st| st.fav_gifs.iter().any(|f| f.url == url));
    let toggle_fav = move |g: Gif| {
        s.update_settings(|st| match st.fav_gifs.iter().position(|f| f.url == g.url) {
            Some(i) => {
                st.fav_gifs.remove(i);
            }
            None => {
                st.fav_gifs.insert(0, g);
                st.fav_gifs.truncate(MAX_FAVS);
            }
        })
    };
    let go = move |v: View, q: String| {
        query.set(q);
        view.set(v);
    };

    // Two columns, each GIF into the shorter one (keeps KLIPY's order, and earlier GIFs stay put as more load).
    let columns = Memo::new(move |_| {
        let items = if view.get() == View::Favorites { s.settings.with(|st| st.fav_gifs.clone()) } else { results.get() };
        let mut cols: [(f64, Vec<Gif>); 2] = [(0.0, vec![]), (0.0, vec![])];
        for g in items {
            let c = usize::from(cols[1].0 < cols[0].0);
            cols[c].0 += g.height as f64 / g.width.max(1) as f64;
            cols[c].1.push(g);
        }
        cols.map(|(_, v)| v)
    });
    let tile = move |g: Gif| {
        let url = g.url.clone();
        let (g1, g2) = (g.clone(), g.clone());
        view! {
            <div class="gif-item" style:aspect-ratio=format!("{} / {}", g.width, g.height) on:click=move |_| pick(g1.clone())>
                <img src=g.preview.clone() alt="" loading="lazy" referrerpolicy="no-referrer"/>
                <button class="gif-star" class:on=move || is_fav(&url) title="Favorite"
                    on:click=move |e| { e.stop_propagation(); toggle_fav(g2.clone()); }>
                    <Icon name="star" size=18/>
                </button>
            </div>
        }
    };
    let empty = move || {
        if loading.get() || columns.with(|c| c.iter().any(|c| !c.is_empty())) {
            return None;
        }
        Some(match view.get() {
            _ if failed.get() => "Couldn't reach KLIPY. Try again in a moment.".to_string(),
            View::Favorites => "Star a GIF to keep it here.".to_string(),
            View::Search => format!("No GIFs found for “{}”.", query.get().trim()),
            _ => String::new(),
        })
    };
    let title = move || match view.get() {
        View::Trending => "Trending GIFs",
        View::Favorites => "Favorites",
        _ => "",
    };

    view! {
        <div class="picker-backdrop" on:click=move |_| picker().set(None)></div>
        <div class="picker gif-picker" style:left=format!("{}px", p.x) style:top=format!("{}px", p.y) on:click=|e| e.stop_propagation()>
            <div class="gif-head">
                <Show when=move || view.get() != View::Home>
                    <button class="icon-btn" title="Back" on:click=move |_| go(View::Home, String::new())><Icon name="arrow-left" size=18/></button>
                </Show>
                <Show when=move || !title().is_empty() fallback=move || view! {
                    <input class="picker-search" placeholder="Search KLIPY" autofocus=true
                        prop:value=move || query.get()
                        on:input=move |e| {
                            let q = event_target_value(&e);
                            let v = if q.trim().is_empty() { View::Home } else { View::Search };
                            go(v, q);
                        }
                        on:keydown=move |e| if e.key() == "Escape" { e.stop_propagation(); picker().set(None); }/>
                }>
                    <span class="gif-title">{title}</span>
                </Show>
            </div>
            <div class="gif-body scroll" on:scroll=on_scroll>
                {move || (view.get() == View::Home).then(|| view! {
                    <div class="gif-tiles">
                        <button class="gif-tile" on:click=move |_| go(View::Favorites, String::new())>
                            {move || s.settings.with(|st| st.fav_gifs.first().map(|f| f.preview.clone())).map(|src| view! { <img src=src alt="" referrerpolicy="no-referrer"/> })}
                            <span><Icon name="star" size=16/>"Favorites"</span>
                        </button>
                        <button class="gif-tile trending" on:click=move |_| go(View::Trending, String::new())>
                            <span><Icon name="trending" size=16/>"Trending GIFs"</span>
                        </button>
                        <For each=move || cats.get() key=|c| c.query.clone() let:c>
                            {
                                let q = c.query.clone();
                                view! {
                                    <button class="gif-tile" on:click=move |_| go(View::Search, q.clone())>
                                        <img src=c.preview_url.clone() alt="" loading="lazy" referrerpolicy="no-referrer"/>
                                        <span>{c.category.clone()}</span>
                                    </button>
                                }
                            }
                        </For>
                    </div>
                })}
                <div class="gif-grid">
                    {[0usize, 1].map(|i| view! {
                        <div class="gif-col">
                            <For each=move || columns.with(|c| c[i].clone()) key=|g| g.url.clone() let:g>{tile(g)}</For>
                        </div>
                    })}
                </div>
                {move || loading.get().then(|| view! { <div class="gif-note">"Loading…"</div> })}
                {move || empty().filter(|t| !t.is_empty()).map(|t| view! { <div class="gif-note">{t}</div> })}
            </div>
        </div>
    }
}
