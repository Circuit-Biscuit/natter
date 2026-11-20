//! Emoji picker: search by name, browse by category, recent first. Shift+click keeps it open.

use leptos::prelude::*;

use crate::store::store;

#[derive(Clone)]
struct Picker {
    x: i32,
    y: i32,
    on_pick: std::sync::Arc<dyn Fn(String) + Send + Sync>,
}

fn picker() -> RwSignal<Option<Picker>> {
    thread_local! { static P: RwSignal<Option<Picker>> = RwSignal::new(None); }
    P.with(|p| *p)
}

/// Create the picker's signal under the app root.
pub fn init() {
    picker();
}

const W: i32 = 352;
const H: i32 = 420;

pub fn open_picker(e: &web_sys::MouseEvent, on_pick: impl Fn(String) + Send + Sync + 'static) {
    e.stop_propagation();
    open_picker_at(e.client_x(), e.client_y(), on_pick);
}

/// Open with the picker's bottom-right corner near (x, y), kept on screen.
pub fn open_picker_at(cx: i32, cy: i32, on_pick: impl Fn(String) + Send + Sync + 'static) {
    let win = crate::util::window();
    let vw = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1200.0) as i32;
    let vh = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(800.0) as i32;
    let x = (cx - W).clamp(8, (vw - W - 8).max(8));
    let y = (cy - H - 8).clamp(8, (vh - H - 8).max(8));
    store().menu.set(None);
    picker().set(Some(Picker { x, y, on_pick: std::sync::Arc::new(on_pick) }));
}

const GROUPS: [(emojis::Group, &str, &str); 9] = [
    (emojis::Group::SmileysAndEmotion, "😀", "Smileys"),
    (emojis::Group::PeopleAndBody, "👋", "People"),
    (emojis::Group::AnimalsAndNature, "🐻", "Nature"),
    (emojis::Group::FoodAndDrink, "🍔", "Food"),
    (emojis::Group::TravelAndPlaces, "✈️", "Travel"),
    (emojis::Group::Activities, "⚽", "Activities"),
    (emojis::Group::Objects, "💡", "Objects"),
    (emojis::Group::Symbols, "❤️", "Symbols"),
    (emojis::Group::Flags, "🏳️", "Flags"),
];

#[component]
pub fn PickerHost() -> impl IntoView {
    view! {
        {move || picker().get().map(|p| view! { <PickerView p=p/> })}
    }
}

#[component]
fn PickerView(p: Picker) -> impl IntoView {
    let s = store();
    let query = RwSignal::new(String::new());
    let group = RwSignal::new(0usize);
    let hover = RwSignal::new(None::<(String, String)>);
    let on_pick = StoredValue::new(p.on_pick);
    let pick = move |e: String| {
        on_pick.with_value(|f| f(e));
        if !s.shift.get_untracked() {
            picker().set(None);
        }
    };
    let list = move || -> Vec<(String, String)> {
        let q = query.get().trim().to_lowercase().replace(':', "");
        if q.is_empty() {
            let g = GROUPS[group.get()].0;
            emojis::iter()
                .filter(|e| e.group() == g && e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default))
                .map(|e| (e.as_str().to_string(), e.shortcode().unwrap_or(e.name()).to_string()))
                .collect()
        } else {
            emojis::iter()
                .filter(|e| e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default))
                .filter(|e| e.name().contains(&q) || e.shortcodes().any(|c| c.contains(&q)))
                .take(120)
                .map(|e| (e.as_str().to_string(), e.shortcode().unwrap_or(e.name()).to_string()))
                .collect()
        }
    };
    let recent = move || s.settings.with(|st| st.recent_emoji.clone());
    view! {
        <div class="picker-backdrop" on:click=move |_| picker().set(None)></div>
        <div class="picker" style:left=format!("{}px", p.x) style:top=format!("{}px", p.y) on:click=|e| e.stop_propagation()>
            <input class="picker-search" placeholder="Find the perfect emoji" autofocus=true
                prop:value=move || query.get() on:input=move |e| query.set(event_target_value(&e))
                on:keydown=move |e| {
                    if e.key() == "Enter" {
                        if let Some((em, _)) = list().into_iter().next() { pick(em); }
                    }
                    if e.key() == "Escape" { e.stop_propagation(); picker().set(None); }
                }/>
            <div class="picker-tabs">
                {GROUPS.iter().enumerate().map(|(i, (_, icon, name))| view! {
                    <button class:active=move || group.get() == i && query.with(|q| q.is_empty()) title=*name
                        on:click=move |_| { query.set(String::new()); group.set(i); }>{*icon}</button>
                }).collect_view()}
            </div>
            <div class="picker-grid scroll">
                <Show when=move || query.with(|q| q.is_empty()) && !recent().is_empty()>
                    <div class="picker-label">"Frequently used"</div>
                    <div class="picker-row">
                        {move || recent().into_iter().map(|em| {
                            let e2 = em.clone();
                            view! { <button on:click=move |_| pick(e2.clone())>{em}</button> }
                        }).collect_view()}
                    </div>
                    <div class="picker-label">{move || GROUPS[group.get()].2}</div>
                </Show>
                <div class="picker-row">
                    {move || list().into_iter().map(|(em, code)| {
                        let e2 = em.clone();
                        let h = (em.clone(), code.clone());
                        view! {
                            <button title=format!(":{code}:") on:mouseenter=move |_| hover.set(Some(h.clone()))
                                on:click=move |_| pick(e2.clone())>{em}</button>
                        }
                    }).collect_view()}
                </div>
            </div>
            <div class="picker-foot">
                {move || hover.get().map(|(em, code)| view! { <span class="big">{em}</span><span>{format!(":{code}:")}</span> })}
                <span class="spacer"></span>
                <span class="muted small">"Shift+click to pick several"</span>
            </div>
        </div>
    }
}
