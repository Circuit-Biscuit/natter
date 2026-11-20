//! Daily Word: our five-letter word game. Same puzzle for everyone each day, English or Turkish; the server keeps
//! the answer and checks every guess, so this is the board, the keyboard and the scoreboards.

use leptos::prelude::*;
use leptos::task::spawn_local;
use natter_proto::Id;
use natter_proto::word::{self, Mark, Overview, Status};

use super::Avatar;
use super::icons::Icon;
use crate::store::store;
use crate::{api, util};

/// Keyboard rows: English QWERTY, Turkish Q (no q, w, x in Turkish words).
fn rows(lang: &str) -> [&'static str; 3] {
    if lang == "tr" { ["ertyuıopğü", "asdfghjklşi", "zcvbnmöç"] } else { ["qwertyuiop", "asdfghjkl", "zxcvbnm"] }
}

fn alphabet(lang: &str) -> String {
    rows(lang).concat()
}

fn praise(tries: usize) -> &'static str {
    ["Unreal!", "Brilliant!", "Excellent!", "Nice one!", "Good!", "Close one!"].get(tries.wrapping_sub(1)).copied().unwrap_or("Done!")
}

fn mark_class(m: Mark) -> &'static str {
    match m {
        Mark::Correct => "correct",
        Mark::Present => "present",
        Mark::Absent => "absent",
    }
}

fn countdown(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// The spoiler-free result, for the clipboard.
fn share_text(o: &Overview) -> String {
    let t = &o.today;
    let flag = word::LANGS.iter().find(|(c, ..)| *c == t.lang).map_or("", |(_, _, f)| f);
    let tries = if t.status == Status::Won { t.guesses.len().to_string() } else { "X".into() };
    let grid: Vec<String> = t
        .guesses
        .iter()
        .map(|g| g.marks.iter().map(|m| match m { Mark::Correct => "🟩", Mark::Present => "🟨", Mark::Absent => "⬛" }).collect())
        .collect();
    format!("Daily Word {flag} #{} · {tries}/6{}\n{}", t.number, if t.hard { " · hard" } else { "" }, grid.join("\n"))
}

#[component]
pub fn WordGame(space: Option<Id>) -> impl IntoView {
    let s = store();
    let browser_tr = util::window().navigator().language().is_some_and(|l| l.starts_with("tr"));
    let lang = RwSignal::new(
        util::storage_get("natter.word.lang").filter(|l| l == "en" || l == "tr").unwrap_or_else(|| if browser_tr { "tr" } else { "en" }.into()),
    );
    let colorblind = RwSignal::new(util::storage_get("natter.word.colorblind").is_some_and(|v| v == "1"));
    let hard = RwSignal::new(util::storage_get("natter.word.hard").is_some_and(|v| v == "1"));
    let data = RwSignal::new(None::<Overview>);
    let input = RwSignal::new(String::new());
    let note = RwSignal::new(None::<String>);
    let shake = RwSignal::new(false);
    let revealing = RwSignal::new(None::<usize>);
    let busy = RwSignal::new(false);
    let tab = RwSignal::new("play");
    let period = RwSignal::new("week");
    let now = RwSignal::new(0i64);

    let load = move || {
        let l = lang.get_untracked();
        spawn_local(async move {
            let q = space.map(|s| format!("&space_id={s}")).unwrap_or_default();
            match api::get::<Overview>(&format!("/api/games/word?lang={l}{q}")).await {
                Ok(o) => {
                    input.set(String::new());
                    data.set(Some(o));
                }
                Err(e) => store().error(e),
            }
        });
    };
    load();
    Effect::new(move |prev: Option<String>| {
        let l = lang.get();
        if prev.is_some_and(|p| p != l) {
            util::storage_set("natter.word.lang", &l);
            data.set(None);
            load();
        }
        l
    });
    // The countdown to the next puzzle.
    let started = js_sys::Date::now() as i64;
    let tick = leptos::prelude::set_interval_with_handle(move || now.set(js_sys::Date::now() as i64 - started), std::time::Duration::from_secs(1)).ok();
    on_cleanup(move || {
        if let Some(t) = tick {
            t.clear();
        }
    });

    let say = move |text: String| {
        note.set(Some(text));
        set_timeout(move || note.set(None), std::time::Duration::from_millis(1800));
    };
    let playing = move || data.with(|d| d.as_ref().is_some_and(|o| o.today.status == Status::Playing));
    let submit = move || {
        let word = input.get_untracked();
        if busy.get_untracked() || !playing() {
            return;
        }
        if word.chars().count() < word::LENGTH {
            shake.set(true);
            set_timeout(move || shake.set(false), std::time::Duration::from_millis(500));
            say("Not enough letters".into());
            return;
        }
        busy.set(true);
        let body = word::GuessReq { lang: lang.get_untracked(), word, hard: hard.get_untracked(), space_id: space };
        spawn_local(async move {
            match api::call::<_, Overview>("POST", "/api/games/word/guess", &body).await {
                Ok(o) => {
                    let row = o.today.guesses.len().saturating_sub(1);
                    let status = o.today.status;
                    let answer = o.today.answer.clone().unwrap_or_default();
                    let lang_now = o.today.lang.clone();
                    input.set(String::new());
                    revealing.set(Some(row));
                    data.set(Some(o));
                    set_timeout(move || {
                        revealing.set(None);
                        match status {
                            Status::Won => say(praise(row + 1).into()),
                            Status::Lost => say(word::display(&lang_now, &answer)),
                            Status::Playing => {}
                        }
                    }, std::time::Duration::from_millis(1700));
                }
                Err(e) => {
                    shake.set(true);
                    set_timeout(move || shake.set(false), std::time::Duration::from_millis(500));
                    let m = e.message.trim_start_matches("hard mode: ");
                    let mut c = m.chars();
                    say(c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default());
                }
            }
            busy.set(false);
        });
    };
    let press = move |key: String| {
        if !playing() || revealing.get_untracked().is_some() {
            return;
        }
        match key.as_str() {
            "Enter" => submit(),
            "Backspace" => input.update(|w| {
                w.pop();
            }),
            k => {
                let l = lang.get_untracked();
                let c = word::normalize(&l, k);
                if c.chars().count() == 1 && alphabet(&l).contains(&c) && input.with_untracked(|w| w.chars().count()) < word::LENGTH {
                    input.update(|w| w.push_str(&c));
                }
            }
        }
    };
    // Best mark per letter, for the keyboard.
    let key_mark = move |c: char| {
        data.with(|d| {
            d.as_ref().and_then(|o| {
                o.today.guesses.iter().flat_map(|g| g.word.chars().zip(g.marks.iter().copied())).filter(|(x, _)| *x == c).map(|(_, m)| m).min_by_key(|m| match m {
                    Mark::Correct => 0,
                    Mark::Present => 1,
                    Mark::Absent => 2,
                })
            })
        })
    };
    let game_ref = NodeRef::<leptos::html::Div>::new();
    Effect::new(move |_| {
        if let Some(el) = game_ref.get() {
            let _ = el.focus();
        }
    });
    let channel_name = move || {
        space.and_then(|sp| s.space(sp)).and_then(|sp| sp.game_channel_id).and_then(|c| s.channel(c)).map(|c| c.name)
    };

    view! {
        <div class="word-game" class:colorblind=move || colorblind.get() tabindex="0" node_ref=game_ref
            on:keydown=move |e: leptos::ev::KeyboardEvent| {
                let k = e.key();
                if e.ctrl_key() || e.meta_key() || e.alt_key() || tab.get_untracked() != "play"
                    || !(k == "Enter" || k == "Backspace" || (k.chars().count() == 1 && k != " ")) { return; }
                e.prevent_default();
                e.stop_propagation();
                press(k);
            }>
            <header class="word-head">
                <div class="word-title">
                    <span class="word-logo"><Icon name="grid" size=20/></span>
                    <b>"Daily Word"</b>
                    <span class="muted">{move || data.with(|d| d.as_ref().map(|o| format!("#{}", o.today.number)).unwrap_or_default())}</span>
                </div>
                <div class="word-langs">
                    {word::LANGS.into_iter().map(|(code, name, _)| view! {
                        <button class:active=move || lang.get() == code on:click=move |_| lang.set(code.into())>{name}</button>
                    }).collect_view()}
                </div>
                <div class="tabs word-tabs">
                    {[("play", "Play"), ("stats", "Stats"), ("board", "Standings")].into_iter().map(|(id, label)| view! {
                        <button class:active=move || tab.get() == id on:click=move |_| tab.set(id)>{label}</button>
                    }).collect_view()}
                </div>
                <button class="icon-btn" title="Close" on:click=move |_| store().modal.set(None)><Icon name="x"/></button>
            </header>

            {move || data.get().is_some().then(|| view! {
                <div class="word-players">
                    {move || data.with(|d| d.as_ref().map(|o| o.players.clone()).unwrap_or_default()).into_iter().map(|p| {
                        let label = match p.status { Status::Won => format!("{}/6", p.tries), Status::Lost => "X/6".into(), Status::Playing => "…".into() };
                        let cls = match p.status { Status::Won => "won", Status::Lost => "lost", Status::Playing => "playing" };
                        view! {
                            <span class=format!("word-player {cls}") title=move || s.name(p.user_id, space)>
                                <Avatar user=p.user_id size=24/><span>{label}</span>
                            </span>
                        }
                    }).collect_view()}
                    {move || data.with(|d| d.as_ref().is_some_and(|o| o.players.is_empty())).then(|| view! { <span class="muted small">"Nobody has played today yet — be the first."</span> })}
                </div>
            })}

            {move || note.get().map(|n| view! { <div class="word-note">{n}</div> })}

            {move || match tab.get() {
                "stats" => view! { <WordStats data=data/> }.into_any(),
                "board" => view! { <WordStandings data=data period=period space=space/> }.into_any(),
                _ => view! {
                    <div class="word-board">
                        {(0..word::TRIES).map(|r| view! {
                            <div class="word-row" class:shake=move || shake.get() && data.with(|d| d.as_ref().map_or(0, |o| o.today.guesses.len())) == r
                                class:reveal=move || revealing.get() == Some(r)
                                class:win=move || revealing.get().is_none() && data.with(|d| d.as_ref().is_some_and(|o| o.today.status == Status::Won && o.today.guesses.len() == r + 1))>
                                {(0..word::LENGTH).map(|i| {
                                    let tile = move || data.with(|d| {
                                        let Some(o) = d.as_ref() else { return (String::new(), None) };
                                        let l = o.today.lang.clone();
                                        match o.today.guesses.get(r) {
                                            Some(g) => (g.word.chars().nth(i).map(|c| word::display(&l, &c.to_string())).unwrap_or_default(), g.marks.get(i).copied()),
                                            None if r == o.today.guesses.len() => (input.with(|w| w.chars().nth(i).map(|c| word::display(&l, &c.to_string())).unwrap_or_default()), None),
                                            None => (String::new(), None),
                                        }
                                    });
                                    view! {
                                        <div class=move || {
                                            let (letter, mark) = tile();
                                            let mut c = String::from("word-tile");
                                            if !letter.is_empty() { c += " filled"; }
                                            if let Some(m) = mark { c += " "; c += mark_class(m); }
                                            c
                                        } style:animation-delay=format!("{}ms", i * 260)>{move || tile().0}</div>
                                    }
                                }).collect_view()}
                            </div>
                        }).collect_view()}
                    </div>
                    {move || data.with(|d| d.as_ref().filter(|o| o.today.status != Status::Playing).map(|o| {
                        let text = share_text(o);
                        let next = o.today.next_in_ms;
                        view! {
                            <div class="word-done">
                                <div>
                                    <div class="muted small">"Next puzzle in"</div>
                                    <div class="word-countdown">{move || countdown(next - now.get())}</div>
                                </div>
                                <div class="word-done-actions">
                                    <button class="btn" on:click=move |_| { util::copy_text(&text); store().toast("Result copied.", false); }>
                                        <Icon name="link" size=16/>" Copy result"
                                    </button>
                                    <div class="muted small">{move || match channel_name() {
                                        Some(c) => format!("Posted to #{c}"),
                                        None => "Space admins can pick a channel for results in Space Settings".into(),
                                    }}</div>
                                </div>
                            </div>
                        }
                    }))}
                    <div class="word-keys">
                        {move || {
                            let l = lang.get();
                            rows(&l).into_iter().enumerate().map(|(i, row)| view! {
                                <div class="word-keyrow">
                                    {(i == 2).then(|| view! { <button class="word-key wide" on:click=move |_| press("Enter".into())>"ENTER"</button> })}
                                    {row.chars().map(|c| {
                                        let l2 = l.clone();
                                        view! {
                                            <button class=move || format!("word-key {}", key_mark(c).map(mark_class).unwrap_or(""))
                                                on:click=move |_| press(c.to_string())>{word::display(&l2, &c.to_string())}</button>
                                        }
                                    }).collect_view()}
                                    {(i == 2).then(|| view! { <button class="word-key wide" title="Delete" on:click=move |_| press("Backspace".into())>"⌫"</button> })}
                                </div>
                            }).collect_view()
                        }}
                    </div>
                    <div class="word-options">
                        <label class="check" title="Revealed hints must be used in later guesses (set before your first guess)">
                            <input type="checkbox" prop:checked=move || hard.get()
                                prop:disabled=move || data.with(|d| d.as_ref().is_some_and(|o| !o.today.guesses.is_empty()))
                                on:change=move |e| { let v = event_target_checked(&e); hard.set(v); util::storage_set("natter.word.hard", if v { "1" } else { "0" }); }/>
                            "Hard mode"
                        </label>
                        <label class="check">
                            <input type="checkbox" prop:checked=move || colorblind.get()
                                on:change=move |e| { let v = event_target_checked(&e); colorblind.set(v); util::storage_set("natter.word.colorblind", if v { "1" } else { "0" }); }/>
                            "High-contrast colours"
                        </label>
                    </div>
                }.into_any(),
            }}
        </div>
    }
}

#[component]
fn WordStats(data: RwSignal<Option<Overview>>) -> impl IntoView {
    view! {
        {move || data.get().map(|o| {
            let st = o.stats.clone();
            let win = if st.played > 0 { st.won * 100 / st.played } else { 0 };
            let top = st.distribution.iter().copied().max().unwrap_or(0).max(1);
            let today = (o.today.status == Status::Won).then_some(o.today.guesses.len());
            view! {
                <div class="word-stats">
                    <div class="word-numbers">
                        {[(st.played.to_string(), "Played"), (win.to_string(), "Win %"), (st.current_streak.to_string(), "Current streak"), (st.max_streak.to_string(), "Best streak")]
                            .into_iter().map(|(n, l)| view! { <div><b>{n}</b><span>{l}</span></div> }).collect_view()}
                    </div>
                    <h3>"Guess distribution"</h3>
                    {st.distribution.iter().enumerate().map(|(i, n)| {
                        let pct = (n * 100 / top).max(7);
                        view! {
                            <div class="word-bar">
                                <span>{i + 1}</span>
                                <div class="word-bar-fill" class:today=today == Some(i + 1) style:width=format!("{pct}%")>{*n}</div>
                            </div>
                        }
                    }).collect_view()}
                </div>
            }
        })}
    }
}

#[component]
fn WordStandings(data: RwSignal<Option<Overview>>, period: RwSignal<&'static str>, space: Option<Id>) -> impl IntoView {
    let s = store();
    view! {
        <div class="word-standings">
            <div class="tabs">
                {[("week", "This week"), ("month", "This month"), ("all", "All time")].into_iter().map(|(id, label)| view! {
                    <button class:active=move || period.get() == id on:click=move |_| period.set(id)>{label}</button>
                }).collect_view()}
            </div>
            <p class="muted small">"Points: 6 for a word in one try, down to 1 in six; English and Turkish both count."</p>
            {move || {
                let list = data.with(|d| d.as_ref().map(|o| match period.get() {
                    "month" => o.month.clone(),
                    "all" => o.all_time.clone(),
                    _ => o.week.clone(),
                }).unwrap_or_default());
                if list.is_empty() {
                    return view! { <p class="muted">"No finished games yet."</p> }.into_any();
                }
                list.into_iter().enumerate().map(|(i, e)| {
                    let medal = if e.points > 0 { ["🥇", "🥈", "🥉"].get(i).copied().unwrap_or("") } else { "" };
                    view! {
                        <div class="word-standing">
                            <span class="word-rank">{if medal.is_empty() { (i + 1).to_string() } else { medal.to_string() }}</span>
                            <Avatar user=e.user_id size=28/>
                            <span class="grow ellipsis">{s.name(e.user_id, space)}</span>
                            <span class="muted small">{format!("{}/{} won", e.won, e.played)}</span>
                            <b class="word-points">{e.points}</b>
                        </div>
                    }
                }).collect_view().into_any()
            }}
        </div>
    }
}
